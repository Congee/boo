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

use std::sync::Arc;

#[derive(Clone, PartialEq, Eq)]
struct LocalGuiTransportState {
    session_id: Option<u32>,
    focused_pane_id: u64,
    pane_frames: Vec<(u64, u64, u64, u64, u64)>,
}

impl BooApp {
    fn remote_full_state_for_pane(&self, pane_id: u64) -> Option<Arc<remote::RemoteFullState>> {
        if let Some(snapshot) = self.backend.render_snapshot_ref(pane_id) {
            return Some(Arc::new(remote::full_state_from_terminal(snapshot)));
        }
        let snapshot = self.backend.ui_terminal_snapshot(pane_id)?;
        Some(Arc::new(remote::full_state_from_ui(&snapshot)))
    }

    fn publish_local_gui_runtime_state_for_active_session(&self) {
        let Some(session_id) = self.server.tabs.active_session_id() else {
            return;
        };
        let Some(server) = self.server.local_gui_server.as_ref() else {
            return;
        };
        server.retarget_local_attached_to_session(session_id);
        server.send_ui_runtime_state_to_local_attached(session_id, &self.ui_runtime_state());
        server.send_session_list_to_local_clients(&self.remote_sessions());
    }

    fn local_gui_transport_state(&self) -> LocalGuiTransportState {
        let session_id = self.server.tabs.active_session_id();
        let focused_pane_id = self.server.tabs.focused_pane().id();
        let pane_frames = session_id
            .and_then(|session_id| self.server.tabs.find_index_by_session_id(session_id))
            .and_then(|tab_index| self.server.tabs.tab_tree(tab_index))
            .map(|tree| {
                tree.export_panes_with_frames(self.terminal_frame())
                    .into_iter()
                    .map(|pane| {
                        let frame = pane.frame.unwrap_or_default();
                        (
                            pane.pane.id(),
                            frame.origin.x.to_bits(),
                            frame.origin.y.to_bits(),
                            frame.size.width.to_bits(),
                            frame.size.height.to_bits(),
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        LocalGuiTransportState {
            session_id,
            focused_pane_id,
            pane_frames,
        }
    }

    fn publish_local_gui_after_ui_action(&self, before: &LocalGuiTransportState) {
        self.publish_local_gui_runtime_state_for_active_session();
        let after = self.local_gui_transport_state();
        if after != *before && let Some(session_id) = after.session_id {
            self.publish_remote_session(session_id);
        }
    }

    fn bootstrap_local_stream_client(&self, server: &remote::RemoteServer, client_id: u64, session_id: u32) {
        server.send_attached(client_id, session_id);
        server.send_ui_appearance(client_id, &self.ui_appearance_snapshot());
        server.send_ui_runtime_state(client_id, &self.ui_runtime_state());
    }

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
                let before = self.local_gui_transport_state();
                self.create_split(Self::split_direction_from_str(&direction));
                self.publish_local_gui_after_ui_action(&before);
            }
            server::Command::FocusSurface { index } => {
                let before = self.local_gui_transport_state();
                let old = self.server.tabs.focused_pane();
                if let Some(tree) = self.server.tabs.active_tree_mut() {
                    tree.set_focus(index);
                }
                let new = self.server.tabs.focused_pane();
                if old != new {
                    self.set_pane_focus(old, false);
                    self.set_pane_focus(new, true);
                }
                self.publish_local_gui_after_ui_action(&before);
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
                let before = self.local_gui_transport_state();
                if self.handle_app_key_event(event) {
                    self.publish_local_gui_after_ui_action(&before);
                }
            }
            server::Command::AppMouseEvent { event } => {
                let before = self.local_gui_transport_state();
                if self.handle_app_mouse_event(event) {
                    self.publish_local_gui_after_ui_action(&before);
                }
            }
            server::Command::AppAction { action } => {
                let before = self.local_gui_transport_state();
                self.dispatch_binding_action(action);
                self.publish_local_gui_after_ui_action(&before);
            }
            server::Command::FocusPane { pane_id } => {
                let before = self.local_gui_transport_state();
                if self.focus_pane_by_id(pane_id) {
                    self.publish_local_gui_after_ui_action(&before);
                }
            }
            server::Command::SendText { text } => {
                self.write_terminal_input(text.as_bytes());
            }
            server::Command::SendVt { text } => {
                self.backend
                    .write_vt_bytes(self.server.tabs.focused_pane(), text.as_bytes());
            }
            server::Command::NewTab => {
                let before = self.local_gui_transport_state();
                let _ = self.new_tab();
                self.publish_local_gui_after_ui_action(&before);
            }
            server::Command::GotoTab { index } => {
                let before = self.local_gui_transport_state();
                self.server.tabs.goto_tab(index);
                self.sync_after_tab_change();
                self.publish_local_gui_after_ui_action(&before);
            }
            server::Command::NextTab => {
                let before = self.local_gui_transport_state();
                self.server.tabs.next_tab();
                self.sync_after_tab_change();
                self.publish_local_gui_after_ui_action(&before);
            }
            server::Command::PrevTab => {
                let before = self.local_gui_transport_state();
                self.server.tabs.prev_tab();
                self.sync_after_tab_change();
                self.publish_local_gui_after_ui_action(&before);
            }
            server::Command::ResizeViewportPoints { width, height } => {
                let before = self.local_gui_transport_state();
                let changed = self.resize_viewport_points(width, height);
                if changed {
                    self.publish_local_gui_after_ui_action(&before);
                }
            }
            server::Command::ResizeViewport { cols, rows } => {
                let before = self.local_gui_transport_state();
                let changed = self.resize_viewport_cells(cols, rows);
                if changed {
                    self.publish_local_gui_after_ui_action(&before);
                }
            }
            server::Command::ResizeFocused { cols, rows } => {
                let pane = self.server.tabs.focused_pane();
                let (width, height) = self.session_size_pixels(cols, rows);
                self.resize_pane_backend(pane, self.scale_factor(), width, height);
            }
            server::Command::SendKey { keyspec } => {
                let before = self.local_gui_transport_state();
                self.inject_key(&keyspec);
                self.publish_local_gui_after_ui_action(&before);
            }
            server::Command::RemoteConnected { client_id } => {
                if let Some(server) = self.server.local_gui_server.as_ref()
                    && server.client_is_local(client_id)
                {
                    if server.client_session(client_id).is_none()
                        && let Some(session_id) = self.server.tabs.active_session_id()
                        && self.pane_for_session(session_id).is_some()
                    {
                        self.bootstrap_local_stream_client(server, client_id, session_id);
                        self.publish_remote_session(session_id);
                    }
                    server.send_session_list(client_id, &self.remote_sessions());
                }
            }
            server::Command::RemoteListSessions { client_id } => {
                if let Some(server) = self.server.local_gui_server.as_ref()
                    && server.client_is_local(client_id)
                    && server.client_session(client_id).is_none()
                    && let Some(session_id) = self.server.tabs.active_session_id()
                    && self.pane_for_session(session_id).is_some()
                {
                    self.bootstrap_local_stream_client(server, client_id, session_id);
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
                        if server.client_is_local(client_id) {
                            self.bootstrap_local_stream_client(server, client_id, session_id);
                        } else {
                            server.send_attached(client_id, session_id);
                        }
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
                if pane == self.server.tabs.focused_pane() {
                    self.write_terminal_input(&bytes);
                } else {
                    let _ = self.backend.write_input(pane, &bytes);
                }
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
                        server.send_ui_runtime_state(client_id, &self.ui_runtime_state());
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
                    server.send_ui_runtime_state(client_id, &self.ui_runtime_state());
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
                    server.send_ui_runtime_state(client_id, &self.ui_runtime_state());
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
                    server.send_ui_runtime_state(client_id, &self.ui_runtime_state());
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
        let servers = self.remote_servers().collect::<Vec<_>>();
        if servers.is_empty() {
            return;
        }
        let Some(pane) = self.pane_for_session(session_id) else {
            for server in servers {
                server.send_session_exited(session_id);
            }
            log_server_latency("publish_remote_session", started_at);
            return;
        };
        let Some(state) = self.remote_full_state_for_pane(pane.id()) else {
            for server in servers {
                server.send_session_exited(session_id);
            }
            log_server_latency("publish_remote_session", started_at);
            return;
        };
        let needs_local_pane_states = servers
            .iter()
            .any(|server| server.local_attached_to_session(session_id));
        let pane_states = if needs_local_pane_states {
            self.server
                .tabs
                .find_index_by_session_id(session_id)
                .and_then(|tab_index| self.server.tabs.tab_tree(tab_index))
                .map(|tree| {
                    let focused_pane_id = tree.focused_pane().id();
                    tree.export_panes()
                        .into_iter()
                        .filter_map(|exported| {
                        let pane_id = exported.pane.id();
                        if pane_id == focused_pane_id {
                            return None;
                        }
                        self.remote_full_state_for_pane(pane_id)
                            .map(|state| (pane_id, state))
                    })
                    .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        if needs_local_pane_states {
            let visible_pane_ids = pane_states.iter().map(|(pane_id, _)| *pane_id).collect::<Vec<_>>();
            for server in &servers {
                server.retain_local_attached_pane_states(session_id, &visible_pane_ids);
            }
        }
        for server in servers {
            server.send_full_state_to_attached(session_id, Arc::clone(&state));
            for (pane_id, pane_state) in &pane_states {
                server.send_pane_state_to_local_attached(
                    session_id,
                    *pane_id,
                    Arc::clone(pane_state),
                );
            }
        }
        log_server_latency("publish_remote_session", started_at);
    }

}
