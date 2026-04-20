//! Transport-layer seam for direct-client connections.
//!
//! Extracted from `remote.rs` to isolate the "produce a handshake-ready
//! stream to a Boo daemon" concern from everything else. Adding or swapping a
//! transport (s2n-quic, a hypothetical DTLS, etc.) is a single
//! `RemoteTransportConnector` impl here, not a scatter of specialized methods
//! on `DirectTransportSession`.

use std::sync::Arc;
use std::time::Duration;

use crate::remote::{DirectReadWrite, DirectTransportSession};
use crate::remote_identity::{REMOTE_DAEMON_SERVER_NAME, build_remote_client_tls_config};

/// Default socket timeout for direct-client probes and RPCs. Generous enough
/// to tolerate the extra scheduling latency seen in constrained environments
/// like the Nix build sandbox on Darwin, where a 3-second timeout occasionally
/// trips during the TLS handshake + HMAC round trip.
pub(crate) const DIRECT_CLIENT_SOCKET_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) type TlsClientStream =
    rustls::StreamOwned<rustls::ClientConnection, std::net::TcpStream>;
pub(crate) type QuicClientStream = crate::remote_quic::QuicBridgeStream;

/// Abstraction over "produce a handshake-ready sync stream to a remote Boo
/// daemon."
///
/// Each transport (plain TCP, TCP+TLS with SPKI pinning, QUIC with SPKI
/// pinning) implements this trait. The rest of the direct-client plumbing —
/// auth, heartbeat, list/attach/create — only sees the finished `Read +
/// Write` stream and stays transport-agnostic.
pub(crate) trait RemoteTransportConnector {
    type Stream: DirectReadWrite + Send + 'static;

    /// Open a connection to `host:port` and complete any transport-level
    /// handshake. The returned stream carries application bytes only.
    fn connect(self, host: &str, port: u16) -> Result<Self::Stream, String>;
}

/// Plain TCP, no transport security. The returned stream is raw bytes; any
/// auth / identity verification happens at the application layer.
pub(crate) struct PlainTcpConnector;

impl RemoteTransportConnector for PlainTcpConnector {
    type Stream = std::net::TcpStream;

    fn connect(self, host: &str, port: u16) -> Result<Self::Stream, String> {
        let stream = std::net::TcpStream::connect((host, port))
            .map_err(|error| format!("failed to connect to {host}:{port}: {error}"))?;
        stream
            .set_read_timeout(Some(DIRECT_CLIENT_SOCKET_TIMEOUT))
            .map_err(|error| {
                format!("failed to configure read timeout for {host}:{port}: {error}")
            })?;
        stream
            .set_write_timeout(Some(DIRECT_CLIENT_SOCKET_TIMEOUT))
            .map_err(|error| {
                format!("failed to configure write timeout for {host}:{port}: {error}")
            })?;
        Ok(stream)
    }
}

/// TCP+TLS with SPKI-pinned cert verification. `expected_identity` is the
/// `base64url(sha256(SPKI))` of the server's ed25519 cert; the TLS handshake
/// aborts before any application data if the presented cert's SPKI does not
/// match.
pub(crate) struct PinnedTlsConnector<'a> {
    pub(crate) expected_identity: &'a str,
}

impl<'a> RemoteTransportConnector for PinnedTlsConnector<'a> {
    type Stream = TlsClientStream;

    fn connect(self, host: &str, port: u16) -> Result<Self::Stream, String> {
        use std::net::TcpStream;

        let tcp = TcpStream::connect((host, port))
            .map_err(|error| format!("failed to connect to {host}:{port}: {error}"))?;
        tcp.set_read_timeout(Some(DIRECT_CLIENT_SOCKET_TIMEOUT))
            .map_err(|error| {
                format!("failed to configure read timeout for {host}:{port}: {error}")
            })?;
        tcp.set_write_timeout(Some(DIRECT_CLIENT_SOCKET_TIMEOUT))
            .map_err(|error| {
                format!("failed to configure write timeout for {host}:{port}: {error}")
            })?;

        let client_config = build_remote_client_tls_config(self.expected_identity)?;
        let server_name = rustls::pki_types::ServerName::try_from(REMOTE_DAEMON_SERVER_NAME)
            .map_err(|error| format!("build remote server name: {error}"))?;
        let client_conn = rustls::ClientConnection::new(Arc::new(client_config), server_name)
            .map_err(|error| format!("build rustls client connection: {error}"))?;
        let mut tls_stream = rustls::StreamOwned::new(client_conn, tcp);
        tls_stream
            .conn
            .complete_io(&mut tls_stream.sock)
            .map_err(|error| format!("tls handshake to {host}:{port} failed: {error}"))?;
        Ok(tls_stream)
    }
}

/// QUIC with the same SPKI-pinned cert verification as `PinnedTlsConnector`.
/// Under the hood, quinn wraps the same `rustls::ClientConfig` + pinning
/// verifier, so the trust model is identical across TLS and QUIC.
pub(crate) struct PinnedQuicConnector<'a> {
    pub(crate) expected_identity: &'a str,
}

impl<'a> RemoteTransportConnector for PinnedQuicConnector<'a> {
    type Stream = QuicClientStream;

    fn connect(self, host: &str, port: u16) -> Result<Self::Stream, String> {
        let client_config = build_remote_client_tls_config(self.expected_identity)?;
        crate::remote_quic::connect_quic_client(
            host,
            port,
            REMOTE_DAEMON_SERVER_NAME,
            client_config,
        )
        .map_err(|error| format!("quic connect to {host}:{port} failed: {error}"))
    }
}

/// Open a `DirectTransportSession` using any transport that implements
/// `RemoteTransportConnector`. This is the single chokepoint that all four
/// public RPCs (probe / list / attach / create) plus their `_tls` / `_quic`
/// variants go through.
pub(crate) fn connect_with<C>(
    connector: C,
    host: &str,
    port: u16,
    expected_server_identity: Option<&str>,
) -> Result<DirectTransportSession<C::Stream>, String>
where
    C: RemoteTransportConnector,
{
    let stream = connector.connect(host, port)?;
    DirectTransportSession::<C::Stream>::connect_over_stream(
        stream,
        host.to_string(),
        port,
        expected_server_identity,
    )
}
