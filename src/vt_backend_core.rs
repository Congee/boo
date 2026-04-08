#![cfg(any(target_os = "linux", target_os = "macos"))]
#![allow(dead_code)]

use crate::unix_pty::{PtyProcess, PtySize};
use crate::vt;
use crossbeam_channel as channel;
use std::collections::{HashMap, VecDeque};
use std::ffi::{CStr, c_void};
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use unicode_width::UnicodeWidthStr;

#[derive(Debug, Clone, Default)]
pub struct CellSnapshot {
    pub text: String,
    pub display_width: u8,
    pub fg: vt::GhosttyColorRgb,
    pub bg: vt::GhosttyColorRgb,
    pub bold: bool,
    pub italic: bool,
    pub underline: i32,
}

#[derive(Debug, Clone, Default)]
pub struct CursorSnapshot {
    pub visible: bool,
    pub blinking: bool,
    pub x: u16,
    pub y: u16,
    pub style: i32,
}

#[derive(Debug, Clone, Default)]
pub struct TerminalSnapshot {
    pub cols: u16,
    pub rows: u16,
    pub title: String,
    pub pwd: String,
    pub cursor: CursorSnapshot,
    pub rows_data: Vec<Vec<CellSnapshot>>,
    pub row_revisions: Vec<u64>,
    pub scrollbar: vt::GhosttyTerminalScrollbar,
    pub colors: vt::GhosttyRenderStateColors,
}

pub struct VtPane {
    terminal: vt::Terminal,
    render_state: vt::RenderState,
    key_encoder: vt::KeyEncoder,
    mouse_encoder: vt::MouseEncoder,
    pty: PtyProcess,
    write_proxy: Box<PtyWriteProxy>,
    cols: u16,
    rows: u16,
    cell_width_px: u32,
    cell_height_px: u32,
    dirty: bool,
    osc_state: OscState,
    running_command: Option<RunningCommand>,
    finished_commands: Vec<CommandFinished>,
    pending_notifications: HashMap<String, PendingNotification>,
    completed_notifications: Vec<DesktopNotification>,
    pending_pty_chunks: VecDeque<PendingPtyChunk>,
    pending_pty_bytes: usize,
    pending_pty_profile: PendingPtyProfile,
    force_full_snapshot_refresh: bool,
    control_sequence_tail: Vec<u8>,
}

// SAFETY: VtPane and the wrapped libghostty-vt handles are never accessed
// concurrently. The worker thread becomes the sole owner after spawn, and all
// interaction happens by message passing.
unsafe impl Send for VtPane {}

pub struct VtPaneUpdate {
    pub snapshot: Arc<TerminalSnapshot>,
    pub version: u64,
    pub running_command: Option<Option<String>>,
    pub finished_commands: Vec<CommandFinished>,
    pub desktop_notifications: Vec<DesktopNotification>,
    pub exited: bool,
}

#[derive(Clone)]
pub struct VtForwardKey {
    pub action: i32,
    pub keycode: u32,
    pub mods: vt::GhosttyMods,
    pub consumed_mods: vt::GhosttyMods,
    pub key_char: Option<char>,
    pub text: String,
    pub composing: bool,
    pub unshifted_codepoint: u32,
}

pub struct VtPaneWorker {
    tx: channel::Sender<WorkerCommand>,
    state: Arc<Mutex<WorkerState>>,
    pending_work: Arc<AtomicBool>,
    join: Option<thread::JoinHandle<()>>,
}

struct WorkerState {
    snapshot: Arc<TerminalSnapshot>,
    version: u64,
    running_command: Option<Option<String>>,
    finished_commands: Vec<CommandFinished>,
    desktop_notifications: Vec<DesktopNotification>,
    exited: bool,
}

enum WorkerCommand {
    Resize {
        cols: u16,
        rows: u16,
        cell_width_px: u32,
        cell_height_px: u32,
    },
    WriteInput(Vec<u8>),
    WriteVtBytes(Vec<u8>),
    ForwardKey(VtForwardKey),
    MouseInput {
        action: vt::GhosttyMouseAction,
        button: Option<vt::GhosttyMouseButton>,
        x: f32,
        y: f32,
        mods: vt::GhosttyMods,
    },
    ScrollViewportDelta(isize),
    ScrollViewportTop,
    ScrollViewportBottom,
    Shutdown,
}

struct PtyWriteProxy {
    fd: i32,
}

#[derive(Default)]
struct OscState {
    mode: OscMode,
    payload: Vec<u8>,
}

#[derive(Default)]
enum OscMode {
    #[default]
    Ground,
    Escape,
    Osc,
    OscEscape,
}

#[derive(Clone)]
pub struct RunningCommand {
    pub command: Option<String>,
    started_at: Instant,
}

#[derive(Clone, Copy)]
pub struct CommandFinished {
    pub exit_code: Option<u8>,
    pub duration_ns: u64,
}

#[derive(Clone)]
pub struct DesktopNotification {
    pub title: String,
    pub body: String,
}

#[derive(Default)]
struct PendingNotification {
    title: String,
    body: String,
}

struct PendingPtyChunk {
    bytes: Vec<u8>,
    offset: usize,
    has_escape: bool,
}

#[derive(Default)]
struct PendingPtyProfile {
    polls: u8,
    read_chunks: u64,
    write_chunks: u64,
    backlog_chunks: u64,
    backlog_bytes: u64,
    write_chunk_size: u64,
}

impl VtPaneWorker {
    const SNAPSHOT_REFRESH_INTERVAL_UNDER_BACKLOG: Duration = Duration::from_millis(8);

    pub fn spawn(
        cols: u16,
        rows: u16,
        cell_width_px: u32,
        cell_height_px: u32,
        command: Option<&CStr>,
        working_directory: Option<&Path>,
    ) -> io::Result<Self> {
        let mut pane = VtPane::spawn(
            cols,
            rows,
            cell_width_px,
            cell_height_px,
            command,
            working_directory,
        )?;
        let snapshot = pane.snapshot()?;
        let state = Arc::new(Mutex::new(WorkerState {
            snapshot: Arc::new(snapshot.clone()),
            version: 1,
            running_command: pane
                .running_command()
                .map(|running| running.command.clone()),
            finished_commands: Vec::new(),
            desktop_notifications: Vec::new(),
            exited: false,
        }));
        let pending_work = Arc::new(AtomicBool::new(false));
        let (tx, rx) = channel::unbounded();
        let worker_state = Arc::clone(&state);
        let worker_pending = Arc::clone(&pending_work);
        let join = thread::Builder::new()
            .name("boo-vt-pane".into())
            .spawn(move || worker_loop(pane, snapshot, rx, worker_state, worker_pending))
            .map_err(io::Error::other)?;
        Ok(Self {
            tx,
            state,
            pending_work,
            join: Some(join),
        })
    }

