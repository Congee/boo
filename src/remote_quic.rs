//! QUIC direct transport for the native remote protocol.
//!
//! Design notes
//!
//! The existing remote subsystem is sync: `std::net::TcpListener`, blocking mpsc
//! channels, and per-connection OS threads running `read_loop` / `writer_loop`.
//! quinn is fundamentally async and tokio-driven, so we keep a *single* shared
//! tokio runtime on a dedicated thread and bridge to/from the sync world via two
//! channels per connection:
//!
//! - Inbound (quinn → sync reader): `std::sync::mpsc` carrying byte chunks.
//!   The async recv task pushes, the sync reader blocks on `recv()`.
//! - Outbound (sync writer → quinn): `tokio::sync::mpsc` bounded.
//!   The sync writer uses `blocking_send` from its OS thread; the async task
//!   drains into `quinn::SendStream::write_all`.
//!
//! The bridge exposes a `Read + Write + Send` handle (`QuicBridgeStream`) so the
//! existing `run_remote_client_session` / `DirectTransportSession::connect_over_stream`
//! machinery handles the protocol unchanged. That keeps the QUIC integration a
//! pure transport swap — the framing, auth, and session logic stay in `remote.rs`.
//!
//! Trust model reuses `PinnedSpkiServerCertVerifier` and the Phase 1 ed25519
//! cert directly — quinn accepts `rustls::ServerConfig`/`ClientConfig`, so QUIC
//! inherits the same SPKI pinning semantics as the TCP+TLS path.

use std::io::{self, Read, Write};
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::mpsc as std_mpsc;

use tokio::runtime::Runtime;

/// Name prefix for tokio worker threads spawned for QUIC work. Makes runtime
/// tasks visible in profiler/debugger stacks without colliding with other
/// threads Boo spawns.
const QUIC_RUNTIME_THREAD_NAME: &str = "boo-quic";

/// Worker thread count for the shared QUIC runtime. Two threads is enough to
/// keep the accept loop unblocked while pumps, timers, and user tasks make
/// progress, and small enough that an idle boo process with QUIC enabled does
/// not pin extra hardware threads.
const QUIC_RUNTIME_WORKER_THREADS: usize = 2;

/// Size of the outbound bounded channel carrying write-request buffers from the
/// sync writer into the async send loop. 32 hands out enough slack to let a
/// burst of terminal output queue without the sync writer blocking, while
/// keeping memory bounded under pathological backpressure. Referenced from
/// `remote.rs` via `pub(crate)` so server and client agree on one value.
pub(crate) const QUIC_OUTBOUND_CHANNEL_CAPACITY: usize = 32;

/// Application-level close code sent when `QuicListener` drops. 0 is the RFC 9000
/// "no error" application error code — matches a graceful daemon shutdown.
const QUIC_NORMAL_CLOSE_CODE: u32 = 0;

/// Process-global tokio runtime used for all QUIC endpoints in this process.
/// Lazily created so there is no tokio overhead in a build/run that never uses
/// QUIC. A single shared runtime keeps thread counts predictable; every QUIC
/// server and client task lives on it.
pub(crate) fn shared_quic_runtime() -> io::Result<&'static Runtime> {
    static RUNTIME: OnceLock<io::Result<Runtime>> = OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(QUIC_RUNTIME_WORKER_THREADS)
                .thread_name(QUIC_RUNTIME_THREAD_NAME)
                .enable_all()
                .build()
        })
        .as_ref()
        .map_err(|error| {
            io::Error::other(format!(
                "failed to initialize shared QUIC tokio runtime: {error}"
            ))
        })
}

/// Largest recv-chunk size requested from `quinn::RecvStream::read`. Chunks
/// larger than a caller's buffer are stashed in `QuicBridgeReader::pending_read`,
/// so this only bounds the pump task's transient allocation.
const QUIC_RECV_CHUNK_SIZE: usize = 64 * 1024;

