//! Binary split tree for managing terminal surface layout.
//!
//! The tree has two node types:
//! - Leaf: a single terminal surface
//! - Split: two children with a direction and ratio
//!
//! The tree is used to compute NSView frames when the window resizes.

use crate::ffi;
use objc2_foundation::{NSPoint, NSRect, NSSize};
use std::ffi::c_void;

/// Unique ID for a leaf node (surface).
pub type LeafId = usize;

/// Split direction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Direction {
    Horizontal, // left | right
    Vertical,   // top / bottom
}

/// A node in the split tree.
#[derive(Debug)]
pub enum Node {
    Leaf {
        id: LeafId,
        surface: ffi::ghostty_surface_t,
        nsview: *mut c_void,
    },
    Split {
        direction: Direction,
        ratio: f64, // 0.0..1.0, size of first child relative to total
        first: Box<Node>,
        second: Box<Node>,
    },
}

/// The split tree manager.
pub struct SplitTree {
    root: Option<Node>,
    next_id: LeafId,
    focused_id: LeafId,
}

impl SplitTree {
    pub fn new() -> Self {
        SplitTree {
            root: None,
            next_id: 0,
            focused_id: 0,
        }
    }

    /// Add the first surface (becomes root leaf).
    pub fn add_root(&mut self, surface: ffi::ghostty_surface_t, nsview: *mut c_void) -> LeafId {
        let id = self.next_id;
        self.next_id += 1;
        self.root = Some(Node::Leaf { id, surface, nsview });
        self.focused_id = id;
        id
    }

    /// Split the focused leaf in the given direction. Returns the new leaf's ID.
    pub fn split_focused(
        &mut self,
        direction: Direction,
        surface: ffi::ghostty_surface_t,
        nsview: *mut c_void,
    ) -> Option<LeafId> {
        self.split_focused_with_ratio(direction, surface, nsview, 0.5)
    }

    /// Split the focused leaf with a specific ratio.
    pub fn split_focused_with_ratio(
        &mut self,
        direction: Direction,
        surface: ffi::ghostty_surface_t,
        nsview: *mut c_void,
        ratio: f64,
    ) -> Option<LeafId> {
        let focused = self.focused_id;
        let new_id = self.next_id;
        self.next_id += 1;

        let root = self.root.take()?;
        self.root = Some(split_node_with_ratio(root, focused, direction, new_id, surface, nsview, ratio));
        self.focused_id = new_id;
        Some(new_id)
    }

    /// Get the focused surface.
    pub fn focused_surface(&self) -> ffi::ghostty_surface_t {
        self.root
            .as_ref()
            .and_then(|r| find_leaf(r, self.focused_id))
            .map(|(s, _)| s)
            .unwrap_or(std::ptr::null_mut())
    }

    /// Set focus to a specific leaf.
    pub fn set_focus(&mut self, id: LeafId) {
        self.focused_id = id;
    }

    /// Focus the next leaf (in-order traversal).
    pub fn focus_next(&mut self) {
        if let Some(ref root) = self.root {
            let leaves = collect_leaf_ids(root);
            if let Some(pos) = leaves.iter().position(|&id| id == self.focused_id) {
                self.focused_id = leaves[(pos + 1) % leaves.len()];
            }
        }
    }

    /// Focus the previous leaf.
    pub fn focus_prev(&mut self) {
        if let Some(ref root) = self.root {
            let leaves = collect_leaf_ids(root);
            if let Some(pos) = leaves.iter().position(|&id| id == self.focused_id) {
                self.focused_id = leaves[(pos + leaves.len() - 1) % leaves.len()];
            }
        }
    }

    /// Lay out all surfaces within the given frame. Calls set_frame on each NSView
    /// and returns (surface, pixel_width, pixel_height) for each leaf.
    pub fn layout(&self, frame: NSRect, scale: f64) -> Vec<(ffi::ghostty_surface_t, u32, u32)> {
        let mut result = Vec::new();
        if let Some(ref root) = self.root {
            layout_node(root, frame, scale, &mut result);
        }
        result
    }

    /// Export the tree as a flat list of panes with split info.
    pub fn export_panes(&self) -> Vec<ExportedPane> {
        let mut panes = Vec::new();
        if let Some(ref root) = self.root {
            export_node(root, None, &mut panes);
        }
        panes
    }

    /// Collect all surfaces for cleanup.
    pub fn all_surfaces(&self) -> Vec<ffi::ghostty_surface_t> {
        match &self.root {
            Some(root) => {
                let mut surfaces = Vec::new();
                collect_surfaces(root, &mut surfaces);
                surfaces
            }
            None => Vec::new(),
        }
    }

    /// Number of leaves.
    pub fn len(&self) -> usize {
        self.root.as_ref().map(count_leaves).unwrap_or(0)
    }

