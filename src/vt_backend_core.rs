#![cfg(any(target_os = "linux", target_os = "macos"))]
#![allow(dead_code)]

use crate::unix_pty::{PtyProcess, PtySize};
use crate::vt;
use std::ffi::{c_void, CStr};
use std::io;
use std::path::Path;
use std::time::Instant;
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
}

pub struct PollPtyResult {
    pub changed: bool,
    pub exited: bool,
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

impl VtPane {
    const VT_WRITE_CHUNK: usize = 512;

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

        let write_proxy = Box::new(PtyWriteProxy { fd: pty.master_fd() });
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

    pub fn poll_pty(&mut self) -> io::Result<PollPtyResult> {
        let mut changed = false;
        for chunk in self.pty.try_read() {
            self.observe_control_sequences(&chunk);
            for slice in chunk.chunks(Self::VT_WRITE_CHUNK) {
                self.terminal.write(slice);
            }
            changed = true;
        }
        if changed {
            self.key_encoder.sync_from_terminal(&self.terminal);
            self.mouse_encoder.sync_from_terminal(&self.terminal);
            self.dirty = true;
        }
        Ok(PollPtyResult {
            changed,
            exited: self.pty.try_wait()?,
        })
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
        self.observe_control_sequences(bytes);
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
            rows_data.push(row);
            let _ = row_iter.clear_dirty();
        }

        self.dirty = false;

        Ok(TerminalSnapshot {
            cols,
            rows,
            title,
            pwd,
            cursor,
            rows_data,
            scrollbar,
            colors,
        })
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

    fn observe_control_sequences(&mut self, bytes: &[u8]) {
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
        let Some(rest) = payload.strip_prefix("133;") else {
            return;
        };

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

        pane.observe_control_sequences(b"\x1b]133;C;cmdline=make test\x07");
        assert_eq!(
            pane.running_command()
                .and_then(|running| running.command.as_deref()),
            Some("make test")
        );

        pane.observe_control_sequences(b"\x1b]133;D;0\x07");
        assert!(pane.running_command().is_none());
        let finished = pane.take_finished_commands();
        assert_eq!(finished.len(), 1);
        assert_eq!(finished[0].exit_code, Some(0));
    }
}
