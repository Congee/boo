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
        auth_required: client.auth_required,
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
    auth_key: Option<&str>,
    expected_server_identity: Option<&str>,
) -> Result<RemoteProbeSummary, String> {
    let mut client =
        connect_with(PlainTcpConnector, host, port, auth_key, expected_server_identity)?;
    probe_summary_from_session(&mut client, port)
}

/// SPKI-pinned TLS variant of `probe_remote_endpoint`. `expected_identity` is
/// the `daemon_identity` string the caller already trusts; the TLS handshake
/// aborts if the presented cert's SPKI hash does not match.
pub fn probe_remote_endpoint_tls(
    host: &str,
    port: u16,
    auth_key: Option<&str>,
    expected_identity: &str,
) -> Result<RemoteProbeSummary, String> {
    let mut client = connect_with(
        PinnedTlsConnector { expected_identity },
        host,
        port,
        auth_key,
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
    auth_key: Option<&str>,
    expected_identity: &str,
) -> Result<RemoteProbeSummary, String> {
    let mut client = connect_with(
        PinnedQuicConnector { expected_identity },
        host,
        port,
        auth_key,
        Some(expected_identity),
    )?;
    probe_summary_from_session(&mut client, port)
}

pub fn probe_selected_direct_transport(
    transport: DirectTransportKind,
    host: &str,
    port: u16,
    auth_key: Option<&str>,
    expected_server_identity: Option<&str>,
) -> Result<RemoteUpgradeProbeSummary, String> {
    match transport {
        DirectTransportKind::TcpDirect => Ok(RemoteUpgradeProbeSummary {
            selected_transport: transport,
            probe: probe_remote_endpoint(host, port, auth_key, expected_server_identity)?,
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
                probe: probe_remote_endpoint_quic(host, port, auth_key, identity)?,
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
    auth_key: Option<&str>,
    expected_identity: &str,
) -> Result<RemoteUpgradeProbeSummary, String> {
    match transport {
        DirectTransportKind::TcpDirect => Ok(RemoteUpgradeProbeSummary {
            selected_transport: transport,
            probe: probe_remote_endpoint_tls(host, port, auth_key, expected_identity)?,
        }),
        DirectTransportKind::QuicDirect => Ok(RemoteUpgradeProbeSummary {
            selected_transport: transport,
            probe: probe_remote_endpoint_quic(host, port, auth_key, expected_identity)?,
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
        auth_required: client.auth_required,
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
    auth_key: Option<&str>,
    expected_server_identity: Option<&str>,
) -> Result<RemoteSessionListSummary, String> {
    let mut client =
        connect_with(PlainTcpConnector, host, port, auth_key, expected_server_identity)?;
    list_summary_from_session(&mut client, port)
}

pub fn list_remote_daemon_sessions_tls(
    host: &str,
    port: u16,
    auth_key: Option<&str>,
    expected_identity: &str,
) -> Result<RemoteSessionListSummary, String> {
    let mut client = connect_with(
        PinnedTlsConnector { expected_identity },
        host,
        port,
        auth_key,
        Some(expected_identity),
    )?;
    list_summary_from_session(&mut client, port)
}

pub fn list_remote_daemon_sessions_quic(
    host: &str,
    port: u16,
    auth_key: Option<&str>,
    expected_identity: &str,
) -> Result<RemoteSessionListSummary, String> {
    let mut client = connect_with(
        PinnedQuicConnector { expected_identity },
        host,
        port,
        auth_key,
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
        auth_required: client.auth_required,
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
    auth_key: Option<&str>,
    expected_server_identity: Option<&str>,
    session_id: u32,
    attachment_id: Option<u64>,
    resume_token: Option<u64>,
) -> Result<RemoteAttachSummary, String> {
    let mut client =
        connect_with(PlainTcpConnector, host, port, auth_key, expected_server_identity)?;
    attach_summary_from_session(&mut client, port, session_id, attachment_id, resume_token)
}

pub fn attach_remote_daemon_session_tls(
    host: &str,
    port: u16,
    auth_key: Option<&str>,
    expected_identity: &str,
    session_id: u32,
    attachment_id: Option<u64>,
    resume_token: Option<u64>,
) -> Result<RemoteAttachSummary, String> {
    let mut client = connect_with(
        PinnedTlsConnector { expected_identity },
        host,
        port,
        auth_key,
        Some(expected_identity),
    )?;
    attach_summary_from_session(&mut client, port, session_id, attachment_id, resume_token)
}

pub fn attach_remote_daemon_session_quic(
    host: &str,
    port: u16,
    auth_key: Option<&str>,
    expected_identity: &str,
    session_id: u32,
    attachment_id: Option<u64>,
    resume_token: Option<u64>,
) -> Result<RemoteAttachSummary, String> {
    let mut client = connect_with(
        PinnedQuicConnector { expected_identity },
        host,
        port,
        auth_key,
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
        auth_required: client.auth_required,
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
    auth_key: Option<&str>,
    expected_server_identity: Option<&str>,
    cols: u16,
    rows: u16,
) -> Result<RemoteCreateSummary, String> {
    let mut client =
        connect_with(PlainTcpConnector, host, port, auth_key, expected_server_identity)?;
    create_summary_from_session(&mut client, port, cols, rows)
}

pub fn create_remote_daemon_session_tls(
    host: &str,
    port: u16,
    auth_key: Option<&str>,
    expected_identity: &str,
    cols: u16,
    rows: u16,
) -> Result<RemoteCreateSummary, String> {
    let mut client = connect_with(
        PinnedTlsConnector { expected_identity },
        host,
        port,
        auth_key,
        Some(expected_identity),
    )?;
    create_summary_from_session(&mut client, port, cols, rows)
}

pub fn create_remote_daemon_session_quic(
    host: &str,
    port: u16,
    auth_key: Option<&str>,
    expected_identity: &str,
    cols: u16,
    rows: u16,
) -> Result<RemoteCreateSummary, String> {
    let mut client = connect_with(
        PinnedQuicConnector { expected_identity },
        host,
        port,
        auth_key,
        Some(expected_identity),
    )?;
    create_summary_from_session(&mut client, port, cols, rows)
}
