//! Tab management — each tab contains its own split tree.

use crate::pane::PaneHandle;
use crate::session::TabLayout;
use crate::splits::SplitTree;

pub struct TabManager {
    tabs: Vec<Tab>,
    active: usize,
    prev_active: usize,
    next_tab_id: u32,
}

pub struct Tab {
    id: u32,
    tree: SplitTree,
    pub title: String,
    pub layout: TabLayout,
    running_command: Option<RunningCommand>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct RunningCommand {
    pub command: Option<String>,
}

#[derive(Clone)]
pub struct TabIdentityInfo {
    pub id: u32,
    pub index: usize,
    pub title: String,
}

impl TabManager {
    pub fn new() -> Self {
        Self {
            tabs: Vec::new(),
            active: 0,
            prev_active: 0,
            next_tab_id: 1,
        }
    }

    fn allocate_tab_id(&mut self) -> u32 {
        let id = self.next_tab_id;
        self.next_tab_id += 1;
        id
    }

    /// Add the first tab with a root pane.
    pub fn add_initial_tab(&mut self, pane: PaneHandle) {
        let mut tree = SplitTree::new();
        tree.add_root(pane);
        let id = self.allocate_tab_id();
        self.tabs.push(Tab {
            id,
            tree,
            title: String::new(),
            layout: TabLayout::Manual,
            running_command: None,
        });
    }

    /// Create a new tab with a root pane. Returns the new tab index.
    pub fn new_tab(&mut self, pane: PaneHandle) -> usize {
        if let Some(tab) = self.tabs.get(self.active) {
            tab.tree.set_hidden(true);
        }

        let mut tree = SplitTree::new();
        tree.add_root(pane);
        let idx = self.tabs.len();
        let id = self.allocate_tab_id();
        self.tabs.push(Tab {
            id,
            tree,
            title: String::new(),
            layout: TabLayout::Manual,
            running_command: None,
        });
        self.active = idx;
        idx
    }

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

    pub fn next_tab(&mut self) -> bool {
        if self.tabs.len() <= 1 {
            return false;
        }
        let next = (self.active + 1) % self.tabs.len();
        self.goto_tab(next)
    }

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

    /// Remove a tab by index. Returns panes for cleanup.
    pub fn remove_tab(&mut self, index: usize) -> Vec<PaneHandle> {
        if index >= self.tabs.len() {
            return Vec::new();
        }
        let tab = self.tabs.remove(index);
        tab.tree.set_hidden(true);
        let panes = tab.tree.all_panes();

        if self.tabs.is_empty() {
            self.active = 0;
        } else if self.active >= self.tabs.len() {
            self.active = self.tabs.len() - 1;
        } else if self.active > index {
            self.active -= 1;
        }

        if let Some(tab) = self.tabs.get(self.active) {
            tab.tree.set_hidden(false);
        }

        panes
    }

    pub fn active_tree(&self) -> Option<&SplitTree> {
        self.tabs.get(self.active).map(|t| &t.tree)
    }

    pub fn active_tree_mut(&mut self) -> Option<&mut SplitTree> {
        self.tabs.get_mut(self.active).map(|t| &mut t.tree)
    }

    pub fn focused_pane(&self) -> PaneHandle {
        self.active_tree()
            .map(|t| t.focused_pane())
            .unwrap_or(PaneHandle::null())
    }

    pub fn focus_active_pane_by_id(&mut self, pane_id: crate::pane::PaneId) -> bool {
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return false;
        };
        tab.tree.set_focus_to_pane(pane_id)
    }

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

    pub fn active_tab_id(&self) -> Option<u32> {
        self.tabs.get(self.active).map(|tab| tab.id)
    }

