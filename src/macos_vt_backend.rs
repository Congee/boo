#![cfg(target_os = "macos")]

use crate::control;
use crate::ffi;
use crate::pane::{self, PaneHandle};
use crate::platform;
use crate::vt_backend_core::{CellSnapshot, TerminalSnapshot, VtPaneWorker};
use std::collections::HashMap;
use std::ffi::{CStr, c_void};
use std::os::unix::ffi::OsStrExt;
use std::sync::Arc;

pub struct MacVtBackend {
    panes: HashMap<pane::PaneId, VtPaneWorker>,
    snapshots: HashMap<pane::PaneId, Arc<TerminalSnapshot>>,
    snapshot_versions: HashMap<pane::PaneId, u64>,
    cell_metrics: HashMap<pane::PaneId, (u32, u32)>,
}

impl MacVtBackend {
    pub fn new() -> Self {
        log::info!("macOS VT backend initialized");
        Self {
            panes: HashMap::new(),
            snapshots: HashMap::new(),
            snapshot_versions: HashMap::new(),
            cell_metrics: HashMap::new(),
        }
    }

    fn pane(&self, focused_pane: PaneHandle) -> Option<&VtPaneWorker> {
        self.panes.get(&focused_pane.id())
    }

    #[cfg(test)]
    pub(crate) fn set_test_snapshot(
        &mut self,
        pane_id: pane::PaneId,
        snapshot: TerminalSnapshot,
        cell_width_px: u32,
        cell_height_px: u32,
    ) {
        let version = self
            .snapshot_versions
            .get(&pane_id)
            .copied()
            .unwrap_or_default()
            + 1;
        self.snapshot_versions.insert(pane_id, version);
        self.snapshots.insert(pane_id, Arc::new(snapshot));
        self.cell_metrics
            .insert(pane_id, (cell_width_px, cell_height_px));
    }
}

impl crate::backend::TerminalBackend for MacVtBackend {
    fn new(_callback_userdata: *mut c_void) -> Self {
        Self::new()
    }

    fn tick(&mut self) {}

    fn set_app_focus(&mut self, _focused: bool) {}

    fn reload_config(&mut self) {}

    fn apply_config_override(
        &mut self,
        _focused_surface: ffi::ghostty_surface_t,
        _key: &str,
        _value: &str,
    ) {
    }

    fn create_pane(
        &mut self,
        _callback_userdata: *mut c_void,
        parent_view: *mut c_void,
        _scale: f64,
        frame: platform::Rect,
        _context: ffi::ghostty_surface_context_e,
        command: Option<&CStr>,
        working_directory: Option<&CStr>,
        cell_width: f64,
        cell_height: f64,
    ) -> Option<PaneHandle> {
        let cols = ((frame.size.width / cell_width).floor() as u16).max(2);
        let rows = ((frame.size.height / cell_height).floor() as u16).max(1);
        let cell_width_px = cell_width.max(1.0).round() as u32;
        let cell_height_px = cell_height.max(1.0).round() as u32;
        let view = if parent_view.is_null() {
            std::ptr::null_mut()
        } else {
            platform::create_focusable_child_view(parent_view, frame)
        };
        let pane = PaneHandle::new(std::ptr::null_mut(), view);
        let wd_path = working_directory
            .map(|wd| std::path::Path::new(std::ffi::OsStr::from_bytes(wd.to_bytes())));
        let backend = match VtPaneWorker::spawn(
            cols,
            rows,
            cell_width_px,
            cell_height_px,
            command,
            wd_path,
        ) {
            Ok(backend) => backend,
            Err(error) => {
                log::warn!("failed to spawn macOS VT pane: {error}");
                return None;
            }
        };

        let update = backend.poll_update();
        self.snapshot_versions.insert(pane.id(), update.version);
        self.snapshots.insert(pane.id(), update.snapshot);
        self.cell_metrics
            .insert(pane.id(), (cell_width_px, cell_height_px));
        self.panes.insert(pane.id(), backend);

        Some(pane)
    }

    fn resize_pane(
        &mut self,
        pane: PaneHandle,
        _scale: f64,
        width: u32,
        height: u32,
        cell_width: f64,
        cell_height: f64,
    ) {
        if let Some(vt_pane) = self.panes.get_mut(&pane.id()) {
            let cols = ((width as f64 / cell_width).floor() as u16).max(2);
            let rows = ((height as f64 / cell_height).floor() as u16).max(1);
            vt_pane.resize(
                cols,
                rows,
                cell_width.max(1.0).round() as u32,
                cell_height.max(1.0).round() as u32,
            );
            self.cell_metrics.insert(
                pane.id(),
                (
                    cell_width.max(1.0).round() as u32,
                    cell_height.max(1.0).round() as u32,
                ),
            );
        }
    }

