//! Tab management — each tab contains its own split tree.

use crate::pane::PaneHandle;
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
    running_command: Option<RunningCommand>,
}

#[derive(Clone)]
pub struct RunningCommand {
    pub command: Option<String>,
}

#[derive(Clone)]
pub struct TabSessionInfo {
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

    pub fn set_active_title(&mut self, title: String) {
        if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.title = title;
        }
    }

    pub fn set_running_command_for_pane(
        &mut self,
        pane_id: crate::pane::PaneId,
        running_command: Option<RunningCommand>,
    ) {
        for tab in &mut self.tabs {
            if tab.tree.focused_pane().id() == pane_id {
                tab.running_command = running_command.clone();
            }
        }
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

    pub fn tab_session_info(&self) -> Vec<TabSessionInfo> {
        self.tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| TabSessionInfo {
                id: tab.id,
                index,
                title: self.display_title(index, None),
            })
            .collect()
    }

    pub fn session_id_for_index(&self, index: usize) -> Option<u32> {
        self.tabs.get(index).map(|tab| tab.id)
    }

    pub fn find_index_by_session_id(&self, session_id: u32) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.id == session_id)
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
}
