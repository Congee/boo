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

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, mpsc};
use std::time::Instant;

use crate::remote_batcher::OutboundMessage;
use crate::remote_wire::RemoteFullState;

pub(crate) struct ClientState {
    pub(crate) outbound: mpsc::Sender<OutboundMessage>,
    pub(crate) authenticated: bool,
    pub(crate) challenge: Option<AuthChallengeState>,
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

#[derive(Clone, Copy)]
pub(crate) struct AuthChallengeState {
    pub(crate) bytes: [u8; 32],
    pub(crate) expires_at: Instant,
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
    pub(crate) auth_key: Option<Vec<u8>>,
    pub(crate) server_identity_id: String,
    pub(crate) server_instance_id: String,
    /// client_ids whose underlying transport is TLS-wrapped TCP. Plain TCP and local
    /// Unix-socket clients are absent. Populated at client registration and scrubbed on
    /// removal; diagnostics look up membership here to distinguish plain-TCP from
    /// TCP-TLS in `transport_kind`.
    pub(crate) tls_clients: HashSet<u64>,
}
