use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::TcpListener;
use std::os::unix::net::UnixListener;
#[cfg(test)]
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

type HmacSha256 = Hmac<Sha256>;

const MAGIC: [u8; 2] = [0x47, 0x53];
const HEADER_LEN: usize = 7;
static NEXT_CLIENT_ID: AtomicU64 = AtomicU64::new(1);
pub const REMOTE_PROTOCOL_VERSION: u16 = 1;
pub const REMOTE_CAPABILITY_HMAC_AUTH: u32 = 1 << 0;
pub const REMOTE_CAPABILITY_SCREEN_DELTAS: u32 = 1 << 1;
pub const REMOTE_CAPABILITY_UI_STATE: u32 = 1 << 2;
pub const REMOTE_CAPABILITY_IMAGES: u32 = 1 << 3;
pub const REMOTE_CAPABILITY_HEARTBEAT: u32 = 1 << 4;
pub const REMOTE_CAPABILITY_ATTACHMENT_RESUME: u32 = 1 << 5;
pub const REMOTE_CAPABILITY_DAEMON_IDENTITY: u32 = 1 << 6;
pub const REMOTE_CAPABILITY_TCP_DIRECT_TRANSPORT: u32 = 1 << 7;
pub const REMOTE_CAPABILITY_QUIC_DIRECT_TRANSPORT: u32 = 1 << 8;
pub const REMOTE_CAPABILITY_TCP_TLS_TRANSPORT: u32 = 1 << 9;
pub const REMOTE_CAPABILITIES: u32 = REMOTE_CAPABILITY_HMAC_AUTH
    | REMOTE_CAPABILITY_SCREEN_DELTAS
    | REMOTE_CAPABILITY_UI_STATE
    | REMOTE_CAPABILITY_IMAGES
    | REMOTE_CAPABILITY_HEARTBEAT
    | REMOTE_CAPABILITY_ATTACHMENT_RESUME
    | REMOTE_CAPABILITY_DAEMON_IDENTITY
    | REMOTE_CAPABILITY_TCP_DIRECT_TRANSPORT
    | REMOTE_CAPABILITY_TCP_TLS_TRANSPORT
    | REMOTE_CAPABILITY_QUIC_DIRECT_TRANSPORT;

/// First byte of a TLS 1.x record when the record carries a handshake (RFC 8446 §5.1).
/// Distinguishes an incoming TLS ClientHello from Boo's plain-TCP wire format, whose
/// first byte is `MAGIC[0]` (0x47).
const TLS_HANDSHAKE_RECORD_TYPE: u8 = 0x16;
const PROTOCOL_PEEK_BYTES: usize = 1;
/// Read timeout on the inner TCP socket for TLS-wrapped connections. Lower than
/// `REMOTE_READ_TIMEOUT` so that the reader thread releases the TLS stream lock
/// promptly and the writer thread does not stall on idle connections.
const TLS_INNER_READ_TIMEOUT: Duration = Duration::from_millis(100);

const LOCAL_INPUT_SEQ_LEN: usize = 8;
const REMOTE_FULL_STATE_HEADER_LEN: usize = 14;
const REMOTE_DELTA_HEADER_LEN: usize = 13;
#[cfg(test)]
const LOCAL_DELTA_HEADER_LEN: usize = LOCAL_INPUT_SEQ_LEN + REMOTE_DELTA_HEADER_LEN;
const REMOTE_CELL_ENCODED_LEN: usize = 12;
const REVIVABLE_ATTACHMENT_WINDOW: Duration = Duration::from_secs(30);
const AUTH_CHALLENGE_WINDOW: Duration = Duration::from_secs(10);
const DIRECT_CLIENT_HEARTBEAT_WINDOW: Duration = Duration::from_secs(20);
const REMOTE_READ_TIMEOUT: Duration = Duration::from_secs(1);
const STYLE_FLAG_BOLD: u8 = 0x01;
const STYLE_FLAG_ITALIC: u8 = 0x02;
const STYLE_FLAG_HYPERLINK: u8 = 0x04;
const STYLE_FLAG_EXPLICIT_FG: u8 = 0x20;
const STYLE_FLAG_EXPLICIT_BG: u8 = 0x40;

enum OutboundMessage {
    Frame(Vec<u8>),
    ScreenUpdate(Vec<u8>),
}

