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
    active_tab_id: Option<u32>,
    focused_pane_id: u64,
    pane_frames: Vec<(u64, u64, u64, u64, u64)>,
}

impl BooApp {
    fn bump_runtime_revision(&mut self) -> u64 {
        self.runtime_revision = self.runtime_revision.wrapping_add(1).max(1);
        self.runtime_revision
    }

    fn remote_view_state_for_client(
        &self,
        client_id: u64,
    ) -> Option<crate::remote::ClientRuntimeViewSnapshot> {
        self.remote_server_for_client(client_id)
            .and_then(|server| server.client_runtime_view(client_id))
    }

    fn ensure_remote_client_view_state(
        &mut self,
        client_id: u64,
    ) -> Option<crate::remote::ClientRuntimeViewSnapshot> {
        let mut snapshot = self.remote_view_state_for_client(client_id)?;
        if snapshot.view_id == 0 {
            let viewed_tab_id = self.active_runtime_tab_id();
            let focused_pane_id = viewed_tab_id.and_then(|tab_id| self.default_focused_pane_for_tab(tab_id));
            let visible_pane_ids = viewed_tab_id
                .map(|tab_id| self.pane_ids_for_tab(tab_id))
                .unwrap_or_default();
            if let Some(server) = self.remote_server_for_client(client_id) {
                server.initialize_client_view(client_id, viewed_tab_id, focused_pane_id, &visible_pane_ids);
                snapshot = server.client_runtime_view(client_id)?;
            }
        }
        if let Some(viewed_tab_id) = snapshot.viewed_tab_id {
            let valid_panes = self.pane_ids_for_tab(viewed_tab_id);
            let needs_focus_repair = snapshot
                .focused_pane_id
                .is_none_or(|pane_id| !valid_panes.contains(&pane_id));
            let needs_visible_repair = snapshot.visible_pane_ids != valid_panes;
            if needs_focus_repair || needs_visible_repair {
                let focused_pane_id = snapshot
                    .focused_pane_id
                    .filter(|pane_id| valid_panes.contains(pane_id))
                    .or_else(|| self.default_focused_pane_for_tab(viewed_tab_id));
                if let Some(server) = self.remote_server_for_client(client_id) {
                    server.update_client_view(client_id, |view| {
                        view.viewed_tab_id = Some(viewed_tab_id);
                        view.focused_pane_id = focused_pane_id;
                        view.visible_pane_ids = valid_panes.clone();
                        view.touch_view();
                    });
                    snapshot = server.client_runtime_view(client_id)?;
                }
            }
        }
        Some(snapshot)
    }

    fn ui_runtime_state_for_client(&self, client_id: u64) -> control::UiRuntimeState {
        let view = self.remote_view_state_for_client(client_id);
        let viewed_tab_id = view.as_ref().and_then(|view| view.viewed_tab_id).or_else(|| self.active_runtime_tab_id());
        let focused_pane = viewed_tab_id
            .and_then(|tab_id| {
                view.as_ref()
                    .and_then(|view| view.focused_pane_id)
                    .filter(|pane_id| self.pane_ids_for_tab(tab_id).contains(pane_id))
                    .or_else(|| self.default_focused_pane_for_tab(tab_id))
            })
            .unwrap_or_default();
        let visible_panes = viewed_tab_id
            .map(|tab_id| self.visible_pane_snapshots_for(tab_id, focused_pane))
            .unwrap_or_default();
        let active_tab = viewed_tab_id
            .and_then(|tab_id| self.server.tabs.find_index_by_tab_id(tab_id))
            .unwrap_or_else(|| self.server.tabs.active_index());
        control::UiRuntimeState {
            active_tab,
            focused_pane,
            tabs: self.runtime_tab_snapshots_for(viewed_tab_id),
            visible_panes: visible_panes.clone(),
            mouse_selection: self.ui_mouse_selection_snapshot(),
            status_bar: self.status_components.snapshot(),
            pwd: self.pwd.clone(),
            runtime_revision: self.runtime_revision,
            view_revision: view.as_ref().map(|view| view.view_revision).unwrap_or(1),
            view_id: view.as_ref().map(|view| view.view_id).unwrap_or(client_id),
            viewed_tab_id,
            viewport_cols: view.as_ref().and_then(|view| view.viewport_cols),
            viewport_rows: view.as_ref().and_then(|view| view.viewport_rows),
            visible_pane_ids: visible_panes.into_iter().map(|pane| pane.pane_id).collect(),
        }
    }

