use crate::control;
use crate::remote;
use crate::tabs;
use std::sync::mpsc;

fn advertised_remote_service_name(port: u16) -> String {
    let host = hostname::get()
        .ok()
        .and_then(|name| name.into_string().ok())
        .map(|name| name.trim_end_matches(".local").trim().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "this host".to_string());
    format!("boo on {host} ({port})")
}

#[derive(Debug)]
#[allow(clippy::enum_variant_names)]
pub enum Command {
    DumpKeys(bool),
    Ping,
    GetRemoteClients {
        reply: mpsc::Sender<control::Response>,
    },
    Quit,
    ListSurfaces {
        reply: mpsc::Sender<control::Response>,
    },
    ListTabs {
        reply: mpsc::Sender<control::Response>,
    },
    GetClipboard {
        reply: mpsc::Sender<control::Response>,
    },
    GetUiSnapshot {
        reply: mpsc::Sender<control::Response>,
    },
    SetStatusComponents {
        zone: crate::status_components::StatusBarZone,
        source: String,
        components: Vec<crate::status_components::StatusComponent>,
    },
    ClearStatusComponents {
        source: String,
        zone: Option<crate::status_components::StatusBarZone>,
    },
    InvokeStatusComponent {
        source: String,
        id: String,
    },
    AppKeyEvent {
        event: crate::AppKeyEvent,
    },
    AppMouseEvent {
        event: crate::AppMouseEvent,
    },
    AppAction {
        action: crate::bindings::Action,
    },
    FocusPane {
        pane_id: u64,
    },
    ExecuteCommand {
        input: String,
    },
    SendKey {
        keyspec: String,
    },
    SendText {
        text: String,
    },
    SendVt {
        text: String,
    },
    NewSplit {
        direction: String,
    },
    NewTab,
    GotoTab {
        index: usize,
    },
    NextTab,
    PrevTab,
    ResizeViewportPoints {
        width: f64,
        height: f64,
    },
    ResizeViewport {
        cols: u16,
        rows: u16,
    },
    ResizeFocused {
        cols: u16,
        rows: u16,
    },
    FocusSurface {
        index: usize,
    },
    RemoteConnected {
        client_id: u64,
    },
    RemoteListTabs {
        client_id: u64,
    },
    RemoteCreate {
        client_id: u64,
        cols: u16,
        rows: u16,
    },
    RemoteInput {
        client_id: u64,
        bytes: Vec<u8>,
        input_seq: Option<u64>,
    },
    RemoteKey {
        client_id: u64,
        keyspec: String,
        input_seq: Option<u64>,
    },
    RemoteResize {
        client_id: u64,
        cols: u16,
        rows: u16,
    },
    RemoteExecuteCommand {
        client_id: u64,
        input: String,
    },
    RemoteAppKeyEvent {
        client_id: u64,
        event: crate::AppKeyEvent,
    },
    RemoteAppMouseEvent {
        client_id: u64,
        event: crate::AppMouseEvent,
    },
    RemoteAppAction {
        client_id: u64,
        action: crate::bindings::Action,
    },
    RemoteFocusPane {
        client_id: u64,
        pane_id: u64,
    },
    RemoteDestroy {
        client_id: u64,
        tab_id: Option<u32>,
    },
    RemoteRuntimeAction {
        client_id: u64,
        action: remote::RuntimeAction,
    },
}

pub struct State {
    pub tabs: tabs::TabManager,
    pub socket_path: Option<String>,
    pub ctl_rx: mpsc::Receiver<control::ControlCmd>,
    pub remote_server: Option<remote::RemoteServer>,
    pub remote_rx: mpsc::Receiver<remote::RemoteCmd>,
    pub local_gui_server: Option<remote::RemoteServer>,
    pub local_gui_rx: mpsc::Receiver<remote::RemoteCmd>,
}