    pub fn set_active_title(&mut self, title: String) {
        if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.title = title;
        }
    }

    pub fn active_layout(&self) -> Option<TabLayout> {
        self.tabs.get(self.active).map(|tab| tab.layout.clone())
    }

    pub fn apply_layout_to_active(&mut self, layout: TabLayout) -> bool {
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return false;
        };
        let panes = tab.tree.all_panes();
        if panes.len() <= 1 {
            tab.layout = layout;
            return false;
        }
        let focused_pane_id = tab.tree.focused_pane().id();
        let specs = crate::session::layout_splits(&layout, panes.len())
            .into_iter()
            .map(|spec| {
                let direction = match spec.direction {
                    crate::session::SplitDir::Right => crate::splits::Direction::Horizontal,
                    crate::session::SplitDir::Down => crate::splits::Direction::Vertical,
                };
                (direction, spec.ratio)
            })
            .collect::<Vec<_>>();
        let changed = tab.tree.rebuild_from_panes(&panes, &specs, focused_pane_id);
        tab.layout = layout;
        changed
    }

    pub fn cycle_active_layout(&mut self, forward: bool) -> bool {
        let current = self.active_layout().unwrap_or(TabLayout::Manual);
        let layouts = [
            TabLayout::EvenHorizontal,
            TabLayout::EvenVertical,
            TabLayout::MainHorizontal,
            TabLayout::MainVertical,
            TabLayout::Tiled,
        ];
        let pos = layouts
            .iter()
            .position(|layout| *layout == current)
            .unwrap_or(0);
        let next = if forward {
            layouts[(pos + 1) % layouts.len()].clone()
        } else {
            layouts[(pos + layouts.len() - 1) % layouts.len()].clone()
        };
        self.apply_layout_to_active(next)
    }

    pub fn rotate_active_panes(&mut self, forward: bool) -> bool {
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return false;
        };
        tab.layout = TabLayout::Manual;
        tab.tree.rotate_panes(forward)
    }

    pub fn swap_active_pane_with_adjacent(&mut self, next: bool) -> bool {
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return false;
        };
        tab.layout = TabLayout::Manual;
        tab.tree.swap_focused_with_adjacent(next)
    }

    pub fn break_active_pane_to_tab(&mut self) -> Option<usize> {
        if self.tabs.is_empty() {
            return None;
        }
        let active_index = self.active;
        let pane = self.tabs.get_mut(active_index)?.tree.remove_focused()?;
        self.tabs[active_index].layout = TabLayout::Manual;
        if self.tabs[active_index].tree.len() == 0 {
            let _ = self.remove_tab(active_index);
        }
        Some(self.new_tab(pane))
    }

    pub fn rebalance_active_layout(&mut self) -> bool {
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return false;
        };
        tab.tree.rebalance()
    }

    pub fn active_title(&self) -> Option<&str> {
        self.tabs.get(self.active).map(|tab| tab.title.as_str())
    }

    pub fn set_running_command_for_pane(
        &mut self,
        pane_id: crate::pane::PaneId,
        running_command: Option<RunningCommand>,
    ) -> bool {
        let mut changed = false;
        for tab in &mut self.tabs {
            if tab.tree.focused_pane().id() == pane_id {
                if tab.running_command != running_command {
                    tab.running_command = running_command.clone();
                    changed = true;
                }
            }
        }
        changed
    }

    pub fn display_title(&self, index: usize, spinner: Option<&str>) -> String {
        let Some(tab) = self.tabs.get(index) else {
            return String::new();
        };

        let base = if let Some(command) = tab
            .running_command
            .as_ref()
            .and_then(|running| running.command.as_deref())
            .filter(|command| !command.is_empty())
        {
            command.to_string()
        } else {
            tab.title.clone()
        };

        match spinner {
            Some(spinner) if !spinner.is_empty() => {
                if base.is_empty() {
                    spinner.to_string()
                } else {
                    format!("{spinner} {base}")
                }
            }
            _ => base,
        }
    }

    pub fn tab_info_with_spinner(&self, spinner_frame: usize) -> Vec<TabInfo> {
        self.tabs
            .iter()
            .enumerate()
            .map(|(i, tab)| TabInfo {
                index: i,
                active: i == self.active,
                title: self.display_title(
                    i,
                    tab.running_command
                        .as_ref()
                        .map(|_| spinner_for(spinner_frame)),
                ),
                surfaces: tab.tree.len(),
            })
            .collect()
    }

    pub fn tab_mut(&mut self, index: usize) -> Option<&mut Tab> {
        self.tabs.get_mut(index)
    }

    pub fn tab_tree(&self, index: usize) -> Option<&SplitTree> {
        self.tabs.get(index).map(|t| &t.tree)
    }

    pub fn tab_layout(&self, index: usize) -> Option<TabLayout> {
        self.tabs.get(index).map(|tab| tab.layout.clone())
    }

    pub fn tab_identity_info(&self) -> Vec<TabIdentityInfo> {
        self.tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| TabIdentityInfo {
                id: tab.id,
                index,
                title: self.display_title(index, None),
            })
            .collect()
    }

    pub fn tab_id_for_index(&self, index: usize) -> Option<u32> {
        self.tabs.get(index).map(|tab| tab.id)
    }

    pub fn find_index_by_tab_id(&self, tab_id: u32) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.id == tab_id)
    }

    pub fn find_pane_location(
        &self,
        pane_id: crate::pane::PaneId,
    ) -> Option<(usize, crate::splits::LeafId)> {
        self.tabs.iter().enumerate().find_map(|(tab_index, tab)| {
            tab.tree
                .export_panes()
                .into_iter()
                .find(|pane| pane.pane.id() == pane_id)
                .map(|pane| (tab_index, pane.leaf_id))
        })
    }

    pub fn remove_pane_by_id(&mut self, pane_id: crate::pane::PaneId) -> Option<PaneHandle> {
        let (tab_index, _) = self.find_pane_location(pane_id)?;
        let pane = self.tabs.get_mut(tab_index)?.tree.remove_pane(pane_id)?;
        self.tabs[tab_index].layout = TabLayout::Manual;
        if self.tabs[tab_index].tree.len() == 0 {
            let _ = self.remove_tab(tab_index);
        } else if tab_index == self.active {
            let focused = self.tabs[tab_index].tree.focused_pane();
            self.tabs[tab_index].tree.set_focus_to_pane(focused.id());
        }
        Some(pane)
    }

    pub fn tab_info(&self) -> Vec<TabInfo> {
        self.tab_info_with_spinner(0)
    }

    pub fn all_panes(&self) -> Vec<PaneHandle> {
        self.tabs.iter().flat_map(|t| t.tree.all_panes()).collect()
    }

    pub fn layout_active(
        &self,
        frame: crate::platform::Rect,
        scale: f64,
    ) -> Vec<(PaneHandle, u32, u32)> {
        self.active_tree()
            .map(|t| t.layout(frame, scale))
            .unwrap_or_default()
    }
}