    fn repair_all_remote_views(&mut self, closed_tab_index: Option<usize>) {
        let client_ids = self
            .remote_servers()
            .flat_map(|server| server.clients_snapshot().clients.into_iter().map(|client| client.client_id))
            .collect::<Vec<_>>();
        for client_id in client_ids {
            let Some(server) = self.remote_server_for_client(client_id) else {
                continue;
            };
            let Some(view) = server.client_runtime_view(client_id) else {
                continue;
            };
            let viewed_tab_id = match view
                .viewed_tab_id
                .filter(|tab_id| self.server.tabs.find_index_by_tab_id(*tab_id).is_some())
            {
                Some(tab_id) => Some(tab_id),
                None if self.server.tabs.is_empty() => None,
                None => {
                    let fallback_index =
                        closed_tab_index.map_or(self.server.tabs.active_index(), |index| {
                            index.min(self.server.tabs.len().saturating_sub(1))
                        });
                    self.server.tabs.tab_id_for_index(fallback_index)
                }
            };
            let focused_pane_id = viewed_tab_id.and_then(|tab_id| {
                let pane_ids = self.pane_ids_for_tab(tab_id);
                view.focused_pane_id
                    .filter(|pane_id| pane_ids.contains(pane_id))
                    .or_else(|| self.default_focused_pane_for_tab(tab_id))
            });
            let visible_pane_ids = viewed_tab_id
                .map(|tab_id| self.pane_ids_for_tab(tab_id))
                .unwrap_or_default();
            server.update_client_view(client_id, |remote_view| {
                remote_view.viewed_tab_id = viewed_tab_id;
                remote_view.focused_pane_id = focused_pane_id;
                remote_view.visible_pane_ids = visible_pane_ids.clone();
                remote_view.touch_view();
            });
        }
    }

    fn pane_handle_by_id_any(&self, pane_id: u64) -> Option<PaneHandle> {
        let (tab_index, _) = self.server.tabs.find_pane_location(pane_id)?;
        self.server
            .tabs
            .tab_tree(tab_index)?
            .export_panes()
            .into_iter()
            .find(|pane| pane.pane.id() == pane_id)
            .map(|pane| pane.pane)
    }

    fn remote_server_for_delivery(&self, client_id: u64) -> Option<&remote::RemoteServer> {
        self.remote_server_for_client(client_id)
            .or(self.server.local_gui_server.as_ref())
            .or(self.server.remote_server.as_ref())
    }

    fn sync_remote_client_runtime_view(&mut self, client_id: u64, subscribe: bool) {
        let tabs = self.current_remote_tabs();
        let ui_appearance = self.ui_appearance_snapshot();
        let Some(server) = self.remote_server_for_delivery(client_id) else {
            return;
        };
        let viewed_tab_id = self.active_runtime_tab_id();
        let focused_pane_id = viewed_tab_id.and_then(|tab_id| self.default_focused_pane_for_tab(tab_id));
        let visible_pane_ids = viewed_tab_id
            .map(|tab_id| self.pane_ids_for_tab(tab_id))
            .unwrap_or_default();
        server.initialize_client_view(client_id, viewed_tab_id, focused_pane_id, &visible_pane_ids);
        let ui_runtime_state = self.ui_runtime_state_for_client(client_id);
        server.send_tab_list(client_id, tabs.as_ref());
        server.send_ui_runtime_state(client_id, &ui_runtime_state);
        server.send_ui_appearance(client_id, &ui_appearance);
        if subscribe {
            server.subscribe_client_to_runtime(client_id);
            if let Some(active_tab_id) = self.active_runtime_tab_id() {
                self.publish_remote_tab(active_tab_id);
            }
        } else {
            server.unsubscribe_client_from_runtime(client_id);
        }
    }