impl State {
    pub fn new(
        control_socket: Option<String>,
        remote_port: Option<u16>,
        remote_bind_address: Option<String>,
    ) -> Self {
        let ctl_rx = control::start(control_socket.as_deref());
        let (remote_server, remote_rx) = if let Some(port) = remote_port {
            match remote::RemoteServer::start(remote::RemoteConfig {
                port,
                bind_address: remote_bind_address,
                service_name: advertised_remote_service_name(port),
            }) {
                Ok((server, rx)) => {
                    log::info!("remote daemon listening on quic/{port}");
                    (Some(server), rx)
                }
                Err(error) => {
                    log::error!("failed to start remote daemon on quic/{port}: {error}");
                    let (_tx, rx) = mpsc::channel();
                    (None, rx)
                }
            }
        } else {
            let (_tx, rx) = mpsc::channel();
            (None, rx)
        };

        let (local_gui_server, local_gui_rx) = if let Some(socket_path) = control_socket.as_deref()
        {
            let gui_socket_path = format!("{socket_path}.stream");
            match remote::RemoteServer::start_local(&gui_socket_path) {
                Ok((server, rx)) => {
                    log::info!("local gui stream listening on {gui_socket_path}");
                    (Some(server), rx)
                }
                Err(error) => {
                    log::error!("failed to start local gui stream on {gui_socket_path}: {error}");
                    let (_tx, rx) = mpsc::channel();
                    (None, rx)
                }
            }
        } else {
            let (_tx, rx) = mpsc::channel();
            (None, rx)
        };

        Self {
            tabs: tabs::TabManager::new(),
            socket_path: control_socket,
            ctl_rx,
            remote_server,
            remote_rx,
            local_gui_server,
            local_gui_rx,
        }
    }
}

impl From<control::ControlCmd> for Command {
    fn from(value: control::ControlCmd) -> Self {
        match value {
            control::ControlCmd::DumpKeysOn => Self::DumpKeys(true),
            control::ControlCmd::DumpKeysOff => Self::DumpKeys(false),
            control::ControlCmd::Ping => Self::Ping,
            control::ControlCmd::GetRemoteClients { reply } => Self::GetRemoteClients { reply },
            control::ControlCmd::ListSurfaces { reply } => Self::ListSurfaces { reply },
            control::ControlCmd::ListTabs { reply } => Self::ListTabs { reply },
            control::ControlCmd::GetClipboard { reply } => Self::GetClipboard { reply },
            control::ControlCmd::GetUiSnapshot { reply } => Self::GetUiSnapshot { reply },
            control::ControlCmd::SetStatusComponents {
                zone,
                source,
                components,
            } => Self::SetStatusComponents {
                zone,
                source,
                components,
            },
            control::ControlCmd::ClearStatusComponents { source, zone } => {
                Self::ClearStatusComponents { source, zone }
            }
            control::ControlCmd::InvokeStatusComponent { source, id } => {
                Self::InvokeStatusComponent { source, id }
            }
            control::ControlCmd::AppKeyEvent { event } => Self::AppKeyEvent { event },
            control::ControlCmd::AppMouseEvent { event } => Self::AppMouseEvent { event },
            control::ControlCmd::AppAction { action } => Self::AppAction { action },
            control::ControlCmd::FocusPane { pane_id } => Self::FocusPane { pane_id },
            control::ControlCmd::ExecuteCommand { input } => Self::ExecuteCommand { input },
            control::ControlCmd::SendKey { keyspec } => Self::SendKey { keyspec },
            control::ControlCmd::SendText { text } => Self::SendText { text },
            control::ControlCmd::SendVt { text } => Self::SendVt { text },
            control::ControlCmd::NewSplit { direction } => Self::NewSplit { direction },
            control::ControlCmd::NewTab => Self::NewTab,
            control::ControlCmd::GotoTab { index } => Self::GotoTab { index },
            control::ControlCmd::NextTab => Self::NextTab,
            control::ControlCmd::PrevTab => Self::PrevTab,
            control::ControlCmd::ResizeViewportPoints { width, height } => {
                Self::ResizeViewportPoints { width, height }
            }
            control::ControlCmd::ResizeViewport { cols, rows } => {
                Self::ResizeViewport { cols, rows }
            }
            control::ControlCmd::ResizeFocused { cols, rows } => Self::ResizeFocused { cols, rows },
            control::ControlCmd::FocusSurface { index } => Self::FocusSurface { index },
            control::ControlCmd::Quit => Self::Quit,
        }
    }
}

