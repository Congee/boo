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

use crate::remote_identity::{
    build_remote_server_tls_config, load_external_daemon_identity_material,
    load_or_create_daemon_identity, load_or_create_daemon_identity_material,
};
use crate::remote_transport::{
    PinnedQuicConnector, PinnedTlsConnector, PlainTcpConnector, QuicClientStream, TlsClientStream,
    connect_with,
};

// Re-export the public data types so existing callers of
// `crate::remote::RemoteProbeSummary` etc. keep working unchanged.
pub use crate::remote_types::{
    DirectTransportKind, RemoteAttachSummary, RemoteAttachedSummary, RemoteClientInfo,
    RemoteClientsSnapshot, RemoteCreateSummary, RemoteDirectSessionInfo, RemoteProbeSummary,
    RemoteServerInfo, RemoteSessionInfo, RemoteSessionListSummary, RemoteUpgradeProbeSummary,
    RevivableAttachmentInfo,
};

// Re-export the direct-client RPCs so existing callers of
// `crate::remote::probe_remote_endpoint` etc. keep working unchanged.
pub use crate::remote_client::{
    attach_remote_daemon_session, attach_remote_daemon_session_quic,
    attach_remote_daemon_session_tls, create_remote_daemon_session,
    create_remote_daemon_session_quic, create_remote_daemon_session_tls,
    list_remote_daemon_sessions, list_remote_daemon_sessions_quic, list_remote_daemon_sessions_tls,
    probe_remote_endpoint, probe_remote_endpoint_quic, probe_remote_endpoint_tls,
    probe_selected_direct_transport, probe_selected_direct_transport_tls, select_direct_transport,
};

pub use crate::remote_full_state::{full_state_from_terminal, full_state_from_ui};

const REVIVABLE_ATTACHMENT_WINDOW: Duration = Duration::from_secs(30);
const AUTH_CHALLENGE_WINDOW: Duration = Duration::from_secs(10);
const DIRECT_CLIENT_HEARTBEAT_WINDOW: Duration = Duration::from_secs(20);
const REMOTE_READ_TIMEOUT: Duration = Duration::from_secs(1);

use crate::remote_wire::{
    PROTOCOL_PEEK_BYTES, TLS_HANDSHAKE_RECORD_TYPE, decode_session_list_payload, elapsed_ms,
    encode_auth_ok_payload, encode_delta, parse_attach_request, parse_input_payload,
    parse_key_payload, parse_pane_id, parse_resize, parse_session_id, random_challenge,
    random_instance_id, random_u64_nonzero, read_attach_bootstrap, read_probe_auth_reply,
    read_probe_reply, remaining_ms,
};
#[cfg(test)]
use crate::remote_wire::{
    HEADER_LEN, LOCAL_DELTA_HEADER_LEN, LOCAL_INPUT_SEQ_LEN, MAGIC, REMOTE_CELL_ENCODED_LEN,
    REMOTE_DELTA_HEADER_LEN, REMOTE_FULL_STATE_HEADER_LEN, decode_attached_payload,
    decode_remote_full_state_payload, detect_scroll_rows, longest_prefix_suffix_overlap,
    push_string,
};

// Re-export wire-level items so external callers that reach through
// `crate::remote::` keep working.
pub use crate::remote_wire::{
    LogicalChannel, MessageType, REMOTE_CAPABILITIES, REMOTE_CAPABILITY_ATTACHMENT_RESUME,
    REMOTE_CAPABILITY_QUIC_DIRECT_TRANSPORT, REMOTE_CAPABILITY_TCP_DIRECT_TRANSPORT,
    REMOTE_CAPABILITY_TCP_TLS_TRANSPORT, REMOTE_PROTOCOL_VERSION, RemoteCell, RemoteFullState,
    STYLE_FLAG_BOLD, STYLE_FLAG_EXPLICIT_BG, STYLE_FLAG_EXPLICIT_FG, STYLE_FLAG_HYPERLINK,
    STYLE_FLAG_ITALIC, decode_auth_ok_payload, encode_full_state, encode_message,
    encode_session_list, logical_channel_for_message_type, read_message, validate_auth_ok_payload,
};

use crate::remote_batcher::{OutboundMessage, writer_loop};

enum AuthHandling {
    Authenticated,
    Pending,
    Disconnect,
}

#[derive(Clone, Debug)]
pub struct RemoteConfig {
    pub port: u16,
    pub bind_address: Option<String>,
    pub auth_key: Option<String>,
    pub allow_insecure_no_auth: bool,
    pub service_name: String,
    /// Optional override for the daemon's TLS cert chain. When both this and
    /// `cert_key_path` are provided, the daemon loads them instead of auto-
    /// generating a self-signed ed25519 cert. Use this for deployments behind
    /// an existing CA (ACME, internal PKI). `daemon_identity` still derives
    /// from the SPKI of whatever key is loaded, so SPKI-pinning keeps working.
    pub cert_chain_path: Option<PathBuf>,
    /// Optional override for the daemon's TLS private key (PEM). Paired with
    /// `cert_chain_path`.
    pub cert_key_path: Option<PathBuf>,
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

pub(crate) trait DirectReadWrite: Read + Write {}
impl<T: Read + Write> DirectReadWrite for T {}

pub(crate) struct DirectTransportSession<S: DirectReadWrite> {
    pub(crate) stream: S,
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) auth_required: bool,
    pub(crate) protocol_version: u16,
    pub(crate) capabilities: u32,
    pub(crate) build_id: Option<String>,
    pub(crate) server_instance_id: Option<String>,
    pub(crate) server_identity_id: Option<String>,
}