    pub fn poll_update(&self) -> VtPaneUpdate {
        let mut state = self.state.lock().unwrap();
        VtPaneUpdate {
            snapshot: Arc::clone(&state.snapshot),
            version: state.version,
            running_command: state.running_command.clone(),
            finished_commands: std::mem::take(&mut state.finished_commands),
            desktop_notifications: std::mem::take(&mut state.desktop_notifications),
            exited: state.exited,
        }
    }

    pub fn has_pending_pty_work(&self) -> bool {
        self.pending_work.load(Ordering::Relaxed)
    }

    pub fn resize(&self, cols: u16, rows: u16, cell_width_px: u32, cell_height_px: u32) {
        let _ = self.tx.send(WorkerCommand::Resize {
            cols,
            rows,
            cell_width_px,
            cell_height_px,
        });
    }

    pub fn write_input(&self, bytes: &[u8]) -> io::Result<()> {
        self.tx
            .send(WorkerCommand::WriteInput(bytes.to_vec()))
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "vt pane worker closed"))
    }

    pub fn write_vt_bytes(&self, bytes: &[u8]) {
        let _ = self.tx.send(WorkerCommand::WriteVtBytes(bytes.to_vec()));
    }

    pub fn forward_key(&self, event: VtForwardKey) -> io::Result<()> {
        self.tx
            .send(WorkerCommand::ForwardKey(event))
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "vt pane worker closed"))
    }

    pub fn send_mouse_input(
        &self,
        action: vt::GhosttyMouseAction,
        button: Option<vt::GhosttyMouseButton>,
        x: f32,
        y: f32,
        mods: vt::GhosttyMods,
    ) -> io::Result<()> {
        self.tx
            .send(WorkerCommand::MouseInput {
                action,
                button,
                x,
                y,
                mods,
            })
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "vt pane worker closed"))
    }

    pub fn scroll_viewport_delta(&self, delta: isize) -> io::Result<()> {
        self.tx
            .send(WorkerCommand::ScrollViewportDelta(delta))
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "vt pane worker closed"))
    }

    pub fn scroll_viewport_top(&self) -> io::Result<()> {
        self.tx
            .send(WorkerCommand::ScrollViewportTop)
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "vt pane worker closed"))
    }

    pub fn scroll_viewport_bottom(&self) -> io::Result<()> {
        self.tx
            .send(WorkerCommand::ScrollViewportBottom)
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "vt pane worker closed"))
    }
}

