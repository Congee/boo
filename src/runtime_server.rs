use super::*;
use std::time::Instant;

fn latency_debug_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("BOO_LATENCY_DEBUG").is_some())
}

fn log_server_latency(stage: &str, started_at: Instant) {
    crate::profiling::record(
        stage_to_profile_path(stage),
        crate::profiling::Kind::Cpu,
        started_at.elapsed(),
    );
    if latency_debug_enabled() {
        log::info!(
            "boo_latency_server stage={stage} ms={:.3}",
            started_at.elapsed().as_secs_f64() * 1000.0
        );
    }
}

fn stage_to_profile_path(stage: &str) -> &'static str {
    match stage {
        "remote_input_applied" => "server.remote.input.apply",
        "remote_key_applied" => "server.remote.key.apply",
        "publish_remote_session" => "server.remote.publish_session",
        _ => "server.unknown",
    }
}

impl BooApp {
    pub(crate) fn remote_servers(&self) -> impl Iterator<Item = &remote::RemoteServer> {
        self.server
            .remote_server
            .iter()
            .chain(self.server.local_gui_server.iter())
    }

    pub(crate) fn has_attached_stream_sessions(&self) -> bool {
        self.remote_servers().any(|server| server.has_attached_sessions())
    }

    fn remote_server_for_client(&self, client_id: u64) -> Option<&remote::RemoteServer> {
        self.remote_servers()
            .find(|server| server.client_session(client_id).is_some())
    }

