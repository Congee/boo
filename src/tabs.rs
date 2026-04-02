//! Tab management — each tab contains its own split tree.

use crate::ffi;
use crate::splits::SplitTree;
use std::ffi::c_void;

pub struct TabManager {
    tabs: Vec<Tab>,
    active: usize,
    prev_active: usize,
}

pub struct Tab {
    tree: SplitTree,
    pub title: String,
}

impl TabManager {
    pub fn new() -> Self {
        TabManager {
            tabs: Vec::new(),
            active: 0,
            prev_active: 0,
        }
    }

    /// Add the first tab with a root surface.
    pub fn add_initial_tab(
        &mut self,
        surface: ffi::ghostty_surface_t,
        nsview: *mut c_void,
    ) {
        let mut tree = SplitTree::new();
        tree.add_root(surface, nsview);
        self.tabs.push(Tab {
            tree,
            title: String::new(),
        });
    }

    /// Create a new tab with a surface. Returns the new tab index.
    pub fn new_tab(
        &mut self,
        surface: ffi::ghostty_surface_t,
        nsview: *mut c_void,
    ) -> usize {
        // Hide current tab
        if let Some(tab) = self.tabs.get(self.active) {
            tab.tree.set_hidden(true);
        }

        let mut tree = SplitTree::new();
        tree.add_root(surface, nsview);
        let idx = self.tabs.len();
        self.tabs.push(Tab {
            tree,
            title: String::new(),
        });
        self.active = idx;
        idx
    }

    /// Switch to tab at index.
    pub fn goto_tab(&mut self, index: usize) -> bool {
        if index >= self.tabs.len() || index == self.active {
            return false;
        }
        self.tabs[self.active].tree.set_hidden(true);
        self.prev_active = self.active;
        self.active = index;
        self.tabs[self.active].tree.set_hidden(false);
        true
    }

    /// Switch to next tab.
    pub fn next_tab(&mut self) -> bool {
        if self.tabs.len() <= 1 {
            return false;
        }
        let next = (self.active + 1) % self.tabs.len();
        self.goto_tab(next)
    }

    /// Switch to previous tab.
    pub fn prev_tab(&mut self) -> bool {
        if self.tabs.len() <= 1 {
            return false;
        }
        let prev = if self.active == 0 {
            self.tabs.len() - 1
        } else {
            self.active - 1
        };
        self.goto_tab(prev)
    }

    /// Remove a tab by index. Returns surfaces for cleanup.
    pub fn remove_tab(&mut self, index: usize) -> Vec<ffi::ghostty_surface_t> {
        if index >= self.tabs.len() {
            return Vec::new();
        }
        let tab = self.tabs.remove(index);
        tab.tree.set_hidden(true);
        let surfaces = tab.tree.all_surfaces();

        // Adjust active index
        if self.tabs.is_empty() {
            self.active = 0;
        } else if self.active >= self.tabs.len() {
            self.active = self.tabs.len() - 1;
        } else if self.active > index {
            self.active -= 1;
        }

        // Show new active tab
        if let Some(tab) = self.tabs.get(self.active) {
            tab.tree.set_hidden(false);
        }

        surfaces
    }

    /// Get the active split tree.
    pub fn active_tree(&self) -> Option<&SplitTree> {
        self.tabs.get(self.active).map(|t| &t.tree)
    }

    /// Get the active split tree mutably.
    pub fn active_tree_mut(&mut self) -> Option<&mut SplitTree> {
        self.tabs.get_mut(self.active).map(|t| &mut t.tree)
    }

    /// Get the focused surface of the active tab.
    pub fn focused_surface(&self) -> ffi::ghostty_surface_t {
        self.active_tree()
            .map(|t| t.focused_surface())
            .unwrap_or(std::ptr::null_mut())
    }

    /// Number of tabs.
    pub fn len(&self) -> usize {
        self.tabs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    pub fn previous_active(&self) -> usize {
        self.prev_active.min(self.tabs.len().saturating_sub(1))
    }

    pub fn active_index(&self) -> usize {
        self.active
    }

    /// Set title for the active tab.
    pub fn set_active_title(&mut self, title: String) {
        if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.title = title;
        }
    }

    /// Get mutable access to a tab by index.
    pub fn tab_mut(&mut self, index: usize) -> Option<&mut Tab> {
        self.tabs.get_mut(index)
    }

    /// Get the tree for a tab by index.
    pub fn tab_tree(&self, index: usize) -> Option<&SplitTree> {
        self.tabs.get(index).map(|t| &t.tree)
    }

    /// Get info for control socket.
    pub fn tab_info(&self) -> Vec<TabInfo> {
        self.tabs
            .iter()
            .enumerate()
            .map(|(i, tab)| TabInfo {
                index: i,
                active: i == self.active,
                title: tab.title.clone(),
                surfaces: tab.tree.len(),
            })
            .collect()
    }

    /// Collect all surfaces across all tabs for cleanup.
    pub fn all_surfaces(&self) -> Vec<ffi::ghostty_surface_t> {
        self.tabs
            .iter()
            .flat_map(|t| t.tree.all_surfaces())
            .collect()
    }

    /// Relay layout to active tab.
    pub fn layout_active(
        &self,
        frame: objc2_foundation::NSRect,
        scale: f64,
    ) -> Vec<(ffi::ghostty_surface_t, u32, u32)> {
        self.active_tree()
            .map(|t| t.layout(frame, scale))
            .unwrap_or_default()
    }
}

#[derive(Debug, serde::Serialize)]
pub struct TabInfo {
    pub index: usize,
    pub active: bool,
    pub title: String,
    pub surfaces: usize,
}
