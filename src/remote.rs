use std::collections::HashMap;
use std::io;
use std::net::TcpListener;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, mpsc};
use std::time::Instant;

pub(crate) use crate::remote_direct_session::DirectTransportSession;

// Re-export the public data types so existing callers of
// `crate::remote::RemoteProbeSummary` etc. keep working unchanged.
#[allow(unused_imports)]
pub use crate::remote_types::{
    DirectTransportKind, RemoteAttachSummary, RemoteAttachedSummary, RemoteClientInfo,
    RemoteClientsSnapshot, RemoteCreateSummary, RemoteDirectSessionInfo, RemoteProbeSummary,
    RemoteServerInfo, RemoteSessionInfo, RemoteSessionListSummary, RemoteUpgradeProbeSummary,
    RevivableAttachmentInfo,
};

// Re-export the direct-client RPCs so existing callers of
// `crate::remote::probe_remote_endpoint` etc. keep working unchanged.
pub use crate::remote_client::{
    attach_remote_daemon_session, create_remote_daemon_session, list_remote_daemon_sessions,
    probe_remote_endpoint, probe_selected_direct_transport, select_direct_transport,
};

pub use crate::remote_full_state::{full_state_from_terminal, full_state_from_ui};


use crate::remote_wire::{random_instance_id, random_u64_nonzero};
// Re-export wire-level items so external callers that reach through
// `crate::remote::` keep working.
#[allow(unused_imports)]
pub use crate::remote_wire::{
    MessageType, REMOTE_CAPABILITIES, REMOTE_CAPABILITY_ATTACHMENT_RESUME,
    REMOTE_CAPABILITY_TCP_DIRECT_TRANSPORT, REMOTE_PROTOCOL_VERSION, RemoteCell, RemoteFullState,
    decode_auth_ok_payload, encode_full_state, encode_message, encode_session_list, read_message,
    validate_auth_ok_payload,
};
use crate::remote_batcher::{OutboundMessage, writer_loop};

#[derive(Clone, Debug)]
pub struct RemoteConfig {
    pub port: u16,
    pub bind_address: Option<String>,
    pub service_name: String,
}

impl RemoteConfig {
    pub(crate) fn effective_bind_address(&self) -> &str {
        self.bind_address.as_deref().unwrap_or("127.0.0.1")
    }