    fn free_pane(&mut self, pane: PaneHandle) {
        self.panes.remove(&pane.id());
        self.snapshots.remove(&pane.id());
        self.snapshot_versions.remove(&pane.id());
        self.cell_metrics.remove(&pane.id());
        platform::remove_view(pane.view());
    }

    fn set_surface_focus(&self, _surface: ffi::ghostty_surface_t, _focused: bool) {}

    fn surface_key_translation_mods(&self, _surface: ffi::ghostty_surface_t, mods: i32) -> i32 {
        mods
    }

    fn surface_key(&mut self, _focused_pane: PaneHandle, _event: ffi::ghostty_input_key_s) -> bool {
        false
    }

    fn surface_mouse_pos(&mut self, _focused_pane: PaneHandle, _x: f64, _y: f64, _mods: i32) {}

    fn surface_mouse_button(
        &mut self,
        _focused_pane: PaneHandle,
        _state: ffi::ghostty_input_mouse_state_e,
        _button: ffi::ghostty_input_mouse_button_e,
        _mods: i32,
    ) {
    }

    fn surface_mouse_scroll(&mut self, focused_pane: PaneHandle, _dx: f64, dy: f64, _mods: i32) {
        let Some(pane) = self.pane(focused_pane) else {
            return;
        };
        let delta = if dy.abs() >= 1.0 {
            -dy.round() as isize
        } else if dy > 0.0 {
            -1
        } else if dy < 0.0 {
            1
        } else {
            0
        };
        let _ = pane.scroll_viewport_delta(delta);
    }

    fn ime_point(&self, focused_pane: PaneHandle) -> Option<(f64, f64, f64, f64)> {
        let snapshot = self.snapshots.get(&focused_pane.id())?;
        let (cell_width, cell_height) = self.cell_metrics.get(&focused_pane.id()).copied()?;
        let cell_width = cell_width as f64;
        let cell_height = cell_height as f64;
        Some((
            snapshot.cursor.x as f64 * cell_width,
            (snapshot.cursor.y as f64 + 1.0) * cell_height,
            cell_width,
            cell_height,
        ))
    }

    fn binding_action(&mut self, focused_pane: PaneHandle, action: &str, scrollbar_len: u64) {
        let id = focused_pane.id();
        match action {
            "scroll_to_top" => {
                if let Some(pane) = self.panes.get_mut(&id) {
                    let _ = pane.scroll_viewport_top();
                }
            }
            "scroll_to_bottom" => {
                if let Some(pane) = self.panes.get_mut(&id) {
                    let _ = pane.scroll_viewport_bottom();
                }
            }
            "scroll_page_up" => {
                if let Some(pane) = self.panes.get_mut(&id) {
                    let page = scrollbar_len.saturating_sub(1).max(1) as isize;
                    let _ = pane.scroll_viewport_delta(-page);
                }
            }
            "scroll_page_down" => {
                if let Some(pane) = self.panes.get_mut(&id) {
                    let page = scrollbar_len.saturating_sub(1).max(1) as isize;
                    let _ = pane.scroll_viewport_delta(page);
                }
            }
            "paste_from_clipboard" => {
                if let Some(text) = platform::clipboard_read()
                    && let Some(active_pane) = self.panes.get(&id)
                {
                    let _ = active_pane.write_input(text.as_bytes());
                }
            }
            "end_search" | "toggle_split_zoom" => {}
            _ => {
                if let Some(lines) = action.strip_prefix("scroll_page_lines:")
                    && let Ok(lines) = lines.parse::<isize>()
                    && let Some(pane) = self.panes.get_mut(&id)
                {
                    let _ = pane.scroll_viewport_delta(lines);
                }
            }
        }
    }

    fn read_selection_text(
        &self,
        focused_pane: PaneHandle,
        selection: ffi::ghostty_selection_s,
    ) -> Option<String> {
        if selection.top_left.tag != ffi::GHOSTTY_POINT_VIEWPORT
            || selection.bottom_right.tag != ffi::GHOSTTY_POINT_VIEWPORT
            || selection.top_left.coord != ffi::GHOSTTY_POINT_COORD_EXACT
            || selection.bottom_right.coord != ffi::GHOSTTY_POINT_COORD_EXACT
        {
            return None;
        }

        let snapshot = self.snapshots.get(&focused_pane.id())?;
        Some(snapshot_selection_text(snapshot, selection))
    }

