//! Public direct-client RPCs — probe / list-sessions / attach / create and
//! their SPKI-pinned `_tls` / `_quic` variants — extracted from `remote.rs`.
//!
//! Each public function is a thin three-step wrapper:
//! 1. Pick a transport connector (`PlainTcpConnector`, `PinnedTlsConnector`,
//!    `PinnedQuicConnector`).
//! 2. Drive `connect_with` to get a handshake-completed
//!    `DirectTransportSession`.
//! 3. Call the corresponding summary helper
//!    (`probe_summary_from_session`, `list_summary_from_session`,
//!    `attach_summary_from_session`, `create_summary_from_session`) which
//!    issues the RPC and snapshots the session metadata.

use crate::remote::{
    DirectReadWrite, DirectTransportSession, REMOTE_CAPABILITY_QUIC_DIRECT_TRANSPORT,
    REMOTE_CAPABILITY_TCP_DIRECT_TRANSPORT,
};
use crate::remote_transport::{
    PinnedQuicConnector, PinnedTlsConnector, PlainTcpConnector, connect_with,
};
use crate::remote_types::{
    DirectTransportKind, RemoteAttachSummary, RemoteCreateSummary, RemoteProbeSummary,
    RemoteSessionListSummary, RemoteUpgradeProbeSummary,
};

pub fn select_direct_transport(
    capabilities: u32,
    migration_capable_path_available: bool,
) -> Result<DirectTransportKind, String> {
    let supports_quic = (capabilities & REMOTE_CAPABILITY_QUIC_DIRECT_TRANSPORT) != 0;
    let supports_tcp = (capabilities & REMOTE_CAPABILITY_TCP_DIRECT_TRANSPORT) != 0;
    if supports_quic && migration_capable_path_available {
        return Ok(DirectTransportKind::QuicDirect);
    }
    if supports_tcp {
        return Ok(DirectTransportKind::TcpDirect);
    }
    if supports_quic {
        return Err("direct remote endpoint only advertises QUIC transport; TCP fallback is unavailable".to_string());
    }
    Err("direct remote endpoint does not advertise a supported transport".to_string())
}

fn probe_summary_from_session<S: DirectReadWrite>(
    client: &mut DirectTransportSession<S>,
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
    let mut client =
        connect_with(PlainTcpConnector, host, port, expected_server_identity)?;
    probe_summary_from_session(&mut client, port)
}

/// SPKI-pinned TLS variant of `probe_remote_endpoint`. `expected_identity` is
/// the `daemon_identity` string the caller already trusts; the TLS handshake
/// aborts if the presented cert's SPKI hash does not match.
pub fn probe_remote_endpoint_tls(
    host: &str,
    port: u16,
    expected_identity: &str,
) -> Result<RemoteProbeSummary, String> {
    let mut client = connect_with(
        PinnedTlsConnector { expected_identity },
        host,
        port,
        Some(expected_identity),
    )?;
    probe_summary_from_session(&mut client, port)
}

/// SPKI-pinned QUIC variant of `probe_remote_endpoint`. Same SPKI pin
/// semantics as `probe_remote_endpoint_tls` — QUIC inherits the exact
/// rustls-based trust model and `PinnedSpkiServerCertVerifier` from the
/// TCP+TLS path — the only difference is the wire transport.
pub fn probe_remote_endpoint_quic(
    host: &str,
    port: u16,
    expected_identity: &str,
) -> Result<RemoteProbeSummary, String> {
    let mut client = connect_with(
        PinnedQuicConnector { expected_identity },
        host,
        port,
        Some(expected_identity),
    )?;
    probe_summary_from_session(&mut client, port)
}