enum AuthHandling {
    Authenticated,
    Pending,
    Disconnect,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageType {
    Auth = 0x01,
    ListSessions = 0x02,
    Attach = 0x03,
    Detach = 0x04,
    Create = 0x05,
    Input = 0x06,
    Resize = 0x07,
    Destroy = 0x08,
    AuthChallenge = 0x09,
    Scroll = 0x0a,
    Key = 0x0b,
    ExecuteCommand = 0x0c,
    AppAction = 0x0d,
    AppKeyEvent = 0x0e,
    FocusPane = 0x0f,
    AppMouseEvent = 0x10,
    Heartbeat = 0x11,

    AuthOk = 0x80,
    AuthFail = 0x81,
    SessionList = 0x82,
    FullState = 0x83,
    Delta = 0x84,
    Attached = 0x85,
    Detached = 0x86,
    ErrorMsg = 0x87,
    SessionCreated = 0x88,
    SessionExited = 0x89,
    ScrollData = 0x8a,
    Clipboard = 0x8b,
    Image = 0x8c,
    UiRuntimeState = 0x8d,
    UiAppearance = 0x8e,
    UiPaneFullState = 0x90,
    UiPaneDelta = 0x91,
    HeartbeatAck = 0x92,
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogicalChannel {
    Control,
    SessionStream,
    InputControl,
    Health,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DirectTransportKind {
    TcpDirect,
    QuicDirect,
}

#[cfg_attr(not(test), allow(dead_code))]
pub const fn logical_channel_for_message_type(message_type: MessageType) -> LogicalChannel {
    match message_type {
        MessageType::Auth
        | MessageType::AuthChallenge
        | MessageType::AuthOk
        | MessageType::AuthFail
        | MessageType::ListSessions
        | MessageType::SessionList
        | MessageType::Create
        | MessageType::SessionCreated
        | MessageType::Destroy
        | MessageType::SessionExited
        | MessageType::ErrorMsg => LogicalChannel::Control,
        MessageType::Attach
        | MessageType::Attached
        | MessageType::Detach
        | MessageType::Detached
        | MessageType::FullState
        | MessageType::Delta
        | MessageType::ScrollData
        | MessageType::UiRuntimeState
        | MessageType::UiAppearance
        | MessageType::UiPaneFullState
        | MessageType::UiPaneDelta => LogicalChannel::SessionStream,
        MessageType::Input
        | MessageType::Resize
        | MessageType::Scroll
        | MessageType::Key
        | MessageType::ExecuteCommand
        | MessageType::AppAction
        | MessageType::AppKeyEvent
        | MessageType::AppMouseEvent
        | MessageType::FocusPane => LogicalChannel::InputControl,
        MessageType::Heartbeat | MessageType::HeartbeatAck => LogicalChannel::Health,
        MessageType::Clipboard | MessageType::Image => LogicalChannel::SessionStream,
    }
}

impl TryFrom<u8> for MessageType {
    type Error = ();

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        let message = match value {
            0x01 => Self::Auth,
            0x02 => Self::ListSessions,
            0x03 => Self::Attach,
            0x04 => Self::Detach,
            0x05 => Self::Create,
            0x06 => Self::Input,
            0x07 => Self::Resize,
            0x08 => Self::Destroy,
            0x09 => Self::AuthChallenge,
            0x0a => Self::Scroll,
            0x0b => Self::Key,
            0x0c => Self::ExecuteCommand,
            0x0d => Self::AppAction,
            0x0e => Self::AppKeyEvent,
            0x0f => Self::FocusPane,
            0x10 => Self::AppMouseEvent,
            0x11 => Self::Heartbeat,
            0x80 => Self::AuthOk,
            0x81 => Self::AuthFail,
            0x82 => Self::SessionList,
            0x83 => Self::FullState,
            0x84 => Self::Delta,
            0x85 => Self::Attached,
            0x86 => Self::Detached,
            0x87 => Self::ErrorMsg,
            0x88 => Self::SessionCreated,
            0x89 => Self::SessionExited,
            0x8a => Self::ScrollData,
            0x8b => Self::Clipboard,
            0x8c => Self::Image,
            0x8d => Self::UiRuntimeState,
            0x8e => Self::UiAppearance,
            0x90 => Self::UiPaneFullState,
            0x91 => Self::UiPaneDelta,
            0x92 => Self::HeartbeatAck,
            _ => return Err(()),
        };
        Ok(message)
    }
}

#[derive(Clone, Debug)]
pub struct RemoteConfig {
    pub port: u16,
    pub bind_address: Option<String>,
    pub auth_key: Option<String>,
    pub allow_insecure_no_auth: bool,
    pub service_name: String,
}

impl RemoteConfig {
    fn effective_bind_address(&self) -> &str {
        self.bind_address.as_deref().unwrap_or_else(|| {
            if self.auth_key.is_some() {
                "0.0.0.0"
            } else {
                "127.0.0.1"
            }
        })
    }

    fn should_advertise(&self) -> bool {
        !matches!(self.effective_bind_address(), "127.0.0.1" | "localhost" | "::1")
    }

    fn rejects_public_authless_bind(&self) -> bool {
        self.auth_key.is_none() && self.should_advertise() && !self.allow_insecure_no_auth
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteSessionInfo {
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
    pub attached_session: Option<u32>,
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
    pub session_id: u32,
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
    pub auth_required: bool,
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
    pub auth_required: bool,
    pub protocol_version: u16,
    pub capabilities: u32,
    pub build_id: Option<String>,
    pub server_instance_id: Option<String>,
    pub server_identity_id: Option<String>,
    pub heartbeat_rtt_ms: u64,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct RemoteDirectSessionInfo {
    pub id: u32,
    pub name: String,
    pub title: String,
    pub pwd: String,
    pub attached: bool,
    pub child_exited: bool,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct RemoteSessionListSummary {
    pub host: String,
    pub port: u16,
    pub auth_required: bool,
    pub protocol_version: u16,
    pub capabilities: u32,
    pub build_id: Option<String>,
    pub server_instance_id: Option<String>,
    pub server_identity_id: Option<String>,
    pub heartbeat_rtt_ms: u64,
    pub sessions: Vec<RemoteDirectSessionInfo>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct RemoteAttachedSummary {
    pub session_id: u32,
    pub attachment_id: Option<u64>,
    pub resume_token: Option<u64>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct RemoteAttachSummary {
    pub host: String,
    pub port: u16,
    pub auth_required: bool,
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
    pub auth_required: bool,
    pub protocol_version: u16,
    pub capabilities: u32,
    pub build_id: Option<String>,
    pub server_instance_id: Option<String>,
    pub server_identity_id: Option<String>,
    pub heartbeat_rtt_ms: u64,
    pub session_id: u32,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct RemoteUpgradeProbeSummary {
    pub selected_transport: DirectTransportKind,
    pub probe: RemoteProbeSummary,
}

trait DirectReadWrite: Read + Write {}
impl<T: Read + Write> DirectReadWrite for T {}

struct DirectTransportSession<S: DirectReadWrite> {
    stream: S,
    host: String,
    port: u16,
    auth_required: bool,
    protocol_version: u16,
    capabilities: u32,
    build_id: Option<String>,
    server_instance_id: Option<String>,
    server_identity_id: Option<String>,
}

type DirectRemoteClient = DirectTransportSession<std::net::TcpStream>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteCell {
    pub codepoint: u32,
    pub fg: [u8; 3],
    pub bg: [u8; 3],
    pub style_flags: u8,
    pub wide: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteFullState {
    pub rows: u16,
    pub cols: u16,
    pub cursor_x: u16,
    pub cursor_y: u16,
    pub cursor_visible: bool,
    pub cursor_blinking: bool,
    pub cursor_style: i32,
    pub cells: Vec<RemoteCell>,
}

#[derive(Debug)]
pub enum RemoteCmd {
    Connected {
        client_id: u64,
    },
    ListSessions {
        client_id: u64,
    },
    Attach {
        client_id: u64,
        session_id: u32,
        attachment_id: Option<u64>,
        resume_token: Option<u64>,
    },
    Detach {
        client_id: u64,
    },
    Create {
        client_id: u64,
        cols: u16,
        rows: u16,
    },
    Input {
        client_id: u64,
        bytes: Vec<u8>,
        input_seq: Option<u64>,
    },
    Key {
        client_id: u64,
        keyspec: String,
        input_seq: Option<u64>,
    },
    Resize {
        client_id: u64,
        cols: u16,
        rows: u16,
    },
    ExecuteCommand {
        client_id: u64,
        input: String,
    },
    AppKeyEvent {
        client_id: u64,
        event: crate::AppKeyEvent,
    },
    AppMouseEvent {
        client_id: u64,
        event: crate::AppMouseEvent,
    },
    AppAction {
        client_id: u64,
        action: crate::bindings::Action,
    },
    FocusPane {
        client_id: u64,
        pane_id: u64,
    },
    Destroy {
        client_id: u64,
        session_id: Option<u32>,
    },
}

struct ClientState {
    outbound: mpsc::Sender<OutboundMessage>,
    authenticated: bool,
    challenge: Option<AuthChallengeState>,
    connected_at: Instant,
    authenticated_at: Option<Instant>,
    last_heartbeat_at: Option<Instant>,
    attached_session: Option<u32>,
    attachment_id: Option<u64>,
    resume_token: Option<u64>,
    last_session_list_payload: Option<Vec<u8>>,
    last_ui_runtime_state_payload: Option<Vec<u8>>,
    last_ui_appearance_payload: Option<Vec<u8>>,
    last_state: Option<Arc<RemoteFullState>>,
    pane_states: HashMap<u64, Arc<RemoteFullState>>,
    latest_input_seq: Option<u64>,
    is_local: bool,
}

#[derive(Clone, Copy)]
struct AuthChallengeState {
    bytes: [u8; 32],
    expires_at: Instant,
}

#[derive(Clone)]
struct RevivableAttachment {
    session_id: u32,
    resume_token: u64,
    last_state: Option<Arc<RemoteFullState>>,
    pane_states: HashMap<u64, Arc<RemoteFullState>>,
    latest_input_seq: Option<u64>,
    expires_at: Instant,
}

struct State {
    clients: HashMap<u64, ClientState>,
    revivable_attachments: HashMap<u64, RevivableAttachment>,
    auth_key: Option<Vec<u8>>,
    server_identity_id: String,
    server_instance_id: String,
    /// client_ids whose underlying transport is TLS-wrapped TCP. Plain TCP and local
    /// Unix-socket clients are absent. Populated at client registration and scrubbed on
    /// removal; diagnostics look up membership here to distinguish plain-TCP from
    /// TCP-TLS in `transport_kind`.
    tls_clients: std::collections::HashSet<u64>,
}

pub struct RemoteServer {
    state: Arc<Mutex<State>>,
    _listener: std::thread::JoinHandle<()>,
    _advertiser: Option<ServiceAdvertiser>,
    local_socket_path: Option<PathBuf>,
    bind_address: Option<String>,
    port: Option<u16>,
    /// Active QUIC listener. Kept alive as a field so the endpoint closes when
    /// the server drops. `None` on local-stream servers or if QUIC bind failed
    /// (TCP is the authoritative transport; QUIC is additive).
    _quic_listener: Option<crate::remote_quic::QuicListener>,
}

struct ServiceAdvertiser {
    child: Child,
}

impl Drop for ServiceAdvertiser {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl RemoteServer {
    fn prune_revivable_attachments(state: &mut State) {
        let now = Instant::now();
        state
            .revivable_attachments
            .retain(|_, attachment| attachment.expires_at > now);
    }

    pub fn start(config: RemoteConfig) -> io::Result<(Self, mpsc::Receiver<RemoteCmd>)> {
        if config.rejects_public_authless_bind() {
            return Err(io::Error::other(format!(
                "refusing to start authless remote daemon on public bind address {}; configure --remote-auth-key or --remote-allow-insecure-no-auth",
                config.effective_bind_address()
            )));
        }
        let bind_address = config.effective_bind_address().to_string();
        let listener = TcpListener::bind((bind_address.as_str(), config.port))?;
        let identity_material = load_or_create_daemon_identity_material();
        let tls_config = build_remote_server_tls_config(&identity_material)
            .map_err(io::Error::other)?;
        let state = Arc::new(Mutex::new(State {
            clients: HashMap::new(),
            revivable_attachments: HashMap::new(),
            auth_key: config.auth_key.clone().map(|key| key.into_bytes()),
            server_identity_id: identity_material.identity_id,
            server_instance_id: random_instance_id(),
            tls_clients: std::collections::HashSet::new(),
        }));
        let state_for_listener = Arc::clone(&state);
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let cmd_tx_for_tcp = cmd_tx.clone();
        let tls_config_for_tcp = Arc::clone(&tls_config);
        let listener_thread = std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else {
                    continue;
                };
                let _ = stream.set_read_timeout(Some(REMOTE_READ_TIMEOUT));
                let state = Arc::clone(&state_for_listener);
                let cmd_tx = cmd_tx_for_tcp.clone();
                let tls_config = Arc::clone(&tls_config_for_tcp);
                std::thread::spawn(move || {
                    serve_incoming_tcp_client(stream, tls_config, state, cmd_tx);
                });
            }
        });

        // QUIC listener binds the same port number on UDP. Failure is non-fatal
        // — TCP remains the authoritative transport and servers that cannot
        // open the UDP port (permissions, port already bound by another
        // process) just keep running without QUIC.
        let quic_bind_addr: std::net::SocketAddr = format!("{bind_address}:{}", config.port)
            .parse()
            .map_err(io::Error::other)?;
        let quic_listener =
            spawn_quic_accept_loop(quic_bind_addr, tls_config, Arc::clone(&state), cmd_tx.clone());

        let advertiser = if config.should_advertise() {
            ServiceAdvertiser::spawn(&config.service_name, config.port)
        } else {
            None
        };
        {
            let state = state.lock().expect("remote server state poisoned");
            log::info!(
                "remote tcp server started: bind_address={} port={} auth_required={} protocol_version={} capabilities={} build_id={} server_identity_id={} server_instance_id={}",
                bind_address,
                config.port,
                state.auth_key.is_some(),
                REMOTE_PROTOCOL_VERSION,
                REMOTE_CAPABILITIES,
                env!("CARGO_PKG_VERSION"),
                state.server_identity_id,
                state.server_instance_id
            );
        }
        Ok((
            Self {
                state,
                _listener: listener_thread,
                _advertiser: advertiser,
                local_socket_path: None,
                bind_address: Some(bind_address),
                port: Some(config.port),
                _quic_listener: quic_listener,
            },
            cmd_rx,
        ))
    }

    pub fn start_local(
        socket_path: impl AsRef<Path>,
    ) -> io::Result<(Self, mpsc::Receiver<RemoteCmd>)> {
        let socket_path = socket_path.as_ref().to_path_buf();
        let _ = std::fs::remove_file(&socket_path);
        let listener = UnixListener::bind(&socket_path)?;
        let state = Arc::new(Mutex::new(State {
            clients: HashMap::new(),
            revivable_attachments: HashMap::new(),
            auth_key: None,
            server_identity_id: load_or_create_daemon_identity(),
            server_instance_id: random_instance_id(),
            tls_clients: std::collections::HashSet::new(),
        }));
        let state_for_listener = Arc::clone(&state);
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let listener_thread = std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else {
                    continue;
                };
                let (client_id, outbound_rx) = {
                    let mut state = state_for_listener
                        .lock()
                        .expect("remote server state poisoned");
                    let client_id = NEXT_CLIENT_ID.fetch_add(1, Ordering::Relaxed);
                    let (outbound_tx, outbound_rx) = mpsc::channel();
                    state.clients.insert(
                        client_id,
                        ClientState {
                            outbound: outbound_tx,
                            authenticated: true,
                            challenge: None,
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
                            is_local: true,
                        },
                    );
                    (client_id, outbound_rx)
                };
                log::info!("remote local-stream client connected: client_id={client_id}");

                let Ok(writer_stream) = stream.try_clone() else {
                    let mut state = state_for_listener
                        .lock()
                        .expect("remote server state poisoned");
                    state.clients.remove(&client_id);
                    continue;
                };
                std::thread::spawn(move || writer_loop(writer_stream, outbound_rx, false, false));

                let cmd_tx = cmd_tx.clone();
                let state = Arc::clone(&state_for_listener);
                let _ = cmd_tx.send(RemoteCmd::Connected { client_id });
                crate::notify_headless_wakeup();
                std::thread::spawn(move || read_loop(stream, client_id, state, cmd_tx));
            }
        });

        {
            let state = state.lock().expect("remote server state poisoned");
            log::info!(
                "remote local-stream server started: socket={} protocol_version={} capabilities={} build_id={} server_identity_id={} server_instance_id={}",
                socket_path.display(),
                REMOTE_PROTOCOL_VERSION,
                REMOTE_CAPABILITIES,
                env!("CARGO_PKG_VERSION"),
                state.server_identity_id,
                state.server_instance_id
            );
        }
        Ok((
            Self {
                state,
                _listener: listener_thread,
                _advertiser: None,
                local_socket_path: Some(socket_path),
                bind_address: None,
                port: None,
                // Local Unix-socket servers never serve a network transport,
                // so there is no QUIC listener to hold here.
                _quic_listener: None,
            },
            cmd_rx,
        ))
    }

    pub fn has_attached_sessions(&self) -> bool {
        let state = self.state.lock().expect("remote server state poisoned");
        state
            .clients
            .values()
            .any(|client| client.attached_session.is_some())
    }

    pub fn attached_to_session(&self, session_id: u32) -> bool {
        let state = self.state.lock().expect("remote server state poisoned");
        state
            .clients
            .values()
            .any(|client| client.attached_session == Some(session_id))
    }

    pub fn local_attached_to_session(&self, session_id: u32) -> bool {
        let state = self.state.lock().expect("remote server state poisoned");
        state.clients.values().any(|client| {
            client.is_local && client.attached_session == Some(session_id)
        })
    }

    pub fn client_session(&self, client_id: u64) -> Option<u32> {
        let state = self.state.lock().expect("remote server state poisoned");
        state
            .clients
            .get(&client_id)
            .and_then(|client| client.attached_session)
    }

    pub fn has_client(&self, client_id: u64) -> bool {
        let state = self.state.lock().expect("remote server state poisoned");
        state.clients.contains_key(&client_id)
    }

    pub fn client_is_local(&self, client_id: u64) -> bool {
        let state = self.state.lock().expect("remote server state poisoned");
        state
            .clients
            .get(&client_id)
            .is_some_and(|client| client.is_local)
    }

    pub fn clients_snapshot(&self) -> RemoteClientsSnapshot {
        let state = self.state.lock().expect("remote server state poisoned");
        let now = Instant::now();
        let connected_clients = state.clients.len();
        let attached_clients = state
            .clients
            .values()
            .filter(|client| client.attached_session.is_some())
            .count();
        let pending_auth_clients = state
            .clients
            .values()
            .filter(|client| client.challenge.is_some())
            .count();
        let revivable_attachments = state.revivable_attachments.len();
        let servers = vec![RemoteServerInfo {
            local_socket_path: self
                .local_socket_path
                .as_ref()
                .map(|path| path.display().to_string()),
            bind_address: self.bind_address.clone(),
            port: self.port,
            protocol_version: REMOTE_PROTOCOL_VERSION,
            capabilities: REMOTE_CAPABILITIES,
            build_id: env!("CARGO_PKG_VERSION").to_string(),
            server_instance_id: state.server_instance_id.clone(),
            server_identity_id: state.server_identity_id.clone(),
            auth_required: state.auth_key.is_some(),
            auth_challenge_window_ms: AUTH_CHALLENGE_WINDOW.as_millis() as u64,
            heartbeat_window_ms: DIRECT_CLIENT_HEARTBEAT_WINDOW.as_millis() as u64,
            revive_window_ms: REVIVABLE_ATTACHMENT_WINDOW.as_millis() as u64,
            connected_clients,
            attached_clients,
            pending_auth_clients,
            revivable_attachments,
        }];
        let mut clients = state
            .clients
            .iter()
            .map(|(client_id, client)| {
                let heartbeat_deadline = if client.authenticated && !client.is_local {
                    client
                        .last_heartbeat_at
                        .or(client.authenticated_at)
                        .map(|last_liveness| last_liveness + DIRECT_CLIENT_HEARTBEAT_WINDOW)
                } else {
                    None
                };
                RemoteClientInfo {
                    client_id: *client_id,
                    authenticated: client.authenticated,
                    is_local: client.is_local,
                    transport_kind: if client.is_local {
                        "local".to_string()
                    } else if state.tls_clients.contains(client_id) {
                        "tcp-tls".to_string()
                    } else {
                        "tcp".to_string()
                    },
                    server_socket_path: self
                        .local_socket_path
                        .as_ref()
                        .map(|path| path.display().to_string()),
                    challenge_pending: client.challenge.is_some(),
                    attached_session: client.attached_session,
                    attachment_id: client.attachment_id,
                    resume_token_present: client.resume_token.is_some(),
                    has_cached_state: client.last_state.is_some(),
                    pane_state_count: client.pane_states.len(),
                    latest_input_seq: client.latest_input_seq,
                    connection_age_ms: elapsed_ms(now, client.connected_at),
                    authenticated_age_ms: client
                        .authenticated_at
                        .map(|authenticated_at| elapsed_ms(now, authenticated_at)),
                    last_heartbeat_age_ms: client
                        .last_heartbeat_at
                        .map(|last_heartbeat_at| elapsed_ms(now, last_heartbeat_at)),
                    heartbeat_expires_in_ms: heartbeat_deadline
                        .map(|deadline| remaining_ms(now, deadline)),
                    heartbeat_overdue: heartbeat_deadline
                        .is_some_and(|deadline| now >= deadline),
                    challenge_expires_in_ms: client
                        .challenge
                        .map(|challenge| remaining_ms(now, challenge.expires_at)),
                }
            })
            .collect::<Vec<_>>();
        clients.sort_by_key(|client| client.client_id);

        let mut revivable_attachments = state
            .revivable_attachments
            .iter()
            .map(|(attachment_id, attachment)| RevivableAttachmentInfo {
                attachment_id: *attachment_id,
                session_id: attachment.session_id,
                resume_token_present: true,
                has_cached_state: attachment.last_state.is_some(),
                pane_state_count: attachment.pane_states.len(),
                latest_input_seq: attachment.latest_input_seq,
                revive_expires_in_ms: remaining_ms(now, attachment.expires_at),
            })
            .collect::<Vec<_>>();
        revivable_attachments.sort_by_key(|attachment| attachment.attachment_id);

        RemoteClientsSnapshot {
            servers,
            clients,
            revivable_attachments,
        }
    }

    pub fn send_session_list(&self, client_id: u64, sessions: &[RemoteSessionInfo]) {
        let payload = encode_session_list(sessions);
        self.send_cached_control_payload_bytes(
            client_id,
            MessageType::SessionList,
            &payload,
            |client| &mut client.last_session_list_payload,
        );
    }

    pub fn reply_session_list(&self, client_id: u64, sessions: &[RemoteSessionInfo]) {
        let payload = encode_session_list(sessions);
        self.update_client(client_id, |client| {
            client.last_session_list_payload = Some(payload.clone());
        });
        self.send_to_client(client_id, MessageType::SessionList, payload);
    }

    pub fn send_session_list_to_local_clients(&self, sessions: &[RemoteSessionInfo]) {
        let payload = encode_session_list(sessions);
        let client_ids = {
            let state_guard = self.state.lock().expect("remote server state poisoned");
            state_guard
                .clients
                .iter()
                .filter_map(|(client_id, client)| client.is_local.then_some(*client_id))
                .collect::<Vec<_>>()
        };
        for client_id in client_ids {
            self.send_cached_control_payload_bytes(
                client_id,
                MessageType::SessionList,
                &payload,
                |client| &mut client.last_session_list_payload,
            );
        }
    }

    pub fn send_attached(&self, client_id: u64, session_id: u32, attachment_id: Option<u64>) {
        let mut payload = session_id.to_le_bytes().to_vec();
        let mut attached_resume_token = None;
        if let Some(attachment_id) = attachment_id {
            payload.extend_from_slice(&attachment_id.to_le_bytes());
        }
        self.update_client(client_id, |client| {
            let same_session = client.attached_session == Some(session_id);
            client.attached_session = Some(session_id);
            client.attachment_id = attachment_id;
            client.resume_token = attachment_id.map(|_| {
                let token = client.resume_token.unwrap_or_else(random_u64_nonzero);
                attached_resume_token = Some(token);
                token
            });
            if !same_session {
                client.last_state = None;
                client.pane_states.clear();
                client.latest_input_seq = None;
            }
        });
        log::info!(
            "remote attach sent: client_id={client_id} session_id={session_id} attachment_id={attachment_id:?} resume_token_present={}",
            attached_resume_token.is_some()
        );
        if let Some(resume_token) = attached_resume_token {
            payload.extend_from_slice(&resume_token.to_le_bytes());
        }
        self.send_to_client(client_id, MessageType::Attached, payload);
    }

    pub fn prepare_attachment(
        &self,
        client_id: u64,
        session_id: u32,
        attachment_id: Option<u64>,
        resume_token: Option<u64>,
    ) -> Result<(), &'static str> {
        let mut state = self.state.lock().expect("remote server state poisoned");
        Self::prune_revivable_attachments(&mut state);
        let Some(client) = state.clients.get(&client_id) else {
            return Err("unknown client");
        };
        if client.is_local || attachment_id.is_none() {
            return Ok(());
        }
        let attachment_id = attachment_id.expect("checked above");
        if state.clients.iter().any(|(other_client_id, other_client)| {
            *other_client_id != client_id
                && !other_client.is_local
                && other_client.attachment_id == Some(attachment_id)
                && other_client.attached_session.is_some()
        }) {
            log::warn!(
                "remote revive rejected: client_id={client_id} attachment_id={attachment_id} reason=already-active"
            );
            return Err("attachment already active");
        }
        let revive = state.revivable_attachments.get(&attachment_id).cloned();
        if let Some(revive) = revive {
            if revive.session_id != session_id {
                log::warn!(
                    "remote revive rejected: client_id={client_id} attachment_id={attachment_id} reason=session-mismatch expected={} actual={session_id}",
                    revive.session_id
                );
                return Err("attachment belongs to different session");
            }
            if resume_token != Some(revive.resume_token) {
                log::warn!(
                    "remote revive rejected: client_id={client_id} attachment_id={attachment_id} reason=resume-token-mismatch"
                );
                return Err("attachment resume token mismatch");
            }
            let _ = state.revivable_attachments.remove(&attachment_id);
            let Some(client) = state.clients.get_mut(&client_id) else {
                return Err("unknown client");
            };
            client.attached_session = Some(session_id);
            client.attachment_id = Some(attachment_id);
            client.resume_token = Some(revive.resume_token);
            client.last_state = revive.last_state;
            client.pane_states = revive.pane_states;
            client.latest_input_seq = revive.latest_input_seq;
            log::info!(
                "remote revive restored: client_id={client_id} session_id={session_id} attachment_id={attachment_id}"
            );
        } else {
            if resume_token.is_some() {
                log::warn!(
                    "remote revive rejected: client_id={client_id} attachment_id={attachment_id} reason=revive-window-expired"
                );
                return Err("attachment resume window expired");
            }
            let Some(client) = state.clients.get_mut(&client_id) else {
                return Err("unknown client");
            };
            client.resume_token = None;
            log::info!(
                "remote attach prepared without revive: client_id={client_id} session_id={session_id} attachment_id={attachment_id}"
            );
        }
        Ok(())
    }

    pub fn send_detached(&self, client_id: u64) {
        self.update_client(client_id, |client| {
            client.attached_session = None;
            client.attachment_id = None;
            client.resume_token = None;
            client.last_state = None;
            client.pane_states.clear();
            client.latest_input_seq = None;
        });
        log::info!("remote detached: client_id={client_id}");
        self.send_to_client(client_id, MessageType::Detached, Vec::new());
    }

    pub fn send_session_created(&self, client_id: u64, session_id: u32) {
        self.send_to_client(
            client_id,
            MessageType::SessionCreated,
            session_id.to_le_bytes().to_vec(),
        );
    }

    pub fn send_error(&self, client_id: u64, message: &str) {
        self.send_to_client(
            client_id,
            MessageType::ErrorMsg,
            message.as_bytes().to_vec(),
        );
    }

    pub fn send_ui_runtime_state(
        &self,
        client_id: u64,
        state: &crate::control::UiRuntimeState,
    ) {
        let is_local = {
            let state_guard = self.state.lock().expect("remote server state poisoned");
            state_guard
                .clients
                .get(&client_id)
                .is_some_and(|client| client.is_local)
        };
        if !is_local {
            return;
        }
        let Ok(payload) = serde_json::to_vec(state) else {
            return;
        };
        self.send_cached_control_payload_bytes(
            client_id,
            MessageType::UiRuntimeState,
            &payload,
            |client| &mut client.last_ui_runtime_state_payload,
        );
    }

    pub fn send_ui_runtime_state_to_local_attached(
        &self,
        session_id: u32,
        state: &crate::control::UiRuntimeState,
    ) {
        let Ok(payload) = serde_json::to_vec(state) else {
            return;
        };
        let client_ids = {
            let state_guard = self.state.lock().expect("remote server state poisoned");
            state_guard
                .clients
                .iter()
                .filter_map(|(client_id, client)| {
                    (client.is_local && client.attached_session == Some(session_id))
                        .then_some(*client_id)
                })
                .collect::<Vec<_>>()
        };
        for client_id in client_ids {
            self.send_cached_control_payload_bytes(
                client_id,
                MessageType::UiRuntimeState,
                &payload,
                |client| &mut client.last_ui_runtime_state_payload,
            );
        }
    }

    pub fn retarget_local_attached_to_session(&self, session_id: u32) -> bool {
        let client_ids = {
            let state_guard = self.state.lock().expect("remote server state poisoned");
            state_guard
                .clients
                .iter()
                .filter_map(|(client_id, client)| {
                    (client.is_local
                        && client.attached_session.is_some()
                        && client.attached_session != Some(session_id))
                        .then_some(*client_id)
                })
                .collect::<Vec<_>>()
        };
        if client_ids.is_empty() {
            return false;
        }
        for client_id in client_ids {
            self.send_attached(client_id, session_id, None);
        }
        true
    }

    pub fn send_ui_appearance(
        &self,
        client_id: u64,
        appearance: &crate::control::UiAppearanceSnapshot,
    ) {
        let is_local = {
            let state_guard = self.state.lock().expect("remote server state poisoned");
            state_guard
                .clients
                .get(&client_id)
                .is_some_and(|client| client.is_local)
        };
        if !is_local {
            return;
        }
        let Ok(payload) = serde_json::to_vec(appearance) else {
            return;
        };
        self.send_cached_control_payload_bytes(
            client_id,
            MessageType::UiAppearance,
            &payload,
            |client| &mut client.last_ui_appearance_payload,
        );
    }

    pub fn send_ui_appearance_to_local_clients(
        &self,
        appearance: &crate::control::UiAppearanceSnapshot,
    ) {
        let Ok(payload) = serde_json::to_vec(appearance) else {
            return;
        };
        let client_ids = {
            let state_guard = self.state.lock().expect("remote server state poisoned");
            state_guard
                .clients
                .iter()
                .filter_map(|(client_id, client)| client.is_local.then_some(*client_id))
                .collect::<Vec<_>>()
        };
        for client_id in client_ids {
            self.send_cached_control_payload_bytes(
                client_id,
                MessageType::UiAppearance,
                &payload,
                |client| &mut client.last_ui_appearance_payload,
            );
        }
    }

    pub fn send_full_state_to_attached(&self, session_id: u32, state: Arc<RemoteFullState>) {
        let client_ids = self.clients_for_session(session_id);
        for client_id in client_ids {
            self.send_state_to_client(client_id, session_id, Arc::clone(&state));
        }
    }

    pub fn send_pane_state_to_local_attached(
        &self,
        session_id: u32,
        pane_id: u64,
        state: Arc<RemoteFullState>,
    ) {
        let client_ids = {
            let state_guard = self.state.lock().expect("remote server state poisoned");
            state_guard
                .clients
                .iter()
                .filter_map(|(client_id, client)| {
                    (client.is_local && client.attached_session == Some(session_id))
                        .then_some(*client_id)
                })
                .collect::<Vec<_>>()
        };
        for client_id in client_ids {
            self.send_pane_state_to_client(client_id, session_id, pane_id, Arc::clone(&state));
        }
    }

    pub fn retain_local_attached_pane_states(
        &self,
        session_id: u32,
        visible_pane_ids: &[u64],
    ) {
        let visible = visible_pane_ids
            .iter()
            .copied()
            .collect::<std::collections::HashSet<_>>();
        let mut guard = self.state.lock().expect("remote server state poisoned");
        for client in guard.clients.values_mut() {
            if client.is_local && client.attached_session == Some(session_id) {
                client.pane_states.retain(|pane_id, _| visible.contains(pane_id));
            }
        }
    }

    pub fn send_session_exited(&self, session_id: u32) {
        let client_ids = self.clients_for_session(session_id);
        for client_id in client_ids {
            self.send_to_client(
                client_id,
                MessageType::SessionExited,
                session_id.to_le_bytes().to_vec(),
            );
            self.update_client(client_id, |client| {
                client.attached_session = None;
                client.last_state = None;
                client.pane_states.clear();
                client.latest_input_seq = None;
            });
        }
    }

    pub fn record_input_seq(&self, client_id: u64, input_seq: Option<u64>) {
        self.update_client(client_id, |client| {
            if let Some(input_seq) = input_seq {
                client.latest_input_seq = Some(input_seq);
            }
        });
    }

    fn clients_for_session(&self, session_id: u32) -> Vec<u64> {
        let state = self.state.lock().expect("remote server state poisoned");
        state
            .clients
            .iter()
            .filter_map(|(client_id, client)| {
                (client.attached_session == Some(session_id)).then_some(*client_id)
            })
            .collect()
    }

    fn update_client(&self, client_id: u64, mut update: impl FnMut(&mut ClientState)) {
        let mut state = self.state.lock().expect("remote server state poisoned");
        if let Some(client) = state.clients.get_mut(&client_id) {
            update(client);
        }
    }

    fn send_to_client(&self, client_id: u64, ty: MessageType, payload: Vec<u8>) {
        let frame = encode_message(ty, &payload);
        let state = self.state.lock().expect("remote server state poisoned");
        if let Some(client) = state.clients.get(&client_id) {
            let _ = client.outbound.send(OutboundMessage::Frame(frame));
        }
    }

    fn send_cached_control_payload_bytes(
        &self,
        client_id: u64,
        ty: MessageType,
        payload: &[u8],
        cache_slot: impl FnOnce(&mut ClientState) -> &mut Option<Vec<u8>>,
    ) {
        let outbound = {
            let mut state = self.state.lock().expect("remote server state poisoned");
            let Some(client) = state.clients.get_mut(&client_id) else {
                return;
            };
            let cached_payload = cache_slot(client);
            if cached_payload.as_deref() == Some(payload) {
                return;
            }
            *cached_payload = Some(payload.to_vec());
            client.outbound.clone()
        };
        let frame = encode_message(ty, payload);
        let _ = outbound.send(OutboundMessage::Frame(frame));
    }

    fn send_state_to_client(&self, client_id: u64, session_id: u32, state: Arc<RemoteFullState>) {
        let _scope =
            crate::profiling::scope("server.stream.encode_state", crate::profiling::Kind::Cpu);
        let (outbound, previous_state, latest_input_seq, is_local) = {
            let guard = self.state.lock().expect("remote server state poisoned");
            let Some(client) = guard.clients.get(&client_id) else {
                return;
            };
            if client.attached_session != Some(session_id) {
                return;
            }
            (
                client.outbound.clone(),
                client.last_state.clone(),
                client.latest_input_seq,
                client.is_local,
            )
        };
        let (ty, payload) = match previous_state
            .as_ref()
            .and_then(|previous| {
                encode_delta(
                    previous.as_ref(),
                    state.as_ref(),
                    latest_input_seq,
                    is_local,
                )
            }) {
            Some(delta) => (MessageType::Delta, delta),
            None => (
                MessageType::FullState,
                encode_full_state(state.as_ref(), latest_input_seq, is_local),
            ),
        };
        let should_send = {
            let mut guard = self.state.lock().expect("remote server state poisoned");
            let Some(client) = guard.clients.get_mut(&client_id) else {
                return;
            };
            if client.attached_session != Some(session_id) {
                false
            } else {
                client.last_state = Some(Arc::clone(&state));
                true
            }
        };
        if !should_send {
            return;
        }
        crate::profiling::record_units(
            match (ty, is_local) {
                (MessageType::Delta, true) => "server.stream.publish_delta.local",
                (MessageType::Delta, false) => "server.stream.publish_delta.remote",
                (MessageType::FullState, true) => "server.stream.publish_full.local",
                (MessageType::FullState, false) => "server.stream.publish_full.remote",
                _ => "server.stream.publish_other",
            },
            crate::profiling::Kind::Cpu,
            1,
        );
        let frame = encode_message(ty, &payload);
        let _ = outbound.send(OutboundMessage::ScreenUpdate(frame));
    }

    fn send_pane_state_to_client(
        &self,
        client_id: u64,
        session_id: u32,
        pane_id: u64,
        state: Arc<RemoteFullState>,
    ) {
        let (outbound, previous_state) = {
            let guard = self.state.lock().expect("remote server state poisoned");
            let Some(client) = guard.clients.get(&client_id) else {
                return;
            };
            if client.attached_session != Some(session_id) {
                return;
            }
            (client.outbound.clone(), client.pane_states.get(&pane_id).cloned())
        };
        let (ty, payload) = match previous_state
            .as_ref()
            .and_then(|previous| encode_delta(previous.as_ref(), state.as_ref(), None, true))
        {
            Some(delta) => (MessageType::UiPaneDelta, delta),
            None => (
                MessageType::UiPaneFullState,
                encode_full_state(state.as_ref(), None, true),
            ),
        };
        let should_send = {
            let mut guard = self.state.lock().expect("remote server state poisoned");
            let Some(client) = guard.clients.get_mut(&client_id) else {
                return;
            };
            if client.attached_session != Some(session_id) {
                false
            } else {
                client.pane_states.insert(pane_id, Arc::clone(&state));
                true
            }
        };
        if !should_send {
            return;
        }
        let mut prefixed = Vec::with_capacity(8 + payload.len());
        prefixed.extend_from_slice(&pane_id.to_le_bytes());
        prefixed.extend_from_slice(&payload);
        let frame = encode_message(ty, &prefixed);
        let _ = outbound.send(OutboundMessage::ScreenUpdate(frame));
    }
}

impl Drop for RemoteServer {
    fn drop(&mut self) {
        if let Some(path) = self.local_socket_path.as_ref() {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn writer_loop<W: Write>(
    mut stream: W,
    outbound_rx: mpsc::Receiver<OutboundMessage>,
    coalesce_screen_updates: bool,
    batch_messages: bool,
) {
    while let Ok(message) = outbound_rx.recv() {
        let mut scope =
            crate::profiling::scope("server.stream.batch_write", crate::profiling::Kind::Io);
        let batch = if batch_messages {
            collect_outbound_batch(message, &outbound_rx, coalesce_screen_updates)
        } else {
            collect_single_outbound_message(message)
        };
        crate::profiling::record_units(
            "server.stream.batch_write.frames",
            crate::profiling::Kind::Io,
            batch.frames.len() as u64,
        );
        crate::profiling::record_units(
            "server.stream.batch_write.messages",
            crate::profiling::Kind::Io,
            batch.message_count as u64,
        );
        crate::profiling::record_units(
            "server.stream.batch_write.coalesced_screen_updates",
            crate::profiling::Kind::Io,
            batch.coalesced_screen_updates as u64,
        );
        crate::profiling::record_units(
            "server.stream.batch_write.coalesced_control_frames",
            crate::profiling::Kind::Io,
            batch.coalesced_control_frames as u64,
        );
        let mut failed = false;
        for frame in batch.frames {
            scope.add_bytes(frame.len() as u64);
            if stream.write_all(&frame).is_err() {
                failed = true;
                break;
            }
        }
        if failed || stream.flush().is_err() {
            break;
        }
    }
}

fn collect_single_outbound_message(message: OutboundMessage) -> OutboundBatch {
    let (frames, coalesced_screen_updates, coalesced_control_frames) = match message {
        OutboundMessage::Frame(frame) => (vec![frame], 0, 0),
        OutboundMessage::ScreenUpdate(frame) => (vec![frame], 0, 0),
    };
    OutboundBatch {
        frames,
        message_count: 1,
        coalesced_screen_updates,
        coalesced_control_frames,
    }
}

struct OutboundBatch {
    frames: Vec<Vec<u8>>,
    message_count: usize,
    coalesced_screen_updates: usize,
    coalesced_control_frames: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CoalescibleFrameKind {
    SessionList,
    UiRuntimeState,
    UiAppearance,
}

#[derive(Default)]
struct PendingOutboundFrames {
    order: Vec<CoalescibleFrameKind>,
    session_list: Option<Vec<u8>>,
    ui_runtime_state: Option<Vec<u8>>,
    ui_appearance: Option<Vec<u8>>,
}

impl PendingOutboundFrames {
    fn push_kind_once(&mut self, kind: CoalescibleFrameKind) {
        if !self.order.contains(&kind) {
            self.order.push(kind);
        }
    }

    fn set(&mut self, kind: CoalescibleFrameKind, frame: Vec<u8>) {
        self.push_kind_once(kind);
        match kind {
            CoalescibleFrameKind::SessionList => self.session_list = Some(frame),
            CoalescibleFrameKind::UiRuntimeState => self.ui_runtime_state = Some(frame),
            CoalescibleFrameKind::UiAppearance => self.ui_appearance = Some(frame),
        }
    }

    fn take_all(&mut self) -> Vec<Vec<u8>> {
        let mut frames = Vec::with_capacity(self.order.len());
        for kind in self.order.drain(..) {
            let frame = match kind {
                CoalescibleFrameKind::SessionList => self.session_list.take(),
                CoalescibleFrameKind::UiRuntimeState => self.ui_runtime_state.take(),
                CoalescibleFrameKind::UiAppearance => self.ui_appearance.take(),
            };
            if let Some(frame) = frame {
                frames.push(frame);
            }
        }
        frames
    }
}

fn collect_outbound_batch(
    first: OutboundMessage,
    outbound_rx: &mpsc::Receiver<OutboundMessage>,
    coalesce_screen_updates: bool,
) -> OutboundBatch {
    let mut frames = Vec::new();
    let mut pending_screen = None;
    let mut pending_control = PendingOutboundFrames::default();
    let mut message_count = 0usize;
    let mut screen_updates = 0usize;
    let mut emitted_screen_frames = 0usize;
    let mut coalesced_control_frames = 0usize;

    let mut push = |message| match message {
        OutboundMessage::Frame(frame) => {
            message_count += 1;
            if let Some(kind) = coalescible_frame_kind(&frame) {
                let replaced = match kind {
                    CoalescibleFrameKind::SessionList => pending_control.session_list.is_some(),
                    CoalescibleFrameKind::UiRuntimeState => {
                        pending_control.ui_runtime_state.is_some()
                    }
                    CoalescibleFrameKind::UiAppearance => pending_control.ui_appearance.is_some(),
                };
                if replaced {
                    coalesced_control_frames += 1;
                }
                pending_control.set(kind, frame);
                return;
            }
            for pending in pending_control.take_all() {
                frames.push(pending);
            }
            if let Some(screen) = pending_screen.take() {
                frames.push(screen);
                emitted_screen_frames += 1;
            }
            frames.push(frame);
        }
        OutboundMessage::ScreenUpdate(frame) => {
            message_count += 1;
            screen_updates += 1;
            if coalesce_screen_updates {
                pending_screen = Some(frame);
            } else {
                for pending in pending_control.take_all() {
                    frames.push(pending);
                }
                if let Some(screen) = pending_screen.take() {
                    frames.push(screen);
                    emitted_screen_frames += 1;
                }
                frames.push(frame);
                emitted_screen_frames += 1;
            }
        }
    };

    push(first);
    while let Ok(message) = outbound_rx.try_recv() {
        push(message);
    }
    for pending in pending_control.take_all() {
        frames.push(pending);
    }
    if let Some(screen) = pending_screen {
        frames.push(screen);
        emitted_screen_frames += 1;
    }
    OutboundBatch {
        frames,
        message_count,
        coalesced_screen_updates: screen_updates.saturating_sub(emitted_screen_frames),
        coalesced_control_frames,
    }
}

fn coalescible_frame_kind(frame: &[u8]) -> Option<CoalescibleFrameKind> {
    let ty = frame.get(2).copied().and_then(|value| MessageType::try_from(value).ok())?;
    match ty {
        MessageType::SessionList => Some(CoalescibleFrameKind::SessionList),
        MessageType::UiRuntimeState => Some(CoalescibleFrameKind::UiRuntimeState),
        MessageType::UiAppearance => Some(CoalescibleFrameKind::UiAppearance),
        _ => None,
    }
}

fn should_disconnect_idle_client(state: &Arc<Mutex<State>>, client_id: u64) -> Option<&'static str> {
    let state = state.lock().expect("remote server state poisoned");
    let client = state.clients.get(&client_id)?;
    let now = Instant::now();
    if client.authenticated {
        if client.is_local {
            return None;
        }
        let last_liveness = client.last_heartbeat_at.or(client.authenticated_at)?;
        if now.saturating_duration_since(last_liveness) > DIRECT_CLIENT_HEARTBEAT_WINDOW {
            return Some("heartbeat-timeout");
        }
        return None;
    }
    // Absolute deadline from connect time. This runs even when a challenge is
    // outstanding so a client cannot keep the socket pinned by sending empty `Auth`
    // frames that refresh `challenge.expires_at` indefinitely.
    if now.saturating_duration_since(client.connected_at) > AUTH_CHALLENGE_WINDOW {
        return Some("auth-timeout");
    }
    if let Some(challenge) = client.challenge {
        if now > challenge.expires_at {
            return Some("challenge-timeout");
        }
    }
    None
}

fn read_loop(
    mut stream: impl Read,
    client_id: u64,
    state: Arc<Mutex<State>>,
    cmd_tx: mpsc::Sender<RemoteCmd>,
) {
    loop {
        let mut scope =
            crate::profiling::scope("server.stream.read_message", crate::profiling::Kind::Io);
        let (ty, payload) = match read_message(&mut stream) {
            Ok(message) => message,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                ) =>
            {
                if let Some(reason) = should_disconnect_idle_client(&state, client_id) {
                    if reason == "heartbeat-timeout" {
                        log::warn!(
                            "remote direct client disconnected: client_id={client_id} reason={reason}"
                        );
                        send_direct_error(&state, client_id, "heartbeat timeout");
                    } else {
                        log::warn!("remote auth failed: client_id={client_id} reason={reason}");
                        send_direct_frame(&state, client_id, MessageType::AuthFail, Vec::new());
                    }
                    break;
                }
                continue;
            }
            Err(_) => break,
        };
        scope.add_bytes(payload.len() as u64);
        let (authenticated, is_local) = {
            let state = state.lock().expect("remote server state poisoned");
            state
                .clients
                .get(&client_id)
                .map(|client| (client.authenticated, client.is_local))
                .unwrap_or((false, false))
        };

        if matches!(ty, MessageType::Auth) {
            match handle_auth_message(client_id, &payload, &state) {
                AuthHandling::Authenticated => {
                    let _ = cmd_tx.send(RemoteCmd::Connected { client_id });
                    crate::notify_headless_wakeup();
                }
                AuthHandling::Pending => {}
                AuthHandling::Disconnect => break,
            }
            continue;
        }

        if !authenticated {
            send_direct_error(&state, client_id, "authentication required");
            continue;
        }

        if matches!(ty, MessageType::Heartbeat) {
            let mut state_guard = state.lock().expect("remote server state poisoned");
            if let Some(client) = state_guard.clients.get_mut(&client_id) {
                client.last_heartbeat_at = Some(Instant::now());
            }
            drop(state_guard);
            send_direct_frame(&state, client_id, MessageType::HeartbeatAck, payload);
            continue;
        }

        let command = match ty {
            MessageType::ListSessions => Some(RemoteCmd::ListSessions { client_id }),
            MessageType::Attach => {
                parse_attach_request(&payload).map(|(session_id, attachment_id, resume_token)| RemoteCmd::Attach {
                    client_id,
                    session_id,
                    attachment_id,
                    resume_token,
                })
            }
            MessageType::Detach => Some(RemoteCmd::Detach { client_id }),
            MessageType::Create => parse_resize(&payload).map(|(cols, rows)| RemoteCmd::Create {
                client_id,
                cols,
                rows,
            }),
            MessageType::Input => {
                parse_input_payload(&payload, is_local).map(|(input_seq, bytes)| RemoteCmd::Input {
                    client_id,
                    bytes,
                    input_seq,
                })
            }
            MessageType::Key => {
                parse_key_payload(&payload, is_local).and_then(|(input_seq, payload)| {
                    String::from_utf8(payload)
                        .ok()
                        .map(|keyspec| RemoteCmd::Key {
                            client_id,
                            keyspec,
                            input_seq,
                        })
                })
            }
            MessageType::Resize => parse_resize(&payload).map(|(cols, rows)| RemoteCmd::Resize {
                client_id,
                cols,
                rows,
            }),
            MessageType::ExecuteCommand => String::from_utf8(payload)
                .ok()
                .map(|input| RemoteCmd::ExecuteCommand { client_id, input }),
            MessageType::AppKeyEvent => serde_json::from_slice::<crate::AppKeyEvent>(&payload)
                .ok()
                .map(|event| RemoteCmd::AppKeyEvent { client_id, event }),
            MessageType::AppMouseEvent => serde_json::from_slice::<crate::AppMouseEvent>(&payload)
                .ok()
                .map(|event| RemoteCmd::AppMouseEvent { client_id, event }),
            MessageType::AppAction => serde_json::from_slice::<crate::bindings::Action>(&payload)
                .ok()
                .map(|action| RemoteCmd::AppAction { client_id, action }),
            MessageType::FocusPane => {
                parse_pane_id(&payload).map(|pane_id| RemoteCmd::FocusPane { client_id, pane_id })
            }
            MessageType::Destroy => Some(RemoteCmd::Destroy {
                client_id,
                session_id: parse_session_id(&payload),
            }),
            _ => None,
        };

        if let Some(command) = command {
            if cmd_tx.send(command).is_err() {
                break;
            }
            crate::notify_headless_wakeup();
        } else {
            send_direct_error(&state, client_id, "invalid payload");
        }
    }

    let mut state = state.lock().expect("remote server state poisoned");
    if let Some(client) = state.clients.remove(&client_id) {
        RemoteServer::prune_revivable_attachments(&mut state);
        if !client.is_local
            && let (Some(session_id), Some(attachment_id), Some(resume_token)) =
                (client.attached_session, client.attachment_id, client.resume_token)
        {
            state.revivable_attachments.insert(
                attachment_id,
                RevivableAttachment {
                    session_id,
                    resume_token,
                    last_state: client.last_state,
                    pane_states: client.pane_states,
                    latest_input_seq: client.latest_input_seq,
                    expires_at: Instant::now() + REVIVABLE_ATTACHMENT_WINDOW,
                },
            );
            log::info!(
                "remote client disconnected with revivable attachment: client_id={client_id} session_id={session_id} attachment_id={attachment_id}"
            );
        } else {
            log::info!("remote client disconnected: client_id={client_id}");
        }
    }
}

fn handle_auth_message(
    client_id: u64,
    payload: &[u8],
    state: &Arc<Mutex<State>>,
) -> AuthHandling {
    let mut state = state.lock().expect("remote server state poisoned");
    let auth_key = state.auth_key.clone();
    let server_identity_id = state.server_identity_id.clone();
    let server_instance_id = state.server_instance_id.clone();
    let Some(client) = state.clients.get_mut(&client_id) else {
        return AuthHandling::Disconnect;
    };

    if auth_key.is_none() {
        client.authenticated = true;
        client.authenticated_at = Some(Instant::now());
        log::info!("remote auth bypassed: client_id={client_id} mode=authless");
        let _ = client.outbound.send(OutboundMessage::Frame(encode_message(
            MessageType::AuthOk,
            &encode_auth_ok_payload(&server_identity_id, &server_instance_id),
        )));
        return AuthHandling::Authenticated;
    }

    if payload.is_empty() {
        let challenge = random_challenge();
        client.challenge = Some(AuthChallengeState {
            bytes: challenge,
            expires_at: Instant::now() + AUTH_CHALLENGE_WINDOW,
        });
        let _ = client.outbound.send(OutboundMessage::Frame(encode_message(
            MessageType::AuthChallenge,
            &challenge,
        )));
        log::info!("remote auth challenge issued: client_id={client_id}");
        return AuthHandling::Pending;
    }

    let Some(challenge) = client.challenge.take() else {
        log::warn!("remote auth failed: client_id={client_id} reason=missing-challenge");
        let _ = client.outbound.send(OutboundMessage::Frame(encode_message(
            MessageType::AuthFail,
            &[],
        )));
        return AuthHandling::Disconnect;
    };
    if Instant::now() > challenge.expires_at {
        log::warn!("remote auth failed: client_id={client_id} reason=expired-challenge");
        let _ = client.outbound.send(OutboundMessage::Frame(encode_message(
            MessageType::AuthFail,
            &[],
        )));
        return AuthHandling::Disconnect;
    }
    let Some(key) = auth_key else {
        log::warn!("remote auth failed: client_id={client_id} reason=missing-auth-key");
        let _ = client.outbound.send(OutboundMessage::Frame(encode_message(
            MessageType::AuthFail,
            &[],
        )));
        return AuthHandling::Disconnect;
    };

    let mut mac = HmacSha256::new_from_slice(&key).expect("valid HMAC key");
    mac.update(&challenge.bytes);
    match mac.verify_slice(payload) {
        Ok(()) => {
            client.authenticated = true;
            client.authenticated_at = Some(Instant::now());
            log::info!("remote auth succeeded: client_id={client_id}");
            let _ = client.outbound.send(OutboundMessage::Frame(encode_message(
                MessageType::AuthOk,
                &encode_auth_ok_payload(&server_identity_id, &server_instance_id),
            )));
            AuthHandling::Authenticated
        }
        Err(_) => {
            log::warn!("remote auth failed: client_id={client_id} reason=invalid-hmac");
            let _ = client.outbound.send(OutboundMessage::Frame(encode_message(
                MessageType::AuthFail,
                &[],
            )));
            AuthHandling::Disconnect
        }
    }
}

fn encode_auth_ok_payload(server_identity_id: &str, server_instance_id: &str) -> Vec<u8> {
    let build_id = env!("CARGO_PKG_VERSION").as_bytes();
    let server_identity_id = server_identity_id.as_bytes();
    let server_instance_id = server_instance_id.as_bytes();
    let mut payload = Vec::with_capacity(
        12 + build_id.len() + server_identity_id.len() + server_instance_id.len(),
    );
    payload.extend_from_slice(&REMOTE_PROTOCOL_VERSION.to_le_bytes());
    payload.extend_from_slice(&REMOTE_CAPABILITIES.to_le_bytes());
    payload.extend_from_slice(&(build_id.len() as u16).to_le_bytes());
    payload.extend_from_slice(build_id);
    payload.extend_from_slice(&(server_instance_id.len() as u16).to_le_bytes());
    payload.extend_from_slice(server_instance_id);
    payload.extend_from_slice(&(server_identity_id.len() as u16).to_le_bytes());
    payload.extend_from_slice(server_identity_id);
    payload
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn decode_auth_ok_payload(
    payload: &[u8],
) -> Option<(u16, u32, Option<String>, Option<String>, Option<String>)> {
    if payload.is_empty() {
        return None;
    }
    if payload.len() < 6 {
        return None;
    }
    let version = u16::from_le_bytes([payload[0], payload[1]]);
    let capabilities = u32::from_le_bytes([payload[2], payload[3], payload[4], payload[5]]);
    if payload.len() < 8 {
        return Some((version, capabilities, None, None, None));
    }
    let build_len = u16::from_le_bytes([payload[6], payload[7]]) as usize;
    if payload.len() < 8 + build_len {
        return None;
    }
    let build_id = String::from_utf8(payload[8..8 + build_len].to_vec()).ok();
    if payload.len() < 10 + build_len {
        return Some((version, capabilities, build_id, None, None));
    }
    let instance_offset = 8 + build_len;
    let instance_len =
        u16::from_le_bytes([payload[instance_offset], payload[instance_offset + 1]]) as usize;
    if payload.len() < instance_offset + 2 + instance_len {
        return None;
    }
    let server_instance_id = String::from_utf8(
        payload[instance_offset + 2..instance_offset + 2 + instance_len].to_vec(),
    )
    .ok();
    let identity_offset = instance_offset + 2 + instance_len;
    if payload.len() < identity_offset + 2 {
        return Some((version, capabilities, build_id, server_instance_id, None));
    }
    let identity_len =
        u16::from_le_bytes([payload[identity_offset], payload[identity_offset + 1]]) as usize;
    if payload.len() < identity_offset + 2 + identity_len {
        return None;
    }
    let server_identity_id = String::from_utf8(
        payload[identity_offset + 2..identity_offset + 2 + identity_len].to_vec(),
    )
    .ok();
    Some((
        version,
        capabilities,
        build_id,
        server_instance_id,
        server_identity_id,
    ))
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn validate_auth_ok_payload(payload: &[u8], auth_required: bool) -> Result<(), String> {
    let Some((version, capabilities, build_id, server_instance_id, server_identity_id)) =
        decode_auth_ok_payload(payload)
    else {
        return Err("Remote handshake is malformed".to_string());
    };
    if version != REMOTE_PROTOCOL_VERSION {
        return Err(format!("Unsupported remote protocol version: {version}"));
    }
    if auth_required && (capabilities & REMOTE_CAPABILITY_HMAC_AUTH) == 0 {
        return Err("Remote server does not advertise HMAC authentication".to_string());
    }
    if (capabilities & REMOTE_CAPABILITY_HEARTBEAT) == 0 {
        return Err("Remote server does not advertise heartbeat support".to_string());
    }
    if (capabilities & REMOTE_CAPABILITY_ATTACHMENT_RESUME) == 0 {
        return Err("Remote server does not advertise attachment resume support".to_string());
    }
    if (capabilities & REMOTE_CAPABILITY_DAEMON_IDENTITY) == 0 {
        return Err("Remote server does not advertise daemon identity support".to_string());
    }
    if (capabilities & (REMOTE_CAPABILITY_TCP_DIRECT_TRANSPORT | REMOTE_CAPABILITY_QUIC_DIRECT_TRANSPORT))
        == 0
    {
        return Err("Remote server does not advertise a supported direct transport".to_string());
    }
    if build_id.as_deref().is_none_or(str::is_empty) {
        return Err("Remote handshake is missing server build metadata".to_string());
    }
    if server_instance_id.as_deref().is_none_or(str::is_empty) {
        return Err("Remote handshake is missing server instance metadata".to_string());
    }
    if server_identity_id.as_deref().is_none_or(str::is_empty) {
        return Err("Remote handshake is missing server identity metadata".to_string());
    }
    Ok(())
}

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
        DirectRemoteClient::connect(host, port, auth_key, expected_server_identity)?;
    probe_summary_from_session(&mut client, port)
}

/// SPKI-pinned TLS variant of `probe_remote_endpoint`. `expected_identity` is the
/// `daemon_identity` string the caller already trusts; the TLS handshake aborts if the
/// presented cert's SPKI hash does not match.
pub fn probe_remote_endpoint_tls(
    host: &str,
    port: u16,
    auth_key: Option<&str>,
    expected_identity: &str,
) -> Result<RemoteProbeSummary, String> {
    let mut client = DirectTransportSession::<TlsClientStream>::connect_tls(
        host,
        port,
        auth_key,
        expected_identity,
    )?;
    probe_summary_from_session(&mut client, port)
}

/// SPKI-pinned QUIC variant of `probe_remote_endpoint`. Same SPKI pin semantics as
/// `probe_remote_endpoint_tls` — QUIC inherits the exact rustls-based trust model and
/// `PinnedSpkiServerCertVerifier` from the TCP+TLS path — the only difference is the
/// wire transport.
pub fn probe_remote_endpoint_quic(
    host: &str,
    port: u16,
    auth_key: Option<&str>,
    expected_identity: &str,
) -> Result<RemoteProbeSummary, String> {
    let mut client = DirectTransportSession::<QuicClientStream>::connect_quic(
        host,
        port,
        auth_key,
        expected_identity,
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
            // QUIC always rides TLS, so a pin is not optional. Callers without a
            // pin must go through probe_selected_direct_transport_tls or drop
            // back to TcpDirect.
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

/// SPKI-pinned TLS variant of `probe_selected_direct_transport`. Intended for the
/// SSH-bootstrap → direct-TLS upgrade flow: the caller has already learned the server's
/// `daemon_identity` out-of-band (typically over the forwarded SSH control socket) and
/// wants the subsequent direct connection to be TLS with that pin as the trust anchor.
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
        DirectRemoteClient::connect(host, port, auth_key, expected_server_identity)?;
    list_summary_from_session(&mut client, port)
}

pub fn list_remote_daemon_sessions_tls(
    host: &str,
    port: u16,
    auth_key: Option<&str>,
    expected_identity: &str,
) -> Result<RemoteSessionListSummary, String> {
    let mut client = DirectTransportSession::<TlsClientStream>::connect_tls(
        host,
        port,
        auth_key,
        expected_identity,
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
        DirectRemoteClient::connect(host, port, auth_key, expected_server_identity)?;
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
    let mut client = DirectTransportSession::<TlsClientStream>::connect_tls(
        host,
        port,
        auth_key,
        expected_identity,
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
        DirectRemoteClient::connect(host, port, auth_key, expected_server_identity)?;
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
    let mut client = DirectTransportSession::<TlsClientStream>::connect_tls(
        host,
        port,
        auth_key,
        expected_identity,
    )?;
    create_summary_from_session(&mut client, port, cols, rows)
}

/// Default socket timeout for direct-client probes and RPCs. Generous enough to
/// tolerate the extra scheduling latency seen in constrained environments like the
/// Nix build sandbox on Darwin, where a 3-second timeout occasionally trips during
/// the TLS handshake + HMAC round trip.
const DIRECT_CLIENT_SOCKET_TIMEOUT: Duration = Duration::from_secs(10);

impl DirectTransportSession<std::net::TcpStream> {
    fn connect(
        host: &str,
        port: u16,
        auth_key: Option<&str>,
        expected_server_identity: Option<&str>,
    ) -> Result<Self, String> {
        use std::net::TcpStream;

        let stream = TcpStream::connect((host, port))
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

        Self::connect_over_stream(
            stream,
            host.to_string(),
            port,
            auth_key,
            expected_server_identity,
        )
    }
}

type TlsClientStream = rustls::StreamOwned<rustls::ClientConnection, std::net::TcpStream>;
type QuicClientStream = crate::remote_quic::QuicBridgeStream;

impl DirectTransportSession<QuicClientStream> {
    /// Connect to a remote daemon over QUIC with SPKI-pinned cert verification.
    /// Identical trust model to `connect_tls` — same rustls
    /// `PinnedSpkiServerCertVerifier` drives the handshake, just over quinn's
    /// QUIC transport instead of TCP+TLS.
    #[cfg_attr(not(test), allow(dead_code))]
    fn connect_quic(
        host: &str,
        port: u16,
        auth_key: Option<&str>,
        expected_identity: &str,
    ) -> Result<Self, String> {
        let client_config = build_remote_client_tls_config(expected_identity)?;
        let stream = crate::remote_quic::connect_quic_client(
            host,
            port,
            REMOTE_DAEMON_SERVER_NAME,
            client_config,
        )
        .map_err(|error| format!("quic connect to {host}:{port} failed: {error}"))?;

        Self::connect_over_stream(
            stream,
            host.to_string(),
            port,
            auth_key,
            Some(expected_identity),
        )
    }
}

impl DirectTransportSession<TlsClientStream> {
    /// Connect to a remote daemon with an SPKI-pinned TLS handshake. `expected_identity`
    /// is the `daemon_identity` string the caller already trusts; it is the
    /// `base64url(sha256(SPKI))` of the server's ed25519 cert and is checked inside the
    /// TLS handshake before any application data is exchanged.
    #[cfg_attr(not(test), allow(dead_code))]
    fn connect_tls(
        host: &str,
        port: u16,
        auth_key: Option<&str>,
        expected_identity: &str,
    ) -> Result<Self, String> {
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

        let client_config = build_remote_client_tls_config(expected_identity)?;
        let server_name = rustls::pki_types::ServerName::try_from(REMOTE_DAEMON_SERVER_NAME)
            .map_err(|error| format!("build remote server name: {error}"))?;
        let client_conn = rustls::ClientConnection::new(Arc::new(client_config), server_name)
            .map_err(|error| format!("build rustls client connection: {error}"))?;
        let mut tls_stream = rustls::StreamOwned::new(client_conn, tcp);
        tls_stream
            .conn
            .complete_io(&mut tls_stream.sock)
            .map_err(|error| format!("tls handshake to {host}:{port} failed: {error}"))?;

        Self::connect_over_stream(
            tls_stream,
            host.to_string(),
            port,
            auth_key,
            Some(expected_identity),
        )
    }
}

impl<S: DirectReadWrite> DirectTransportSession<S> {
    fn connect_over_stream(
        mut stream: S,
        host: String,
        port: u16,
        auth_key: Option<&str>,
        expected_server_identity: Option<&str>,
    ) -> Result<Self, String> {
        stream
            .write_all(&encode_message(MessageType::Auth, &[]))
            .map_err(|error| format!("failed to send auth request to {host}:{port}: {error}"))?;
        let (ty, auth_payload) = read_probe_auth_reply(&mut stream, &host, port)?;
        let (auth_required, auth_ok_payload) = match ty {
            MessageType::AuthOk => (false, auth_payload),
            MessageType::AuthChallenge => {
                let key = auth_key
                    .ok_or_else(|| format!("remote endpoint {host}:{port} requires --auth-key"))?;
                let mut mac = HmacSha256::new_from_slice(key.as_bytes()).expect("valid HMAC key");
                mac.update(&auth_payload);
                let response = mac.finalize().into_bytes().to_vec();
                stream.write_all(&encode_message(MessageType::Auth, &response)).map_err(
                    |error| format!("failed to send auth response to {host}:{port}: {error}"),
                )?;
                let (reply_ty, reply_payload) = read_message(&mut stream).map_err(|error| {
                    format!("failed to read authenticated reply from {host}:{port}: {error}")
                })?;
                if reply_ty != MessageType::AuthOk {
                    return Err(format!(
                        "expected auth ok from {host}:{port}, got {reply_ty:?}"
                    ));
                }
                (true, reply_payload)
            }
            MessageType::AuthFail => {
                return Err(format!("authentication failed for remote endpoint {host}:{port}"));
            }
            other => {
                return Err(format!(
                    "unexpected auth reply from {host}:{port}: {other:?}"
                ));
            }
        };

        validate_auth_ok_payload(&auth_ok_payload, auth_required)?;
        let (protocol_version, capabilities, build_id, server_instance_id, server_identity_id) =
            decode_auth_ok_payload(&auth_ok_payload).ok_or_else(|| {
                format!("remote endpoint {host}:{port} returned malformed handshake metadata")
            })?;
        if let Some(expected_server_identity) = expected_server_identity {
            if server_identity_id.as_deref() != Some(expected_server_identity) {
                return Err(format!(
                    "remote endpoint {host}:{port} reported daemon identity {:?}, expected {:?}",
                    server_identity_id, expected_server_identity
                ));
            }
        }

        Ok(Self {
            stream,
            host,
            port,
            auth_required,
            protocol_version,
            capabilities,
            build_id,
            server_instance_id,
            server_identity_id,
        })
    }

    fn heartbeat_round_trip(&mut self, payload: &[u8]) -> Result<u64, String> {
        let heartbeat_start = Instant::now();
        self.stream
            .write_all(&encode_message(MessageType::Heartbeat, payload))
            .map_err(|error| {
                format!(
                    "failed to send heartbeat to {}:{}: {error}",
                    self.host, self.port
                )
            })?;
        let (_heartbeat_ty, heartbeat_reply) =
            read_probe_reply(&mut self.stream, &self.host, self.port, MessageType::HeartbeatAck)?;
        if heartbeat_reply != payload {
            return Err(format!(
                "heartbeat payload mismatch from remote endpoint {}:{}",
                self.host, self.port
            ));
        }
        Ok(heartbeat_start.elapsed().as_millis() as u64)
    }

    fn list_sessions(&mut self) -> Result<Vec<RemoteDirectSessionInfo>, String> {
        self.stream
            .write_all(&encode_message(MessageType::ListSessions, &[]))
            .map_err(|error| {
                format!(
                    "failed to send list sessions request to {}:{}: {error}",
                    self.host, self.port
                )
            })?;
        let (_reply_ty, payload) =
            read_probe_reply(&mut self.stream, &self.host, self.port, MessageType::SessionList)?;
        decode_session_list_payload(&payload).map_err(|error| {
            format!(
                "failed to decode remote session list from {}:{}: {error}",
                self.host, self.port
            )
        })
    }

    fn attach(
        &mut self,
        session_id: u32,
        attachment_id: Option<u64>,
        resume_token: Option<u64>,
    ) -> Result<(RemoteAttachedSummary, RemoteFullState), String> {
        let mut attach_payload = session_id.to_le_bytes().to_vec();
        if let Some(attachment_id) = attachment_id {
            attach_payload.extend_from_slice(&attachment_id.to_le_bytes());
        }
        if let Some(resume_token) = resume_token {
            if attachment_id.is_none() {
                return Err("resume token requires attachment id".to_string());
            }
            attach_payload.extend_from_slice(&resume_token.to_le_bytes());
        }
        self.stream
            .write_all(&encode_message(MessageType::Attach, &attach_payload))
            .map_err(|error| {
                format!(
                    "failed to send attach request to {}:{}: {error}",
                    self.host, self.port
                )
            })?;

        read_attach_bootstrap(&mut self.stream, &self.host, self.port)
    }

    fn create_session(&mut self, cols: u16, rows: u16) -> Result<u32, String> {
        let mut payload = Vec::with_capacity(4);
        payload.extend_from_slice(&cols.to_le_bytes());
        payload.extend_from_slice(&rows.to_le_bytes());
        self.stream
            .write_all(&encode_message(MessageType::Create, &payload))
            .map_err(|error| {
                format!(
                    "failed to send create request to {}:{}: {error}",
                    self.host, self.port
                )
            })?;
        let (_reply_ty, payload) =
            read_probe_reply(&mut self.stream, &self.host, self.port, MessageType::SessionCreated)?;
        parse_session_id(&payload).ok_or_else(|| {
            format!(
                "invalid session-created payload from remote endpoint {}:{}",
                self.host, self.port
            )
        })
    }
}

fn read_probe_reply(
    stream: &mut impl Read,
    host: &str,
    port: u16,
    expected: MessageType,
) -> Result<(MessageType, Vec<u8>), String> {
    for _ in 0..8 {
        let (ty, payload) = read_message(stream)
            .map_err(|error| format!("failed to read probe reply from {host}:{port}: {error}"))?;
        if ty == expected {
            return Ok((ty, payload));
        }
        match ty {
            MessageType::SessionList
            | MessageType::FullState
            | MessageType::Delta
            | MessageType::Attached
            | MessageType::Detached
            | MessageType::UiRuntimeState
            | MessageType::UiAppearance
            | MessageType::UiPaneFullState
            | MessageType::UiPaneDelta => continue,
            MessageType::AuthFail => {
                return Err(format!("authentication failed for remote endpoint {host}:{port}"));
            }
            MessageType::ErrorMsg => {
                let message = String::from_utf8_lossy(&payload);
                return Err(format!(
                    "remote endpoint {host}:{port} reported probe error: {message}"
                ));
            }
            other => {
                return Err(format!(
                    "expected {expected:?} from {host}:{port}, got {other:?}"
                ));
            }
        }
    }
    Err(format!(
        "timed out waiting for {expected:?} from remote endpoint {host}:{port}"
    ))
}

fn read_attach_bootstrap(
    stream: &mut impl Read,
    host: &str,
    port: u16,
) -> Result<(RemoteAttachedSummary, RemoteFullState), String> {
    let mut attached = None;
    let mut full_state = None;
    for _ in 0..12 {
        let (ty, payload) = read_message(stream)
            .map_err(|error| format!("failed to read attach reply from {host}:{port}: {error}"))?;
        match ty {
            MessageType::SessionList
            | MessageType::UiRuntimeState
            | MessageType::UiAppearance
            | MessageType::UiPaneFullState
            | MessageType::UiPaneDelta => continue,
            MessageType::Attached => {
                attached = Some(
                    decode_attached_payload(&payload)
                        .map_err(|error| format!("invalid attached payload from {host}:{port}: {error}"))?,
                );
            }
            MessageType::FullState => {
                full_state = Some(
                    decode_remote_full_state_payload(&payload)
                        .map_err(|error| format!("invalid full state payload from {host}:{port}: {error}"))?,
                );
            }
            MessageType::Delta => continue,
            MessageType::AuthFail => {
                return Err(format!("authentication failed for remote endpoint {host}:{port}"));
            }
            MessageType::ErrorMsg => {
                let message = String::from_utf8_lossy(&payload);
                return Err(format!(
                    "remote endpoint {host}:{port} reported attach error: {message}"
                ));
            }
            MessageType::Detached => {
                return Err(format!(
                    "remote endpoint {host}:{port} detached immediately after attach"
                ));
            }
            other => {
                return Err(format!(
                    "unexpected attach reply from {host}:{port}: {other:?}"
                ));
            }
        }
        if let (Some(attached), Some(full_state)) = (attached.clone(), full_state.clone()) {
            return Ok((attached, full_state));
        }
    }
    Err(format!(
        "timed out waiting for attach bootstrap from remote endpoint {host}:{port}"
    ))
}

fn read_probe_auth_reply(
    stream: &mut impl Read,
    host: &str,
    port: u16,
) -> Result<(MessageType, Vec<u8>), String> {
    for _ in 0..8 {
        let (ty, payload) = read_message(stream)
            .map_err(|error| format!("failed to read auth reply from {host}:{port}: {error}"))?;
        match ty {
            MessageType::AuthOk | MessageType::AuthChallenge | MessageType::AuthFail => {
                return Ok((ty, payload));
            }
            MessageType::SessionList
            | MessageType::FullState
            | MessageType::Delta
            | MessageType::Attached
            | MessageType::Detached
            | MessageType::UiRuntimeState
            | MessageType::UiAppearance
            | MessageType::UiPaneFullState
            | MessageType::UiPaneDelta => continue,
            other => {
                return Err(format!(
                    "unexpected auth reply from {host}:{port}: {other:?}"
                ));
            }
        }
    }
    Err(format!(
        "timed out waiting for auth reply from remote endpoint {host}:{port}"
    ))
}

fn decode_session_list_payload(payload: &[u8]) -> Result<Vec<RemoteDirectSessionInfo>, String> {
    if payload.len() < 4 {
        return Err("payload too short".to_string());
    }
    let mut offset = 0usize;
    let count = u32::from_le_bytes(
        payload[offset..offset + 4]
            .try_into()
            .map_err(|_| "invalid session count".to_string())?,
    ) as usize;
    offset += 4;

    fn read_u32(payload: &[u8], offset: &mut usize) -> Result<u32, String> {
        if payload.len() < *offset + 4 {
            return Err("payload truncated".to_string());
        }
        let value = u32::from_le_bytes(
            payload[*offset..*offset + 4]
                .try_into()
                .map_err(|_| "invalid u32".to_string())?,
        );
        *offset += 4;
        Ok(value)
    }

    fn read_u16(payload: &[u8], offset: &mut usize) -> Result<u16, String> {
        if payload.len() < *offset + 2 {
            return Err("payload truncated".to_string());
        }
        let value = u16::from_le_bytes(
            payload[*offset..*offset + 2]
                .try_into()
                .map_err(|_| "invalid u16".to_string())?,
        );
        *offset += 2;
        Ok(value)
    }

    fn read_string(payload: &[u8], offset: &mut usize) -> Result<String, String> {
        let len = read_u16(payload, offset)? as usize;
        if payload.len() < *offset + len {
            return Err("payload truncated".to_string());
        }
        let value = std::str::from_utf8(&payload[*offset..*offset + len])
            .map_err(|_| "invalid utf-8".to_string())?
            .to_string();
        *offset += len;
        Ok(value)
    }

    let mut sessions = Vec::with_capacity(count);
    for _ in 0..count {
        let id = read_u32(payload, &mut offset)?;
        let name = read_string(payload, &mut offset)?;
        let title = read_string(payload, &mut offset)?;
        let pwd = read_string(payload, &mut offset)?;
        let flags = *payload
            .get(offset)
            .ok_or_else(|| "payload truncated".to_string())?;
        offset += 1;
        sessions.push(RemoteDirectSessionInfo {
            id,
            name,
            title,
            pwd,
            attached: (flags & 0x01) != 0,
            child_exited: (flags & 0x02) != 0,
        });
    }
    if offset != payload.len() {
        return Err("payload has trailing bytes".to_string());
    }
    Ok(sessions)
}

fn decode_attached_payload(payload: &[u8]) -> Result<RemoteAttachedSummary, String> {
    if payload.len() < 4 {
        return Err("payload too short".to_string());
    }
    let session_id = u32::from_le_bytes(
        payload[..4]
            .try_into()
            .map_err(|_| "invalid session id".to_string())?,
    );
    let attachment_id = if payload.len() >= 12 {
        Some(u64::from_le_bytes(
            payload[4..12]
                .try_into()
                .map_err(|_| "invalid attachment id".to_string())?,
        ))
    } else {
        None
    };
    let resume_token = if payload.len() >= 20 {
        Some(u64::from_le_bytes(
            payload[12..20]
                .try_into()
                .map_err(|_| "invalid resume token".to_string())?,
        ))
    } else {
        None
    };
    if payload.len() != 4 && payload.len() != 12 && payload.len() != 20 {
        return Err("payload has unexpected length".to_string());
    }
    Ok(RemoteAttachedSummary {
        session_id,
        attachment_id,
        resume_token,
    })
}

fn decode_remote_full_state_payload(payload: &[u8]) -> Result<RemoteFullState, String> {
    if payload.len() < REMOTE_FULL_STATE_HEADER_LEN {
        return Err("payload too short".to_string());
    }
    let rows = u16::from_le_bytes(
        payload[0..2]
            .try_into()
            .map_err(|_| "invalid rows".to_string())?,
    );
    let cols = u16::from_le_bytes(
        payload[2..4]
            .try_into()
            .map_err(|_| "invalid cols".to_string())?,
    );
    let cursor_x = u16::from_le_bytes(
        payload[4..6]
            .try_into()
            .map_err(|_| "invalid cursor_x".to_string())?,
    );
    let cursor_y = u16::from_le_bytes(
        payload[6..8]
            .try_into()
            .map_err(|_| "invalid cursor_y".to_string())?,
    );
    let cursor_visible = payload[8] != 0;
    let cursor_blinking = payload[9] != 0;
    let cursor_style = i32::from_le_bytes(
        payload[10..14]
            .try_into()
            .map_err(|_| "invalid cursor_style".to_string())?,
    );
    let cell_count = rows as usize * cols as usize;
    let expected_len = REMOTE_FULL_STATE_HEADER_LEN + cell_count * REMOTE_CELL_ENCODED_LEN;
    if payload.len() != expected_len {
        return Err("payload length does not match grid size".to_string());
    }
    let mut cells = Vec::with_capacity(cell_count);
    let mut offset = REMOTE_FULL_STATE_HEADER_LEN;
    for _ in 0..cell_count {
        let codepoint = u32::from_le_bytes(
            payload[offset..offset + 4]
                .try_into()
                .map_err(|_| "invalid codepoint".to_string())?,
        );
        let fg = [payload[offset + 4], payload[offset + 5], payload[offset + 6]];
        let bg = [payload[offset + 7], payload[offset + 8], payload[offset + 9]];
        let style_flags = payload[offset + 10];
        let wide = payload[offset + 11] != 0;
        cells.push(RemoteCell {
            codepoint,
            fg,
            bg,
            style_flags,
            wide,
        });
        offset += REMOTE_CELL_ENCODED_LEN;
    }
    Ok(RemoteFullState {
        rows,
        cols,
        cursor_x,
        cursor_y,
        cursor_visible,
        cursor_blinking,
        cursor_style,
        cells,
    })
}

fn send_direct_error(state: &Arc<Mutex<State>>, client_id: u64, message: &str) {
    send_direct_frame(
        state,
        client_id,
        MessageType::ErrorMsg,
        message.as_bytes().to_vec(),
    );
}

fn send_direct_frame(
    state: &Arc<Mutex<State>>,
    client_id: u64,
    ty: MessageType,
    payload: Vec<u8>,
) {
    let state = state.lock().expect("remote server state poisoned");
    if let Some(client) = state.clients.get(&client_id) {
        let _ = client
            .outbound
            .send(OutboundMessage::Frame(encode_message(ty, &payload)));
    }
}

pub(crate) fn read_message(stream: &mut impl Read) -> io::Result<(MessageType, Vec<u8>)> {
    let mut header = [0u8; HEADER_LEN];
    stream.read_exact(&mut header)?;
    if header[..2] != MAGIC {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid remote magic",
        ));
    }
    let ty = MessageType::try_from(header[2])
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "unknown remote message"))?;
    let payload_len = u32::from_le_bytes([header[3], header[4], header[5], header[6]]) as usize;
    let mut payload = vec![0u8; payload_len];
    if payload_len > 0 {
        stream.read_exact(&mut payload)?;
    }
    Ok((ty, payload))
}

fn parse_session_id(payload: &[u8]) -> Option<u32> {
    (payload.len() >= 4)
        .then(|| u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]))
}

fn parse_attach_request(payload: &[u8]) -> Option<(u32, Option<u64>, Option<u64>)> {
    let session_id = parse_session_id(payload)?;
    let attachment_id = (payload.len() >= 12).then(|| {
        u64::from_le_bytes([
            payload[4], payload[5], payload[6], payload[7], payload[8], payload[9], payload[10],
            payload[11],
        ])
    });
    let resume_token = (payload.len() >= 20).then(|| {
        u64::from_le_bytes([
            payload[12],
            payload[13],
            payload[14],
            payload[15],
            payload[16],
            payload[17],
            payload[18],
            payload[19],
        ])
    });
    Some((session_id, attachment_id, resume_token))
}

fn parse_pane_id(payload: &[u8]) -> Option<u64> {
    (payload.len() >= 8).then(|| {
        u64::from_le_bytes([
            payload[0], payload[1], payload[2], payload[3], payload[4], payload[5], payload[6],
            payload[7],
        ])
    })
}

fn parse_resize(payload: &[u8]) -> Option<(u16, u16)> {
    (payload.len() >= 4).then(|| {
        (
            u16::from_le_bytes([payload[0], payload[1]]),
            u16::from_le_bytes([payload[2], payload[3]]),
        )
    })
}

fn parse_input_payload(payload: &[u8], is_local: bool) -> Option<(Option<u64>, Vec<u8>)> {
    if is_local {
        if payload.len() < 8 {
            return None;
        }
        let input_seq = u64::from_le_bytes(payload[..8].try_into().ok()?);
        return Some((Some(input_seq), payload[8..].to_vec()));
    }
    Some((None, payload.to_vec()))
}

fn parse_key_payload(payload: &[u8], is_local: bool) -> Option<(Option<u64>, Vec<u8>)> {
    parse_input_payload(payload, is_local)
}

pub(crate) fn encode_message(ty: MessageType, payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(HEADER_LEN + payload.len());
    frame.extend_from_slice(&MAGIC);
    frame.push(ty as u8);
    frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    frame.extend_from_slice(payload);
    frame
}

pub fn encode_session_list(sessions: &[RemoteSessionInfo]) -> Vec<u8> {
    let mut payload = Vec::new();
    payload.extend_from_slice(&(sessions.len() as u32).to_le_bytes());
    for session in sessions {
        payload.extend_from_slice(&session.id.to_le_bytes());
        push_string(&mut payload, &session.name);
        push_string(&mut payload, &session.title);
        push_string(&mut payload, &session.pwd);
        let mut flags = 0u8;
        if session.attached {
            flags |= 0x01;
        }
        if session.child_exited {
            flags |= 0x02;
        }
        payload.push(flags);
    }
    payload
}

pub fn encode_full_state(
    state: &RemoteFullState,
    latest_input_seq: Option<u64>,
    local: bool,
) -> Vec<u8> {
    let prefix_len = if local { LOCAL_INPUT_SEQ_LEN } else { 0 };
    let mut payload = Vec::with_capacity(
        prefix_len + REMOTE_FULL_STATE_HEADER_LEN + state.cells.len() * REMOTE_CELL_ENCODED_LEN,
    );
    if local {
        payload.extend_from_slice(&latest_input_seq.unwrap_or(0).to_le_bytes());
    }
    payload.extend_from_slice(&state.rows.to_le_bytes());
    payload.extend_from_slice(&state.cols.to_le_bytes());
    payload.extend_from_slice(&state.cursor_x.to_le_bytes());
    payload.extend_from_slice(&state.cursor_y.to_le_bytes());
    payload.push(u8::from(state.cursor_visible));
    payload.push(u8::from(state.cursor_blinking));
    payload.extend_from_slice(&state.cursor_style.to_le_bytes());
    for cell in &state.cells {
        payload.extend_from_slice(&cell.codepoint.to_le_bytes());
        payload.extend_from_slice(&cell.fg);
        payload.extend_from_slice(&cell.bg);
        payload.push(cell.style_flags);
        payload.push(u8::from(cell.wide));
    }
    crate::profiling::record_bytes_and_units(
        if local {
            "server.stream.encode_full_state.local"
        } else {
            "server.stream.encode_full_state.remote"
        },
        crate::profiling::Kind::Cpu,
        Duration::ZERO,
        payload.len() as u64,
        state.cells.len() as u64,
    );
    payload
}

fn encode_delta(
    previous: &RemoteFullState,
    current: &RemoteFullState,
    latest_input_seq: Option<u64>,
    local: bool,
) -> Option<Vec<u8>> {
    if previous.rows != current.rows || previous.cols != current.cols {
        return None;
    }
    if previous == current {
        let prefix_len = if local { LOCAL_INPUT_SEQ_LEN } else { 0 };
        let mut payload = Vec::with_capacity(prefix_len + REMOTE_DELTA_HEADER_LEN);
        if local {
            payload.extend_from_slice(&latest_input_seq.unwrap_or(0).to_le_bytes());
        }
        payload.extend_from_slice(&0u16.to_le_bytes());
        payload.extend_from_slice(&current.cursor_x.to_le_bytes());
        payload.extend_from_slice(&current.cursor_y.to_le_bytes());
        payload.push(u8::from(current.cursor_visible));
        payload.push(u8::from(current.cursor_blinking));
        payload.push(0);
        payload.extend_from_slice(&current.cursor_style.to_le_bytes());
        return Some(payload);
    }

    let cols = current.cols as usize;
    let rows = current.rows as usize;
    let mut changed_rows = Vec::new();
    for row in 0..rows {
        let start = row * cols;
        let end = start + cols;
        if previous.cells[start..end] != current.cells[start..end] {
            changed_rows.push((
                row as u16,
                changed_segment(&previous.cells[start..end], &current.cells[start..end]),
            ));
        }
    }

    if changed_rows.is_empty() {
        let prefix_len = if local { LOCAL_INPUT_SEQ_LEN } else { 0 };
        let mut payload = Vec::with_capacity(prefix_len + REMOTE_DELTA_HEADER_LEN);
        if local {
            payload.extend_from_slice(&latest_input_seq.unwrap_or(0).to_le_bytes());
        }
        payload.extend_from_slice(&0u16.to_le_bytes());
        payload.extend_from_slice(&current.cursor_x.to_le_bytes());
        payload.extend_from_slice(&current.cursor_y.to_le_bytes());
        payload.push(u8::from(current.cursor_visible));
        payload.push(u8::from(current.cursor_blinking));
        payload.push(0);
        payload.extend_from_slice(&current.cursor_style.to_le_bytes());
        return Some(payload);
    }

    let scroll_rows = if local {
        None
    } else {
        detect_scroll_rows(previous, current)
    };
    if changed_rows.len() == rows
        && scroll_rows.is_none()
        && changed_rows
            .iter()
            .all(|(_, (start_col, cells))| *start_col == 0 && cells.len() == cols)
    {
        return None;
    }

    let mut payload = Vec::new();
    if local {
        payload.extend_from_slice(&latest_input_seq.unwrap_or(0).to_le_bytes());
    }
    let rows_to_encode = if let Some(scroll_rows) = scroll_rows {
        rows_changed_after_scroll(current.rows as usize, scroll_rows)
            .into_iter()
            .map(|row| {
                let start = row as usize * cols;
                let end = start + cols;
                (
                    row,
                    changed_segment(&previous.cells[start..end], &current.cells[start..end]),
                )
            })
            .collect::<Vec<_>>()
    } else {
        changed_rows
    };
    let encoded_rows = rows_to_encode.len() as u64;
    let encoded_cells = rows_to_encode
        .iter()
        .map(|(_, (_, cells))| cells.len() as u64)
        .sum::<u64>();
    payload.extend_from_slice(&(rows_to_encode.len() as u16).to_le_bytes());
    payload.extend_from_slice(&current.cursor_x.to_le_bytes());
    payload.extend_from_slice(&current.cursor_y.to_le_bytes());
    payload.push(u8::from(current.cursor_visible));
    payload.push(u8::from(current.cursor_blinking));
    let mut flags = 0u8;
    if scroll_rows.is_some() {
        flags |= 0x01;
    }
    payload.push(flags);
    payload.extend_from_slice(&current.cursor_style.to_le_bytes());
    if let Some(scroll_rows) = scroll_rows {
        payload.extend_from_slice(&scroll_rows.to_le_bytes());
    }
    for (row, (start_col, cells)) in rows_to_encode {
        payload.extend_from_slice(&row.to_le_bytes());
        payload.extend_from_slice(&(start_col as u16).to_le_bytes());
        payload.extend_from_slice(&(cells.len() as u16).to_le_bytes());
        for cell in &cells {
            payload.extend_from_slice(&cell.codepoint.to_le_bytes());
            payload.extend_from_slice(&cell.fg);
            payload.extend_from_slice(&cell.bg);
            payload.push(cell.style_flags);
            payload.push(u8::from(cell.wide));
        }
    }
    crate::profiling::record_bytes_and_units(
        if local {
            "server.stream.encode_delta.local"
        } else {
            "server.stream.encode_delta.remote"
        },
        crate::profiling::Kind::Cpu,
        Duration::ZERO,
        payload.len() as u64,
        encoded_cells,
    );
    crate::profiling::record_units(
        if local {
            "server.stream.encode_delta_rows.local"
        } else {
            "server.stream.encode_delta_rows.remote"
        },
        crate::profiling::Kind::Cpu,
        encoded_rows,
    );
    Some(payload)
}

fn changed_segment(previous: &[RemoteCell], current: &[RemoteCell]) -> (usize, Vec<RemoteCell>) {
    debug_assert_eq!(previous.len(), current.len());
    let first = previous
        .iter()
        .zip(current.iter())
        .position(|(a, b)| a != b);
    let Some(first) = first else {
        return (0, Vec::new());
    };
    let last = previous
        .iter()
        .zip(current.iter())
        .rposition(|(a, b)| a != b)
        .unwrap_or(first);
    (first, current[first..=last].to_vec())
}

fn detect_scroll_rows(previous: &RemoteFullState, current: &RemoteFullState) -> Option<i16> {
    if previous.rows != current.rows || previous.cols != current.cols || current.rows <= 1 {
        return None;
    }
    let rows = current.rows as usize;
    let cols = current.cols as usize;
    if previous.cells[cols..rows * cols] == current.cells[..(rows - 1) * cols] {
        return Some(1);
    }
    if previous.cells[..(rows - 1) * cols] == current.cells[cols..rows * cols] {
        return Some(-1);
    }
    let previous_rows = row_fingerprints(previous);
    let current_rows = row_fingerprints(current);

    let positive_overlap = longest_prefix_suffix_overlap(&current_rows, &previous_rows);
    if positive_overlap > 0 {
        let shift = rows - positive_overlap;
        if previous.cells[shift * cols..rows * cols] == current.cells[..positive_overlap * cols] {
            return Some(shift as i16);
        }
    }

    let negative_overlap = longest_prefix_suffix_overlap(&previous_rows, &current_rows);
    if negative_overlap > 0 {
        let shift = rows - negative_overlap;
        if previous.cells[..negative_overlap * cols] == current.cells[shift * cols..rows * cols] {
            return Some(-(shift as i16));
        }
    }
    None
}

fn longest_prefix_suffix_overlap(prefix: &[u64], suffix_source: &[u64]) -> usize {
    if prefix.is_empty() || suffix_source.is_empty() {
        return 0;
    }

    let mut sequence = Vec::with_capacity(prefix.len() + 1 + suffix_source.len());
    sequence.extend(prefix.iter().copied().map(Some));
    sequence.push(None);
    sequence.extend(suffix_source.iter().copied().map(Some));

    let mut prefix_function = vec![0usize; sequence.len()];
    for index in 1..sequence.len() {
        let mut matched = prefix_function[index - 1];
        while matched > 0 && sequence[index] != sequence[matched] {
            matched = prefix_function[matched - 1];
        }
        if sequence[index] == sequence[matched] {
            matched += 1;
        }
        prefix_function[index] = matched;
    }

    prefix_function.last().copied().unwrap_or(0).min(prefix.len())
}

fn row_fingerprints(state: &RemoteFullState) -> Vec<u64> {
    use std::hash::Hasher;

    let cols = state.cols as usize;
    state
        .cells
        .chunks(cols)
        .map(|row| {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            for cell in row {
                hasher.write_u32(cell.codepoint);
                hasher.write(&cell.fg);
                hasher.write(&cell.bg);
                hasher.write_u8(cell.style_flags);
                hasher.write_u8(u8::from(cell.wide));
            }
            hasher.finish()
        })
        .collect()
}

fn rows_changed_after_scroll(rows: usize, scroll_rows: i16) -> Vec<u16> {
    if scroll_rows > 0 {
        let shift = scroll_rows as usize;
        ((rows.saturating_sub(shift))..rows)
            .map(|row| row as u16)
            .collect()
    } else {
        let shift = (-scroll_rows) as usize;
        (0..shift.min(rows)).map(|row| row as u16).collect()
    }
}

fn push_string(payload: &mut Vec<u8>, text: &str) {
    let bytes = text.as_bytes();
    let len = bytes.len().min(u16::MAX as usize);
    payload.extend_from_slice(&(len as u16).to_le_bytes());
    payload.extend_from_slice(&bytes[..len]);
}

fn random_challenge() -> [u8; 32] {
    let mut challenge = [0u8; 32];
    if let Ok(mut file) = std::fs::File::open("/dev/urandom") {
        let _ = file.read_exact(&mut challenge);
        if challenge.iter().any(|byte| *byte != 0) {
            return challenge;
        }
    }
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    for (idx, byte) in challenge.iter_mut().enumerate() {
        *byte = (seed.wrapping_shr((idx % 8) as u32) as u8) ^ (idx as u8).wrapping_mul(17);
    }
    challenge
}

fn random_instance_id() -> String {
    let challenge = random_challenge();
    let mut output = String::with_capacity(16);
    for byte in &challenge[..8] {
        use std::fmt::Write as _;
        let _ = write!(&mut output, "{byte:02x}");
    }
    output
}

fn random_u64_nonzero() -> u64 {
    let challenge = random_challenge();
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&challenge[..8]);
    let value = u64::from_le_bytes(bytes);
    if value == 0 { 1 } else { value }
}

fn elapsed_ms(now: Instant, earlier: Instant) -> u64 {
    now.saturating_duration_since(earlier).as_millis() as u64
}

fn remaining_ms(now: Instant, deadline: Instant) -> u64 {
    deadline.saturating_duration_since(now).as_millis() as u64
}

#[derive(Clone)]
pub struct DaemonIdentityMaterial {
    pub identity_id: String,
    pub key_pem: String,
    pub cert_pem: String,
}

fn daemon_identity_dir() -> PathBuf {
    crate::config::config_dir().join("remote-daemon-identity")
}

fn load_or_create_daemon_identity() -> String {
    load_or_create_daemon_identity_material().identity_id
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn load_or_create_daemon_identity_material() -> DaemonIdentityMaterial {
    load_or_create_daemon_identity_material_at(&daemon_identity_dir())
}

fn load_or_create_daemon_identity_material_at(dir: &Path) -> DaemonIdentityMaterial {
    let key_path = dir.join("key.pem");
    let cert_path = dir.join("cert.pem");

    if let Some(material) = try_load_validated_identity(&key_path, &cert_path) {
        return material;
    }

    let material = generate_daemon_identity_material();
    if let Err(error) = persist_daemon_identity(dir, &material) {
        log::warn!(
            "failed to persist remote daemon identity at {}: {error}",
            dir.display()
        );
    }
    material
}

/// Load a previously-persisted daemon identity pair and verify that the cert's
/// `SubjectPublicKeyInfo` hash matches the key's. A mismatch means the two files drifted
/// (partial write, manual edit, disk corruption) and the stored pair cannot be trusted as
/// a stable pin anchor, so the caller regenerates instead of silently rotating the
/// identity.
fn try_load_validated_identity(
    key_path: &Path,
    cert_path: &Path,
) -> Option<DaemonIdentityMaterial> {
    let key_pem = std::fs::read_to_string(key_path).ok()?;
    let cert_pem = std::fs::read_to_string(cert_path).ok()?;
    let keypair = rcgen::KeyPair::from_pem(&key_pem).ok()?;
    let key_spki: Vec<u8> = keypair.public_key_der();

    let cert_ders: Vec<_> = rustls_pemfile::certs(&mut cert_pem.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    let cert_der = cert_ders.first()?;
    let cert_spki = extract_cert_spki_der(cert_der.as_ref()).ok()?;
    if cert_spki != key_spki {
        log::warn!(
            "remote daemon cert/key SPKI mismatch at {}: regenerating identity pair",
            key_path.parent().unwrap_or(key_path).display()
        );
        return None;
    }

    let identity_id = derive_identity_id(key_spki);
    Some(DaemonIdentityMaterial {
        identity_id,
        key_pem,
        cert_pem,
    })
}

/// Persist a daemon identity pair via temp-file + rename. Errors surface to the caller
/// instead of being silently dropped: the daemon may continue with the in-memory
/// material for the current process, but the next restart can then retry persistence
/// rather than masking disk-full or permission issues.
fn persist_daemon_identity(dir: &Path, material: &DaemonIdentityMaterial) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let key_path = dir.join("key.pem");
    let cert_path = dir.join("cert.pem");
    let key_tmp = dir.join("key.pem.tmp");
    let cert_tmp = dir.join("cert.pem.tmp");

    write_and_fsync(&key_tmp, material.key_pem.as_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&key_tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    write_and_fsync(&cert_tmp, material.cert_pem.as_bytes())?;
    // Rename order is not load-atomic across the two files, but the SPKI validation
    // in try_load_validated_identity catches any mismatch after a partial rename and
    // forces regeneration — preventing silent identity rotation or startup failure.
    std::fs::rename(&cert_tmp, &cert_path)?;
    std::fs::rename(&key_tmp, &key_path)?;
    Ok(())
}

fn write_and_fsync(path: &Path, data: &[u8]) -> io::Result<()> {
    let mut file = std::fs::File::create(path)?;
    file.write_all(data)?;
    file.sync_all()?;
    Ok(())
}

/// Bind a QUIC endpoint on `bind_addr` using the same rustls server config the
/// TCP+TLS path uses, and spawn an async accept loop on the shared QUIC
/// runtime. Returns the `QuicListener` so the caller can keep its lifetime
/// tied to the surrounding `RemoteServer`. `None` on bind failure — QUIC is
/// additive to TCP; if it cannot come up, the daemon still serves TCP.
fn spawn_quic_accept_loop(
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

async fn handle_quic_incoming(
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
    let (outbound_tx, outbound_rx) =
        tokio::sync::mpsc::channel::<Vec<u8>>(REMOTE_QUIC_OUTBOUND_CAPACITY);

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

fn serve_incoming_tcp_client(
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

fn run_remote_client_session<R, W>(
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
    let (client_id, outbound_rx, authenticated) = {
        let mut state = state.lock().expect("remote server state poisoned");
        let client_id = NEXT_CLIENT_ID.fetch_add(1, Ordering::Relaxed);
        let (outbound_tx, outbound_rx) = mpsc::channel();
        let authenticated = state.auth_key.is_none();
        state.clients.insert(
            client_id,
            ClientState {
                outbound: outbound_tx,
                authenticated,
                challenge: None,
                connected_at: Instant::now(),
                authenticated_at: authenticated.then(Instant::now),
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
        (client_id, outbound_rx, authenticated)
    };
    log::info!(
        "remote tcp client connected: client_id={client_id} authenticated={authenticated} transport={transport_label}"
    );

    std::thread::spawn(move || writer_loop(writer, outbound_rx, true, true));

    if authenticated {
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

const TRANSPORT_LABEL_TLS: &str = "tls";
const TRANSPORT_LABEL_PLAIN: &str = "plain";
const TRANSPORT_LABEL_QUIC: &str = "quic";

/// Size of the outbound bounded channel between the sync writer and the async
/// QUIC send pump. Mirrors the constant in `remote_quic.rs`; declared at the
/// call site here so the capacity choice is visible alongside the usage.
const REMOTE_QUIC_OUTBOUND_CAPACITY: usize = 32;

/// DNS-like name presented by the server's self-signed cert. Used as the `ServerName`
/// input to rustls on the client side. The pinning verifier ignores CA chain and hostname
/// anyway, but rustls still requires a syntactically valid name to feed SNI.
pub(crate) const REMOTE_DAEMON_SERVER_NAME: &str = "boo-remote-daemon";

fn extract_cert_spki_der(cert_der: &[u8]) -> Result<Vec<u8>, String> {
    use x509_parser::prelude::*;
    let (_, cert) = X509Certificate::from_der(cert_der)
        .map_err(|error| format!("parse remote daemon cert: {error}"))?;
    Ok(cert.tbs_certificate.subject_pki.raw.to_vec())
}

fn cert_der_matches_identity(cert_der: &[u8], expected_identity: &str) -> bool {
    match extract_cert_spki_der(cert_der) {
        Ok(spki) => derive_identity_id(spki) == expected_identity,
        Err(_) => false,
    }
}

#[derive(Debug)]
struct PinnedSpkiServerCertVerifier {
    expected_identity: String,
    provider: Arc<rustls::crypto::CryptoProvider>,
}

impl PinnedSpkiServerCertVerifier {
    fn new(expected_identity: String) -> Self {
        Self {
            expected_identity,
            provider: Arc::new(rustls::crypto::ring::default_provider()),
        }
    }
}

impl rustls::client::danger::ServerCertVerifier for PinnedSpkiServerCertVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        if cert_der_matches_identity(end_entity.as_ref(), &self.expected_identity) {
            Ok(rustls::client::danger::ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::ApplicationVerificationFailure,
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

pub(crate) fn build_remote_client_tls_config(
    expected_identity: &str,
) -> Result<rustls::ClientConfig, String> {
    let verifier = Arc::new(PinnedSpkiServerCertVerifier::new(
        expected_identity.to_string(),
    ));
    let provider = Arc::clone(&verifier.provider);
    Ok(rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|error| format!("negotiate TLS protocol versions: {error}"))?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth())
}

fn build_remote_server_tls_config(
    material: &DaemonIdentityMaterial,
) -> Result<Arc<rustls::ServerConfig>, String> {
    use rustls::pki_types::PrivateKeyDer;

    let cert_chain = rustls_pemfile::certs(&mut material.cert_pem.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("parse remote daemon cert: {error}"))?;
    if cert_chain.is_empty() {
        return Err("remote daemon cert.pem contained no certificates".to_string());
    }
    let key_der: PrivateKeyDer<'static> =
        rustls_pemfile::private_key(&mut material.key_pem.as_bytes())
            .map_err(|error| format!("parse remote daemon private key: {error}"))?
            .ok_or_else(|| "remote daemon key.pem contained no private key".to_string())?;

    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|error| format!("negotiate TLS protocol versions: {error}"))?
        .with_no_client_auth()
        .with_single_cert(cert_chain, key_der)
        .map_err(|error| format!("install remote daemon cert in rustls: {error}"))?;
    Ok(Arc::new(config))
}

/// `Read + Write + Send + Clone` handle around a single underlying stream. Used for the
/// TLS-wrapped path where the rustls session state cannot be duplicated via
/// `TcpStream::try_clone` the way the plain-TCP path does. Reader and writer threads each
/// hold a clone of the `Arc<Mutex<_>>` and serialize I/O; the lock hold time is bounded by
/// `TLS_INNER_READ_TIMEOUT` on the reader side so writers are not starved during idle
/// reads.
struct SharedStream {
    inner: Arc<Mutex<Box<dyn ReadWriteSend>>>,
}

trait ReadWriteSend: Read + Write + Send {}
impl<T: Read + Write + Send + ?Sized> ReadWriteSend for T {}

impl SharedStream {
    fn new<S: Read + Write + Send + 'static>(stream: S) -> Self {
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

fn generate_daemon_identity_material() -> DaemonIdentityMaterial {
    let keypair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519)
        .expect("generate ed25519 keypair for remote daemon identity");
    let mut params = rcgen::CertificateParams::new(vec!["boo-remote-daemon".to_string()])
        .expect("build remote daemon cert params");
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "boo remote daemon");
    let cert = params
        .self_signed(&keypair)
        .expect("self-sign remote daemon cert");
    let identity_id = derive_identity_id(keypair.public_key_der());
    DaemonIdentityMaterial {
        identity_id,
        key_pem: keypair.serialize_pem(),
        cert_pem: cert.pem(),
    }
}

fn derive_identity_id(spki_der: impl AsRef<[u8]>) -> String {
    use base64::Engine;
    use sha2::Digest;
    let digest = sha2::Sha256::digest(spki_der.as_ref());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

impl ServiceAdvertiser {
    fn spawn(service_name: &str, port: u16) -> Option<Self> {
        #[cfg(target_os = "macos")]
        let mut command = {
            let mut command = Command::new("dns-sd");
            command
                .args(["-R", service_name, "_boo._tcp", "local", &port.to_string()])
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            command
        };

        #[cfg(target_os = "linux")]
        let mut command = {
            let mut command = Command::new("avahi-publish-service");
            command
                .args([service_name, "_boo._tcp", &port.to_string()])
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            command
        };

        command.spawn().ok().map(|child| Self { child })
    }
}

pub fn full_state_from_ui(snapshot: &crate::control::UiTerminalSnapshot) -> RemoteFullState {
    let cells = snapshot
        .rows_data
        .iter()
        .flat_map(|row| row.cells.iter())
        .map(|cell| {
            let mut style_flags = 0u8;
            if cell.bold {
                style_flags |= STYLE_FLAG_BOLD;
            }
            if cell.italic {
                style_flags |= STYLE_FLAG_ITALIC;
            }
            if cell.hyperlink {
                style_flags |= STYLE_FLAG_HYPERLINK;
            }
            if cell.fg != [0, 0, 0] {
                style_flags |= STYLE_FLAG_EXPLICIT_FG;
            }
            if !cell.bg_is_default {
                style_flags |= STYLE_FLAG_EXPLICIT_BG;
            }
            RemoteCell {
                codepoint: cell.text.chars().next().map(u32::from).unwrap_or(0),
                fg: cell.fg,
                bg: cell.bg,
                style_flags,
                wide: cell.display_width > 1,
            }
        })
        .collect();
    RemoteFullState {
        rows: snapshot.rows,
        cols: snapshot.cols,
        cursor_x: snapshot.cursor.x,
        cursor_y: snapshot.cursor.y,
        cursor_visible: snapshot.cursor.visible,
        cursor_blinking: snapshot.cursor.blinking,
        cursor_style: snapshot.cursor.style,
        cells,
    }
}

pub fn full_state_from_terminal(
    snapshot: &crate::vt_backend_core::TerminalSnapshot,
) -> RemoteFullState {
    let cells = snapshot
        .rows_data
        .iter()
        .flat_map(|row| row.iter())
        .map(|cell| {
            let mut style_flags = 0u8;
            if cell.bold {
                style_flags |= STYLE_FLAG_BOLD;
            }
            if cell.italic {
                style_flags |= STYLE_FLAG_ITALIC;
            }
            if cell.hyperlink {
                style_flags |= STYLE_FLAG_HYPERLINK;
            }
            let has_explicit_fg = cell.fg.r != snapshot.colors.foreground.r
                || cell.fg.g != snapshot.colors.foreground.g
                || cell.fg.b != snapshot.colors.foreground.b;
            let has_explicit_bg = !cell.bg_is_default;
            if has_explicit_fg {
                style_flags |= STYLE_FLAG_EXPLICIT_FG;
            }
            if has_explicit_bg {
                style_flags |= STYLE_FLAG_EXPLICIT_BG;
            }
            RemoteCell {
                codepoint: cell.text.chars().next().map(u32::from).unwrap_or(0),
                fg: [cell.fg.r, cell.fg.g, cell.fg.b],
                bg: [cell.bg.r, cell.bg.g, cell.bg.b],
                style_flags,
                wide: cell.display_width > 1,
            }
        })
        .collect();
    RemoteFullState {
        rows: snapshot.rows,
        cols: snapshot.cols,
        cursor_x: snapshot.cursor.x,
        cursor_y: snapshot.cursor.y,
        cursor_visible: snapshot.cursor.visible,
        cursor_blinking: snapshot.cursor.blinking,
        cursor_style: snapshot.cursor.style,
        cells,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control;
    use std::collections::VecDeque;

    struct TimeoutScriptedReader {
        chunks: VecDeque<Result<Vec<u8>, io::ErrorKind>>,
    }

    impl TimeoutScriptedReader {
        fn new(chunks: impl IntoIterator<Item = Result<Vec<u8>, io::ErrorKind>>) -> Self {
            Self {
                chunks: chunks.into_iter().collect(),
            }
        }
    }

    impl Read for TimeoutScriptedReader {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            match self.chunks.pop_front() {
                Some(Ok(chunk)) => {
                    let len = chunk.len().min(buf.len());
                    buf[..len].copy_from_slice(&chunk[..len]);
                    if len < chunk.len() {
                        self.chunks.push_front(Ok(chunk[len..].to_vec()));
                    }
                    Ok(len)
                }
                Some(Err(kind)) => Err(io::Error::new(kind, "scripted timeout")),
                None => Ok(0),
            }
        }
    }

    #[test]
    fn remote_config_defaults_authless_tcp_to_loopback_without_advertising() {
        let config = RemoteConfig {
            port: 7337,
            bind_address: None,
            auth_key: None,
            allow_insecure_no_auth: false,
            service_name: "boo".to_string(),
        };
        assert_eq!(config.effective_bind_address(), "127.0.0.1");
        assert!(!config.should_advertise());
    }

    #[test]
    fn remote_config_defaults_authenticated_tcp_to_public_bind_with_advertising() {
        let config = RemoteConfig {
            port: 7337,
            bind_address: None,
            auth_key: Some("secret".to_string()),
            allow_insecure_no_auth: false,
            service_name: "boo".to_string(),
        };
        assert_eq!(config.effective_bind_address(), "0.0.0.0");
        assert!(config.should_advertise());
    }

    #[test]
    fn remote_config_explicit_bind_address_overrides_defaults() {
        let config = RemoteConfig {
            port: 7337,
            bind_address: Some("192.168.0.5".to_string()),
            auth_key: None,
            allow_insecure_no_auth: false,
            service_name: "boo".to_string(),
        };
        assert_eq!(config.effective_bind_address(), "192.168.0.5");
        assert!(config.should_advertise());
        assert!(config.rejects_public_authless_bind());
    }

    #[test]
    fn remote_config_allows_explicit_insecure_public_bind_when_acknowledged() {
        let config = RemoteConfig {
            port: 7337,
            bind_address: Some("192.168.0.5".to_string()),
            auth_key: None,
            allow_insecure_no_auth: true,
            service_name: "boo".to_string(),
        };
        assert_eq!(config.effective_bind_address(), "192.168.0.5");
        assert!(config.should_advertise());
        assert!(!config.rejects_public_authless_bind());
    }

    #[test]
    fn session_list_encoding_matches_client_layout() {
        let payload = encode_session_list(&[RemoteSessionInfo {
            id: 7,
            name: "Tab 1".to_string(),
            title: "shell".to_string(),
            pwd: "/tmp".to_string(),
            attached: true,
            child_exited: false,
        }]);
        assert_eq!(u32::from_le_bytes(payload[0..4].try_into().unwrap()), 1);
        assert_eq!(u32::from_le_bytes(payload[4..8].try_into().unwrap()), 7);
        assert_eq!(u16::from_le_bytes(payload[8..10].try_into().unwrap()), 5);
        assert_eq!(&payload[10..15], b"Tab 1");
        assert_eq!(*payload.last().unwrap(), 0x01);
    }

    #[test]
    fn full_state_encoding_uses_12_byte_cells() {
        let payload = encode_full_state(
            &RemoteFullState {
                rows: 1,
                cols: 2,
                cursor_x: 1,
                cursor_y: 0,
                cursor_visible: true,
                cursor_blinking: true,
                cursor_style: 5,
                cells: vec![
                    RemoteCell {
                        codepoint: u32::from('A'),
                        fg: [1, 2, 3],
                        bg: [4, 5, 6],
                        style_flags: 0x21,
                        wide: false,
                    },
                    RemoteCell {
                        codepoint: u32::from('好'),
                        fg: [7, 8, 9],
                        bg: [10, 11, 12],
                        style_flags: 0x42,
                        wide: true,
                    },
                ],
            },
            None,
            false,
        );
        assert_eq!(
            payload.len(),
            REMOTE_FULL_STATE_HEADER_LEN + 2 * REMOTE_CELL_ENCODED_LEN
        );
        assert_eq!(u16::from_le_bytes(payload[0..2].try_into().unwrap()), 1);
        assert_eq!(u16::from_le_bytes(payload[2..4].try_into().unwrap()), 2);
        assert_eq!(
            u32::from_le_bytes(
                payload[REMOTE_FULL_STATE_HEADER_LEN..REMOTE_FULL_STATE_HEADER_LEN + 4]
                    .try_into()
                    .unwrap()
            ),
            u32::from('A')
        );
        let second_offset = REMOTE_FULL_STATE_HEADER_LEN + REMOTE_CELL_ENCODED_LEN;
        assert_eq!(payload[REMOTE_FULL_STATE_HEADER_LEN + 10], 0x21);
        assert_eq!(payload[REMOTE_FULL_STATE_HEADER_LEN + 11], 0);
        assert_eq!(
            u32::from_le_bytes(
                payload[second_offset..second_offset + 4]
                    .try_into()
                    .unwrap()
            ),
            u32::from('好')
        );
        assert_eq!(payload[second_offset + 10], 0x42);
        assert_eq!(payload[second_offset + 11], 1);
    }

    #[test]
    fn local_full_state_encoding_prefixes_latest_input_seq() {
        let payload = encode_full_state(
            &RemoteFullState {
                rows: 1,
                cols: 1,
                cursor_x: 0,
                cursor_y: 0,
                cursor_visible: true,
                cursor_blinking: false,
                cursor_style: 1,
                cells: vec![RemoteCell {
                    codepoint: u32::from('A'),
                    fg: [1, 2, 3],
                    bg: [4, 5, 6],
                    style_flags: 0,
                    wide: false,
                }],
            },
            Some(42),
            true,
        );
        assert_eq!(u64::from_le_bytes(payload[0..8].try_into().unwrap()), 42);
        assert_eq!(u16::from_le_bytes(payload[8..10].try_into().unwrap()), 1);
        assert_eq!(u16::from_le_bytes(payload[10..12].try_into().unwrap()), 1);
    }

    #[test]
    fn full_state_from_ui_snapshot_flattens_rows() {
        let snapshot = control::UiTerminalSnapshot {
            cols: 2,
            rows: 1,
            title: String::new(),
            pwd: String::new(),
            cursor: control::UiCursorSnapshot {
                visible: true,
                blinking: false,
                x: 1,
                y: 0,
                style: 0,
            },
            rows_data: vec![control::UiTerminalRowSnapshot {
                cells: vec![
                    control::UiTerminalCellSnapshot {
                        text: "a".to_string(),
                        display_width: 1,
                        fg: [1, 1, 1],
                        bg: [0, 0, 0],
                        bg_is_default: true,
                        bold: false,
                        italic: false,
                        underline: 0,
                        hyperlink: false,
                    },
                    control::UiTerminalCellSnapshot {
                        text: "界".to_string(),
                        display_width: 2,
                        fg: [2, 2, 2],
                        bg: [3, 3, 3],
                        bg_is_default: false,
                        bold: true,
                        italic: true,
                        underline: 0,
                        hyperlink: false,
                    },
                ],
            }],
        };
        let state = full_state_from_ui(&snapshot);
        assert_eq!(state.cells.len(), 2);
        assert_eq!(state.cells[0].codepoint, u32::from('a'));
        assert!(!state.cells[0].wide);
        assert_eq!(state.cells[1].codepoint, u32::from('界'));
        assert!(state.cells[1].wide);
        assert_eq!(state.cells[1].style_flags & 0x03, 0x03);
    }

    #[test]
    fn full_state_from_terminal_snapshot_flattens_rows() {
        let snapshot = crate::vt_backend_core::TerminalSnapshot {
            cols: 2,
            rows: 1,
            cursor: crate::vt_backend_core::CursorSnapshot {
                visible: true,
                blinking: true,
                x: 1,
                y: 0,
                style: 0,
            },
            rows_data: vec![vec![
                crate::vt_backend_core::CellSnapshot {
                    text: "a".to_string(),
                    display_width: 1,
                    fg: crate::vt::GhosttyColorRgb { r: 1, g: 1, b: 1 },
                    bg: crate::vt::GhosttyColorRgb { r: 0, g: 0, b: 0 },
                    bg_is_default: true,
                    bold: false,
                    italic: false,
                    underline: 0,
                    hyperlink: false,
                },
                crate::vt_backend_core::CellSnapshot {
                    text: "界".to_string(),
                    display_width: 2,
                    fg: crate::vt::GhosttyColorRgb { r: 2, g: 2, b: 2 },
                    bg: crate::vt::GhosttyColorRgb { r: 3, g: 3, b: 3 },
                    bg_is_default: false,
                    bold: true,
                    italic: true,
                    underline: 0,
                    hyperlink: false,
                },
            ]],
            colors: crate::vt::GhosttyRenderStateColors {
                foreground: crate::vt::GhosttyColorRgb { r: 1, g: 1, b: 1 },
                background: crate::vt::GhosttyColorRgb { r: 0, g: 0, b: 0 },
                ..Default::default()
            },
            ..Default::default()
        };
        let state = full_state_from_terminal(&snapshot);
        assert_eq!(state.cells.len(), 2);
        assert_eq!(state.cells[0].codepoint, u32::from('a'));
        assert!(!state.cells[0].wide);
        assert_eq!(state.cells[1].codepoint, u32::from('界'));
        assert!(state.cells[1].wide);
        assert_eq!(state.cells[1].style_flags & 0x03, 0x03);
        assert_eq!(state.cells[1].style_flags & 0x60, 0x60);
    }

    #[test]
    fn outbound_batch_coalesces_consecutive_screen_updates() {
        let (tx, rx) = mpsc::channel();
        tx.send(OutboundMessage::ScreenUpdate(vec![1])).unwrap();
        tx.send(OutboundMessage::ScreenUpdate(vec![2])).unwrap();
        tx.send(OutboundMessage::Frame(vec![9])).unwrap();
        tx.send(OutboundMessage::ScreenUpdate(vec![3])).unwrap();
        let first = rx.recv().unwrap();
        let batch = collect_outbound_batch(first, &rx, true);
        assert_eq!(batch.frames, vec![vec![2], vec![9], vec![3]]);
        assert_eq!(batch.message_count, 4);
        assert_eq!(batch.coalesced_screen_updates, 1);
    }

    #[test]
    fn outbound_batch_keeps_all_screen_updates_when_coalescing_disabled() {
        let (tx, rx) = mpsc::channel();
        tx.send(OutboundMessage::ScreenUpdate(vec![1])).unwrap();
        tx.send(OutboundMessage::ScreenUpdate(vec![2])).unwrap();
        tx.send(OutboundMessage::Frame(vec![9])).unwrap();
        tx.send(OutboundMessage::ScreenUpdate(vec![3])).unwrap();
        let first = rx.recv().unwrap();
        let batch = collect_outbound_batch(first, &rx, false);
        assert_eq!(batch.frames, vec![vec![1], vec![2], vec![9], vec![3]]);
        assert_eq!(batch.message_count, 4);
        assert_eq!(batch.coalesced_screen_updates, 0);
    }

    #[test]
    fn outbound_batch_coalesces_superseded_control_frames() {
        let (tx, rx) = mpsc::channel();
        let runtime_a = encode_message(MessageType::UiRuntimeState, b"runtime-a");
        let runtime_b = encode_message(MessageType::UiRuntimeState, b"runtime-b");
        let appearance_a = encode_message(MessageType::UiAppearance, b"appearance-a");
        let appearance_b = encode_message(MessageType::UiAppearance, b"appearance-b");
        let barrier = encode_message(MessageType::Attached, &7_u32.to_le_bytes());
        tx.send(OutboundMessage::Frame(runtime_a)).unwrap();
        tx.send(OutboundMessage::Frame(appearance_a)).unwrap();
        tx.send(OutboundMessage::Frame(runtime_b.clone())).unwrap();
        tx.send(OutboundMessage::Frame(appearance_b.clone())).unwrap();
        tx.send(OutboundMessage::Frame(barrier.clone())).unwrap();

        let first = rx.recv().unwrap();
        let batch = collect_outbound_batch(first, &rx, true);
        assert_eq!(batch.frames, vec![runtime_b, appearance_b, barrier]);
        assert_eq!(batch.message_count, 5);
        assert_eq!(batch.coalesced_control_frames, 2);
    }

    #[test]
    fn auth_ok_payload_round_trips_protocol_version_and_capabilities() {
        let frame = encode_message(
            MessageType::AuthOk,
            &encode_auth_ok_payload("daemon-identity-01", "deadbeefcafebabe"),
        );
        let mut cursor = std::io::Cursor::new(frame);
        let (ty, payload) = read_message(&mut cursor).expect("auth ok frame");
        assert_eq!(ty, MessageType::AuthOk);
        assert_eq!(
            decode_auth_ok_payload(&payload),
            Some((
                REMOTE_PROTOCOL_VERSION,
                REMOTE_CAPABILITIES,
                Some(env!("CARGO_PKG_VERSION").to_string()),
                Some("deadbeefcafebabe".to_string()),
                Some("daemon-identity-01".to_string()),
            ))
        );
    }

    #[test]
    fn logical_channel_mapping_matches_current_message_families() {
        assert_eq!(logical_channel_for_message_type(MessageType::Auth), LogicalChannel::Control);
        assert_eq!(
            logical_channel_for_message_type(MessageType::SessionList),
            LogicalChannel::Control
        );
        assert_eq!(
            logical_channel_for_message_type(MessageType::Attach),
            LogicalChannel::SessionStream
        );
        assert_eq!(
            logical_channel_for_message_type(MessageType::Delta),
            LogicalChannel::SessionStream
        );
        assert_eq!(
            logical_channel_for_message_type(MessageType::UiPaneDelta),
            LogicalChannel::SessionStream
        );
        assert_eq!(
            logical_channel_for_message_type(MessageType::Input),
            LogicalChannel::InputControl
        );
        assert_eq!(
            logical_channel_for_message_type(MessageType::Scroll),
            LogicalChannel::InputControl
        );
        assert_eq!(
            logical_channel_for_message_type(MessageType::ExecuteCommand),
            LogicalChannel::InputControl
        );
        assert_eq!(
            logical_channel_for_message_type(MessageType::Heartbeat),
            LogicalChannel::Health
        );
        assert_eq!(
            logical_channel_for_message_type(MessageType::HeartbeatAck),
            LogicalChannel::Health
        );
    }

    #[test]
    fn handle_auth_message_accepts_valid_challenge_response_within_window() {
        let (outbound_tx, outbound_rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(State {
            clients: HashMap::from([(
                1,
                ClientState {
                    outbound: outbound_tx,
                    authenticated: false,
                    challenge: None,
                    connected_at: Instant::now(),
                    authenticated_at: None,
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
            )]),
            revivable_attachments: HashMap::new(),
            auth_key: Some(b"test-key".to_vec()),
            server_identity_id: "test-daemon".to_string(),
            server_instance_id: "test-instance".to_string(),
            tls_clients: std::collections::HashSet::new(),
        }));

        assert!(matches!(
            handle_auth_message(1, &[], &state),
            AuthHandling::Pending
        ));
        let challenge = match outbound_rx.recv().expect("challenge frame") {
            OutboundMessage::Frame(frame) => {
                let mut cursor = std::io::Cursor::new(frame);
                let (ty, payload) = read_message(&mut cursor).expect("decoded challenge");
                assert_eq!(ty, MessageType::AuthChallenge);
                payload
            }
            OutboundMessage::ScreenUpdate(_) => panic!("unexpected screen update"),
        };
        let mut mac =
            HmacSha256::new_from_slice(b"test-key").expect("valid HMAC key for test");
        mac.update(&challenge);
        let response = mac.finalize().into_bytes().to_vec();

        assert!(matches!(
            handle_auth_message(1, &response, &state),
            AuthHandling::Authenticated
        ));
        match outbound_rx.recv().expect("auth ok frame") {
            OutboundMessage::Frame(frame) => {
                let mut cursor = std::io::Cursor::new(frame);
                let (ty, payload) = read_message(&mut cursor).expect("decoded auth ok");
                assert_eq!(ty, MessageType::AuthOk);
                assert!(validate_auth_ok_payload(&payload, true).is_ok());
            }
            OutboundMessage::ScreenUpdate(_) => panic!("unexpected screen update"),
        }
        assert!(
            state
                .lock()
                .expect("remote server state poisoned")
                .clients
                .get(&1)
                .expect("client state")
                .authenticated
        );
    }

    #[test]
    fn handle_auth_message_rejects_expired_challenge_response() {
        let (outbound_tx, outbound_rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(State {
            clients: HashMap::from([(
                1,
                ClientState {
                    outbound: outbound_tx,
                    authenticated: false,
                    challenge: None,
                    connected_at: Instant::now(),
                    authenticated_at: None,
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
            )]),
            revivable_attachments: HashMap::new(),
            auth_key: Some(b"test-key".to_vec()),
            server_identity_id: "test-daemon".to_string(),
            server_instance_id: "test-instance".to_string(),
            tls_clients: std::collections::HashSet::new(),
        }));

        assert!(matches!(
            handle_auth_message(1, &[], &state),
            AuthHandling::Pending
        ));
        let challenge = match outbound_rx.recv().expect("challenge frame") {
            OutboundMessage::Frame(frame) => {
                let mut cursor = std::io::Cursor::new(frame);
                let (ty, payload) = read_message(&mut cursor).expect("decoded challenge");
                assert_eq!(ty, MessageType::AuthChallenge);
                payload
            }
            OutboundMessage::ScreenUpdate(_) => panic!("unexpected screen update"),
        };
        {
            let mut guard = state.lock().expect("remote server state poisoned");
            let client = guard.clients.get_mut(&1).expect("client state");
            let auth_challenge = client.challenge.as_mut().expect("stored challenge");
            auth_challenge.expires_at = Instant::now() - Duration::from_millis(1);
        }
        let mut mac =
            HmacSha256::new_from_slice(b"test-key").expect("valid HMAC key for test");
        mac.update(&challenge);
        let response = mac.finalize().into_bytes().to_vec();

        assert!(matches!(
            handle_auth_message(1, &response, &state),
            AuthHandling::Disconnect
        ));
        match outbound_rx.recv().expect("auth fail frame") {
            OutboundMessage::Frame(frame) => {
                let mut cursor = std::io::Cursor::new(frame);
                let (ty, payload) = read_message(&mut cursor).expect("decoded auth fail");
                assert_eq!(ty, MessageType::AuthFail);
                assert!(payload.is_empty());
            }
            OutboundMessage::ScreenUpdate(_) => panic!("unexpected screen update"),
        }
        let guard = state.lock().expect("remote server state poisoned");
        let client = guard.clients.get(&1).expect("client state");
        assert!(!client.authenticated);
        assert!(client.challenge.is_none());
    }

    #[test]
    fn validate_auth_ok_payload_accepts_current_handshake_contract() {
        let payload = encode_auth_ok_payload("daemon-identity-01", "deadbeefcafebabe");
        assert_eq!(validate_auth_ok_payload(&payload, true), Ok(()));
    }

    #[test]
    fn validate_auth_ok_payload_rejects_missing_resume_capability() {
        let mut payload = encode_auth_ok_payload("daemon-identity-01", "deadbeefcafebabe");
        payload[2..6].copy_from_slice(&(REMOTE_CAPABILITIES & !REMOTE_CAPABILITY_ATTACHMENT_RESUME).to_le_bytes());
        assert_eq!(
            validate_auth_ok_payload(&payload, true),
            Err("Remote server does not advertise attachment resume support".to_string())
        );
    }

    #[test]
    fn validate_auth_ok_payload_rejects_missing_daemon_identity_metadata() {
        let mut payload = encode_auth_ok_payload("daemon-identity-01", "deadbeefcafebabe");
        payload.truncate(payload.len() - "daemon-identity-01".len());
        assert_eq!(
            validate_auth_ok_payload(&payload, true),
            Err("Remote handshake is malformed".to_string())
        );
    }

    #[test]
    fn validate_auth_ok_payload_rejects_missing_direct_transport_capability() {
        let mut payload = encode_auth_ok_payload("daemon-identity-01", "deadbeefcafebabe");
        payload[2..6].copy_from_slice(
            &(REMOTE_CAPABILITIES
                & !(REMOTE_CAPABILITY_TCP_DIRECT_TRANSPORT | REMOTE_CAPABILITY_QUIC_DIRECT_TRANSPORT))
                .to_le_bytes(),
        );
        assert_eq!(
            validate_auth_ok_payload(&payload, true),
            Err("Remote server does not advertise a supported direct transport".to_string())
        );
    }

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
            None,
        )
        .expect_err("quic probing without a pin should be rejected");
        assert!(
            error.contains("QUIC direct transport requires an expected_server_identity pin"),
            "expected pin-required error, got: {error}"
        );
    }

    #[test]
    fn read_loop_emits_list_sessions_for_authenticated_client() {
        let (outbound_tx, _outbound_rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(State {
            clients: HashMap::from([(
                1,
                ClientState {
                    outbound: outbound_tx,
                    authenticated: true,
                    challenge: None,
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
            )]),
            revivable_attachments: HashMap::new(),
            auth_key: Some(b"test-key".to_vec()),
            server_identity_id: "test-daemon".to_string(),
            server_instance_id: "test-instance".to_string(),
            tls_clients: std::collections::HashSet::new(),
        }));
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let mut frames = Vec::new();
        frames.extend_from_slice(&encode_message(MessageType::ListSessions, &[]));

        read_loop(std::io::Cursor::new(frames), 1, state, cmd_tx);

        match cmd_rx.recv().expect("remote command") {
            RemoteCmd::ListSessions { client_id } => assert_eq!(client_id, 1),
            other => panic!("unexpected remote command: {other:?}"),
        }
    }

    #[test]
    fn read_loop_replies_to_heartbeat_without_runtime_command() {
        let (outbound_tx, outbound_rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(State {
            clients: HashMap::from([(
                1,
                ClientState {
                    outbound: outbound_tx,
                    authenticated: true,
                    challenge: None,
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
            )]),
            revivable_attachments: HashMap::new(),
            auth_key: Some(b"test-key".to_vec()),
            server_identity_id: "test-daemon".to_string(),
            server_instance_id: "test-instance".to_string(),
            tls_clients: std::collections::HashSet::new(),
        }));
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let frame = encode_message(MessageType::Heartbeat, b"ping");

        read_loop(std::io::Cursor::new(frame), 1, state, cmd_tx);

        match outbound_rx.recv().expect("heartbeat ack frame") {
            OutboundMessage::Frame(frame) => {
                let mut cursor = std::io::Cursor::new(frame);
                let (ty, payload) = read_message(&mut cursor).expect("decoded heartbeat ack");
                assert_eq!(ty, MessageType::HeartbeatAck);
                assert_eq!(payload, b"ping");
            }
            OutboundMessage::ScreenUpdate(_) => panic!("unexpected screen update"),
        }
        assert!(cmd_rx.try_recv().is_err());
    }

    #[test]
    fn read_probe_auth_reply_skips_unsolicited_session_list() {
        let mut frames = Vec::new();
        frames.extend_from_slice(&encode_message(MessageType::SessionList, b"[]"));
        frames.extend_from_slice(&encode_message(
            MessageType::AuthOk,
            &encode_auth_ok_payload("test-daemon", "test-instance"),
        ));
        let (ty, payload) = read_probe_auth_reply(
            &mut std::io::Cursor::new(frames),
            "127.0.0.1",
            7359,
        )
        .expect("auth reply");
        assert_eq!(ty, MessageType::AuthOk);
        assert!(validate_auth_ok_payload(&payload, false).is_ok());
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

        let error = probe_remote_endpoint("127.0.0.1", port, None, None)
            .expect_err("probe should reject");
        assert!(
            error.contains("Unsupported remote protocol version"),
            "unexpected error: {error}"
        );

        server.join().expect("probe server thread");
    }

    #[test]
    fn decode_session_list_payload_round_trips_encoded_sessions() {
        let payload = encode_session_list(&[
            RemoteSessionInfo {
                id: 7,
                name: "Tab 1".to_string(),
                title: "shell".to_string(),
                pwd: "/tmp".to_string(),
                attached: true,
                child_exited: false,
            },
            RemoteSessionInfo {
                id: 8,
                name: String::new(),
                title: "logs".to_string(),
                pwd: "/var/log".to_string(),
                attached: false,
                child_exited: true,
            },
        ]);

        let decoded = decode_session_list_payload(&payload).expect("decode session list");
        assert_eq!(
            decoded,
            vec![
                RemoteDirectSessionInfo {
                    id: 7,
                    name: "Tab 1".to_string(),
                    title: "shell".to_string(),
                    pwd: "/tmp".to_string(),
                    attached: true,
                    child_exited: false,
                },
                RemoteDirectSessionInfo {
                    id: 8,
                    name: String::new(),
                    title: "logs".to_string(),
                    pwd: "/var/log".to_string(),
                    attached: false,
                    child_exited: true,
                },
            ]
        );
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

        let summary = list_remote_daemon_sessions("127.0.0.1", port, None, None)
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
    fn direct_transport_session_lists_sessions_over_unix_stream() {
        let (client_stream, mut server_stream) =
            UnixStream::pair().expect("create unix stream pair");
        let server = std::thread::spawn(move || {
            let (ty, payload) = read_message(&mut server_stream).expect("read auth request");
            assert_eq!(ty, MessageType::Auth);
            assert!(payload.is_empty());

            server_stream
                .write_all(&encode_message(
                    MessageType::AuthOk,
                    &encode_auth_ok_payload("unix-daemon", "unix-instance"),
                ))
                .expect("write auth ok");

            let (ty, payload) = read_message(&mut server_stream).expect("read heartbeat");
            assert_eq!(ty, MessageType::Heartbeat);
            server_stream
                .write_all(&encode_message(MessageType::HeartbeatAck, &payload))
                .expect("write heartbeat ack");

            let (ty, payload) = read_message(&mut server_stream).expect("read list sessions");
            assert_eq!(ty, MessageType::ListSessions);
            assert!(payload.is_empty());
            server_stream
                .write_all(&encode_message(
                    MessageType::SessionList,
                    &encode_session_list(&[RemoteSessionInfo {
                        id: 21,
                        name: "unix".to_string(),
                        title: "shell".to_string(),
                        pwd: "/tmp".to_string(),
                        attached: false,
                        child_exited: false,
                    }]),
                ))
                .expect("write session list");
        });

        let mut client = DirectTransportSession::connect_over_stream(
            client_stream,
            "unix-test".to_string(),
            0,
            None,
            Some("unix-daemon"),
        )
        .expect("connect over unix stream");
        let heartbeat_rtt_ms = client
            .heartbeat_round_trip(b"unix-heartbeat")
            .expect("heartbeat round trip");
        assert!(heartbeat_rtt_ms <= 5_000);
        let sessions = client.list_sessions().expect("list sessions over unix stream");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, 21);
        assert_eq!(sessions[0].name, "unix");
        assert_eq!(client.server_identity_id.as_deref(), Some("unix-daemon"));
        assert_eq!(client.server_instance_id.as_deref(), Some("unix-instance"));

        server.join().expect("unix server thread");
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
            None,
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
    fn decode_attached_payload_accepts_legacy_and_resume_forms() {
        let legacy = decode_attached_payload(&7_u32.to_le_bytes()).expect("legacy attached");
        assert_eq!(legacy.session_id, 7);
        assert_eq!(legacy.attachment_id, None);
        assert_eq!(legacy.resume_token, None);

        let mut resumed = 7_u32.to_le_bytes().to_vec();
        resumed.extend_from_slice(&99_u64.to_le_bytes());
        resumed.extend_from_slice(&1234_u64.to_le_bytes());
        let resumed = decode_attached_payload(&resumed).expect("resumed attached");
        assert_eq!(resumed.session_id, 7);
        assert_eq!(resumed.attachment_id, Some(99));
        assert_eq!(resumed.resume_token, Some(1234));
    }

    #[test]
    fn decode_remote_full_state_payload_round_trips_encoded_state() {
        let state = RemoteFullState {
            rows: 1,
            cols: 2,
            cursor_x: 1,
            cursor_y: 0,
            cursor_visible: true,
            cursor_blinking: false,
            cursor_style: 2,
            cells: vec![
                RemoteCell {
                    codepoint: u32::from('A'),
                    fg: [1, 2, 3],
                    bg: [4, 5, 6],
                    style_flags: STYLE_FLAG_BOLD,
                    wide: false,
                },
                RemoteCell {
                    codepoint: u32::from('B'),
                    fg: [7, 8, 9],
                    bg: [10, 11, 12],
                    style_flags: STYLE_FLAG_ITALIC,
                    wide: true,
                },
            ],
        };
        let payload = encode_full_state(&state, None, false);
        let decoded = decode_remote_full_state_payload(&payload).expect("decode full state");
        assert_eq!(decoded, state);
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
            None,
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

        let summary = attach_remote_daemon_session("127.0.0.1", port, None, Some("test-daemon"), 7, None, None)
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
            None,
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
    }

    #[test]
    fn read_loop_closes_connection_after_auth_failure() {
        let (outbound_tx, outbound_rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(State {
            clients: HashMap::from([(
                1,
                ClientState {
                    outbound: outbound_tx,
                    authenticated: false,
                    challenge: None,
                    connected_at: Instant::now(),
                    authenticated_at: None,
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
            )]),
            revivable_attachments: HashMap::new(),
            auth_key: Some(b"test-key".to_vec()),
            server_identity_id: "test-daemon".to_string(),
            server_instance_id: "test-instance".to_string(),
            tls_clients: std::collections::HashSet::new(),
        }));
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let mut frames = Vec::new();
        frames.extend_from_slice(&encode_message(MessageType::Auth, &[]));
        frames.extend_from_slice(&encode_message(MessageType::Auth, b"definitely-wrong-hmac"));
        frames.extend_from_slice(&encode_message(MessageType::ListSessions, &[]));

        read_loop(std::io::Cursor::new(frames), 1, Arc::clone(&state), cmd_tx);

        let mut saw_challenge = false;
        let mut saw_fail = false;
        while let Ok(message) = outbound_rx.try_recv() {
            match message {
                OutboundMessage::Frame(frame) => {
                    let mut cursor = std::io::Cursor::new(frame);
                    let (ty, _) = read_message(&mut cursor).expect("decoded outbound frame");
                    if ty == MessageType::AuthChallenge {
                        saw_challenge = true;
                    } else if ty == MessageType::AuthFail {
                        saw_fail = true;
                    }
                }
                OutboundMessage::ScreenUpdate(_) => panic!("unexpected screen update"),
            }
        }
        assert!(saw_challenge);
        assert!(saw_fail);
        assert!(cmd_rx.try_recv().is_err());
        let guard = state.lock().expect("remote server state poisoned");
        assert!(!guard.clients.contains_key(&1));
    }

    #[test]
    fn read_loop_closes_idle_unauthenticated_connection_after_timeout() {
        let (outbound_tx, outbound_rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(State {
            clients: HashMap::from([(
                1,
                ClientState {
                    outbound: outbound_tx,
                    authenticated: false,
                    challenge: None,
                    connected_at: Instant::now() - AUTH_CHALLENGE_WINDOW - Duration::from_secs(1),
                    authenticated_at: None,
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
            )]),
            revivable_attachments: HashMap::new(),
            auth_key: Some(b"test-key".to_vec()),
            server_identity_id: "test-daemon".to_string(),
            server_instance_id: "test-instance".to_string(),
            tls_clients: std::collections::HashSet::new(),
        }));
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let reader = TimeoutScriptedReader::new([Err(io::ErrorKind::TimedOut)]);

        read_loop(reader, 1, Arc::clone(&state), cmd_tx);

        match outbound_rx.recv().expect("auth fail frame") {
            OutboundMessage::Frame(frame) => {
                let mut cursor = std::io::Cursor::new(frame);
                let (ty, payload) = read_message(&mut cursor).expect("decoded auth fail");
                assert_eq!(ty, MessageType::AuthFail);
                assert!(payload.is_empty());
            }
            OutboundMessage::ScreenUpdate(_) => panic!("unexpected screen update"),
        }
        assert!(cmd_rx.try_recv().is_err());
        let guard = state.lock().expect("remote server state poisoned");
        assert!(!guard.clients.contains_key(&1));
    }

    #[test]
    fn read_loop_closes_expired_auth_challenge_after_timeout() {
        let (outbound_tx, outbound_rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(State {
            clients: HashMap::from([(
                1,
                ClientState {
                    outbound: outbound_tx,
                    authenticated: false,
                    challenge: Some(AuthChallengeState {
                        bytes: [5; 32],
                        expires_at: Instant::now() - Duration::from_secs(1),
                    }),
                    connected_at: Instant::now(),
                    authenticated_at: None,
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
            )]),
            revivable_attachments: HashMap::new(),
            auth_key: Some(b"test-key".to_vec()),
            server_identity_id: "test-daemon".to_string(),
            server_instance_id: "test-instance".to_string(),
            tls_clients: std::collections::HashSet::new(),
        }));
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let reader = TimeoutScriptedReader::new([Err(io::ErrorKind::TimedOut)]);

        read_loop(reader, 1, Arc::clone(&state), cmd_tx);

        match outbound_rx.recv().expect("auth fail frame") {
            OutboundMessage::Frame(frame) => {
                let mut cursor = std::io::Cursor::new(frame);
                let (ty, payload) = read_message(&mut cursor).expect("decoded auth fail");
                assert_eq!(ty, MessageType::AuthFail);
                assert!(payload.is_empty());
            }
            OutboundMessage::ScreenUpdate(_) => panic!("unexpected screen update"),
        }
        assert!(cmd_rx.try_recv().is_err());
        let guard = state.lock().expect("remote server state poisoned");
        assert!(!guard.clients.contains_key(&1));
    }

    #[test]
    fn read_loop_closes_authenticated_remote_client_after_heartbeat_timeout() {
        let (outbound_tx, outbound_rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(State {
            clients: HashMap::from([(
                1,
                ClientState {
                    outbound: outbound_tx,
                    authenticated: true,
                    challenge: None,
                    connected_at: Instant::now()
                        - DIRECT_CLIENT_HEARTBEAT_WINDOW
                        - Duration::from_secs(2),
                    authenticated_at: Some(
                        Instant::now()
                            - DIRECT_CLIENT_HEARTBEAT_WINDOW
                            - Duration::from_secs(2),
                    ),
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
            )]),
            revivable_attachments: HashMap::new(),
            auth_key: None,
            server_identity_id: "test-daemon".to_string(),
            server_instance_id: "test-instance".to_string(),
            tls_clients: std::collections::HashSet::new(),
        }));
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let reader = TimeoutScriptedReader::new([Err(io::ErrorKind::TimedOut)]);

        read_loop(reader, 1, Arc::clone(&state), cmd_tx);

        match outbound_rx.recv().expect("heartbeat timeout error frame") {
            OutboundMessage::Frame(frame) => {
                let mut cursor = std::io::Cursor::new(frame);
                let (ty, payload) = read_message(&mut cursor).expect("decoded error frame");
                assert_eq!(ty, MessageType::ErrorMsg);
                assert_eq!(String::from_utf8(payload).expect("utf8"), "heartbeat timeout");
            }
            OutboundMessage::ScreenUpdate(_) => panic!("unexpected screen update"),
        }
        assert!(cmd_rx.try_recv().is_err());
        let guard = state.lock().expect("remote server state poisoned");
        assert!(!guard.clients.contains_key(&1));
    }

    #[test]
    fn local_authenticated_clients_do_not_timeout_without_heartbeat() {
        let (outbound_tx, outbound_rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(State {
            clients: HashMap::from([(
                1,
                ClientState {
                    outbound: outbound_tx,
                    authenticated: true,
                    challenge: None,
                    connected_at: Instant::now()
                        - DIRECT_CLIENT_HEARTBEAT_WINDOW
                        - Duration::from_secs(2),
                    authenticated_at: Some(
                        Instant::now()
                            - DIRECT_CLIENT_HEARTBEAT_WINDOW
                            - Duration::from_secs(2),
                    ),
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
                    is_local: true,
                },
            )]),
            revivable_attachments: HashMap::new(),
            auth_key: None,
            server_identity_id: "test-daemon".to_string(),
            server_instance_id: "test-instance".to_string(),
            tls_clients: std::collections::HashSet::new(),
        }));
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let reader = TimeoutScriptedReader::new([
            Err(io::ErrorKind::TimedOut),
            Err(io::ErrorKind::BrokenPipe),
        ]);

        read_loop(reader, 1, Arc::clone(&state), cmd_tx);

        assert!(outbound_rx.try_recv().is_err());
        assert!(cmd_rx.try_recv().is_err());
        let guard = state.lock().expect("remote server state poisoned");
        assert!(!guard.clients.contains_key(&1));
    }

    #[test]
    fn heartbeat_timeout_preserves_revivable_attachment_for_remote_client() {
        let (outbound_tx, outbound_rx) = mpsc::channel();
        let cached_state = Arc::new(RemoteFullState {
            rows: 1,
            cols: 1,
            cursor_x: 0,
            cursor_y: 0,
            cursor_visible: true,
            cursor_blinking: false,
            cursor_style: 1,
            cells: vec![RemoteCell {
                codepoint: u32::from('x'),
                fg: [1, 2, 3],
                bg: [0, 0, 0],
                style_flags: 0,
                wide: false,
            }],
        });
        let state = Arc::new(Mutex::new(State {
            clients: HashMap::from([(
                1,
                ClientState {
                    outbound: outbound_tx,
                    authenticated: true,
                    challenge: None,
                    connected_at: Instant::now()
                        - DIRECT_CLIENT_HEARTBEAT_WINDOW
                        - Duration::from_secs(2),
                    authenticated_at: Some(
                        Instant::now()
                            - DIRECT_CLIENT_HEARTBEAT_WINDOW
                            - Duration::from_secs(2),
                    ),
                    last_heartbeat_at: None,
                    attached_session: Some(11),
                    attachment_id: Some(0xabc),
                    resume_token: Some(0xdef),
                    last_session_list_payload: None,
                    last_ui_runtime_state_payload: None,
                    last_ui_appearance_payload: None,
                    last_state: Some(Arc::clone(&cached_state)),
                    pane_states: HashMap::new(),
                    latest_input_seq: Some(9),
                    is_local: false,
                },
            )]),
            revivable_attachments: HashMap::new(),
            auth_key: None,
            server_identity_id: "test-daemon".to_string(),
            server_instance_id: "test-instance".to_string(),
            tls_clients: std::collections::HashSet::new(),
        }));
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let reader = TimeoutScriptedReader::new([Err(io::ErrorKind::TimedOut)]);

        read_loop(reader, 1, Arc::clone(&state), cmd_tx);

        match outbound_rx.recv().expect("heartbeat timeout error frame") {
            OutboundMessage::Frame(frame) => {
                let mut cursor = std::io::Cursor::new(frame);
                let (ty, payload) = read_message(&mut cursor).expect("decoded error frame");
                assert_eq!(ty, MessageType::ErrorMsg);
                assert_eq!(String::from_utf8(payload).expect("utf8"), "heartbeat timeout");
            }
            OutboundMessage::ScreenUpdate(_) => panic!("unexpected screen update"),
        }
        assert!(cmd_rx.try_recv().is_err());
        let guard = state.lock().expect("remote server state poisoned");
        assert!(!guard.clients.contains_key(&1));
        let attachment = guard
            .revivable_attachments
            .get(&0xabc)
            .expect("revivable attachment preserved");
        assert_eq!(attachment.session_id, 11);
        assert_eq!(attachment.resume_token, 0xdef);
        assert_eq!(attachment.latest_input_seq, Some(9));
        assert_eq!(attachment.last_state.as_ref(), Some(&cached_state));
    }

    #[test]
    fn parse_attach_request_supports_session_only_attachment_and_resume_token_forms() {
        let session_only = 11_u32.to_le_bytes().to_vec();
        assert_eq!(parse_attach_request(&session_only), Some((11, None, None)));

        let mut with_attachment = session_only.clone();
        with_attachment.extend_from_slice(&0xabc_u64.to_le_bytes());
        assert_eq!(
            parse_attach_request(&with_attachment),
            Some((11, Some(0xabc), None))
        );

        let mut with_resume = with_attachment.clone();
        with_resume.extend_from_slice(&0xdef_u64.to_le_bytes());
        assert_eq!(
            parse_attach_request(&with_resume),
            Some((11, Some(0xabc), Some(0xdef)))
        );
    }

    fn unique_identity_dir(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!(
            "boo-remote-daemon-identity-{label}-{}-{nanos}",
            std::process::id()
        ))
    }

    #[test]
    fn load_or_create_daemon_identity_material_persists_keypair_and_cert() {
        let dir = unique_identity_dir("persist");
        let _ = std::fs::remove_dir_all(&dir);

        let first = load_or_create_daemon_identity_material_at(&dir);
        let second = load_or_create_daemon_identity_material_at(&dir);

        assert!(!first.identity_id.is_empty());
        assert_eq!(first.identity_id, second.identity_id);
        assert_eq!(first.key_pem, second.key_pem);
        assert_eq!(first.cert_pem, second.cert_pem);
        assert!(dir.join("key.pem").exists());
        assert!(dir.join("cert.pem").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_or_create_daemon_identity_material_derives_identity_from_spki() {
        let dir = unique_identity_dir("spki");
        let _ = std::fs::remove_dir_all(&dir);

        let material = load_or_create_daemon_identity_material_at(&dir);
        let keypair = rcgen::KeyPair::from_pem(&material.key_pem).expect("parse key");
        let expected = derive_identity_id(keypair.public_key_der());
        assert_eq!(material.identity_id, expected);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[derive(Debug)]
    struct TrustAllServerCertVerifier {
        provider: Arc<rustls::crypto::CryptoProvider>,
    }

    impl rustls::client::danger::ServerCertVerifier for TrustAllServerCertVerifier {
        fn verify_server_cert(
            &self,
            _end_entity: &rustls::pki_types::CertificateDer<'_>,
            _intermediates: &[rustls::pki_types::CertificateDer<'_>],
            _server_name: &rustls::pki_types::ServerName<'_>,
            _ocsp_response: &[u8],
            _now: rustls::pki_types::UnixTime,
        ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
            Ok(rustls::client::danger::ServerCertVerified::assertion())
        }

        fn verify_tls12_signature(
            &self,
            message: &[u8],
            cert: &rustls::pki_types::CertificateDer<'_>,
            dss: &rustls::DigitallySignedStruct,
        ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
            rustls::crypto::verify_tls12_signature(
                message,
                cert,
                dss,
                &self.provider.signature_verification_algorithms,
            )
        }

        fn verify_tls13_signature(
            &self,
            message: &[u8],
            cert: &rustls::pki_types::CertificateDer<'_>,
            dss: &rustls::DigitallySignedStruct,
        ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
            rustls::crypto::verify_tls13_signature(
                message,
                cert,
                dss,
                &self.provider.signature_verification_algorithms,
            )
        }

        fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
            self.provider.signature_verification_algorithms.supported_schemes()
        }
    }

    fn build_test_trust_all_client_config() -> rustls::ClientConfig {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        rustls::ClientConfig::builder_with_provider(Arc::clone(&provider))
            .with_safe_default_protocol_versions()
            .expect("client tls protocol versions")
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(TrustAllServerCertVerifier {
                provider,
            }))
            .with_no_client_auth()
    }

    fn start_tls_test_server(
        material: &DaemonIdentityMaterial,
        tls_config: Arc<rustls::ServerConfig>,
    ) -> (std::net::SocketAddr, std::thread::JoinHandle<()>) {
        start_tls_test_server_with_auth(material, tls_config, None)
    }

    fn start_tls_test_server_with_auth(
        material: &DaemonIdentityMaterial,
        tls_config: Arc<rustls::ServerConfig>,
        auth_key: Option<&[u8]>,
    ) -> (std::net::SocketAddr, std::thread::JoinHandle<()>) {
        use std::net::TcpListener;

        let state = Arc::new(Mutex::new(State {
            clients: HashMap::new(),
            revivable_attachments: HashMap::new(),
            auth_key: auth_key.map(<[u8]>::to_vec),
            server_identity_id: material.identity_id.clone(),
            server_instance_id: "test-instance".to_string(),
            tls_clients: std::collections::HashSet::new(),
        }));
        let (cmd_tx, _cmd_rx) = mpsc::channel();
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
        let addr = listener.local_addr().expect("local addr");
        let handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            let _ = stream.set_read_timeout(Some(REMOTE_READ_TIMEOUT));
            serve_incoming_tcp_client(stream, tls_config, state, cmd_tx);
        });
        // Prevent cmd_rx from being dropped early — keep the _cmd_rx alive via leak.
        // (Simpler than plumbing it back out for a test that never reads from it.)
        std::mem::forget(_cmd_rx);
        (addr, handle)
    }

    #[test]
    fn direct_tls_client_connects_with_matching_pin() {
        let dir = unique_identity_dir("tls-pin-ok");
        let _ = std::fs::remove_dir_all(&dir);
        let material = load_or_create_daemon_identity_material_at(&dir);
        let tls_config =
            build_remote_server_tls_config(&material).expect("build server tls config");
        let (addr, server_handle) = start_tls_test_server(&material, tls_config);

        let session = DirectTransportSession::<TlsClientStream>::connect_tls(
            &addr.ip().to_string(),
            addr.port(),
            None,
            &material.identity_id,
        )
        .expect("tls connect");

        assert!(!session.auth_required);
        assert_eq!(
            session.server_identity_id.as_deref(),
            Some(material.identity_id.as_str())
        );
        assert_ne!(
            session.capabilities & REMOTE_CAPABILITY_TCP_TLS_TRANSPORT,
            0,
        );

        drop(session);
        let _ = server_handle.join();
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn start_quic_test_server(
        material: &DaemonIdentityMaterial,
        auth_key: Option<&[u8]>,
    ) -> (
        std::net::SocketAddr,
        crate::remote_quic::QuicListener,
        mpsc::Receiver<RemoteCmd>,
    ) {
        let tls_config =
            build_remote_server_tls_config(material).expect("build server tls config");
        let server_config =
            crate::remote_quic::build_quic_server_config(tls_config).expect("quic config");
        let bind_addr: std::net::SocketAddr = "127.0.0.1:0".parse().unwrap();
        let listener =
            crate::remote_quic::bind_quic_listener(bind_addr, server_config).expect("bind quic");
        let addr = listener.local_addr();

        let state = Arc::new(Mutex::new(State {
            clients: HashMap::new(),
            revivable_attachments: HashMap::new(),
            auth_key: auth_key.map(<[u8]>::to_vec),
            server_identity_id: material.identity_id.clone(),
            server_instance_id: "test-instance".to_string(),
            tls_clients: std::collections::HashSet::new(),
        }));
        let (cmd_tx, cmd_rx) = mpsc::channel();

        let endpoint = listener.endpoint();
        let runtime = crate::remote_quic::shared_quic_runtime().expect("runtime");
        runtime.spawn(async move {
            while let Some(incoming) = endpoint.accept().await {
                let state = Arc::clone(&state);
                let cmd_tx = cmd_tx.clone();
                tokio::spawn(async move {
                    handle_quic_incoming(incoming, state, cmd_tx).await;
                });
            }
        });

        (addr, listener, cmd_rx)
    }

    #[test]
    fn direct_quic_client_connects_with_matching_pin() {
        let dir = unique_identity_dir("quic-pin-ok");
        let _ = std::fs::remove_dir_all(&dir);
        let material = load_or_create_daemon_identity_material_at(&dir);
        let (addr, _listener, _cmd_rx) = start_quic_test_server(&material, None);

        let session = DirectTransportSession::<QuicClientStream>::connect_quic(
            &addr.ip().to_string(),
            addr.port(),
            None,
            &material.identity_id,
        )
        .expect("quic connect");

        assert!(!session.auth_required);
        assert_eq!(
            session.server_identity_id.as_deref(),
            Some(material.identity_id.as_str())
        );
        assert_ne!(
            session.capabilities & REMOTE_CAPABILITY_QUIC_DIRECT_TRANSPORT,
            0,
            "server should advertise the QUIC transport capability"
        );

        drop(session);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn probe_selected_direct_transport_tls_dispatches_quic_over_quinn() {
        let dir = unique_identity_dir("probe-quic");
        let _ = std::fs::remove_dir_all(&dir);
        let material = load_or_create_daemon_identity_material_at(&dir);
        let (addr, _listener, _cmd_rx) = start_quic_test_server(&material, None);

        let summary = probe_selected_direct_transport_tls(
            DirectTransportKind::QuicDirect,
            &addr.ip().to_string(),
            addr.port(),
            None,
            &material.identity_id,
        )
        .expect("upgrade probe over QUIC");

        assert_eq!(summary.selected_transport, DirectTransportKind::QuicDirect);
        assert_eq!(
            summary.probe.server_identity_id.as_deref(),
            Some(material.identity_id.as_str())
        );
        assert_ne!(
            summary.probe.capabilities & REMOTE_CAPABILITY_QUIC_DIRECT_TRANSPORT,
            0,
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn direct_quic_client_rejects_mismatched_pin() {
        let dir = unique_identity_dir("quic-pin-mismatch");
        let _ = std::fs::remove_dir_all(&dir);
        let material = load_or_create_daemon_identity_material_at(&dir);
        let (addr, _listener, _cmd_rx) = start_quic_test_server(&material, None);

        let bogus_pin = derive_identity_id([0xCCu8; 32].as_slice());
        assert_ne!(bogus_pin, material.identity_id);

        let err = match DirectTransportSession::<QuicClientStream>::connect_quic(
            &addr.ip().to_string(),
            addr.port(),
            None,
            &bogus_pin,
        ) {
            Ok(_) => panic!("quic connect must fail on mismatched pin"),
            Err(err) => err,
        };
        assert!(
            err.to_lowercase().contains("quic")
                || err.to_lowercase().contains("handshake")
                || err.to_lowercase().contains("cert"),
            "expected a TLS/QUIC handshake failure message, got: {err}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn direct_tls_client_succeeds_with_auth_key() {
        let dir = unique_identity_dir("tls-auth-ok");
        let _ = std::fs::remove_dir_all(&dir);
        let material = load_or_create_daemon_identity_material_at(&dir);
        let tls_config =
            build_remote_server_tls_config(&material).expect("build server tls config");
        let (addr, server_handle) =
            start_tls_test_server_with_auth(&material, tls_config, Some(b"test-secret"));

        let session = DirectTransportSession::<TlsClientStream>::connect_tls(
            &addr.ip().to_string(),
            addr.port(),
            Some("test-secret"),
            &material.identity_id,
        )
        .expect("tls+auth connect");
        assert!(session.auth_required);
        assert_eq!(
            session.server_identity_id.as_deref(),
            Some(material.identity_id.as_str())
        );

        drop(session);
        let _ = server_handle.join();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn direct_tls_client_rejects_wrong_auth_key() {
        let dir = unique_identity_dir("tls-auth-bad");
        let _ = std::fs::remove_dir_all(&dir);
        let material = load_or_create_daemon_identity_material_at(&dir);
        let tls_config =
            build_remote_server_tls_config(&material).expect("build server tls config");
        let (addr, server_handle) =
            start_tls_test_server_with_auth(&material, tls_config, Some(b"correct-key"));

        let err = match DirectTransportSession::<TlsClientStream>::connect_tls(
            &addr.ip().to_string(),
            addr.port(),
            Some("wrong-key"),
            &material.identity_id,
        ) {
            Ok(_) => panic!("wrong auth key must fail inside tls tunnel"),
            Err(err) => err,
        };
        assert!(
            err.to_lowercase().contains("auth"),
            "expected an auth failure message, got: {err}"
        );

        let _ = server_handle.join();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn direct_tls_client_rejects_mismatched_pin() {
        let dir = unique_identity_dir("tls-pin-mismatch");
        let _ = std::fs::remove_dir_all(&dir);
        let material = load_or_create_daemon_identity_material_at(&dir);
        let tls_config =
            build_remote_server_tls_config(&material).expect("build server tls config");
        let (addr, server_handle) = start_tls_test_server(&material, tls_config);

        // Pin an identity that does not match the server's SPKI.
        let bogus_pin = derive_identity_id([0xAAu8; 32].as_slice());
        assert_ne!(bogus_pin, material.identity_id);

        let err = match DirectTransportSession::<TlsClientStream>::connect_tls(
            &addr.ip().to_string(),
            addr.port(),
            None,
            &bogus_pin,
        ) {
            Ok(_) => panic!("tls connect must fail on mismatched pin"),
            Err(error) => error,
        };
        assert!(
            err.contains("tls handshake") || err.contains("failed"),
            "error should describe a handshake failure, got: {err}"
        );

        // Server thread should also terminate (handshake failed there).
        let _ = server_handle.join();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cert_der_matches_identity_accepts_current_cert() {
        let dir = unique_identity_dir("spki-match");
        let _ = std::fs::remove_dir_all(&dir);
        let material = load_or_create_daemon_identity_material_at(&dir);

        let cert_ders = rustls_pemfile::certs(&mut material.cert_pem.as_bytes())
            .collect::<Result<Vec<_>, _>>()
            .expect("parse certs");
        let cert_der = cert_ders.first().expect("at least one cert");
        assert!(cert_der_matches_identity(
            cert_der.as_ref(),
            &material.identity_id
        ));

        let bogus = derive_identity_id([0u8; 32].as_slice());
        assert!(!cert_der_matches_identity(cert_der.as_ref(), &bogus));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn should_disconnect_idle_client_enforces_absolute_auth_deadline() {
        // Simulates an attacker that keeps refreshing the HMAC challenge past
        // AUTH_CHALLENGE_WINDOW. The client HAS an active challenge (expires in the
        // future) but its connected_at is already older than the window. Absent the
        // absolute deadline, should_disconnect_idle_client would return None and the
        // socket would stay pinned forever.
        let (outbound_tx, _outbound_rx) = mpsc::channel();
        let now = Instant::now();
        let state = Arc::new(Mutex::new(State {
            clients: HashMap::from([(
                1,
                ClientState {
                    outbound: outbound_tx,
                    authenticated: false,
                    challenge: Some(AuthChallengeState {
                        bytes: [0u8; 32],
                        // Fresh challenge — not expired on its own.
                        expires_at: now + AUTH_CHALLENGE_WINDOW,
                    }),
                    connected_at: now - AUTH_CHALLENGE_WINDOW - Duration::from_secs(1),
                    authenticated_at: None,
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
            )]),
            revivable_attachments: HashMap::new(),
            auth_key: Some(b"test-key".to_vec()),
            server_identity_id: "test-daemon".to_string(),
            server_instance_id: "test-instance".to_string(),
            tls_clients: std::collections::HashSet::new(),
        }));

        assert_eq!(
            should_disconnect_idle_client(&state, 1),
            Some("auth-timeout"),
            "clients past the absolute auth deadline must be disconnected even with a fresh challenge",
        );
    }

    #[test]
    fn serve_incoming_tcp_client_completes_tls_handshake_and_auth() {
        use std::net::{TcpListener, TcpStream};

        let dir = unique_identity_dir("tls-serve");
        let _ = std::fs::remove_dir_all(&dir);
        let material = load_or_create_daemon_identity_material_at(&dir);
        let expected_identity = material.identity_id.clone();
        let tls_config =
            build_remote_server_tls_config(&material).expect("build server tls config");

        let state = Arc::new(Mutex::new(State {
            clients: HashMap::new(),
            revivable_attachments: HashMap::new(),
            auth_key: None,
            server_identity_id: expected_identity.clone(),
            server_instance_id: "test-instance".to_string(),
            tls_clients: std::collections::HashSet::new(),
        }));
        let (cmd_tx, _cmd_rx) = mpsc::channel();

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
        let addr = listener.local_addr().expect("local addr");
        let server_handle = std::thread::spawn({
            let tls_config = Arc::clone(&tls_config);
            let state = Arc::clone(&state);
            move || {
                let (stream, _) = listener.accept().expect("accept");
                let _ = stream.set_read_timeout(Some(REMOTE_READ_TIMEOUT));
                serve_incoming_tcp_client(stream, tls_config, state, cmd_tx);
            }
        });

        let tcp = TcpStream::connect(addr).expect("tcp connect");
        // Generous read timeout — the Nix build sandbox on Darwin has been observed
        // to need several seconds for the TLS handshake + Auth round trip.
        tcp.set_read_timeout(Some(Duration::from_secs(30)))
            .expect("set read timeout");
        let client_config = build_test_trust_all_client_config();
        let server_name = rustls::pki_types::ServerName::try_from("boo-remote-daemon")
            .expect("parse server name");
        let client_conn =
            rustls::ClientConnection::new(Arc::new(client_config), server_name)
                .expect("client connection");
        let mut tls_stream = rustls::StreamOwned::new(client_conn, tcp);

        tls_stream
            .write_all(&encode_message(MessageType::Auth, &[]))
            .expect("send auth");
        let (ty, payload) = read_message(&mut tls_stream).expect("read auth reply");
        assert!(matches!(ty, MessageType::AuthOk), "got {ty:?}");

        validate_auth_ok_payload(&payload, false).expect("auth ok payload valid");
        let (_, capabilities, _, _, server_identity_id) =
            decode_auth_ok_payload(&payload).expect("decode auth ok");
        assert_eq!(
            server_identity_id.as_deref(),
            Some(expected_identity.as_str())
        );
        assert_ne!(
            capabilities & REMOTE_CAPABILITY_TCP_TLS_TRANSPORT,
            0,
            "server should advertise TLS transport capability"
        );

        // The TLS-wrapped client must be tracked in State::tls_clients so the
        // diagnostic snapshot can report transport_kind="tcp-tls".
        {
            let state_guard = state.lock().expect("state lock");
            assert_eq!(
                state_guard.tls_clients.len(),
                1,
                "expected one TLS-tracked client, got {}",
                state_guard.tls_clients.len()
            );
        }

        drop(tls_stream);
        let _ = server_handle.join();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn daemon_identity_material_regenerates_on_cert_key_spki_mismatch() {
        // Write a valid key.pem from keypair A but a valid cert.pem from keypair B.
        // The pair is well-formed individually but the SPKI hashes do not match, so
        // a stable-pin trust decision cannot be made and the loader must regenerate.
        let dir = unique_identity_dir("spki-mismatch");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create dir");

        let material_a = generate_daemon_identity_material();
        let material_b = generate_daemon_identity_material();
        assert_ne!(
            material_a.identity_id, material_b.identity_id,
            "test fixture requires distinct keypairs"
        );
        std::fs::write(dir.join("key.pem"), &material_a.key_pem).expect("write key");
        std::fs::write(dir.join("cert.pem"), &material_b.cert_pem).expect("write cert");

        let loaded = load_or_create_daemon_identity_material_at(&dir);
        assert_ne!(
            loaded.identity_id, material_a.identity_id,
            "mismatched pair must not be accepted as identity A"
        );
        assert_ne!(
            loaded.identity_id, material_b.identity_id,
            "mismatched pair must not be accepted as identity B"
        );

        // Sanity: the newly-written pair must now validate on reload.
        let reload = load_or_create_daemon_identity_material_at(&dir);
        assert_eq!(reload.identity_id, loaded.identity_id);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn daemon_identity_material_regenerates_when_key_is_missing() {
        let dir = unique_identity_dir("regen");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create dir");
        // Only a cert present, key missing — must regenerate both.
        std::fs::write(dir.join("cert.pem"), "not a real cert").expect("write stale cert");

        let material = load_or_create_daemon_identity_material_at(&dir);

        assert!(!material.identity_id.is_empty());
        assert!(
            rcgen::KeyPair::from_pem(&material.key_pem).is_ok(),
            "regenerated key must parse",
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn encode_delta_uses_scroll_delta_for_scrolling_output() {
        let row = |ch: char| -> Vec<RemoteCell> {
            vec![RemoteCell {
                codepoint: u32::from(ch),
                fg: [1, 2, 3],
                bg: [0, 0, 0],
                style_flags: 0,
                wide: false,
            }]
        };
        let previous = RemoteFullState {
            rows: 3,
            cols: 1,
            cursor_x: 0,
            cursor_y: 2,
            cursor_visible: true,
            cursor_blinking: false,
            cursor_style: 1,
            cells: [row('a'), row('b'), row('c')].concat(),
        };
        let current = RemoteFullState {
            rows: 3,
            cols: 1,
            cursor_x: 0,
            cursor_y: 2,
            cursor_visible: true,
            cursor_blinking: false,
            cursor_style: 1,
            cells: [row('b'), row('c'), row('d')].concat(),
        };

        let payload = encode_delta(&previous, &current, Some(7), false).expect("delta payload");
        assert_eq!(u16::from_le_bytes(payload[0..2].try_into().unwrap()), 1);
        assert_eq!(payload[8] & 0x01, 0x01);
        assert_eq!(
            i16::from_le_bytes(
                payload[REMOTE_DELTA_HEADER_LEN..REMOTE_DELTA_HEADER_LEN + 2]
                    .try_into()
                    .unwrap()
            ),
            1
        );
        assert_eq!(
            u16::from_le_bytes(
                payload[REMOTE_DELTA_HEADER_LEN + 2..REMOTE_DELTA_HEADER_LEN + 4]
                    .try_into()
                    .unwrap()
            ),
            2
        );
    }

    #[test]
    fn encode_delta_skips_scroll_optimization_for_local_clients() {
        let row = |ch: char| -> Vec<RemoteCell> {
            vec![RemoteCell {
                codepoint: u32::from(ch),
                fg: [1, 2, 3],
                bg: [0, 0, 0],
                style_flags: 0,
                wide: false,
            }]
        };
        let previous = RemoteFullState {
            rows: 4,
            cols: 1,
            cursor_x: 0,
            cursor_y: 3,
            cursor_visible: true,
            cursor_blinking: false,
            cursor_style: 1,
            cells: [row('a'), row('b'), row('c'), row('d')].concat(),
        };
        let current = RemoteFullState {
            rows: 4,
            cols: 1,
            cursor_x: 0,
            cursor_y: 3,
            cursor_visible: true,
            cursor_blinking: false,
            cursor_style: 1,
            cells: [row('b'), row('c'), row('d'), row('e')].concat(),
        };

        assert!(encode_delta(&previous, &current, Some(9), true).is_none());
    }

    #[test]
    fn encode_delta_trims_unchanged_prefix_and_suffix_within_row() {
        let cell = |ch: char| RemoteCell {
            codepoint: u32::from(ch),
            fg: [1, 2, 3],
            bg: [0, 0, 0],
            style_flags: 0,
            wide: false,
        };
        let previous = RemoteFullState {
            rows: 1,
            cols: 5,
            cursor_x: 2,
            cursor_y: 0,
            cursor_visible: true,
            cursor_blinking: false,
            cursor_style: 1,
            cells: vec![cell('a'), cell('b'), cell('c'), cell('d'), cell('e')],
        };
        let current = RemoteFullState {
            rows: 1,
            cols: 5,
            cursor_x: 2,
            cursor_y: 0,
            cursor_visible: true,
            cursor_blinking: false,
            cursor_style: 1,
            cells: vec![cell('a'), cell('b'), cell('X'), cell('d'), cell('e')],
        };

        let payload = encode_delta(&previous, &current, Some(5), true).expect("delta payload");
        let row_offset = LOCAL_DELTA_HEADER_LEN;
        assert_eq!(
            u16::from_le_bytes(payload[row_offset..row_offset + 2].try_into().unwrap()),
            0
        );
        assert_eq!(
            u16::from_le_bytes(payload[row_offset + 2..row_offset + 4].try_into().unwrap()),
            2
        );
        assert_eq!(
            u16::from_le_bytes(payload[row_offset + 4..row_offset + 6].try_into().unwrap()),
            1
        );
        assert_eq!(
            u32::from_le_bytes(payload[row_offset + 6..row_offset + 10].try_into().unwrap()),
            u32::from('X')
        );
    }

    #[test]
    fn longest_prefix_suffix_overlap_matches_scroll_overlap() {
        assert_eq!(longest_prefix_suffix_overlap(&[2, 3, 4], &[1, 2, 3, 4]), 3);
        assert_eq!(longest_prefix_suffix_overlap(&[1, 2, 3], &[1, 2, 3, 4]), 0);
        assert_eq!(longest_prefix_suffix_overlap(&[7, 8], &[5, 6, 7, 8]), 2);
    }

    #[test]
    fn detect_scroll_rows_handles_multi_row_scroll() {
        let row = |ch: char| -> Vec<RemoteCell> {
            vec![RemoteCell {
                codepoint: u32::from(ch),
                fg: [1, 2, 3],
                bg: [0, 0, 0],
                style_flags: 0,
                wide: false,
            }]
        };
        let previous = RemoteFullState {
            rows: 5,
            cols: 1,
            cursor_x: 0,
            cursor_y: 4,
            cursor_visible: true,
            cursor_blinking: false,
            cursor_style: 1,
            cells: [row('a'), row('b'), row('c'), row('d'), row('e')].concat(),
        };
        let current = RemoteFullState {
            rows: 5,
            cols: 1,
            cursor_x: 0,
            cursor_y: 4,
            cursor_visible: true,
            cursor_blinking: false,
            cursor_style: 1,
            cells: [row('c'), row('d'), row('e'), row('f'), row('g')].concat(),
        };

        assert_eq!(detect_scroll_rows(&previous, &current), Some(2));
    }

    #[test]
    fn send_attached_to_same_session_preserves_stream_state() {
        let (outbound, outbound_rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(State {
            clients: HashMap::from([(
                7,
                ClientState {
                    outbound,
                    authenticated: true,
                    challenge: None,
                    connected_at: Instant::now(),
                    authenticated_at: Some(Instant::now()),
                    last_heartbeat_at: None,
                    attached_session: Some(11),
                    attachment_id: None,
                    resume_token: None,
                    last_session_list_payload: None,
                    last_ui_runtime_state_payload: None,
                    last_ui_appearance_payload: None,
                    last_state: Some(Arc::new(RemoteFullState {
                        rows: 1,
                        cols: 1,
                        cursor_x: 0,
                        cursor_y: 0,
                        cursor_visible: true,
                        cursor_blinking: false,
                        cursor_style: 1,
                        cells: vec![RemoteCell {
                            codepoint: u32::from('A'),
                            fg: [1, 2, 3],
                            bg: [0, 0, 0],
                            style_flags: 0,
                            wide: false,
                        }],
                    })),
                    pane_states: HashMap::new(),
                    latest_input_seq: Some(42),
                    is_local: true,
                },
            )]),
            revivable_attachments: HashMap::new(),
            auth_key: None,
            server_identity_id: "test-daemon".to_string(),
            server_instance_id: "test-instance".to_string(),
            tls_clients: std::collections::HashSet::new(),
        }));
        let server = RemoteServer {
            state: Arc::clone(&state),
            _listener: std::thread::spawn(|| {}),
            _advertiser: None,
            local_socket_path: None,
            bind_address: None,
            port: None,
            _quic_listener: None,
        };

        server.send_attached(7, 11, None);

        let guard = state.lock().expect("remote server state poisoned");
        let client = guard.clients.get(&7).expect("client state");
        assert_eq!(client.attached_session, Some(11));
        assert_eq!(client.latest_input_seq, Some(42));
        assert!(client.last_state.is_some());
        drop(guard);

        match outbound_rx.recv().expect("attached frame") {
            OutboundMessage::Frame(frame) => {
                assert_eq!(&frame[..2], &MAGIC);
                assert_eq!(frame[2], MessageType::Attached as u8);
            }
            OutboundMessage::ScreenUpdate(_) => panic!("unexpected screen update"),
        }
    }

    #[test]
    fn send_attached_for_remote_client_includes_resume_token() {
        let (outbound, outbound_rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(State {
            clients: HashMap::from([(
                7,
                ClientState {
                    outbound,
                    authenticated: true,
                    challenge: None,
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
            )]),
            revivable_attachments: HashMap::new(),
            auth_key: None,
            server_identity_id: "test-daemon".to_string(),
            server_instance_id: "test-instance".to_string(),
            tls_clients: std::collections::HashSet::new(),
        }));
        let server = RemoteServer {
            state: Arc::clone(&state),
            _listener: std::thread::spawn(|| {}),
            _advertiser: None,
            local_socket_path: None,
            bind_address: None,
            port: None,
            _quic_listener: None,
        };

        server.send_attached(7, 11, Some(0xabc));

        let guard = state.lock().expect("remote server state poisoned");
        let client = guard.clients.get(&7).expect("client state");
        let resume_token = client.resume_token.expect("resume token");
        assert_ne!(resume_token, 0);
        drop(guard);

        match outbound_rx.recv().expect("attached frame") {
            OutboundMessage::Frame(frame) => {
                let mut cursor = std::io::Cursor::new(frame);
                let (ty, payload) = read_message(&mut cursor).expect("attached frame decode");
                assert_eq!(ty, MessageType::Attached);
                assert_eq!(payload.len(), 20);
                assert_eq!(u32::from_le_bytes(payload[0..4].try_into().unwrap()), 11);
                assert_eq!(u64::from_le_bytes(payload[4..12].try_into().unwrap()), 0xabc);
                assert_eq!(u64::from_le_bytes(payload[12..20].try_into().unwrap()), resume_token);
            }
            OutboundMessage::ScreenUpdate(_) => panic!("unexpected screen update"),
        }
    }

    #[test]
    fn prepare_attachment_restores_revived_state_for_matching_identity() {
        let (tx, _rx) = mpsc::channel();
        let restored_state = Arc::new(RemoteFullState {
            rows: 1,
            cols: 1,
            cursor_x: 0,
            cursor_y: 0,
            cursor_visible: true,
            cursor_blinking: false,
            cursor_style: 1,
            cells: vec![RemoteCell {
                codepoint: u32::from('R'),
                fg: [1, 2, 3],
                bg: [0, 0, 0],
                style_flags: 0,
                wide: false,
            }],
        });
        let state = Arc::new(Mutex::new(State {
            clients: HashMap::from([(
                1,
                ClientState {
                    outbound: tx,
                    authenticated: true,
                    challenge: None,
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
            )]),
            revivable_attachments: HashMap::from([(
                0xabc,
                RevivableAttachment {
                    session_id: 11,
                    resume_token: 0xdef,
                    last_state: Some(Arc::clone(&restored_state)),
                    pane_states: HashMap::new(),
                    latest_input_seq: Some(9),
                    expires_at: Instant::now() + REVIVABLE_ATTACHMENT_WINDOW,
                },
            )]),
            auth_key: None,
            server_identity_id: "test-daemon".to_string(),
            server_instance_id: "test-instance".to_string(),
            tls_clients: std::collections::HashSet::new(),
        }));
        let server = RemoteServer {
            state: Arc::clone(&state),
            _listener: std::thread::spawn(|| {}),
            _advertiser: None,
            local_socket_path: None,
            bind_address: None,
            port: None,
            _quic_listener: None,
        };

        server
            .prepare_attachment(1, 11, Some(0xabc), Some(0xdef))
            .expect("prepare attachment");

        let guard = state.lock().expect("remote server state poisoned");
        let client = guard.clients.get(&1).expect("client state");
        assert_eq!(client.attached_session, Some(11));
        assert_eq!(client.attachment_id, Some(0xabc));
        assert_eq!(client.resume_token, Some(0xdef));
        assert_eq!(client.latest_input_seq, Some(9));
        assert_eq!(client.last_state.as_deref(), Some(restored_state.as_ref()));
        assert!(!guard.revivable_attachments.contains_key(&0xabc));
    }

    #[test]
    fn prepare_attachment_rejects_duplicate_active_attachment() {
        let (active_tx, _active_rx) = mpsc::channel();
        let (new_tx, _new_rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(State {
            clients: HashMap::from([
                (
                    1,
                    ClientState {
                        outbound: active_tx,
                        authenticated: true,
                        challenge: None,
                        connected_at: Instant::now(),
                        authenticated_at: Some(Instant::now()),
                        last_heartbeat_at: None,
                        attached_session: Some(11),
                        attachment_id: Some(0xabc),
                        resume_token: None,
                        last_session_list_payload: None,
                        last_ui_runtime_state_payload: None,
                        last_ui_appearance_payload: None,
                        last_state: None,
                        pane_states: HashMap::new(),
                        latest_input_seq: None,
                        is_local: false,
                    },
                ),
                (
                    2,
                    ClientState {
                        outbound: new_tx,
                        authenticated: true,
                        challenge: None,
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
                ),
            ]),
            revivable_attachments: HashMap::new(),
            auth_key: None,
            server_identity_id: "test-daemon".to_string(),
            server_instance_id: "test-instance".to_string(),
            tls_clients: std::collections::HashSet::new(),
        }));
        let server = RemoteServer {
            state,
            _listener: std::thread::spawn(|| {}),
            _advertiser: None,
            local_socket_path: None,
            bind_address: None,
            port: None,
            _quic_listener: None,
        };

        let error = server
            .prepare_attachment(2, 11, Some(0xabc), Some(0xdef))
            .expect_err("duplicate active attachment should fail");
        assert_eq!(error, "attachment already active");
    }

    #[test]
    fn prepare_attachment_rejects_wrong_resume_token() {
        let (tx, _rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(State {
            clients: HashMap::from([(
                1,
                ClientState {
                    outbound: tx,
                    authenticated: true,
                    challenge: None,
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
            )]),
            revivable_attachments: HashMap::from([(
                0xabc,
                RevivableAttachment {
                    session_id: 11,
                    resume_token: 0xdef,
                    last_state: None,
                    pane_states: HashMap::new(),
                    latest_input_seq: None,
                    expires_at: Instant::now() + REVIVABLE_ATTACHMENT_WINDOW,
                },
            )]),
            auth_key: None,
            server_identity_id: "test-daemon".to_string(),
            server_instance_id: "test-instance".to_string(),
            tls_clients: std::collections::HashSet::new(),
        }));
        let server = RemoteServer {
            state: Arc::clone(&state),
            _listener: std::thread::spawn(|| {}),
            _advertiser: None,
            local_socket_path: None,
            bind_address: None,
            port: None,
            _quic_listener: None,
        };

        let error = server
            .prepare_attachment(1, 11, Some(0xabc), Some(0x123))
            .expect_err("wrong resume token should fail");
        assert_eq!(error, "attachment resume token mismatch");
        assert!(
            state
                .lock()
                .expect("remote server state poisoned")
                .revivable_attachments
                .contains_key(&0xabc)
        );
    }

    #[test]
    fn prepare_attachment_rejects_expired_resume_attempt() {
        let (tx, _rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(State {
            clients: HashMap::from([(
                1,
                ClientState {
                    outbound: tx,
                    authenticated: true,
                    challenge: None,
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
            )]),
            revivable_attachments: HashMap::new(),
            auth_key: None,
            server_identity_id: "test-daemon".to_string(),
            server_instance_id: "test-instance".to_string(),
            tls_clients: std::collections::HashSet::new(),
        }));
        let server = RemoteServer {
            state: Arc::clone(&state),
            _listener: std::thread::spawn(|| {}),
            _advertiser: None,
            local_socket_path: None,
            bind_address: None,
            port: None,
            _quic_listener: None,
        };

        let error = server
            .prepare_attachment(1, 11, Some(0xabc), Some(0xdef))
            .expect_err("expired resume attempt should fail");
        assert_eq!(error, "attachment resume window expired");

        let guard = state.lock().expect("remote server state poisoned");
        let client = guard.clients.get(&1).expect("client state");
        assert!(client.attached_session.is_none());
        assert!(client.attachment_id.is_none());
        assert!(client.resume_token.is_none());
    }

    #[test]
    fn prepare_attachment_allows_new_attach_without_resume_token() {
        let (tx, _rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(State {
            clients: HashMap::from([(
                1,
                ClientState {
                    outbound: tx,
                    authenticated: true,
                    challenge: None,
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
            )]),
            revivable_attachments: HashMap::new(),
            auth_key: None,
            server_identity_id: "test-daemon".to_string(),
            server_instance_id: "test-instance".to_string(),
            tls_clients: std::collections::HashSet::new(),
        }));
        let server = RemoteServer {
            state: Arc::clone(&state),
            _listener: std::thread::spawn(|| {}),
            _advertiser: None,
            local_socket_path: None,
            bind_address: None,
            port: None,
            _quic_listener: None,
        };

        server
            .prepare_attachment(1, 11, Some(0xabc), None)
            .expect("attach without resume token should succeed");
    }

    #[test]
    fn client_info_reports_remote_client_diagnostics() {
        let (tx, _rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(State {
                clients: HashMap::from([(
                    7,
                    ClientState {
                        outbound: tx,
                        authenticated: true,
                        challenge: None,
                        connected_at: Instant::now(),
                        authenticated_at: Some(Instant::now()),
                        last_heartbeat_at: Some(Instant::now()),
                        attached_session: Some(11),
                        attachment_id: Some(0xabc),
                        resume_token: Some(0xdef),
                    last_session_list_payload: None,
                    last_ui_runtime_state_payload: None,
                    last_ui_appearance_payload: None,
                    last_state: Some(Arc::new(RemoteFullState {
                        rows: 1,
                        cols: 1,
                        cursor_x: 0,
                        cursor_y: 0,
                        cursor_visible: true,
                        cursor_blinking: false,
                        cursor_style: 1,
                        cells: vec![RemoteCell {
                            codepoint: u32::from('x'),
                            fg: [1, 2, 3],
                            bg: [0, 0, 0],
                            style_flags: 0,
                            wide: false,
                        }],
                    })),
                    pane_states: HashMap::from([(
                        22,
                        Arc::new(RemoteFullState {
                            rows: 1,
                            cols: 1,
                            cursor_x: 0,
                            cursor_y: 0,
                            cursor_visible: true,
                            cursor_blinking: false,
                            cursor_style: 1,
                            cells: vec![],
                        }),
                    )]),
                    latest_input_seq: Some(9),
                    is_local: false,
                },
            )]),
            revivable_attachments: HashMap::new(),
            auth_key: None,
            server_identity_id: "test-daemon".to_string(),
            server_instance_id: "test-instance".to_string(),
            tls_clients: std::collections::HashSet::new(),
        }));
        let server = RemoteServer {
            state,
            _listener: std::thread::spawn(|| {}),
            _advertiser: None,
            local_socket_path: None,
            bind_address: None,
            port: None,
            _quic_listener: None,
        };

        let snapshot = server.clients_snapshot();
        assert_eq!(snapshot.servers.len(), 1);
        let server_info = &snapshot.servers[0];
        assert_eq!(server_info.protocol_version, REMOTE_PROTOCOL_VERSION);
        assert_eq!(server_info.capabilities, REMOTE_CAPABILITIES);
        assert_eq!(server_info.build_id, env!("CARGO_PKG_VERSION"));
        assert_eq!(server_info.server_instance_id, "test-instance");
        assert_eq!(server_info.server_identity_id, "test-daemon");
        assert!(!server_info.auth_required);
        assert_eq!(
            server_info.heartbeat_window_ms,
            DIRECT_CLIENT_HEARTBEAT_WINDOW.as_millis() as u64
        );
        assert_eq!(server_info.connected_clients, 1);
        assert_eq!(server_info.attached_clients, 1);
        assert_eq!(server_info.pending_auth_clients, 0);
        assert_eq!(server_info.revivable_attachments, 0);
        assert!(snapshot.revivable_attachments.is_empty());
        assert_eq!(snapshot.clients.len(), 1);
        let client = &snapshot.clients[0];
        assert_eq!(client.client_id, 7);
        assert!(client.authenticated);
        assert!(!client.is_local);
        assert_eq!(client.transport_kind, "tcp");
        assert_eq!(client.server_socket_path, None);
        assert!(!client.challenge_pending);
        assert_eq!(client.attached_session, Some(11));
        assert_eq!(client.attachment_id, Some(0xabc));
        assert!(client.resume_token_present);
        assert!(client.has_cached_state);
        assert_eq!(client.pane_state_count, 1);
        assert_eq!(client.latest_input_seq, Some(9));
        assert!(client.connection_age_ms <= 250);
        assert!(client.authenticated_age_ms.is_some_and(|age| age <= 250));
        assert!(client.last_heartbeat_age_ms.is_some_and(|age| age <= 250));
        assert!(client.heartbeat_expires_in_ms.is_some_and(
            |ms| ms <= DIRECT_CLIENT_HEARTBEAT_WINDOW.as_millis() as u64
        ));
        assert!(!client.heartbeat_overdue);
        assert_eq!(client.challenge_expires_in_ms, None);
    }

    #[test]
    fn clients_snapshot_includes_revivable_attachments() {
        let state = Arc::new(Mutex::new(State {
            clients: HashMap::new(),
            revivable_attachments: HashMap::from([(
                0xabc,
                RevivableAttachment {
                    session_id: 11,
                    resume_token: 0xdef,
                    last_state: Some(Arc::new(RemoteFullState {
                        rows: 1,
                        cols: 1,
                        cursor_x: 0,
                        cursor_y: 0,
                        cursor_visible: true,
                        cursor_blinking: false,
                        cursor_style: 1,
                        cells: vec![],
                    })),
                    pane_states: HashMap::from([(
                        22,
                        Arc::new(RemoteFullState {
                            rows: 1,
                            cols: 1,
                            cursor_x: 0,
                            cursor_y: 0,
                            cursor_visible: true,
                            cursor_blinking: false,
                            cursor_style: 1,
                            cells: vec![],
                        }),
                    )]),
                    latest_input_seq: Some(9),
                    expires_at: Instant::now() + REVIVABLE_ATTACHMENT_WINDOW,
                },
            )]),
            auth_key: None,
            server_identity_id: "test-daemon".to_string(),
            server_instance_id: "test-instance".to_string(),
            tls_clients: std::collections::HashSet::new(),
        }));
        let server = RemoteServer {
            state,
            _listener: std::thread::spawn(|| {}),
            _advertiser: None,
            local_socket_path: None,
            bind_address: None,
            port: None,
            _quic_listener: None,
        };

        let snapshot = server.clients_snapshot();
        assert_eq!(snapshot.servers.len(), 1);
        assert_eq!(snapshot.servers[0].connected_clients, 0);
        assert_eq!(snapshot.servers[0].attached_clients, 0);
        assert_eq!(snapshot.servers[0].pending_auth_clients, 0);
        assert_eq!(snapshot.servers[0].revivable_attachments, 1);
        assert!(snapshot.clients.is_empty());
        assert_eq!(snapshot.revivable_attachments.len(), 1);
        let attachment = &snapshot.revivable_attachments[0];
        assert_eq!(attachment.attachment_id, 0xabc);
        assert_eq!(attachment.session_id, 11);
        assert!(attachment.resume_token_present);
        assert!(attachment.has_cached_state);
        assert_eq!(attachment.pane_state_count, 1);
        assert_eq!(attachment.latest_input_seq, Some(9));
        assert!(attachment.revive_expires_in_ms <= REVIVABLE_ATTACHMENT_WINDOW.as_millis() as u64);
        assert!(attachment.revive_expires_in_ms > 0);
    }

    #[test]
    fn clients_snapshot_reports_challenge_and_heartbeat_diagnostics() {
        let (tx, _rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(State {
            clients: HashMap::from([(
                1,
                ClientState {
                    outbound: tx,
                    authenticated: false,
                    challenge: Some(AuthChallengeState {
                        bytes: [7; 32],
                        expires_at: Instant::now() + Duration::from_secs(5),
                    }),
                    connected_at: Instant::now() - Duration::from_secs(2),
                    authenticated_at: None,
                    last_heartbeat_at: Some(Instant::now() - Duration::from_millis(750)),
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
            )]),
            revivable_attachments: HashMap::new(),
            auth_key: Some(b"test-key".to_vec()),
            server_identity_id: "test-daemon".to_string(),
            server_instance_id: "test-instance".to_string(),
            tls_clients: std::collections::HashSet::new(),
        }));
        let server = RemoteServer {
            state,
            _listener: std::thread::spawn(|| {}),
            _advertiser: None,
            local_socket_path: None,
            bind_address: None,
            port: None,
            _quic_listener: None,
        };

        let snapshot = server.clients_snapshot();
        assert_eq!(snapshot.servers.len(), 1);
        assert!(snapshot.servers[0].auth_required);
        assert_eq!(
            snapshot.servers[0].auth_challenge_window_ms,
            AUTH_CHALLENGE_WINDOW.as_millis() as u64
        );
        assert_eq!(snapshot.servers[0].connected_clients, 1);
        assert_eq!(snapshot.servers[0].attached_clients, 0);
        assert_eq!(snapshot.servers[0].pending_auth_clients, 1);
        assert_eq!(snapshot.servers[0].revivable_attachments, 0);
        assert_eq!(snapshot.clients.len(), 1);
        let client = &snapshot.clients[0];
        assert!(!client.authenticated);
        assert_eq!(client.transport_kind, "tcp");
        assert_eq!(client.server_socket_path, None);
        assert!(client.challenge_pending);
        assert!(client.connection_age_ms >= 2_000);
        assert_eq!(client.authenticated_age_ms, None);
        assert!(client.last_heartbeat_age_ms.is_some_and(|age| age >= 750));
        assert_eq!(client.heartbeat_expires_in_ms, None);
        assert!(!client.heartbeat_overdue);
        assert!(client.challenge_expires_in_ms.is_some_and(|age| age > 0));
    }

    #[test]
    fn clients_snapshot_reports_overdue_direct_heartbeat() {
        let (tx, _rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(State {
            clients: HashMap::from([(
                1,
                ClientState {
                    outbound: tx,
                    authenticated: true,
                    challenge: None,
                    connected_at: Instant::now() - Duration::from_secs(30),
                    authenticated_at: Some(
                        Instant::now() - DIRECT_CLIENT_HEARTBEAT_WINDOW - Duration::from_secs(1),
                    ),
                    last_heartbeat_at: Some(
                        Instant::now() - DIRECT_CLIENT_HEARTBEAT_WINDOW - Duration::from_secs(1),
                    ),
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
            )]),
            revivable_attachments: HashMap::new(),
            auth_key: None,
            server_identity_id: "test-daemon".to_string(),
            server_instance_id: "test-instance".to_string(),
            tls_clients: std::collections::HashSet::new(),
        }));
        let server = RemoteServer {
            state,
            _listener: std::thread::spawn(|| {}),
            _advertiser: None,
            local_socket_path: None,
            bind_address: None,
            port: None,
            _quic_listener: None,
        };

        let snapshot = server.clients_snapshot();
        assert_eq!(snapshot.servers.len(), 1);
        assert_eq!(snapshot.servers[0].connected_clients, 1);
        assert_eq!(snapshot.servers[0].attached_clients, 0);
        assert_eq!(snapshot.servers[0].pending_auth_clients, 0);
        assert_eq!(snapshot.servers[0].revivable_attachments, 0);
        assert_eq!(snapshot.clients.len(), 1);
        let client = &snapshot.clients[0];
        assert_eq!(client.transport_kind, "tcp");
        assert_eq!(client.server_socket_path, None);
        assert!(client.heartbeat_overdue);
        assert_eq!(client.heartbeat_expires_in_ms, Some(0));
    }

    #[test]
    fn clients_snapshot_recovers_after_fresh_direct_heartbeat() {
        let (tx, _rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(State {
            clients: HashMap::from([(
                1,
                ClientState {
                    outbound: tx,
                    authenticated: true,
                    challenge: None,
                    connected_at: Instant::now() - Duration::from_secs(30),
                    authenticated_at: Some(
                        Instant::now() - DIRECT_CLIENT_HEARTBEAT_WINDOW - Duration::from_secs(1),
                    ),
                    last_heartbeat_at: Some(
                        Instant::now() - DIRECT_CLIENT_HEARTBEAT_WINDOW - Duration::from_secs(1),
                    ),
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
            )]),
            revivable_attachments: HashMap::new(),
            auth_key: None,
            server_identity_id: "test-daemon".to_string(),
            server_instance_id: "test-instance".to_string(),
            tls_clients: std::collections::HashSet::new(),
        }));
        let server = RemoteServer {
            state: Arc::clone(&state),
            _listener: std::thread::spawn(|| {}),
            _advertiser: None,
            local_socket_path: None,
            bind_address: None,
            port: None,
            _quic_listener: None,
        };

        let stale_snapshot = server.clients_snapshot();
        assert!(stale_snapshot.clients[0].heartbeat_overdue);

        {
            let mut guard = state.lock().expect("remote server state poisoned");
            guard.clients.get_mut(&1).expect("client state").last_heartbeat_at = Some(Instant::now());
        }

        let recovered_snapshot = server.clients_snapshot();
        let client = &recovered_snapshot.clients[0];
        assert!(!client.heartbeat_overdue);
        assert!(client.last_heartbeat_age_ms.is_some_and(|age| age <= 250));
        assert!(
            client.heartbeat_expires_in_ms.is_some_and(
                |ms| ms > 0 && ms <= DIRECT_CLIENT_HEARTBEAT_WINDOW.as_millis() as u64
            )
        );
    }

    #[test]
    fn send_ui_runtime_state_to_local_attached_only_targets_matching_session() {
        let (attached_tx, attached_rx) = mpsc::channel();
        let (unattached_tx, unattached_rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(State {
            clients: HashMap::from([
                (
                    1,
                ClientState {
                    outbound: attached_tx,
                    authenticated: true,
                    challenge: None,
                    connected_at: Instant::now(),
                    authenticated_at: Some(Instant::now()),
                    last_heartbeat_at: None,
                    attached_session: Some(11),
                    attachment_id: None,
                    resume_token: None,
                        last_session_list_payload: None,
                        last_ui_runtime_state_payload: None,
                        last_ui_appearance_payload: None,
                        last_state: None,
                        pane_states: HashMap::new(),
                        latest_input_seq: None,
                        is_local: true,
                    },
                ),
                (
                    2,
                ClientState {
                    outbound: unattached_tx,
                    authenticated: true,
                    challenge: None,
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
                        is_local: true,
                    },
                ),
            ]),
            revivable_attachments: HashMap::new(),
            auth_key: None,
            server_identity_id: "test-daemon".to_string(),
            server_instance_id: "test-instance".to_string(),
            tls_clients: std::collections::HashSet::new(),
        }));
        let server = RemoteServer {
            state,
            _listener: std::thread::spawn(|| {}),
            _advertiser: None,
            local_socket_path: None,
            bind_address: None,
            port: None,
            _quic_listener: None,
        };
        let ui_state = control::UiRuntimeState {
            active_tab: 0,
            focused_pane: 7,
            tabs: Vec::new(),
            visible_panes: Vec::new(),
            mouse_selection: control::UiMouseSelectionSnapshot::default(),
            status_bar: crate::status_components::UiStatusBarSnapshot::default(),
            pwd: "/tmp".to_string(),
        };

        server.send_ui_runtime_state_to_local_attached(11, &ui_state);

        match attached_rx.recv().expect("attached frame") {
            OutboundMessage::Frame(frame) => {
                assert_eq!(&frame[..2], &MAGIC);
                assert_eq!(frame[2], MessageType::UiRuntimeState as u8);
            }
            OutboundMessage::ScreenUpdate(_) => panic!("unexpected screen update"),
        }
        assert!(unattached_rx.try_recv().is_err());
    }

    #[test]
    fn send_ui_runtime_state_skips_unchanged_payloads() {
        let (tx, rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(State {
            clients: HashMap::from([(
                1,
                ClientState {
                    outbound: tx,
                    authenticated: true,
                    challenge: None,
                    connected_at: Instant::now(),
                    authenticated_at: Some(Instant::now()),
                    last_heartbeat_at: None,
                    attached_session: Some(11),
                    attachment_id: None,
                    resume_token: None,
                    last_session_list_payload: None,
                    last_ui_runtime_state_payload: None,
                    last_ui_appearance_payload: None,
                    last_state: None,
                    pane_states: HashMap::new(),
                    latest_input_seq: None,
                    is_local: true,
                },
            )]),
            revivable_attachments: HashMap::new(),
            auth_key: None,
            server_identity_id: "test-daemon".to_string(),
            server_instance_id: "test-instance".to_string(),
            tls_clients: std::collections::HashSet::new(),
        }));
        let server = RemoteServer {
            state,
            _listener: std::thread::spawn(|| {}),
            _advertiser: None,
            local_socket_path: None,
            bind_address: None,
            port: None,
            _quic_listener: None,
        };
        let ui_state = control::UiRuntimeState {
            active_tab: 0,
            focused_pane: 7,
            tabs: Vec::new(),
            visible_panes: Vec::new(),
            mouse_selection: control::UiMouseSelectionSnapshot::default(),
            status_bar: crate::status_components::UiStatusBarSnapshot::default(),
            pwd: "/tmp".to_string(),
        };

        server.send_ui_runtime_state(1, &ui_state);
        server.send_ui_runtime_state(1, &ui_state);

        match rx.recv().expect("runtime state frame") {
            OutboundMessage::Frame(frame) => {
                assert_eq!(&frame[..2], &MAGIC);
                assert_eq!(frame[2], MessageType::UiRuntimeState as u8);
            }
            OutboundMessage::ScreenUpdate(_) => panic!("unexpected screen update"),
        }
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn send_session_list_skips_unchanged_payloads() {
        let (tx, rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(State {
            clients: HashMap::from([(
                1,
                ClientState {
                    outbound: tx,
                    authenticated: true,
                    challenge: None,
                    connected_at: Instant::now(),
                    authenticated_at: Some(Instant::now()),
                    last_heartbeat_at: None,
                    attached_session: Some(11),
                    attachment_id: None,
                    resume_token: None,
                    last_session_list_payload: None,
                    last_ui_runtime_state_payload: None,
                    last_ui_appearance_payload: None,
                    last_state: None,
                    pane_states: HashMap::new(),
                    latest_input_seq: None,
                    is_local: true,
                },
            )]),
            revivable_attachments: HashMap::new(),
            auth_key: None,
            server_identity_id: "test-daemon".to_string(),
            server_instance_id: "test-instance".to_string(),
            tls_clients: std::collections::HashSet::new(),
        }));
        let server = RemoteServer {
            state,
            _listener: std::thread::spawn(|| {}),
            _advertiser: None,
            local_socket_path: None,
            bind_address: None,
            port: None,
            _quic_listener: None,
        };
        let sessions = vec![RemoteSessionInfo {
            id: 11,
            name: "Tab 1".to_string(),
            title: "boo".to_string(),
            pwd: "/tmp".to_string(),
            attached: true,
            child_exited: false,
        }];

        server.send_session_list(1, &sessions);
        server.send_session_list(1, &sessions);

        match rx.recv().expect("session list frame") {
            OutboundMessage::Frame(frame) => {
                assert_eq!(&frame[..2], &MAGIC);
                assert_eq!(frame[2], MessageType::SessionList as u8);
            }
            OutboundMessage::ScreenUpdate(_) => panic!("unexpected screen update"),
        }
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn reply_session_list_does_not_skip_unchanged_payloads() {
        let (tx, rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(State {
            clients: HashMap::from([(
                1,
                ClientState {
                    outbound: tx,
                    authenticated: true,
                    challenge: None,
                    connected_at: Instant::now(),
                    authenticated_at: Some(Instant::now()),
                    last_heartbeat_at: None,
                    attached_session: Some(11),
                    attachment_id: None,
                    resume_token: None,
                    last_session_list_payload: None,
                    last_ui_runtime_state_payload: None,
                    last_ui_appearance_payload: None,
                    last_state: None,
                    pane_states: HashMap::new(),
                    latest_input_seq: None,
                    is_local: true,
                },
            )]),
            revivable_attachments: HashMap::new(),
            auth_key: None,
            server_identity_id: "test-daemon".to_string(),
            server_instance_id: "test-instance".to_string(),
            tls_clients: std::collections::HashSet::new(),
        }));
        let server = RemoteServer {
            state,
            _listener: std::thread::spawn(|| {}),
            _advertiser: None,
            local_socket_path: None,
            bind_address: None,
            port: None,
            _quic_listener: None,
        };
        let sessions = vec![RemoteSessionInfo {
            id: 11,
            name: "Tab 1".to_string(),
            title: "boo".to_string(),
            pwd: "/tmp".to_string(),
            attached: true,
            child_exited: false,
        }];

        server.reply_session_list(1, &sessions);
        server.reply_session_list(1, &sessions);

        for _ in 0..2 {
            match rx.recv().expect("session list frame") {
                OutboundMessage::Frame(frame) => {
                    assert_eq!(&frame[..2], &MAGIC);
                    assert_eq!(frame[2], MessageType::SessionList as u8);
                }
                OutboundMessage::ScreenUpdate(_) => panic!("unexpected screen update"),
            }
        }
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn has_client_is_true_before_attach() {
        let (tx, _rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(State {
            clients: HashMap::from([(
                1,
                ClientState {
                    outbound: tx,
                    authenticated: true,
                    challenge: None,
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
            )]),
            revivable_attachments: HashMap::new(),
            auth_key: None,
            server_identity_id: "test-daemon".to_string(),
            server_instance_id: "test-instance".to_string(),
            tls_clients: std::collections::HashSet::new(),
        }));
        let server = RemoteServer {
            state,
            _listener: std::thread::spawn(|| {}),
            _advertiser: None,
            local_socket_path: None,
            bind_address: None,
            port: None,
            _quic_listener: None,
        };

        assert!(server.has_client(1));
        assert_eq!(server.client_session(1), None);
    }

    #[test]
    fn retarget_local_attached_to_session_skips_same_session_unattached_and_remote_clients() {
        let (local_attached_tx, local_attached_rx) = mpsc::channel();
        let (local_unattached_tx, local_unattached_rx) = mpsc::channel();
        let (local_same_session_tx, local_same_session_rx) = mpsc::channel();
        let (remote_attached_tx, remote_attached_rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(State {
            clients: HashMap::from([
                (
                    1,
                    ClientState {
                        outbound: local_attached_tx,
                        authenticated: true,
                        challenge: None,
                        connected_at: Instant::now(),
                        authenticated_at: Some(Instant::now()),
                        last_heartbeat_at: None,
                        attached_session: Some(11),
                        attachment_id: None,
                        resume_token: None,
                        last_session_list_payload: None,
                        last_ui_runtime_state_payload: None,
                        last_ui_appearance_payload: None,
                        last_state: None,
                        pane_states: HashMap::new(),
                        latest_input_seq: None,
                        is_local: true,
                    },
                ),
                (
                    2,
                    ClientState {
                        outbound: local_unattached_tx,
                        authenticated: true,
                        challenge: None,
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
                        is_local: true,
                    },
                ),
                (
                    3,
                    ClientState {
                        outbound: local_same_session_tx,
                        authenticated: true,
                        challenge: None,
                        connected_at: Instant::now(),
                        authenticated_at: Some(Instant::now()),
                        last_heartbeat_at: None,
                        attached_session: Some(22),
                        attachment_id: None,
                        resume_token: None,
                        last_session_list_payload: None,
                        last_ui_runtime_state_payload: None,
                        last_ui_appearance_payload: None,
                        last_state: None,
                        pane_states: HashMap::new(),
                        latest_input_seq: None,
                        is_local: true,
                    },
                ),
                (
                    4,
                    ClientState {
                        outbound: remote_attached_tx,
                        authenticated: true,
                        challenge: None,
                        connected_at: Instant::now(),
                        authenticated_at: Some(Instant::now()),
                        last_heartbeat_at: None,
                        attached_session: Some(11),
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
                ),
            ]),
            revivable_attachments: HashMap::new(),
            auth_key: None,
            server_identity_id: "test-daemon".to_string(),
            server_instance_id: "test-instance".to_string(),
            tls_clients: std::collections::HashSet::new(),
        }));
        let server = RemoteServer {
            state,
            _listener: std::thread::spawn(|| {}),
            _advertiser: None,
            local_socket_path: None,
            bind_address: None,
            port: None,
            _quic_listener: None,
        };

        server.retarget_local_attached_to_session(22);

        match local_attached_rx.recv().expect("local attached frame") {
            OutboundMessage::Frame(frame) => {
                assert_eq!(&frame[..2], &MAGIC);
                assert_eq!(frame[2], MessageType::Attached as u8);
            }
            OutboundMessage::ScreenUpdate(_) => panic!("unexpected screen update"),
        }
        assert!(local_unattached_rx.try_recv().is_err());
        assert!(local_same_session_rx.try_recv().is_err());
        assert!(remote_attached_rx.try_recv().is_err());
    }

    #[test]
    fn retain_local_attached_pane_states_prunes_invisible_panes() {
        let (tx, _rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(State {
            clients: HashMap::from([(
                1,
                ClientState {
                    outbound: tx,
                    authenticated: true,
                    challenge: None,
                    connected_at: Instant::now(),
                    authenticated_at: Some(Instant::now()),
                    last_heartbeat_at: None,
                    attached_session: Some(11),
                    attachment_id: None,
                    resume_token: None,
                    last_session_list_payload: None,
                    last_ui_runtime_state_payload: None,
                    last_ui_appearance_payload: None,
                    last_state: None,
                    pane_states: HashMap::from([
                        (
                            10,
                            Arc::new(RemoteFullState {
                                rows: 1,
                                cols: 1,
                                cursor_x: 0,
                                cursor_y: 0,
                                cursor_visible: true,
                                cursor_blinking: false,
                                cursor_style: 1,
                                cells: vec![RemoteCell {
                                    codepoint: u32::from('a'),
                                    fg: [1, 2, 3],
                                    bg: [0, 0, 0],
                                    style_flags: 0,
                                    wide: false,
                                }],
                            }),
                        ),
                        (
                            20,
                            Arc::new(RemoteFullState {
                                rows: 1,
                                cols: 1,
                                cursor_x: 0,
                                cursor_y: 0,
                                cursor_visible: true,
                                cursor_blinking: false,
                                cursor_style: 1,
                                cells: vec![RemoteCell {
                                    codepoint: u32::from('b'),
                                    fg: [1, 2, 3],
                                    bg: [0, 0, 0],
                                    style_flags: 0,
                                    wide: false,
                                }],
                            }),
                        ),
                    ]),
                    latest_input_seq: None,
                    is_local: true,
                },
            )]),
            revivable_attachments: HashMap::new(),
            auth_key: None,
            server_identity_id: "test-daemon".to_string(),
            server_instance_id: "test-instance".to_string(),
            tls_clients: std::collections::HashSet::new(),
        }));
        let server = RemoteServer {
            state: Arc::clone(&state),
            _listener: std::thread::spawn(|| {}),
            _advertiser: None,
            local_socket_path: None,
            bind_address: None,
            port: None,
            _quic_listener: None,
        };

        server.retain_local_attached_pane_states(11, &[20]);

        let guard = state.lock().expect("remote server state poisoned");
        let client = guard.clients.get(&1).expect("client state");
        assert!(!client.pane_states.contains_key(&10));
        assert!(client.pane_states.contains_key(&20));
    }
}
