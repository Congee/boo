//! Minimal Rust wrappers for `libghostty-vt`.
//!
//! Boo keeps its own small wrapper API so the rest of the Linux terminal
//! backend does not depend directly on the raw sys-crate layout. The
//! implementation is backed by `libghostty_vt_sys` and the native
//! `libghostty-vt` shared library.

#![cfg(any(target_os = "linux", target_os = "macos"))]
#![allow(dead_code, non_camel_case_types)]

use libghostty_vt_sys as ffi;
use std::ffi::c_void;
use std::fmt;
use std::mem::MaybeUninit;
use std::os::raw::c_char;

pub const GHOSTTY_SUCCESS: i32 = ffi::GhosttyResult_GHOSTTY_SUCCESS;
pub const GHOSTTY_OUT_OF_SPACE: i32 = ffi::GhosttyResult_GHOSTTY_OUT_OF_SPACE;
pub const GHOSTTY_NO_VALUE: i32 = ffi::GhosttyResult_GHOSTTY_INVALID_VALUE;

pub type GhosttyTerminal = ffi::GhosttyTerminal_ptr;
pub type GhosttyRenderState = ffi::GhosttyRenderState_ptr;
pub type GhosttyRenderStateRowIterator = ffi::GhosttyRenderStateRowIterator_ptr;
pub type GhosttyRenderStateRowCells = ffi::GhosttyRenderStateRowCells_ptr;
pub type GhosttyCell = ffi::GhosttyCell;
pub type GhosttyFormatter = ffi::GhosttyFormatter_ptr;
pub type GhosttyKeyEncoder = ffi::GhosttyKeyEncoder_ptr;
pub type GhosttyKeyEvent = ffi::GhosttyKeyEvent_ptr;
pub type GhosttyMouseEncoder = ffi::GhosttyMouseEncoder_ptr;
pub type GhosttyMouseEvent = ffi::GhosttyMouseEvent_ptr;

pub type GhosttyKey = ffi::GhosttyKey;
pub type GhosttyMods = ffi::GhosttyMods;
pub type GhosttyMouseAction = ffi::GhosttyMouseAction;
pub type GhosttyMouseButton = ffi::GhosttyMouseButton;
pub type GhosttyMouseTrackingMode = ffi::GhosttyMouseTrackingMode;
pub type GhosttyMouseFormat = ffi::GhosttyMouseFormat;

pub const GHOSTTY_KEY_ACTION_RELEASE: i32 = ffi::GhosttyKeyAction_GHOSTTY_KEY_ACTION_RELEASE as i32;
pub const GHOSTTY_KEY_ACTION_PRESS: i32 = ffi::GhosttyKeyAction_GHOSTTY_KEY_ACTION_PRESS as i32;
pub const GHOSTTY_KEY_ACTION_REPEAT: i32 = ffi::GhosttyKeyAction_GHOSTTY_KEY_ACTION_REPEAT as i32;

pub const GHOSTTY_MOUSE_ACTION_PRESS: GhosttyMouseAction =
    ffi::GhosttyMouseAction_GHOSTTY_MOUSE_ACTION_PRESS;
pub const GHOSTTY_MOUSE_ACTION_RELEASE: GhosttyMouseAction =
    ffi::GhosttyMouseAction_GHOSTTY_MOUSE_ACTION_RELEASE;
pub const GHOSTTY_MOUSE_ACTION_MOTION: GhosttyMouseAction =
    ffi::GhosttyMouseAction_GHOSTTY_MOUSE_ACTION_MOTION;

pub const GHOSTTY_MOUSE_BUTTON_UNKNOWN: GhosttyMouseButton =
    ffi::GhosttyMouseButton_GHOSTTY_MOUSE_BUTTON_UNKNOWN;
pub const GHOSTTY_MOUSE_BUTTON_LEFT: GhosttyMouseButton =
    ffi::GhosttyMouseButton_GHOSTTY_MOUSE_BUTTON_LEFT;
pub const GHOSTTY_MOUSE_BUTTON_RIGHT: GhosttyMouseButton =
    ffi::GhosttyMouseButton_GHOSTTY_MOUSE_BUTTON_RIGHT;