    fn recover_remote_client_runtime_view(&mut self, client_id: u64) {
        self.sync_remote_client_runtime_view(client_id, self.active_runtime_tab_id().is_some());
    }

    fn active_runtime_tab_id(&self) -> Option<u32> {
        self.server
            .tabs
            .active_tab_id()
            .filter(|tab_id| self.pane_for_tab(*tab_id).is_some())
    }

    fn focus_runtime_tab(&mut self, tab_id: u32) -> bool {
        let Some(tab_index) = self.server.tabs.find_index_by_tab_id(tab_id) else {
            return false;
        };
        let old = self.server.tabs.focused_pane();
        self.server.tabs.goto_tab(tab_index);
        let new = self.server.tabs.focused_pane();
        if old != new {
            self.set_pane_focus(old, false);
            self.set_pane_focus(new, true);
        }
        true
    }

    fn broadcast_runtime_view_to_all_viewers(&mut self) -> Option<u32> {
        let focused_tab_id = self.active_runtime_tab_id();
        let tabs = self.current_remote_tabs();
        let ui_appearance = self.ui_appearance_snapshot();
        self.bump_runtime_revision();
        self.remote_servers().for_each(|server| {
            server.send_tab_list_to_viewers(tabs.as_ref());
            server.send_ui_appearance_to_viewers(&ui_appearance);
        });
        let client_ids = self
            .remote_servers()
            .flat_map(|server| server.clients_snapshot().clients.into_iter().map(|client| client.client_id))
            .collect::<Vec<_>>();
        for client_id in client_ids {
            let ui_runtime_state = self.ui_runtime_state_for_client(client_id);
            if let Some(server) = self.remote_server_for_delivery(client_id) {
                server.send_ui_runtime_state(client_id, &ui_runtime_state);
            }
        }
        if let Some(active_tab_id) = focused_tab_id {
            self.publish_remote_tab(active_tab_id);
        }
        focused_tab_id
    }

    fn remote_full_state_for_pane(&self, pane_id: u64) -> Option<Arc<remote::RemoteFullState>> {
        if let Some(snapshot) = self.backend.render_snapshot(pane_id) {
            return Some(Arc::new(remote::full_state_from_terminal(&snapshot)));
        }
        let snapshot = self.backend.ui_terminal_snapshot(pane_id)?;
        Some(Arc::new(remote::full_state_from_ui(&snapshot)))
    }

    fn publish_local_gui_runtime_state_for_active_tab(&mut self) {
        if self.server.tabs.active_tab_id().is_none() {
            return;
        };
        if self.server.local_gui_server.is_none() {
            return;
        }
        let ui_state = self.ui_runtime_state();
        let tabs = self.current_remote_tabs();
        let server = self
            .server
            .local_gui_server
            .as_ref()
            .expect("local gui server");
        server.send_ui_runtime_state_to_local_viewers(&ui_state);
        server.send_tab_list_to_local_clients(tabs.as_ref());
    }

