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

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct RemoteClientInfo {
    pub client_id: u64,
    pub authenticated: bool,
    pub is_local: bool,
    pub transport_kind: String,
    pub server_socket_path: Option<String>,
    pub challenge_pending: bool,
    pub attached_tab: Option<u32>,
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
    pub connected_clients: usize,
    pub attached_clients: usize,
    pub pending_auth_clients: usize,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct RemoteClientsSnapshot {
    pub servers: Vec<RemoteServerInfo>,
    pub clients: Vec<RemoteClientInfo>,
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

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct RemoteAttachedSummary {
    pub tab_id: u32,
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
    pub tab_id: u32,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct RemoteUpgradeProbeSummary {
    pub selected_transport: DirectTransportKind,
    pub probe: RemoteProbeSummary,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_client_info_serializes_canonical_attached_tab_field() {
        let value = serde_json::to_value(RemoteClientInfo {
            client_id: 7,
            authenticated: true,
            is_local: false,
            transport_kind: "quic-direct".into(),
            server_socket_path: None,
            challenge_pending: false,
            attached_tab: Some(42),
            has_cached_state: true,
            pane_state_count: 2,
            latest_input_seq: Some(11),
            connection_age_ms: 100,
            authenticated_age_ms: Some(80),
            last_heartbeat_age_ms: Some(5),
            heartbeat_expires_in_ms: Some(50),
            heartbeat_overdue: false,
            challenge_expires_in_ms: None,
        })
        .unwrap();

        assert_eq!(value.get("attached_tab").and_then(|v| v.as_u64()), Some(42));
        assert!(value.get("attached_session").is_none());
        assert!(value.get("attachment_id").is_none());
        assert!(value.get("resume_token_present").is_none());
    }

    #[test]
    fn remote_create_summary_serializes_canonical_tab_id_field() {
        let value = serde_json::to_value(RemoteCreateSummary {
            host: "127.0.0.1".into(),
            port: 7337,
            protocol_version: 1,
            capabilities: 0,
            build_id: Some("debug".into()),
            server_instance_id: Some("instance".into()),
            server_identity_id: Some("identity".into()),
            heartbeat_rtt_ms: 12,
            tab_id: 42,
        })
        .unwrap();

        assert_eq!(value.get("tab_id").and_then(|v| v.as_u64()), Some(42));
        assert!(value.get("session_id").is_none());
    }
}