    fn poll(
        &mut self,
        active_pane_ids: &[pane::PaneId],
        active_id: pane::PaneId,
        _scrollbar_len: u64,
        _cell_width: f64,
        _cell_height: f64,
    ) -> crate::backend::BackendPollResult {
        let mut result = crate::backend::BackendPollResult {
            terminal_dirty: false,
            exited_panes: Vec::new(),
            active_title: None,
            active_pwd: None,
            active_scrollbar: None,
            running_commands: Vec::new(),
            finished_commands: Vec::new(),
            status_component_updates: Vec::new(),
            desktop_notifications: Vec::new(),
        };

        for id in active_pane_ids {
            let Some(pane) = self.panes.get(id) else {
                continue;
            };
            let update = pane.poll_update();
            if update.exited {
                result.exited_panes.push(*id);
            }
            for finished in update.finished_commands {
                result
                    .finished_commands
                    .push(crate::backend::CommandFinished {
                        exit_code: finished.exit_code,
                        duration_ns: finished.duration_ns,
                    });
            }
            for status_update in update.status_component_updates {
                let source = if status_update.source.is_empty() {
                    crate::status_components::osc_source_for_pane(*id)
                } else {
                    status_update.source
                };
                result.status_component_updates.push(
                    crate::status_components::StatusComponentsUpdate {
                        zone: status_update.zone,
                        source,
                        components: status_update.components,
                    },
                );
            }
            for notification in update.desktop_notifications {
                result
                    .desktop_notifications
                    .push(crate::backend::DesktopNotification {
                        title: notification.title,
                        body: notification.body,
                    });
            }

            let cached_version = self.snapshot_versions.get(id).copied().unwrap_or_default();
            if update.version != cached_version || !self.snapshots.contains_key(id) {
                self.snapshot_versions.insert(*id, update.version);
                self.snapshots.insert(*id, Arc::clone(&update.snapshot));
                result.terminal_dirty = true;
            }
            if *id == active_id {
                let snapshot = update.snapshot.as_ref();
                result.active_pwd = Some(snapshot.pwd.clone());
                if !snapshot.title.is_empty() {
                    result.active_title = Some(snapshot.title.clone());
                }
                result.active_scrollbar = Some(ffi::ghostty_action_scrollbar_s {
                    total: snapshot.scrollbar.total,
                    offset: snapshot.scrollbar.offset,
                    len: snapshot.scrollbar.len,
                });
            }
            if let Some(running_command) = update.running_command {
                result
                    .running_commands
                    .push(crate::backend::PaneRunningCommand {
                        pane_id: *id,
                        command: running_command,
                    });
            }
        }

        result
    }

    fn ui_terminal_snapshot(&self, pane_id: pane::PaneId) -> Option<control::UiTerminalSnapshot> {
        self.snapshots
            .get(&pane_id)
            .map(|snapshot| ui_terminal_snapshot(snapshot.as_ref()))
    }

    fn render_snapshot(
        &self,
        pane_id: pane::PaneId,
    ) -> Option<crate::vt_backend_core::TerminalSnapshot> {
        self.snapshots
            .get(&pane_id)
            .map(|snapshot| snapshot.as_ref().clone())
    }

    fn render_snapshot_ref(
        &self,
        pane_id: pane::PaneId,
    ) -> Option<&crate::vt_backend_core::TerminalSnapshot> {
        self.snapshots
            .get(&pane_id)
            .map(|snapshot| snapshot.as_ref())
    }

    fn forward_vt_key(
        &mut self,
        focused_pane: PaneHandle,
        action: i32,
        keycode: u32,
        mods: crate::vt::GhosttyMods,
        consumed_mods: crate::vt::GhosttyMods,
        key_char: Option<char>,
        text: &str,
        composing: bool,
        unshifted_codepoint: u32,
    ) -> std::io::Result<()> {
        let Some(pane) = self.pane(focused_pane) else {
            return Ok(());
        };
        pane.forward_key(crate::vt_backend_core::VtForwardKey {
            action,
            keycode,
            mods,
            consumed_mods,
            key_char,
            text: text.to_string(),
            composing,
            unshifted_codepoint,
        })
    }

    fn send_mouse_input(
        &mut self,
        focused_pane: PaneHandle,
        action: crate::vt::GhosttyMouseAction,
        button: Option<crate::vt::GhosttyMouseButton>,
        x: f32,
        y: f32,
        mods: crate::vt::GhosttyMods,
    ) -> std::io::Result<()> {
        let Some(pane) = self.pane(focused_pane) else {
            return Ok(());
        };
        pane.send_mouse_input(action, button, x, y, mods)
    }

