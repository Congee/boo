use super::*;
use crate::remote::RemoteErrorCode;
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
        "publish_remote_tab" => "server.remote.publish_tab",
        _ => "server.unknown",
    }
}

use std::sync::Arc;

#[derive(Clone, PartialEq, Eq)]
struct LocalGuiTransportState {
    visible_tab_id: Option<u32>,
    focused_pane_id: u64,
    pane_frames: Vec<(u64, u64, u64, u64, u64)>,
}

impl BooApp {
    fn remote_full_state_for_pane(&self, pane_id: u64) -> Option<Arc<remote::RemoteFullState>> {
        if let Some(snapshot) = self.backend.render_snapshot(pane_id) {
            return Some(Arc::new(remote::full_state_from_terminal(&snapshot)));
        }
        let snapshot = self.backend.ui_terminal_snapshot(pane_id)?;
        Some(Arc::new(remote::full_state_from_ui(&snapshot)))
    }

    fn publish_local_gui_runtime_state_for_active_tab(&mut self) {
        let Some(visible_tab_id) = self.server.tabs.active_tab_id() else {
            return;
        };
        if self.server.local_gui_server.is_none() {
            return;
        }
        let ui_state = self.ui_runtime_state();
        let retargeted = {
            let server = self.server.local_gui_server.as_ref().expect("local gui server");
            server.retarget_local_viewing_tab(visible_tab_id)
        };
        if retargeted {
            self.invalidate_remote_tabs_cache();
        }
        let tabs = self.current_remote_tabs();
        let server = self.server.local_gui_server.as_ref().expect("local gui server");
        server.send_ui_runtime_state_to_local_viewers(visible_tab_id, &ui_state);
        server.send_tab_list_to_local_clients(tabs.as_ref());
    }