    /// Show or hide all NSViews in this tree.
    pub fn set_hidden(&self, hidden: bool) {
        if let Some(ref root) = self.root {
            set_hidden_recursive(root, hidden);
        }
    }

    /// Remove the focused leaf from the tree.
    /// Returns the removed surface and nsview, or None if not found.
    /// When a split loses one child, the other child replaces the split node.
    pub fn remove_focused(&mut self) -> Option<(ffi::ghostty_surface_t, *mut c_void)> {
        let root = self.root.take()?;
        match remove_leaf(root, self.focused_id) {
            RemoveResult::Removed {
                surface,
                nsview,
                remaining,
            } => {
                self.root = remaining;
                // Focus the first available leaf
                if let Some(ref root) = self.root {
                    let leaves = collect_leaf_ids(root);
                    if !leaves.is_empty() {
                        self.focused_id = leaves[0];
                    }
                }
                Some((surface, nsview))
            }
            RemoveResult::NotFound(node) => {
                self.root = Some(node);
                None
            }
        }
    }

    /// Resize the nearest matching split ancestor of the focused leaf.
    /// `axis` is which split direction to resize (Horizontal or Vertical).
    /// `delta` is the ratio change (positive = grow first child).
    pub fn resize_focused(&mut self, axis: Direction, delta: f64) {
        if let Some(ref mut root) = self.root {
            resize_toward_leaf(root, self.focused_id, axis, delta);
        }
    }

    /// Check if a point is on a split divider. Returns the direction if so.
    pub fn divider_at(&self, frame: NSRect, point: (f64, f64)) -> Option<Direction> {
        self.root
            .as_ref()
            .and_then(|root| divider_at_node(root, frame, point))
    }

    /// Update the ratio of the split being dragged. Finds the shallowest split
    /// with matching direction whose frame contains the point.
    pub fn resize_drag(&mut self, frame: NSRect, dir: Direction, point: (f64, f64)) {
        if let Some(ref mut root) = self.root {
            resize_drag_node(root, frame, dir, point);
        }
    }

    /// Find the leaf at a given point and set focus to it.
    /// Returns true if focus changed.
    pub fn focus_at(&mut self, frame: NSRect, point: (f64, f64)) -> bool {
        if let Some(ref root) = self.root {
            if let Some(id) = leaf_at_point(root, frame, point) {
                if id != self.focused_id {
                    self.focused_id = id;
                    return true;
                }
            }
        }
        false
    }

    /// Get surface info for control socket.
    pub fn surface_info(&self) -> Vec<(LeafId, bool)> {
        match &self.root {
            Some(root) => {
                let ids = collect_leaf_ids(root);
                ids.into_iter().map(|id| (id, id == self.focused_id)).collect()
            }
            None => Vec::new(),
        }
    }
}

fn split_node_with_ratio(
    node: Node,
    target_id: LeafId,
    direction: Direction,
    new_id: LeafId,
    surface: ffi::ghostty_surface_t,
    nsview: *mut c_void,
    ratio: f64,
) -> Node {
    match node {
        Node::Leaf { id, .. } if id == target_id => {
            let new_leaf = Node::Leaf { id: new_id, surface, nsview };
            Node::Split {
                direction,
                ratio: ratio.clamp(0.1, 0.9),
                first: Box::new(node),
                second: Box::new(new_leaf),
            }
        }
        Node::Split { direction: d, ratio: r, first, second } => {
            Node::Split {
                direction: d,
                ratio: r,
                first: Box::new(split_node_with_ratio(*first, target_id, direction, new_id, surface, nsview, ratio)),
                second: Box::new(split_node_with_ratio(*second, target_id, direction, new_id, surface, nsview, ratio)),
            }
        }
        other => other,
    }
}

fn find_leaf(node: &Node, id: LeafId) -> Option<(ffi::ghostty_surface_t, *mut c_void)> {
    match node {
        Node::Leaf { id: lid, surface, nsview } if *lid == id => Some((*surface, *nsview)),
        Node::Split { first, second, .. } => {
            find_leaf(first, id).or_else(|| find_leaf(second, id))
        }
        _ => None,
    }
}

fn collect_leaf_ids(node: &Node) -> Vec<LeafId> {
    match node {
        Node::Leaf { id, .. } => vec![*id],
        Node::Split { first, second, .. } => {
            let mut ids = collect_leaf_ids(first);
            ids.extend(collect_leaf_ids(second));
            ids
        }
    }
}

fn collect_surfaces(node: &Node, out: &mut Vec<ffi::ghostty_surface_t>) {
    match node {
        Node::Leaf { surface, .. } => out.push(*surface),
        Node::Split { first, second, .. } => {
            collect_surfaces(first, out);
            collect_surfaces(second, out);
        }
    }
}

fn count_leaves(node: &Node) -> usize {
    match node {
        Node::Leaf { .. } => 1,
        Node::Split { first, second, .. } => count_leaves(first) + count_leaves(second),
    }
}