pub const GHOSTTY_MOUSE_BUTTON_MIDDLE: GhosttyMouseButton =
    ffi::GhosttyMouseButton_GHOSTTY_MOUSE_BUTTON_MIDDLE;

pub const GHOSTTY_RENDER_STATE_DATA_COLS: i32 =
    ffi::GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_COLS as i32;
pub const GHOSTTY_RENDER_STATE_DATA_ROWS: i32 =
    ffi::GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_ROWS as i32;
pub const GHOSTTY_RENDER_STATE_DATA_DIRTY: i32 =
    ffi::GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_DIRTY as i32;
pub const GHOSTTY_RENDER_STATE_DATA_ROW_ITERATOR: i32 =
    ffi::GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_ROW_ITERATOR as i32;
pub const GHOSTTY_RENDER_STATE_DATA_CURSOR_VISUAL_STYLE: i32 =
    ffi::GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_CURSOR_VISUAL_STYLE as i32;
pub const GHOSTTY_RENDER_STATE_DATA_CURSOR_VISIBLE: i32 =
    ffi::GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_CURSOR_VISIBLE as i32;
pub const GHOSTTY_RENDER_STATE_DATA_CURSOR_BLINKING: i32 =
    ffi::GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_CURSOR_BLINKING as i32;
pub const GHOSTTY_RENDER_STATE_DATA_CURSOR_VIEWPORT_HAS_VALUE: i32 =
    ffi::GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_CURSOR_VIEWPORT_HAS_VALUE as i32;
pub const GHOSTTY_RENDER_STATE_DATA_CURSOR_VIEWPORT_X: i32 =
    ffi::GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_CURSOR_VIEWPORT_X as i32;
pub const GHOSTTY_RENDER_STATE_DATA_CURSOR_VIEWPORT_Y: i32 =
    ffi::GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_CURSOR_VIEWPORT_Y as i32;

pub const GHOSTTY_RENDER_STATE_CURSOR_VISUAL_STYLE_BAR: i32 =
    ffi::GhosttyRenderStateCursorVisualStyle_GHOSTTY_RENDER_STATE_CURSOR_VISUAL_STYLE_BAR as i32;
pub const GHOSTTY_RENDER_STATE_CURSOR_VISUAL_STYLE_BLOCK: i32 =
    ffi::GhosttyRenderStateCursorVisualStyle_GHOSTTY_RENDER_STATE_CURSOR_VISUAL_STYLE_BLOCK as i32;
pub const GHOSTTY_RENDER_STATE_CURSOR_VISUAL_STYLE_UNDERLINE: i32 =
    ffi::GhosttyRenderStateCursorVisualStyle_GHOSTTY_RENDER_STATE_CURSOR_VISUAL_STYLE_UNDERLINE
        as i32;
pub const GHOSTTY_RENDER_STATE_CURSOR_VISUAL_STYLE_BLOCK_HOLLOW: i32 =
    ffi::GhosttyRenderStateCursorVisualStyle_GHOSTTY_RENDER_STATE_CURSOR_VISUAL_STYLE_BLOCK_HOLLOW
        as i32;

pub const GHOSTTY_RENDER_STATE_ROW_DATA_DIRTY: i32 =
    ffi::GhosttyRenderStateRowData_GHOSTTY_RENDER_STATE_ROW_DATA_DIRTY as i32;
pub const GHOSTTY_RENDER_STATE_ROW_DATA_CELLS: i32 =
    ffi::GhosttyRenderStateRowData_GHOSTTY_RENDER_STATE_ROW_DATA_CELLS as i32;

pub const GHOSTTY_RENDER_STATE_ROW_OPTION_DIRTY: i32 =
    ffi::GhosttyRenderStateRowOption_GHOSTTY_RENDER_STATE_ROW_OPTION_DIRTY as i32;

pub const GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_STYLE: i32 =
    ffi::GhosttyRenderStateRowCellsData_GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_STYLE as i32;
pub const GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_RAW: i32 =
    ffi::GhosttyRenderStateRowCellsData_GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_RAW as i32;
