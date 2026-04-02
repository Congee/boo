//! Control pipe for testing and debugging boo.
//!
//! Creates a named pipe at /tmp/boo.ctl. Send commands:
//!   echo "key ctrl+c" > /tmp/boo.ctl
//!   echo "key ctrl+s" > /tmp/boo.ctl
//!   echo "split right" > /tmp/boo.ctl
//!   echo "focus next" > /tmp/boo.ctl
//!   echo "dump-keys on" > /tmp/boo.ctl
//!   echo "quit" > /tmp/boo.ctl

use std::io::{BufRead, BufReader};
use std::sync::mpsc;

const PIPE_PATH: &str = "/tmp/boo.ctl";

#[derive(Debug)]
pub enum ControlCmd {
    DumpKeysOn,
    DumpKeysOff,
    /// Inject a key event: (keycode, mods, text)
    Key { keycode: u32, mods: i32, text: Option<String> },
    Quit,
}

pub fn start() -> mpsc::Receiver<ControlCmd> {
    let (tx, rx) = mpsc::channel();

    // Remove stale pipe
    let _ = std::fs::remove_file(PIPE_PATH);

    // Create named pipe
    unsafe {
        let path = std::ffi::CString::new(PIPE_PATH).unwrap();
        libc::mkfifo(path.as_ptr(), 0o644);
    }

    std::thread::spawn(move || {
        loop {
            // Open blocks until a writer connects
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
                    _ if line.starts_with("key ") => parse_key_cmd(&line[4..]),
                    _ => {
                        log::info!("control: unknown command: {line}");
                        None
                    }
                };
                if let Some(cmd) = cmd {
                    log::info!("control: {cmd:?}");
                    if tx.send(cmd).is_err() {
                        return;
                    }
                }
            }
        }
    });

    log::info!("control pipe: {PIPE_PATH}");
    rx
}

/// Parse "key <spec>" where spec is like "a", "ctrl+c", "ctrl+s", "0x08" (keycode)
fn parse_key_cmd(spec: &str) -> Option<ControlCmd> {
    let mut mods: i32 = 0;
    let mut key_part = spec;

    // Parse modifier prefixes
    loop {
        if let Some(rest) = key_part.strip_prefix("ctrl+") {
            mods |= 1 << 1; // GHOSTTY_MODS_CTRL
            key_part = rest;
        } else if let Some(rest) = key_part.strip_prefix("shift+") {
            mods |= 1 << 0; // GHOSTTY_MODS_SHIFT
            key_part = rest;
        } else if let Some(rest) = key_part.strip_prefix("alt+") {
            mods |= 1 << 2; // GHOSTTY_MODS_ALT
            key_part = rest;
        } else if let Some(rest) = key_part.strip_prefix("super+") {
            mods |= 1 << 3; // GHOSTTY_MODS_SUPER
            key_part = rest;
        } else {
            break;
        }
    }

    // Map key name to macOS virtual keycode
    let keycode = match key_part {
        "a" => 0x00, "s" => 0x01, "d" => 0x02, "f" => 0x03,
        "h" => 0x04, "g" => 0x05, "z" => 0x06, "x" => 0x07,
        "c" => 0x08, "v" => 0x09, "b" => 0x0B, "q" => 0x0C,
        "w" => 0x0D, "e" => 0x0E, "r" => 0x0F, "y" => 0x10,
        "t" => 0x11, "u" => 0x20, "i" => 0x22, "o" => 0x1F,
        "p" => 0x23, "l" => 0x25, "j" => 0x26, "k" => 0x28,
        "n" => 0x2D, "m" => 0x2E,
        "enter" | "return" => 0x24,
        "tab" => 0x30,
        "space" => 0x31,
        "escape" | "esc" => 0x35,
        "backspace" => 0x33,
        _ if key_part.starts_with("0x") => {
            u32::from_str_radix(&key_part[2..], 16).ok()?
        }
        _ => {
            log::warn!("control: unknown key: {key_part}");
            return None;
        }
    };

    let text = None; // text computed by main.rs from shifted_codepoint

    Some(ControlCmd::Key { keycode, mods, text })
}

pub fn cleanup() {
    let _ = std::fs::remove_file(PIPE_PATH);
}
