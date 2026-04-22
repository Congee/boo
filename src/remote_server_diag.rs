//! Diagnostic snapshot helpers for the remote daemon.
//!
//! This module owns the read-only projection of daemon/client state into the
//! public diagnostic RPC types used by CLI and debugging workflows.

use crate::remote_state::{
    AUTH_CHALLENGE_WINDOW, ClientState, DIRECT_CLIENT_HEARTBEAT_WINDOW,
    REVIVABLE_ATTACHMENT_WINDOW, State,
};
use crate::remote_types::{
    RemoteClientInfo, RemoteClientsSnapshot, RemoteServerInfo, RevivableAttachmentInfo,
};
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
    let attached_clients = state
        .clients
        .values()
        .filter(|client| client.attached_tab.is_some())
        .count();
    let pending_auth_clients = state
        .clients
        .values()
        .filter(|client| !client.authenticated)
        .count();
    let revivable_attachments = state.revivable_attachments.len();
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
        revive_window_ms: REVIVABLE_ATTACHMENT_WINDOW.as_millis() as u64,
        connected_clients,
        attached_clients,
        pending_auth_clients,
        revivable_attachments,
    }];
    let mut clients = state
        .clients
        .iter()
        .map(|(client_id, client)| client_info_for_client(state, now, local_socket_path, client_id, client))
        .collect::<Vec<_>>();
    clients.sort_by_key(|client| client.client_id);

    let mut revivable_attachments = state
        .revivable_attachments
        .iter()
        .map(|(attachment_id, attachment)| RevivableAttachmentInfo {
            attachment_id: *attachment_id,
            tab_id: attachment.tab_id,
            resume_token_present: true,
            has_cached_state: attachment.last_state.is_some(),
            pane_state_count: attachment.pane_states.len(),
            latest_input_seq: attachment.latest_input_seq,
            revive_expires_in_ms: remaining_ms(now, attachment.expires_at),
        })
        .collect::<Vec<_>>();
    revivable_attachments.sort_by_key(|attachment| attachment.attachment_id);

    RemoteClientsSnapshot {
        servers,
        clients,
        revivable_attachments,
    }
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
        attached_tab: client.attached_tab,
        attachment_id: client.attachment_id,
        resume_token_present: client.resume_token.is_some(),
        has_cached_state: client.last_state.is_some(),
        pane_state_count: client.pane_states.len(),
        latest_input_seq: client.latest_input_seq,
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
        AUTH_CHALLENGE_WINDOW, ClientState, DIRECT_CLIENT_HEARTBEAT_WINDOW,
        REVIVABLE_ATTACHMENT_WINDOW, RevivableAttachment, State,
    };
    use crate::remote_wire::{REMOTE_CAPABILITIES, REMOTE_PROTOCOL_VERSION, RemoteCell, RemoteFullState};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, mpsc};
    use std::time::{Duration, Instant};

    fn empty_state() -> State {
        State {
            clients: HashMap::new(),
            revivable_attachments: HashMap::new(),
            server_identity_id: "test-daemon".to_string(),
            server_instance_id: "test-instance".to_string(),
        }
    }

    #[test]
    fn client_info_reports_remote_client_diagnostics() {
        let (tx, _rx) = mpsc::channel();
        let mut state = empty_state();
        state.clients.insert(
            7,
            ClientState {
                outbound: tx,
                authenticated: true,
                connected_at: Instant::now(),
                authenticated_at: Some(Instant::now()),
                last_heartbeat_at: Some(Instant::now()),
                attached_tab: Some(11),
                attachment_id: Some(0xabc),
                resume_token: Some(0xdef),
                last_tab_list_payload: None,
                last_ui_runtime_state_payload: None,
                last_ui_appearance_payload: None,
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
                is_local: false,
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
        assert_eq!(server_info.attached_clients, 1);
        assert_eq!(server_info.pending_auth_clients, 0);
        assert_eq!(server_info.revivable_attachments, 0);
        assert!(snapshot.revivable_attachments.is_empty());
        assert_eq!(snapshot.clients.len(), 1);
        let client = &snapshot.clients[0];
        assert_eq!(client.client_id, 7);
        assert!(client.authenticated);
        assert!(!client.is_local);
        assert_eq!(client.transport_kind, "tcp");
        assert_eq!(client.server_socket_path, None);
        assert!(!client.challenge_pending);
        assert_eq!(client.attached_tab, Some(11));
        assert_eq!(client.attachment_id, Some(0xabc));
        assert!(client.resume_token_present);
        assert!(client.has_cached_state);
        assert_eq!(client.pane_state_count, 1);
        assert_eq!(client.latest_input_seq, Some(9));
        assert!(client.connection_age_ms <= 250);
        assert!(client.authenticated_age_ms.is_some_and(|age| age <= 250));
        assert!(client.last_heartbeat_age_ms.is_some_and(|age| age <= 250));
        assert!(client.heartbeat_expires_in_ms.is_some_and(
            |ms| ms <= DIRECT_CLIENT_HEARTBEAT_WINDOW.as_millis() as u64
        ));
        assert!(!client.heartbeat_overdue);
        assert_eq!(client.challenge_expires_in_ms, None);
    }

    #[test]
    fn clients_snapshot_includes_revivable_attachments() {
        let mut state = empty_state();
        state.revivable_attachments.insert(
            0xabc,
            RevivableAttachment {
                tab_id: 11,
                resume_token: 0xdef,
                last_state: Some(Arc::new(RemoteFullState {
                    rows: 1,
                    cols: 1,
                    cursor_x: 0,
                    cursor_y: 0,
                    cursor_visible: true,
                    cursor_blinking: false,
                    cursor_style: 1,
                    cells: vec![],
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
                expires_at: Instant::now() + REVIVABLE_ATTACHMENT_WINDOW,
            },
        );

        let snapshot = clients_snapshot(&state, None, None, None);
        assert_eq!(snapshot.servers.len(), 1);
        assert_eq!(snapshot.servers[0].connected_clients, 0);
        assert_eq!(snapshot.servers[0].attached_clients, 0);
        assert_eq!(snapshot.servers[0].pending_auth_clients, 0);
        assert_eq!(snapshot.servers[0].revivable_attachments, 1);
        assert!(snapshot.clients.is_empty());
        assert_eq!(snapshot.revivable_attachments.len(), 1);
        let attachment = &snapshot.revivable_attachments[0];
        assert_eq!(attachment.attachment_id, 0xabc);
        assert_eq!(attachment.tab_id, 11);
        assert!(attachment.resume_token_present);
        assert!(attachment.has_cached_state);
        assert_eq!(attachment.pane_state_count, 1);
        assert_eq!(attachment.latest_input_seq, Some(9));
        assert!(attachment.revive_expires_in_ms <= REVIVABLE_ATTACHMENT_WINDOW.as_millis() as u64);
        assert!(attachment.revive_expires_in_ms > 0);
    }

    #[test]
    fn clients_snapshot_reports_overdue_direct_heartbeat() {
        let (tx, _rx) = mpsc::channel();
        let mut state = empty_state();
        state.clients.insert(
            1,
            ClientState {
                outbound: tx,
                authenticated: true,
                connected_at: Instant::now() - Duration::from_secs(30),
                authenticated_at: Some(
                    Instant::now() - DIRECT_CLIENT_HEARTBEAT_WINDOW - Duration::from_secs(1),
                ),
                last_heartbeat_at: Some(
                    Instant::now() - DIRECT_CLIENT_HEARTBEAT_WINDOW - Duration::from_secs(1),
                ),
                attached_tab: None,
                attachment_id: None,
                resume_token: None,
                last_tab_list_payload: None,
                last_ui_runtime_state_payload: None,
                last_ui_appearance_payload: None,
                last_state: None,
                pane_states: HashMap::new(),
                latest_input_seq: None,
                is_local: false,
            },
        );

        let snapshot = clients_snapshot(&state, None, None, None);
        assert_eq!(snapshot.servers.len(), 1);
        assert_eq!(snapshot.servers[0].connected_clients, 1);
        assert_eq!(snapshot.servers[0].attached_clients, 0);
        assert_eq!(snapshot.servers[0].pending_auth_clients, 0);
        assert_eq!(snapshot.servers[0].revivable_attachments, 0);
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
        state.lock().expect("remote server state poisoned").clients.insert(
            1,
            ClientState {
                outbound: tx,
                authenticated: true,
                connected_at: Instant::now() - Duration::from_secs(30),
                authenticated_at: Some(
                    Instant::now() - DIRECT_CLIENT_HEARTBEAT_WINDOW - Duration::from_secs(1),
                ),
                last_heartbeat_at: Some(
                    Instant::now() - DIRECT_CLIENT_HEARTBEAT_WINDOW - Duration::from_secs(1),
                ),
                attached_tab: None,
                attachment_id: None,
                resume_token: None,
                last_tab_list_payload: None,
                last_ui_runtime_state_payload: None,
                last_ui_appearance_payload: None,
                last_state: None,
                pane_states: HashMap::new(),
                latest_input_seq: None,
                is_local: false,
            },
        );

        let stale_snapshot = {
            let guard = state.lock().expect("remote server state poisoned");
            clients_snapshot(&guard, None, None, None)
        };
        assert!(stale_snapshot.clients[0].heartbeat_overdue);

        {
            let mut guard = state.lock().expect("remote server state poisoned");
            guard.clients.get_mut(&1).expect("client state").last_heartbeat_at = Some(Instant::now());
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
