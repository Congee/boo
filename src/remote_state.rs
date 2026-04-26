//! Shared mutable state for the remote daemon.
//!
//! `State` is wrapped in `Arc<Mutex<_>>` by `RemoteServer` and passed to every
//! reader, writer, accept-loop, and RPC handler. Pulled into its own module so
//! that the listener, auth, and RPC subsystems can reference the state graph
//! without depending on the whole `remote.rs` translation unit.
//!
//! Everything here is `pub(crate)` by design — these types are internal plumbing
//! that the daemon crates operate on directly (field access rather than
//! accessor methods). Keep the module tight: no I/O, no server lifecycle,
//! no wire format — just the data the mutex is guarding.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use crate::remote_batcher::OutboundMessage;
use crate::remote_wire::{
    MessageType, RemoteErrorCode, RemoteFullState, encode_error_payload, encode_message,
};

/// Absolute deadline from `connected_at` for an unauthenticated client to
/// finish the initial auth acknowledgement. Protects against clients pinning
/// the socket with empty `Auth` frames.
pub(crate) const AUTH_CHALLENGE_WINDOW: Duration = Duration::from_secs(30);

/// Max silence from a direct (non-local) authenticated client before the daemon
/// treats it as stale and tears the connection down.
pub(crate) const DIRECT_CLIENT_HEARTBEAT_WINDOW: Duration = Duration::from_secs(20);
pub(crate) const VIEW_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// Runtime-view state cached per connected client.
///
/// The authoritative tab/pane/runtime model lives in the server runtime. This
/// struct only tracks whether a client is subscribed to the runtime stream plus
/// the cached payloads/full states needed for efficient transport updates.
pub(crate) struct ClientRuntimeView {
    pub(crate) view_id: u64,
    pub(crate) subscribed_to_runtime: bool,
    pub(crate) view_revision: u64,
    pub(crate) viewed_tab_id: Option<u32>,
    pub(crate) focused_pane_id: Option<u64>,
    pub(crate) viewport_cols: Option<u16>,
    pub(crate) viewport_rows: Option<u16>,
    pub(crate) visible_pane_ids: Vec<u64>,
    pub(crate) ui_attached: bool,
    pub(crate) detached_at: Option<Instant>,
    pub(crate) acked_client_action_id: Option<u64>,
    pub(crate) last_render_ack_at: Option<Instant>,
    pub(crate) last_rendered_pane_id: Option<u64>,
    pub(crate) last_rendered_pane_revision: Option<u64>,
    pub(crate) last_rendered_runtime_revision: Option<u64>,
    pub(crate) last_tab_list_payload: Option<Vec<u8>>,
    pub(crate) last_ui_runtime_state_payload: Option<Vec<u8>>,
    pub(crate) last_ui_appearance_payload: Option<Vec<u8>>,
    pub(crate) last_state: Option<Arc<RemoteFullState>>,
    pub(crate) pane_states: HashMap<u64, Arc<RemoteFullState>>,
    pub(crate) latest_input_seq: Option<u64>,
}

impl ClientRuntimeView {
    pub(crate) fn idle() -> Self {
        Self {
            view_id: 0,
            subscribed_to_runtime: false,
            view_revision: 1,
            viewed_tab_id: None,
            focused_pane_id: None,
            viewport_cols: None,
            viewport_rows: None,
            visible_pane_ids: Vec::new(),
            ui_attached: true,
            detached_at: None,
            acked_client_action_id: None,
            last_render_ack_at: None,
            last_rendered_pane_id: None,
            last_rendered_pane_revision: None,
            last_rendered_runtime_revision: None,
            last_tab_list_payload: None,
            last_ui_runtime_state_payload: None,
            last_ui_appearance_payload: None,
            last_state: None,
            pane_states: HashMap::new(),
            latest_input_seq: None,
        }
    }

    pub(crate) fn clear_stream_state(&mut self) {
        self.last_state = None;
        self.pane_states.clear();
        self.latest_input_seq = None;
    }

    pub(crate) fn touch_view(&mut self) {
        self.view_revision = self.view_revision.wrapping_add(1).max(1);
        self.clear_stream_state();
    }

    pub(crate) fn attach_ui(&mut self) {
        self.ui_attached = true;
        self.detached_at = None;
        self.touch_view();
    }

    pub(crate) fn detach_ui(&mut self) {
        self.ui_attached = false;
        self.detached_at = Some(Instant::now());
        self.touch_view();
    }
}

#[cfg(test)]
impl ClientState {
    pub(crate) fn test_client(
        outbound: mpsc::Sender<OutboundMessage>,
        authenticated: bool,
        is_local: bool,
    ) -> Self {
        Self {
            outbound,
            authenticated,
            connected_at: Instant::now(),
            authenticated_at: authenticated.then(Instant::now),
            last_heartbeat_at: None,
            runtime_view: ClientRuntimeView::idle(),
            is_local,
        }
    }
}

pub(crate) struct ClientState {
    pub(crate) outbound: mpsc::Sender<OutboundMessage>,
    pub(crate) authenticated: bool,
    pub(crate) connected_at: Instant,
    pub(crate) authenticated_at: Option<Instant>,
    pub(crate) last_heartbeat_at: Option<Instant>,
    pub(crate) runtime_view: ClientRuntimeView,
    pub(crate) is_local: bool,
}

pub(crate) struct State {
    pub(crate) clients: HashMap<u64, ClientState>,
    pub(crate) server_identity_id: String,
    pub(crate) server_instance_id: String,
}

#[cfg(test)]
impl State {
    pub(crate) fn test_empty() -> Self {
        Self {
            clients: HashMap::new(),
            server_identity_id: "test-daemon".to_string(),
            server_instance_id: "test-instance".to_string(),
        }
    }
}

pub(crate) fn should_disconnect_idle_client(
    state: &Arc<Mutex<State>>,
    client_id: u64,
    auth_challenge_window: Duration,
    heartbeat_window: Duration,
) -> Option<&'static str> {
    let state = state.lock().expect("remote server state poisoned");
    let client = state.clients.get(&client_id)?;
    let now = Instant::now();
    if client.authenticated {
        if client.is_local {
            return None;
        }
        let last_liveness = client.last_heartbeat_at.or(client.authenticated_at)?;
        if now.saturating_duration_since(last_liveness) > heartbeat_window {
            return Some("heartbeat-timeout");
        }
        return None;
    }
    // Absolute deadline from connect time for the handshake to complete.
    if now.saturating_duration_since(client.connected_at) > auth_challenge_window {
        return Some("auth-timeout");
    }
    None
}

pub(crate) fn send_direct_error(
    state: &Arc<Mutex<State>>,
    client_id: u64,
    code: RemoteErrorCode,
    message: &str,
) {
    send_direct_frame(
        state,
        client_id,
        MessageType::ErrorMsg,
        encode_error_payload(code, message),
    );
}

pub(crate) fn send_direct_frame(
    state: &Arc<Mutex<State>>,
    client_id: u64,
    ty: MessageType,
    payload: Vec<u8>,
) {
    let state = state.lock().expect("remote server state poisoned");
    if let Some(client) = state.clients.get(&client_id) {
        let _ = client
            .outbound
            .send(OutboundMessage::Frame(encode_message(ty, &payload)));
    }
}
