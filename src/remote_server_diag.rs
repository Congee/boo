//! Diagnostic snapshot helpers for the remote daemon.
//!
//! This module owns the read-only projection of daemon/client state into the
//! public diagnostic RPC types used by CLI and debugging workflows.

use crate::remote_state::{
    AUTH_CHALLENGE_WINDOW, ClientState, DIRECT_CLIENT_HEARTBEAT_WINDOW, State,
};
use crate::remote_types::{RemoteClientInfo, RemoteClientsSnapshot, RemoteServerInfo};
use crate::remote_wire::{REMOTE_CAPABILITIES, REMOTE_PROTOCOL_VERSION, elapsed_ms, remaining_ms};
use std::path::Path;
use std::time::Instant;

pub(crate) fn clients_snapshot(
    state: &State,
    local_socket_path: Option<&Path>,
    bind_address: Option<&str>,
    port: Option<u16>,
) -> RemoteClientsSnapshot {
    let now = Instant::now();
    let connected_clients = state.clients.len();
    let viewing_clients = state
        .clients
        .values()
        .filter(|client| client.runtime_view.subscribed_to_runtime)
        .count();
    let pending_auth_clients = state
        .clients
        .values()
        .filter(|client| !client.authenticated)
        .count();
    let servers = vec![RemoteServerInfo {
        local_socket_path: local_socket_path.map(|path| path.display().to_string()),
        bind_address: bind_address.map(str::to_string),
        port,
        protocol_version: REMOTE_PROTOCOL_VERSION,
        capabilities: REMOTE_CAPABILITIES,
        build_id: env!("CARGO_PKG_VERSION").to_string(),
        server_instance_id: state.server_instance_id.clone(),
        server_identity_id: state.server_identity_id.clone(),
        auth_challenge_window_ms: AUTH_CHALLENGE_WINDOW.as_millis() as u64,
        heartbeat_window_ms: DIRECT_CLIENT_HEARTBEAT_WINDOW.as_millis() as u64,
        connected_clients,
        viewing_clients,
        pending_auth_clients,
    }];
    let mut clients = state
        .clients
        .iter()
        .map(|(client_id, client)| {
            client_info_for_client(state, now, local_socket_path, client_id, client)
        })
        .collect::<Vec<_>>();
    clients.sort_by_key(|client| client.client_id);

    RemoteClientsSnapshot { servers, clients }
}

fn client_info_for_client(
    _state: &State,
    now: Instant,
    local_socket_path: Option<&Path>,
    client_id: &u64,
    client: &ClientState,
) -> RemoteClientInfo {
    let heartbeat_deadline = if client.authenticated && !client.is_local {
        client
            .last_heartbeat_at
            .or(client.authenticated_at)
            .map(|last_liveness| last_liveness + DIRECT_CLIENT_HEARTBEAT_WINDOW)
    } else {
        None
    };
    RemoteClientInfo {
        client_id: *client_id,
        authenticated: client.authenticated,
        is_local: client.is_local,
        transport_kind: if client.is_local {
            "local".to_string()
        } else {
            "tcp".to_string()
        },
        server_socket_path: local_socket_path.map(|path| path.display().to_string()),
        challenge_pending: false,
        subscribed_to_runtime: client.runtime_view.subscribed_to_runtime,
        view_id: client.runtime_view.view_id,
        viewed_tab_id: client.runtime_view.viewed_tab_id,
        focused_pane_id: client.runtime_view.focused_pane_id,
        visible_pane_count: client.runtime_view.visible_pane_ids.len(),
        has_cached_state: client.runtime_view.last_state.is_some(),
        pane_state_count: client.runtime_view.pane_states.len(),
        latest_input_seq: client.runtime_view.latest_input_seq,
        connection_age_ms: elapsed_ms(now, client.connected_at),
        authenticated_age_ms: client
            .authenticated_at
            .map(|authenticated_at| elapsed_ms(now, authenticated_at)),
        last_heartbeat_age_ms: client
            .last_heartbeat_at
            .map(|last_heartbeat_at| elapsed_ms(now, last_heartbeat_at)),
        heartbeat_expires_in_ms: heartbeat_deadline.map(|deadline| remaining_ms(now, deadline)),
        heartbeat_overdue: heartbeat_deadline.is_some_and(|deadline| now >= deadline),
        challenge_expires_in_ms: None,
    }
}