    pub(crate) fn handle_server_cmd(&mut self, cmd: server::Command) {
        match cmd {
            server::Command::DumpKeys(enabled) => self.dump_keys = enabled,
            server::Command::Ping => {}
            server::Command::Quit => self.terminate(0),
            server::Command::ListSurfaces { reply } => {
                let info = if let Some(tree) = self.server.tabs.active_tree() {
                    tree.surface_info()
                        .into_iter()
                        .map(|(id, focused)| control::SurfaceInfo { index: id, focused })
                        .collect()
                } else {
                    Vec::new()
                };
                let _ = reply.send(control::Response::Surfaces { surfaces: info });
            }
            server::Command::NewSplit { direction } => {
                self.create_split(Self::split_direction_from_str(&direction));
            }
            server::Command::FocusSurface { index } => {
                let old = self.server.tabs.focused_pane();
                if let Some(tree) = self.server.tabs.active_tree_mut() {
                    tree.set_focus(index);
                }
                let new = self.server.tabs.focused_pane();
                if old != new {
                    self.set_pane_focus(old, false);
                    self.set_pane_focus(new, true);
                }
            }
            server::Command::ListTabs { reply } => {
                let _ = reply.send(control::Response::Tabs {
                    tabs: self.server.tabs.tab_info(),
                });
            }
            server::Command::GetClipboard { reply } => {
                let _ = reply.send(control::Response::Clipboard {
                    text: if self.last_clipboard_text.is_empty() {
                        platform::clipboard_read().unwrap_or_default()
                    } else {
                        self.last_clipboard_text.clone()
                    },
                });
            }
            server::Command::GetUiSnapshot { reply } => {
                let _ = reply.send(control::Response::UiSnapshot {
                    snapshot: self.ui_snapshot(),
                });
            }
            server::Command::ExecuteCommand { input } => {
                self.execute_command(&input);
            }
            server::Command::AppKeyEvent { event } => {
                self.handle_app_key_event(event);
            }
            server::Command::AppMouseEvent { event } => {
                self.handle_app_mouse_event(event);
            }
            server::Command::AppAction { action } => {
                self.dispatch_binding_action(action);
            }
            server::Command::FocusPane { pane_id } => {
                self.focus_pane_by_id(pane_id);
            }
            server::Command::SendText { text } => {
                let _ = self
                    .backend
                    .write_input(self.server.tabs.focused_pane(), text.as_bytes());
            }
            server::Command::SendVt { text } => {
                self.backend
                    .write_vt_bytes(self.server.tabs.focused_pane(), text.as_bytes());
            }
            server::Command::NewTab => {
                let _ = self.new_tab();
            }
            server::Command::GotoTab { index } => {
                self.server.tabs.goto_tab(index);
                self.sync_after_tab_change();
            }
            server::Command::NextTab => {
                self.server.tabs.next_tab();
                self.sync_after_tab_change();
            }
            server::Command::PrevTab => {
                self.server.tabs.prev_tab();
                self.sync_after_tab_change();
            }
            server::Command::ResizeViewportPoints { width, height } => {
                let changed = self.resize_viewport_points(width, height);
                if changed && let Some(server) = self.server.local_gui_server.as_ref() {
                    server.send_ui_snapshot_to_local_clients(&self.ui_snapshot());
                }
            }
            server::Command::ResizeViewport { cols, rows } => {
                let changed = self.resize_viewport_cells(cols, rows);
                if changed && let Some(server) = self.server.local_gui_server.as_ref() {
                    server.send_ui_snapshot_to_local_clients(&self.ui_snapshot());
                }
            }
            server::Command::ResizeFocused { cols, rows } => {
                let pane = self.server.tabs.focused_pane();
                let (width, height) = self.session_size_pixels(cols, rows);
                self.resize_pane_backend(pane, self.scale_factor(), width, height);
            }
            server::Command::SendKey { keyspec } => {
                self.inject_key(&keyspec);
            }
            server::Command::RemoteListSessions { client_id } => {
                if let Some(server) = self.server.local_gui_server.as_ref()
                    && server.client_is_local(client_id)
                    && server.client_session(client_id).is_none()
                    && let Some(session_id) = self.server.tabs.active_session_id()
                    && self.pane_for_session(session_id).is_some()
                {
                    server.send_attached(client_id, session_id);
                    server.send_ui_snapshot(client_id, &self.ui_snapshot());
                    self.publish_remote_session(session_id);
                }
                if let Some(server) = self
                    .remote_server_for_client(client_id)
                    .or(self.server.local_gui_server.as_ref())
                    .or(self.server.remote_server.as_ref())
                {
                    server.send_session_list(client_id, &self.remote_sessions());
                }
            }
            server::Command::RemoteAttach {
                client_id,
                session_id,
            } => {
                if self.pane_for_session(session_id).is_some() {
                    if let Some(server) = self
                        .remote_server_for_client(client_id)
                        .or(self.server.local_gui_server.as_ref())
                        .or(self.server.remote_server.as_ref())
                    {
                        server.send_attached(client_id, session_id);
                        server.send_ui_snapshot(client_id, &self.ui_snapshot());
                    }
                    self.publish_remote_session(session_id);
                } else if let Some(server) = self
                    .remote_server_for_client(client_id)
                    .or(self.server.local_gui_server.as_ref())
                    .or(self.server.remote_server.as_ref())
                {
                    server.send_error(client_id, "unknown session");
                }
            }
            server::Command::RemoteDetach { client_id } => {
                if let Some(server) = self
                    .remote_server_for_client(client_id)
                    .or(self.server.local_gui_server.as_ref())
                    .or(self.server.remote_server.as_ref())
                {
                    server.send_detached(client_id);
                }
            }
            server::Command::RemoteCreate {
                client_id,
                cols,
                rows,
            } => {
                let created = self.new_tab();
                let Some(session_id) = created else {
                    if let Some(server) = self
                        .remote_server_for_client(client_id)
                        .or(self.server.local_gui_server.as_ref())
                        .or(self.server.remote_server.as_ref())
                    {
                        server.send_error(client_id, "failed to create session");
                    }
                    return;
                };
                if let Some(pane) = self.pane_for_session(session_id) {
                    let (width, height) = self.session_size_pixels(cols, rows);
                    self.resize_pane_backend(pane, self.scale_factor(), width, height);
                }
                if let Some(server) = self
                    .remote_server_for_client(client_id)
                    .or(self.server.local_gui_server.as_ref())
                    .or(self.server.remote_server.as_ref())
                {
                    server.send_session_created(client_id, session_id);
                }
            }
            server::Command::RemoteInput {
                client_id,
                bytes,
                input_seq,
            } => {
                let started_at = Instant::now();
                let Some(session_id) = self
                    .remote_server_for_client(client_id)
                    .and_then(|server| server.client_session(client_id))
                else {
                    if let Some(server) = self
                        .remote_server_for_client(client_id)
                        .or(self.server.local_gui_server.as_ref())
                        .or(self.server.remote_server.as_ref())
                    {
                        server.send_error(client_id, "not attached");
                    }
                    return;
                };
                let Some(pane) = self.pane_for_session(session_id) else {
                    if let Some(server) = self
                        .remote_server_for_client(client_id)
                        .or(self.server.local_gui_server.as_ref())
                        .or(self.server.remote_server.as_ref())
                    {
                        server.send_session_exited(session_id);
                    }
                    return;
                };
                if let Some(server) = self.remote_server_for_client(client_id) {
                    server.record_input_seq(client_id, input_seq);
                }
                let _ = self.backend.write_input(pane, &bytes);
                log_server_latency("remote_input_applied", started_at);
            }
            server::Command::RemoteKey {
                client_id,
                keyspec,
                input_seq,
            } => {
                let started_at = Instant::now();
                let Some(session_id) = self
                    .remote_server_for_client(client_id)
                    .and_then(|server| server.client_session(client_id))
                else {
                    if let Some(server) = self
                        .remote_server_for_client(client_id)
                        .or(self.server.local_gui_server.as_ref())
                        .or(self.server.remote_server.as_ref())
                    {
                        server.send_error(client_id, "not attached");
                    }
                    return;
                };
                let Some(tab_index) = self.server.tabs.find_index_by_session_id(session_id) else {
                    if let Some(server) = self
                        .remote_server_for_client(client_id)
                        .or(self.server.local_gui_server.as_ref())
                        .or(self.server.remote_server.as_ref())
                    {
                        server.send_session_exited(session_id);
                    }
                    return;
                };
                let old = self.server.tabs.focused_pane();
                self.server.tabs.goto_tab(tab_index);
                let new = self.server.tabs.focused_pane();
                if old != new {
                    self.set_pane_focus(old, false);
                    self.set_pane_focus(new, true);
                }
                if let Some(server) = self.remote_server_for_client(client_id) {
                    server.record_input_seq(client_id, input_seq);
                }
                self.inject_key(&keyspec);
                log_server_latency("remote_key_applied", started_at);
            }
            server::Command::RemoteResize {
                client_id,
                cols,
                rows,
            } => {
                let Some(session_id) = self
                    .remote_server_for_client(client_id)
                    .and_then(|server| server.client_session(client_id))
                else {
                    if let Some(server) = self
                        .remote_server_for_client(client_id)
                        .or(self.server.local_gui_server.as_ref())
                        .or(self.server.remote_server.as_ref())
                    {
                        server.send_error(client_id, "not attached");
                    }
                    return;
                };
                let Some(_pane) = self.pane_for_session(session_id) else {
                    if let Some(server) = self
                        .remote_server_for_client(client_id)
                        .or(self.server.local_gui_server.as_ref())
                        .or(self.server.remote_server.as_ref())
                    {
                        server.send_session_exited(session_id);
                    }
                    return;
                };
                if self.resize_viewport_cells(cols, rows) {
                    self.publish_remote_session(session_id);
                }
            }
            server::Command::RemoteExecuteCommand { client_id, input } => {
                self.execute_command(&input);
                let focused_session_id = self.server.tabs.active_session_id();
                if let Some(server) = self
                    .remote_server_for_client(client_id)
                    .or(self.server.local_gui_server.as_ref())
                    .or(self.server.remote_server.as_ref())
                {
                    server.send_session_list(client_id, &self.remote_sessions());
                    if let Some(session_id) = focused_session_id {
                        server.send_attached(client_id, session_id);
                        self.publish_remote_session(session_id);
                    }
                }
            }
            server::Command::RemoteAppKeyEvent { client_id, event } => {
                let Some(session_id) = self
                    .remote_server_for_client(client_id)
                    .and_then(|server| server.client_session(client_id))
                else {
                    if let Some(server) = self
                        .remote_server_for_client(client_id)
                        .or(self.server.local_gui_server.as_ref())
                        .or(self.server.remote_server.as_ref())
                    {
                        server.send_error(client_id, "not attached");
                    }
                    return;
                };
                let Some(tab_index) = self.server.tabs.find_index_by_session_id(session_id) else {
                    if let Some(server) = self
                        .remote_server_for_client(client_id)
                        .or(self.server.local_gui_server.as_ref())
                        .or(self.server.remote_server.as_ref())
                    {
                        server.send_session_exited(session_id);
                    }
                    return;
                };
                let old = self.server.tabs.focused_pane();
                self.server.tabs.goto_tab(tab_index);
                let new = self.server.tabs.focused_pane();
                if old != new {
                    self.set_pane_focus(old, false);
                    self.set_pane_focus(new, true);
                }
                if let Some(server) = self.remote_server_for_client(client_id) {
                    server.record_input_seq(client_id, event.input_seq);
                }
                let consumed = self.handle_app_key_event(event);
                if consumed {
                    let focused_session_id = self.server.tabs.active_session_id();
                    if let Some(server) = self
                        .remote_server_for_client(client_id)
                        .or(self.server.local_gui_server.as_ref())
                        .or(self.server.remote_server.as_ref())
                    {
                        server.send_session_list(client_id, &self.remote_sessions());
                        if let Some(session_id) = focused_session_id {
                            server.send_attached(client_id, session_id);
                            self.publish_remote_session(session_id);
                        }
                    }
                }
            }
            server::Command::RemoteAppMouseEvent { client_id, event } => {
                let Some(session_id) = self
                    .remote_server_for_client(client_id)
                    .and_then(|server| server.client_session(client_id))
                else {
                    if let Some(server) = self
                        .remote_server_for_client(client_id)
                        .or(self.server.local_gui_server.as_ref())
                        .or(self.server.remote_server.as_ref())
                    {
                        server.send_error(client_id, "not attached");
                    }
                    return;
                };
                let Some(tab_index) = self.server.tabs.find_index_by_session_id(session_id) else {
                    if let Some(server) = self
                        .remote_server_for_client(client_id)
                        .or(self.server.local_gui_server.as_ref())
                        .or(self.server.remote_server.as_ref())
                    {
                        server.send_session_exited(session_id);
                    }
                    return;
                };
                let old = self.server.tabs.focused_pane();
                self.server.tabs.goto_tab(tab_index);
                let new = self.server.tabs.focused_pane();
                if old != new {
                    self.set_pane_focus(old, false);
                    self.set_pane_focus(new, true);
                }
                let changed_ui = self.handle_app_mouse_event(event);
                if changed_ui
                    && let Some(server) = self
                        .remote_server_for_client(client_id)
                        .or(self.server.local_gui_server.as_ref())
                        .or(self.server.remote_server.as_ref())
                {
                    server.send_session_list(client_id, &self.remote_sessions());
                    server.send_attached(client_id, session_id);
                    self.publish_remote_session(session_id);
                }
            }
            server::Command::RemoteAppAction { client_id, action } => {
                self.dispatch_binding_action(action);
                let focused_session_id = self.server.tabs.active_session_id();
                if let Some(server) = self
                    .remote_server_for_client(client_id)
                    .or(self.server.local_gui_server.as_ref())
                    .or(self.server.remote_server.as_ref())
                {
                    server.send_session_list(client_id, &self.remote_sessions());
                    if let Some(session_id) = focused_session_id {
                        server.send_attached(client_id, session_id);
                        self.publish_remote_session(session_id);
                    }
                }
            }
            server::Command::RemoteFocusPane { client_id, pane_id } => {
                let Some(session_id) = self
                    .remote_server_for_client(client_id)
                    .and_then(|server| server.client_session(client_id))
                else {
                    if let Some(server) = self
                        .remote_server_for_client(client_id)
                        .or(self.server.local_gui_server.as_ref())
                        .or(self.server.remote_server.as_ref())
                    {
                        server.send_error(client_id, "not attached");
                    }
                    return;
                };
                let Some(tab_index) = self.server.tabs.find_index_by_session_id(session_id) else {
                    if let Some(server) = self
                        .remote_server_for_client(client_id)
                        .or(self.server.local_gui_server.as_ref())
                        .or(self.server.remote_server.as_ref())
                    {
                        server.send_session_exited(session_id);
                    }
                    return;
                };
                let old = self.server.tabs.focused_pane();
                self.server.tabs.goto_tab(tab_index);
                let new = self.server.tabs.focused_pane();
                if old != new {
                    self.set_pane_focus(old, false);
                    self.set_pane_focus(new, true);
                }
                if self.focus_pane_by_id(pane_id)
                    && let Some(server) = self
                        .remote_server_for_client(client_id)
                        .or(self.server.local_gui_server.as_ref())
                        .or(self.server.remote_server.as_ref())
                {
                    server.send_session_list(client_id, &self.remote_sessions());
                    server.send_attached(client_id, session_id);
                    self.publish_remote_session(session_id);
                }
            }
            server::Command::RemoteDestroy {
                client_id,
                session_id,
            } => {
                let target = session_id.or_else(|| {
                    self.remote_server_for_client(client_id)
                        .and_then(|server| server.client_session(client_id))
                });
                let Some(target) = target else {
                    if let Some(server) = self
                        .remote_server_for_client(client_id)
                        .or(self.server.local_gui_server.as_ref())
                        .or(self.server.remote_server.as_ref())
                    {
                        server.send_error(client_id, "unknown session");
                    }
                    return;
                };
                let Some(tab_index) = self.server.tabs.find_index_by_session_id(target) else {
                    if let Some(server) = self
                        .remote_server_for_client(client_id)
                        .or(self.server.local_gui_server.as_ref())
                        .or(self.server.remote_server.as_ref())
                    {
                        server.send_session_exited(target);
                    }
                    return;
                };
                if self.server.tabs.len() <= 1 {
                    if let Some(server) = self
                        .remote_server_for_client(client_id)
                        .or(self.server.local_gui_server.as_ref())
                        .or(self.server.remote_server.as_ref())
                    {
                        server.send_error(client_id, "cannot destroy last session");
                    }
                    return;
                }
                let was_active = tab_index == self.server.tabs.active_index();
                let panes = self.server.tabs.remove_tab(tab_index);
                for pane in panes {
                    self.backend.free_pane(pane);
                }
                if was_active && !self.server.tabs.is_empty() {
                    self.sync_after_tab_change();
                }
                if let Some(server) = self
                    .remote_server_for_client(client_id)
                    .or(self.server.local_gui_server.as_ref())
                    .or(self.server.remote_server.as_ref())
                {
                    server.send_session_exited(target);
                }
            }
        }
    }