impl Drop for VtPaneWorker {
    fn drop(&mut self) {
        let _ = self.tx.send(WorkerCommand::Shutdown);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

impl VtPane {
    const VT_WRITE_CHUNK: usize = 1024;
    const VT_WRITE_CHUNK_PLAIN: usize = 4096;
    const VT_WRITE_CHUNK_UNDER_BACKLOG: usize = 256;
    const VT_WRITE_CHUNK_PLAIN_UNDER_BACKLOG: usize = 2048;
    const PTY_POLL_MAX_DURATION: Duration = Duration::from_millis(2);
    const PTY_BACKLOG_SOFT_LIMIT_BYTES: usize = 16 * 1024;
    const PTY_BACKLOG_HARD_LIMIT_BYTES: usize = 64 * 1024;

    pub fn spawn(
        cols: u16,
        rows: u16,
        cell_width_px: u32,
        cell_height_px: u32,
        command: Option<&CStr>,
        working_directory: Option<&Path>,
    ) -> io::Result<Self> {
        let pty = PtyProcess::spawn(
            command,
            working_directory,
            PtySize::new(
                cols,
                rows,
                cols.saturating_mul(cell_width_px as u16),
                rows.saturating_mul(cell_height_px as u16),
            ),
        )?;

        let mut terminal = vt::Terminal::new(cols, rows, 10_000).map_err(vt_to_io)?;
        terminal
            .resize(cols, rows, cell_width_px, cell_height_px)
            .map_err(vt_to_io)?;

        let write_proxy = Box::new(PtyWriteProxy {
            fd: pty.master_fd(),
        });
        terminal
            .set_userdata((&*write_proxy as *const PtyWriteProxy).cast_mut().cast())
            .map_err(vt_to_io)?;
        terminal
            .set_write_pty(Some(write_pty_callback))
            .map_err(vt_to_io)?;

        let render_state = vt::RenderState::new().map_err(vt_to_io)?;
        let mut key_encoder = vt::KeyEncoder::new().map_err(vt_to_io)?;
        key_encoder.sync_from_terminal(&terminal);
        let mut mouse_encoder = vt::MouseEncoder::new().map_err(vt_to_io)?;
        mouse_encoder.sync_from_terminal(&terminal);
        sync_mouse_encoder_size(
            &mut mouse_encoder,
            cols,
            rows,
            cell_width_px,
            cell_height_px,
        );

        Ok(Self {
            terminal,
            render_state,
            key_encoder,
            mouse_encoder,
            pty,
            write_proxy,
            cols,
            rows,
            cell_width_px,
            cell_height_px,
            dirty: true,
            osc_state: OscState::default(),
            running_command: None,
            finished_commands: Vec::new(),
            pending_notifications: HashMap::new(),
            completed_notifications: Vec::new(),
            pending_pty_chunks: VecDeque::new(),
            pending_pty_bytes: 0,
            pending_pty_profile: PendingPtyProfile::default(),
            force_full_snapshot_refresh: false,
            control_sequence_tail: Vec::new(),
        })
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn running_command(&self) -> Option<&RunningCommand> {
        self.running_command.as_ref()
    }

    pub fn take_finished_commands(&mut self) -> Vec<CommandFinished> {
        std::mem::take(&mut self.finished_commands)
    }

    pub fn take_desktop_notifications(&mut self) -> Vec<DesktopNotification> {
        std::mem::take(&mut self.completed_notifications)
    }

    fn enqueue_pty_chunk(&mut self, chunk: Vec<u8>) {
        let chunk_len = chunk.len();
        let has_escape = chunk.contains(&0x1b);
        {
            let mut scope = crate::profiling::scope(
                "server.backend.pty_event.osc",
                crate::profiling::Kind::Cpu,
            );
            scope.add_bytes(chunk_len as u64);
            self.observe_control_sequences(&chunk, has_escape);
        }
        self.pending_pty_chunks.push_back(PendingPtyChunk {
            bytes: chunk,
            offset: 0,
            has_escape,
        });
        self.pending_pty_bytes = self.pending_pty_bytes.saturating_add(chunk_len);
    }

    pub fn process_pending_pty_work(&mut self) -> io::Result<bool> {
        let mut changed = false;
        let started_at = Instant::now();
        let mut written_chunks = 0u64;
        let mut write_chunk_units = 0u64;
        while started_at.elapsed() < Self::PTY_POLL_MAX_DURATION {
            let Some(has_escape) = self.pending_pty_chunks.front().map(|chunk| chunk.has_escape) else {
                break;
            };
            let write_chunk_size = self.write_chunk_size_for_chunk(has_escape);
            let Some(chunk) = self.pending_pty_chunks.front_mut() else {
                break;
            };
            let remaining = chunk.bytes.len().saturating_sub(chunk.offset);
            if remaining == 0 {
                self.pending_pty_chunks.pop_front();
                continue;
            }
            write_chunk_units = write_chunk_units.saturating_add(write_chunk_size as u64);
            let end = (chunk.offset + write_chunk_size).min(chunk.bytes.len());
            let wrote = end.saturating_sub(chunk.offset);
            {
                let mut scope = crate::profiling::scope(
                    "server.backend.poll_pty.write",
                    crate::profiling::Kind::Cpu,
                );
                scope.add_bytes(wrote as u64);
                self.terminal.write(&chunk.bytes[chunk.offset..end]);
            }
            written_chunks = written_chunks.saturating_add(1);
            chunk.offset = end;
            self.pending_pty_bytes = self.pending_pty_bytes.saturating_sub(wrote);
            changed = true;
            if chunk.offset >= chunk.bytes.len() {
                self.pending_pty_chunks.pop_front();
            }
        }
        if changed {
            {
                let _scope = crate::profiling::scope(
                    "server.backend.poll_pty.sync_encoders",
                    crate::profiling::Kind::Cpu,
                );
                self.key_encoder.sync_from_terminal(&self.terminal);
                self.mouse_encoder.sync_from_terminal(&self.terminal);
            }
            self.dirty = true;
        }
        crate::profiling::record_bytes(
            "server.backend.pty_event.total",
            crate::profiling::Kind::Cpu,
            started_at.elapsed(),
            0,
        );
        self.pending_pty_profile.polls = self.pending_pty_profile.polls.wrapping_add(1);
        self.pending_pty_profile.write_chunks =
            self.pending_pty_profile.write_chunks.saturating_add(written_chunks);
        self.pending_pty_profile.backlog_chunks = self
            .pending_pty_profile
            .backlog_chunks
            .saturating_add(self.pending_pty_chunks.len() as u64);
        self.pending_pty_profile.backlog_bytes = self
            .pending_pty_profile
            .backlog_bytes
            .saturating_add(self.pending_pty_bytes as u64);
        self.pending_pty_profile.write_chunk_size = self
            .pending_pty_profile
            .write_chunk_size
            .saturating_add(write_chunk_units);
        if self.pending_pty_profile.polls >= 8 {
            crate::profiling::record_batch(&[
                crate::profiling::Record {
                    name: "server.backend.pty_event.read_chunks",
                    kind: crate::profiling::Kind::Cpu,
                    elapsed: Duration::ZERO,
                    bytes: 0,
                    units: self.pending_pty_profile.read_chunks,
                },
                crate::profiling::Record {
                    name: "server.backend.pty_event.write_chunks",
                    kind: crate::profiling::Kind::Cpu,
                    elapsed: Duration::ZERO,
                    bytes: 0,
                    units: self.pending_pty_profile.write_chunks,
                },
                crate::profiling::Record {
                    name: "server.backend.pty_event.backlog_chunks",
                    kind: crate::profiling::Kind::Cpu,
                    elapsed: Duration::ZERO,
                    bytes: 0,
                    units: self.pending_pty_profile.backlog_chunks,
                },
                crate::profiling::Record {
                    name: "server.backend.pty_event.backlog_bytes",
                    kind: crate::profiling::Kind::Cpu,
                    elapsed: Duration::ZERO,
                    bytes: self.pending_pty_profile.backlog_bytes,
                    units: 0,
                },
                crate::profiling::Record {
                    name: "server.backend.pty_event.write_chunk_size",
                    kind: crate::profiling::Kind::Cpu,
                    elapsed: Duration::ZERO,
                    bytes: 0,
                    units: self.pending_pty_profile.write_chunk_size,
                },
            ]);
            self.pending_pty_profile = PendingPtyProfile::default();
        }
        Ok(changed)
    }

    pub fn resize(
        &mut self,
        cols: u16,
        rows: u16,
        cell_width_px: u32,
        cell_height_px: u32,
    ) -> io::Result<()> {
        self.cols = cols;
        self.rows = rows;
        self.cell_width_px = cell_width_px;
        self.cell_height_px = cell_height_px;
        self.pty.resize(PtySize::new(
            cols,
            rows,
            cols.saturating_mul(cell_width_px as u16),
            rows.saturating_mul(cell_height_px as u16),
        ))?;
        self.terminal
            .resize(cols, rows, cell_width_px, cell_height_px)
            .map_err(vt_to_io)?;
        sync_mouse_encoder_size(
            &mut self.mouse_encoder,
            cols,
            rows,
            cell_width_px,
            cell_height_px,
        );
        self.dirty = true;
        Ok(())
    }

    pub fn write_input(&self, bytes: &[u8]) -> io::Result<()> {
        self.pty.write(bytes)
    }

    pub fn write_vt_bytes(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        self.observe_control_sequences(bytes, bytes.contains(&0x1b));
        for slice in bytes.chunks(Self::VT_WRITE_CHUNK) {
            self.terminal.write(slice);
        }
        self.key_encoder.sync_from_terminal(&self.terminal);
        self.mouse_encoder.sync_from_terminal(&self.terminal);
        self.dirty = true;
    }

    pub fn send_mouse_input(
        &mut self,
        action: vt::GhosttyMouseAction,
        button: Option<vt::GhosttyMouseButton>,
        x: f32,
        y: f32,
        mods: vt::GhosttyMods,
    ) -> io::Result<()> {
        let mut event = vt::MouseEvent::new().map_err(vt_to_io)?;
        event.set_action(action);
        if let Some(button) = button {
            event.set_button(button);
        } else {
            event.clear_button();
        }
        event.set_mods(mods);
        event.set_position(x, y);
        let encoded = self.mouse_encoder.encode(&event).map_err(vt_to_io)?;
        if !encoded.is_empty() {
            self.pty.write(&encoded)?;
        }
        Ok(())
    }

    pub fn scroll_viewport_delta(&mut self, delta: isize) -> io::Result<()> {
        if delta == 0 {
            return Ok(());
        }
        self.terminal.scroll_viewport_delta(delta);
        self.dirty = true;
        Ok(())
    }

    pub fn scroll_viewport_top(&mut self) -> io::Result<()> {
        self.terminal.scroll_viewport_top();
        self.dirty = true;
        Ok(())
    }

    pub fn scroll_viewport_bottom(&mut self) -> io::Result<()> {
        self.terminal.scroll_viewport_bottom();
        self.dirty = true;
        Ok(())
    }

    pub fn snapshot(&mut self) -> io::Result<TerminalSnapshot> {
        self.render_state.update(&self.terminal).map_err(vt_to_io)?;

        let cols = self
            .render_state
            .get_u16(vt::GHOSTTY_RENDER_STATE_DATA_COLS)
            .map_err(vt_to_io)?;
        let rows = self
            .render_state
            .get_u16(vt::GHOSTTY_RENDER_STATE_DATA_ROWS)
            .map_err(vt_to_io)?;
        let colors = self.render_state.colors().map_err(vt_to_io)?;
        let title = self.terminal.title().map_err(vt_to_io)?;
        let pwd = self.terminal.pwd().map_err(vt_to_io)?;
        let scrollbar = self.terminal.scrollbar().map_err(vt_to_io)?;
        let cursor = CursorSnapshot {
            visible: self
                .render_state
                .get_bool(vt::GHOSTTY_RENDER_STATE_DATA_CURSOR_VISIBLE)
                .map_err(vt_to_io)?,
            blinking: self
                .render_state
                .get_bool(vt::GHOSTTY_RENDER_STATE_DATA_CURSOR_BLINKING)
                .unwrap_or(false),
            x: self
                .render_state
                .get_u16(vt::GHOSTTY_RENDER_STATE_DATA_CURSOR_VIEWPORT_X)
                .unwrap_or(0),
            y: self
                .render_state
                .get_u16(vt::GHOSTTY_RENDER_STATE_DATA_CURSOR_VIEWPORT_Y)
                .unwrap_or(0),
            style: self
                .render_state
                .get_i32(vt::GHOSTTY_RENDER_STATE_DATA_CURSOR_VISUAL_STYLE)
                .unwrap_or(0),
        };

        let mut row_iter = self.render_state.row_iterator().map_err(vt_to_io)?;
        let mut rows_data = Vec::with_capacity(rows as usize);
        while row_iter.next() {
            let row = snapshot_row(&row_iter, cols, colors)?;
            rows_data.push(row);
            let _ = row_iter.clear_dirty();
        }
        crate::profiling::record_units(
            "server.backend.snapshot.rows",
            crate::profiling::Kind::Cpu,
            rows as u64,
        );
        crate::profiling::record_units(
            "server.backend.snapshot.cells",
            crate::profiling::Kind::Cpu,
            rows as u64 * cols as u64,
        );

        self.dirty = false;

        Ok(TerminalSnapshot {
            cols,
            rows,
            title,
            pwd,
            cursor,
            rows_data,
            row_revisions: vec![1; rows as usize],
            scrollbar,
            colors,
        })
    }

    pub fn refresh_snapshot(&mut self, snapshot: &mut TerminalSnapshot) -> io::Result<()> {
        self.render_state.update(&self.terminal).map_err(vt_to_io)?;

        let cols = self
            .render_state
            .get_u16(vt::GHOSTTY_RENDER_STATE_DATA_COLS)
            .map_err(vt_to_io)?;
        let rows = self
            .render_state
            .get_u16(vt::GHOSTTY_RENDER_STATE_DATA_ROWS)
            .map_err(vt_to_io)?;
        let colors = self.render_state.colors().map_err(vt_to_io)?;
        let title = self.terminal.title().map_err(vt_to_io)?;
        let pwd = self.terminal.pwd().map_err(vt_to_io)?;
        let scrollbar = self.terminal.scrollbar().map_err(vt_to_io)?;
        let cursor = CursorSnapshot {
            visible: self
                .render_state
                .get_bool(vt::GHOSTTY_RENDER_STATE_DATA_CURSOR_VISIBLE)
                .map_err(vt_to_io)?,
            blinking: self
                .render_state
                .get_bool(vt::GHOSTTY_RENDER_STATE_DATA_CURSOR_BLINKING)
                .unwrap_or(false),
            x: self
                .render_state
                .get_u16(vt::GHOSTTY_RENDER_STATE_DATA_CURSOR_VIEWPORT_X)
                .unwrap_or(0),
            y: self
                .render_state
                .get_u16(vt::GHOSTTY_RENDER_STATE_DATA_CURSOR_VIEWPORT_Y)
                .unwrap_or(0),
            style: self
                .render_state
                .get_i32(vt::GHOSTTY_RENDER_STATE_DATA_CURSOR_VISUAL_STYLE)
                .unwrap_or(0),
        };

        let size_changed = snapshot.cols != cols || snapshot.rows != rows;
        let force_full_refresh = self.force_full_snapshot_refresh;
        if size_changed || snapshot.rows_data.len() != rows as usize {
            snapshot.rows_data.resize_with(rows as usize, Vec::new);
            snapshot.row_revisions.resize(rows as usize, 1);
        }

        let mut row_iter = self.render_state.row_iterator().map_err(vt_to_io)?;
        let mut row_index = 0usize;
        let mut rebuilt_rows = 0u64;
        let mut rebuilt_cells = 0u64;
        while row_iter.next() {
            let row_dirty =
                force_full_refresh || size_changed || row_iter.dirty().map_err(vt_to_io)?;
            if row_dirty {
                snapshot.rows_data[row_index] = snapshot_row(&row_iter, cols, colors)?;
                snapshot.row_revisions[row_index] =
                    snapshot.row_revisions[row_index].wrapping_add(1);
                let _ = row_iter.clear_dirty();
                rebuilt_rows = rebuilt_rows.saturating_add(1);
                rebuilt_cells = rebuilt_cells.saturating_add(cols as u64);
            }
            row_index += 1;
        }

        snapshot.cols = cols;
        snapshot.rows = rows;
        snapshot.title = title;
        snapshot.pwd = pwd;
        snapshot.cursor = cursor;
        snapshot.scrollbar = scrollbar;
        snapshot.colors = colors;
        self.dirty = false;
        self.force_full_snapshot_refresh = false;
        crate::profiling::record_units(
            "server.backend.snapshot_refresh.rows",
            crate::profiling::Kind::Cpu,
            rebuilt_rows,
        );
        crate::profiling::record_units(
            "server.backend.snapshot_refresh.cells",
            crate::profiling::Kind::Cpu,
            rebuilt_cells,
        );
        crate::profiling::record_units(
            "server.backend.snapshot_refresh.full",
            crate::profiling::Kind::Cpu,
            u64::from(force_full_refresh || size_changed),
        );
        Ok(())
    }

    pub fn key_encoder(&mut self) -> &mut vt::KeyEncoder {
        &mut self.key_encoder
    }

    pub fn mouse_encoder(&mut self) -> &mut vt::MouseEncoder {
        &mut self.mouse_encoder
    }

    pub fn cell_width_px(&self) -> u32 {
        self.cell_width_px
    }

    pub fn cell_height_px(&self) -> u32 {
        self.cell_height_px
    }

    pub fn terminal(&self) -> &vt::Terminal {
        &self.terminal
    }

    pub fn has_pending_pty_work(&self) -> bool {
        !self.pending_pty_chunks.is_empty()
    }

    fn observe_control_sequences(&mut self, bytes: &[u8], has_escape: bool) {
        if self.control_sequence_tail.is_empty()
            && matches!(self.osc_state.mode, OscMode::Ground)
            && !has_escape
        {
            return;
        }
        if should_force_full_snapshot_refresh(&self.control_sequence_tail, bytes) {
            self.force_full_snapshot_refresh = true;
            crate::profiling::record_units(
                "server.backend.snapshot_refresh.trigger_full",
                crate::profiling::Kind::Cpu,
                1,
            );
        }
        update_control_sequence_tail(&mut self.control_sequence_tail, bytes);
        if matches!(self.osc_state.mode, OscMode::Ground) && !has_escape {
            return;
        }
        for &byte in bytes {
            match self.osc_state.mode {
                OscMode::Ground => {
                    if byte == 0x1b {
                        self.osc_state.mode = OscMode::Escape;
                    }
                }
                OscMode::Escape => {
                    if byte == b']' {
                        self.osc_state.mode = OscMode::Osc;
                        self.osc_state.payload.clear();
                    } else if byte == 0x1b {
                        self.osc_state.mode = OscMode::Escape;
                    } else {
                        self.osc_state.mode = OscMode::Ground;
                    }
                }
                OscMode::Osc => match byte {
                    0x07 => self.finish_osc_payload(),
                    0x1b => self.osc_state.mode = OscMode::OscEscape,
                    _ => {
                        if self.osc_state.payload.len() < 4096 {
                            self.osc_state.payload.push(byte);
                        } else {
                            self.osc_state = OscState::default();
                        }
                    }
                },
                OscMode::OscEscape => {
                    if byte == b'\\' {
                        self.finish_osc_payload();
                    } else if self.osc_state.payload.len() + 2 <= 4096 {
                        self.osc_state.payload.push(0x1b);
                        self.osc_state.payload.push(byte);
                        self.osc_state.mode = OscMode::Osc;
                    } else {
                        self.osc_state = OscState::default();
                    }
                }
            }
        }
    }

    fn finish_osc_payload(&mut self) {
        let payload = std::mem::take(&mut self.osc_state.payload);
        self.osc_state = OscState::default();
        if let Ok(payload) = std::str::from_utf8(&payload) {
            self.handle_osc_payload(payload);
        }
    }

    fn handle_osc_payload(&mut self, payload: &str) {
        if let Some(rest) = payload.strip_prefix("133;") {
            self.handle_osc_133(rest);
            return;
        }

        if let Some(rest) = payload.strip_prefix("99;") {
            self.handle_osc_99(rest);
        }
    }

    fn handle_osc_133(&mut self, rest: &str) {
        if rest.starts_with('C') {
            self.running_command = Some(RunningCommand {
                command: osc_133_command(rest),
                started_at: Instant::now(),
            });
        } else if rest.starts_with('D') {
            let exit_code = osc_133_exit_code(rest);
            let duration_ns = self
                .running_command
                .as_ref()
                .map(|running| running.started_at.elapsed().as_nanos() as u64)
                .unwrap_or(0);
            self.finished_commands.push(CommandFinished {
                exit_code,
                duration_ns,
            });
            self.running_command = None;
        }
    }

    fn handle_osc_99(&mut self, rest: &str) {
        let Some((metadata, payload)) = rest.split_once(';') else {
            return;
        };

        let meta = parse_osc_99_metadata(metadata);
        if meta.base64
            || matches!(
                meta.payload_type.as_deref(),
                Some("close" | "?" | "alive" | "icon" | "buttons")
            )
        {
            return;
        }

        let id = meta.identifier.unwrap_or_else(|| "0".to_string());
        let mut notification = self.pending_notifications.remove(&id).unwrap_or_default();
        match meta.payload_type.as_deref().unwrap_or("title") {
            "body" => notification.body.push_str(payload),
            _ => notification.title.push_str(payload),
        }

        if meta.done {
            let title = if notification.title.is_empty() {
                notification.body.clone()
            } else {
                notification.title.clone()
            };
            if !title.is_empty() {
                self.completed_notifications.push(DesktopNotification {
                    title,
                    body: notification.body,
                });
            }
        } else {
            self.pending_notifications.insert(id, notification);
        }
    }

    fn pending_backlog_bytes(&self) -> usize {
        self.pending_pty_bytes
    }

    fn write_chunk_size_for_chunk(&self, has_escape: bool) -> usize {
        if self.pending_backlog_bytes() >= Self::PTY_BACKLOG_SOFT_LIMIT_BYTES {
            if has_escape {
                Self::VT_WRITE_CHUNK_UNDER_BACKLOG
            } else {
                Self::VT_WRITE_CHUNK_PLAIN_UNDER_BACKLOG
            }
        } else {
            if has_escape {
                Self::VT_WRITE_CHUNK
            } else {
                Self::VT_WRITE_CHUNK_PLAIN
            }
        }
    }

}

fn worker_loop(
    mut pane: VtPane,
    mut snapshot: TerminalSnapshot,
    rx: channel::Receiver<WorkerCommand>,
    state: Arc<Mutex<WorkerState>>,
    pending_work: Arc<AtomicBool>,
) {
    let mut disconnected = false;
    let mut exited = false;
    let mut last_snapshot_refresh = Instant::now();
    let pty_rx = pane.pty.event_rx();
    loop {
        if !pane.has_pending_pty_work() && !exited && !disconnected {
            channel::select! {
                recv(rx) -> message => match message {
                    Ok(command) => {
                        if !handle_worker_command(&mut pane, command) {
                            disconnected = true;
                        }
                    }
                    Err(_) => disconnected = true,
                },
                recv(pty_rx) -> event => match event {
                    Ok(crate::unix_pty::PtyReadEvent::Chunk(chunk)) => {
                        pane.enqueue_pty_chunk(chunk);
                        pane.pending_pty_profile.read_chunks =
                            pane.pending_pty_profile.read_chunks.saturating_add(1);
                    }
                    Ok(crate::unix_pty::PtyReadEvent::Exited) | Err(_) => exited = true,
                }
            }
        }

        while let Ok(command) = rx.try_recv() {
            if !handle_worker_command(&mut pane, command) {
                disconnected = true;
                break;
            }
        }

        while let Ok(event) = pty_rx.try_recv() {
            match event {
                crate::unix_pty::PtyReadEvent::Chunk(chunk) => {
                    pane.enqueue_pty_chunk(chunk);
                    pane.pending_pty_profile.read_chunks =
                        pane.pending_pty_profile.read_chunks.saturating_add(1);
                }
                crate::unix_pty::PtyReadEvent::Exited => {
                    exited = true;
                    break;
                }
            }
        }

        let _changed = match pane.process_pending_pty_work() {
            Ok(changed) => changed,
            Err(error) => {
                log::warn!("vt pane worker PTY processing failed: {error}");
                exited = true;
                false
            }
        };

        let should_refresh_snapshot = if pane.is_dirty() {
            if pane.has_pending_pty_work() {
                last_snapshot_refresh.elapsed()
                    >= VtPaneWorker::SNAPSHOT_REFRESH_INTERVAL_UNDER_BACKLOG
            } else {
                true
            }
        } else {
            false
        };

        let snapshot_changed = if should_refresh_snapshot {
            let _scope = crate::profiling::scope(
                "server.backend.snapshot_refresh",
                crate::profiling::Kind::Cpu,
            );
            match pane.refresh_snapshot(&mut snapshot) {
                Ok(()) => {
                    last_snapshot_refresh = Instant::now();
                    true
                }
                Err(error) => {
                    log::warn!("vt pane worker snapshot refresh failed: {error}");
                    false
                }
            }
        } else {
            if pane.is_dirty() && pane.has_pending_pty_work() {
                crate::profiling::record_units(
                    "server.backend.snapshot_refresh.deferred_for_backlog",
                    crate::profiling::Kind::Cpu,
                    1,
                );
            }
            false
        };

        pending_work.store(pane.has_pending_pty_work(), Ordering::Relaxed);
        {
            let mut shared = state.lock().unwrap();
            if snapshot_changed {
                shared.snapshot = Arc::new(snapshot.clone());
                shared.version = shared.version.wrapping_add(1);
            }
            shared.running_command = pane
                .running_command()
                .map(|running| running.command.clone());
            shared
                .finished_commands
                .extend(pane.take_finished_commands());
            shared
                .desktop_notifications
                .extend(pane.take_desktop_notifications());
            shared.exited = shared.exited || exited;
        }

        if exited || disconnected {
            break;
        }

        if pane.has_pending_pty_work() {
            continue;
        }

        if pane.is_dirty()
            && last_snapshot_refresh.elapsed() < VtPaneWorker::SNAPSHOT_REFRESH_INTERVAL_UNDER_BACKLOG
        {
            let wait = VtPaneWorker::SNAPSHOT_REFRESH_INTERVAL_UNDER_BACKLOG
                .saturating_sub(last_snapshot_refresh.elapsed());
            if !wait.is_zero() {
                channel::select! {
                    recv(rx) -> message => match message {
                        Ok(command) => {
                            if !handle_worker_command(&mut pane, command) {
                                disconnected = true;
                            }
                        }
                        Err(_) => disconnected = true,
                    },
                    recv(pty_rx) -> event => match event {
                        Ok(crate::unix_pty::PtyReadEvent::Chunk(chunk)) => {
                            pane.enqueue_pty_chunk(chunk);
                            pane.pending_pty_profile.read_chunks =
                                pane.pending_pty_profile.read_chunks.saturating_add(1);
                        }
                        Ok(crate::unix_pty::PtyReadEvent::Exited) | Err(_) => exited = true,
                    },
                    recv(channel::after(wait)) -> _ => {}
                }
            }
        }
    }

    pending_work.store(false, Ordering::Relaxed);
}

fn handle_worker_command(pane: &mut VtPane, command: WorkerCommand) -> bool {
    match command {
        WorkerCommand::Resize {
            cols,
            rows,
            cell_width_px,
            cell_height_px,
        } => {
            let _ = pane.resize(cols, rows, cell_width_px, cell_height_px);
        }
        WorkerCommand::WriteInput(bytes) => {
            let _ = pane.write_input(&bytes);
        }
        WorkerCommand::WriteVtBytes(bytes) => pane.write_vt_bytes(&bytes),
        WorkerCommand::ForwardKey(event) => {
            let _ = pane_forward_key(pane, &event);
        }
        WorkerCommand::MouseInput {
            action,
            button,
            x,
            y,
            mods,
        } => {
            let _ = pane.send_mouse_input(action, button, x, y, mods);
        }
        WorkerCommand::ScrollViewportDelta(delta) => {
            let _ = pane.scroll_viewport_delta(delta);
        }
        WorkerCommand::ScrollViewportTop => {
            let _ = pane.scroll_viewport_top();
        }
        WorkerCommand::ScrollViewportBottom => {
            let _ = pane.scroll_viewport_bottom();
        }
        WorkerCommand::Shutdown => return false,
    }
    true
}

fn pane_forward_key(pane: &mut VtPane, request: &VtForwardKey) -> io::Result<()> {
    let mut ev = vt::KeyEvent::new().map_err(vt_to_io)?;
    ev.set_action(request.action);
    ev.set_key(request.keycode as vt::GhosttyKey);
    ev.set_mods(request.mods);
    ev.set_consumed_mods(request.consumed_mods);
    ev.set_composing(request.composing);
    ev.set_unshifted_codepoint(request.unshifted_codepoint);

    let fallback_text = request
        .key_char
        .filter(|ch| !ch.is_control())
        .map(|ch| ch.to_string());
    let utf8 = if request
        .text
        .as_bytes()
        .first()
        .is_some_and(|&byte| byte >= 0x20)
    {
        Some(request.text.as_str())
    } else {
        fallback_text.as_deref()
    };
    if let Some(utf8) = utf8 {
        ev.set_utf8(utf8);
    }
    let bytes = pane.key_encoder().encode(&ev).map_err(vt_to_io)?;
    if !bytes.is_empty() {
        pane.write_input(&bytes)?;
    }
    Ok(())
}

const FULL_REFRESH_CONTROL_SEQUENCES: &[&[u8]] = &[
    b"\x1b[?1049h",
    b"\x1b[?1049l",
    b"\x1b[?1047h",
    b"\x1b[?1047l",
    b"\x1b[?47h",
    b"\x1b[?47l",
    b"\x1b[2J",
    b"\x1b[3J",
];

fn should_force_full_snapshot_refresh(tail: &[u8], bytes: &[u8]) -> bool {
    if bytes.is_empty() {
        return false;
    }
    let mut haystack = Vec::with_capacity(tail.len() + bytes.len());
    haystack.extend_from_slice(tail);
    haystack.extend_from_slice(bytes);
    FULL_REFRESH_CONTROL_SEQUENCES.iter().any(|pattern| {
        haystack
            .windows(pattern.len())
            .any(|window| window == *pattern)
    })
}

fn update_control_sequence_tail(tail: &mut Vec<u8>, bytes: &[u8]) {
    const MAX_TAIL: usize = 16;
    if bytes.is_empty() {
        return;
    }
    tail.extend_from_slice(bytes);
    if tail.len() > MAX_TAIL {
        let trim = tail.len() - MAX_TAIL;
        tail.drain(0..trim);
    }
}

struct Osc99Metadata {
    payload_type: Option<String>,
    identifier: Option<String>,
    done: bool,
    base64: bool,
}

fn parse_osc_99_metadata(metadata: &str) -> Osc99Metadata {
    let mut meta = Osc99Metadata {
        payload_type: None,
        identifier: None,
        done: true,
        base64: false,
    };
    for entry in metadata.split(':').filter(|entry| !entry.is_empty()) {
        let Some((key, value)) = entry.split_once('=') else {
            continue;
        };
        match key {
            "p" => meta.payload_type = Some(value.to_string()),
            "i" => meta.identifier = Some(value.to_string()),
            "d" => meta.done = value != "0",
            "e" => meta.base64 = value == "1",
            _ => {}
        }
    }
    meta
}

fn osc_133_exit_code(rest: &str) -> Option<u8> {
    let mut parts = rest.split(';');
    match parts.next() {
        Some("D") => {}
        _ => return None,
    }
    parts.next()?.parse().ok()
}

impl Drop for VtPane {
    fn drop(&mut self) {
        let _ = &self.write_proxy;
    }
}

fn sync_mouse_encoder_size(
    mouse_encoder: &mut vt::MouseEncoder,
    cols: u16,
    rows: u16,
    cell_width_px: u32,
    cell_height_px: u32,
) {
    mouse_encoder.set_size(&vt::GhosttyMouseEncoderSize {
        size: std::mem::size_of::<vt::GhosttyMouseEncoderSize>(),
        screen_width: cols as u32 * cell_width_px,
        screen_height: rows as u32 * cell_height_px,
        cell_width: cell_width_px,
        cell_height: cell_height_px,
        padding_top: 0,
        padding_bottom: 0,
        padding_right: 0,
        padding_left: 0,
    });
}

fn osc_133_command(payload: &str) -> Option<String> {
    for segment in payload.split(';').skip(1) {
        if let Some(value) = segment.strip_prefix("cmdline_url=") {
            return percent_decode(value);
        }
        if let Some(value) = segment.strip_prefix("cmdline=") {
            return Some(value.to_string());
        }
    }
    None
}

fn percent_decode(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                i += 3;
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

unsafe extern "C" fn write_pty_callback(
    _terminal: vt::GhosttyTerminal,
    userdata: *mut c_void,
    data: *const u8,
    len: usize,
) {
    if userdata.is_null() || data.is_null() || len == 0 {
        return;
    }
    let proxy = unsafe { &*(userdata as *const PtyWriteProxy) };
    let bytes = unsafe { std::slice::from_raw_parts(data, len) };
    let _ = write_all_fd(proxy.fd, bytes);
}

fn write_all_fd(fd: i32, mut bytes: &[u8]) -> io::Result<()> {
    while !bytes.is_empty() {
        let written = unsafe { libc::write(fd, bytes.as_ptr() as *const _, bytes.len()) };
        if written < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err);
        }
        bytes = &bytes[written as usize..];
    }
    Ok(())
}

fn vt_to_io(err: vt::Error) -> io::Error {
    io::Error::new(io::ErrorKind::Other, err)
}

fn text_width(text: &str) -> u8 {
    UnicodeWidthStr::width(text).max(1).min(u8::MAX as usize) as u8
}

fn snapshot_row(
    row_iter: &vt::RowIterator,
    cols: u16,
    colors: vt::GhosttyRenderStateColors,
) -> io::Result<Vec<CellSnapshot>> {
    let mut cells = row_iter.cells().map_err(vt_to_io)?;
    let mut row = Vec::with_capacity(cols as usize);
    while cells.next() {
        let len = cells.grapheme_len().map_err(vt_to_io)? as usize;
        let text = if len == 0 {
            String::new()
        } else {
            let graphemes = cells.graphemes(len).map_err(vt_to_io)?;
            graphemes
                .into_iter()
                .filter_map(char::from_u32)
                .collect::<String>()
        };
        let style = cells.style().map_err(vt_to_io)?;
        let fg = cells.fg_color().unwrap_or(colors.foreground);
        let bg = cells.bg_color().unwrap_or(colors.background);
        row.push(CellSnapshot {
            display_width: text_width(&text),
            text,
            fg,
            bg,
            bold: style.bold,
            italic: style.italic,
            underline: style.underline,
        });
    }
    Ok(row)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vt_snapshot_preserves_bold_italic_and_underline() {
        let mut terminal = vt::Terminal::new(16, 4, 0).expect("terminal");
        terminal.resize(16, 4, 8, 16).expect("resize");
        terminal.write(b"\x1b[1;3;4mSTYLE\x1b[0m");

        let mut render_state = vt::RenderState::new().expect("render state");
        render_state.update(&terminal).expect("update");

        let mut rows = render_state.row_iterator().expect("rows");
        assert!(rows.next(), "expected first row");
        let mut cells = rows.cells().expect("cells");

        let mut seen = String::new();
        let mut matched = Vec::new();
        while cells.next() {
            let len = cells.grapheme_len().expect("grapheme len") as usize;
            let text = if len == 0 {
                String::new()
            } else {
                cells
                    .graphemes(len)
                    .expect("graphemes")
                    .into_iter()
                    .filter_map(char::from_u32)
                    .collect::<String>()
            };
            if text.is_empty() {
                continue;
            }
            let style = cells.style().expect("style");
            seen.push_str(&text);
            matched.push((text, style.bold, style.italic, style.underline));
            if seen == "STYLE" {
                break;
            }
        }

        assert_eq!(seen, "STYLE");
        assert_eq!(matched.len(), 5);
        for (text, bold, italic, underline) in matched {
            assert!(!text.is_empty());
            assert!(bold, "cell {text:?} should be bold");
            assert!(italic, "cell {text:?} should be italic");
            assert_ne!(underline, 0, "cell {text:?} should be underlined");
        }
    }

    #[test]
    fn vt_snapshot_preserves_combining_and_wide_graphemes() {
        let mut terminal = vt::Terminal::new(16, 4, 0).expect("terminal");
        terminal.resize(16, 4, 8, 16).expect("resize");
        terminal.write("e\u{301}🙂".as_bytes());

        let mut render_state = vt::RenderState::new().expect("render state");
        render_state.update(&terminal).expect("update");

        let mut rows = render_state.row_iterator().expect("rows");
        assert!(rows.next(), "expected first row");
        let mut cells = rows.cells().expect("cells");

        let mut seen = Vec::new();
        while cells.next() {
            let len = cells.grapheme_len().expect("grapheme len") as usize;
            if len == 0 {
                continue;
            }
            let text = cells
                .graphemes(len)
                .expect("graphemes")
                .into_iter()
                .filter_map(char::from_u32)
                .collect::<String>();
            seen.push((text.clone(), text_width(&text)));
            if seen.len() == 2 {
                break;
            }
        }

        assert_eq!(seen[0], ("e\u{301}".to_string(), 1));
        assert_eq!(seen[1], ("🙂".to_string(), 2));
    }

    #[test]
    fn osc_133_cmdline_url_is_decoded() {
        let command = osc_133_command("C;cmdline_url=printf%20hello%0A");
        assert_eq!(command.as_deref(), Some("printf hello\n"));
    }

    #[test]
    fn osc_133_running_command_tracks_start_and_finish() {
        let mut pane = VtPane::spawn(2, 1, 8, 16, None, None).expect("pane");

        pane.observe_control_sequences(b"\x1b]133;C;cmdline=make test\x07", true);
        assert_eq!(
            pane.running_command()
                .and_then(|running| running.command.as_deref()),
            Some("make test")
        );

        pane.observe_control_sequences(b"\x1b]133;D;0\x07", true);
        assert!(pane.running_command().is_none());
        let finished = pane.take_finished_commands();
        assert_eq!(finished.len(), 1);
        assert_eq!(finished[0].exit_code, Some(0));
    }

    #[test]
    fn osc_99_emits_desktop_notification() {
        let mut pane = VtPane::spawn(2, 1, 8, 16, None, None).expect("pane");

        pane.observe_control_sequences(b"\x1b]99;i=test:p=title:d=0;Build done\x07", true);
        pane.observe_control_sequences(b"\x1b]99;i=test:p=body;All checks passed\x07", true);

        let notifications = pane.take_desktop_notifications();
        assert_eq!(notifications.len(), 1);
        assert_eq!(notifications[0].title, "Build done");
        assert_eq!(notifications[0].body, "All checks passed");
    }

    #[test]
    fn full_refresh_detection_catches_alternate_screen_sequences() {
        assert!(should_force_full_snapshot_refresh(&[], b"\x1b[?1049h"));
        assert!(should_force_full_snapshot_refresh(b"\x1b[?10", b"49l"));
        assert!(should_force_full_snapshot_refresh(&[], b"\x1b[2J"));
        assert!(!should_force_full_snapshot_refresh(&[], b"plain output"));
    }
}
