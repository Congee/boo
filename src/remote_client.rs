//! Public direct-client RPCs over QUIC. Identity is enforced by the
//! higher-level Boo handshake and the surrounding network substrate.

use crate::remote::DirectTransportClient;
use crate::remote_direct_transport::DirectReadWrite;
use crate::remote_types::{
    DirectTransportKind, RemoteCreateSummary, RemoteProbeSummary, RemoteTabListSummary,
    RemoteUpgradeProbeSummary,
};

pub fn select_direct_transport(
    capabilities: u32,
    _migration_capable_path_available: bool,
) -> Result<DirectTransportKind, String> {
    if (capabilities & crate::remote::REMOTE_CAPABILITY_QUIC_DIRECT_TRANSPORT) != 0 {
        Ok(DirectTransportKind::QuicDirect)
    } else {
        Err("remote daemon does not advertise QUIC direct transport".to_string())
    }
}

fn probe_summary_from_transport<S: DirectReadWrite>(
    client: &mut DirectTransportClient<S>,
    port: u16,
) -> Result<RemoteProbeSummary, String> {
    let heartbeat_rtt_ms = client.heartbeat_round_trip(b"boo-remote-probe")?;
    Ok(RemoteProbeSummary {
        host: client.host.clone(),
        port,
        protocol_version: client.protocol_version,
        capabilities: client.capabilities,
        build_id: client.build_id.clone(),
        server_instance_id: client.server_instance_id.clone(),
        server_identity_id: client.server_identity_id.clone(),
        heartbeat_rtt_ms,
    })
}

pub fn probe_remote_endpoint(
    host: &str,
    port: u16,
    expected_server_identity: Option<&str>,
) -> Result<RemoteProbeSummary, String> {
    let mut client = crate::remote_quic::connect_direct(host, port, expected_server_identity)?;
    probe_summary_from_transport(&mut client, port)
}

pub fn probe_selected_direct_transport(
    transport: DirectTransportKind,
    host: &str,
    port: u16,
    expected_server_identity: Option<&str>,
) -> Result<RemoteUpgradeProbeSummary, String> {
    match transport {
        DirectTransportKind::QuicDirect => Ok(RemoteUpgradeProbeSummary {
            selected_transport: DirectTransportKind::QuicDirect,
            probe: probe_remote_endpoint(host, port, expected_server_identity)?,
        }),
    }
}

fn list_summary_from_transport<S: DirectReadWrite>(
    client: &mut DirectTransportClient<S>,
    port: u16,
) -> Result<RemoteTabListSummary, String> {
    let heartbeat_rtt_ms = client.heartbeat_round_trip(b"boo-remote-list")?;
    let tabs = client.list_tabs()?;
    Ok(RemoteTabListSummary {
        host: client.host.clone(),
        port,
        protocol_version: client.protocol_version,
        capabilities: client.capabilities,
        build_id: client.build_id.clone(),
        server_instance_id: client.server_instance_id.clone(),
        server_identity_id: client.server_identity_id.clone(),
        heartbeat_rtt_ms,
        tabs,
    })
}

pub fn list_remote_daemon_tabs(
    host: &str,
    port: u16,
    expected_server_identity: Option<&str>,
) -> Result<RemoteTabListSummary, String> {
    let mut client = crate::remote_quic::connect_direct(host, port, expected_server_identity)?;
    list_summary_from_transport(&mut client, port)
}

fn create_summary_from_transport<S: DirectReadWrite>(
    client: &mut DirectTransportClient<S>,
    port: u16,
    cols: u16,
    rows: u16,
) -> Result<RemoteCreateSummary, String> {
    let heartbeat_rtt_ms = client.heartbeat_round_trip(b"boo-remote-create")?;
    let tab_id = client.create_tab(cols, rows)?;
    Ok(RemoteCreateSummary {
        host: client.host.clone(),
        port,
        protocol_version: client.protocol_version,
        capabilities: client.capabilities,
        build_id: client.build_id.clone(),
        server_instance_id: client.server_instance_id.clone(),
        server_identity_id: client.server_identity_id.clone(),
        heartbeat_rtt_ms,
        tab_id,
    })
}

