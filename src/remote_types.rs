//! Public data types surfaced by the remote subsystem.
//!
//! These are plain `#[derive(Serialize)]` structs that flow over the control
//! socket and the CLI RPC boundary. Kept in their own module so the
//! transport/server code in `remote.rs` does not mingle with API surface.

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DirectTransportKind {
    QuicDirect,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteTabInfo {
    pub id: u32,
    pub name: String,
    pub title: String,
    pub pwd: String,
    pub attached: bool,
    pub child_exited: bool,
}

#[allow(dead_code)]
pub type RemoteSessionInfo = RemoteTabInfo;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct RemoteClientInfo {
    pub client_id: u64,
    pub authenticated: bool,
    pub is_local: bool,
    pub transport_kind: String,
    pub server_socket_path: Option<String>,
    pub challenge_pending: bool,
    #[serde(rename = "attached_tab", alias = "attached_session")]
    pub attached_tab: Option<u32>,
    pub attachment_id: Option<u64>,
    pub resume_token_present: bool,
    pub has_cached_state: bool,
    pub pane_state_count: usize,
    pub latest_input_seq: Option<u64>,
    pub connection_age_ms: u64,
    pub authenticated_age_ms: Option<u64>,
    pub last_heartbeat_age_ms: Option<u64>,
    pub heartbeat_expires_in_ms: Option<u64>,
    pub heartbeat_overdue: bool,
    pub challenge_expires_in_ms: Option<u64>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct RevivableAttachmentInfo {
    pub attachment_id: u64,
    #[serde(rename = "tab_id", alias = "session_id")]
    pub tab_id: u32,
    pub resume_token_present: bool,
    pub has_cached_state: bool,
    pub pane_state_count: usize,
    pub latest_input_seq: Option<u64>,
    pub revive_expires_in_ms: u64,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct RemoteServerInfo {
    pub local_socket_path: Option<String>,
    pub bind_address: Option<String>,
    pub port: Option<u16>,
    pub protocol_version: u16,
    pub capabilities: u32,
    pub build_id: String,
    pub server_instance_id: String,
    pub server_identity_id: String,
    pub auth_challenge_window_ms: u64,
    pub heartbeat_window_ms: u64,
    pub revive_window_ms: u64,
    pub connected_clients: usize,
    pub attached_clients: usize,
    pub pending_auth_clients: usize,
    pub revivable_attachments: usize,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct RemoteClientsSnapshot {
    pub servers: Vec<RemoteServerInfo>,
    pub clients: Vec<RemoteClientInfo>,
    pub revivable_attachments: Vec<RevivableAttachmentInfo>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct RemoteProbeSummary {
    pub host: String,
    pub port: u16,
    pub protocol_version: u16,
    pub capabilities: u32,
    pub build_id: Option<String>,
    pub server_instance_id: Option<String>,
    pub server_identity_id: Option<String>,
    pub heartbeat_rtt_ms: u64,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct RemoteDirectTabInfo {
    pub id: u32,
    pub name: String,
    pub title: String,
    pub pwd: String,
    pub attached: bool,
    pub child_exited: bool,
}

#[allow(dead_code)]
pub type RemoteDirectSessionInfo = RemoteDirectTabInfo;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct RemoteTabListSummary {
    pub host: String,
    pub port: u16,
    pub protocol_version: u16,
    pub capabilities: u32,
    pub build_id: Option<String>,
    pub server_instance_id: Option<String>,
    pub server_identity_id: Option<String>,
    pub heartbeat_rtt_ms: u64,
    pub tabs: Vec<RemoteDirectTabInfo>,
}

#[allow(dead_code)]
pub type RemoteSessionListSummary = RemoteTabListSummary;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct RemoteAttachedSummary {
    #[serde(rename = "tab_id", alias = "session_id")]
    pub tab_id: u32,
    pub attachment_id: Option<u64>,
    pub resume_token: Option<u64>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct RemoteAttachSummary {
    pub host: String,
    pub port: u16,
    pub protocol_version: u16,
    pub capabilities: u32,
    pub build_id: Option<String>,
    pub server_instance_id: Option<String>,
    pub server_identity_id: Option<String>,
    pub heartbeat_rtt_ms: u64,
    pub attached: RemoteAttachedSummary,
    pub rows: u16,
    pub cols: u16,
    pub cursor_x: u16,
    pub cursor_y: u16,
    pub cursor_visible: bool,
    pub cursor_blinking: bool,
    pub cursor_style: i32,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct RemoteCreateSummary {
    pub host: String,
    pub port: u16,
    pub protocol_version: u16,
    pub capabilities: u32,
    pub build_id: Option<String>,
    pub server_instance_id: Option<String>,
    pub server_identity_id: Option<String>,
    pub heartbeat_rtt_ms: u64,
    #[serde(rename = "tab_id", alias = "session_id")]
    pub tab_id: u32,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct RemoteUpgradeProbeSummary {
    pub selected_transport: DirectTransportKind,
    pub probe: RemoteProbeSummary,
}
