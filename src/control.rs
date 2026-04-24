//! Control socket for boo IPC.
//!
//! Unix domain socket accepting JSON-line commands:
//!   {"cmd": "list-surfaces"}
//!   {"cmd": "new-split", "direction": "right"}
//!   {"cmd": "focus-surface", "index": 1}
//!   {"cmd": "send-key", "key": "ctrl+c"}
//!   {"cmd": "dump-keys", "enabled": true}
//!   {"cmd": "quit"}
//!
//! Responses are JSON lines sent back on the same connection.
//!
//! Also supports the legacy named pipe at /tmp/boo.ctl for simple commands.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::os::unix::net::UnixStream;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Read/write timeout for a one-shot control-socket RPC: connect, write the
/// request, read the reply, hang up. Long enough to tolerate normal GUI
/// scheduling latency; short enough that a stuck server surfaces quickly
/// instead of hanging a CLI caller.
const CONTROL_SOCKET_TIMEOUT: Duration = Duration::from_secs(2);

/// Timeout waiting on an internal `mpsc::Receiver` for the GUI thread to
/// produce a reply to a control-socket request. Same budget as the socket-level
/// timeout above: the reply either arrives within this window or the caller
/// gets a "timeout" error, not an indefinite hang.
const CONTROL_REPLY_TIMEOUT: Duration = Duration::from_secs(2);