    fn scroll_viewport_delta(
        &mut self,
        focused_pane: PaneHandle,
        delta: isize,
    ) -> std::io::Result<()> {
        let Some(pane) = self.pane(focused_pane) else {
            return Ok(());
        };
        pane.scroll_viewport_delta(delta)
    }

    fn scroll_viewport_top(&mut self, focused_pane: PaneHandle) -> std::io::Result<()> {
        let Some(pane) = self.pane(focused_pane) else {
            return Ok(());
        };
        pane.scroll_viewport_top()
    }

    fn scroll_viewport_bottom(&mut self, focused_pane: PaneHandle) -> std::io::Result<()> {
        let Some(pane) = self.pane(focused_pane) else {
            return Ok(());
        };
        pane.scroll_viewport_bottom()
    }

    fn write_input(&self, focused_pane: PaneHandle, bytes: &[u8]) -> std::io::Result<()> {
        let Some(pane) = self.pane(focused_pane) else {
            return Ok(());
        };
        pane.write_input(bytes)
    }

    fn write_vt_bytes(&mut self, focused_pane: PaneHandle, bytes: &[u8]) {
        if let Some(pane) = self.pane(focused_pane) {
            pane.write_vt_bytes(bytes);
        }
    }

    fn hyperlink_at(&self, focused_pane: PaneHandle, row: u16, col: u16) -> Option<String> {
        let pane = self.pane(focused_pane)?;
        pane.hyperlink_at(row, col).ok().flatten()
    }
}

fn snapshot_selection_text(
    snapshot: &TerminalSnapshot,
    selection: ffi::ghostty_selection_s,
) -> String {
    let start_row = selection.top_left.y.min(selection.bottom_right.y) as usize;
    let end_row = selection.top_left.y.max(selection.bottom_right.y) as usize;
    let start_col = selection.top_left.x.min(selection.bottom_right.x) as usize;
    let end_col = selection.top_left.x.max(selection.bottom_right.x) as usize;
    let max_row = snapshot.rows_data.len().saturating_sub(1);

    let mut lines = Vec::new();
    for row_index in start_row.min(max_row)..=end_row.min(max_row) {
        let row = snapshot
            .rows_data
            .get(row_index)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let line_start = if selection.rectangle || row_index == start_row {
            start_col
        } else {
            0
        };
        let line_end = if selection.rectangle || row_index == end_row {
            end_col
        } else {
            snapshot.cols.saturating_sub(1) as usize
        };
        let text = snapshot_row_text(row, line_start, line_end, selection.rectangle);
        lines.push(text);
    }

    lines.join("\n")
}

fn snapshot_row_text(
    row: &[CellSnapshot],
    start_col: usize,
    end_col: usize,
    preserve_trailing_spaces: bool,
) -> String {
    if row.is_empty() || start_col > end_col {
        return String::new();
    }

    let mut out = String::new();
    for col in start_col..=end_col {
        let text = row
            .get(col)
            .map(|cell| cell.text.as_str())
            .filter(|text| !text.is_empty() && *text != "\0")
            .unwrap_or(" ");
        out.push_str(text);
    }

    if preserve_trailing_spaces {
        out
    } else {
        out.trim_end_matches(' ').to_string()
    }
}

fn ui_terminal_snapshot(snapshot: &TerminalSnapshot) -> control::UiTerminalSnapshot {
    control::UiTerminalSnapshot {
        cols: snapshot.cols,
        rows: snapshot.rows,
        title: snapshot.title.clone(),
        pwd: snapshot.pwd.clone(),
        cursor: control::UiCursorSnapshot {
            visible: snapshot.cursor.visible,
            blinking: snapshot.cursor.blinking,
            x: snapshot.cursor.x,
            y: snapshot.cursor.y,
            style: snapshot.cursor.style,
        },
        rows_data: snapshot
            .rows_data
            .iter()
            .map(|row| control::UiTerminalRowSnapshot {
                cells: row
                    .iter()
                    .map(|cell| control::UiTerminalCellSnapshot {
                        text: cell.text.clone(),
                        display_width: cell.display_width,
                        fg: [cell.fg.r, cell.fg.g, cell.fg.b],
                        bg: [cell.bg.r, cell.bg.g, cell.bg.b],
                        bg_is_default: cell.bg_is_default,
                        bold: cell.bold,
                        italic: cell.italic,
                        underline: cell.underline,
                        hyperlink: cell.hyperlink,
                    })
                    .collect(),
            })
            .collect(),
    }
}
