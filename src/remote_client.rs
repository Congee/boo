//! Public direct-client RPCs over QUIC. Identity is enforced by the
//! higher-level Boo handshake and the surrounding network substrate.

use crate::remote::DirectTransportClient;
use crate::remote_types::{
    DirectTransportKind, RemoteCreateSummary, RemoteProbeSummary,
    RemoteTabListSummary, RemoteUpgradeProbeSummary,
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

fn probe_summary_from_transport(
    client: &mut DirectTransportClient<crate::remote_quic::QuicDirectStream>,
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

fn list_summary_from_transport(
    client: &mut DirectTransportClient<crate::remote_quic::QuicDirectStream>,
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

fn create_summary_from_transport(
    client: &mut DirectTransportClient<crate::remote_quic::QuicDirectStream>,
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
    use crate::remote_types::RemoteTabInfo;
    use crate::status_components::UiStatusBarSnapshot;
    use crate::remote_wire::{
        MESSAGE_TYPE_LIST_TABS, MESSAGE_TYPE_TAB_LIST, MessageType,
        REMOTE_PROTOCOL_VERSION, encode_auth_ok_payload, encode_message, encode_tab_list,
        read_message,
    };
    use std::io::Write;

    #[test]
    fn probe_remote_endpoint_rejects_unsupported_protocol_version_over_socket() {
        use std::net::TcpListener;

        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind test listener");
        let port = listener.local_addr().expect("listener addr").port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept probe client");
            let (ty, payload) = read_message(&mut stream).expect("read auth request");
            assert_eq!(ty, MessageType::Auth);
            assert!(payload.is_empty());

            let mut auth_ok = encode_auth_ok_payload("test-daemon", "test-instance");
            auth_ok[0..2].copy_from_slice(&(REMOTE_PROTOCOL_VERSION + 1).to_le_bytes());
            stream
                .write_all(&encode_message(MessageType::AuthOk, &auth_ok))
                .expect("write auth ok");
        });

        let error = probe_remote_endpoint("127.0.0.1", port, None)
            .expect_err("probe should reject");
        assert!(
            error.contains("Unsupported remote protocol version"),
            "unexpected error: {error}"
        );

        server.join().expect("probe server thread");
    }

    #[test]
    fn list_remote_daemon_tabs_reuses_handshake_validation_over_socket() {
        use std::net::TcpListener;

        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind test listener");
        let port = listener.local_addr().expect("listener addr").port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept list client");
            let (ty, payload) = read_message(&mut stream).expect("read auth request");
            assert_eq!(ty, MessageType::Auth);
            assert!(payload.is_empty());

            stream
                .write_all(&encode_message(
                    MessageType::AuthOk,
                    &encode_auth_ok_payload("test-daemon", "test-instance"),
                ))
                .expect("write auth ok");

            let (ty, payload) = read_message(&mut stream).expect("read heartbeat");
            assert_eq!(ty, MessageType::Heartbeat);
            stream
                .write_all(&encode_message(MessageType::HeartbeatAck, &payload))
                .expect("write heartbeat ack");

            let (ty, payload) = read_message(&mut stream).expect("read list tabs");
            assert_eq!(ty, MESSAGE_TYPE_LIST_TABS);
            assert!(payload.is_empty());
            stream
                .write_all(&encode_message(
                    MESSAGE_TYPE_TAB_LIST,
                    &encode_tab_list(&[RemoteTabInfo {
                        id: 11,
                        name: "dev".to_string(),
                        title: "shell".to_string(),
                        pwd: "/home/example/dev/boo".to_string(),
                        active: true,
                        child_exited: false,
                    }]),
                ))
                .expect("write tab list");
        });

        let summary =
            list_remote_daemon_tabs("127.0.0.1", port, None).expect("list tabs summary");
        assert_eq!(summary.protocol_version, REMOTE_PROTOCOL_VERSION);
        assert_eq!(summary.server_identity_id.as_deref(), Some("test-daemon"));
        assert_eq!(summary.server_instance_id.as_deref(), Some("test-instance"));
        assert_eq!(summary.tabs.len(), 1);
        assert_eq!(summary.tabs[0].id, 11);
        assert_eq!(summary.tabs[0].name, "dev");
        assert_eq!(summary.tabs[0].pwd, "/home/example/dev/boo");

        server.join().expect("list server thread");
    }

    #[test]
    fn list_remote_daemon_tabs_rejects_unexpected_server_identity() {
        use std::net::TcpListener;

        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind test listener");
        let port = listener.local_addr().expect("listener addr").port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept list client");
            let (ty, payload) = read_message(&mut stream).expect("read auth request");
            assert_eq!(ty, MessageType::Auth);
            assert!(payload.is_empty());

            stream
                .write_all(&encode_message(
                    MessageType::AuthOk,
                    &encode_auth_ok_payload("actual-daemon", "test-instance"),
                ))
                .expect("write auth ok");
        });

        let error = list_remote_daemon_tabs(
            "127.0.0.1",
            port,
            Some("expected-daemon"),
        )
        .expect_err("unexpected daemon identity should fail");
        assert!(
            error.contains("expected"),
            "unexpected error text: {error}"
        );

        server.join().expect("list server thread");
    }

    #[test]
    fn create_remote_daemon_tab_uses_shared_handshake_and_create_path() {
        use std::net::TcpListener;

        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind test listener");
        let port = listener.local_addr().expect("listener addr").port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept create client");
            let (ty, payload) = read_message(&mut stream).expect("read auth request");
            assert_eq!(ty, MessageType::Auth);
            assert!(payload.is_empty());

            stream
                .write_all(&encode_message(
                    MessageType::AuthOk,
                    &encode_auth_ok_payload("test-daemon", "test-instance"),
                ))
                .expect("write auth ok");

            let (ty, payload) = read_message(&mut stream).expect("read heartbeat");
            assert_eq!(ty, MessageType::Heartbeat);
            stream
                .write_all(&encode_message(MessageType::HeartbeatAck, &payload))
                .expect("write heartbeat ack");

            let (ty, payload) = read_message(&mut stream).expect("read create");
            assert_eq!(ty, MessageType::Create);
            assert_eq!(payload, [132, 0, 48, 0]);
            stream
                .write_all(&encode_message(
                    MESSAGE_TYPE_TAB_LIST,
                    &encode_tab_list(&[RemoteTabInfo {
                        id: 77,
                        name: "Tab 1".to_string(),
                        title: "Tab 1".to_string(),
                        pwd: "/tmp".to_string(),
                        active: true,
                        child_exited: false,
                    }]),
                ))
                .expect("write tab list");
            stream
                .write_all(&encode_message(
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
                        }],
                        visible_panes: vec![],
                        mouse_selection: UiMouseSelectionSnapshot::default(),
                        status_bar: UiStatusBarSnapshot {
                            left: vec![],
                            right: vec![],
                        },
                        pwd: "/tmp".to_string(),
                    })
                    .expect("encode runtime state"),
                ))
                .expect("write runtime state");
        });

        let summary = create_remote_daemon_tab(
            "127.0.0.1",
            port,
            Some("test-daemon"),
            132,
            48,
        )
        .expect("create summary");
        assert_eq!(summary.protocol_version, REMOTE_PROTOCOL_VERSION);
        assert_eq!(summary.server_identity_id.as_deref(), Some("test-daemon"));
        assert_eq!(summary.server_instance_id.as_deref(), Some("test-instance"));
        assert_eq!(summary.tab_id, 77);

        server.join().expect("create server thread");
    }
}