fn layout_node(
    node: &Node,
    frame: NSRect,
    scale: f64,
    out: &mut Vec<(ffi::ghostty_surface_t, u32, u32)>,
) {
    match node {
        Node::Leaf { surface, nsview, .. } => {
            crate::appkit::set_view_frame(*nsview, frame);
            let w = (frame.size.width * scale) as u32;
            let h = (frame.size.height * scale) as u32;
            out.push((*surface, w, h));
        }
        Node::Split { direction, ratio, first, second } => {
            let (first_frame, second_frame) = split_frame(frame, *direction, *ratio);
            layout_node(first, first_frame, scale, out);
            layout_node(second, second_frame, scale, out);
        }
    }
}

/// Gap between split panes (points). The iced background shows through.
const SPLIT_BORDER: f64 = 1.0;

fn split_frame(frame: NSRect, direction: Direction, ratio: f64) -> (NSRect, NSRect) {
    match direction {
        Direction::Horizontal => {
            let usable = frame.size.width - SPLIT_BORDER;
            let w1 = usable * ratio;
            let w2 = usable - w1;
            (
                NSRect::new(frame.origin, NSSize::new(w1, frame.size.height)),
                NSRect::new(
                    NSPoint::new(frame.origin.x + w1 + SPLIT_BORDER, frame.origin.y),
                    NSSize::new(w2, frame.size.height),
                ),
            )
        }
        Direction::Vertical => {
            // First child at top, second below (flipped coordinates: origin is top-left)
            let usable = frame.size.height - SPLIT_BORDER;
            let h1 = usable * ratio;
            let h2 = usable - h1;
            (
                NSRect::new(frame.origin, NSSize::new(frame.size.width, h1)),
                NSRect::new(
                    NSPoint::new(frame.origin.x, frame.origin.y + h1 + SPLIT_BORDER),
                    NSSize::new(frame.size.width, h2),
                ),
            )
        }
    }
}


enum RemoveResult {
    Removed {
        surface: ffi::ghostty_surface_t,
        nsview: *mut c_void,
        remaining: Option<Node>,
    },
    NotFound(Node),
}

fn remove_leaf(node: Node, target_id: LeafId) -> RemoveResult {
    match node {
        Node::Leaf { id, surface, nsview } if id == target_id => RemoveResult::Removed {
            surface,
            nsview,
            remaining: None,
        },
        Node::Split {
            direction,
            ratio,
            first,
            second,
        } => {
            match remove_leaf(*first, target_id) {
                RemoveResult::Removed {
                    surface,
                    nsview,
                    remaining: Some(remaining_first),
                } => RemoveResult::Removed {
                    surface,
                    nsview,
                    remaining: Some(Node::Split {
                        direction,
                        ratio,
                        first: Box::new(remaining_first),
                        second,
                    }),
                },
                RemoveResult::Removed {
                    surface,
                    nsview,
                    remaining: None,
                } => {
                    // First child was the target leaf — replace split with second child
                    RemoveResult::Removed {
                        surface,
                        nsview,
                        remaining: Some(*second),
                    }
                }
                RemoveResult::NotFound(first_node) => {
                    // Try second child
                    match remove_leaf(*second, target_id) {
                        RemoveResult::Removed {
                            surface,
                            nsview,
                            remaining: Some(remaining_second),
                        } => RemoveResult::Removed {
                            surface,
                            nsview,
                            remaining: Some(Node::Split {
                                direction,
                                ratio,
                                first: Box::new(first_node),
                                second: Box::new(remaining_second),
                            }),
                        },
                        RemoveResult::Removed {
                            surface,
                            nsview,
                            remaining: None,
                        } => {
                            // Second child was target — replace split with first child
                            RemoveResult::Removed {
                                surface,
                                nsview,
                                remaining: Some(first_node),
                            }
                        }
                        RemoveResult::NotFound(second_node) => {
                            RemoveResult::NotFound(Node::Split {
                                direction,
                                ratio,
                                first: Box::new(first_node),
                                second: Box::new(second_node),
                            })
                        }
                    }
                }
            }
        }
        other => RemoveResult::NotFound(other),
    }
}

/// Find the nearest ancestor split matching `axis` that contains the focused leaf,
/// and adjust its ratio by `delta`. Returns (found_focused, resize_applied).
fn resize_toward_leaf(node: &mut Node, target: LeafId, axis: Direction, delta: f64) -> (bool, bool) {
    match node {
        Node::Leaf { id, .. } => (*id == target, false),
        Node::Split { direction, ratio, first, second } => {
            let (found, done) = resize_toward_leaf(first, target, axis, delta);
            if found && done {
                return (true, true);
            }

            let (found, done) = if found {
                (true, false)
            } else {
                resize_toward_leaf(second, target, axis, delta)
            };

            if !found {
                return (false, false);
            }
            if done {
                return (true, true);
            }

            if *direction == axis {
                *ratio = (*ratio + delta).clamp(0.1, 0.9);
                (true, true)
            } else {
                (true, false)
            }
        }
    }
}