pub fn probe_selected_direct_transport(
    transport: DirectTransportKind,
    host: &str,
    port: u16,
    expected_server_identity: Option<&str>,
) -> Result<RemoteUpgradeProbeSummary, String> {
    match transport {
        DirectTransportKind::TcpDirect => Ok(RemoteUpgradeProbeSummary {
            selected_transport: transport,
            probe: probe_remote_endpoint(host, port, expected_server_identity)?,
        }),
        DirectTransportKind::QuicDirect => {
            // QUIC always rides TLS, so a pin is not optional. Callers without
            // a pin must go through probe_selected_direct_transport_tls or
            // drop back to TcpDirect.
            let identity = expected_server_identity.ok_or_else(|| {
                "QUIC direct transport requires an expected_server_identity pin".to_string()
            })?;
            Ok(RemoteUpgradeProbeSummary {
                selected_transport: transport,
                probe: probe_remote_endpoint_quic(host, port, identity)?,
            })
        }
    }
}

/// SPKI-pinned TLS variant of `probe_selected_direct_transport`. Intended for
/// the SSH-bootstrap → direct-TLS upgrade flow: the caller has already
/// learned the server's `daemon_identity` out-of-band (typically over the
/// forwarded SSH control socket) and wants the subsequent direct connection
/// to be TLS with that pin as the trust anchor.
pub fn probe_selected_direct_transport_tls(
    transport: DirectTransportKind,
    host: &str,
    port: u16,
    expected_identity: &str,
) -> Result<RemoteUpgradeProbeSummary, String> {
    match transport {
        DirectTransportKind::TcpDirect => Ok(RemoteUpgradeProbeSummary {
            selected_transport: transport,
            probe: probe_remote_endpoint_tls(host, port, expected_identity)?,
        }),
        DirectTransportKind::QuicDirect => Ok(RemoteUpgradeProbeSummary {
            selected_transport: transport,
            probe: probe_remote_endpoint_quic(host, port, expected_identity)?,
        }),
    }
}

fn list_summary_from_session<S: DirectReadWrite>(
    client: &mut DirectTransportSession<S>,
    port: u16,
) -> Result<RemoteSessionListSummary, String> {
    let heartbeat_rtt_ms = client.heartbeat_round_trip(b"boo-remote-list")?;
    let sessions = client.list_sessions()?;
    Ok(RemoteSessionListSummary {
        host: client.host.clone(),
        port,
        protocol_version: client.protocol_version,
        capabilities: client.capabilities,
        build_id: client.build_id.clone(),
        server_instance_id: client.server_instance_id.clone(),
        server_identity_id: client.server_identity_id.clone(),
        heartbeat_rtt_ms,
        sessions,
    })
}

pub fn list_remote_daemon_sessions(
    host: &str,
    port: u16,
    expected_server_identity: Option<&str>,
) -> Result<RemoteSessionListSummary, String> {
    let mut client =
        connect_with(PlainTcpConnector, host, port, expected_server_identity)?;
    list_summary_from_session(&mut client, port)
}

pub fn list_remote_daemon_sessions_tls(
    host: &str,
    port: u16,
    expected_identity: &str,
) -> Result<RemoteSessionListSummary, String> {
    let mut client = connect_with(
        PinnedTlsConnector { expected_identity },
        host,
        port,
        Some(expected_identity),
    )?;
    list_summary_from_session(&mut client, port)
}

pub fn list_remote_daemon_sessions_quic(
    host: &str,
    port: u16,
    expected_identity: &str,
) -> Result<RemoteSessionListSummary, String> {
    let mut client = connect_with(
        PinnedQuicConnector { expected_identity },
        host,
        port,
        Some(expected_identity),
    )?;
    list_summary_from_session(&mut client, port)
}

fn attach_summary_from_session<S: DirectReadWrite>(
    client: &mut DirectTransportSession<S>,
    port: u16,
    session_id: u32,
    attachment_id: Option<u64>,
    resume_token: Option<u64>,
) -> Result<RemoteAttachSummary, String> {
    let heartbeat_rtt_ms = client.heartbeat_round_trip(b"boo-remote-attach")?;
    let (attached, full_state) = client.attach(session_id, attachment_id, resume_token)?;
    Ok(RemoteAttachSummary {
        host: client.host.clone(),
        port,
        protocol_version: client.protocol_version,
        capabilities: client.capabilities,
        build_id: client.build_id.clone(),
        server_instance_id: client.server_instance_id.clone(),
        server_identity_id: client.server_identity_id.clone(),
        heartbeat_rtt_ms,
        attached,
        rows: full_state.rows,
        cols: full_state.cols,
        cursor_x: full_state.cursor_x,
        cursor_y: full_state.cursor_y,
        cursor_visible: full_state.cursor_visible,
        cursor_blinking: full_state.cursor_blinking,
        cursor_style: full_state.cursor_style,
    })
}

