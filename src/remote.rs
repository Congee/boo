use std::collections::HashMap;
use std::io;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, mpsc};
use std::time::Instant;

pub(crate) use crate::remote_direct_transport::DirectTransportClient;

// Re-export the public data types so existing callers of
// `crate::remote::RemoteProbeSummary` etc. keep working unchanged.
#[allow(unused_imports)]
pub use crate::remote_types::{
    DirectTransportKind, RemoteClientInfo, RemoteClientsSnapshot, RemoteCreateSummary,
    RemoteDirectTabInfo, RemoteProbeSummary, RemoteServerInfo, RemoteTabInfo, RemoteTabListSummary,
    RemoteUpgradeProbeSummary,
};

// Re-export the direct-client RPCs so existing callers of
// `crate::remote::probe_remote_endpoint` etc. keep working unchanged.
#[allow(unused_imports)]
pub use crate::remote_client::{
    create_remote_daemon_tab, list_remote_daemon_tabs, probe_remote_endpoint,
    probe_selected_direct_transport, select_direct_transport,
};

pub use crate::remote_full_state::{full_state_from_terminal, full_state_from_ui};

use crate::remote_wire::{encode_error_payload, encode_row_range_response, random_instance_id};
// Re-export wire-level items so external callers that reach through
// `crate::remote::` keep working.
use crate::remote_batcher::{OutboundMessage, writer_loop};
#[allow(unused_imports)]
pub use crate::remote_wire::{
    MESSAGE_TYPE_LIST_TABS, MESSAGE_TYPE_TAB_CREATED, MESSAGE_TYPE_TAB_EXITED,
    MESSAGE_TYPE_TAB_LIST, MessageType, REMOTE_CAPABILITIES,
    REMOTE_CAPABILITY_QUIC_DIRECT_TRANSPORT, REMOTE_PROTOCOL_VERSION, RemoteCell, RemoteErrorCode,
    RemoteFullState, RemoteRowRangeRequest, RemoteRowRangeResponse, decode_auth_ok_payload,
    encode_full_state, encode_message, encode_tab_list, read_message, validate_auth_ok_payload,
};

