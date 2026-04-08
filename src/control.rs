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
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::os::unix::net::UnixStream;
use std::sync::mpsc;
use std::time::Duration;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "kebab-case")]
pub enum Request {
    ListSurfaces,
    ListTabs,
    GetClipboard,
    GetUiSnapshot,
    AppKeyEvent { event: crate::AppKeyEvent },
    AppMouseEvent { event: crate::AppMouseEvent },
    AppAction { action: crate::bindings::Action },
    FocusPane { pane_id: u64 },
    ExecuteCommand { input: String },
    SendText { text: String },
    SendVt { text: String },
    NewSplit { direction: Option<String> },
    NewTab,
    GotoTab { index: usize },
    NextTab,
    PrevTab,
    ResizeFocused { cols: u16, rows: u16 },
    FocusSurface { index: usize },
    SendKey { key: String },
    DumpKeys { enabled: bool },
    Quit,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Response {
    Surfaces { surfaces: Vec<SurfaceInfo> },
    Tabs { tabs: Vec<crate::tabs::TabInfo> },
    Clipboard { text: String },
    UiSnapshot { snapshot: UiSnapshot },
    Ok { ok: bool },
    Error { error: String },
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
    pub search: UiSearchSnapshot,
    pub command_prompt: UiCommandPromptSnapshot,
    pub pwd: String,
    pub scrollbar: UiScrollbarSnapshot,
    pub terminal: Option<UiTerminalSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UiAppearanceSnapshot {
    pub font_family: Option<String>,
    pub font_size: f32,
    pub background_opacity: f32,
    pub background_opacity_cells: bool,
    pub cursor_style: Option<i32>,
    pub cursor_blink: bool,
    pub cursor_blink_interval_ns: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UiTabSnapshot {
    pub index: usize,
    pub active: bool,
    pub title: String,
    pub pane_count: usize,
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
    pub bold: bool,
    pub italic: bool,
    pub underline: i32,
}

/// Commands sent from the control thread to the main iced update loop.
#[derive(Debug)]
pub enum ControlCmd {
    DumpKeysOn,
    DumpKeysOff,
    ListSurfaces { reply: mpsc::Sender<Response> },
    ListTabs { reply: mpsc::Sender<Response> },
    GetClipboard { reply: mpsc::Sender<Response> },
    GetUiSnapshot { reply: mpsc::Sender<Response> },
    AppKeyEvent { event: crate::AppKeyEvent },
    AppMouseEvent { event: crate::AppMouseEvent },
    AppAction { action: crate::bindings::Action },
    FocusPane { pane_id: u64 },
    ExecuteCommand { input: String },
    SendKey { keyspec: String },
    SendText { text: String },
    SendVt { text: String },
    NewSplit { direction: String },
    NewTab,
    GotoTab { index: usize },
    NextTab,
    PrevTab,
    ResizeFocused { cols: u16, rows: u16 },
    FocusSurface { index: usize },
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
        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
        let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
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
                        let resp = dispatch_request(req, &tx);
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
    match req {
        Request::Quit => {
            let _ = tx.send(ControlCmd::Quit);
            Response::Ok { ok: true }
        }
        Request::DumpKeys { enabled } => {
            let _ = tx.send(if enabled {
                ControlCmd::DumpKeysOn
            } else {
                ControlCmd::DumpKeysOff
            });
            Response::Ok { ok: true }
        }
        Request::SendKey { key } => {
            let _ = tx.send(ControlCmd::SendKey { keyspec: key });
            Response::Ok { ok: true }
        }
        Request::ExecuteCommand { input } => {
            let _ = tx.send(ControlCmd::ExecuteCommand { input });
            Response::Ok { ok: true }
        }
        Request::AppKeyEvent { event } => {
            let _ = tx.send(ControlCmd::AppKeyEvent { event });
            Response::Ok { ok: true }
        }
        Request::AppMouseEvent { event } => {
            let _ = tx.send(ControlCmd::AppMouseEvent { event });
            Response::Ok { ok: true }
        }
        Request::AppAction { action } => {
            let _ = tx.send(ControlCmd::AppAction { action });
            Response::Ok { ok: true }
        }
        Request::FocusPane { pane_id } => {
            let _ = tx.send(ControlCmd::FocusPane { pane_id });
            Response::Ok { ok: true }
        }
        Request::SendText { text } => {
            let _ = tx.send(ControlCmd::SendText { text });
            Response::Ok { ok: true }
        }
        Request::SendVt { text } => {
            let _ = tx.send(ControlCmd::SendVt { text });
            Response::Ok { ok: true }
        }
        Request::GetClipboard => {
            let (reply_tx, reply_rx) = mpsc::channel();
            let _ = tx.send(ControlCmd::GetClipboard { reply: reply_tx });
            reply_rx
                .recv_timeout(std::time::Duration::from_millis(500))
                .unwrap_or(Response::Error {
                    error: "clipboard request timed out".to_string(),
                })
        }
        Request::NewSplit { direction } => {
            let _ = tx.send(ControlCmd::NewSplit {
                direction: direction.unwrap_or_else(|| "right".into()),
            });
            Response::Ok { ok: true }
        }
        Request::FocusSurface { index } => {
            let _ = tx.send(ControlCmd::FocusSurface { index });
            Response::Ok { ok: true }
        }
        Request::ListSurfaces => {
            let (reply_tx, reply_rx) = mpsc::channel();
            let _ = tx.send(ControlCmd::ListSurfaces { reply: reply_tx });
            match reply_rx.recv_timeout(std::time::Duration::from_secs(2)) {
                Ok(resp) => resp,
                Err(_) => Response::Error {
                    error: "timeout".into(),
                },
            }
        }
        Request::ListTabs => {
            let (reply_tx, reply_rx) = mpsc::channel();
            let _ = tx.send(ControlCmd::ListTabs { reply: reply_tx });
            match reply_rx.recv_timeout(std::time::Duration::from_secs(2)) {
                Ok(resp) => resp,
                Err(_) => Response::Error {
                    error: "timeout".into(),
                },
            }
        }
        Request::GetUiSnapshot => {
            let (reply_tx, reply_rx) = mpsc::channel();
            let _ = tx.send(ControlCmd::GetUiSnapshot { reply: reply_tx });
            match reply_rx.recv_timeout(std::time::Duration::from_secs(2)) {
                Ok(resp) => resp,
                Err(_) => Response::Error {
                    error: "timeout".into(),
                },
            }
        }
        Request::NewTab => {
            let _ = tx.send(ControlCmd::NewTab);
            Response::Ok { ok: true }
        }
        Request::GotoTab { index } => {
            let _ = tx.send(ControlCmd::GotoTab { index });
            Response::Ok { ok: true }
        }
        Request::NextTab => {
            let _ = tx.send(ControlCmd::NextTab);
            Response::Ok { ok: true }
        }
        Request::PrevTab => {
            let _ = tx.send(ControlCmd::PrevTab);
            Response::Ok { ok: true }
        }
        Request::ResizeFocused { cols, rows } => {
            let _ = tx.send(ControlCmd::ResizeFocused { cols, rows });
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
        UiCopyModeSnapshot, UiPaneSnapshot, UiRectSnapshot, UiScrollbarSnapshot, UiSearchSnapshot,
        UiSnapshot, UiTabSnapshot, dispatch_request,
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
                                font_family: Some("JetBrains Mono".to_string()),
                                font_size: 14.0,
                                background_opacity: 0.8,
                                background_opacity_cells: true,
                                cursor_style: Some(3),
                                cursor_blink: true,
                                cursor_blink_interval_ns: 600_000_000,
                            },
                            tabs: vec![UiTabSnapshot {
                                index: 0,
                                active: true,
                                title: "shell".to_string(),
                                pane_count: 1,
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
                    font_family: Some("Fira Code".to_string()),
                    font_size: 15.0,
                    background_opacity: 0.7,
                    background_opacity_cells: false,
                    cursor_style: Some(0),
                    cursor_blink: false,
                    cursor_blink_interval_ns: 600_000_000,
                },
                tabs: vec![
                    UiTabSnapshot {
                        index: 0,
                        active: false,
                        title: "shell".to_string(),
                        pane_count: 1,
                    },
                    UiTabSnapshot {
                        index: 1,
                        active: true,
                        title: "logs".to_string(),
                        pane_count: 2,
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
        assert_eq!(value["snapshot"]["appearance"]["font_family"], "Fira Code");
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
        assert_eq!(
            value["snapshot"]["visible_panes"][0]["frame"]["width"],
            200.0
        );
        assert_eq!(value["snapshot"]["search"]["query"], "panic");
    }
}