pub fn attach_remote_daemon_session(
    host: &str,
    port: u16,
    expected_server_identity: Option<&str>,
    session_id: u32,
    attachment_id: Option<u64>,
    resume_token: Option<u64>,
) -> Result<RemoteAttachSummary, String> {
    let mut client =
        connect_with(PlainTcpConnector, host, port, expected_server_identity)?;
    attach_summary_from_session(&mut client, port, session_id, attachment_id, resume_token)
}

pub fn attach_remote_daemon_session_tls(
    host: &str,
    port: u16,
    expected_identity: &str,
    session_id: u32,
    attachment_id: Option<u64>,
    resume_token: Option<u64>,
) -> Result<RemoteAttachSummary, String> {
    let mut client = connect_with(
        PinnedTlsConnector { expected_identity },
        host,
        port,
        Some(expected_identity),
    )?;
    attach_summary_from_session(&mut client, port, session_id, attachment_id, resume_token)
}

pub fn attach_remote_daemon_session_quic(
    host: &str,
    port: u16,
    expected_identity: &str,
    session_id: u32,
    attachment_id: Option<u64>,
    resume_token: Option<u64>,
) -> Result<RemoteAttachSummary, String> {
    let mut client = connect_with(
        PinnedQuicConnector { expected_identity },
        host,
        port,
        Some(expected_identity),
    )?;
    attach_summary_from_session(&mut client, port, session_id, attachment_id, resume_token)
}

fn create_summary_from_session<S: DirectReadWrite>(
    client: &mut DirectTransportSession<S>,
    port: u16,
    cols: u16,
    rows: u16,
) -> Result<RemoteCreateSummary, String> {
    let heartbeat_rtt_ms = client.heartbeat_round_trip(b"boo-remote-create")?;
    let session_id = client.create_session(cols, rows)?;
    Ok(RemoteCreateSummary {
        host: client.host.clone(),
        port,
        protocol_version: client.protocol_version,
        capabilities: client.capabilities,
        build_id: client.build_id.clone(),
        server_instance_id: client.server_instance_id.clone(),
        server_identity_id: client.server_identity_id.clone(),
        heartbeat_rtt_ms,
        session_id,
    })
}

pub fn create_remote_daemon_session(
    host: &str,
    port: u16,
    expected_server_identity: Option<&str>,
    cols: u16,
    rows: u16,
) -> Result<RemoteCreateSummary, String> {
    let mut client =
        connect_with(PlainTcpConnector, host, port, expected_server_identity)?;
    create_summary_from_session(&mut client, port, cols, rows)
}

pub fn create_remote_daemon_session_tls(
    host: &str,
    port: u16,
    expected_identity: &str,
    cols: u16,
    rows: u16,
) -> Result<RemoteCreateSummary, String> {
    let mut client = connect_with(
        PinnedTlsConnector { expected_identity },
        host,
        port,
        Some(expected_identity),
    )?;
    create_summary_from_session(&mut client, port, cols, rows)
}

