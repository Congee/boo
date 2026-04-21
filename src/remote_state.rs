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
use crate::remote_wire::{MessageType, RemoteFullState, encode_message};

/// How long a revived attachment stays in the state graph before it is pruned
/// (remote client must reconnect + resume within this window).
pub(crate) const REVIVABLE_ATTACHMENT_WINDOW: Duration = Duration::from_secs(30);

/// Absolute deadline from `connected_at` for an unauthenticated client to
/// finish the challenge/response exchange. Protects against clients pinning
/// the socket with empty `Auth` frames.
pub(crate) const AUTH_CHALLENGE_WINDOW: Duration = Duration::from_secs(30);

/// Max silence from a direct (non-local) authenticated client before the daemon
/// treats it as stale and tears the connection down.
pub(crate) const DIRECT_CLIENT_HEARTBEAT_WINDOW: Duration = Duration::from_secs(20);

pub(crate) struct ClientState {
    pub(crate) outbound: mpsc::Sender<OutboundMessage>,
    pub(crate) authenticated: bool,
    pub(crate) connected_at: Instant,
    pub(crate) authenticated_at: Option<Instant>,
    pub(crate) last_heartbeat_at: Option<Instant>,
    pub(crate) attached_session: Option<u32>,
    pub(crate) attachment_id: Option<u64>,
    pub(crate) resume_token: Option<u64>,
    pub(crate) last_session_list_payload: Option<Vec<u8>>,
    pub(crate) last_ui_runtime_state_payload: Option<Vec<u8>>,
    pub(crate) last_ui_appearance_payload: Option<Vec<u8>>,
    pub(crate) last_state: Option<Arc<RemoteFullState>>,
    pub(crate) pane_states: HashMap<u64, Arc<RemoteFullState>>,
    pub(crate) latest_input_seq: Option<u64>,
    pub(crate) is_local: bool,
}

#[derive(Clone)]
pub(crate) struct RevivableAttachment {
    pub(crate) session_id: u32,
    pub(crate) resume_token: u64,
    pub(crate) last_state: Option<Arc<RemoteFullState>>,
    pub(crate) pane_states: HashMap<u64, Arc<RemoteFullState>>,
    pub(crate) latest_input_seq: Option<u64>,
    pub(crate) expires_at: Instant,
}

pub(crate) struct State {
    pub(crate) clients: HashMap<u64, ClientState>,
    pub(crate) revivable_attachments: HashMap<u64, RevivableAttachment>,
    pub(crate) server_identity_id: String,
    pub(crate) server_instance_id: String,
}

pub(crate) fn prune_revivable_attachments(state: &mut State) {
    let now = Instant::now();
    state
        .revivable_attachments
        .retain(|_, attachment| attachment.expires_at > now);
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

pub(crate) fn send_direct_error(state: &Arc<Mutex<State>>, client_id: u64, message: &str) {
    send_direct_frame(
        state,
        client_id,
        MessageType::ErrorMsg,
        message.as_bytes().to_vec(),
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