pub const GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_GRAPHEMES_LEN: i32 =
    ffi::GhosttyRenderStateRowCellsData_GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_GRAPHEMES_LEN as i32;
pub const GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_GRAPHEMES_BUF: i32 =
    ffi::GhosttyRenderStateRowCellsData_GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_GRAPHEMES_BUF as i32;
pub const GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_BG_COLOR: i32 =
    ffi::GhosttyRenderStateRowCellsData_GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_BG_COLOR as i32;
pub const GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_FG_COLOR: i32 =
    ffi::GhosttyRenderStateRowCellsData_GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_FG_COLOR as i32;
pub const GHOSTTY_CELL_DATA_HAS_HYPERLINK: i32 =
    ffi::GhosttyCellData_GHOSTTY_CELL_DATA_HAS_HYPERLINK as i32;
pub const GHOSTTY_STYLE_COLOR_NONE: ffi::GhosttyStyleColorTag =
    ffi::GhosttyStyleColorTag_GHOSTTY_STYLE_COLOR_NONE;

pub const GHOSTTY_TERMINAL_OPT_USERDATA: i32 =
    ffi::GhosttyTerminalOption_GHOSTTY_TERMINAL_OPT_USERDATA as i32;
pub const GHOSTTY_TERMINAL_OPT_WRITE_PTY: i32 =
    ffi::GhosttyTerminalOption_GHOSTTY_TERMINAL_OPT_WRITE_PTY as i32;
pub const GHOSTTY_TERMINAL_DATA_SCROLLBAR: i32 =
    ffi::GhosttyTerminalData_GHOSTTY_TERMINAL_DATA_SCROLLBAR as i32;
pub const GHOSTTY_TERMINAL_DATA_TITLE: i32 =
    ffi::GhosttyTerminalData_GHOSTTY_TERMINAL_DATA_TITLE as i32;
pub const GHOSTTY_TERMINAL_DATA_PWD: i32 =
    ffi::GhosttyTerminalData_GHOSTTY_TERMINAL_DATA_PWD as i32;
pub const GHOSTTY_SCROLL_VIEWPORT_TOP: i32 =
    ffi::GhosttyTerminalScrollViewportTag_GHOSTTY_SCROLL_VIEWPORT_TOP as i32;
pub const GHOSTTY_SCROLL_VIEWPORT_BOTTOM: i32 =
    ffi::GhosttyTerminalScrollViewportTag_GHOSTTY_SCROLL_VIEWPORT_BOTTOM as i32;
pub const GHOSTTY_SCROLL_VIEWPORT_DELTA: i32 =
    ffi::GhosttyTerminalScrollViewportTag_GHOSTTY_SCROLL_VIEWPORT_DELTA as i32;

pub use ffi::{
    GhosttyColorRgb, GhosttyMouseEncoderSize, GhosttyMousePosition, GhosttyRenderStateColors,
    GhosttyString, GhosttyStyle, GhosttyTerminalOptions, GhosttyTerminalScrollViewport,
    GhosttyTerminalScrollViewportValue, GhosttyTerminalScrollbar,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Error(pub i32);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "libghostty-vt error {}", self.0)
    }
}

impl std::error::Error for Error {}

fn result(code: i32) -> Result<(), Error> {
    if code == GHOSTTY_SUCCESS {
        Ok(())
    } else {
        Err(Error(code))
    }
}

pub struct Terminal {
    raw: GhosttyTerminal,
}

impl Terminal {
    pub fn new(cols: u16, rows: u16, max_scrollback: usize) -> Result<Self, Error> {
        let mut raw = MaybeUninit::uninit();
        let options = GhosttyTerminalOptions {
            cols,
            rows,
            max_scrollback,
        };
        unsafe {
            result(ffi::ghostty_terminal_new(
                std::ptr::null(),
                raw.as_mut_ptr(),
                options,
            ))?;
            Ok(Self {
                raw: raw.assume_init(),
            })
        }
    }

    pub fn raw(&self) -> GhosttyTerminal {
        self.raw
    }

    pub fn write(&mut self, bytes: &[u8]) {
        unsafe { ffi::ghostty_terminal_vt_write(self.raw, bytes.as_ptr(), bytes.len()) };
    }