impl From<remote::RemoteCmd> for Command {
    fn from(value: remote::RemoteCmd) -> Self {
        match value {
            remote::RemoteCmd::Connected { client_id } => Self::RemoteConnected { client_id },
            remote::RemoteCmd::ListTabs { client_id } => Self::RemoteListTabs { client_id },
            remote::RemoteCmd::Create {
                client_id,
                cols,
                rows,
            } => Self::RemoteCreate {
                client_id,
                cols,
                rows,
            },
            remote::RemoteCmd::Input {
                client_id,
                bytes,
                input_seq,
            } => Self::RemoteInput {
                client_id,
                bytes,
                input_seq,
            },
            remote::RemoteCmd::Key {
                client_id,
                keyspec,
                input_seq,
            } => Self::RemoteKey {
                client_id,
                keyspec,
                input_seq,
            },
            remote::RemoteCmd::Resize {
                client_id,
                cols,
                rows,
            } => Self::RemoteResize {
                client_id,
                cols,
                rows,
            },
            remote::RemoteCmd::ExecuteCommand { client_id, input } => {
                Self::RemoteExecuteCommand { client_id, input }
            }
            remote::RemoteCmd::AppKeyEvent { client_id, event } => {
                Self::RemoteAppKeyEvent { client_id, event }
            }
            remote::RemoteCmd::AppMouseEvent { client_id, event } => {
                Self::RemoteAppMouseEvent { client_id, event }
            }
            remote::RemoteCmd::AppAction { client_id, action } => {
                Self::RemoteAppAction { client_id, action }
            }
            remote::RemoteCmd::FocusPane { client_id, pane_id } => {
                Self::RemoteFocusPane { client_id, pane_id }
            }
            remote::RemoteCmd::Destroy { client_id, tab_id } => {
                Self::RemoteDestroy { client_id, tab_id }
            }
            remote::RemoteCmd::RuntimeAction { client_id, action } => {
                Self::RemoteRuntimeAction { client_id, action }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Command, advertised_remote_service_name};
    use crate::control;
    use crate::remote;
    use std::sync::mpsc;

    #[test]
    fn local_control_commands_map_to_server_surface() {
        let (tx, _rx) = mpsc::channel();
        match Command::from(control::ControlCmd::ListTabs { reply: tx }) {
            Command::ListTabs { .. } => {}
            other => panic!("expected list-tabs mapping, got {other:?}"),
        }
        let (tx, _rx) = mpsc::channel();
        match Command::from(control::ControlCmd::GetRemoteClients { reply: tx }) {
            Command::GetRemoteClients { .. } => {}
            other => panic!("expected get-remote-clients mapping, got {other:?}"),
        }
        match Command::from(control::ControlCmd::DumpKeysOn) {
            Command::DumpKeys(true) => {}
            other => panic!("expected dump-keys on mapping, got {other:?}"),
        }
    }

    #[test]
    fn remote_commands_map_to_server_surface() {
        match Command::from(remote::RemoteCmd::AppMouseEvent {
            client_id: 8,
            event: crate::AppMouseEvent::CursorMoved {
                x: 1.0,
                y: 2.0,
                mods: 0,
            },
        }) {
            Command::RemoteAppMouseEvent { client_id, event } => {
                assert_eq!(client_id, 8);
                assert!(matches!(
                    event,
                    crate::AppMouseEvent::CursorMoved { x, y, .. } if x == 1.0 && y == 2.0
                ));
            }
            other => panic!("expected remote app-mouse mapping, got {other:?}"),
        }

        match Command::from(remote::RemoteCmd::FocusPane {
            client_id: 9,
            pane_id: 77,
        }) {
            Command::RemoteFocusPane { client_id, pane_id } => {
                assert_eq!(client_id, 9);
                assert_eq!(pane_id, 77);
            }
            other => panic!("expected remote focus-pane mapping, got {other:?}"),
        }
    }

    #[test]
    fn advertised_service_name_includes_boo_and_port() {
        let name = advertised_remote_service_name(crate::config::DEFAULT_REMOTE_PORT);
        assert!(name.starts_with("boo on "));
        assert!(name.ends_with(&format!("({})", crate::config::DEFAULT_REMOTE_PORT)));
    }
}