/// Drain a `quinn::RecvStream` into a sync `std::sync::mpsc::Sender`. On EOF or
/// error the channel is closed (by dropping the sender when the task exits);
/// `QuicBridgeReader::read` turns both into a `read -> Ok(0)` or an `io::Error`
/// respectively.
pub(crate) async fn run_quic_recv_pump(
    mut recv: quinn::RecvStream,
    inbound: std_mpsc::Sender<io::Result<Vec<u8>>>,
) {
    let mut buf = vec![0u8; QUIC_RECV_CHUNK_SIZE];
    loop {
        match recv.read(&mut buf).await {
            Ok(None) => break,
            Ok(Some(0)) => break,
            Ok(Some(n)) => {
                if inbound.send(Ok(buf[..n].to_vec())).is_err() {
                    break;
                }
            }
            Err(error) => {
                let io_err = io::Error::other(format!("quic recv error: {error}"));
                let _ = inbound.send(Err(io_err));
                break;
            }
        }
    }
}

/// Drain a `tokio::sync::mpsc::Receiver<Vec<u8>>` into a `quinn::SendStream`.
/// The sender side (held by `QuicBridgeWriter`) closes the channel when the
/// sync writer is dropped, which ends this loop and finishes the stream.
pub(crate) async fn run_quic_send_pump(
    mut send: quinn::SendStream,
    mut outbound: tokio::sync::mpsc::Receiver<Vec<u8>>,
) {
    while let Some(chunk) = outbound.recv().await {
        if let Err(error) = send.write_all(&chunk).await {
            log::warn!("quic send error: {error}");
            break;
        }
    }
    let _ = send.finish();
}

/// Convert an existing rustls `ServerConfig` (the one produced from Phase 1's
/// ed25519 cert) into a `quinn::ServerConfig`. The underlying crypto is
/// identical — quinn just needs the config wrapped in its QUIC-specific
/// adapter.
pub(crate) fn build_quic_server_config(
    tls_config: Arc<rustls::ServerConfig>,
) -> io::Result<quinn::ServerConfig> {
    let crypto = quinn::crypto::rustls::QuicServerConfig::try_from((*tls_config).clone())
        .map_err(|error| io::Error::other(format!("build QUIC server crypto: {error}")))?;
    Ok(quinn::ServerConfig::with_crypto(Arc::new(crypto)))
}

/// Build a `quinn::ClientConfig` backed by a pinning rustls `ClientConfig`.
/// Callers pass in the full rustls config built via
/// `remote::build_remote_client_tls_config` so the same
/// `PinnedSpkiServerCertVerifier` guards QUIC handshakes as the TCP+TLS ones.
pub(crate) fn build_quic_client_config(
    tls_config: rustls::ClientConfig,
) -> io::Result<quinn::ClientConfig> {
    let crypto = quinn::crypto::rustls::QuicClientConfig::try_from(tls_config)
        .map_err(|error| io::Error::other(format!("build QUIC client crypto: {error}")))?;
    Ok(quinn::ClientConfig::new(Arc::new(crypto)))
}

/// Listener handle for a running QUIC endpoint. Dropping the handle closes the
/// endpoint's listening socket; in-flight connections continue until the async
/// task observes the drop.
pub(crate) struct QuicListener {
    endpoint: quinn::Endpoint,
}

impl QuicListener {
    #[cfg(test)]
    pub(crate) fn local_addr(&self) -> SocketAddr {
        self.endpoint
            .local_addr()
            .expect("quic endpoint should retain a local addr while listener is alive")
    }

    pub(crate) fn endpoint(&self) -> quinn::Endpoint {
        self.endpoint.clone()
    }
}

impl Drop for QuicListener {
    fn drop(&mut self) {
        self.endpoint
            .close(QUIC_NORMAL_CLOSE_CODE.into(), b"shutting down");
    }
}

/// Bind a QUIC endpoint on `bind_addr` using `server_config`. Returns the
/// active listener (caller drives the accept loop against it).
pub(crate) fn bind_quic_listener(
    bind_addr: SocketAddr,
    server_config: quinn::ServerConfig,
) -> io::Result<QuicListener> {
    let runtime = shared_quic_runtime()?;
    let _guard = runtime.enter();
    let endpoint = quinn::Endpoint::server(server_config, bind_addr)?;
    Ok(QuicListener { endpoint })
}