fn spinner_for(frame: usize) -> &'static str {
    const FRAMES: [&str; 4] = ["-", "\\", "|", "/"];
    FRAMES[frame % FRAMES.len()]
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct TabInfo {
    pub index: usize,
    pub active: bool,
    pub title: String,
    pub surfaces: usize,
}

#[cfg(test)]
mod tests {
    use super::TabManager;
    use crate::pane::PaneHandle;
    use crate::session::TabLayout;

    #[test]
    fn new_tab_switches_active_pane() {
        let mut tabs = TabManager::new();
        let first = PaneHandle::detached();
        let second = PaneHandle::detached();

        tabs.add_initial_tab(first);
        let new_index = tabs.new_tab(second);

        assert_eq!(new_index, 1);
        assert_eq!(tabs.active_index(), 1);
        assert_eq!(tabs.focused_pane(), second);
        assert_eq!(tabs.len(), 2);
    }

    #[test]
    fn goto_next_and_prev_tab_update_active_index() {
        let mut tabs = TabManager::new();
        tabs.add_initial_tab(PaneHandle::detached());
        tabs.new_tab(PaneHandle::detached());
        tabs.new_tab(PaneHandle::detached());

        assert!(tabs.goto_tab(0));
        assert_eq!(tabs.active_index(), 0);
        assert!(tabs.next_tab());
        assert_eq!(tabs.active_index(), 1);
        assert!(tabs.prev_tab());
        assert_eq!(tabs.active_index(), 0);
    }

