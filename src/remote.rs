use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::TcpListener;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

type HmacSha256 = Hmac<Sha256>;

const MAGIC: [u8; 2] = [0x47, 0x53];
const HEADER_LEN: usize = 7;
static NEXT_CLIENT_ID: AtomicU64 = AtomicU64::new(1);

const LOCAL_INPUT_SEQ_LEN: usize = 8;
const REMOTE_FULL_STATE_HEADER_LEN: usize = 14;
const REMOTE_DELTA_HEADER_LEN: usize = 13;
#[cfg(test)]
const LOCAL_DELTA_HEADER_LEN: usize = LOCAL_INPUT_SEQ_LEN + REMOTE_DELTA_HEADER_LEN;
const REMOTE_CELL_ENCODED_LEN: usize = 12;
const STYLE_FLAG_BOLD: u8 = 0x01;
const STYLE_FLAG_ITALIC: u8 = 0x02;
const STYLE_FLAG_HYPERLINK: u8 = 0x04;
const STYLE_FLAG_EXPLICIT_FG: u8 = 0x20;
const STYLE_FLAG_EXPLICIT_BG: u8 = 0x40;

enum OutboundMessage {
    Frame(Vec<u8>),
    ScreenUpdate(Vec<u8>),
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
            _ => return Err(()),
        };
        Ok(message)
    }
}

#[derive(Clone, Debug)]
pub struct RemoteConfig {
    pub port: u16,
    pub auth_key: Option<String>,
    pub service_name: String,
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
    challenge: Option<[u8; 32]>,
    attached_session: Option<u32>,
    last_state: Option<Arc<RemoteFullState>>,
    pane_states: HashMap<u64, Arc<RemoteFullState>>,
    latest_input_seq: Option<u64>,
    is_local: bool,
}

struct State {
    clients: HashMap<u64, ClientState>,
    auth_key: Option<Vec<u8>>,
}