/// Synchronously open a QUIC connection to `host:port`, complete the handshake,
/// open a single bidirectional stream, and return a sync-facing
/// `QuicBridgeStream` that the existing `DirectTransportSession::connect_over_stream`
/// machinery can drive as if it were a TCP socket.
///
/// `client_config` should be built via `remote::build_remote_client_tls_config`
/// so the same `PinnedSpkiServerCertVerifier` guards the QUIC handshake as the
/// TCP+TLS one.
pub(crate) fn connect_quic_client(
    host: &str,
    port: u16,
    server_name: &str,
    client_config: rustls::ClientConfig,
) -> io::Result<QuicBridgeStream> {
    use std::net::ToSocketAddrs;

    let runtime = shared_quic_runtime()?;
    let quic_config = build_quic_client_config(client_config)?;

    let server_addr: SocketAddr = (host, port)
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| {
            io::Error::other(format!("unable to resolve quic endpoint {host}:{port}"))
        })?;

    // Endpoint must bind on a local UDP address in the same family as the
    // peer; IPv4 peers need 0.0.0.0:0, IPv6 peers need [::]:0.
    let local_bind: SocketAddr = if server_addr.is_ipv4() {
        "0.0.0.0:0".parse().expect("static ipv4 bind")
    } else {
        "[::]:0".parse().expect("static ipv6 bind")
    };

    let server_name_owned = server_name.to_string();
    let host_for_error = host.to_string();

    type BridgeHandles = (
        std_mpsc::Receiver<io::Result<Vec<u8>>>,
        tokio::sync::mpsc::Sender<Vec<u8>>,
    );
    let (init_tx, init_rx) = std_mpsc::channel::<io::Result<BridgeHandles>>();

    runtime.spawn(async move {
        let mut endpoint = match quinn::Endpoint::client(local_bind) {
            Ok(endpoint) => endpoint,
            Err(error) => {
                let _ = init_tx.send(Err(error));
                return;
            }
        };
        endpoint.set_default_client_config(quic_config);

        let connecting = match endpoint.connect(server_addr, &server_name_owned) {
            Ok(c) => c,
            Err(error) => {
                let _ = init_tx.send(Err(io::Error::other(format!(
                    "quic connect to {host_for_error}:{port}: {error}"
                ))));
                return;
            }
        };
        let connection = match connecting.await {
            Ok(c) => c,
            Err(error) => {
                let _ = init_tx.send(Err(io::Error::other(format!(
                    "quic handshake with {host_for_error}:{port}: {error}"
                ))));
                return;
            }
        };
        let (send_stream, recv_stream) = match connection.open_bi().await {
            Ok(pair) => pair,
            Err(error) => {
                let _ = init_tx.send(Err(io::Error::other(format!(
                    "quic open_bi with {host_for_error}:{port}: {error}"
                ))));
                return;
            }
        };

        let (inbound_tx, inbound_rx) = std_mpsc::channel::<io::Result<Vec<u8>>>();
        let (outbound_tx, outbound_rx) =
            tokio::sync::mpsc::channel::<Vec<u8>>(QUIC_OUTBOUND_CHANNEL_CAPACITY);

        // Hand the sync caller the bridge halves *before* spawning pumps so
        // the caller can start driving the protocol as soon as streams are
        // ready; the pumps then run in the background until the bridge
        // channels close.
        if init_tx.send(Ok((inbound_rx, outbound_tx))).is_err() {
            return;
        }

        let recv = tokio::spawn(run_quic_recv_pump(recv_stream, inbound_tx));
        let send = tokio::spawn(run_quic_send_pump(send_stream, outbound_rx));
        let _ = tokio::join!(recv, send);

        // Keep the endpoint + connection alive until pumps finish, then close
        // gracefully. endpoint.wait_idle().await returns when all connections
        // have fully drained.
        drop(connection);
        endpoint.wait_idle().await;
    });

    let (inbound_rx, outbound_tx) = init_rx
        .recv()
        .map_err(|_| io::Error::other("shared quic runtime shut down before connect completed"))?
        ?;
    Ok(QuicBridgeStream::new(inbound_rx, outbound_tx))
}

/// Sync-facing read half of the QUIC bridge.
///
/// Reads come from `inbound` (fed by an async recv pump); `pending_read` holds
/// leftover bytes when a recv chunk was larger than the caller's buffer.
pub(crate) struct QuicBridgeReader {
    inbound: std_mpsc::Receiver<io::Result<Vec<u8>>>,
    pending_read: Vec<u8>,
}

impl QuicBridgeReader {
    pub(crate) fn new(inbound: std_mpsc::Receiver<io::Result<Vec<u8>>>) -> Self {
        Self {
            inbound,
            pending_read: Vec::new(),
        }
    }
}