#[derive(Clone, Debug)]
pub struct RemoteConfig {
    pub port: u16,
    pub bind_address: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RuntimeAction {
    SetViewedTab {
        view_id: u64,
        tab_id: u32,
    },
    FocusPane {
        view_id: u64,
        tab_id: u32,
        pane_id: u64,
    },
    NewTab {
        view_id: u64,
        cols: Option<u16>,
        rows: Option<u16>,
    },
    CloseTab {
        view_id: u64,
        tab_id: Option<u32>,
    },
    NextTab {
        view_id: u64,
    },
    PrevTab {
        view_id: u64,
    },
    AttachView {
        view_id: u64,
    },
    DetachView {
        view_id: u64,
    },
    NewSplit {
        view_id: u64,
        direction: Option<String>,
    },
    ResizeSplit {
        view_id: u64,
        direction: String,
        amount: u16,
        ratio: Option<f64>,
    },
    ScrollFocusedPane {
        view_id: u64,
        rows: i64,
    },
    SetCopyMode {
        view_id: u64,
        active: bool,
    },
    SetSearchQuery {
        view_id: u64,
        query: String,
    },
    NavigateSearch {
        view_id: u64,
        direction: String,
    },
    Noop {
        view_id: u64,
    },
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct RuntimeActionEnvelope {
    pub client_action_id: u64,
    pub action: RuntimeAction,
}

impl RuntimeAction {
    pub(crate) fn trace_action(&self) -> crate::trace_schema::RuntimeActionKind {
        match self {
            Self::SetViewedTab { .. } => crate::trace_schema::RuntimeActionKind::SetViewedTab,
            Self::FocusPane { .. } => crate::trace_schema::RuntimeActionKind::FocusPane,
            Self::NewTab { .. } => crate::trace_schema::RuntimeActionKind::NewTab,
            Self::CloseTab { .. } => crate::trace_schema::RuntimeActionKind::CloseTab,
            Self::NextTab { .. } => crate::trace_schema::RuntimeActionKind::NextTab,
            Self::PrevTab { .. } => crate::trace_schema::RuntimeActionKind::PrevTab,
            Self::AttachView { .. } => crate::trace_schema::RuntimeActionKind::AttachView,
            Self::DetachView { .. } => crate::trace_schema::RuntimeActionKind::DetachView,
            Self::NewSplit { .. } => crate::trace_schema::RuntimeActionKind::NewSplit,
            Self::ResizeSplit { .. } => crate::trace_schema::RuntimeActionKind::ResizeSplit,
            Self::ScrollFocusedPane { .. } => crate::trace_schema::RuntimeActionKind::ScrollFocusedPane,
            Self::SetCopyMode { .. } => crate::trace_schema::RuntimeActionKind::SetCopyMode,
            Self::SetSearchQuery { .. } => crate::trace_schema::RuntimeActionKind::SetSearchQuery,
            Self::NavigateSearch { .. } => crate::trace_schema::RuntimeActionKind::NavigateSearch,
            Self::Noop { .. } => crate::trace_schema::RuntimeActionKind::Noop,
        }
    }
}

pub(crate) fn decode_runtime_action_payload(
    payload: &[u8],
) -> Result<(Option<u64>, RuntimeAction), serde_json::Error> {
    match serde_json::from_slice::<RuntimeActionEnvelope>(payload) {
        Ok(envelope) => Ok((Some(envelope.client_action_id), envelope.action)),
        Err(_) => serde_json::from_slice::<RuntimeAction>(payload).map(|action| (None, action)),
    }
}

impl RemoteConfig {
    pub(crate) fn effective_bind_address(&self) -> &str {
        self.bind_address.as_deref().unwrap_or("127.0.0.1")
    }
}

#[derive(Debug)]
pub enum RemoteCmd {
    Connected {
        client_id: u64,
    },
    ListTabs {
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
        tab_id: Option<u32>,
    },
    RuntimeAction {
        client_id: u64,
        client_action_id: Option<u64>,
        action: RuntimeAction,
    },
    RenderAck {
        client_id: u64,
        view_id: u64,
        tab_id: u32,
        pane_id: u64,
        pane_revision: u64,
        runtime_revision: u64,
    },
    RowRangeRequest {
        client_id: u64,
        request: crate::remote_wire::RemoteRowRangeRequest,
    },
}

use crate::remote_auth::read_loop;
use crate::remote_listener::NEXT_CLIENT_ID;
use crate::remote_quic::{QuicServerHandle, start_quic_listener};
use crate::remote_server_control::{
    reply_tab_list as send_reply_tab_list, send_tab_list as send_cached_tab_list,
    send_tab_list_to_local_clients as send_cached_tab_list_to_local_clients,
    send_ui_appearance as send_local_ui_appearance,
    send_ui_appearance_to_local_clients as send_ui_appearance_to_all_local_clients,
    send_ui_runtime_state as send_local_ui_runtime_state,
    send_ui_runtime_state_to_local_viewers as send_ui_runtime_state_to_local_viewers_inner,
};
use crate::remote_server_diag::clients_snapshot as build_clients_snapshot;
use crate::remote_server_stream::{
    send_pane_state_to_client as publish_pane_state_to_client,
    send_state_to_client as publish_state_to_client,
};
use crate::remote_server_targets::{
    local_viewer_client_ids,
    retain_local_viewer_pane_states as retain_local_viewer_pane_states_inner, viewer_client_ids,
};
use crate::remote_state::{ClientRuntimeView, ClientState, State};

pub struct RemoteServer {
    state: Arc<Mutex<State>>,
    _quic_listener: Option<QuicServerHandle>,
    _local_listener: Option<std::thread::JoinHandle<()>>,
    local_socket_path: Option<PathBuf>,
    bind_address: Option<String>,
    port: Option<u16>,
}

impl RemoteServer {
    pub fn start(config: RemoteConfig) -> io::Result<(Self, mpsc::Receiver<RemoteCmd>)> {
        let bind_address = config.effective_bind_address().to_string();
        let state = Arc::new(Mutex::new(State {
            clients: HashMap::new(),
            server_instance_id: random_instance_id(),
        }));
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let quic_listener = start_quic_listener(
            &bind_address,
            config.port,
            Arc::clone(&state),
            cmd_tx.clone(),
        )?;

        {
            let state = state.lock().expect("remote server state poisoned");
            log::info!(
                "remote quic server started: bind_address={} port={} protocol_version={} capabilities={} build_id={} server_instance_id={}",
                bind_address,
                config.port,
                REMOTE_PROTOCOL_VERSION,
                REMOTE_CAPABILITIES,
                env!("CARGO_PKG_VERSION"),
                state.server_instance_id
            );
        }
        Ok((
            Self {
                state,
                _quic_listener: Some(quic_listener),
                _local_listener: None,
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
                            runtime_view: ClientRuntimeView::idle(),
                            is_local: true,
                        },
                    );
                    (client_id, outbound_rx)
                };
                log::info!("remote local-stream client connected: client_id={client_id}");
                tracing::info!(
                    target: "boo::latency",
                    interaction_id = 0_u64,
                    view_id = client_id,
                    tab_id = 0_u32,
                    pane_id = 0_u64,
                    action = "connect",
                    route = "local_stream",
                    runtime_revision = 0_u64,
                    view_revision = 1_u64,
                    pane_revision = 0_u64,
                    elapsed_ms = 0.0_f64,
                    "{}",
                    crate::trace_schema::events::REMOTE_CONNECT
                );

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
                "remote local-stream server started: socket={} protocol_version={} capabilities={} build_id={} server_instance_id={}",
                socket_path.display(),
                REMOTE_PROTOCOL_VERSION,
                REMOTE_CAPABILITIES,
                env!("CARGO_PKG_VERSION"),
                state.server_instance_id
            );
        }
        Ok((
            Self {
                state,
                _quic_listener: None,
                _local_listener: Some(listener_thread),
                local_socket_path: Some(socket_path),
                bind_address: None,
                port: None,
            },
            cmd_rx,
        ))
    }

