//! Accept-loop and per-connection socket lifecycle for the remote daemon.
//!
//! Handles three inbound transports:
//! * Plain TCP — `serve_incoming_tcp_client` peeks the first byte, and if it
//!   is not the TLS `ClientHello` sentinel, it hands the raw socket directly to
//!   [`run_remote_client_session`].
//! * TCP+TLS — the same peek routes into a rustls `ServerConnection`, which is
//!   wrapped in [`SharedStream`] so the reader and writer halves can share
//!   one session behind an `Arc<Mutex<_>>`.
//! * QUIC — [`spawn_quic_accept_loop`] binds a quinn endpoint on the shared
//!   tokio runtime and, for every bi-directional stream, shuttles bytes across
//!   the `remote_quic` sync bridge so the same blocking protocol loop can
//!   drive QUIC clients.
//!
//! Each connection ends up in [`run_remote_client_session`], which registers
//! a `ClientState`, spawns the writer thread, and hands the reader off to the
//! blocking `read_loop` in `remote.rs`. That layered structure — listener does
//! accept + protocol detection, read_loop does message dispatch — is why this
//! module is thin: it only owns the socket-to-state handoff.

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use crate::remote::RemoteCmd;
use crate::remote_auth::read_loop;
use crate::remote_batcher::writer_loop;
use crate::remote_state::{ClientState, State};
use crate::remote_wire::{PROTOCOL_PEEK_BYTES, TLS_HANDSHAKE_RECORD_TYPE};

/// Monotonic client-id allocator. Shared across transports so diagnostics can
/// cross-reference a client_id in the clients snapshot with the reader/writer
/// threads that serve it.
pub(crate) static NEXT_CLIENT_ID: AtomicU64 = AtomicU64::new(1);

/// Read timeout on the inner TCP socket for TLS-wrapped connections. Keep it
/// below the higher-level heartbeat/auth windows, but not so low that
/// scheduling jitter in constrained environments turns an otherwise healthy TLS
/// session into a stream of spurious WouldBlock errors.
const TLS_INNER_READ_TIMEOUT: Duration = Duration::from_secs(1);

const TRANSPORT_LABEL_TLS: &str = "tls";
const TRANSPORT_LABEL_PLAIN: &str = "plain";
const TRANSPORT_LABEL_QUIC: &str = "quic";

/// Bind a QUIC endpoint on `bind_addr` using the same rustls server config the
/// TCP+TLS path uses, and spawn an async accept loop on the shared QUIC
/// runtime. Returns the `QuicListener` so the caller can keep its lifetime
/// tied to the surrounding `RemoteServer`. `None` on bind failure — QUIC is
/// additive to TCP; if it cannot come up, the daemon still serves TCP.
pub(crate) fn spawn_quic_accept_loop(
    bind_addr: std::net::SocketAddr,
    tls_config: Arc<rustls::ServerConfig>,
    state: Arc<Mutex<State>>,
    cmd_tx: mpsc::Sender<RemoteCmd>,
) -> Option<crate::remote_quic::QuicListener> {
    let server_config = match crate::remote_quic::build_quic_server_config(tls_config) {
        Ok(cfg) => cfg,
        Err(error) => {
            log::warn!("remote quic: failed to build quinn server config: {error}");
            return None;
        }
    };
    let listener = match crate::remote_quic::bind_quic_listener(bind_addr, server_config) {
        Ok(listener) => listener,
        Err(error) => {
            log::warn!("remote quic: failed to bind {bind_addr}: {error}");
            return None;
        }
    };

    let runtime = match crate::remote_quic::shared_quic_runtime() {
        Ok(rt) => rt,
        Err(error) => {
            log::warn!("remote quic: shared runtime unavailable: {error}");
            return None;
        }
    };

    let endpoint = listener.endpoint();
    runtime.spawn(async move {
        while let Some(incoming) = endpoint.accept().await {
            let state = Arc::clone(&state);
            let cmd_tx = cmd_tx.clone();
            tokio::spawn(async move {
                handle_quic_incoming(incoming, state, cmd_tx).await;
            });
        }
    });

    Some(listener)
}