    pub(crate) fn remote_sessions(&self) -> Vec<remote::RemoteSessionInfo> {
        self.server
            .tabs
            .tab_session_info()
            .into_iter()
            .map(|tab| {
                let pane = self
                    .server
                    .tabs
                    .tab_tree(tab.index)
                    .map(|tree| tree.focused_pane())
                    .unwrap_or(PaneHandle::null());
                let terminal = self.backend.ui_terminal_snapshot(pane.id());
                remote::RemoteSessionInfo {
                    id: tab.id,
                    name: format!("Tab {}", tab.index + 1),
                    title: tab.title,
                    pwd: terminal
                        .as_ref()
                        .map(|snapshot| snapshot.pwd.clone())
                        .unwrap_or_default(),
                    attached: self
                        .remote_servers()
                        .any(|server| server.attached_to_session(tab.id)),
                    child_exited: pane.id() == 0 || terminal.is_none(),
                }
            })
            .collect()
    }

    pub(crate) fn pane_for_session(&self, session_id: u32) -> Option<PaneHandle> {
        let tab_index = self.server.tabs.find_index_by_session_id(session_id)?;
        self.server
            .tabs
            .tab_tree(tab_index)
            .map(|tree| tree.focused_pane())
    }

    pub(crate) fn session_size_pixels(&self, cols: u16, rows: u16) -> (u32, u32) {
        let width = (cols as f64 * self.cell_width).round().max(1.0) as u32;
        let height = (rows as f64 * self.cell_height).round().max(1.0) as u32;
        (width, height)
    }

    pub(crate) fn publish_remote_session(&self, session_id: u32) {
        let started_at = Instant::now();
        let mut sent = false;
        for server in self.remote_servers() {
            sent = true;
            let Some(pane) = self.pane_for_session(session_id) else {
                server.send_session_exited(session_id);
                continue;
            };
            let Some(snapshot) = self.backend.render_snapshot_ref(pane.id()) else {
                server.send_session_exited(session_id);
                continue;
            };
            let state = remote::full_state_from_terminal(snapshot);
            server.send_full_state_to_attached(session_id, &state);
        }
        if !sent {
            return;
        }
        log_server_latency("publish_remote_session", started_at);
    }

    pub(crate) fn publish_remote_state(&self) {
        let mut session_ids = Vec::new();
        for server in self.remote_servers() {
            for session_id in server.attached_sessions() {
                if !session_ids.contains(&session_id) {
                    session_ids.push(session_id);
                }
            }
        }
        for session_id in session_ids {
            self.publish_remote_session(session_id);
        }
    }
}