pub fn create_remote_daemon_session_quic(
    host: &str,
    port: u16,
    expected_identity: &str,
    cols: u16,
    rows: u16,
) -> Result<RemoteCreateSummary, String> {
    let mut client = connect_with(
        PinnedQuicConnector { expected_identity },
        host,
        port,
        Some(expected_identity),
    )?;
    create_summary_from_session(&mut client, port, cols, rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote_types::RemoteSessionInfo;
    use crate::remote_wire::{
        MessageType, REMOTE_PROTOCOL_VERSION, RemoteCell, RemoteFullState, encode_auth_ok_payload,
        encode_full_state, encode_message, encode_session_list, parse_attach_request, read_message,
    };
    use std::io::Write;

    #[test]
    fn select_direct_transport_prefers_quic_when_migration_path_is_available() {
        let transport = select_direct_transport(
            REMOTE_CAPABILITY_TCP_DIRECT_TRANSPORT | REMOTE_CAPABILITY_QUIC_DIRECT_TRANSPORT,
            true,
        )
        .expect("selected transport");
        assert_eq!(transport, DirectTransportKind::QuicDirect);
    }

    #[test]
    fn select_direct_transport_falls_back_to_tcp_when_udp_path_is_unavailable() {
        let transport = select_direct_transport(
            REMOTE_CAPABILITY_TCP_DIRECT_TRANSPORT | REMOTE_CAPABILITY_QUIC_DIRECT_TRANSPORT,
            false,
        )
        .expect("selected transport");
        assert_eq!(transport, DirectTransportKind::TcpDirect);
    }

    #[test]
    fn select_direct_transport_rejects_quic_only_endpoints_without_fallback() {
        let error = select_direct_transport(REMOTE_CAPABILITY_QUIC_DIRECT_TRANSPORT, false)
            .expect_err("quic-only endpoint without migration-capable path should fail");
        assert!(error.contains("QUIC"));
    }

    #[test]
    fn probe_selected_direct_transport_requires_pin_for_quic() {
        // QUIC is now implemented but requires an SPKI pin (expected_server_identity)
        // because the QUIC handshake has no TOFU fallback. Without a pin the probe
        // must refuse cleanly instead of silently attempting an un-pinned handshake.
        let error = probe_selected_direct_transport(
            DirectTransportKind::QuicDirect,
            "127.0.0.1",
            7337,
            None,
        )
        .expect_err("quic probing without a pin should be rejected");
        assert!(
            error.contains("QUIC direct transport requires an expected_server_identity pin"),
            "expected pin-required error, got: {error}"
        );
    }

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
    fn list_remote_daemon_sessions_reuses_handshake_validation_over_socket() {
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

            let (ty, payload) = read_message(&mut stream).expect("read list sessions");
            assert_eq!(ty, MessageType::ListSessions);
            assert!(payload.is_empty());
            stream
                .write_all(&encode_message(
                    MessageType::SessionList,
                    &encode_session_list(&[RemoteSessionInfo {
                        id: 11,
                        name: "dev".to_string(),
                        title: "shell".to_string(),
                        pwd: "/home/example/dev/boo".to_string(),
                        attached: true,
                        child_exited: false,
                    }]),
                ))
                .expect("write session list");
        });

        let summary = list_remote_daemon_sessions("127.0.0.1", port, None)
            .expect("list sessions summary");
        assert_eq!(summary.protocol_version, REMOTE_PROTOCOL_VERSION);
        assert_eq!(summary.server_identity_id.as_deref(), Some("test-daemon"));
        assert_eq!(summary.server_instance_id.as_deref(), Some("test-instance"));
        assert_eq!(summary.sessions.len(), 1);
        assert_eq!(summary.sessions[0].id, 11);
        assert_eq!(summary.sessions[0].name, "dev");
        assert_eq!(summary.sessions[0].pwd, "/home/example/dev/boo");
        assert!(summary.sessions[0].attached);

        server.join().expect("list server thread");
    }

    #[test]
    fn list_remote_daemon_sessions_rejects_unexpected_server_identity() {
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

        let error = list_remote_daemon_sessions(
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
    fn attach_remote_daemon_session_reads_attached_and_initial_state_over_socket() {
        use std::net::TcpListener;

        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind test listener");
        let port = listener.local_addr().expect("listener addr").port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept attach client");
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

            let (ty, payload) = read_message(&mut stream).expect("read attach");
            assert_eq!(ty, MessageType::Attach);
            let (session_id, attachment_id, resume_token) =
                parse_attach_request(&payload).expect("decoded attach");
            assert_eq!(session_id, 7);
            assert_eq!(attachment_id, Some(99));
            assert_eq!(resume_token, Some(1234));

            let mut attached = 7_u32.to_le_bytes().to_vec();
            attached.extend_from_slice(&99_u64.to_le_bytes());
            attached.extend_from_slice(&1234_u64.to_le_bytes());
            stream
                .write_all(&encode_message(MessageType::Attached, &attached))
                .expect("write attached");

            let state = RemoteFullState {
                rows: 1,
                cols: 1,
                cursor_x: 0,
                cursor_y: 0,
                cursor_visible: true,
                cursor_blinking: true,
                cursor_style: 1,
                cells: vec![RemoteCell {
                    codepoint: u32::from('Z'),
                    fg: [1, 2, 3],
                    bg: [4, 5, 6],
                    style_flags: 0,
                    wide: false,
                }],
            };
            stream
                .write_all(&encode_message(
                    MessageType::FullState,
                    &encode_full_state(&state, None, false),
                ))
                .expect("write full state");
        });

        let summary = attach_remote_daemon_session(
            "127.0.0.1",
            port,
            Some("test-daemon"),
            7,
            Some(99),
            Some(1234),
        )
        .expect("attach summary");
        assert_eq!(summary.protocol_version, REMOTE_PROTOCOL_VERSION);
        assert_eq!(summary.server_identity_id.as_deref(), Some("test-daemon"));
        assert_eq!(summary.server_instance_id.as_deref(), Some("test-instance"));
        assert_eq!(summary.attached.session_id, 7);
        assert_eq!(summary.attached.attachment_id, Some(99));
        assert_eq!(summary.attached.resume_token, Some(1234));
        assert_eq!(summary.rows, 1);
        assert_eq!(summary.cols, 1);
        assert_eq!(summary.cursor_style, 1);

        server.join().expect("attach server thread");
    }

    #[test]
    fn attach_remote_daemon_session_tolerates_cached_session_list_before_attach() {
        use std::net::TcpListener;

        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind test listener");
        let port = listener.local_addr().expect("listener addr").port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept attach client");
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

            let (ty, payload) = read_message(&mut stream).expect("read attach");
            assert_eq!(ty, MessageType::Attach);
            let (session_id, attachment_id, resume_token) =
                parse_attach_request(&payload).expect("decoded attach");
            assert_eq!(session_id, 7);
            assert_eq!(attachment_id, None);
            assert_eq!(resume_token, None);

            stream
                .write_all(&encode_message(
                    MessageType::SessionList,
                    &encode_session_list(&[RemoteSessionInfo {
                        id: 7,
                        name: "Tab 7".to_string(),
                        title: "shell".to_string(),
                        pwd: "/tmp".to_string(),
                        attached: true,
                        child_exited: false,
                    }]),
                ))
                .expect("write session list");

            stream
                .write_all(&encode_message(MessageType::Attached, &7_u32.to_le_bytes()))
                .expect("write attached");

            let state = RemoteFullState {
                rows: 1,
                cols: 1,
                cursor_x: 0,
                cursor_y: 0,
                cursor_visible: true,
                cursor_blinking: false,
                cursor_style: 1,
                cells: vec![RemoteCell {
                    codepoint: u32::from('Q'),
                    fg: [1, 2, 3],
                    bg: [4, 5, 6],
                    style_flags: 0,
                    wide: false,
                }],
            };
            stream
                .write_all(&encode_message(
                    MessageType::FullState,
                    &encode_full_state(&state, None, false),
                ))
                .expect("write full state");
        });

        let summary = attach_remote_daemon_session("127.0.0.1", port, Some("test-daemon"), 7, None, None)
            .expect("attach summary");
        assert_eq!(summary.attached.session_id, 7);
        assert_eq!(summary.rows, 1);
        assert_eq!(summary.cols, 1);

        server.join().expect("attach server thread");
    }

    #[test]
    fn create_remote_daemon_session_uses_shared_handshake_and_create_path() {
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
                    MessageType::SessionCreated,
                    &77_u32.to_le_bytes(),
                ))
                .expect("write session created");
        });

        let summary = create_remote_daemon_session(
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
        assert_eq!(summary.session_id, 77);

        server.join().expect("create server thread");
    }}