#[cfg(test)]
mod tests {
    use super::clients_snapshot;
    use crate::remote_state::{
        ClientRuntimeView, ClientState, DIRECT_CLIENT_HEARTBEAT_WINDOW, State,
    };
    use crate::remote_wire::{
        REMOTE_CAPABILITIES, REMOTE_PROTOCOL_VERSION, RemoteCell, RemoteFullState,
    };
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, mpsc};
    use std::time::{Duration, Instant};

    fn empty_state() -> State {
        State::test_empty()
    }

    fn remote_client(
        outbound: mpsc::Sender<crate::remote_batcher::OutboundMessage>,
    ) -> ClientState {
        ClientState::test_client(outbound, true, false)
    }

    #[test]
    fn client_info_reports_remote_client_diagnostics() {
        let (tx, _rx) = mpsc::channel();
        let mut state = empty_state();
        state.clients.insert(
            7,
            ClientState {
                last_heartbeat_at: Some(Instant::now()),
                runtime_view: ClientRuntimeView {
                    view_id: 17,
                    subscribed_to_runtime: true,
                    viewed_tab_id: Some(2),
                    focused_pane_id: Some(22),
                    visible_pane_ids: vec![22, 23],
                    last_state: Some(Arc::new(RemoteFullState {
                        rows: 1,
                        cols: 1,
                        cursor_x: 0,
                        cursor_y: 0,
                        cursor_visible: true,
                        cursor_blinking: false,
                        cursor_style: 1,
                        cells: vec![RemoteCell {
                            codepoint: u32::from('x'),
                            fg: [1, 2, 3],
                            bg: [0, 0, 0],
                            style_flags: 0,
                            wide: false,
                        }],
                    })),
                    pane_states: HashMap::from([(
                        22,
                        Arc::new(RemoteFullState {
                            rows: 1,
                            cols: 1,
                            cursor_x: 0,
                            cursor_y: 0,
                            cursor_visible: true,
                            cursor_blinking: false,
                            cursor_style: 1,
                            cells: vec![],
                        }),
                    )]),
                    latest_input_seq: Some(9),
                    ..ClientRuntimeView::idle()
                },
                ..remote_client(tx)
            },
        );

        let snapshot = clients_snapshot(&state, None, None, None);
        assert_eq!(snapshot.servers.len(), 1);
        let server_info = &snapshot.servers[0];
        assert_eq!(server_info.protocol_version, REMOTE_PROTOCOL_VERSION);
        assert_eq!(server_info.capabilities, REMOTE_CAPABILITIES);
        assert_eq!(server_info.build_id, env!("CARGO_PKG_VERSION"));
        assert_eq!(server_info.server_instance_id, "test-instance");
        assert_eq!(server_info.server_identity_id, "test-daemon");
        assert_eq!(
            server_info.heartbeat_window_ms,
            DIRECT_CLIENT_HEARTBEAT_WINDOW.as_millis() as u64
        );
        assert_eq!(server_info.connected_clients, 1);
        assert_eq!(server_info.viewing_clients, 1);
        assert_eq!(server_info.pending_auth_clients, 0);
        assert_eq!(snapshot.clients.len(), 1);
        let client = &snapshot.clients[0];
        assert_eq!(client.client_id, 7);
        assert!(client.authenticated);
        assert!(!client.is_local);
        assert_eq!(client.transport_kind, "tcp");
        assert_eq!(client.server_socket_path, None);
        assert!(!client.challenge_pending);
        assert!(client.subscribed_to_runtime);
        assert_eq!(client.view_id, 17);
        assert_eq!(client.viewed_tab_id, Some(2));
        assert_eq!(client.focused_pane_id, Some(22));
        assert_eq!(client.visible_pane_count, 2);
        assert!(client.has_cached_state);
        assert_eq!(client.pane_state_count, 1);
        assert_eq!(client.latest_input_seq, Some(9));
        assert!(client.connection_age_ms <= 250);
        assert!(client.authenticated_age_ms.is_some_and(|age| age <= 250));
        assert!(client.last_heartbeat_age_ms.is_some_and(|age| age <= 250));
        assert!(
            client
                .heartbeat_expires_in_ms
                .is_some_and(|ms| ms <= DIRECT_CLIENT_HEARTBEAT_WINDOW.as_millis() as u64)
        );
        assert!(!client.heartbeat_overdue);
        assert_eq!(client.challenge_expires_in_ms, None);
    }

    #[test]
    fn clients_snapshot_reports_overdue_direct_heartbeat() {
        let (tx, _rx) = mpsc::channel();
        let mut state = empty_state();
        state.clients.insert(
            1,
            ClientState {
                connected_at: Instant::now() - Duration::from_secs(30),
                authenticated_at: Some(
                    Instant::now() - DIRECT_CLIENT_HEARTBEAT_WINDOW - Duration::from_secs(1),
                ),
                last_heartbeat_at: Some(
                    Instant::now() - DIRECT_CLIENT_HEARTBEAT_WINDOW - Duration::from_secs(1),
                ),
                ..remote_client(tx)
            },
        );

        let snapshot = clients_snapshot(&state, None, None, None);
        assert_eq!(snapshot.servers.len(), 1);
        assert_eq!(snapshot.servers[0].connected_clients, 1);
        assert_eq!(snapshot.servers[0].viewing_clients, 0);
        assert_eq!(snapshot.servers[0].pending_auth_clients, 0);
        assert_eq!(snapshot.clients.len(), 1);
        let client = &snapshot.clients[0];
        assert_eq!(client.transport_kind, "tcp");
        assert_eq!(client.server_socket_path, None);
        assert!(client.heartbeat_overdue);
        assert_eq!(client.heartbeat_expires_in_ms, Some(0));
    }

    #[test]
    fn clients_snapshot_recovers_after_fresh_direct_heartbeat() {
        let (tx, _rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(empty_state()));
        state
            .lock()
            .expect("remote server state poisoned")
            .clients
            .insert(
                1,
                ClientState {
                    connected_at: Instant::now() - Duration::from_secs(30),
                    authenticated_at: Some(
                        Instant::now() - DIRECT_CLIENT_HEARTBEAT_WINDOW - Duration::from_secs(1),
                    ),
                    last_heartbeat_at: Some(
                        Instant::now() - DIRECT_CLIENT_HEARTBEAT_WINDOW - Duration::from_secs(1),
                    ),
                    ..remote_client(tx)
                },
            );

        let stale_snapshot = {
            let guard = state.lock().expect("remote server state poisoned");
            clients_snapshot(&guard, None, None, None)
        };
        assert!(stale_snapshot.clients[0].heartbeat_overdue);

        {
            let mut guard = state.lock().expect("remote server state poisoned");
            guard
                .clients
                .get_mut(&1)
                .expect("client state")
                .last_heartbeat_at = Some(Instant::now());
        }

        let recovered_snapshot = {
            let guard = state.lock().expect("remote server state poisoned");
            clients_snapshot(&guard, None, None, None)
        };
        let client = &recovered_snapshot.clients[0];
        assert!(!client.heartbeat_overdue);
        assert!(client.last_heartbeat_age_ms.is_some_and(|age| age <= 250));
        assert!(
            client.heartbeat_expires_in_ms.is_some_and(
                |ms| ms > 0 && ms <= DIRECT_CLIENT_HEARTBEAT_WINDOW.as_millis() as u64
            )
        );
    }
}