    pub fn set_userdata(&mut self, userdata: *mut c_void) -> Result<(), Error> {
        unsafe {
            result(ffi::ghostty_terminal_set(
                self.raw,
                ffi::GhosttyTerminalOption_GHOSTTY_TERMINAL_OPT_USERDATA,
                userdata.cast_const(),
            ))
        }
    }

    pub fn set_write_pty(&mut self, callback: GhosttyTerminalWritePtyFn) -> Result<(), Error> {
        unsafe {
            result(ffi::ghostty_terminal_set(
                self.raw,
                ffi::GhosttyTerminalOption_GHOSTTY_TERMINAL_OPT_WRITE_PTY,
                callback
                    .map(|cb| cb as *const c_void)
                    .unwrap_or(std::ptr::null()),
            ))
        }
    }

    pub fn resize(
        &mut self,
        cols: u16,
        rows: u16,
        cell_width_px: u32,
        cell_height_px: u32,
    ) -> Result<(), Error> {
        unsafe {
            result(ffi::ghostty_terminal_resize(
                self.raw,
                cols,
                rows,
                cell_width_px,
                cell_height_px,
            ))
        }
    }

    pub fn title(&self) -> Result<String, Error> {
        self.get_borrowed_string(ffi::GhosttyTerminalData_GHOSTTY_TERMINAL_DATA_TITLE)
    }

    pub fn pwd(&self) -> Result<String, Error> {
        self.get_borrowed_string(ffi::GhosttyTerminalData_GHOSTTY_TERMINAL_DATA_PWD)
    }

    pub fn scrollbar(&self) -> Result<GhosttyTerminalScrollbar, Error> {
        let mut out = GhosttyTerminalScrollbar::default();
        unsafe {
            result(ffi::ghostty_terminal_get(
                self.raw,
                ffi::GhosttyTerminalData_GHOSTTY_TERMINAL_DATA_SCROLLBAR,
                &mut out as *mut _ as *mut c_void,
            ))?
        };
        Ok(out)
    }

    pub fn scroll_viewport_delta(&mut self, delta: isize) {
        unsafe {
            ffi::ghostty_terminal_scroll_viewport(
                self.raw,
                GhosttyTerminalScrollViewport {
                    tag: ffi::GhosttyTerminalScrollViewportTag_GHOSTTY_SCROLL_VIEWPORT_DELTA,
                    value: GhosttyTerminalScrollViewportValue { delta },
                },
            );
        }
    }

    pub fn scroll_viewport_top(&mut self) {
        unsafe {
            ffi::ghostty_terminal_scroll_viewport(
                self.raw,
                GhosttyTerminalScrollViewport {
                    tag: ffi::GhosttyTerminalScrollViewportTag_GHOSTTY_SCROLL_VIEWPORT_TOP,
                    value: GhosttyTerminalScrollViewportValue::default(),
                },
            );
        }
    }

    pub fn scroll_viewport_bottom(&mut self) {
        unsafe {
            ffi::ghostty_terminal_scroll_viewport(
                self.raw,
                GhosttyTerminalScrollViewport {
                    tag: ffi::GhosttyTerminalScrollViewportTag_GHOSTTY_SCROLL_VIEWPORT_BOTTOM,
                    value: GhosttyTerminalScrollViewportValue::default(),
                },
            );
        }
    }

    fn get_borrowed_string(&self, data: ffi::GhosttyTerminalData) -> Result<String, Error> {
        let mut value = GhosttyString::default();
        unsafe {
            result(ffi::ghostty_terminal_get(
                self.raw,
                data,
                &mut value as *mut _ as *mut c_void,
            ))?
        };
        if value.ptr.is_null() || value.len == 0 {
            return Ok(String::new());
        }
        let bytes = unsafe { std::slice::from_raw_parts(value.ptr, value.len) };
        Ok(String::from_utf8_lossy(bytes).into_owned())
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        unsafe { ffi::ghostty_terminal_free(self.raw) };
    }
}

pub struct Formatter {
    raw: GhosttyFormatter,
}