/// Expand hit area beyond the 1pt border for easier clicking.
const DIVIDER_HIT_MARGIN: f64 = 3.0;

fn divider_at_node(node: &Node, frame: NSRect, point: (f64, f64)) -> Option<Direction> {
    match node {
        Node::Leaf { .. } => None,
        Node::Split {
            direction,
            ratio,
            first,
            second,
        } => {
            let (first_frame, second_frame) = split_frame(frame, *direction, *ratio);
            let half = SPLIT_BORDER / 2.0 + DIVIDER_HIT_MARGIN;

            let on_divider = match direction {
                Direction::Horizontal => {
                    let div_x = first_frame.origin.x + first_frame.size.width + SPLIT_BORDER / 2.0;
                    (point.0 - div_x).abs() < half
                        && point.1 >= frame.origin.y
                        && point.1 <= frame.origin.y + frame.size.height
                }
                Direction::Vertical => {
                    let div_y =
                        first_frame.origin.y + first_frame.size.height + SPLIT_BORDER / 2.0;
                    (point.1 - div_y).abs() < half
                        && point.0 >= frame.origin.x
                        && point.0 <= frame.origin.x + frame.size.width
                }
            };

            if on_divider {
                return Some(*direction);
            }

            // Recurse into children (deeper splits take priority)
            divider_at_node(first, first_frame, point)
                .or_else(|| divider_at_node(second, second_frame, point))
        }
    }
}

/// During drag: find the shallowest split with matching direction whose frame
/// contains the point, and set its ratio from the mouse position.
fn resize_drag_node(
    node: &mut Node,
    frame: NSRect,
    target_dir: Direction,
    point: (f64, f64),
) -> bool {
    match node {
        Node::Leaf { .. } => false,
        Node::Split {
            direction,
            ratio,
            first,
            second,
        } => {
            if *direction == target_dir {
                // Update this split's ratio from mouse position
                let new_ratio = match direction {
                    Direction::Horizontal => (point.0 - frame.origin.x) / frame.size.width,
                    Direction::Vertical => (point.1 - frame.origin.y) / frame.size.height,
                };
                *ratio = new_ratio.clamp(0.1, 0.9);
                return true;
            }

            // Wrong direction — recurse to find a matching child split
            let (first_frame, second_frame) = split_frame(frame, *direction, *ratio);
            let in_first = point_in_frame(point, first_frame);
            if in_first {
                resize_drag_node(first, first_frame, target_dir, point)
            } else {
                resize_drag_node(second, second_frame, target_dir, point)
            }
        }
    }
}

fn leaf_at_point(node: &Node, frame: NSRect, point: (f64, f64)) -> Option<LeafId> {
    match node {
        Node::Leaf { id, .. } => {
            if point_in_frame(point, frame) {
                Some(*id)
            } else {
                None
            }
        }
        Node::Split {
            direction,
            ratio,
            first,
            second,
            ..
        } => {
            let (first_frame, second_frame) = split_frame(frame, *direction, *ratio);
            leaf_at_point(first, first_frame, point)
                .or_else(|| leaf_at_point(second, second_frame, point))
        }
    }
}

fn point_in_frame(point: (f64, f64), frame: NSRect) -> bool {
    point.0 >= frame.origin.x
        && point.0 <= frame.origin.x + frame.size.width
        && point.1 >= frame.origin.y
        && point.1 <= frame.origin.y + frame.size.height
}

fn set_hidden_recursive(node: &Node, hidden: bool) {
    match node {
        Node::Leaf { nsview, .. } => crate::appkit::set_view_hidden(*nsview, hidden),
        Node::Split { first, second, .. } => {
            set_hidden_recursive(first, hidden);
            set_hidden_recursive(second, hidden);
        }
    }
}

/// A pane exported from the split tree for session saving.
pub struct ExportedPane {
    /// The split that created this pane (None for the first pane).
    pub split: Option<(Direction, f64)>, // (direction, ratio)
}

/// In-order walk: first child inherits parent's split context, second child
/// records the split that separated it from first.
fn export_node(
    node: &Node,
    split_info: Option<(Direction, f64)>,
    out: &mut Vec<ExportedPane>,
) {
    match node {
        Node::Leaf { .. } => {
            out.push(ExportedPane { split: split_info });
        }
        Node::Split { direction, ratio, first, second } => {
            // First child: inherits whatever split_info was passed to us
            export_node(first, split_info, out);
            // Second child: was created by THIS split
            export_node(second, Some((*direction, *ratio)), out);
        }
    }
}