    fn local_gui_transport_state(&self) -> LocalGuiTransportState {
        let active_tab_id = self.server.tabs.active_tab_id();
        let focused_pane_id = self.server.tabs.focused_pane().id();
        let pane_frames = active_tab_id
            .and_then(|active_tab_id| self.server.tabs.find_index_by_tab_id(active_tab_id))
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
            active_tab_id,
            focused_pane_id,
            pane_frames,
        }
    }

    fn publish_local_gui_after_ui_action(&mut self, before: &LocalGuiTransportState) {
        self.publish_local_gui_runtime_state_for_active_tab();
        let after = self.local_gui_transport_state();
        if after != *before
            && let Some(active_tab_id) = after.active_tab_id
        {
            self.publish_remote_tab(active_tab_id);
        }
    }

    pub(crate) fn remote_servers(&self) -> impl Iterator<Item = &remote::RemoteServer> {
        self.server
            .remote_server
            .iter()
            .chain(self.server.local_gui_server.iter())
    }

    pub(crate) fn has_runtime_stream_subscribers(&self) -> bool {
        self.remote_servers()
            .any(|server| server.has_runtime_viewers())
    }

    fn remote_server_for_client(&self, client_id: u64) -> Option<&remote::RemoteServer> {
        self.remote_servers()
            .find(|server| server.has_client(client_id))
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
                if self
                    .status_components
                    .set(crate::status_components::StatusComponentsUpdate {
                        zone,
                        source,
                        components,
                    })
                    && self.has_runtime_stream_subscribers()
                {
                    self.mark_active_remote_tab_dirty();
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
                self.sync_remote_client_runtime_view(
                    client_id,
                    self.active_runtime_tab_id().is_some(),
                );
            }
            server::Command::RemoteListTabs { client_id } => {
                // Compatibility request: reply with tab metadata, but do not
                // let ListTabs choose or retarget a client-owned lifecycle
                // object. Runtime subscription/bootstrap is owned by
                // RemoteConnected and UiRuntimeState.
                let tabs = self.current_remote_tabs();
                let ui_runtime_state = self.ui_runtime_state_for_client(client_id);
                let ui_appearance = self.ui_appearance_snapshot();
                if let Some(server) = self.remote_server_for_delivery(client_id) {
                    server.reply_tab_list(client_id, tabs.as_ref());
                    server.send_ui_runtime_state(client_id, &ui_runtime_state);
                    server.send_ui_appearance(client_id, &ui_appearance);
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
                let Some(created_tab_id) = created else {
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
                if let Some(pane) = self.pane_for_tab(created_tab_id) {
                    let (width, height) = self.tab_size_pixels(cols, rows);
                    self.resize_pane_backend(pane, self.scale_factor(), width, height);
                }
                self.invalidate_remote_tabs_cache();
                log::info!(
                    "remote_create_succeeded client_id={client_id} created_tab_id={created_tab_id} tabs_after={}",
                    self.server.tabs.len()
                );
                if let Some(server) = self
                    .remote_server_for_client(client_id)
                    .or(self.server.local_gui_server.as_ref())
                    .or(self.server.remote_server.as_ref())
                {
                    server.send_tab_created(client_id, created_tab_id);
                }
                self.broadcast_runtime_view_to_all_viewers();
            }
            server::Command::RemoteInput {
                client_id,
                bytes,
                input_seq,
            } => {
                let started_at = Instant::now();
                let Some(view) = self.ensure_remote_client_view_state(client_id) else {
                    return;
                };
                let Some(active_tab_id) = view.viewed_tab_id else {
                    if let Some(server) = self
                        .remote_server_for_client(client_id)
                        .or(self.server.local_gui_server.as_ref())
                        .or(self.server.remote_server.as_ref())
                    {
                        server.send_error(client_id, RemoteErrorCode::NoActiveTab, "no active tab");
                    }
                    return;
                };
                let Some(pane_id) = view
                    .focused_pane_id
                    .filter(|pane_id| view.visible_pane_ids.contains(pane_id))
                    .or_else(|| self.default_focused_pane_for_tab(active_tab_id))
                else {
                    self.recover_remote_client_runtime_view(client_id);
                    return;
                };
                let Some(pane) = self.pane_handle_by_id_any(pane_id) else {
                    self.recover_remote_client_runtime_view(client_id);
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
                self.mark_remote_tab_dirty(active_tab_id);
                log_server_latency("remote_input_applied", started_at);
            }
            server::Command::RemoteKey {
                client_id,
                keyspec,
                input_seq,
            } => {
                let started_at = Instant::now();
                let Some(active_tab_id) = self.active_runtime_tab_id() else {
                    if let Some(server) = self
                        .remote_server_for_client(client_id)
                        .or(self.server.local_gui_server.as_ref())
                        .or(self.server.remote_server.as_ref())
                    {
                        server.send_error(client_id, RemoteErrorCode::NoActiveTab, "no active tab");
                    }
                    return;
                };
                if !self.focus_runtime_tab(active_tab_id) {
                    self.recover_remote_client_runtime_view(client_id);
                    return;
                }
                if let Some(server) = self.remote_server_for_client(client_id) {
                    server.record_input_seq(client_id, input_seq);
                }
                self.inject_key(&keyspec);
                self.mark_remote_tab_dirty(active_tab_id);
                log_server_latency("remote_key_applied", started_at);
            }
            server::Command::RemoteResize {
                client_id,
                cols,
                rows,
            } => {
                if let Some(server) = self.remote_server_for_client(client_id) {
                    server.update_client_view(client_id, |view| {
                        view.viewport_cols = Some(cols);
                        view.viewport_rows = Some(rows);
                        view.touch_view();
                    });
                }
                let Some(view) = self.ensure_remote_client_view_state(client_id) else {
                    return;
                };
                let Some(active_tab_id) = view.viewed_tab_id else {
                    if let Some(server) = self
                        .remote_server_for_client(client_id)
                        .or(self.server.local_gui_server.as_ref())
                        .or(self.server.remote_server.as_ref())
                    {
                        server.send_error(client_id, RemoteErrorCode::NoActiveTab, "no active tab");
                    }
                    return;
                };
                let Some(_pane) = self.pane_for_tab(active_tab_id) else {
                    self.recover_remote_client_runtime_view(client_id);
                    return;
                };
                if Some(active_tab_id) == self.active_runtime_tab_id() && self.resize_viewport_cells(cols, rows) {
                    self.publish_remote_tab(active_tab_id);
                }
            }
            server::Command::RemoteExecuteCommand { client_id, input } => {
                self.invalidate_remote_tabs_cache();
                self.execute_command(&input);
                let _ = client_id;
                self.broadcast_runtime_view_to_all_viewers();
            }
            server::Command::RemoteAppKeyEvent { client_id, event } => {
                let Some(active_tab_id) = self.active_runtime_tab_id() else {
                    if let Some(server) = self
                        .remote_server_for_client(client_id)
                        .or(self.server.local_gui_server.as_ref())
                        .or(self.server.remote_server.as_ref())
                    {
                        server.send_error(client_id, RemoteErrorCode::NoActiveTab, "no active tab");
                    }
                    return;
                };
                if !self.focus_runtime_tab(active_tab_id) {
                    self.recover_remote_client_runtime_view(client_id);
                    return;
                }
                if let Some(server) = self.remote_server_for_client(client_id) {
                    server.record_input_seq(client_id, event.input_seq);
                }
                let consumed = self.handle_app_key_event(event);
                if consumed {
                    self.broadcast_runtime_view_to_all_viewers();
                }
            }
            server::Command::RemoteAppMouseEvent { client_id, event } => {
                let should_republish_tab = matches!(
                    event,
                    crate::app_input::AppMouseEvent::WheelScrolledLines { .. }
                        | crate::app_input::AppMouseEvent::WheelScrolledPixels { .. }
                );
                let Some(active_tab_id) = self.active_runtime_tab_id() else {
                    if let Some(server) = self
                        .remote_server_for_client(client_id)
                        .or(self.server.local_gui_server.as_ref())
                        .or(self.server.remote_server.as_ref())
                    {
                        server.send_error(client_id, RemoteErrorCode::NoActiveTab, "no active tab");
                    }
                    return;
                };
                if !self.focus_runtime_tab(active_tab_id) {
                    self.recover_remote_client_runtime_view(client_id);
                    return;
                }
                let changed_ui = self.handle_app_mouse_event(event);
                if changed_ui || should_republish_tab {
                    let _ = self
                        .broadcast_runtime_view_to_all_viewers()
                        .unwrap_or(active_tab_id);
                }
            }
            server::Command::RemoteAppAction { client_id, action } => {
                self.dispatch_binding_action(action);
                let _ = client_id;
                self.broadcast_runtime_view_to_all_viewers();
            }
            server::Command::RemoteFocusPane { client_id, pane_id } => {
                let Some(mut view) = self.ensure_remote_client_view_state(client_id) else {
                    return;
                };
                let Some(active_tab_id) = view.viewed_tab_id else {
                    if let Some(server) = self
                        .remote_server_for_client(client_id)
                        .or(self.server.local_gui_server.as_ref())
                        .or(self.server.remote_server.as_ref())
                    {
                        server.send_error(client_id, RemoteErrorCode::NoActiveTab, "no active tab");
                    }
                    return;
                };
                let valid_panes = self.pane_ids_for_tab(active_tab_id);
                if valid_panes.contains(&pane_id) {
                    view.focused_pane_id = Some(pane_id);
                    view.visible_pane_ids = valid_panes.clone();
                    if let Some(server) = self.remote_server_for_client(client_id) {
                        server.update_client_view(client_id, |remote_view| {
                            remote_view.focused_pane_id = Some(pane_id);
                            remote_view.visible_pane_ids = valid_panes.clone();
                            remote_view.touch_view();
                        });
                    }
                    self.bump_runtime_revision();
                    let ui_runtime_state = self.ui_runtime_state_for_client(client_id);
                    if let Some(server) = self.remote_server_for_delivery(client_id) {
                        server.send_ui_runtime_state(client_id, &ui_runtime_state);
                    }
                    self.publish_remote_tab(active_tab_id);
                }
            }
            server::Command::RemoteDestroy { client_id, tab_id } => {
                let target = tab_id.or_else(|| self.active_runtime_tab_id());
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
                    self.recover_remote_client_runtime_view(client_id);
                    return;
                };
                let closed_tab_index = tab_index;
                let was_active = tab_index == self.server.tabs.active_index();
                let panes = self.server.tabs.remove_tab(tab_index);
                for pane in panes {
                    self.backend.free_pane(pane);
                }
                if was_active && !self.server.tabs.is_empty() {
                    self.sync_after_tab_change();
                }
                self.invalidate_remote_tabs_cache();
                self.repair_all_remote_views(Some(closed_tab_index));
                let focused_tab_id = self.broadcast_runtime_view_to_all_viewers();
                log::info!(
                    "remote_destroy_done client_id={client_id} destroyed_tab={target} tabs_after={} focused_after={focused_tab_id:?}",
                    self.server.tabs.len()
                );
                let _ = client_id;
            }
            server::Command::RemoteRuntimeAction { client_id, action } => match action {
                remote::RuntimeAction::SetViewedTab { view_id: _, tab_id } => {
                    let valid_panes = self.pane_ids_for_tab(tab_id);
                    let focused_pane_id = self.default_focused_pane_for_tab(tab_id);
                    if let Some(server) = self.remote_server_for_client(client_id) {
                        server.update_client_view(client_id, |view| {
                            view.viewed_tab_id = Some(tab_id);
                            view.focused_pane_id = focused_pane_id;
                            view.visible_pane_ids = valid_panes.clone();
                            view.touch_view();
                        });
                    }
                    self.bump_runtime_revision();
                    let ui_runtime_state = self.ui_runtime_state_for_client(client_id);
                    if let Some(server) = self.remote_server_for_delivery(client_id) {
                        server.send_ui_runtime_state(client_id, &ui_runtime_state);
                    }
                    self.publish_remote_tab(tab_id);
                }
                remote::RuntimeAction::FocusPane {
                    view_id: _,
                    tab_id,
                    pane_id,
                } => {
                    self.handle_server_cmd(server::Command::RemoteFocusPane { client_id, pane_id });
                    if let Some(server) = self.remote_server_for_client(client_id) {
                        server.update_client_view(client_id, |view| {
                            view.viewed_tab_id = Some(tab_id);
                        });
                    }
                }
                remote::RuntimeAction::NewTab { cols, rows, .. } => {
                    self.handle_server_cmd(server::Command::RemoteCreate {
                        client_id,
                        cols: cols.unwrap_or(120),
                        rows: rows.unwrap_or(36),
                    });
                }
                remote::RuntimeAction::CloseTab { tab_id, .. } => {
                    self.handle_server_cmd(server::Command::RemoteDestroy { client_id, tab_id });
                }
                remote::RuntimeAction::NextTab { .. } => {
                    let Some(view) = self.ensure_remote_client_view_state(client_id) else {
                        return;
                    };
                    let current_index = view
                        .viewed_tab_id
                        .and_then(|tab_id| self.server.tabs.find_index_by_tab_id(tab_id))
                        .unwrap_or_else(|| self.server.tabs.active_index());
                    let tab_count = self.server.tabs.len();
                    if tab_count == 0 {
                        return;
                    }
                    let next_index = (current_index + 1) % tab_count;
                    if let Some(tab_id) = self.server.tabs.tab_id_for_index(next_index) {
                        self.handle_server_cmd(server::Command::RemoteRuntimeAction {
                            client_id,
                            action: remote::RuntimeAction::SetViewedTab { view_id: view.view_id, tab_id },
                        });
                    }
                }
                remote::RuntimeAction::PrevTab { .. } => {
                    let Some(view) = self.ensure_remote_client_view_state(client_id) else {
                        return;
                    };
                    let current_index = view
                        .viewed_tab_id
                        .and_then(|tab_id| self.server.tabs.find_index_by_tab_id(tab_id))
                        .unwrap_or_else(|| self.server.tabs.active_index());
                    let tab_count = self.server.tabs.len();
                    if tab_count == 0 {
                        return;
                    }
                    let next_index = (current_index + tab_count - 1) % tab_count;
                    if let Some(tab_id) = self.server.tabs.tab_id_for_index(next_index) {
                        self.handle_server_cmd(server::Command::RemoteRuntimeAction {
                            client_id,
                            action: remote::RuntimeAction::SetViewedTab { view_id: view.view_id, tab_id },
                        });
                    }
                }
                remote::RuntimeAction::NewSplit { direction, .. } => {
                    let before = self.local_gui_transport_state();
                    self.create_split(Self::split_direction_from_str(direction.as_deref().unwrap_or("right")));
                    self.publish_local_gui_after_ui_action(&before);
                    self.broadcast_runtime_view_to_all_viewers();
                }
                remote::RuntimeAction::ResizeSplit { direction, amount, .. } => {
                    let direction = match direction.as_str() {
                        "left" => crate::bindings::Direction::Left,
                        "right" => crate::bindings::Direction::Right,
                        "up" => crate::bindings::Direction::Up,
                        _ => crate::bindings::Direction::Down,
                    };
                    self.dispatch_binding_action(crate::bindings::Action::ResizeSplit(
                        direction,
                        amount,
                    ));
                    self.broadcast_runtime_view_to_all_viewers();
                }
            },
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
                    active: self.server.tabs.active_index() == tab.index,
                    child_exited: pane.id() == 0 || terminal.is_none(),
                }
            })
            .collect()
    }

    pub(crate) fn pane_for_tab(&self, tab_id: u32) -> Option<PaneHandle> {
        let tab_index = self.server.tabs.find_index_by_tab_id(tab_id)?;
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

    pub(crate) fn publish_remote_tab(&mut self, tab_id: u32) {
        let started_at = Instant::now();
        let servers = self.remote_servers().collect::<Vec<_>>();
        if servers.is_empty() {
            return;
        }
        for server in servers {
            let client_ids = server.clients_snapshot().clients.into_iter().map(|client| client.client_id).collect::<Vec<_>>();
            for client_id in client_ids {
                let Some(view) = server.client_runtime_view(client_id) else {
                    continue;
                };
                if !view.subscribed_to_runtime || view.viewed_tab_id != Some(tab_id) {
                    continue;
                }
                let focused_pane_id = view
                    .focused_pane_id
                    .filter(|pane_id| view.visible_pane_ids.contains(pane_id))
                    .or_else(|| self.default_focused_pane_for_tab(tab_id));
                if let Some(focused_pane_id) = focused_pane_id
                    && let Some(state) = self.remote_full_state_for_pane(focused_pane_id)
                {
                    server.send_full_state_to_client(client_id, tab_id, Arc::clone(&state));
                }
                for pane_id in &view.visible_pane_ids {
                    if Some(*pane_id) == focused_pane_id {
                        continue;
                    }
                    if let Some(pane_state) = self.remote_full_state_for_pane(*pane_id) {
                        server.send_pane_state_to_client(
                            client_id,
                            tab_id,
                            *pane_id,
                            self.runtime_revision,
                            self.runtime_revision,
                            Arc::clone(&pane_state),
                        );
                    }
                }
            }
        }
        log_server_latency("publish_remote_tab", started_at);
    }
}