impl Formatter {
    pub fn for_terminal_hyperlinks(terminal: &Terminal) -> Result<Self, Error> {
        let mut raw = MaybeUninit::uninit();
        let options = ffi::GhosttyFormatterTerminalOptions {
            size: std::mem::size_of::<ffi::GhosttyFormatterTerminalOptions>(),
            emit: ffi::GhosttyFormatterFormat_GHOSTTY_FORMATTER_FORMAT_VT,
            unwrap: false,
            trim: false,
            extra: ffi::GhosttyFormatterTerminalExtra {
                size: std::mem::size_of::<ffi::GhosttyFormatterTerminalExtra>(),
                screen: ffi::GhosttyFormatterScreenExtra {
                    size: std::mem::size_of::<ffi::GhosttyFormatterScreenExtra>(),
                    hyperlink: true,
                    ..Default::default()
                },
                ..Default::default()
            },
        };
        unsafe {
            result(ffi::ghostty_formatter_terminal_new(
                std::ptr::null(),
                raw.as_mut_ptr(),
                terminal.raw(),
                options,
            ))?;
            Ok(Self {
                raw: raw.assume_init(),
            })
        }
    }

    pub fn format_alloc(&self) -> Result<Vec<u8>, Error> {
        let mut ptr = std::ptr::null_mut();
        let mut len = 0usize;
        unsafe {
            result(ffi::ghostty_formatter_format_alloc(
                self.raw,
                std::ptr::null(),
                &mut ptr,
                &mut len,
            ))?;
            if ptr.is_null() || len == 0 {
                return Ok(Vec::new());
            }
            let bytes = std::slice::from_raw_parts(ptr, len).to_vec();
            ffi::ghostty_free(std::ptr::null(), ptr, len);
            Ok(bytes)
        }
    }
}

impl Drop for Formatter {
    fn drop(&mut self) {
        unsafe { ffi::ghostty_formatter_free(self.raw) };
    }
}

pub struct RenderState {
    raw: GhosttyRenderState,
}

impl RenderState {
    pub fn new() -> Result<Self, Error> {
        let mut raw = MaybeUninit::uninit();
        unsafe {
            result(ffi::ghostty_render_state_new(
                std::ptr::null(),
                raw.as_mut_ptr(),
            ))?;
            Ok(Self {
                raw: raw.assume_init(),
            })
        }
    }

    pub fn update(&mut self, terminal: &Terminal) -> Result<(), Error> {
        unsafe { result(ffi::ghostty_render_state_update(self.raw, terminal.raw())) }
    }

    pub fn colors(&self) -> Result<GhosttyRenderStateColors, Error> {
        let mut colors = ffi::sized!(GhosttyRenderStateColors);
        unsafe {
            result(ffi::ghostty_render_state_colors_get(self.raw, &mut colors))?;
        }
        Ok(colors)
    }

    pub fn get_u16(&self, data: i32) -> Result<u16, Error> {
        let mut out = 0u16;
        unsafe {
            result(ffi::ghostty_render_state_get(
                self.raw,
                data as ffi::GhosttyRenderStateData,
                &mut out as *mut _ as *mut c_void,
            ))?
        };
        Ok(out)
    }

    pub fn get_bool(&self, data: i32) -> Result<bool, Error> {
        let mut out = false;
        unsafe {
            result(ffi::ghostty_render_state_get(
                self.raw,
                data as ffi::GhosttyRenderStateData,
                &mut out as *mut _ as *mut c_void,
            ))?
        };
        Ok(out)
    }

    pub fn get_i32(&self, data: i32) -> Result<i32, Error> {
        let mut out = 0i32;
        unsafe {
            result(ffi::ghostty_render_state_get(
                self.raw,
                data as ffi::GhosttyRenderStateData,
                &mut out as *mut _ as *mut c_void,
            ))?
        };
        Ok(out)
    }

    pub fn row_iterator(&self) -> Result<RowIterator, Error> {
        let mut iter = MaybeUninit::uninit();
        unsafe {
            result(ffi::ghostty_render_state_row_iterator_new(
                std::ptr::null(),
                iter.as_mut_ptr(),
            ))?;
            let iter = iter.assume_init();
            result(ffi::ghostty_render_state_get(
                self.raw,
                ffi::GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_ROW_ITERATOR,
                &iter as *const _ as *mut c_void,
            ))?;
            Ok(RowIterator { raw: iter })
        }
    }
}