    fn local_gui_transport_state(&self) -> LocalGuiTransportState {
        let visible_tab_id = self.server.tabs.active_tab_id();
        let focused_pane_id = self.server.tabs.focused_pane().id();
        let pane_frames = visible_tab_id
            .and_then(|visible_tab_id| self.server.tabs.find_index_by_tab_id(visible_tab_id))
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
            visible_tab_id,
            focused_pane_id,
            pane_frames,
        }
    }

    fn publish_local_gui_after_ui_action(&mut self, before: &LocalGuiTransportState) {
        self.publish_local_gui_runtime_state_for_active_tab();
        let after = self.local_gui_transport_state();
        if after != *before && let Some(visible_tab_id) = after.visible_tab_id {
            self.publish_remote_tab(visible_tab_id);
        }
    }

    fn bootstrap_local_stream_client(&self, server: &remote::RemoteServer, client_id: u64, visible_tab_id: u32) {
        server.set_client_visible_tab(client_id, visible_tab_id);
        server.send_ui_appearance(client_id, &self.ui_appearance_snapshot());
        server.send_ui_runtime_state(client_id, &self.ui_runtime_state());
    }

    pub(crate) fn remote_servers(&self) -> impl Iterator<Item = &remote::RemoteServer> {
        self.server
            .remote_server
            .iter()
            .chain(self.server.local_gui_server.iter())
    }

    pub(crate) fn has_runtime_stream_subscribers(&self) -> bool {
        self.remote_servers().any(|server| server.has_runtime_viewers())
    }

    fn remote_server_for_client(&self, client_id: u64) -> Option<&remote::RemoteServer> {
        self.remote_servers().find(|server| server.has_client(client_id))
    }

    pub(crate) fn handle_server_cmd(&mut self, cmd: server::Command) {
        match cmd {
            server::Command::DumpKeys(enabled) => self.dump_keys = enabled,
            server::Command::Ping => {}
            server::Command::GetRemoteClients { reply } => {
                let mut snapshot = crate::remote::RemoteClientsSnapshot {
                    servers: Vec::new(),
                    clients: Vec::new(),
                };
                for server in self.remote_servers() {
                    let server_snapshot = server.clients_snapshot();
                    snapshot.servers.extend(server_snapshot.servers);
                    snapshot.clients.extend(server_snapshot.clients);
                }
                snapshot.servers.sort_by(|a, b| {
                    a.local_socket_path
                        .cmp(&b.local_socket_path)
                        .then_with(|| a.server_instance_id.cmp(&b.server_instance_id))
                });
                snapshot.clients.sort_by_key(|client| client.client_id);
                let _ = reply.send(control::Response::RemoteClients { snapshot });
            }
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
            server::Command::SetStatusComponents {
                zone,
                source,
                components,
            } => {
                if self.status_components.set(crate::status_components::StatusComponentsUpdate {
                    zone,
                    source,
                    components,
                }) {
                    if self.has_runtime_stream_subscribers() {
                        self.mark_active_remote_tab_dirty();
                    }
                }
            }
            server::Command::ClearStatusComponents { source, zone } => {
                if self.status_components.clear(&source, zone)
                    && self.has_runtime_stream_subscribers()
                {
                    self.mark_active_remote_tab_dirty();
                }
            }
            server::Command::InvokeStatusComponent { source, id } => {
                let before = self.local_gui_transport_state();
                if self.invoke_status_component(&source, &id) {
                    self.publish_local_gui_after_ui_action(&before);
                }
            }
            server::Command::ExecuteCommand { input } => {
                self.invalidate_remote_tabs_cache();
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
                self.invalidate_remote_tabs_cache();
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
                let (width, height) = self.tab_size_pixels(cols, rows);
                self.resize_pane_backend(pane, self.scale_factor(), width, height);
            }
            server::Command::SendKey { keyspec } => {
                let before = self.local_gui_transport_state();
                self.inject_key(&keyspec);
                self.publish_local_gui_after_ui_action(&before);
            }
            server::Command::RemoteConnected { client_id } => {
                let bootstrap_tab_id = self.server.tabs.active_tab_id().filter(|visible_tab_id| {
                    self.pane_for_tab(*visible_tab_id).is_some()
                        && self
                            .remote_server_for_client(client_id)
                            .or(self.server.local_gui_server.as_ref())
                            .or(self.server.remote_server.as_ref())
                            .is_some_and(|server| server.client_visible_tab(client_id).is_none())
                });
                if let Some(visible_tab_id) = bootstrap_tab_id
                    && let Some(server) = self
                        .remote_server_for_client(client_id)
                        .or(self.server.local_gui_server.as_ref())
                        .or(self.server.remote_server.as_ref())
                {
                    if server.client_is_local(client_id) {
                        self.bootstrap_local_stream_client(server, client_id, visible_tab_id);
                    } else {
                        server.set_client_visible_tab(client_id, visible_tab_id);
                    }
                    self.publish_remote_tab(visible_tab_id);
                }
                let tabs = self.current_remote_tabs();
                if let Some(server) = self
                    .remote_server_for_client(client_id)
                    .or(self.server.local_gui_server.as_ref())
                    .or(self.server.remote_server.as_ref())
                {
                    server.reply_tab_list(client_id, tabs.as_ref());
                    server.send_ui_runtime_state(client_id, &self.ui_runtime_state());
                    server.send_ui_appearance(client_id, &self.ui_appearance_snapshot());
                }
            }
            server::Command::RemoteListTabs { client_id } => {
                let bootstrap_tab_id = self.server.tabs.active_tab_id().filter(|visible_tab_id| {
                    self.pane_for_tab(*visible_tab_id).is_some()
                        && self
                            .remote_server_for_client(client_id)
                            .or(self.server.local_gui_server.as_ref())
                            .or(self.server.remote_server.as_ref())
                            .is_some_and(|server| server.client_visible_tab(client_id).is_none())
                });
                if let Some(visible_tab_id) = bootstrap_tab_id
                    && let Some(server) = self
                        .remote_server_for_client(client_id)
                        .or(self.server.local_gui_server.as_ref())
                        .or(self.server.remote_server.as_ref())
                {
                    if server.client_is_local(client_id) {
                        self.bootstrap_local_stream_client(server, client_id, visible_tab_id);
                    } else {
                        server.set_client_visible_tab(client_id, visible_tab_id);
                    }
                    self.publish_remote_tab(visible_tab_id);
                }
                let tabs = self.current_remote_tabs();
                if let Some(server) = self
                    .remote_server_for_client(client_id)
                    .or(self.server.local_gui_server.as_ref())
                    .or(self.server.remote_server.as_ref())
                {
                    server.reply_tab_list(client_id, tabs.as_ref());
                    server.send_ui_runtime_state(client_id, &self.ui_runtime_state());
                    server.send_ui_appearance(client_id, &self.ui_appearance_snapshot());
                }
            }
            server::Command::RemoteCreate {
                client_id,
                cols,
                rows,
            } => {
                log::info!(
                    "remote_create client_id={client_id} cols={cols} rows={rows} tabs_before={}",
                    self.server.tabs.len()
                );
                let created = self.new_tab();
                let Some(visible_tab_id) = created else {
                    log::warn!(
                        "remote_create_failed client_id={client_id} cols={cols} rows={rows} tabs_after={}",
                        self.server.tabs.len()
                    );
                    if let Some(server) = self
                        .remote_server_for_client(client_id)
                        .or(self.server.local_gui_server.as_ref())
                        .or(self.server.remote_server.as_ref())
                    {
                        server.send_error(
                            client_id,
                            RemoteErrorCode::FailedCreateTab,
                            "failed to create tab",
                        );
                    }
                    return;
                };
                if let Some(pane) = self.pane_for_tab(visible_tab_id) {
                    let (width, height) = self.tab_size_pixels(cols, rows);
                    self.resize_pane_backend(pane, self.scale_factor(), width, height);
                }
                self.invalidate_remote_tabs_cache();
                log::info!(
                    "remote_create_succeeded client_id={client_id} visible_tab_id={visible_tab_id} tabs_after={}",
                    self.server.tabs.len()
                );
                if let Some(server) = self
                    .remote_server_for_client(client_id)
                    .or(self.server.local_gui_server.as_ref())
                    .or(self.server.remote_server.as_ref())
                {
                    server.send_tab_created(client_id, visible_tab_id);
                    server.set_client_visible_tab(client_id, visible_tab_id);
                    self.publish_remote_tab(visible_tab_id);
                }
            }
            server::Command::RemoteInput {
                client_id,
                bytes,
                input_seq,
            } => {
                let started_at = Instant::now();
                let Some(visible_tab_id) = self
                    .remote_server_for_client(client_id)
                    .and_then(|server| server.client_visible_tab(client_id))
                else {
                    if let Some(server) = self
                        .remote_server_for_client(client_id)
                        .or(self.server.local_gui_server.as_ref())
                        .or(self.server.remote_server.as_ref())
                    {
                        server.send_error(client_id, RemoteErrorCode::NoActiveTab, "no active tab");
                    }
                    return;
                };
                let Some(pane) = self.pane_for_tab(visible_tab_id) else {
                    if let Some(server) = self
                        .remote_server_for_client(client_id)
                        .or(self.server.local_gui_server.as_ref())
                        .or(self.server.remote_server.as_ref())
                    {
                        server.send_tab_exited(visible_tab_id);
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
                self.mark_remote_tab_dirty(visible_tab_id);
                log_server_latency("remote_input_applied", started_at);
            }
            server::Command::RemoteKey {
                client_id,
                keyspec,
                input_seq,
            } => {
                let started_at = Instant::now();
                let Some(visible_tab_id) = self
                    .remote_server_for_client(client_id)
                    .and_then(|server| server.client_visible_tab(client_id))
                else {
                    if let Some(server) = self
                        .remote_server_for_client(client_id)
                        .or(self.server.local_gui_server.as_ref())
                        .or(self.server.remote_server.as_ref())
                    {
                        server.send_error(client_id, RemoteErrorCode::NoActiveTab, "no active tab");
                    }
                    return;
                };
                let Some(tab_index) = self.server.tabs.find_index_by_tab_id(visible_tab_id) else {
                    if let Some(server) = self
                        .remote_server_for_client(client_id)
                        .or(self.server.local_gui_server.as_ref())
                        .or(self.server.remote_server.as_ref())
                    {
                        server.send_tab_exited(visible_tab_id);
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
                self.mark_remote_tab_dirty(visible_tab_id);
                log_server_latency("remote_key_applied", started_at);
            }
            server::Command::RemoteResize {
                client_id,
                cols,
                rows,
            } => {
                let Some(visible_tab_id) = self
                    .remote_server_for_client(client_id)
                    .and_then(|server| server.client_visible_tab(client_id))
                else {
                    if let Some(server) = self
                        .remote_server_for_client(client_id)
                        .or(self.server.local_gui_server.as_ref())
                        .or(self.server.remote_server.as_ref())
                    {
                        server.send_error(client_id, RemoteErrorCode::NoActiveTab, "no active tab");
                    }
                    return;
                };
                let Some(_pane) = self.pane_for_tab(visible_tab_id) else {
                    if let Some(server) = self
                        .remote_server_for_client(client_id)
                        .or(self.server.local_gui_server.as_ref())
                        .or(self.server.remote_server.as_ref())
                    {
                        server.send_tab_exited(visible_tab_id);
                    }
                    return;
                };
                if self.resize_viewport_cells(cols, rows) {
                    self.publish_remote_tab(visible_tab_id);
                }
            }
            server::Command::RemoteExecuteCommand { client_id, input } => {
                self.invalidate_remote_tabs_cache();
                self.execute_command(&input);
                let focused_tab_id = self.server.tabs.active_tab_id();
                let ui_state = self.ui_runtime_state();
                let tabs = self.current_remote_tabs();
                if let Some(server) = self
                    .remote_server_for_client(client_id)
                    .or(self.server.local_gui_server.as_ref())
                    .or(self.server.remote_server.as_ref())
                {
                    server.send_ui_runtime_state(client_id, &ui_state);
                    server.send_tab_list(client_id, tabs.as_ref());
                    if let Some(visible_tab_id) = focused_tab_id {
                        server.set_client_visible_tab(client_id, visible_tab_id);
                        self.publish_remote_tab(visible_tab_id);
                    }
                }
            }
            server::Command::RemoteAppKeyEvent { client_id, event } => {
                let Some(visible_tab_id) = self
                    .remote_server_for_client(client_id)
                    .and_then(|server| server.client_visible_tab(client_id))
                else {
                    if let Some(server) = self
                        .remote_server_for_client(client_id)
                        .or(self.server.local_gui_server.as_ref())
                        .or(self.server.remote_server.as_ref())
                    {
                        server.send_error(client_id, RemoteErrorCode::NoActiveTab, "no active tab");
                    }
                    return;
                };
                let Some(tab_index) = self.server.tabs.find_index_by_tab_id(visible_tab_id) else {
                    if let Some(server) = self
                        .remote_server_for_client(client_id)
                        .or(self.server.local_gui_server.as_ref())
                        .or(self.server.remote_server.as_ref())
                    {
                        server.send_tab_exited(visible_tab_id);
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
                    let focused_tab_id = self.server.tabs.active_tab_id();
                    let ui_state = self.ui_runtime_state();
                    let tabs = self.current_remote_tabs();
                    if let Some(server) = self
                        .remote_server_for_client(client_id)
                        .or(self.server.local_gui_server.as_ref())
                        .or(self.server.remote_server.as_ref())
                    {
                        server.send_ui_runtime_state(client_id, &ui_state);
                        server.send_tab_list(client_id, tabs.as_ref());
                        if let Some(visible_tab_id) = focused_tab_id {
                            server.set_client_visible_tab(client_id, visible_tab_id);
                            self.publish_remote_tab(visible_tab_id);
                        }
                    }
                }
            }
            server::Command::RemoteAppMouseEvent { client_id, event } => {
                let should_republish_tab = matches!(
                    event,
                    crate::app_input::AppMouseEvent::WheelScrolledLines { .. }
                        | crate::app_input::AppMouseEvent::WheelScrolledPixels { .. }
                );
                let Some(visible_tab_id) = self
                    .remote_server_for_client(client_id)
                    .and_then(|server| server.client_visible_tab(client_id))
                else {
                    if let Some(server) = self
                        .remote_server_for_client(client_id)
                        .or(self.server.local_gui_server.as_ref())
                        .or(self.server.remote_server.as_ref())
                    {
                        server.send_error(client_id, RemoteErrorCode::NoActiveTab, "no active tab");
                    }
                    return;
                };
                let Some(tab_index) = self.server.tabs.find_index_by_tab_id(visible_tab_id) else {
                    if let Some(server) = self
                        .remote_server_for_client(client_id)
                        .or(self.server.local_gui_server.as_ref())
                        .or(self.server.remote_server.as_ref())
                    {
                        server.send_tab_exited(visible_tab_id);
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
                let ui_state = self.ui_runtime_state();
                let tabs = self.current_remote_tabs();
                if let Some(server) = self
                    .remote_server_for_client(client_id)
                    .or(self.server.local_gui_server.as_ref())
                    .or(self.server.remote_server.as_ref())
                {
                    if changed_ui {
                        server.send_ui_runtime_state(client_id, &ui_state);
                        server.send_tab_list(client_id, tabs.as_ref());
                    }
                    if changed_ui || should_republish_tab {
                        server.set_client_visible_tab(client_id, visible_tab_id);
                        self.publish_remote_tab(visible_tab_id);
                    }
                }
            }
            server::Command::RemoteAppAction { client_id, action } => {
                self.dispatch_binding_action(action);
                let focused_tab_id = self.server.tabs.active_tab_id();
                let ui_state = self.ui_runtime_state();
                let tabs = self.current_remote_tabs();
                if let Some(server) = self
                    .remote_server_for_client(client_id)
                    .or(self.server.local_gui_server.as_ref())
                    .or(self.server.remote_server.as_ref())
                {
                    server.send_ui_runtime_state(client_id, &ui_state);
                    server.send_tab_list(client_id, tabs.as_ref());
                    if let Some(visible_tab_id) = focused_tab_id {
                        server.set_client_visible_tab(client_id, visible_tab_id);
                        self.publish_remote_tab(visible_tab_id);
                    }
                }
            }
            server::Command::RemoteFocusPane { client_id, pane_id } => {
                let Some(visible_tab_id) = self
                    .remote_server_for_client(client_id)
                    .and_then(|server| server.client_visible_tab(client_id))
                else {
                    if let Some(server) = self
                        .remote_server_for_client(client_id)
                        .or(self.server.local_gui_server.as_ref())
                        .or(self.server.remote_server.as_ref())
                    {
                        server.send_error(client_id, RemoteErrorCode::NoActiveTab, "no active tab");
                    }
                    return;
                };
                let Some(tab_index) = self.server.tabs.find_index_by_tab_id(visible_tab_id) else {
                    if let Some(server) = self
                        .remote_server_for_client(client_id)
                        .or(self.server.local_gui_server.as_ref())
                        .or(self.server.remote_server.as_ref())
                    {
                        server.send_tab_exited(visible_tab_id);
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
                {
                    let ui_state = self.ui_runtime_state();
                    let tabs = self.current_remote_tabs();
                    if let Some(server) = self
                        .remote_server_for_client(client_id)
                        .or(self.server.local_gui_server.as_ref())
                        .or(self.server.remote_server.as_ref())
                    {
                    server.send_ui_runtime_state(client_id, &ui_state);
                    server.send_tab_list(client_id, tabs.as_ref());
                    server.set_client_visible_tab(client_id, visible_tab_id);
                    self.publish_remote_tab(visible_tab_id);
                    }
                }
            }
            server::Command::RemoteDestroy {
                client_id,
                tab_id,
            } => {
                let target = tab_id.or_else(|| {
                    self.remote_server_for_client(client_id)
                        .and_then(|server| server.client_visible_tab(client_id))
                });
                let Some(target) = target else {
                    if let Some(server) = self
                        .remote_server_for_client(client_id)
                        .or(self.server.local_gui_server.as_ref())
                        .or(self.server.remote_server.as_ref())
                    {
                        server.send_error(client_id, RemoteErrorCode::UnknownTab, "unknown tab");
                    }
                    return;
                };
                log::info!(
                    "remote_destroy client_id={client_id} requested_tab={tab_id:?} resolved_tab={target} tabs_before={}",
                    self.server.tabs.len()
                );
                let Some(tab_index) = self.server.tabs.find_index_by_tab_id(target) else {
                    if let Some(server) = self
                        .remote_server_for_client(client_id)
                        .or(self.server.local_gui_server.as_ref())
                        .or(self.server.remote_server.as_ref())
                    {
                        server.send_tab_exited(target);
                    }
                    return;
                };
                let was_active = tab_index == self.server.tabs.active_index();
                let panes = self.server.tabs.remove_tab(tab_index);
                for pane in panes {
                    self.backend.free_pane(pane);
                }
                if was_active && !self.server.tabs.is_empty() {
                    self.sync_after_tab_change();
                }
                self.invalidate_remote_tabs_cache();
                let tabs = self.current_remote_tabs();
                let focused_tab_id = self.server.tabs.active_tab_id();
                let ui_state = self.ui_runtime_state();
                log::info!(
                    "remote_destroy_done client_id={client_id} destroyed_tab={target} tabs_after={} focused_after={focused_tab_id:?}",
                    self.server.tabs.len()
                );
                if let Some(server) = self
                    .remote_server_for_client(client_id)
                    .or(self.server.local_gui_server.as_ref())
                    .or(self.server.remote_server.as_ref())
                {
                    server.send_tab_exited(target);
                    server.send_ui_runtime_state(client_id, &ui_state);
                    server.send_tab_list(client_id, tabs.as_ref());
                    if let Some(visible_tab_id) = focused_tab_id {
                        server.set_client_visible_tab(client_id, visible_tab_id);
                        self.publish_remote_tab(visible_tab_id);
                    }
                }
            }
        }
    }

    pub(crate) fn remote_tabs(&self) -> Vec<remote::RemoteTabInfo> {
        self.server
            .tabs
            .tab_identity_info()
            .into_iter()
            .map(|tab| {
                let pane = self
                    .server
                    .tabs
                    .tab_tree(tab.index)
                    .map(|tree| tree.focused_pane())
                    .unwrap_or(PaneHandle::null());
                let terminal = self.backend.ui_terminal_snapshot(pane.id());
                remote::RemoteTabInfo {
                    id: tab.id,
                    name: format!("Tab {}", tab.index + 1),
                    title: tab.title,
                    pwd: terminal
                        .as_ref()
                        .map(|snapshot| snapshot.pwd.clone())
                        .unwrap_or_default(),
                    child_exited: pane.id() == 0 || terminal.is_none(),
                }
            })
            .collect()
    }

    pub(crate) fn pane_for_tab(&self, visible_tab_id: u32) -> Option<PaneHandle> {
        let tab_index = self.server.tabs.find_index_by_tab_id(visible_tab_id)?;
        self.server
            .tabs
            .tab_tree(tab_index)
            .map(|tree| tree.focused_pane())
    }

    pub(crate) fn tab_size_pixels(&self, cols: u16, rows: u16) -> (u32, u32) {
        let width = (cols as f64 * self.cell_width).round().max(1.0) as u32;
        let height = (rows as f64 * self.cell_height).round().max(1.0) as u32;
        (width, height)
    }

    pub(crate) fn publish_remote_tab(&self, visible_tab_id: u32) {
        let started_at = Instant::now();
        let servers = self.remote_servers().collect::<Vec<_>>();
        if servers.is_empty() {
            return;
        }
        let Some(pane) = self.pane_for_tab(visible_tab_id) else {
            for server in servers {
                server.send_tab_exited(visible_tab_id);
            }
            log_server_latency("publish_remote_tab", started_at);
            return;
        };
        let Some(state) = self.remote_full_state_for_pane(pane.id()) else {
            for server in servers {
                server.send_tab_exited(visible_tab_id);
            }
            log_server_latency("publish_remote_tab", started_at);
            return;
        };
        let needs_local_pane_states = servers
            .iter()
            .any(|server| server.local_viewing_tab(visible_tab_id));
        let pane_states = if needs_local_pane_states {
            self.server
                .tabs
                .find_index_by_tab_id(visible_tab_id)
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
                server.retain_local_subscribed_pane_states(visible_tab_id, &visible_pane_ids);
            }
        }
        for server in servers {
            server.send_full_state_to_viewers(visible_tab_id, Arc::clone(&state));
            for (pane_id, pane_state) in &pane_states {
                server.send_pane_state_to_local_viewers(
                    visible_tab_id,
                    *pane_id,
                    Arc::clone(pane_state),
                );
            }
        }
        log_server_latency("publish_remote_tab", started_at);
    }

}