    pub(crate) fn should_advertise(&self) -> bool {
        !matches!(self.effective_bind_address(), "127.0.0.1" | "localhost" | "::1")
    }
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

use crate::remote_auth::read_loop;
use crate::remote_listener::{NEXT_CLIENT_ID, serve_incoming_tcp_client};
use crate::remote_server_advertise::ServiceAdvertiser;
use crate::remote_server_attach::prepare_attachment as prepare_remote_attachment;
use crate::remote_server_control::{
    reply_session_list as send_reply_session_list,
    send_session_list as send_cached_session_list,
    send_session_list_to_local_clients as send_cached_session_list_to_local_clients,
    send_ui_appearance as send_local_ui_appearance,
    send_ui_appearance_to_local_clients as send_ui_appearance_to_all_local_clients,
    send_ui_runtime_state as send_local_ui_runtime_state,
    send_ui_runtime_state_to_local_attached as send_ui_runtime_state_to_attached_locals,
};
use crate::remote_server_diag::clients_snapshot as build_clients_snapshot;
use crate::remote_server_stream::{
    send_pane_state_to_client as publish_pane_state_to_client,
    send_state_to_client as publish_state_to_client,
};
use crate::remote_server_targets::{
    client_ids_for_session, local_attached_client_ids,
    retain_local_attached_pane_states as retain_local_attached_pane_states_inner,
    retarget_local_attached_client_ids,
};
use crate::remote_state::{ClientState, REMOTE_READ_TIMEOUT, State};

pub struct RemoteServer {
    state: Arc<Mutex<State>>,
    _listener: std::thread::JoinHandle<()>,
    _advertiser: Option<ServiceAdvertiser>,
    local_socket_path: Option<PathBuf>,
    bind_address: Option<String>,
    port: Option<u16>,
}

impl RemoteServer {
    pub fn start(config: RemoteConfig) -> io::Result<(Self, mpsc::Receiver<RemoteCmd>)> {
        let bind_address = config.effective_bind_address().to_string();
        let listener = TcpListener::bind((bind_address.as_str(), config.port))?;
        let state = Arc::new(Mutex::new(State {
            clients: HashMap::new(),
            revivable_attachments: HashMap::new(),
            server_identity_id: random_instance_id(),
            server_instance_id: random_instance_id(),
        }));
        let state_for_listener = Arc::clone(&state);
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let cmd_tx_for_tcp = cmd_tx.clone();
        let listener_thread = std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else {
                    continue;
                };
                let _ = stream.set_read_timeout(Some(REMOTE_READ_TIMEOUT));
                let state = Arc::clone(&state_for_listener);
                let cmd_tx = cmd_tx_for_tcp.clone();
                std::thread::spawn(move || {
                    serve_incoming_tcp_client(stream, state, cmd_tx);
                });
            }
        });

        let advertiser = if config.should_advertise() {
            ServiceAdvertiser::spawn(&config.service_name, config.port)
        } else {
            None
        };
        {
            let state = state.lock().expect("remote server state poisoned");
            log::info!(
                "remote tcp server started: bind_address={} port={} protocol_version={} capabilities={} build_id={} server_identity_id={} server_instance_id={}",
                bind_address,
                config.port,
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
            server_identity_id: random_instance_id(),
            server_instance_id: random_instance_id(),
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

    #[cfg(test)]
    pub(crate) fn for_test(state: Arc<Mutex<State>>) -> Self {
        Self {
            state,
            _listener: std::thread::spawn(|| {}),
            _advertiser: None,
            local_socket_path: None,
            bind_address: None,
            port: None,
        }
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
        build_clients_snapshot(
            &state,
            self.local_socket_path.as_deref(),
            self.bind_address.as_deref(),
            self.port,
        )
    }

    pub fn send_session_list(&self, client_id: u64, sessions: &[RemoteSessionInfo]) {
        send_cached_session_list(&self.state, client_id, sessions);
    }

    pub fn reply_session_list(&self, client_id: u64, sessions: &[RemoteSessionInfo]) {
        send_reply_session_list(&self.state, client_id, sessions);
    }

    pub fn send_session_list_to_local_clients(&self, sessions: &[RemoteSessionInfo]) {
        send_cached_session_list_to_local_clients(&self.state, sessions);
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
        prepare_remote_attachment(&mut state, client_id, session_id, attachment_id, resume_token)
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
        send_local_ui_runtime_state(&self.state, client_id, state);
    }

    pub fn send_ui_runtime_state_to_local_attached(
        &self,
        session_id: u32,
        state: &crate::control::UiRuntimeState,
    ) {
        send_ui_runtime_state_to_attached_locals(&self.state, session_id, state);
    }

    pub fn retarget_local_attached_to_session(&self, session_id: u32) -> bool {
        let client_ids = {
            let state_guard = self.state.lock().expect("remote server state poisoned");
            retarget_local_attached_client_ids(&state_guard, session_id)
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
        send_local_ui_appearance(&self.state, client_id, appearance);
    }

    pub fn send_ui_appearance_to_local_clients(
        &self,
        appearance: &crate::control::UiAppearanceSnapshot,
    ) {
        send_ui_appearance_to_all_local_clients(&self.state, appearance);
    }

    pub fn send_full_state_to_attached(&self, session_id: u32, state: Arc<RemoteFullState>) {
        let client_ids = self.clients_for_session(session_id);
        for client_id in client_ids {
            publish_state_to_client(&self.state, client_id, session_id, Arc::clone(&state));
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
            local_attached_client_ids(&state_guard, session_id)
        };
        for client_id in client_ids {
            publish_pane_state_to_client(
                &self.state,
                client_id,
                session_id,
                pane_id,
                Arc::clone(&state),
            );
        }
    }

    pub fn retain_local_attached_pane_states(
        &self,
        session_id: u32,
        visible_pane_ids: &[u64],
    ) {
        let mut guard = self.state.lock().expect("remote server state poisoned");
        retain_local_attached_pane_states_inner(&mut guard, session_id, visible_pane_ids);
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
        client_ids_for_session(&state, session_id)
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

}

impl Drop for RemoteServer {
    fn drop(&mut self) {
        if let Some(path) = self.local_socket_path.as_ref() {
            let _ = std::fs::remove_file(path);
        }
    }
}