    pub fn has_runtime_viewers(&self) -> bool {
        let state = self.state.lock().expect("remote server state poisoned");
        state
            .clients
            .values()
            .any(|client| client.runtime_view.subscribed_to_runtime)
    }

    #[allow(dead_code)]
    pub fn has_local_runtime_viewers(&self) -> bool {
        let state = self.state.lock().expect("remote server state poisoned");
        state
            .clients
            .values()
            .any(|client| client.is_local && client.runtime_view.subscribed_to_runtime)
    }

    pub fn client_subscribed_to_runtime(&self, client_id: u64) -> bool {
        let state = self.state.lock().expect("remote server state poisoned");
        state
            .clients
            .get(&client_id)
            .is_some_and(|client| client.runtime_view.subscribed_to_runtime)
    }

    #[allow(dead_code)]
    pub fn client_is_viewer(&self, client_id: u64) -> bool {
        self.client_subscribed_to_runtime(client_id)
    }

    #[cfg(test)]
    pub(crate) fn for_test(state: Arc<Mutex<State>>) -> Self {
            Self {
                state,
                _quic_listener: None,
                _local_listener: Some(std::thread::spawn(|| {})),
                local_socket_path: None,
            bind_address: None,
            port: None,
        }
    }

    pub fn has_client(&self, client_id: u64) -> bool {
        let state = self.state.lock().expect("remote server state poisoned");
        state.clients.contains_key(&client_id)
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

    pub fn send_tab_list(&self, client_id: u64, tabs: &[RemoteTabInfo]) {
        send_cached_tab_list(&self.state, client_id, tabs);
    }

    pub fn reply_tab_list(&self, client_id: u64, tabs: &[RemoteTabInfo]) {
        send_reply_tab_list(&self.state, client_id, tabs);
    }

    pub fn send_tab_list_to_local_clients(&self, tabs: &[RemoteTabInfo]) {
        send_cached_tab_list_to_local_clients(&self.state, tabs);
    }

    pub fn send_tab_list_to_viewers(&self, tabs: &[RemoteTabInfo]) {
        let client_ids = {
            let state = self.state.lock().expect("remote server state poisoned");
            state
                .clients
                .iter()
                .filter_map(|(client_id, client)| {
                    client
                        .runtime_view
                        .subscribed_to_runtime
                        .then_some(*client_id)
                })
                .collect::<Vec<_>>()
        };
        for client_id in client_ids {
            self.send_tab_list(client_id, tabs);
        }
    }

    pub fn initialize_client_view(
        &self,
        client_id: u64,
        viewed_tab_id: Option<u32>,
        focused_pane_id: Option<u64>,
        visible_pane_ids: &[u64],
    ) {
        self.update_client(client_id, |client| {
            if client.runtime_view.view_id == 0 {
                client.runtime_view.view_id = client_id;
            }
            client.runtime_view.viewed_tab_id = viewed_tab_id;
            client.runtime_view.focused_pane_id = focused_pane_id;
            client.runtime_view.visible_pane_ids = visible_pane_ids.to_vec();
            client.runtime_view.ui_attached = true;
            client.runtime_view.detached_at = None;
            client.runtime_view.touch_view();
        });
    }

    pub fn update_client_view(
        &self,
        client_id: u64,
        mut update: impl FnMut(&mut ClientRuntimeView),
    ) {
        self.update_client(client_id, |client| {
            update(&mut client.runtime_view);
        });
    }

    pub fn client_runtime_view(&self, client_id: u64) -> Option<ClientRuntimeViewSnapshot> {
        let state = self.state.lock().expect("remote server state poisoned");
        let client = state.clients.get(&client_id)?;
        Some(ClientRuntimeViewSnapshot::from(&client.runtime_view))
    }

    pub fn subscribe_client_to_runtime(&self, client_id: u64) {
        self.update_client(client_id, |client| {
            if !client.runtime_view.subscribed_to_runtime {
                client.runtime_view.subscribed_to_runtime = true;
                client.runtime_view.clear_stream_state();
            }
        });
        log::info!("remote runtime viewer subscribed: client_id={client_id}");
    }

    pub fn unsubscribe_client_from_runtime(&self, client_id: u64) {
        self.update_client(client_id, |client| {
            client.runtime_view.subscribed_to_runtime = false;
            client.runtime_view.clear_stream_state();
        });
        log::info!("remote runtime view cleared: client_id={client_id}");
    }

    pub fn sweep_idle_views(&self, idle_timeout: std::time::Duration) -> Vec<u64> {
        let mut expired = Vec::new();
        let now = Instant::now();
        let mut state = self.state.lock().expect("remote server state poisoned");
        for (client_id, client) in &mut state.clients {
            let Some(detached_at) = client.runtime_view.detached_at else {
                continue;
            };
            if client.runtime_view.ui_attached {
                continue;
            }
            if now.saturating_duration_since(detached_at) < idle_timeout {
                continue;
            }
            client.runtime_view.subscribed_to_runtime = false;
            client.runtime_view.viewed_tab_id = None;
            client.runtime_view.focused_pane_id = None;
            client.runtime_view.visible_pane_ids.clear();
            client.runtime_view.viewport_cols = None;
            client.runtime_view.viewport_rows = None;
            client.runtime_view.detached_at = None;
            client.runtime_view.clear_stream_state();
            client.runtime_view.touch_view();
            expired.push(*client_id);
        }
        expired
    }

    pub fn send_error(&self, client_id: u64, code: RemoteErrorCode, message: &str) {
        self.send_to_client(
            client_id,
            MessageType::ErrorMsg,
            encode_error_payload(code, message),
        );
    }

    pub fn send_tab_created(&self, client_id: u64, tab_id: u32) {
        self.send_to_client(
            client_id,
            MessageType::TabCreated,
            tab_id.to_le_bytes().to_vec(),
        );
    }

    pub fn send_ui_runtime_state(&self, client_id: u64, state: &crate::control::UiRuntimeState) {
        send_local_ui_runtime_state(&self.state, client_id, state);
    }

    pub fn send_ui_runtime_state_to_local_viewers(&self, state: &crate::control::UiRuntimeState) {
        send_ui_runtime_state_to_local_viewers_inner(&self.state, state);
    }

    #[allow(dead_code)]
    pub fn send_ui_runtime_state_to_viewers(&self, state: &crate::control::UiRuntimeState) {
        let client_ids = {
            let state_guard = self.state.lock().expect("remote server state poisoned");
            state_guard
                .clients
                .iter()
                .filter_map(|(client_id, client)| {
                    client
                        .runtime_view
                        .subscribed_to_runtime
                        .then_some(*client_id)
                })
                .collect::<Vec<_>>()
        };
        for client_id in client_ids {
            send_local_ui_runtime_state(&self.state, client_id, state);
        }
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

    pub fn send_ui_appearance_to_viewers(&self, appearance: &crate::control::UiAppearanceSnapshot) {
        let client_ids = {
            let state_guard = self.state.lock().expect("remote server state poisoned");
            state_guard
                .clients
                .iter()
                .filter_map(|(client_id, client)| {
                    client
                        .runtime_view
                        .subscribed_to_runtime
                        .then_some(*client_id)
                })
                .collect::<Vec<_>>()
        };
        for client_id in client_ids {
            send_local_ui_appearance(&self.state, client_id, appearance);
        }
    }

    #[allow(dead_code)]
    pub fn send_full_state_to_viewers(&self, tab_id: u32, state: Arc<RemoteFullState>) {
        let client_ids = self.viewer_client_ids();
        for client_id in client_ids {
            publish_state_to_client(&self.state, client_id, tab_id, Arc::clone(&state));
        }
    }

    pub fn send_full_state_to_client(&self, client_id: u64, tab_id: u32, state: Arc<RemoteFullState>) {
        publish_state_to_client(&self.state, client_id, tab_id, state);
    }

    #[allow(dead_code)]
    pub fn send_pane_state_to_local_viewers(
        &self,
        tab_id: u32,
        pane_id: u64,
        pane_revision: u64,
        runtime_revision: u64,
        state: Arc<RemoteFullState>,
    ) {
        let client_ids = {
            let state_guard = self.state.lock().expect("remote server state poisoned");
            local_viewer_client_ids(&state_guard)
        };
        for client_id in client_ids {
            publish_pane_state_to_client(
                &self.state,
                client_id,
                tab_id,
                pane_id,
                pane_revision,
                runtime_revision,
                Arc::clone(&state),
            );
        }
    }

    pub fn send_pane_state_to_client(
        &self,
        client_id: u64,
        tab_id: u32,
        pane_id: u64,
        pane_revision: u64,
        runtime_revision: u64,
        state: Arc<RemoteFullState>,
    ) {
        publish_pane_state_to_client(
            &self.state,
            client_id,
            tab_id,
            pane_id,
            pane_revision,
            runtime_revision,
            state,
        );
    }

    pub fn send_pane_rows_to_client(
        &self,
        client_id: u64,
        tab_id: u32,
        pane_id: u64,
        pane_revision: u64,
        runtime_revision: u64,
        response: &RemoteRowRangeResponse,
    ) {
        self.send_to_client(
            client_id,
            MessageType::UiPaneRows,
            crate::remote_wire::encode_ui_pane_update_payload(
                tab_id,
                pane_id,
                pane_revision,
                runtime_revision,
                &encode_row_range_response(response),
            ),
        );
    }

    #[allow(dead_code)]
    pub fn retain_local_viewer_pane_states(&self, visible_pane_ids: &[u64]) {
        let mut guard = self.state.lock().expect("remote server state poisoned");
        retain_local_viewer_pane_states_inner(&mut guard, visible_pane_ids);
    }

    pub fn record_input_seq(&self, client_id: u64, input_seq: Option<u64>) {
        self.update_client(client_id, |client| {
            if let Some(input_seq) = input_seq {
                client.runtime_view.latest_input_seq = Some(input_seq);
            }
        });
    }

    #[allow(dead_code)]
    fn viewer_client_ids(&self) -> Vec<u64> {
        let state = self.state.lock().expect("remote server state poisoned");
        viewer_client_ids(&state)
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientRuntimeViewSnapshot {
    pub view_id: u64,
    pub subscribed_to_runtime: bool,
    pub view_revision: u64,
    pub viewed_tab_id: Option<u32>,
    pub focused_pane_id: Option<u64>,
    pub viewport_cols: Option<u16>,
    pub viewport_rows: Option<u16>,
    pub visible_pane_ids: Vec<u64>,
    pub ui_attached: bool,
    pub acked_client_action_id: Option<u64>,
    pub last_rendered_pane_id: Option<u64>,
    pub last_rendered_pane_revision: Option<u64>,
    pub last_rendered_runtime_revision: Option<u64>,
    pub scroll_offset_rows: i64,
    pub copy_mode_active: bool,
    pub search_active: bool,
    pub search_query: String,
}

impl From<&ClientRuntimeView> for ClientRuntimeViewSnapshot {
    fn from(value: &ClientRuntimeView) -> Self {
        Self {
            view_id: value.view_id,
            subscribed_to_runtime: value.subscribed_to_runtime,
            view_revision: value.view_revision,
            viewed_tab_id: value.viewed_tab_id,
            focused_pane_id: value.focused_pane_id,
            viewport_cols: value.viewport_cols,
            viewport_rows: value.viewport_rows,
            visible_pane_ids: value.visible_pane_ids.clone(),
            ui_attached: value.ui_attached,
            acked_client_action_id: value.acked_client_action_id,
            last_rendered_pane_id: value.last_rendered_pane_id,
            last_rendered_pane_revision: value.last_rendered_pane_revision,
            last_rendered_runtime_revision: value.last_rendered_runtime_revision,
            scroll_offset_rows: value.scroll_offset_rows,
            copy_mode_active: value.copy_mode_active,
            search_active: value.search_active,
            search_query: value.search_query.clone(),
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