    #[test]
    fn remove_tab_returns_panes_and_keeps_valid_active_tab() {
        let mut tabs = TabManager::new();
        let first = PaneHandle::detached();
        let second = PaneHandle::detached();

        tabs.add_initial_tab(first);
        tabs.new_tab(second);

        let removed = tabs.remove_tab(1);

        assert_eq!(removed, vec![second]);
        assert_eq!(tabs.len(), 1);
        assert_eq!(tabs.active_index(), 0);
        assert_eq!(tabs.focused_pane(), first);
    }

    #[test]
    fn tab_info_tracks_active_state_and_titles() {
        let mut tabs = TabManager::new();
        tabs.add_initial_tab(PaneHandle::detached());
        tabs.set_active_title("shell".to_string());
        tabs.new_tab(PaneHandle::detached());
        tabs.set_active_title("logs".to_string());

        let info = tabs.tab_info();

        assert_eq!(info.len(), 2);
        assert_eq!(info[0].title, "shell");
        assert!(!info[0].active);
        assert_eq!(info[1].title, "logs");
        assert!(info[1].active);
    }

    #[test]
    fn tab_info_prefers_running_command_with_spinner() {
        let mut tabs = TabManager::new();
        let pane = PaneHandle::detached();
        tabs.add_initial_tab(pane);
        tabs.set_active_title("shell".to_string());
        tabs.set_running_command_for_pane(
            pane.id(),
            Some(super::RunningCommand {
                command: Some("cargo test".to_string()),
            }),
        );

        let info = tabs.tab_info_with_spinner(1);

        assert_eq!(info[0].title, "\\ cargo test");
    }

    #[test]
    fn tab_info_falls_back_to_title_when_running_command_has_no_cmdline() {
        let mut tabs = TabManager::new();
        let pane = PaneHandle::detached();
        tabs.add_initial_tab(pane);
        tabs.set_active_title("shell".to_string());
        tabs.set_running_command_for_pane(pane.id(), Some(super::RunningCommand { command: None }));

        let info = tabs.tab_info_with_spinner(2);

        assert_eq!(info[0].title, "| shell");
    }

    #[test]
    fn apply_layout_updates_active_layout() {
        let mut tabs = TabManager::new();
        let a = PaneHandle::detached();
        let b = PaneHandle::detached();
        let c = PaneHandle::detached();

        tabs.add_initial_tab(a);
        tabs.active_tree_mut()
            .unwrap()
            .split_focused(crate::splits::Direction::Horizontal, b);
        tabs.active_tree_mut()
            .unwrap()
            .split_focused(crate::splits::Direction::Horizontal, c);

        assert!(tabs.apply_layout_to_active(TabLayout::EvenVertical));
        assert_eq!(tabs.active_layout(), Some(TabLayout::EvenVertical));
    }

    #[test]
    fn break_active_pane_creates_new_tab() {
        let mut tabs = TabManager::new();
        let a = PaneHandle::detached();
        let b = PaneHandle::detached();

        tabs.add_initial_tab(a);
        tabs.active_tree_mut()
            .unwrap()
            .split_focused(crate::splits::Direction::Horizontal, b);

        let new_index = tabs.break_active_pane_to_tab().unwrap();

        assert_eq!(new_index, 1);
        assert_eq!(tabs.len(), 2);
        assert_eq!(tabs.focused_pane(), b);
        assert_eq!(tabs.tab_tree(0).unwrap().len(), 1);
    }

    #[test]
    fn focus_active_pane_by_id_selects_matching_pane() {
        let mut tabs = TabManager::new();
        let a = PaneHandle::detached();
        let b = PaneHandle::detached();

        tabs.add_initial_tab(a);
        tabs.active_tree_mut()
            .unwrap()
            .split_focused(crate::splits::Direction::Horizontal, b);

        assert!(tabs.focus_active_pane_by_id(a.id()));
        assert_eq!(tabs.focused_pane(), a);
        assert!(!tabs.focus_active_pane_by_id(999_999));
    }
}
