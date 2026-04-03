#![cfg(target_os = "linux")]
#![allow(dead_code)]

use crate::unix_pty::{PtyProcess, PtySize};
use crate::vt;
use std::ffi::{c_void, CStr};
use std::io;
use std::path::Path;

#[derive(Debug, Clone, Default)]
pub struct CellSnapshot {
    pub text: String,
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

pub struct LinuxVtPane {
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
}

struct PtyWriteProxy {
    fd: i32,
}

impl LinuxVtPane {
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
            PtySize::new(cols, rows, cols.saturating_mul(cell_width_px as u16), rows.saturating_mul(cell_height_px as u16)),
        )?;

        let mut terminal = vt::Terminal::new(cols, rows, 10_000)
            .map_err(vt_to_io)?;
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
        })
    }

    pub fn poll_pty(&mut self) -> io::Result<bool> {
        let mut changed = false;
        for chunk in self.pty.try_read() {
            self.terminal.write(&chunk);
            changed = true;
        }
        if changed {
            self.render_state.update(&self.terminal).map_err(vt_to_io)?;
            self.key_encoder.sync_from_terminal(&self.terminal);
            self.mouse_encoder.sync_from_terminal(&self.terminal);
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
        self.mouse_encoder.set_size(&vt::GhosttyMouseEncoderSize {
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
        self.render_state.update(&self.terminal).map_err(vt_to_io)?;
        Ok(())
    }

    pub fn write_input(&self, bytes: &[u8]) -> io::Result<()> {
        self.pty.write(bytes)
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
        self.render_state.update(&self.terminal).map_err(vt_to_io)?;
        Ok(())
    }

    pub fn scroll_viewport_top(&mut self) -> io::Result<()> {
        self.terminal.scroll_viewport_top();
        self.render_state.update(&self.terminal).map_err(vt_to_io)?;
        Ok(())
    }

    pub fn scroll_viewport_bottom(&mut self) -> io::Result<()> {
        self.terminal.scroll_viewport_bottom();
        self.render_state.update(&self.terminal).map_err(vt_to_io)?;
        Ok(())
    }

    pub fn snapshot(&mut self) -> io::Result<TerminalSnapshot> {
        self.render_state.update(&self.terminal).map_err(vt_to_io)?;

        let cols = self.render_state.get_u16(vt::GHOSTTY_RENDER_STATE_DATA_COLS).map_err(vt_to_io)?;
        let rows = self.render_state.get_u16(vt::GHOSTTY_RENDER_STATE_DATA_ROWS).map_err(vt_to_io)?;
        let colors = self.render_state.colors().map_err(vt_to_io)?;
        let title = self.terminal.title().map_err(vt_to_io)?;
        let pwd = self.terminal.pwd().map_err(vt_to_io)?;
        let scrollbar = self.terminal.scrollbar().map_err(vt_to_io)?;
        let cursor = CursorSnapshot {
            visible: self.render_state.get_bool(vt::GHOSTTY_RENDER_STATE_DATA_CURSOR_VISIBLE).map_err(vt_to_io)?,
            x: self.render_state.get_u16(vt::GHOSTTY_RENDER_STATE_DATA_CURSOR_VIEWPORT_X).unwrap_or(0),
            y: self.render_state.get_u16(vt::GHOSTTY_RENDER_STATE_DATA_CURSOR_VIEWPORT_Y).unwrap_or(0),
            style: self.render_state.get_i32(vt::GHOSTTY_RENDER_STATE_DATA_CURSOR_VISUAL_STYLE).unwrap_or(0),
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

    pub fn key_encoder(&mut self) -> &mut vt::KeyEncoder { &mut self.key_encoder }

    pub fn mouse_encoder(&mut self) -> &mut vt::MouseEncoder { &mut self.mouse_encoder }

    pub fn terminal(&self) -> &vt::Terminal { &self.terminal }
}

impl Drop for LinuxVtPane {
    fn drop(&mut self) {
        let _ = &self.write_proxy;
    }
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