pub struct RemoteServer {
    state: Arc<Mutex<State>>,
    _listener: std::thread::JoinHandle<()>,
    _advertiser: Option<ServiceAdvertiser>,
    local_socket_path: Option<PathBuf>,
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
        let listener = TcpListener::bind(("0.0.0.0", config.port))?;
        let state = Arc::new(Mutex::new(State {
            clients: HashMap::new(),
            auth_key: config.auth_key.map(|key| key.into_bytes()),
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
                    let authenticated = state.auth_key.is_none();
                    state.clients.insert(
                        client_id,
                        ClientState {
                            outbound: outbound_tx,
                            authenticated,
                            challenge: None,
                            attached_session: None,
                            last_state: None,
                            pane_states: HashMap::new(),
                            latest_input_seq: None,
                            is_local: false,
                        },
                    );
                    (client_id, outbound_rx)
                };

                let Ok(writer_stream) = stream.try_clone() else {
                    let mut state = state_for_listener
                        .lock()
                        .expect("remote server state poisoned");
                    state.clients.remove(&client_id);
                    continue;
                };
                std::thread::spawn(move || writer_loop(writer_stream, outbound_rx));

                let cmd_tx = cmd_tx.clone();
                let state = Arc::clone(&state_for_listener);
                std::thread::spawn(move || read_loop(stream, client_id, state, cmd_tx));
            }
        });

        let advertiser = ServiceAdvertiser::spawn(&config.service_name, config.port);
        Ok((
            Self {
                state,
                _listener: listener_thread,
                _advertiser: advertiser,
                local_socket_path: None,
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
            auth_key: None,
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
                            attached_session: None,
                            last_state: None,
                            pane_states: HashMap::new(),
                            latest_input_seq: None,
                            is_local: true,
                        },
                    );
                    (client_id, outbound_rx)
                };

                let Ok(writer_stream) = stream.try_clone() else {
                    let mut state = state_for_listener
                        .lock()
                        .expect("remote server state poisoned");
                    state.clients.remove(&client_id);
                    continue;
                };
                std::thread::spawn(move || writer_loop(writer_stream, outbound_rx));

                let cmd_tx = cmd_tx.clone();
                let state = Arc::clone(&state_for_listener);
                let _ = cmd_tx.send(RemoteCmd::Connected { client_id });
                crate::notify_headless_wakeup();
                std::thread::spawn(move || read_loop(stream, client_id, state, cmd_tx));
            }
        });

        Ok((
            Self {
                state,
                _listener: listener_thread,
                _advertiser: None,
                local_socket_path: Some(socket_path),
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

    pub fn client_is_local(&self, client_id: u64) -> bool {
        let state = self.state.lock().expect("remote server state poisoned");
        state
            .clients
            .get(&client_id)
            .is_some_and(|client| client.is_local)
    }

    pub fn send_session_list(&self, client_id: u64, sessions: &[RemoteSessionInfo]) {
        self.send_to_client(
            client_id,
            MessageType::SessionList,
            encode_session_list(sessions),
        );
    }

    pub fn send_session_list_to_local_clients(&self, sessions: &[RemoteSessionInfo]) {
        let client_ids = {
            let state_guard = self.state.lock().expect("remote server state poisoned");
            state_guard
                .clients
                .iter()
                .filter_map(|(client_id, client)| client.is_local.then_some(*client_id))
                .collect::<Vec<_>>()
        };
        for client_id in client_ids {
            self.send_session_list(client_id, sessions);
        }
    }

    pub fn send_attached(&self, client_id: u64, session_id: u32) {
        let payload = session_id.to_le_bytes().to_vec();
        self.update_client(client_id, |client| {
            let same_session = client.attached_session == Some(session_id);
            client.attached_session = Some(session_id);
            if !same_session {
                client.last_state = None;
                client.pane_states.clear();
                client.latest_input_seq = None;
            }
        });
        self.send_to_client(client_id, MessageType::Attached, payload);
    }

    pub fn send_detached(&self, client_id: u64) {
        self.update_client(client_id, |client| {
            client.attached_session = None;
            client.last_state = None;
            client.pane_states.clear();
            client.latest_input_seq = None;
        });
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
        self.send_to_client(client_id, MessageType::UiRuntimeState, payload);
    }

    pub fn send_ui_runtime_state_to_local_attached(
        &self,
        session_id: u32,
        state: &crate::control::UiRuntimeState,
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
            self.send_ui_runtime_state(client_id, state);
        }
    }

    pub fn send_attached_to_local_attached(&self, session_id: u32) {
        let client_ids = {
            let state_guard = self.state.lock().expect("remote server state poisoned");
            state_guard
                .clients
                .iter()
                .filter_map(|(client_id, client)| {
                    (client.is_local && client.attached_session.is_some()).then_some(*client_id)
                })
                .collect::<Vec<_>>()
        };
        for client_id in client_ids {
            self.send_attached(client_id, session_id);
        }
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
        self.send_to_client(client_id, MessageType::UiAppearance, payload);
    }

    pub fn send_ui_appearance_to_local_clients(
        &self,
        appearance: &crate::control::UiAppearanceSnapshot,
    ) {
        let client_ids = {
            let state_guard = self.state.lock().expect("remote server state poisoned");
            state_guard
                .clients
                .iter()
                .filter_map(|(client_id, client)| client.is_local.then_some(*client_id))
                .collect::<Vec<_>>()
        };
        for client_id in client_ids {
            self.send_ui_appearance(client_id, appearance);
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

fn writer_loop<W: Write>(mut stream: W, outbound_rx: mpsc::Receiver<OutboundMessage>) {
    while let Ok(message) = outbound_rx.recv() {
        let mut scope =
            crate::profiling::scope("server.stream.batch_write", crate::profiling::Kind::Io);
        let batch = collect_outbound_batch(message, &outbound_rx);
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
            pending_screen = Some(frame);
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

fn read_loop(
    mut stream: impl Read,
    client_id: u64,
    state: Arc<Mutex<State>>,
    cmd_tx: mpsc::Sender<RemoteCmd>,
) {
    loop {
        let mut scope =
            crate::profiling::scope("server.stream.read_message", crate::profiling::Kind::Io);
        let Ok((ty, payload)) = read_message(&mut stream) else {
            break;
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
            handle_auth_message(client_id, &payload, &state);
            continue;
        }

        if !authenticated {
            send_direct_error(&state, client_id, "authentication required");
            continue;
        }

        let command = match ty {
            MessageType::ListSessions => Some(RemoteCmd::ListSessions { client_id }),
            MessageType::Attach => parse_session_id(&payload).map(|session_id| RemoteCmd::Attach {
                client_id,
                session_id,
            }),
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
    state.clients.remove(&client_id);
}

fn handle_auth_message(client_id: u64, payload: &[u8], state: &Arc<Mutex<State>>) {
    let mut state = state.lock().expect("remote server state poisoned");
    let auth_key = state.auth_key.clone();
    let Some(client) = state.clients.get_mut(&client_id) else {
        return;
    };

    if auth_key.is_none() {
        client.authenticated = true;
        let _ = client.outbound.send(OutboundMessage::Frame(encode_message(
            MessageType::AuthOk,
            &[],
        )));
        return;
    }

    if payload.is_empty() {
        let challenge = random_challenge();
        client.challenge = Some(challenge);
        let _ = client.outbound.send(OutboundMessage::Frame(encode_message(
            MessageType::AuthChallenge,
            &challenge,
        )));
        return;
    }

    let Some(challenge) = client.challenge.take() else {
        let _ = client.outbound.send(OutboundMessage::Frame(encode_message(
            MessageType::AuthFail,
            &[],
        )));
        return;
    };
    let Some(key) = auth_key else {
        let _ = client.outbound.send(OutboundMessage::Frame(encode_message(
            MessageType::AuthFail,
            &[],
        )));
        return;
    };

    let mut mac = HmacSha256::new_from_slice(&key).expect("valid HMAC key");
    mac.update(&challenge);
    match mac.verify_slice(payload) {
        Ok(()) => {
            client.authenticated = true;
            let _ = client.outbound.send(OutboundMessage::Frame(encode_message(
                MessageType::AuthOk,
                &[],
            )));
        }
        Err(_) => {
            let _ = client.outbound.send(OutboundMessage::Frame(encode_message(
                MessageType::AuthFail,
                &[],
            )));
        }
    }
}

fn send_direct_error(state: &Arc<Mutex<State>>, client_id: u64, message: &str) {
    let state = state.lock().expect("remote server state poisoned");
    if let Some(client) = state.clients.get(&client_id) {
        let _ = client.outbound.send(OutboundMessage::Frame(encode_message(
            MessageType::ErrorMsg,
            message.as_bytes(),
        )));
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
        let batch = collect_outbound_batch(first, &rx);
        assert_eq!(batch.frames, vec![vec![2], vec![9], vec![3]]);
        assert_eq!(batch.message_count, 4);
        assert_eq!(batch.coalesced_screen_updates, 1);
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
        let batch = collect_outbound_batch(first, &rx);
        assert_eq!(batch.frames, vec![runtime_b, appearance_b, barrier]);
        assert_eq!(batch.message_count, 5);
        assert_eq!(batch.coalesced_control_frames, 2);
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
                    attached_session: Some(11),
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
            auth_key: None,
        }));
        let server = RemoteServer {
            state: Arc::clone(&state),
            _listener: std::thread::spawn(|| {}),
            _advertiser: None,
            local_socket_path: None,
        };

        server.send_attached(7, 11);

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
                        attached_session: Some(11),
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
                        attached_session: None,
                        last_state: None,
                        pane_states: HashMap::new(),
                        latest_input_seq: None,
                        is_local: true,
                    },
                ),
            ]),
            auth_key: None,
        }));
        let server = RemoteServer {
            state,
            _listener: std::thread::spawn(|| {}),
            _advertiser: None,
            local_socket_path: None,
        };
        let ui_state = control::UiRuntimeState {
            active_tab: 0,
            focused_pane: 7,
            tabs: Vec::new(),
            visible_panes: Vec::new(),
            mouse_selection: control::UiMouseSelectionSnapshot::default(),
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
    fn send_attached_to_local_attached_skips_unattached_and_remote_clients() {
        let (local_attached_tx, local_attached_rx) = mpsc::channel();
        let (local_unattached_tx, local_unattached_rx) = mpsc::channel();
        let (remote_attached_tx, remote_attached_rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(State {
            clients: HashMap::from([
                (
                    1,
                    ClientState {
                        outbound: local_attached_tx,
                        authenticated: true,
                        challenge: None,
                        attached_session: Some(11),
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
                        attached_session: None,
                        last_state: None,
                        pane_states: HashMap::new(),
                        latest_input_seq: None,
                        is_local: true,
                    },
                ),
                (
                    3,
                    ClientState {
                        outbound: remote_attached_tx,
                        authenticated: true,
                        challenge: None,
                        attached_session: Some(11),
                        last_state: None,
                        pane_states: HashMap::new(),
                        latest_input_seq: None,
                        is_local: false,
                    },
                ),
            ]),
            auth_key: None,
        }));
        let server = RemoteServer {
            state,
            _listener: std::thread::spawn(|| {}),
            _advertiser: None,
            local_socket_path: None,
        };

        server.send_attached_to_local_attached(22);

        match local_attached_rx.recv().expect("local attached frame") {
            OutboundMessage::Frame(frame) => {
                assert_eq!(&frame[..2], &MAGIC);
                assert_eq!(frame[2], MessageType::Attached as u8);
            }
            OutboundMessage::ScreenUpdate(_) => panic!("unexpected screen update"),
        }
        assert!(local_unattached_rx.try_recv().is_err());
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
                    attached_session: Some(11),
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
            auth_key: None,
        }));
        let server = RemoteServer {
            state: Arc::clone(&state),
            _listener: std::thread::spawn(|| {}),
            _advertiser: None,
            local_socket_path: None,
        };

        server.retain_local_attached_pane_states(11, &[20]);

        let guard = state.lock().expect("remote server state poisoned");
        let client = guard.clients.get(&1).expect("client state");
        assert!(!client.pane_states.contains_key(&10));
        assert!(client.pane_states.contains_key(&20));
    }
}