impl Read for QuicBridgeReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if !self.pending_read.is_empty() {
            let take = buf.len().min(self.pending_read.len());
            buf[..take].copy_from_slice(&self.pending_read[..take]);
            self.pending_read.drain(..take);
            return Ok(take);
        }
        match self.inbound.recv() {
            Ok(Ok(chunk)) => {
                if chunk.is_empty() {
                    return Ok(0);
                }
                let take = buf.len().min(chunk.len());
                buf[..take].copy_from_slice(&chunk[..take]);
                if take < chunk.len() {
                    self.pending_read.extend_from_slice(&chunk[take..]);
                }
                Ok(take)
            }
            Ok(Err(error)) => Err(error),
            Err(_) => Ok(0),
        }
    }
}

/// Sync-facing write half of the QUIC bridge.
///
/// Each `write` pushes a byte vector into the outbound tokio mpsc; the async
/// send pump drains into `quinn::SendStream::write_all`. `Clone` is implemented
/// because the caller (`DirectTransportSession::connect_over_stream`) plus
/// writer loops occasionally want independent handles on the same transport.
#[derive(Clone)]
pub(crate) struct QuicBridgeWriter {
    outbound: tokio::sync::mpsc::Sender<Vec<u8>>,
}

impl QuicBridgeWriter {
    pub(crate) fn new(outbound: tokio::sync::mpsc::Sender<Vec<u8>>) -> Self {
        Self { outbound }
    }
}

impl Write for QuicBridgeWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        self.outbound
            .blocking_send(buf.to_vec())
            .map_err(|_| io::Error::other("quic outbound channel closed"))?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Unified `Read + Write` facade for callers that want one stream handle (the
/// sync direct-client path passes a single value to
/// `DirectTransportSession::connect_over_stream`). Internally delegates to the
/// split reader/writer halves above.
pub(crate) struct QuicBridgeStream {
    reader: QuicBridgeReader,
    writer: QuicBridgeWriter,
}

impl QuicBridgeStream {
    pub(crate) fn new(
        inbound: std_mpsc::Receiver<io::Result<Vec<u8>>>,
        outbound: tokio::sync::mpsc::Sender<Vec<u8>>,
    ) -> Self {
        Self {
            reader: QuicBridgeReader::new(inbound),
            writer: QuicBridgeWriter::new(outbound),
        }
    }
}

impl Read for QuicBridgeStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.reader.read(buf)
    }
}

impl Write for QuicBridgeStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.writer.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quic_bridge_stream_read_splits_large_chunks() {
        let (tx, rx) = std_mpsc::channel::<io::Result<Vec<u8>>>();
        let (out_tx, _out_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(1);
        let mut stream = QuicBridgeStream::new(rx, out_tx);

        tx.send(Ok(vec![1, 2, 3, 4, 5])).expect("seed chunk");
        drop(tx);

        let mut buf = [0u8; 2];
        assert_eq!(stream.read(&mut buf).expect("read 1"), 2);
        assert_eq!(&buf, &[1, 2]);
        assert_eq!(stream.read(&mut buf).expect("read 2"), 2);
        assert_eq!(&buf, &[3, 4]);
        assert_eq!(stream.read(&mut buf).expect("read 3"), 1);
        assert_eq!(buf[0], 5);
        assert_eq!(stream.read(&mut buf).expect("eof"), 0);
    }

    #[test]
    fn quic_bridge_stream_surfaces_inbound_errors() {
        let (tx, rx) = std_mpsc::channel::<io::Result<Vec<u8>>>();
        let (out_tx, _out_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(1);
        let mut stream = QuicBridgeStream::new(rx, out_tx);
        tx.send(Err(io::Error::from(io::ErrorKind::ConnectionReset)))
            .expect("seed err");

        let mut buf = [0u8; 8];
        let err = stream.read(&mut buf).expect_err("must surface error");
        assert_eq!(err.kind(), io::ErrorKind::ConnectionReset);
    }

    #[test]
    fn shared_quic_runtime_returns_same_instance() {
        let a = shared_quic_runtime().expect("runtime a") as *const _;
        let b = shared_quic_runtime().expect("runtime b") as *const _;
        assert_eq!(a, b, "runtime must be a single OnceLock-initialised instance");
    }
}
