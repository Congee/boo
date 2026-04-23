//! Binary split tree for managing terminal pane layout.
//!
//! The tree has two node types:
//! - Leaf: a single terminal pane
//! - Split: two children with a direction and ratio
//!
//! The tree is used to compute view frames when the window resizes.

use crate::pane::PaneHandle;
use crate::platform::{Point, Rect, Size};

/// Unique ID for a leaf node (pane).
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
        pane: PaneHandle,
    },
    Split {
        direction: Direction,
        ratio: f64,
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
        Self {
            root: None,
            next_id: 0,
            focused_id: 0,
        }
    }

    /// Add the first pane (becomes root leaf).
    pub fn add_root(&mut self, pane: PaneHandle) -> LeafId {
        let id = self.next_id;
        self.next_id += 1;
        self.root = Some(Node::Leaf { id, pane });
        self.focused_id = id;
        id
    }

    /// Split the focused leaf in the given direction. Returns the new leaf's ID.
    pub fn split_focused(&mut self, direction: Direction, pane: PaneHandle) -> Option<LeafId> {
        self.split_focused_with_ratio(direction, pane, 0.5)
    }

    /// Split the focused leaf with a specific ratio.
    pub fn split_focused_with_ratio(
        &mut self,
        direction: Direction,
        pane: PaneHandle,
        ratio: f64,
    ) -> Option<LeafId> {
        let focused = self.focused_id;
        let new_id = self.next_id;
        self.next_id += 1;

        let root = self.root.take()?;
        self.root = Some(split_node_with_ratio(
            root, focused, direction, new_id, pane, ratio,
        ));
        self.focused_id = new_id;
        Some(new_id)
    }

    /// Get the focused pane.
    pub fn focused_pane(&self) -> PaneHandle {
        self.root
            .as_ref()
            .and_then(|r| find_leaf(r, self.focused_id))
            .unwrap_or(PaneHandle::null())
    }

    /// Set focus to a specific leaf.
    pub fn set_focus(&mut self, id: LeafId) {
        self.focused_id = id;
    }

    pub fn set_focus_to_pane(&mut self, pane_id: crate::pane::PaneId) -> bool {
        let Some(root) = self.root.as_ref() else {
            return false;
        };
        let Some(id) = find_leaf_id_by_pane(root, pane_id) else {
            return false;
        };
        self.focused_id = id;
        true
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

    /// Lay out all panes within the given frame and return pane sizes.
    pub fn layout(&self, frame: Rect, scale: f64) -> Vec<(PaneHandle, u32, u32)> {
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

    /// Export the tree as a flat list of panes with split info and frames.
    pub fn export_panes_with_frames(&self, frame: Rect) -> Vec<ExportedPane> {
        let mut panes = Vec::new();
        if let Some(ref root) = self.root {
            export_node_with_frame(root, frame, None, &mut panes);
        }
        panes
    }

    /// Collect all panes for cleanup.
    pub fn all_panes(&self) -> Vec<PaneHandle> {
        match &self.root {
            Some(root) => {
                let mut panes = Vec::new();
                collect_panes(root, &mut panes);
                panes
            }
            None => Vec::new(),
        }
    }

    /// Number of leaves.
    pub fn len(&self) -> usize {
        self.root.as_ref().map(count_leaves).unwrap_or(0)
    }

    /// Show or hide all native views in this tree.
    pub fn set_hidden(&self, hidden: bool) {
        if let Some(ref root) = self.root {
            set_hidden_recursive(root, hidden);
        }
    }

    /// Remove the focused leaf from the tree.
    /// Returns the removed pane, or None if not found.
    pub fn remove_focused(&mut self) -> Option<PaneHandle> {
        let root = self.root.take()?;
        match remove_leaf(root, self.focused_id) {
            RemoveResult::Removed { pane, remaining } => {
                self.root = remaining;
                if let Some(ref root) = self.root {
                    let leaves = collect_leaf_ids(root);
                    if !leaves.is_empty() {
                        self.focused_id = leaves[0];
                    }
                }
                Some(pane)
            }
            RemoveResult::NotFound(node) => {
                self.root = Some(node);
                None
            }
        }
    }

    pub fn remove_pane(&mut self, pane_id: crate::pane::PaneId) -> Option<PaneHandle> {
        let root = self.root.take()?;
        let Some(target_id) = find_leaf_id_by_pane(&root, pane_id) else {
            self.root = Some(root);
            return None;
        };
        match remove_leaf(root, target_id) {
            RemoveResult::Removed { pane, remaining } => {
                self.root = remaining;
                if let Some(ref root) = self.root {
                    let leaves = collect_leaf_ids(root);
                    if !leaves.is_empty() {
                        self.focused_id = leaves[0];
                    }
                }
                Some(pane)
            }
            RemoveResult::NotFound(node) => {
                self.root = Some(node);
                None
            }
        }
    }

    /// Resize the nearest matching split ancestor of the focused leaf.
    pub fn resize_focused_by_cells(
        &mut self,
        frame: Rect,
        axis: Direction,
        delta_cells: i32,
        cell_extent: f64,
    ) {
        if let Some(ref mut root) = self.root {
            resize_toward_leaf_by_cells(
                root,
                frame,
                self.focused_id,
                axis,
                delta_cells,
                cell_extent,
            );
        }
    }

    pub fn swap_focused_with_adjacent(&mut self, next: bool) -> bool {
        let Some(root) = self.root.as_ref() else {
            return false;
        };
        let leaves = collect_leaf_ids(root);
        let Some(pos) = leaves.iter().position(|&id| id == self.focused_id) else {
            return false;
        };
        if leaves.len() < 2 {
            return false;
        }
        let other_pos = if next {
            (pos + 1) % leaves.len()
        } else {
            (pos + leaves.len() - 1) % leaves.len()
        };
        let other_id = leaves[other_pos];
        let Some(root) = self.root.as_mut() else {
            return false;
        };
        swap_leaf_panes(root, self.focused_id, other_id)
    }

    pub fn rotate_panes(&mut self, forward: bool) -> bool {
        let Some(root) = self.root.as_ref() else {
            return false;
        };
        let leaves = collect_leaf_ids(root);
        if leaves.len() < 2 {
            return false;
        }
        let panes = leaves
            .iter()
            .filter_map(|id| find_leaf(root, *id))
            .collect::<Vec<_>>();
        if panes.len() != leaves.len() {
            return false;
        }
        let rotated = if forward {
            let mut out = Vec::with_capacity(panes.len());
            out.push(*panes.last().unwrap());
            out.extend(panes.iter().copied().take(panes.len() - 1));
            out
        } else {
            let mut out = panes.iter().copied().skip(1).collect::<Vec<_>>();
            out.push(panes[0]);
            out
        };
        let Some(root) = self.root.as_mut() else {
            return false;
        };
        assign_leaf_panes(root, &leaves, &rotated)
    }

    pub fn rebuild_from_panes(
        &mut self,
        panes: &[PaneHandle],
        splits: &[(Direction, f64)],
        focused_pane_id: crate::pane::PaneId,
    ) -> bool {
        if panes.is_empty() {
            self.root = None;
            return false;
        }
        let mut next_id = 0usize;
        let root_id = next_id;
        next_id += 1;
        let mut root = Node::Leaf {
            id: root_id,
            pane: panes[0],
        };
        for (idx, pane) in panes.iter().copied().enumerate().skip(1) {
            let (direction, ratio) = splits
                .get(idx - 1)
                .copied()
                .unwrap_or((Direction::Vertical, 0.5));
            let new_id = next_id;
            next_id += 1;
            root = split_node_with_ratio(root, new_id - 1, direction, new_id, pane, ratio);
        }
        self.root = Some(root);
        self.next_id = next_id;
        let _ = self.set_focus_to_pane(focused_pane_id);
        if self.focused_pane().id() != focused_pane_id {
            self.focused_id = 0;
        }
        true
    }

    pub fn rebalance(&mut self) -> bool {
        let Some(root) = self.root.as_mut() else {
            return false;
        };
        rebalance_node(root);
        true
    }

    /// Check if a point is on a split divider. Returns the direction if so.
    pub fn divider_at(&self, frame: Rect, point: (f64, f64)) -> Option<Direction> {
        self.root
            .as_ref()
            .and_then(|root| divider_at_node(root, frame, point))
    }

    /// Update the ratio of the split being dragged.
    pub fn resize_drag(&mut self, frame: Rect, dir: Direction, point: (f64, f64)) {
        if let Some(ref mut root) = self.root {
            resize_drag_node(root, frame, dir, point);
        }
    }

    /// Find the leaf at a given point and set focus to it.
    pub fn focus_at(&mut self, frame: Rect, point: (f64, f64)) -> bool {
        if let Some(ref root) = self.root
            && let Some(id) = leaf_at_point(root, frame, point)
            && id != self.focused_id
        {
            self.focused_id = id;
            return true;
        }
        false
    }

    pub fn focus_direction(
        &mut self,
        frame: Rect,
        direction: crate::bindings::PaneFocusDirection,
    ) -> bool {
        let panes = self.export_panes_with_frames(frame);
        let Some(current) = panes
            .iter()
            .find(|pane| pane.leaf_id == self.focused_id)
            .and_then(|pane| pane.frame.map(|frame| (pane.leaf_id, frame)))
        else {
            return false;
        };

        let current_center_x = current.1.origin.x + current.1.size.width / 2.0;
        let current_center_y = current.1.origin.y + current.1.size.height / 2.0;
        let current_min_x = current.1.origin.x;
        let current_max_x = current.1.origin.x + current.1.size.width;
        let current_min_y = current.1.origin.y;
        let current_max_y = current.1.origin.y + current.1.size.height;

        let mut best: Option<(LeafId, f64, f64)> = None;
        for pane in panes {
            if pane.leaf_id == current.0 {
                continue;
            }
            let Some(frame) = pane.frame else {
                continue;
            };
            let center_x = frame.origin.x + frame.size.width / 2.0;
            let center_y = frame.origin.y + frame.size.height / 2.0;
            let min_x = frame.origin.x;
            let max_x = frame.origin.x + frame.size.width;
            let min_y = frame.origin.y;
            let max_y = frame.origin.y + frame.size.height;

            let (primary_distance, secondary_distance) = match direction {
                crate::bindings::PaneFocusDirection::Left => {
                    if max_x > current_min_x {
                        continue;
                    }
                    (
                        current_min_x - max_x,
                        axis_separation(current_min_y, current_max_y, min_y, max_y)
                            .min((center_y - current_center_y).abs()),
                    )
                }
                crate::bindings::PaneFocusDirection::Right => {
                    if min_x < current_max_x {
                        continue;
                    }
                    (
                        min_x - current_max_x,
                        axis_separation(current_min_y, current_max_y, min_y, max_y)
                            .min((center_y - current_center_y).abs()),
                    )
                }
                crate::bindings::PaneFocusDirection::Up => {
                    if max_y > current_min_y {
                        continue;
                    }
                    (
                        current_min_y - max_y,
                        axis_separation(current_min_x, current_max_x, min_x, max_x)
                            .min((center_x - current_center_x).abs()),
                    )
                }
                crate::bindings::PaneFocusDirection::Down => {
                    if min_y < current_max_y {
                        continue;
                    }
                    (
                        min_y - current_max_y,
                        axis_separation(current_min_x, current_max_x, min_x, max_x)
                            .min((center_x - current_center_x).abs()),
                    )
                }
            };

            let candidate = (pane.leaf_id, primary_distance, secondary_distance);
            if best
                .as_ref()
                .map(|best| {
                    candidate.1 < best.1
                        || ((candidate.1 - best.1).abs() < f64::EPSILON && candidate.2 < best.2)
                })
                .unwrap_or(true)
            {
                best = Some(candidate);
            }
        }

        if let Some((leaf_id, _, _)) = best
            && leaf_id != self.focused_id
        {
            self.focused_id = leaf_id;
            return true;
        }
        false
    }

    /// Get surface info for control socket.
    pub fn surface_info(&self) -> Vec<(LeafId, bool)> {
        match &self.root {
            Some(root) => {
                let ids = collect_leaf_ids(root);
                ids.into_iter()
                    .map(|id| (id, id == self.focused_id))
                    .collect()
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
    pane: PaneHandle,
    ratio: f64,
) -> Node {
    match node {
        Node::Leaf { id, .. } if id == target_id => {
            let new_leaf = Node::Leaf { id: new_id, pane };
            Node::Split {
                direction,
                ratio: ratio.clamp(0.1, 0.9),
                first: Box::new(node),
                second: Box::new(new_leaf),
            }
        }
        Node::Split {
            direction: existing_direction,
            ratio: existing_ratio,
            first,
            second,
        } => Node::Split {
            direction: existing_direction,
            ratio: existing_ratio,
            first: Box::new(split_node_with_ratio(
                *first, target_id, direction, new_id, pane, ratio,
            )),
            second: Box::new(split_node_with_ratio(
                *second, target_id, direction, new_id, pane, ratio,
            )),
        },
        other => other,
    }
}

fn find_leaf(node: &Node, id: LeafId) -> Option<PaneHandle> {
    match node {
        Node::Leaf { id: leaf_id, pane } if *leaf_id == id => Some(*pane),
        Node::Split { first, second, .. } => find_leaf(first, id).or_else(|| find_leaf(second, id)),
        _ => None,
    }
}

fn find_leaf_id_by_pane(node: &Node, pane_id: crate::pane::PaneId) -> Option<LeafId> {
    match node {
        Node::Leaf { id, pane } if pane.id() == pane_id => Some(*id),
        Node::Split { first, second, .. } => {
            find_leaf_id_by_pane(first, pane_id).or_else(|| find_leaf_id_by_pane(second, pane_id))
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

fn collect_panes(node: &Node, out: &mut Vec<PaneHandle>) {
    match node {
        Node::Leaf { pane, .. } => out.push(*pane),
        Node::Split { first, second, .. } => {
            collect_panes(first, out);
            collect_panes(second, out);
        }
    }
}

fn swap_leaf_panes(node: &mut Node, a: LeafId, b: LeafId) -> bool {
    let Some(first) = find_leaf(node, a) else {
        return false;
    };
    let Some(second) = find_leaf(node, b) else {
        return false;
    };
    set_leaf_pane(node, a, second) && set_leaf_pane(node, b, first)
}

fn assign_leaf_panes(node: &mut Node, leaf_ids: &[LeafId], panes: &[PaneHandle]) -> bool {
    if leaf_ids.len() != panes.len() {
        return false;
    }
    let mut assigned = 0usize;
    assign_leaf_panes_inner(node, leaf_ids, panes, &mut assigned);
    assigned == panes.len()
}

fn set_leaf_pane(node: &mut Node, target: LeafId, new_pane: PaneHandle) -> bool {
    match node {
        Node::Leaf { id, pane } if *id == target => {
            *pane = new_pane;
            true
        }
        Node::Split { first, second, .. } => {
            set_leaf_pane(first, target, new_pane) || set_leaf_pane(second, target, new_pane)
        }
        _ => false,
    }
}

fn assign_leaf_panes_inner(
    node: &mut Node,
    leaf_ids: &[LeafId],
    panes: &[PaneHandle],
    assigned: &mut usize,
) {
    match node {
        Node::Leaf { id, pane } => {
            if let Some(index) = leaf_ids.iter().position(|leaf_id| leaf_id == id) {
                *pane = panes[index];
                *assigned += 1;
            }
        }
        Node::Split { first, second, .. } => {
            assign_leaf_panes_inner(first, leaf_ids, panes, assigned);
            assign_leaf_panes_inner(second, leaf_ids, panes, assigned);
        }
    }
}

fn rebalance_node(node: &mut Node) {
    match node {
        Node::Leaf { .. } => {}
        Node::Split {
            ratio,
            first,
            second,
            ..
        } => {
            *ratio = 0.5;
            rebalance_node(first);
            rebalance_node(second);
        }
    }
}

fn count_leaves(node: &Node) -> usize {
    match node {
        Node::Leaf { .. } => 1,
        Node::Split { first, second, .. } => count_leaves(first) + count_leaves(second),
    }
}

fn layout_node(node: &Node, frame: Rect, scale: f64, out: &mut Vec<(PaneHandle, u32, u32)>) {
    match node {
        Node::Leaf { pane, .. } => {
            crate::platform::set_view_frame(pane.view(), frame);
            let w = (frame.size.width * scale) as u32;
            let h = (frame.size.height * scale) as u32;
            out.push((*pane, w, h));
        }
        Node::Split {
            direction,
            ratio,
            first,
            second,
        } => {
            let (first_frame, second_frame) = split_frame(frame, *direction, *ratio);
            layout_node(first, first_frame, scale, out);
            layout_node(second, second_frame, scale, out);
        }
    }
}

/// Gap between split panes (points). The iced background shows through.
const SPLIT_BORDER: f64 = 1.0;

fn split_frame(frame: Rect, direction: Direction, ratio: f64) -> (Rect, Rect) {
    match direction {
        Direction::Horizontal => {
            let usable = frame.size.width - SPLIT_BORDER;
            let w1 = usable * ratio;
            let w2 = usable - w1;
            (
                Rect::new(frame.origin, Size::new(w1, frame.size.height)),
                Rect::new(
                    Point::new(frame.origin.x + w1 + SPLIT_BORDER, frame.origin.y),
                    Size::new(w2, frame.size.height),
                ),
            )
        }
        Direction::Vertical => {
            let usable = frame.size.height - SPLIT_BORDER;
            let h1 = usable * ratio;
            let h2 = usable - h1;
            (
                Rect::new(frame.origin, Size::new(frame.size.width, h1)),
                Rect::new(
                    Point::new(frame.origin.x, frame.origin.y + h1 + SPLIT_BORDER),
                    Size::new(frame.size.width, h2),
                ),
            )
        }
    }
}

enum RemoveResult {
    Removed {
        pane: PaneHandle,
        remaining: Option<Node>,
    },
    NotFound(Node),
}

fn remove_leaf(node: Node, target_id: LeafId) -> RemoveResult {
    match node {
        Node::Leaf { id, pane } if id == target_id => RemoveResult::Removed {
            pane,
            remaining: None,
        },
        Node::Split {
            direction,
            ratio,
            first,
            second,
        } => match remove_leaf(*first, target_id) {
            RemoveResult::Removed {
                pane,
                remaining: Some(remaining_first),
            } => RemoveResult::Removed {
                pane,
                remaining: Some(Node::Split {
                    direction,
                    ratio,
                    first: Box::new(remaining_first),
                    second,
                }),
            },
            RemoveResult::Removed {
                pane,
                remaining: None,
            } => RemoveResult::Removed {
                pane,
                remaining: Some(*second),
            },
            RemoveResult::NotFound(first_node) => match remove_leaf(*second, target_id) {
                RemoveResult::Removed {
                    pane,
                    remaining: Some(remaining_second),
                } => RemoveResult::Removed {
                    pane,
                    remaining: Some(Node::Split {
                        direction,
                        ratio,
                        first: Box::new(first_node),
                        second: Box::new(remaining_second),
                    }),
                },
                RemoveResult::Removed {
                    pane,
                    remaining: None,
                } => RemoveResult::Removed {
                    pane,
                    remaining: Some(first_node),
                },
                RemoveResult::NotFound(second_node) => RemoveResult::NotFound(Node::Split {
                    direction,
                    ratio,
                    first: Box::new(first_node),
                    second: Box::new(second_node),
                }),
            },
        },
        other => RemoveResult::NotFound(other),
    }
}

fn resize_toward_leaf_by_cells(
    node: &mut Node,
    frame: Rect,
    target: LeafId,
    axis: Direction,
    delta_cells: i32,
    cell_extent: f64,
) -> (bool, bool) {
    match node {
        Node::Leaf { id, .. } => (*id == target, false),
        Node::Split {
            direction,
            ratio,
            first,
            second,
        } => {
            let (first_frame, second_frame) = split_frame(frame, *direction, *ratio);
            let (found, done) = resize_toward_leaf_by_cells(
                first,
                first_frame,
                target,
                axis,
                delta_cells,
                cell_extent,
            );
            if found && done {
                return (true, true);
            }

            let (found, done) = if found {
                (true, false)
            } else {
                resize_toward_leaf_by_cells(
                    second,
                    second_frame,
                    target,
                    axis,
                    delta_cells,
                    cell_extent,
                )
            };

            if !found {
                return (false, false);
            }
            if done {
                return (true, true);
            }

            if *direction == axis {
                let usable_extent = match axis {
                    Direction::Horizontal => (frame.size.width - SPLIT_BORDER).max(1.0),
                    Direction::Vertical => (frame.size.height - SPLIT_BORDER).max(1.0),
                };
                let delta = (delta_cells as f64 * cell_extent) / usable_extent;
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

fn divider_at_node(node: &Node, frame: Rect, point: (f64, f64)) -> Option<Direction> {
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
                    let div_y = first_frame.origin.y + first_frame.size.height + SPLIT_BORDER / 2.0;
                    (point.1 - div_y).abs() < half
                        && point.0 >= frame.origin.x
                        && point.0 <= frame.origin.x + frame.size.width
                }
            };

            if on_divider {
                Some(*direction)
            } else {
                divider_at_node(first, first_frame, point)
                    .or_else(|| divider_at_node(second, second_frame, point))
            }
        }
    }
}

fn resize_drag_node(node: &mut Node, frame: Rect, dir: Direction, point: (f64, f64)) -> bool {
    match node {
        Node::Leaf { .. } => false,
        Node::Split {
            direction,
            ratio,
            first,
            second,
        } => {
            let (first_frame, second_frame) = split_frame(frame, *direction, *ratio);

            if *direction == dir {
                match dir {
                    Direction::Horizontal => {
                        if point.1 >= frame.origin.y
                            && point.1 <= frame.origin.y + frame.size.height
                        {
                            let rel = (point.0 - frame.origin.x) / frame.size.width.max(1.0);
                            *ratio = rel.clamp(0.1, 0.9);
                            return true;
                        }
                    }
                    Direction::Vertical => {
                        if point.0 >= frame.origin.x && point.0 <= frame.origin.x + frame.size.width
                        {
                            let rel = (point.1 - frame.origin.y) / frame.size.height.max(1.0);
                            *ratio = rel.clamp(0.1, 0.9);
                            return true;
                        }
                    }
                }
            }

            resize_drag_node(first, first_frame, dir, point)
                || resize_drag_node(second, second_frame, dir, point)
        }
    }
}

fn leaf_at_point(node: &Node, frame: Rect, point: (f64, f64)) -> Option<LeafId> {
    match node {
        Node::Leaf { id, .. } => {
            let inside_x =
                point.0 >= frame.origin.x && point.0 <= frame.origin.x + frame.size.width;
            let inside_y =
                point.1 >= frame.origin.y && point.1 <= frame.origin.y + frame.size.height;
            if inside_x && inside_y {
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
        } => {
            let (first_frame, second_frame) = split_frame(frame, *direction, *ratio);
            leaf_at_point(first, first_frame, point)
                .or_else(|| leaf_at_point(second, second_frame, point))
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ExportedPane {
    pub leaf_id: LeafId,
    #[allow(dead_code)]
    pub pane: PaneHandle,
    pub frame: Option<Rect>,
    pub split: Option<(Direction, f64)>,
}

fn export_node(node: &Node, parent_split: Option<(Direction, f64)>, out: &mut Vec<ExportedPane>) {
    match node {
        Node::Leaf { id, pane } => out.push(ExportedPane {
            leaf_id: *id,
            pane: *pane,
            frame: None,
            split: parent_split,
        }),
        Node::Split {
            direction,
            ratio,
            first,
            second,
        } => {
            export_node(first, Some((*direction, *ratio)), out);
            export_node(second, Some((*direction, *ratio)), out);
        }
    }
}

fn export_node_with_frame(
    node: &Node,
    frame: Rect,
    parent_split: Option<(Direction, f64)>,
    out: &mut Vec<ExportedPane>,
) {
    match node {
        Node::Leaf { id, pane } => out.push(ExportedPane {
            leaf_id: *id,
            pane: *pane,
            frame: Some(frame),
            split: parent_split,
        }),
        Node::Split {
            direction,
            ratio,
            first,
            second,
        } => {
            let (first_frame, second_frame) = split_frame(frame, *direction, *ratio);
            export_node_with_frame(first, first_frame, Some((*direction, *ratio)), out);
            export_node_with_frame(second, second_frame, Some((*direction, *ratio)), out);
        }
    }
}

fn axis_separation(a_min: f64, a_max: f64, b_min: f64, b_max: f64) -> f64 {
    if a_max < b_min {
        b_min - a_max
    } else if b_max < a_min {
        a_min - b_max
    } else {
        0.0
    }
}

fn set_hidden_recursive(node: &Node, hidden: bool) {
    match node {
        Node::Leaf { pane, .. } => crate::platform::set_view_hidden(pane.view(), hidden),
        Node::Split { first, second, .. } => {
            set_hidden_recursive(first, hidden);
            set_hidden_recursive(second, hidden);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Direction, SPLIT_BORDER, SplitTree, split_frame};
    use crate::pane::PaneHandle;
    use crate::platform::{Point, Rect, Size};

    #[test]
    fn split_focused_adds_new_leaf_and_focuses_it() {
        let mut tree = SplitTree::new();
        let root = PaneHandle::detached();
        let split = PaneHandle::detached();

        tree.add_root(root);
        let new_id = tree
            .split_focused(Direction::Horizontal, split)
            .expect("split should succeed");

        assert_eq!(tree.len(), 2);
        assert_eq!(tree.focused_pane(), split);

        let info = tree.surface_info();
        assert_eq!(info.len(), 2);
        assert!(info.iter().any(|(id, focused)| *id == new_id && *focused));
    }

    #[test]
    fn remove_focused_promotes_remaining_leaf() {
        let mut tree = SplitTree::new();
        let root = PaneHandle::detached();
        let split = PaneHandle::detached();

        tree.add_root(root);
        tree.split_focused(Direction::Vertical, split);

        let removed = tree
            .remove_focused()
            .expect("focused leaf should be removed");

        assert_eq!(removed, split);
        assert_eq!(tree.len(), 1);
        assert_eq!(tree.focused_pane(), root);
    }

    #[test]
    fn focus_navigation_cycles_through_leaves() {
        let mut tree = SplitTree::new();
        let a = PaneHandle::detached();
        let b = PaneHandle::detached();
        let c = PaneHandle::detached();

        tree.add_root(a);
        tree.split_focused(Direction::Horizontal, b);
        tree.split_focused(Direction::Vertical, c);

        assert_eq!(tree.focused_pane(), c);
        tree.focus_next();
        assert_eq!(tree.focused_pane(), a);
        tree.focus_prev();
        assert_eq!(tree.focused_pane(), c);
    }

    #[test]
    fn swap_focused_with_adjacent_swaps_panes() {
        let mut tree = SplitTree::new();
        let a = PaneHandle::detached();
        let b = PaneHandle::detached();

        tree.add_root(a);
        tree.split_focused(Direction::Horizontal, b);

        assert!(tree.swap_focused_with_adjacent(false));
        assert_eq!(tree.all_panes(), vec![b, a]);
        assert_eq!(tree.focused_pane(), a);
    }

    #[test]
    fn rotate_panes_reorders_leaf_contents() {
        let mut tree = SplitTree::new();
        let a = PaneHandle::detached();
        let b = PaneHandle::detached();
        let c = PaneHandle::detached();

        tree.add_root(a);
        tree.split_focused(Direction::Horizontal, b);
        tree.split_focused(Direction::Vertical, c);

        assert!(tree.rotate_panes(true));
        assert_eq!(tree.all_panes(), vec![c, a, b]);
    }

    #[test]
    fn remove_pane_by_id_removes_nonfocused_leaf() {
        let mut tree = SplitTree::new();
        let a = PaneHandle::detached();
        let b = PaneHandle::detached();

        tree.add_root(a);
        tree.split_focused(Direction::Horizontal, b);

        let removed = tree.remove_pane(a.id()).unwrap();

        assert_eq!(removed, a);
        assert_eq!(tree.len(), 1);
        assert_eq!(tree.focused_pane(), b);
    }

    #[test]
    fn rebalance_sets_nested_split_ratios_to_half() {
        let mut tree = SplitTree::new();
        let a = PaneHandle::detached();
        let b = PaneHandle::detached();
        let c = PaneHandle::detached();

        tree.add_root(a);
        tree.split_focused_with_ratio(Direction::Horizontal, b, 0.8);
        tree.split_focused_with_ratio(Direction::Vertical, c, 0.2);

        assert!(tree.rebalance());
        let exported = tree.export_panes();
        assert!(exported.iter().all(|pane| {
            pane.split
                .map(|(_, ratio)| (ratio - 0.5).abs() < 0.0001)
                .unwrap_or(true)
        }));
    }

    #[test]
    fn split_frame_respects_border_gap() {
        let frame = Rect::new(Point::new(0.0, 0.0), Size::new(100.0, 50.0));
        let (left, right) = split_frame(frame, Direction::Horizontal, 0.5);

        assert_eq!(left.origin.x, 0.0);
        assert_eq!(left.size.width + right.size.width + SPLIT_BORDER, 100.0);
        assert_eq!(right.origin.x, left.size.width + SPLIT_BORDER);
    }

    #[test]
    fn focus_at_selects_leaf_from_point() {
        let mut tree = SplitTree::new();
        let left = PaneHandle::detached();
        let right = PaneHandle::detached();
        let frame = Rect::new(Point::new(0.0, 0.0), Size::new(120.0, 40.0));

        tree.add_root(left);
        tree.split_focused(Direction::Horizontal, right);

        assert!(tree.focus_at(frame, (10.0, 10.0)));
        assert_eq!(tree.focused_pane(), left);
        assert!(tree.focus_at(frame, (110.0, 10.0)));
        assert_eq!(tree.focused_pane(), right);
    }

    #[test]
    fn export_panes_with_frames_preserves_split_geometry() {
        let mut tree = SplitTree::new();
        tree.add_root(PaneHandle::detached());
        tree.split_focused(Direction::Horizontal, PaneHandle::detached());

        let frame = Rect::new(Point::new(0.0, 20.0), Size::new(120.0, 40.0));
        let panes = tree.export_panes_with_frames(frame);

        assert_eq!(panes.len(), 2);
        assert_eq!(panes[0].frame.unwrap().origin.x, 0.0);
        assert_eq!(panes[0].frame.unwrap().origin.y, 20.0);
        assert_eq!(panes[0].frame.unwrap().size.height, 40.0);
        assert_eq!(panes[1].frame.unwrap().origin.y, 20.0);
        assert_eq!(
            panes[0].frame.unwrap().size.width + panes[1].frame.unwrap().size.width + SPLIT_BORDER,
            120.0
        );
    }

    #[test]
    fn directional_focus_moves_to_adjacent_pane() {
        let mut tree = SplitTree::new();
        let left = PaneHandle::detached();
        let right = PaneHandle::detached();
        let frame = Rect::new(Point::new(0.0, 0.0), Size::new(120.0, 40.0));

        tree.add_root(left);
        tree.split_focused(Direction::Horizontal, right);

        assert_eq!(tree.focused_pane(), right);
        assert!(tree.focus_direction(frame, crate::bindings::PaneFocusDirection::Left));
        assert_eq!(tree.focused_pane(), left);
        assert!(tree.focus_direction(frame, crate::bindings::PaneFocusDirection::Right));
        assert_eq!(tree.focused_pane(), right);
    }

    #[test]
    fn directional_focus_prefers_nearest_pane_in_requested_direction() {
        let mut tree = SplitTree::new();
        let top_left = PaneHandle::detached();
        let bottom_left = PaneHandle::detached();
        let right = PaneHandle::detached();
        let frame = Rect::new(Point::new(0.0, 0.0), Size::new(160.0, 80.0));

        tree.add_root(top_left);
        tree.split_focused(Direction::Vertical, bottom_left);
        assert!(tree.focus_direction(frame, crate::bindings::PaneFocusDirection::Up));
        tree.split_focused(Direction::Horizontal, right);

        assert_eq!(tree.focused_pane(), right);
        assert!(tree.focus_direction(frame, crate::bindings::PaneFocusDirection::Left));
        assert_eq!(tree.focused_pane(), top_left);
        assert!(tree.focus_direction(frame, crate::bindings::PaneFocusDirection::Down));
        assert_eq!(tree.focused_pane(), bottom_left);
    }

    #[test]
    fn resize_focused_by_cells_uses_cell_count_not_percent() {
        let mut tree = SplitTree::new();
        let left = PaneHandle::detached();
        let right = PaneHandle::detached();
        let frame = Rect::new(Point::new(0.0, 0.0), Size::new(200.0, 40.0));

        tree.add_root(left);
        tree.split_focused(Direction::Horizontal, right);

        let before = tree.export_panes_with_frames(frame);
        let before_left_width = before[0].frame.unwrap().size.width;

        tree.resize_focused_by_cells(frame, Direction::Horizontal, -2, 10.0);

        let after = tree.export_panes_with_frames(frame);
        let after_left_width = after[0].frame.unwrap().size.width;
        let after_right_width = after[1].frame.unwrap().size.width;

        assert!((after_left_width - (before_left_width - 20.0)).abs() < 1.0);
        assert!((after_right_width - (before[1].frame.unwrap().size.width + 20.0)).abs() < 1.0);
    }
}
