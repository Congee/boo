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
        let focused = self.focused_id;
        let new_id = self.next_id;
        self.next_id += 1;

        let root = self.root.take()?;
        self.root = Some(split_node(root, focused, direction, new_id, surface, nsview));
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

    /// Get the focused leaf ID.
    pub fn focused_id(&self) -> LeafId {
        self.focused_id
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

    pub fn is_empty(&self) -> bool {
        self.root.is_none()
    }

    /// Show or hide all NSViews in this tree.
    pub fn set_hidden(&self, hidden: bool) {
        if let Some(ref root) = self.root {
            set_hidden_recursive(root, hidden);
        }
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

fn split_node(
    node: Node,
    target_id: LeafId,
    direction: Direction,
    new_id: LeafId,
    surface: ffi::ghostty_surface_t,
    nsview: *mut c_void,
) -> Node {
    match node {
        Node::Leaf { id, .. } if id == target_id => {
            let new_leaf = Node::Leaf { id: new_id, surface, nsview };
            Node::Split {
                direction,
                ratio: 0.5,
                first: Box::new(node),
                second: Box::new(new_leaf),
            }
        }
        Node::Split { direction: d, ratio, first, second } => {
            Node::Split {
                direction: d,
                ratio,
                first: Box::new(split_node(*first, target_id, direction, new_id, surface, nsview)),
                second: Box::new(split_node(*second, target_id, direction, new_id, surface, nsview)),
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

fn split_frame(frame: NSRect, direction: Direction, ratio: f64) -> (NSRect, NSRect) {
    match direction {
        Direction::Horizontal => {
            let w1 = frame.size.width * ratio;
            let w2 = frame.size.width - w1;
            (
                NSRect::new(frame.origin, NSSize::new(w1, frame.size.height)),
                NSRect::new(
                    NSPoint::new(frame.origin.x + w1, frame.origin.y),
                    NSSize::new(w2, frame.size.height),
                ),
            )
        }
        Direction::Vertical => {
            // NSView origin is bottom-left; first child at top, second at bottom
            let h1 = frame.size.height * ratio;
            let h2 = frame.size.height - h1;
            (
                NSRect::new(
                    NSPoint::new(frame.origin.x, frame.origin.y + h2),
                    NSSize::new(frame.size.width, h1),
                ),
                NSRect::new(frame.origin, NSSize::new(frame.size.width, h2)),
            )
        }
    }
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