/// Shorter reply timeout for clipboard fetches. The clipboard request can
/// complete very quickly when the main thread is idle, but a 2s timeout would
/// leave interactive terminal paste chains feeling sluggish if the GUI thread
/// happens to be mid-frame.
const CLIPBOARD_REPLY_TIMEOUT: Duration = Duration::from_millis(500);

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "kebab-case")]
pub enum Request {
    Ping,
    GetVersion,
    GetRemoteClients,
    ListSurfaces,
    ListTabs,
    GetClipboard,
    GetUiSnapshot,
    GetUiRuntimeState,
    GetUiTextSnapshot,
    SubscribeStatusClicks {
        source: String,
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
    SendText {
        text: String,
    },
    SendVt {
        text: String,
    },
    NewSplit {
        direction: Option<String>,
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
    SendKey {
        key: String,
    },
    DumpKeys {
        enabled: bool,
    },
    Quit,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
#[allow(clippy::large_enum_variant)]
pub enum Response {
    Version {
        version: String,
    },
    RemoteClients {
        snapshot: crate::remote::RemoteClientsSnapshot,
    },
    Surfaces {
        surfaces: Vec<SurfaceInfo>,
    },
    Tabs {
        tabs: Vec<crate::tabs::TabInfo>,
    },
    Clipboard {
        text: String,
    },
    UiSnapshot {
        snapshot: UiSnapshot,
    },
    UiRuntimeState {
        state: UiRuntimeState,
    },
    UiTextSnapshot {
        snapshot: UiTextSnapshot,
    },
    Ok {
        ok: bool,
    },
    Error {
        error: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StatusClickEvent {
    pub event: String,
    pub source: String,
    pub id: String,
    pub button: String,
    pub x_offset: f64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SurfaceInfo {
    pub index: usize,
    pub focused: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UiSnapshot {
    pub active_tab: usize,
    pub focused_pane: u64,
    pub appearance: UiAppearanceSnapshot,
    pub tabs: Vec<UiTabSnapshot>,
    pub visible_panes: Vec<UiPaneSnapshot>,
    pub pane_terminals: Vec<UiPaneTerminalSnapshot>,
    pub copy_mode: UiCopyModeSnapshot,
    pub mouse_selection: UiMouseSelectionSnapshot,
    pub search: UiSearchSnapshot,
    pub command_prompt: UiCommandPromptSnapshot,
    pub status_bar: crate::status_components::UiStatusBarSnapshot,
    pub pwd: String,
    pub scrollbar: UiScrollbarSnapshot,
    pub terminal: Option<UiTerminalSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UiRuntimeState {
    pub active_tab: usize,
    pub focused_pane: u64,
    pub tabs: Vec<UiTabSnapshot>,
    pub visible_panes: Vec<UiPaneSnapshot>,
    pub mouse_selection: UiMouseSelectionSnapshot,
    pub status_bar: crate::status_components::UiStatusBarSnapshot,
    pub pwd: String,
    pub runtime_revision: u64,
    pub view_revision: u64,
    pub view_id: u64,
    pub viewed_tab_id: Option<u32>,
    pub viewport_cols: Option<u16>,
    pub viewport_rows: Option<u16>,
    pub visible_pane_ids: Vec<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UiTextSnapshot {
    pub active_tab: usize,
    pub focused_pane: u64,
    pub tabs: Vec<UiTabSnapshot>,
    pub visible_panes: Vec<UiPaneSnapshot>,
    pub status_bar: crate::status_components::UiStatusBarSnapshot,
    pub pane_texts: Vec<UiPaneTextSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UiPaneTextSnapshot {
    pub pane_id: u64,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UiAppearanceSnapshot {
    pub font_families: Vec<String>,
    pub font_size: f32,
    pub background_opacity: f32,
    pub background_opacity_cells: bool,
    pub terminal_foreground: crate::config::RgbColor,
    pub terminal_background: crate::config::RgbColor,
    pub cursor_color: crate::config::RgbColor,
    pub selection_background: crate::config::RgbColor,
    pub selection_foreground: crate::config::RgbColor,
    pub cursor_text_color: crate::config::RgbColor,
    pub url_color: crate::config::RgbColor,
    pub active_tab_foreground: crate::config::RgbColor,
    pub active_tab_background: crate::config::RgbColor,
    pub inactive_tab_foreground: crate::config::RgbColor,
    pub inactive_tab_background: crate::config::RgbColor,
    pub cursor_style: Option<i32>,
    pub cursor_blink: bool,
    pub cursor_blink_interval_ns: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UiTabSnapshot {
    pub tab_id: u32,
    pub index: usize,
    pub active: bool,
    pub title: String,
    pub pane_count: usize,
    pub focused_pane: Option<u64>,
    pub pane_ids: Vec<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UiPaneSnapshot {
    pub leaf_index: usize,
    pub leaf_id: usize,
    pub pane_id: u64,
    pub focused: bool,
    pub frame: UiRectSnapshot,
    pub split_direction: Option<String>,
    pub split_ratio: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UiPaneTerminalSnapshot {
    pub pane_id: u64,
    pub terminal: UiTerminalSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UiRectSnapshot {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UiCopyModeSnapshot {
    pub active: bool,
    pub cursor_row: i64,
    pub cursor_col: u32,
    pub selection_mode: String,
    pub has_selection_anchor: bool,
    pub anchor_row: Option<i64>,
    pub anchor_col: Option<u32>,
    pub selection_rects: Vec<UiRectSnapshot>,
    pub show_position: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct UiMouseSelectionSnapshot {
    pub active: bool,
    pub pane_id: Option<u64>,
    pub selection_rects: Vec<UiRectSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UiSearchSnapshot {
    pub active: bool,
    pub query: String,
    pub total: isize,
    pub selected: isize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UiCommandPromptSnapshot {
    pub active: bool,
    pub input: String,
    pub selected_suggestion: usize,
    pub suggestions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UiScrollbarSnapshot {
    pub total: u64,
    pub offset: u64,
    pub len: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UiTerminalSnapshot {
    pub cols: u16,
    pub rows: u16,
    pub title: String,
    pub pwd: String,
    pub cursor: UiCursorSnapshot,
    pub rows_data: Vec<UiTerminalRowSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UiCursorSnapshot {
    pub visible: bool,
    pub blinking: bool,
    pub x: u16,
    pub y: u16,
    pub style: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UiTerminalRowSnapshot {
    pub cells: Vec<UiTerminalCellSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UiTerminalCellSnapshot {
    pub text: String,
    pub display_width: u8,
    pub fg: [u8; 3],
    pub bg: [u8; 3],
    #[serde(default = "default_true")]
    pub bg_is_default: bool,
    pub bold: bool,
    pub italic: bool,
    pub underline: i32,
    pub hyperlink: bool,
}

fn default_true() -> bool {
    true
}

/// Commands sent from the control thread to the main iced update loop.
#[derive(Debug)]
pub enum ControlCmd {
    DumpKeysOn,
    DumpKeysOff,
    Ping,
    GetRemoteClients {
        reply: mpsc::Sender<Response>,
    },
    ListSurfaces {
        reply: mpsc::Sender<Response>,
    },
    ListTabs {
        reply: mpsc::Sender<Response>,
    },
    GetClipboard {
        reply: mpsc::Sender<Response>,
    },
    GetUiSnapshot {
        reply: mpsc::Sender<Response>,
    },
    GetUiRuntimeState {
        reply: mpsc::Sender<Response>,
    },
    GetUiTextSnapshot {
        reply: mpsc::Sender<Response>,
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
    Quit,
}

pub struct Client {
    socket_path: String,
}

impl Client {
    pub fn connect(socket_path: impl Into<String>) -> Self {
        Self {
            socket_path: socket_path.into(),
        }
    }

    pub fn request(&self, request: &Request) -> Result<Response, String> {
        let mut stream = UnixStream::connect(&self.socket_path)
            .map_err(|error| format!("connect {}: {}", self.socket_path, error))?;
        let _ = stream.set_read_timeout(Some(CONTROL_SOCKET_TIMEOUT));
        let _ = stream.set_write_timeout(Some(CONTROL_SOCKET_TIMEOUT));
        serde_json::to_writer(&mut stream, request)
            .map_err(|error| format!("serialize request: {error}"))?;
        stream
            .write_all(b"\n")
            .map_err(|error| format!("write request: {error}"))?;
        stream
            .flush()
            .map_err(|error| format!("flush request: {error}"))?;
        let _ = stream.shutdown(std::net::Shutdown::Write);

        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .map_err(|error| format!("read response: {error}"))?;
        if line.is_empty() {
            return Err("empty response from control socket".to_string());
        }
        serde_json::from_str(line.trim())
            .map_err(|error| format!("parse response: {error}; raw={line:?}"))
    }

    pub fn get_ui_snapshot(&self) -> Result<UiSnapshot, String> {
        match self.request(&Request::GetUiSnapshot)? {
            Response::UiSnapshot { snapshot } => Ok(snapshot),
            Response::Error { error } => Err(error),
            other => Err(format!("unexpected response: {other:?}")),
        }
    }

    pub fn ping(&self) -> Result<(), String> {
        match self.request(&Request::Ping)? {
            Response::Ok { ok: true } => Ok(()),
            Response::Error { error } => Err(error),
            other => Err(format!("unexpected response: {other:?}")),
        }
    }

    pub fn get_version(&self) -> Result<String, String> {
        match self.request(&Request::GetVersion)? {
            Response::Version { version } => Ok(version),
            Response::Error { error } => Err(error),
            other => Err(format!("unexpected response: {other:?}")),
        }
    }

    pub fn get_remote_clients(&self) -> Result<crate::remote::RemoteClientsSnapshot, String> {
        match self.request(&Request::GetRemoteClients)? {
            Response::RemoteClients { snapshot } => Ok(snapshot),
            Response::Error { error } => Err(error),
            other => Err(format!("unexpected response: {other:?}")),
        }
    }

    pub fn send(&self, request: &Request) -> Result<(), String> {
        match self.request(request)? {
            Response::Ok { ok: true } => Ok(()),
            Response::Error { error } => Err(error),
            _ => Ok(()),
        }
    }
}

pub fn default_socket_path() -> String {
    "/tmp/boo.sock".to_string()
}

pub fn start(socket_path: Option<&str>) -> mpsc::Receiver<ControlCmd> {
    let (tx, rx) = mpsc::channel();

    if let Some(path) = socket_path {
        let path = path.to_owned();
        let tx = tx.clone();
        std::thread::spawn(move || run_socket(&path, &tx));
    }

    // Also keep the legacy named pipe for simple testing
    {
        let tx = tx;
        std::thread::spawn(move || run_pipe(&tx));
    }

    rx
}

type StatusClickSubscribers = HashMap<String, Vec<UnixStream>>;

fn status_click_subscribers() -> &'static Arc<Mutex<StatusClickSubscribers>> {
    static SUBSCRIBERS: std::sync::OnceLock<Arc<Mutex<StatusClickSubscribers>>> =
        std::sync::OnceLock::new();
    SUBSCRIBERS.get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
}

pub fn notify_status_click(source: &str, id: &str, button: &str, x_offset: f64) {
    let Ok(mut guard) = status_click_subscribers().lock() else {
        return;
    };
    let Some(subscribers) = guard.get_mut(source) else {
        return;
    };
    let event = StatusClickEvent {
        event: "status-click".to_string(),
        source: source.to_string(),
        id: id.to_string(),
        button: button.to_string(),
        x_offset,
    };
    let payload = match serde_json::to_vec(&event) {
        Ok(mut payload) => {
            payload.push(b'\n');
            payload
        }
        Err(_) => return,
    };
    subscribers.retain_mut(|stream| stream.write_all(&payload).is_ok() && stream.flush().is_ok());
    if subscribers.is_empty() {
        guard.remove(source);
    }
}

fn register_status_click_subscription(source: String, stream: &UnixStream) -> Response {
    if source.is_empty() {
        return Response::Error {
            error: "source must not be empty".to_string(),
        };
    }
    let Ok(cloned) = stream.try_clone() else {
        return Response::Error {
            error: "failed to clone subscriber stream".to_string(),
        };
    };
    let _ = cloned.set_write_timeout(Some(CONTROL_SOCKET_TIMEOUT));
    let Ok(mut guard) = status_click_subscribers().lock() else {
        return Response::Error {
            error: "subscriber registry poisoned".to_string(),
        };
    };
    guard.entry(source).or_default().push(cloned);
    Response::Ok { ok: true }
}

fn run_socket(path: &str, tx: &mpsc::Sender<ControlCmd>) {
    let _ = std::fs::remove_file(path);
    let listener = match UnixListener::bind(path) {
        Ok(l) => l,
        Err(e) => {
            log::error!("control socket bind({path}): {e}");
            return;
        }
    };
    log::info!("control socket: {path}");

    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let tx = tx.clone();
        std::thread::spawn(move || {
            let reader = BufReader::new(&stream);
            for line in reader.lines() {
                let Ok(line) = line else { break };
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                match serde_json::from_str::<Request>(line) {
                    Ok(req) => {
                        let resp = match req {
                            Request::SubscribeStatusClicks { source } => {
                                register_status_click_subscription(source, &stream)
                            }
                            other => dispatch_request(other, &tx),
                        };
                        let mut writer = &stream;
                        let _ = serde_json::to_writer(&mut writer, &resp);
                        let _ = writer.write_all(b"\n");
                        let _ = writer.flush();
                    }
                    Err(e) => {
                        let resp = Response::Error {
                            error: format!("parse error: {e}"),
                        };
                        let mut writer = &stream;
                        let _ = serde_json::to_writer(&mut writer, &resp);
                        let _ = writer.write_all(b"\n");
                        let _ = writer.flush();
                    }
                }
            }
        });
    }
}

fn dispatch_request(req: Request, tx: &mpsc::Sender<ControlCmd>) -> Response {
    let notify = || crate::notify_headless_wakeup();
    match req {
        Request::Ping => {
            let _ = tx.send(ControlCmd::Ping);
            notify();
            Response::Ok { ok: true }
        }
        Request::GetVersion => Response::Version {
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
        Request::GetRemoteClients => {
            let (reply_tx, reply_rx) = mpsc::channel();
            let _ = tx.send(ControlCmd::GetRemoteClients { reply: reply_tx });
            notify();
            match reply_rx.recv_timeout(CONTROL_REPLY_TIMEOUT) {
                Ok(resp) => resp,
                Err(_) => Response::Error {
                    error: "timeout".into(),
                },
            }
        }
        Request::Quit => {
            let _ = tx.send(ControlCmd::Quit);
            notify();
            Response::Ok { ok: true }
        }
        Request::DumpKeys { enabled } => {
            let _ = tx.send(if enabled {
                ControlCmd::DumpKeysOn
            } else {
                ControlCmd::DumpKeysOff
            });
            notify();
            Response::Ok { ok: true }
        }
        Request::SendKey { key } => {
            let _ = tx.send(ControlCmd::SendKey { keyspec: key });
            notify();
            Response::Ok { ok: true }
        }
        Request::ExecuteCommand { input } => {
            let _ = tx.send(ControlCmd::ExecuteCommand { input });
            notify();
            Response::Ok { ok: true }
        }
        Request::AppKeyEvent { event } => {
            let _ = tx.send(ControlCmd::AppKeyEvent { event });
            notify();
            Response::Ok { ok: true }
        }
        Request::AppMouseEvent { event } => {
            let _ = tx.send(ControlCmd::AppMouseEvent { event });
            notify();
            Response::Ok { ok: true }
        }
        Request::AppAction { action } => {
            let _ = tx.send(ControlCmd::AppAction { action });
            notify();
            Response::Ok { ok: true }
        }
        Request::FocusPane { pane_id } => {
            let _ = tx.send(ControlCmd::FocusPane { pane_id });
            notify();
            Response::Ok { ok: true }
        }
        Request::SendText { text } => {
            let _ = tx.send(ControlCmd::SendText { text });
            notify();
            Response::Ok { ok: true }
        }
        Request::SendVt { text } => {
            let _ = tx.send(ControlCmd::SendVt { text });
            notify();
            Response::Ok { ok: true }
        }
        Request::GetClipboard => {
            let (reply_tx, reply_rx) = mpsc::channel();
            let _ = tx.send(ControlCmd::GetClipboard { reply: reply_tx });
            notify();
            reply_rx
                .recv_timeout(CLIPBOARD_REPLY_TIMEOUT)
                .unwrap_or(Response::Error {
                    error: "clipboard request timed out".to_string(),
                })
        }
        Request::NewSplit { direction } => {
            let _ = tx.send(ControlCmd::NewSplit {
                direction: direction.unwrap_or_else(|| "right".into()),
            });
            notify();
            Response::Ok { ok: true }
        }
        Request::FocusSurface { index } => {
            let _ = tx.send(ControlCmd::FocusSurface { index });
            notify();
            Response::Ok { ok: true }
        }
        Request::ListSurfaces => {
            let (reply_tx, reply_rx) = mpsc::channel();
            let _ = tx.send(ControlCmd::ListSurfaces { reply: reply_tx });
            notify();
            match reply_rx.recv_timeout(CONTROL_REPLY_TIMEOUT) {
                Ok(resp) => resp,
                Err(_) => Response::Error {
                    error: "timeout".into(),
                },
            }
        }
        Request::ListTabs => {
            let (reply_tx, reply_rx) = mpsc::channel();
            let _ = tx.send(ControlCmd::ListTabs { reply: reply_tx });
            notify();
            match reply_rx.recv_timeout(CONTROL_REPLY_TIMEOUT) {
                Ok(resp) => resp,
                Err(_) => Response::Error {
                    error: "timeout".into(),
                },
            }
        }
        Request::GetUiSnapshot => {
            let (reply_tx, reply_rx) = mpsc::channel();
            let _ = tx.send(ControlCmd::GetUiSnapshot { reply: reply_tx });
            notify();
            match reply_rx.recv_timeout(CONTROL_REPLY_TIMEOUT) {
                Ok(resp) => resp,
                Err(_) => Response::Error {
                    error: "timeout".into(),
                },
            }
        }
        Request::GetUiRuntimeState => {
            let (reply_tx, reply_rx) = mpsc::channel();
            let _ = tx.send(ControlCmd::GetUiRuntimeState { reply: reply_tx });
            notify();
            match reply_rx.recv_timeout(CONTROL_REPLY_TIMEOUT) {
                Ok(resp) => resp,
                Err(_) => Response::Error {
                    error: "timeout".into(),
                },
            }
        }
        Request::GetUiTextSnapshot => {
            let (reply_tx, reply_rx) = mpsc::channel();
            let _ = tx.send(ControlCmd::GetUiTextSnapshot { reply: reply_tx });
            notify();
            match reply_rx.recv_timeout(CONTROL_REPLY_TIMEOUT) {
                Ok(resp) => resp,
                Err(_) => Response::Error {
                    error: "timeout".into(),
                },
            }
        }
        Request::SubscribeStatusClicks { .. } => Response::Error {
            error: "status click subscriptions require a persistent control socket connection"
                .to_string(),
        },
        Request::SetStatusComponents {
            zone,
            source,
            components,
        } => {
            let _ = tx.send(ControlCmd::SetStatusComponents {
                zone,
                source,
                components,
            });
            notify();
            Response::Ok { ok: true }
        }
        Request::ClearStatusComponents { source, zone } => {
            let _ = tx.send(ControlCmd::ClearStatusComponents { source, zone });
            notify();
            Response::Ok { ok: true }
        }
        Request::InvokeStatusComponent { source, id } => {
            let _ = tx.send(ControlCmd::InvokeStatusComponent { source, id });
            notify();
            Response::Ok { ok: true }
        }
        Request::NewTab => {
            let _ = tx.send(ControlCmd::NewTab);
            notify();
            Response::Ok { ok: true }
        }
        Request::GotoTab { index } => {
            let _ = tx.send(ControlCmd::GotoTab { index });
            notify();
            Response::Ok { ok: true }
        }
        Request::NextTab => {
            let _ = tx.send(ControlCmd::NextTab);
            notify();
            Response::Ok { ok: true }
        }
        Request::PrevTab => {
            let _ = tx.send(ControlCmd::PrevTab);
            notify();
            Response::Ok { ok: true }
        }
        Request::ResizeViewportPoints { width, height } => {
            let _ = tx.send(ControlCmd::ResizeViewportPoints { width, height });
            notify();
            Response::Ok { ok: true }
        }
        Request::ResizeViewport { cols, rows } => {
            let _ = tx.send(ControlCmd::ResizeViewport { cols, rows });
            notify();
            Response::Ok { ok: true }
        }
        Request::ResizeFocused { cols, rows } => {
            let _ = tx.send(ControlCmd::ResizeFocused { cols, rows });
            notify();
            Response::Ok { ok: true }
        }
    }
}

const PIPE_PATH: &str = "/tmp/boo.ctl";

fn run_pipe(tx: &mpsc::Sender<ControlCmd>) {
    let _ = std::fs::remove_file(PIPE_PATH);
    unsafe {
        let path = std::ffi::CString::new(PIPE_PATH).unwrap();
        libc::mkfifo(path.as_ptr(), 0o644);
    }
    log::info!("control pipe: {PIPE_PATH}");

    loop {
        let Ok(file) = std::fs::File::open(PIPE_PATH) else {
            break;
        };
        let reader = BufReader::new(file);
        for line in reader.lines() {
            let Ok(line) = line else { break };
            let line = line.trim();
            let cmd = match line {
                "dump-keys on" => Some(ControlCmd::DumpKeysOn),
                "dump-keys off" => Some(ControlCmd::DumpKeysOff),
                "quit" => Some(ControlCmd::Quit),
                _ if line.starts_with("key ") => Some(ControlCmd::SendKey {
                    keyspec: line[4..].to_owned(),
                }),
                _ if line.starts_with("{") => {
                    // JSON on the pipe — dispatch like socket
                    if let Ok(req) = serde_json::from_str::<Request>(line) {
                        let _ = dispatch_request(req, tx);
                    }
                    None
                }
                _ => {
                    log::info!("control: unknown command: {line}");
                    None
                }
            };
            if let Some(cmd) = cmd {
                if tx.send(cmd).is_err() {
                    return;
                }
                crate::notify_headless_wakeup();
            }
        }
    }
}

pub fn cleanup(socket_path: Option<&str>) {
    let _ = std::fs::remove_file(PIPE_PATH);
    if let Some(path) = socket_path {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ControlCmd, Request, Response, UiAppearanceSnapshot, UiCommandPromptSnapshot,
        UiCopyModeSnapshot, UiMouseSelectionSnapshot, UiPaneSnapshot, UiRectSnapshot,
        UiScrollbarSnapshot, UiSearchSnapshot, UiSnapshot, UiTabSnapshot, dispatch_request,
    };
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn dump_keys_request_maps_to_control_command() {
        let (tx, rx) = mpsc::channel();

        let response = dispatch_request(Request::DumpKeys { enabled: true }, &tx);

        assert!(matches!(response, Response::Ok { ok: true }));
        assert!(matches!(rx.recv().unwrap(), ControlCmd::DumpKeysOn));
    }

    #[test]
    fn new_split_defaults_to_right() {
        let (tx, rx) = mpsc::channel();

        let response = dispatch_request(Request::NewSplit { direction: None }, &tx);

        assert!(matches!(response, Response::Ok { ok: true }));
        assert!(matches!(
            rx.recv().unwrap(),
            ControlCmd::NewSplit { direction } if direction == "right"
        ));
    }

    #[test]
    fn execute_command_request_maps_to_control_command() {
        let (tx, rx) = mpsc::channel();

        let response = dispatch_request(
            Request::ExecuteCommand {
                input: "search".to_string(),
            },
            &tx,
        );

        assert!(matches!(response, Response::Ok { ok: true }));
        assert!(matches!(
            rx.recv().unwrap(),
            ControlCmd::ExecuteCommand { input } if input == "search"
        ));
    }

    #[test]
    fn app_key_event_request_maps_to_control_command() {
        let (tx, rx) = mpsc::channel();

        let response = dispatch_request(
            Request::AppKeyEvent {
                event: crate::AppKeyEvent {
                    keycode: 0x27,
                    mods: crate::ffi::GHOSTTY_MODS_SHIFT,
                    text: Some("\"".to_string()),
                    modified_text: Some("\"".to_string()),
                    named_key: None,
                    repeat: false,
                    input_seq: Some(7),
                },
            },
            &tx,
        );

        assert!(matches!(response, Response::Ok { ok: true }));
        assert!(matches!(
            rx.recv().unwrap(),
            ControlCmd::AppKeyEvent { event }
                if event.keycode == 0x27
                    && event.input_seq == Some(7)
                    && event.text.as_deref() == Some("\"")
        ));
    }

    #[test]
    fn app_action_request_maps_to_control_command() {
        let (tx, rx) = mpsc::channel();

        let response = dispatch_request(
            Request::AppAction {
                action: crate::bindings::Action::NewSplit(crate::bindings::SplitDirection::Right),
            },
            &tx,
        );

        assert!(matches!(response, Response::Ok { ok: true }));
        assert!(matches!(
            rx.recv().unwrap(),
            ControlCmd::AppAction {
                action: crate::bindings::Action::NewSplit(crate::bindings::SplitDirection::Right),
            }
        ));
    }

    #[test]
    fn app_mouse_event_request_maps_to_control_command() {
        let (tx, rx) = mpsc::channel();

        let response = dispatch_request(
            Request::AppMouseEvent {
                event: crate::AppMouseEvent::ButtonPressed {
                    button: crate::AppMouseButton::Left,
                    x: 10.0,
                    y: 20.0,
                    mods: 0,
                },
            },
            &tx,
        );

        assert!(matches!(response, Response::Ok { ok: true }));
        assert!(matches!(
            rx.recv().unwrap(),
            ControlCmd::AppMouseEvent {
                event: crate::AppMouseEvent::ButtonPressed { x, y, .. },
            } if x == 10.0 && y == 20.0
        ));
    }

    #[test]
    fn focus_pane_request_maps_to_control_command() {
        let (tx, rx) = mpsc::channel();

        let response = dispatch_request(Request::FocusPane { pane_id: 42 }, &tx);

        assert!(matches!(response, Response::Ok { ok: true }));
        assert!(matches!(
            rx.recv().unwrap(),
            ControlCmd::FocusPane { pane_id } if pane_id == 42
        ));
    }

    #[test]
    fn send_text_request_maps_to_control_command() {
        let (tx, rx) = mpsc::channel();

        let response = dispatch_request(
            Request::SendText {
                text: "pwd\r".to_string(),
            },
            &tx,
        );

        assert!(matches!(response, Response::Ok { ok: true }));
        assert!(matches!(
            rx.recv().unwrap(),
            ControlCmd::SendText { text } if text == "pwd\r"
        ));
    }

    #[test]
    fn send_vt_request_maps_to_control_command() {
        let (tx, rx) = mpsc::channel();

        let response = dispatch_request(
            Request::SendVt {
                text: "\u{1b}[1mSTYLE\u{1b}[0m".to_string(),
            },
            &tx,
        );

        assert!(matches!(response, Response::Ok { ok: true }));
        assert!(matches!(
            rx.recv().unwrap(),
            ControlCmd::SendVt { text } if text == "\u{1b}[1mSTYLE\u{1b}[0m"
        ));
    }

    #[test]
    fn get_clipboard_round_trips_reply() {
        let (tx, rx) = mpsc::channel();
        let worker = std::thread::spawn(move || match rx.recv().unwrap() {
            ControlCmd::GetClipboard { reply } => {
                reply
                    .send(Response::Clipboard {
                        text: "copied text".to_string(),
                    })
                    .unwrap();
            }
            other => panic!("unexpected command: {other:?}"),
        });

        let response = dispatch_request(Request::GetClipboard, &tx);
        worker.join().unwrap();

        match response {
            Response::Clipboard { text } => assert_eq!(text, "copied text"),
            other => panic!("unexpected response: {other:?}"),
        }
    }

    #[test]
    fn list_surfaces_round_trips_reply() {
        let (tx, rx) = mpsc::channel();
        let worker = std::thread::spawn(move || match rx.recv().unwrap() {
            ControlCmd::ListSurfaces { reply } => {
                reply
                    .send(Response::Surfaces {
                        surfaces: vec![super::SurfaceInfo {
                            index: 0,
                            focused: true,
                        }],
                    })
                    .unwrap();
            }
            other => panic!("unexpected command: {other:?}"),
        });

        let response = dispatch_request(Request::ListSurfaces, &tx);
        worker.join().unwrap();

        match response {
            Response::Surfaces { surfaces } => {
                assert_eq!(surfaces.len(), 1);
                assert_eq!(surfaces[0].index, 0);
                assert!(surfaces[0].focused);
            }
            other => panic!("unexpected response: {other:?}"),
        }
    }

    #[test]
    fn list_tabs_times_out_without_reply() {
        let (tx, rx) = mpsc::channel::<ControlCmd>();
        let worker = std::thread::spawn(move || {
            let _ = rx.recv_timeout(Duration::from_millis(50));
        });

        let response = dispatch_request(Request::ListTabs, &tx);
        worker.join().unwrap();

        assert!(matches!(response, Response::Error { error } if error == "timeout"));
    }

    #[test]
    fn ping_maps_to_control_command() {
        let (tx, rx) = mpsc::channel();
        let worker = std::thread::spawn(move || match rx.recv().unwrap() {
            ControlCmd::Ping => {}
            other => panic!("unexpected command: {other:?}"),
        });

        let response = dispatch_request(Request::Ping, &tx);
        worker.join().unwrap();

        assert!(matches!(response, Response::Ok { ok: true }));
    }

    #[test]
    fn get_version_returns_local_package_version() {
        let (tx, _rx) = mpsc::channel();

        let response = dispatch_request(Request::GetVersion, &tx);

        assert!(matches!(
            response,
            Response::Version { version } if version == env!("CARGO_PKG_VERSION")
        ));
    }

    #[test]
    fn get_remote_clients_round_trips_reply() {
        let (tx, rx) = mpsc::channel();
        let worker = std::thread::spawn(move || match rx.recv().unwrap() {
            ControlCmd::GetRemoteClients { reply } => {
                reply
                    .send(Response::RemoteClients {
                        snapshot: crate::remote::RemoteClientsSnapshot {
                            servers: vec![crate::remote::RemoteServerInfo {
                                local_socket_path: Some("/tmp/boo.sock".to_string()),
                                bind_address: Some("0.0.0.0".to_string()),
                                port: Some(crate::config::DEFAULT_REMOTE_PORT),
                                protocol_version: 1,
                                capabilities: crate::remote::REMOTE_CAPABILITIES,
                                build_id: env!("CARGO_PKG_VERSION").to_string(),
                                server_instance_id: "test-instance".to_string(),
                                server_identity_id: "test-daemon".to_string(),
                                auth_challenge_window_ms: 10_000,
                                heartbeat_window_ms: 20_000,
                                connected_clients: 1,
                                viewing_clients: 1,
                                pending_auth_clients: 0,
                            }],
                            clients: vec![crate::remote::RemoteClientInfo {
                                client_id: 7,
                                authenticated: true,
                                is_local: false,
                                transport_kind: "tcp".to_string(),
                                server_socket_path: Some("/tmp/boo.sock".to_string()),
                                challenge_pending: false,
                                subscribed_to_runtime: true,
                                view_id: 12,
                                viewed_tab_id: Some(1),
                                focused_pane_id: Some(3),
                                visible_pane_count: 1,
                                has_cached_state: true,
                                pane_state_count: 1,
                                latest_input_seq: Some(9),
                                connection_age_ms: 0,
                                authenticated_age_ms: Some(0),
                                last_heartbeat_age_ms: Some(0),
                                heartbeat_expires_in_ms: Some(20_000),
                                heartbeat_overdue: false,
                                challenge_expires_in_ms: None,
                            }],
                        },
                    })
                    .unwrap();
            }
            other => panic!("unexpected command: {other:?}"),
        });

        let response = dispatch_request(Request::GetRemoteClients, &tx);
        worker.join().unwrap();

        match response {
            Response::RemoteClients { snapshot } => {
                assert_eq!(snapshot.clients.len(), 1);
                assert_eq!(snapshot.clients[0].client_id, 7);
                assert!(snapshot.clients[0].subscribed_to_runtime);
            }
            other => panic!("unexpected response: {other:?}"),
        }
    }

    #[test]
    fn get_ui_snapshot_round_trips_reply() {
        let (tx, rx) = mpsc::channel();
        let worker = std::thread::spawn(move || match rx.recv().unwrap() {
            ControlCmd::GetUiSnapshot { reply } => {
                reply
                    .send(Response::UiSnapshot {
                        snapshot: UiSnapshot {
                            active_tab: 0,
                            focused_pane: 42,
                            appearance: UiAppearanceSnapshot {
                                font_families: vec![
                                    "JetBrains Mono".to_string(),
                                    "Apple Color Emoji".to_string(),
                                ],
                                font_size: 14.0,
                                background_opacity: 0.8,
                                background_opacity_cells: true,
                                terminal_foreground: crate::DEFAULT_TERMINAL_FOREGROUND,
                                terminal_background: crate::DEFAULT_TERMINAL_BACKGROUND,
                                cursor_color: crate::DEFAULT_CURSOR_COLOR,
                                selection_background: crate::DEFAULT_SELECTION_BACKGROUND,
                                selection_foreground: crate::DEFAULT_SELECTION_FOREGROUND,
                                cursor_text_color: crate::DEFAULT_CURSOR_TEXT_COLOR,
                                url_color: crate::DEFAULT_URL_COLOR,
                                active_tab_foreground: crate::DEFAULT_ACTIVE_TAB_FOREGROUND,
                                active_tab_background: crate::DEFAULT_ACTIVE_TAB_BACKGROUND,
                                inactive_tab_foreground: crate::DEFAULT_INACTIVE_TAB_FOREGROUND,
                                inactive_tab_background: crate::DEFAULT_INACTIVE_TAB_BACKGROUND,
                                cursor_style: Some(3),
                                cursor_blink: true,
                                cursor_blink_interval_ns: 600_000_000,
                            },
                            tabs: vec![UiTabSnapshot {
                                tab_id: 1,
                                index: 0,
                                active: true,
                                title: "shell".to_string(),
                                pane_count: 1,
                                focused_pane: Some(42),
                                pane_ids: vec![42],
                            }],
                            visible_panes: vec![UiPaneSnapshot {
                                leaf_index: 0,
                                leaf_id: 0,
                                pane_id: 42,
                                focused: true,
                                frame: UiRectSnapshot {
                                    x: 0.0,
                                    y: 20.0,
                                    width: 100.0,
                                    height: 80.0,
                                },
                                split_direction: None,
                                split_ratio: None,
                            }],
                            pane_terminals: Vec::new(),
                            copy_mode: UiCopyModeSnapshot {
                                active: false,
                                cursor_row: 0,
                                cursor_col: 0,
                                selection_mode: "none".to_string(),
                                has_selection_anchor: false,
                                anchor_row: None,
                                anchor_col: None,
                                selection_rects: Vec::new(),
                                show_position: false,
                            },
                            mouse_selection: UiMouseSelectionSnapshot::default(),
                            search: UiSearchSnapshot {
                                active: false,
                                query: String::new(),
                                total: 0,
                                selected: 0,
                            },
                            command_prompt: UiCommandPromptSnapshot {
                                active: false,
                                input: String::new(),
                                selected_suggestion: 0,
                                suggestions: Vec::new(),
                            },
                            status_bar: crate::status_components::UiStatusBarSnapshot::default(),
                            pwd: "/tmp".to_string(),
                            scrollbar: UiScrollbarSnapshot {
                                total: 10,
                                offset: 0,
                                len: 10,
                            },
                            terminal: None,
                        },
                    })
                    .unwrap();
            }
            other => panic!("unexpected command: {other:?}"),
        });

        let response = dispatch_request(Request::GetUiSnapshot, &tx);
        worker.join().unwrap();

        match response {
            Response::UiSnapshot { snapshot } => {
                assert_eq!(snapshot.focused_pane, 42);
                assert_eq!(snapshot.tabs.len(), 1);
                assert_eq!(snapshot.visible_panes.len(), 1);
            }
            other => panic!("unexpected response: {other:?}"),
        }
    }

    #[test]
    fn ui_snapshot_response_serializes_to_json() {
        let response = Response::UiSnapshot {
            snapshot: UiSnapshot {
                active_tab: 1,
                focused_pane: 7,
                appearance: UiAppearanceSnapshot {
                    font_families: vec!["Fira Code".to_string(), "Apple Color Emoji".to_string()],
                    font_size: 15.0,
                    background_opacity: 0.7,
                    background_opacity_cells: false,
                    terminal_foreground: crate::DEFAULT_TERMINAL_FOREGROUND,
                    terminal_background: crate::DEFAULT_TERMINAL_BACKGROUND,
                    cursor_color: crate::DEFAULT_CURSOR_COLOR,
                    selection_background: crate::DEFAULT_SELECTION_BACKGROUND,
                    selection_foreground: crate::DEFAULT_SELECTION_FOREGROUND,
                    cursor_text_color: crate::DEFAULT_CURSOR_TEXT_COLOR,
                    url_color: crate::DEFAULT_URL_COLOR,
                    active_tab_foreground: crate::DEFAULT_ACTIVE_TAB_FOREGROUND,
                    active_tab_background: crate::DEFAULT_ACTIVE_TAB_BACKGROUND,
                    inactive_tab_foreground: crate::DEFAULT_INACTIVE_TAB_FOREGROUND,
                    inactive_tab_background: crate::DEFAULT_INACTIVE_TAB_BACKGROUND,
                    cursor_style: Some(0),
                    cursor_blink: false,
                    cursor_blink_interval_ns: 600_000_000,
                },
                tabs: vec![
                    UiTabSnapshot {
                        tab_id: 1,
                        index: 0,
                        active: false,
                        title: "shell".to_string(),
                        pane_count: 1,
                        focused_pane: Some(7),
                        pane_ids: vec![7],
                    },
                    UiTabSnapshot {
                        tab_id: 2,
                        index: 1,
                        active: true,
                        title: "logs".to_string(),
                        pane_count: 2,
                        focused_pane: Some(9),
                        pane_ids: vec![8, 9],
                    },
                ],
                visible_panes: vec![UiPaneSnapshot {
                    leaf_index: 0,
                    leaf_id: 0,
                    pane_id: 7,
                    focused: true,
                    frame: UiRectSnapshot {
                        x: 0.0,
                        y: 20.0,
                        width: 200.0,
                        height: 100.0,
                    },
                    split_direction: Some("horizontal".to_string()),
                    split_ratio: Some(0.5),
                }],
                pane_terminals: Vec::new(),
                copy_mode: UiCopyModeSnapshot {
                    active: true,
                    cursor_row: 12,
                    cursor_col: 4,
                    selection_mode: "character".to_string(),
                    has_selection_anchor: true,
                    anchor_row: Some(12),
                    anchor_col: Some(4),
                    selection_rects: vec![UiRectSnapshot {
                        x: 32.0,
                        y: 48.0,
                        width: 8.0,
                        height: 16.0,
                    }],
                    show_position: true,
                },
                mouse_selection: UiMouseSelectionSnapshot {
                    active: true,
                    pane_id: Some(7),
                    selection_rects: vec![UiRectSnapshot {
                        x: 40.0,
                        y: 64.0,
                        width: 24.0,
                        height: 16.0,
                    }],
                },
                search: UiSearchSnapshot {
                    active: true,
                    query: "panic".to_string(),
                    total: 2,
                    selected: 1,
                },
                command_prompt: UiCommandPromptSnapshot {
                    active: false,
                    input: String::new(),
                    selected_suggestion: 0,
                    suggestions: Vec::new(),
                },
                status_bar: crate::status_components::UiStatusBarSnapshot::default(),
                pwd: "/repo".to_string(),
                scrollbar: UiScrollbarSnapshot {
                    total: 100,
                    offset: 40,
                    len: 20,
                },
                terminal: None,
            },
        };

        let value = serde_json::to_value(response).unwrap();

        assert_eq!(value["snapshot"]["active_tab"], 1);
        assert_eq!(value["snapshot"]["focused_pane"], 7);
        assert_eq!(
            value["snapshot"]["appearance"]["font_families"][0],
            "Fira Code"
        );
        assert!(
            value["snapshot"]["appearance"]["background_opacity"]
                .as_f64()
                .is_some_and(|opacity| (opacity - 0.7).abs() < 0.001)
        );
        assert_eq!(
            value["snapshot"]["copy_mode"]["selection_mode"],
            "character"
        );
        assert_eq!(value["snapshot"]["copy_mode"]["anchor_row"], 12);
        assert_eq!(
            value["snapshot"]["copy_mode"]["selection_rects"][0]["width"],
            8.0
        );
        assert_eq!(value["snapshot"]["mouse_selection"]["pane_id"], 7);
        assert_eq!(
            value["snapshot"]["visible_panes"][0]["frame"]["width"],
            200.0
        );
        assert_eq!(value["snapshot"]["search"]["query"], "panic");
    }
}