impl Drop for RenderState {
    fn drop(&mut self) {
        unsafe { ffi::ghostty_render_state_free(self.raw) };
    }
}

pub struct RowIterator {
    raw: GhosttyRenderStateRowIterator,
}

impl RowIterator {
    pub fn next(&mut self) -> bool {
        unsafe { ffi::ghostty_render_state_row_iterator_next(self.raw) }
    }

    pub fn dirty(&self) -> Result<bool, Error> {
        let mut out = false;
        unsafe {
            result(ffi::ghostty_render_state_row_get(
                self.raw,
                ffi::GhosttyRenderStateRowData_GHOSTTY_RENDER_STATE_ROW_DATA_DIRTY,
                &mut out as *mut _ as *mut c_void,
            ))?
        };
        Ok(out)
    }

    pub fn clear_dirty(&mut self) -> Result<(), Error> {
        let clean = false;
        unsafe {
            result(ffi::ghostty_render_state_row_set(
                self.raw,
                ffi::GhosttyRenderStateRowOption_GHOSTTY_RENDER_STATE_ROW_OPTION_DIRTY,
                &clean as *const _ as *const c_void,
            ))
        }
    }

    pub fn cells(&self) -> Result<RowCells, Error> {
        let mut cells = MaybeUninit::uninit();
        unsafe {
            result(ffi::ghostty_render_state_row_cells_new(
                std::ptr::null(),
                cells.as_mut_ptr(),
            ))?;
            let cells = cells.assume_init();
            result(ffi::ghostty_render_state_row_get(
                self.raw,
                ffi::GhosttyRenderStateRowData_GHOSTTY_RENDER_STATE_ROW_DATA_CELLS,
                &cells as *const _ as *mut c_void,
            ))?;
            Ok(RowCells { raw: cells })
        }
    }
}

impl Drop for RowIterator {
    fn drop(&mut self) {
        unsafe { ffi::ghostty_render_state_row_iterator_free(self.raw) };
    }
}

pub struct RowCells {
    raw: GhosttyRenderStateRowCells,
}

impl RowCells {
    pub fn next(&mut self) -> bool {
        unsafe { ffi::ghostty_render_state_row_cells_next(self.raw) }
    }

    pub fn style(&self) -> Result<GhosttyStyle, Error> {
        let mut style = ffi::sized!(GhosttyStyle);
        unsafe {
            result(ffi::ghostty_render_state_row_cells_get(
                self.raw,
                ffi::GhosttyRenderStateRowCellsData_GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_STYLE,
                &mut style as *mut _ as *mut c_void,
            ))?;
        }
        Ok(style)
    }

    pub fn grapheme_len(&self) -> Result<u32, Error> {
        let mut out = 0u32;
        unsafe {
            result(ffi::ghostty_render_state_row_cells_get(
                self.raw,
                ffi::GhosttyRenderStateRowCellsData_GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_GRAPHEMES_LEN,
                &mut out as *mut _ as *mut c_void,
            ))?;
        }
        Ok(out)
    }

    pub fn graphemes(&self, len: usize) -> Result<Vec<u32>, Error> {
        let mut out = vec![0u32; len];
        unsafe {
            result(ffi::ghostty_render_state_row_cells_get(
                self.raw,
                ffi::GhosttyRenderStateRowCellsData_GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_GRAPHEMES_BUF,
                out.as_mut_ptr() as *mut c_void,
            ))?;
        }
        Ok(out)
    }

    pub fn fg_color(&self) -> Result<GhosttyColorRgb, Error> {
        let mut out = GhosttyColorRgb::default();
        unsafe {
            result(ffi::ghostty_render_state_row_cells_get(
                self.raw,
                ffi::GhosttyRenderStateRowCellsData_GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_FG_COLOR,
                &mut out as *mut _ as *mut c_void,
            ))?;
        }
        Ok(out)
    }