pub fn create_remote_daemon_tab(
    host: &str,
    port: u16,
    expected_server_identity: Option<&str>,
    cols: u16,
    rows: u16,
) -> Result<RemoteCreateSummary, String> {
    let mut client = crate::remote_quic::connect_direct(host, port, expected_server_identity)?;
    create_summary_from_transport(&mut client, port, cols, rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::{UiMouseSelectionSnapshot, UiRuntimeState, UiTabSnapshot};
    use crate::remote_direct_transport::test_support::{ScriptedDirectStream, auth_ok_reply};
    use crate::remote_types::RemoteTabInfo;
    use crate::remote_wire::{
        MESSAGE_TYPE_LIST_TABS, MESSAGE_TYPE_TAB_LIST, MessageType, REMOTE_PROTOCOL_VERSION,
        encode_auth_ok_payload, encode_message, encode_tab_list,
    };
    use crate::status_components::UiStatusBarSnapshot;

    fn scripted_client(
        replies: Vec<Vec<u8>>,
        expected_server_identity: Option<&str>,
    ) -> DirectTransportClient<ScriptedDirectStream> {
        DirectTransportClient::connect_over_stream(
            ScriptedDirectStream::new(replies),
            "127.0.0.1".to_string(),
            43210,
            expected_server_identity,
        )
        .expect("connect scripted transport")
    }

    #[test]
    fn probe_summary_rejects_unsupported_protocol_version() {
        let mut auth_ok = encode_auth_ok_payload("test-daemon", "test-instance");
        auth_ok[0..2].copy_from_slice(&(REMOTE_PROTOCOL_VERSION + 1).to_le_bytes());
        let error = match DirectTransportClient::connect_over_stream(
            ScriptedDirectStream::new(vec![encode_message(MessageType::AuthOk, &auth_ok)]),
            "127.0.0.1".to_string(),
            43210,
            None,
        ) {
            Ok(_) => panic!("probe should reject"),
            Err(error) => error,
        };
        assert!(
            error.contains("Unsupported remote protocol version"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn list_summary_reuses_handshake_validation() {
        let mut client = scripted_client(
            vec![
                auth_ok_reply("test-daemon", "test-instance"),
                encode_message(MessageType::HeartbeatAck, b"boo-remote-list"),
                encode_message(
                    MESSAGE_TYPE_TAB_LIST,
                    &encode_tab_list(&[RemoteTabInfo {
                        id: 11,
                        name: "dev".to_string(),
                        title: "shell".to_string(),
                        pwd: "/home/example/dev/boo".to_string(),
                        active: true,
                        child_exited: false,
                    }]),
                ),
            ],
            None,
        );

        let summary = list_summary_from_transport(&mut client, 43210).expect("list tabs summary");
        assert_eq!(summary.protocol_version, REMOTE_PROTOCOL_VERSION);
        assert_eq!(summary.server_identity_id.as_deref(), Some("test-daemon"));
        assert_eq!(summary.server_instance_id.as_deref(), Some("test-instance"));
        assert_eq!(summary.tabs.len(), 1);
        assert_eq!(summary.tabs[0].id, 11);
        assert_eq!(summary.tabs[0].name, "dev");
        assert_eq!(summary.tabs[0].pwd, "/home/example/dev/boo");

        let frames = client.stream.written_frames();
        assert_eq!(frames[0], (MessageType::Auth, Vec::new()));
        assert_eq!(
            frames[1],
            (MessageType::Heartbeat, b"boo-remote-list".to_vec())
        );
        assert_eq!(frames[2], (MESSAGE_TYPE_LIST_TABS, Vec::new()));
    }

    #[test]
    fn list_remote_daemon_tabs_rejects_unexpected_server_identity() {
        let error = match DirectTransportClient::connect_over_stream(
            ScriptedDirectStream::new(vec![auth_ok_reply("actual-daemon", "test-instance")]),
            "127.0.0.1".to_string(),
            43210,
            Some("expected-daemon"),
        ) {
            Ok(_) => panic!("unexpected daemon identity should fail"),
            Err(error) => error,
        };
        assert!(error.contains("expected"), "unexpected error text: {error}");
    }

    #[test]
    fn create_summary_uses_shared_handshake_and_runtime_action_path() {
        let mut client = scripted_client(
            vec![
                auth_ok_reply("test-daemon", "test-instance"),
                encode_message(MessageType::HeartbeatAck, b"boo-remote-create"),
                encode_message(
                    MessageType::UiRuntimeState,
                    &serde_json::to_vec(&UiRuntimeState {
                        active_tab: 0,
                        focused_pane: 1,
                        tabs: vec![UiTabSnapshot {
                            tab_id: 77,
                            index: 0,
                            active: true,
                            title: "Tab 1".to_string(),
                            pane_count: 1,
                            focused_pane: Some(1),
                            pane_ids: vec![1],
                        }],
                        visible_panes: vec![],
                        mouse_selection: UiMouseSelectionSnapshot::default(),
                        status_bar: UiStatusBarSnapshot {
                            left: vec![],
                            right: vec![],
                        },
                        pwd: "/tmp".to_string(),
                        runtime_revision: 1,
                        view_revision: 1,
                        view_id: 1,
                        viewed_tab_id: Some(77),
                        viewport_cols: None,
                        viewport_rows: None,
                        visible_pane_ids: vec![1],
                        acked_client_action_id: None,
                    })
                    .expect("encode runtime state"),
                ),
            ],
            Some("test-daemon"),
        );

        let summary =
            create_summary_from_transport(&mut client, 43210, 132, 48).expect("create summary");
        assert_eq!(summary.protocol_version, REMOTE_PROTOCOL_VERSION);
        assert_eq!(summary.server_identity_id.as_deref(), Some("test-daemon"));
        assert_eq!(summary.server_instance_id.as_deref(), Some("test-instance"));
        assert_eq!(summary.tab_id, 77);

        let frames = client.stream.written_frames();
        assert_eq!(frames[0], (MessageType::Auth, Vec::new()));
        assert_eq!(
            frames[1],
            (MessageType::Heartbeat, b"boo-remote-create".to_vec())
        );
        let (ty, payload) = &frames[2];
        assert_eq!(*ty, MessageType::RuntimeAction);
        let action: crate::remote::RuntimeAction =
            serde_json::from_slice(payload).expect("decode runtime action");
        assert_eq!(
            action,
            crate::remote::RuntimeAction::NewTab {
                view_id: 0,
                cols: Some(132),
                rows: Some(48),
            }
        );
    }
}
