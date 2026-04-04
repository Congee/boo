#![cfg(target_os = "linux")]
#![allow(dead_code)]

pub use crate::vt_backend_core::{
    CellSnapshot,
    CursorSnapshot,
    PollPtyResult,
    RunningCommand,
    TerminalSnapshot,
    VtPane as LinuxVtPane,
};