    pub fn bg_color(&self) -> Result<GhosttyColorRgb, Error> {
        let mut out = GhosttyColorRgb::default();
        unsafe {
            result(ffi::ghostty_render_state_row_cells_get(
                self.raw,
                ffi::GhosttyRenderStateRowCellsData_GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_BG_COLOR,
                &mut out as *mut _ as *mut c_void,
            ))?;
        }
        Ok(out)
    }

    pub fn has_hyperlink(&self) -> Result<bool, Error> {
        let mut cell = 0 as GhosttyCell;
        unsafe {
            result(ffi::ghostty_render_state_row_cells_get(
                self.raw,
                ffi::GhosttyRenderStateRowCellsData_GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_RAW,
                &mut cell as *mut _ as *mut c_void,
            ))?;
        }
        let mut out = false;
        unsafe {
            result(ffi::ghostty_cell_get(
                cell,
                ffi::GhosttyCellData_GHOSTTY_CELL_DATA_HAS_HYPERLINK,
                &mut out as *mut _ as *mut c_void,
            ))?;
        }
        Ok(out)
    }
}

impl Drop for RowCells {
    fn drop(&mut self) {
        unsafe { ffi::ghostty_render_state_row_cells_free(self.raw) };
    }
}

pub struct KeyEncoder {
    raw: GhosttyKeyEncoder,
}

impl KeyEncoder {
    pub fn new() -> Result<Self, Error> {
        let mut raw = MaybeUninit::uninit();
        unsafe {
            result(ffi::ghostty_key_encoder_new(
                std::ptr::null(),
                raw.as_mut_ptr(),
            ))?;
            Ok(Self {
                raw: raw.assume_init(),
            })
        }
    }

    pub fn sync_from_terminal(&mut self, terminal: &Terminal) {
        unsafe { ffi::ghostty_key_encoder_setopt_from_terminal(self.raw, terminal.raw()) };
    }

    pub fn encode(&mut self, event: &KeyEvent) -> Result<Vec<u8>, Error> {
        let mut required = 0usize;
        unsafe {
            let rc = ffi::ghostty_key_encoder_encode(
                self.raw,
                event.raw,
                std::ptr::null_mut(),
                0,
                &mut required,
            );
            if rc != GHOSTTY_OUT_OF_SPACE && rc != GHOSTTY_SUCCESS {
                return Err(Error(rc));
            }
        }

        if required == 0 {
            return Ok(Vec::new());
        }

        let mut out = vec![0u8; required];
        let mut written = 0usize;
        unsafe {
            result(ffi::ghostty_key_encoder_encode(
                self.raw,
                event.raw,
                out.as_mut_ptr() as *mut c_char,
                out.len(),
                &mut written,
            ))?;
        }
        out.truncate(written);
        Ok(out)
    }
}

impl Drop for KeyEncoder {
    fn drop(&mut self) {
        unsafe { ffi::ghostty_key_encoder_free(self.raw) };
    }
}

pub struct KeyEvent {
    raw: GhosttyKeyEvent,
}

impl KeyEvent {
    pub fn new() -> Result<Self, Error> {
        let mut raw = MaybeUninit::uninit();
        unsafe {
            result(ffi::ghostty_key_event_new(
                std::ptr::null(),
                raw.as_mut_ptr(),
            ))?;
            Ok(Self {
                raw: raw.assume_init(),
            })
        }
    }

    pub fn set_action(&mut self, action: i32) {
        unsafe {
            ffi::ghostty_key_event_set_action(self.raw, action as ffi::GhosttyKeyAction);
        };
    }

    pub fn set_key(&mut self, key: GhosttyKey) {
        unsafe { ffi::ghostty_key_event_set_key(self.raw, key) };
    }

    pub fn set_mods(&mut self, mods: GhosttyMods) {
        unsafe { ffi::ghostty_key_event_set_mods(self.raw, mods) };
    }

    pub fn set_consumed_mods(&mut self, mods: GhosttyMods) {
        unsafe { ffi::ghostty_key_event_set_consumed_mods(self.raw, mods) };
    }

    pub fn set_composing(&mut self, composing: bool) {
        unsafe { ffi::ghostty_key_event_set_composing(self.raw, composing) };
    }