pub(crate) async fn handle_quic_incoming(
    incoming: quinn::Incoming,
    state: Arc<Mutex<State>>,
    cmd_tx: mpsc::Sender<RemoteCmd>,
) {
    let connection = match incoming.await {
        Ok(connection) => connection,
        Err(error) => {
            log::warn!("remote quic handshake failed: {error}");
            return;
        }
    };
    let remote_addr = connection.remote_address();
    let (send_stream, recv_stream) = match connection.accept_bi().await {
        Ok(pair) => pair,
        Err(error) => {
            log::warn!("remote quic accept_bi failed from {remote_addr}: {error}");
            return;
        }
    };

    let (inbound_tx, inbound_rx) = mpsc::channel::<io::Result<Vec<u8>>>();
    let (outbound_tx, outbound_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(
        crate::remote_quic::QUIC_OUTBOUND_CHANNEL_CAPACITY,
    );

    tokio::spawn(crate::remote_quic::run_quic_recv_pump(recv_stream, inbound_tx));
    tokio::spawn(crate::remote_quic::run_quic_send_pump(send_stream, outbound_rx));

    // Hand the sync bridge halves to an OS thread so the existing blocking
    // read_loop / writer_loop pair can drive the protocol exactly as it does
    // for TCP and TLS clients. Keeping QUIC's async surface strictly at the
    // transport boundary is the whole point of the bridge.
    std::thread::spawn(move || {
        let reader = crate::remote_quic::QuicBridgeReader::new(inbound_rx);
        let writer = crate::remote_quic::QuicBridgeWriter::new(outbound_tx);
        run_remote_client_session(reader, writer, state, cmd_tx, TRANSPORT_LABEL_QUIC);
    });
}

pub(crate) fn serve_incoming_tcp_client(
    stream: std::net::TcpStream,
    tls_config: Arc<rustls::ServerConfig>,
    state: Arc<Mutex<State>>,
    cmd_tx: mpsc::Sender<RemoteCmd>,
) {
    let mut peek_buf = [0u8; PROTOCOL_PEEK_BYTES];
    let peeked = match stream.peek(&mut peek_buf) {
        Ok(n) => n,
        Err(error) => {
            log::debug!("remote tcp client dropped before peek: {error}");
            return;
        }
    };
    if peeked < PROTOCOL_PEEK_BYTES {
        log::debug!("remote tcp client closed before protocol detection");
        return;
    }

    if peek_buf[0] == TLS_HANDSHAKE_RECORD_TYPE {
        let _ = stream.set_read_timeout(Some(TLS_INNER_READ_TIMEOUT));
        let tls_conn = match rustls::ServerConnection::new(tls_config) {
            Ok(conn) => conn,
            Err(error) => {
                log::warn!("remote tls: failed to construct server connection: {error}");
                return;
            }
        };
        let mut tls_stream = rustls::StreamOwned::new(tls_conn, stream);
        if let Err(error) = tls_stream.conn.complete_io(&mut tls_stream.sock) {
            log::warn!("remote tls handshake failed: {error}");
            return;
        }
        log::info!(
            "remote tls handshake completed: protocol_version={:?}",
            tls_stream.conn.protocol_version()
        );
        let shared = SharedStream::new(tls_stream);
        run_remote_client_session(
            shared.clone(),
            shared,
            state,
            cmd_tx,
            TRANSPORT_LABEL_TLS,
        );
    } else {
        let Ok(writer_stream) = stream.try_clone() else {
            log::warn!("remote tcp client dropped: failed to clone stream for writer");
            return;
        };
        run_remote_client_session(
            stream,
            writer_stream,
            state,
            cmd_tx,
            TRANSPORT_LABEL_PLAIN,
        );
    }
}

pub(crate) fn run_remote_client_session<R, W>(
    reader: R,
    writer: W,
    state: Arc<Mutex<State>>,
    cmd_tx: mpsc::Sender<RemoteCmd>,
    transport_label: &'static str,
) where
    R: Read + Send + 'static,
    W: Write + Send + 'static,
{
    let is_tls_transport = transport_label == TRANSPORT_LABEL_TLS;
    let (client_id, outbound_rx) = {
        let mut state = state.lock().expect("remote server state poisoned");
        let client_id = NEXT_CLIENT_ID.fetch_add(1, Ordering::Relaxed);
        let (outbound_tx, outbound_rx) = mpsc::channel();
        state.clients.insert(
            client_id,
            ClientState {
                outbound: outbound_tx,
                authenticated: true,
                connected_at: Instant::now(),
                authenticated_at: Some(Instant::now()),
                last_heartbeat_at: None,
                attached_session: None,
                attachment_id: None,
                resume_token: None,
                last_session_list_payload: None,
                last_ui_runtime_state_payload: None,
                last_ui_appearance_payload: None,
                last_state: None,
                pane_states: HashMap::new(),
                latest_input_seq: None,
                is_local: false,
            },
        );
        if is_tls_transport {
            state.tls_clients.insert(client_id);
        }
        (client_id, outbound_rx)
    };
    log::info!(
        "remote tcp client connected: client_id={client_id} transport={transport_label}"
    );

    std::thread::spawn(move || writer_loop(writer, outbound_rx, true, true));

    {
        let _ = cmd_tx.send(RemoteCmd::Connected { client_id });
        crate::notify_headless_wakeup();
    }
    read_loop(reader, client_id, Arc::clone(&state), cmd_tx);
    // Reader loop exited -> client disconnected. Scrub the TLS membership so the
    // HashSet does not leak entries for dead client ids.
    if is_tls_transport {
        let mut state = state.lock().expect("remote server state poisoned");
        state.tls_clients.remove(&client_id);
    }
}

/// `Read + Write + Send + Clone` handle around a single underlying stream. Used for the
/// TLS-wrapped path where the rustls session state cannot be duplicated via
/// `TcpStream::try_clone` the way the plain-TCP path does. Reader and writer threads each
/// hold a clone of the `Arc<Mutex<_>>` and serialize I/O; the lock hold time is bounded by
/// `TLS_INNER_READ_TIMEOUT` on the reader side so writers are not starved during idle
/// reads.
pub(crate) struct SharedStream {
    inner: Arc<Mutex<Box<dyn ReadWriteSend>>>,
}

trait ReadWriteSend: Read + Write + Send {}
impl<T: Read + Write + Send + ?Sized> ReadWriteSend for T {}

impl SharedStream {
    pub(crate) fn new<S: Read + Write + Send + 'static>(stream: S) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Box::new(stream))),
        }
    }
}

impl Clone for SharedStream {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl Read for SharedStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("shared remote stream poisoned"))?;
        guard.read(buf)
    }
}

impl Write for SharedStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("shared remote stream poisoned"))?;
        guard.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("shared remote stream poisoned"))?;
        guard.flush()
    }
}
