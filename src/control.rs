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
use std::sync::mpsc;

#[derive(Debug, Deserialize)]
#[serde(tag = "cmd", rename_all = "kebab-case")]
pub enum Request {
    ListSurfaces,
    ListTabs,
    NewSplit { direction: Option<String> },
    NewTab,
    GotoTab { index: usize },
    NextTab,
    PrevTab,
    FocusSurface { index: usize },
    SendKey { key: String },
    DumpKeys { enabled: bool },
    Quit,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum Response {
    Surfaces { surfaces: Vec<SurfaceInfo> },
    Tabs { tabs: Vec<crate::tabs::TabInfo> },
    Ok { ok: bool },
    Error { error: String },
}

#[derive(Debug, Serialize)]
pub struct SurfaceInfo {
    pub index: usize,
    pub focused: bool,
}

/// Commands sent from the control thread to the main iced update loop.
#[derive(Debug)]
pub enum ControlCmd {
    DumpKeysOn,
    DumpKeysOff,
    ListSurfaces { reply: mpsc::Sender<Response> },
    ListTabs { reply: mpsc::Sender<Response> },
    SendKey { keyspec: String },
    NewSplit { direction: String },
    NewTab,
    GotoTab { index: usize },
    NextTab,
    PrevTab,
    FocusSurface { index: usize },
    Quit,
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
                Err(_) => Response::Error { error: "timeout".into() },
            }
        }
        Request::ListTabs => {
            let (reply_tx, reply_rx) = mpsc::channel();
            let _ = tx.send(ControlCmd::ListTabs { reply: reply_tx });
            match reply_rx.recv_timeout(std::time::Duration::from_secs(2)) {
                Ok(resp) => resp,
                Err(_) => Response::Error { error: "timeout".into() },
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
                _ if line.starts_with("key ") => {
                    Some(ControlCmd::SendKey { keyspec: line[4..].to_owned() })
                }
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