    pub fn set_utf8(&mut self, utf8: &str) {
        unsafe {
            ffi::ghostty_key_event_set_utf8(self.raw, utf8.as_ptr() as *const c_char, utf8.len())
        };
    }

    pub fn set_unshifted_codepoint(&mut self, codepoint: u32) {
        unsafe { ffi::ghostty_key_event_set_unshifted_codepoint(self.raw, codepoint) };
    }
}

impl Drop for KeyEvent {
    fn drop(&mut self) {
        unsafe { ffi::ghostty_key_event_free(self.raw) };
    }
}

pub struct MouseEncoder {
    raw: GhosttyMouseEncoder,
}

impl MouseEncoder {
    pub fn new() -> Result<Self, Error> {
        let mut raw = MaybeUninit::uninit();
        unsafe {
            result(ffi::ghostty_mouse_encoder_new(
                std::ptr::null(),
                raw.as_mut_ptr(),
            ))?;
            Ok(Self {
                raw: raw.assume_init(),
            })
        }
    }

    pub fn sync_from_terminal(&mut self, terminal: &Terminal) {
        unsafe { ffi::ghostty_mouse_encoder_setopt_from_terminal(self.raw, terminal.raw()) };
    }

    pub fn set_size(&mut self, size: &GhosttyMouseEncoderSize) {
        unsafe {
            ffi::ghostty_mouse_encoder_setopt(
                self.raw,
                ffi::GhosttyMouseEncoderOption_GHOSTTY_MOUSE_ENCODER_OPT_SIZE,
                size as *const _ as *const c_void,
            )
        };
    }

    pub fn encode(&mut self, event: &MouseEvent) -> Result<Vec<u8>, Error> {
        let mut required = 0usize;
        unsafe {
            let rc = ffi::ghostty_mouse_encoder_encode(
                self.raw,
                event.raw,
                std::ptr::null_mut(),
                0,
                &mut required,
            );
            if rc != GHOSTTY_OUT_OF_SPACE && rc != GHOSTTY_SUCCESS {
                return Err(Error(rc));
            }
        }

        if required == 0 {
            return Ok(Vec::new());
        }

        let mut out = vec![0u8; required];
        let mut written = 0usize;
        unsafe {
            result(ffi::ghostty_mouse_encoder_encode(
                self.raw,
                event.raw,
                out.as_mut_ptr() as *mut c_char,
                out.len(),
                &mut written,
            ))?;
        }
        out.truncate(written);
        Ok(out)
    }
}

impl Drop for MouseEncoder {
    fn drop(&mut self) {
        unsafe { ffi::ghostty_mouse_encoder_free(self.raw) };
    }
}

pub struct MouseEvent {
    raw: GhosttyMouseEvent,
}

pub type GhosttyTerminalWritePtyFn = ffi::GhosttyTerminalWritePtyFn;

impl MouseEvent {
    pub fn new() -> Result<Self, Error> {
        let mut raw = MaybeUninit::uninit();
        unsafe {
            result(ffi::ghostty_mouse_event_new(
                std::ptr::null(),
                raw.as_mut_ptr(),
            ))?;
            Ok(Self {
                raw: raw.assume_init(),
            })
        }
    }

    pub fn set_action(&mut self, action: GhosttyMouseAction) {
        unsafe { ffi::ghostty_mouse_event_set_action(self.raw, action) };
    }

    pub fn set_button(&mut self, button: GhosttyMouseButton) {
        unsafe { ffi::ghostty_mouse_event_set_button(self.raw, button) };
    }

    pub fn clear_button(&mut self) {
        unsafe { ffi::ghostty_mouse_event_clear_button(self.raw) };
    }

    pub fn set_mods(&mut self, mods: GhosttyMods) {
        unsafe { ffi::ghostty_mouse_event_set_mods(self.raw, mods) };
    }

    pub fn set_position(&mut self, x: f32, y: f32) {
        let pos = GhosttyMousePosition { x, y };
        unsafe { ffi::ghostty_mouse_event_set_position(self.raw, pos) };
    }
}

impl Drop for MouseEvent {
    fn drop(&mut self) {
        unsafe { ffi::ghostty_mouse_event_free(self.raw) };
    }
}