type DirectRemoteClient = DirectTransportSession<std::net::TcpStream>;

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

use crate::remote_listener::{NEXT_CLIENT_ID, serve_incoming_tcp_client, spawn_quic_accept_loop};
#[cfg(test)]
use crate::remote_listener::handle_quic_incoming;
use crate::remote_state::{
    AuthChallengeState, ClientState, RevivableAttachment, State, prune_revivable_attachments,
    send_direct_error, send_direct_frame,
    should_disconnect_idle_client as should_disconnect_idle_client_inner,
};

/// Call-site shim that passes the file-local auth and heartbeat windows.
/// Keeps callers inside remote.rs agnostic of which module owns the policy
/// constants.
fn should_disconnect_idle_client(state: &Arc<Mutex<State>>, client_id: u64) -> Option<&'static str> {
    should_disconnect_idle_client_inner(
        state,
        client_id,
        AUTH_CHALLENGE_WINDOW,
        DIRECT_CLIENT_HEARTBEAT_WINDOW,
    )
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
    pub fn start(config: RemoteConfig) -> io::Result<(Self, mpsc::Receiver<RemoteCmd>)> {
        if config.rejects_public_authless_bind() {
            return Err(io::Error::other(format!(
                "refusing to start authless remote daemon on public bind address {}; configure --remote-auth-key or --remote-allow-insecure-no-auth",
                config.effective_bind_address()
            )));
        }
        let bind_address = config.effective_bind_address().to_string();
        let listener = TcpListener::bind((bind_address.as_str(), config.port))?;
        let identity_material = match (&config.cert_chain_path, &config.cert_key_path) {
            (Some(cert), Some(key)) => {
                log::info!(
                    "remote tls: using caller-provided identity material cert={} key={}",
                    cert.display(),
                    key.display()
                );
                load_external_daemon_identity_material(cert, key).map_err(io::Error::other)?
            }
            (None, None) => load_or_create_daemon_identity_material(),
            _ => {
                return Err(io::Error::other(
                    "remote tls: --remote-cert-path and --remote-key-path must be provided together",
                ));
            }
        };
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
        prune_revivable_attachments(&mut state);
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

pub(crate) fn read_loop(
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
        prune_revivable_attachments(&mut state);
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

impl DirectTransportSession<std::net::TcpStream> {
    /// Thin wrapper around `connect_with(PlainTcpConnector, ...)` for backwards
    /// compatibility with tests and callers that already target the TCP type.
    #[cfg(test)]
    fn connect(
        host: &str,
        port: u16,
        auth_key: Option<&str>,
        expected_server_identity: Option<&str>,
    ) -> Result<Self, String> {
        connect_with(PlainTcpConnector, host, port, auth_key, expected_server_identity)
    }
}

impl DirectTransportSession<QuicClientStream> {
    /// Backwards-compatible entry point; delegates to `connect_with`.
    #[cfg(test)]
    fn connect_quic(
        host: &str,
        port: u16,
        auth_key: Option<&str>,
        expected_identity: &str,
    ) -> Result<Self, String> {
        connect_with(
            PinnedQuicConnector { expected_identity },
            host,
            port,
            auth_key,
            Some(expected_identity),
        )
    }
}

impl DirectTransportSession<TlsClientStream> {
    /// Backwards-compatible entry point; delegates to `connect_with`.
    #[cfg(test)]
    fn connect_tls(
        host: &str,
        port: u16,
        auth_key: Option<&str>,
        expected_identity: &str,
    ) -> Result<Self, String> {
        connect_with(
            PinnedTlsConnector { expected_identity },
            host,
            port,
            auth_key,
            Some(expected_identity),
        )
    }
}


impl<S: DirectReadWrite> DirectTransportSession<S> {
    pub(crate) fn connect_over_stream(
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

    pub(crate) fn heartbeat_round_trip(&mut self, payload: &[u8]) -> Result<u64, String> {
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

    pub(crate) fn list_sessions(&mut self) -> Result<Vec<RemoteDirectSessionInfo>, String> {
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

    pub(crate) fn attach(
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

    pub(crate) fn create_session(&mut self, cols: u16, rows: u16) -> Result<u32, String> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote_identity::{
        DaemonIdentityMaterial, derive_identity_id, load_or_create_daemon_identity_material_at,
    };

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
            cert_chain_path: None,
            cert_key_path: None,
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
            cert_chain_path: None,
            cert_key_path: None,
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
            cert_chain_path: None,
            cert_key_path: None,
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
            cert_chain_path: None,
            cert_key_path: None,
        };
        assert_eq!(config.effective_bind_address(), "192.168.0.5");
        assert!(config.should_advertise());
        assert!(!config.rejects_public_authless_bind());
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
