//! Minimal Rust wrappers for `libghostty-vt`.
//!
//! This is the foundation for Boo's Linux migration away from full
//! `libghostty` surface embedding. The current wrapper intentionally
//! covers only the pieces we need first: terminal state, render-state
//! snapshots, and input encoding.

#![allow(dead_code, non_camel_case_types)]

use std::ffi::c_void;
use std::fmt;
use std::mem::MaybeUninit;
use std::os::raw::c_char;

pub const GHOSTTY_SUCCESS: i32 = 0;
pub const GHOSTTY_OUT_OF_SPACE: i32 = -3;
pub const GHOSTTY_NO_VALUE: i32 = -4;

pub type GhosttyTerminal = *mut c_void;
pub type GhosttyRenderState = *mut c_void;
pub type GhosttyRenderStateRowIterator = *mut c_void;
pub type GhosttyRenderStateRowCells = *mut c_void;
pub type GhosttyKeyEncoder = *mut c_void;
pub type GhosttyKeyEvent = *mut c_void;
pub type GhosttyMouseEncoder = *mut c_void;
pub type GhosttyMouseEvent = *mut c_void;

pub type GhosttyKey = i32;
pub type GhosttyMods = u16;
pub type GhosttyMouseAction = i32;
pub type GhosttyMouseButton = i32;
pub type GhosttyMouseTrackingMode = i32;
pub type GhosttyMouseFormat = i32;

pub const GHOSTTY_KEY_ACTION_RELEASE: i32 = 0;
pub const GHOSTTY_KEY_ACTION_PRESS: i32 = 1;
pub const GHOSTTY_KEY_ACTION_REPEAT: i32 = 2;

pub const GHOSTTY_MOUSE_ACTION_PRESS: i32 = 0;
pub const GHOSTTY_MOUSE_ACTION_RELEASE: i32 = 1;
pub const GHOSTTY_MOUSE_ACTION_MOTION: i32 = 2;

pub const GHOSTTY_MOUSE_BUTTON_UNKNOWN: i32 = 0;
pub const GHOSTTY_MOUSE_BUTTON_LEFT: i32 = 1;
pub const GHOSTTY_MOUSE_BUTTON_RIGHT: i32 = 2;
pub const GHOSTTY_MOUSE_BUTTON_MIDDLE: i32 = 3;

pub const GHOSTTY_RENDER_STATE_DATA_COLS: i32 = 1;
pub const GHOSTTY_RENDER_STATE_DATA_ROWS: i32 = 2;
pub const GHOSTTY_RENDER_STATE_DATA_DIRTY: i32 = 3;
pub const GHOSTTY_RENDER_STATE_DATA_ROW_ITERATOR: i32 = 4;
pub const GHOSTTY_RENDER_STATE_DATA_CURSOR_VISUAL_STYLE: i32 = 10;
pub const GHOSTTY_RENDER_STATE_DATA_CURSOR_VISIBLE: i32 = 11;
pub const GHOSTTY_RENDER_STATE_DATA_CURSOR_VIEWPORT_HAS_VALUE: i32 = 14;
pub const GHOSTTY_RENDER_STATE_DATA_CURSOR_VIEWPORT_X: i32 = 15;
pub const GHOSTTY_RENDER_STATE_DATA_CURSOR_VIEWPORT_Y: i32 = 16;

pub const GHOSTTY_RENDER_STATE_ROW_DATA_DIRTY: i32 = 1;
pub const GHOSTTY_RENDER_STATE_ROW_DATA_CELLS: i32 = 3;

pub const GHOSTTY_RENDER_STATE_ROW_OPTION_DIRTY: i32 = 0;
pub const GHOSTTY_RENDER_STATE_OPTION_DIRTY: i32 = 0;

pub const GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_STYLE: i32 = 2;
pub const GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_GRAPHEMES_LEN: i32 = 3;
pub const GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_GRAPHEMES_BUF: i32 = 4;
pub const GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_BG_COLOR: i32 = 5;
pub const GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_FG_COLOR: i32 = 6;
pub const GHOSTTY_TERMINAL_OPT_USERDATA: i32 = 0;
pub const GHOSTTY_TERMINAL_OPT_WRITE_PTY: i32 = 1;
pub const GHOSTTY_TERMINAL_DATA_SCROLLBAR: i32 = 9;
pub const GHOSTTY_TERMINAL_DATA_TITLE: i32 = 12;
pub const GHOSTTY_TERMINAL_DATA_PWD: i32 = 13;
pub const GHOSTTY_SCROLL_VIEWPORT_TOP: i32 = 0;
pub const GHOSTTY_SCROLL_VIEWPORT_BOTTOM: i32 = 1;
pub const GHOSTTY_SCROLL_VIEWPORT_DELTA: i32 = 2;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct GhosttyTerminalOptions {
    pub cols: u16,
    pub rows: u16,
    pub max_scrollback: usize,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct GhosttyColorRgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct GhosttyStyleColor {
    pub tag: i32,
    pub value: GhosttyStyleColorValue,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub union GhosttyStyleColorValue {
    pub palette: u8,
    pub rgb: GhosttyColorRgb,
    pub _padding: u64,
}

impl fmt::Debug for GhosttyStyleColorValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("GhosttyStyleColorValue(..)")
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct GhosttyStyle {
    pub size: usize,
    pub fg_color: GhosttyStyleColor,
    pub bg_color: GhosttyStyleColor,
    pub underline_color: GhosttyStyleColor,
    pub bold: bool,
    pub italic: bool,
    pub faint: bool,
    pub blink: bool,
    pub inverse: bool,
    pub invisible: bool,
    pub strikethrough: bool,
    pub overline: bool,
    pub underline: i32,
}

impl Default for GhosttyStyle {
    fn default() -> Self {
        Self {
            size: std::mem::size_of::<Self>(),
            fg_color: GhosttyStyleColor {
                tag: 0,
                value: GhosttyStyleColorValue { _padding: 0 },
            },
            bg_color: GhosttyStyleColor {
                tag: 0,
                value: GhosttyStyleColorValue { _padding: 0 },
            },
            underline_color: GhosttyStyleColor {
                tag: 0,
                value: GhosttyStyleColorValue { _padding: 0 },
            },
            bold: false,
            italic: false,
            faint: false,
            blink: false,
            inverse: false,
            invisible: false,
            strikethrough: false,
            overline: false,
            underline: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct GhosttyRenderStateColors {
    pub size: usize,
    pub background: GhosttyColorRgb,
    pub foreground: GhosttyColorRgb,
    pub cursor: GhosttyColorRgb,
    pub cursor_has_value: bool,
    pub palette: [GhosttyColorRgb; 256],
}

impl Default for GhosttyRenderStateColors {
    fn default() -> Self {
        Self {
            size: std::mem::size_of::<Self>(),
            background: GhosttyColorRgb::default(),
            foreground: GhosttyColorRgb::default(),
            cursor: GhosttyColorRgb::default(),
            cursor_has_value: false,
            palette: [GhosttyColorRgb::default(); 256],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct GhosttyMouseEncoderSize {
    pub size: usize,
    pub screen_width: u32,
    pub screen_height: u32,
    pub cell_width: u32,
    pub cell_height: u32,
    pub padding_top: u32,
    pub padding_bottom: u32,
    pub padding_right: u32,
    pub padding_left: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct GhosttyMousePosition {
    pub x: f32,
    pub y: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct GhosttyString {
    pub ptr: *const u8,
    pub len: usize,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct GhosttyTerminalScrollbar {
    pub total: u64,
    pub offset: u64,
    pub len: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub union GhosttyTerminalScrollViewportValue {
    pub delta: isize,
    pub _padding: [u64; 2],
}

impl Default for GhosttyTerminalScrollViewportValue {
    fn default() -> Self {
        Self { _padding: [0; 2] }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct GhosttyTerminalScrollViewport {
    pub tag: i32,
    pub value: GhosttyTerminalScrollViewportValue,
}

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
            result(ghostty_terminal_new(std::ptr::null(), raw.as_mut_ptr(), options))?;
            Ok(Self { raw: raw.assume_init() })
        }
    }

    pub fn raw(&self) -> GhosttyTerminal { self.raw }

    pub fn write(&mut self, bytes: &[u8]) {
        unsafe { ghostty_terminal_vt_write(self.raw, bytes.as_ptr(), bytes.len()) };
    }

    pub fn set_userdata(&mut self, userdata: *mut c_void) -> Result<(), Error> {
        unsafe { result(ghostty_terminal_set(self.raw, GHOSTTY_TERMINAL_OPT_USERDATA, userdata.cast_const())) }
    }

    pub fn set_write_pty(
        &mut self,
        callback: GhosttyTerminalWritePtyFn,
    ) -> Result<(), Error> {
        unsafe {
            result(ghostty_terminal_set(
                self.raw,
                GHOSTTY_TERMINAL_OPT_WRITE_PTY,
                callback.map(|cb| cb as *const c_void).unwrap_or(std::ptr::null()),
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
        unsafe { result(ghostty_terminal_resize(self.raw, cols, rows, cell_width_px, cell_height_px)) }
    }

    pub fn title(&self) -> Result<String, Error> {
        self.get_borrowed_string(GHOSTTY_TERMINAL_DATA_TITLE)
    }

    pub fn pwd(&self) -> Result<String, Error> {
        self.get_borrowed_string(GHOSTTY_TERMINAL_DATA_PWD)
    }

    pub fn scrollbar(&self) -> Result<GhosttyTerminalScrollbar, Error> {
        let mut out = GhosttyTerminalScrollbar::default();
        unsafe { result(ghostty_terminal_get(self.raw, GHOSTTY_TERMINAL_DATA_SCROLLBAR, &mut out as *mut _ as *mut c_void))? };
        Ok(out)
    }

    pub fn scroll_viewport_delta(&mut self, delta: isize) {
        unsafe {
            ghostty_terminal_scroll_viewport(
                self.raw,
                GhosttyTerminalScrollViewport {
                    tag: GHOSTTY_SCROLL_VIEWPORT_DELTA,
                    value: GhosttyTerminalScrollViewportValue { delta },
                },
            );
        }
    }

    pub fn scroll_viewport_top(&mut self) {
        unsafe {
            ghostty_terminal_scroll_viewport(
                self.raw,
                GhosttyTerminalScrollViewport {
                    tag: GHOSTTY_SCROLL_VIEWPORT_TOP,
                    value: GhosttyTerminalScrollViewportValue::default(),
                },
            );
        }
    }

    pub fn scroll_viewport_bottom(&mut self) {
        unsafe {
            ghostty_terminal_scroll_viewport(
                self.raw,
                GhosttyTerminalScrollViewport {
                    tag: GHOSTTY_SCROLL_VIEWPORT_BOTTOM,
                    value: GhosttyTerminalScrollViewportValue::default(),
                },
            );
        }
    }

    fn get_borrowed_string(&self, data: i32) -> Result<String, Error> {
        let mut value = GhosttyString::default();
        unsafe { result(ghostty_terminal_get(self.raw, data, &mut value as *mut _ as *mut c_void))? };
        if value.ptr.is_null() || value.len == 0 {
            return Ok(String::new());
        }
        let bytes = unsafe { std::slice::from_raw_parts(value.ptr, value.len) };
        Ok(String::from_utf8_lossy(bytes).into_owned())
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        unsafe { ghostty_terminal_free(self.raw) };
    }
}

pub struct RenderState {
    raw: GhosttyRenderState,
}

impl RenderState {
    pub fn new() -> Result<Self, Error> {
        let mut raw = MaybeUninit::uninit();
        unsafe {
            result(ghostty_render_state_new(std::ptr::null(), raw.as_mut_ptr()))?;
            Ok(Self { raw: raw.assume_init() })
        }
    }

    pub fn update(&mut self, terminal: &Terminal) -> Result<(), Error> {
        unsafe { result(ghostty_render_state_update(self.raw, terminal.raw())) }
    }

    pub fn colors(&self) -> Result<GhosttyRenderStateColors, Error> {
        let mut colors = GhosttyRenderStateColors::default();
        unsafe {
            result(ghostty_render_state_colors_get(self.raw, &mut colors))?;
        }
        Ok(colors)
    }

    pub fn get_u16(&self, data: i32) -> Result<u16, Error> {
        let mut out = 0u16;
        unsafe { result(ghostty_render_state_get(self.raw, data, &mut out as *mut _ as *mut c_void))? };
        Ok(out)
    }

    pub fn get_bool(&self, data: i32) -> Result<bool, Error> {
        let mut out = false;
        unsafe { result(ghostty_render_state_get(self.raw, data, &mut out as *mut _ as *mut c_void))? };
        Ok(out)
    }

    pub fn get_i32(&self, data: i32) -> Result<i32, Error> {
        let mut out = 0i32;
        unsafe { result(ghostty_render_state_get(self.raw, data, &mut out as *mut _ as *mut c_void))? };
        Ok(out)
    }

    pub fn row_iterator(&self) -> Result<RowIterator, Error> {
        let mut iter = MaybeUninit::uninit();
        unsafe {
            result(ghostty_render_state_row_iterator_new(std::ptr::null(), iter.as_mut_ptr()))?;
            let iter = iter.assume_init();
            result(ghostty_render_state_get(
                self.raw,
                GHOSTTY_RENDER_STATE_DATA_ROW_ITERATOR,
                &iter as *const _ as *mut c_void,
            ))?;
            Ok(RowIterator { raw: iter })
        }
    }
}

impl Drop for RenderState {
    fn drop(&mut self) {
        unsafe { ghostty_render_state_free(self.raw) };
    }
}

pub struct RowIterator {
    raw: GhosttyRenderStateRowIterator,
}

impl RowIterator {
    pub fn next(&mut self) -> bool {
        unsafe { ghostty_render_state_row_iterator_next(self.raw) }
    }

    pub fn dirty(&self) -> Result<bool, Error> {
        let mut out = false;
        unsafe { result(ghostty_render_state_row_get(self.raw, GHOSTTY_RENDER_STATE_ROW_DATA_DIRTY, &mut out as *mut _ as *mut c_void))? };
        Ok(out)
    }

    pub fn clear_dirty(&mut self) -> Result<(), Error> {
        let clean = false;
        unsafe { result(ghostty_render_state_row_set(self.raw, GHOSTTY_RENDER_STATE_ROW_OPTION_DIRTY, &clean as *const _ as *const c_void)) }
    }

    pub fn cells(&self) -> Result<RowCells, Error> {
        let mut cells = MaybeUninit::uninit();
        unsafe {
            result(ghostty_render_state_row_cells_new(std::ptr::null(), cells.as_mut_ptr()))?;
            let cells = cells.assume_init();
            result(ghostty_render_state_row_get(
                self.raw,
                GHOSTTY_RENDER_STATE_ROW_DATA_CELLS,
                &cells as *const _ as *mut c_void,
            ))?;
            Ok(RowCells { raw: cells })
        }
    }
}

impl Drop for RowIterator {
    fn drop(&mut self) {
        unsafe { ghostty_render_state_row_iterator_free(self.raw) };
    }
}

pub struct RowCells {
    raw: GhosttyRenderStateRowCells,
}

impl RowCells {
    pub fn next(&mut self) -> bool {
        unsafe { ghostty_render_state_row_cells_next(self.raw) }
    }

    pub fn style(&self) -> Result<GhosttyStyle, Error> {
        let mut style = GhosttyStyle::default();
        unsafe {
            result(ghostty_render_state_row_cells_get(
                self.raw,
                GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_STYLE,
                &mut style as *mut _ as *mut c_void,
            ))?;
        }
        Ok(style)
    }

    pub fn grapheme_len(&self) -> Result<u32, Error> {
        let mut out = 0u32;
        unsafe {
            result(ghostty_render_state_row_cells_get(
                self.raw,
                GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_GRAPHEMES_LEN,
                &mut out as *mut _ as *mut c_void,
            ))?;
        }
        Ok(out)
    }

    pub fn graphemes(&self, len: usize) -> Result<Vec<u32>, Error> {
        let mut out = vec![0u32; len];
        unsafe {
            result(ghostty_render_state_row_cells_get(
                self.raw,
                GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_GRAPHEMES_BUF,
                out.as_mut_ptr() as *mut c_void,
            ))?;
        }
        Ok(out)
    }

    pub fn fg_color(&self) -> Result<GhosttyColorRgb, Error> {
        let mut out = GhosttyColorRgb::default();
        unsafe {
            result(ghostty_render_state_row_cells_get(
                self.raw,
                GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_FG_COLOR,
                &mut out as *mut _ as *mut c_void,
            ))?;
        }
        Ok(out)
    }

    pub fn bg_color(&self) -> Result<GhosttyColorRgb, Error> {
        let mut out = GhosttyColorRgb::default();
        unsafe {
            result(ghostty_render_state_row_cells_get(
                self.raw,
                GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_BG_COLOR,
                &mut out as *mut _ as *mut c_void,
            ))?;
        }
        Ok(out)
    }
}

impl Drop for RowCells {
    fn drop(&mut self) {
        unsafe { ghostty_render_state_row_cells_free(self.raw) };
    }
}

pub struct KeyEncoder {
    raw: GhosttyKeyEncoder,
}

impl KeyEncoder {
    pub fn new() -> Result<Self, Error> {
        let mut raw = MaybeUninit::uninit();
        unsafe {
            result(ghostty_key_encoder_new(std::ptr::null(), raw.as_mut_ptr()))?;
            Ok(Self { raw: raw.assume_init() })
        }
    }

    pub fn sync_from_terminal(&mut self, terminal: &Terminal) {
        unsafe { ghostty_key_encoder_setopt_from_terminal(self.raw, terminal.raw()) };
    }

    pub fn encode(&mut self, event: &KeyEvent) -> Result<Vec<u8>, Error> {
        let mut required = 0usize;
        unsafe {
            let rc = ghostty_key_encoder_encode(self.raw, event.raw, std::ptr::null_mut(), 0, &mut required);
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
            result(ghostty_key_encoder_encode(
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
        unsafe { ghostty_key_encoder_free(self.raw) };
    }
}

pub struct KeyEvent {
    raw: GhosttyKeyEvent,
}

impl KeyEvent {
    pub fn new() -> Result<Self, Error> {
        let mut raw = MaybeUninit::uninit();
        unsafe {
            result(ghostty_key_event_new(std::ptr::null(), raw.as_mut_ptr()))?;
            Ok(Self { raw: raw.assume_init() })
        }
    }

    pub fn set_action(&mut self, action: i32) {
        unsafe { ghostty_key_event_set_action(self.raw, action) };
    }

    pub fn set_key(&mut self, key: GhosttyKey) {
        unsafe { ghostty_key_event_set_key(self.raw, key) };
    }

    pub fn set_mods(&mut self, mods: GhosttyMods) {
        unsafe { ghostty_key_event_set_mods(self.raw, mods) };
    }

    pub fn set_consumed_mods(&mut self, mods: GhosttyMods) {
        unsafe { ghostty_key_event_set_consumed_mods(self.raw, mods) };
    }

    pub fn set_composing(&mut self, composing: bool) {
        unsafe { ghostty_key_event_set_composing(self.raw, composing) };
    }

    pub fn set_utf8(&mut self, utf8: &str) {
        unsafe { ghostty_key_event_set_utf8(self.raw, utf8.as_ptr() as *const c_char, utf8.len()) };
    }

    pub fn set_unshifted_codepoint(&mut self, codepoint: u32) {
        unsafe { ghostty_key_event_set_unshifted_codepoint(self.raw, codepoint) };
    }
}

impl Drop for KeyEvent {
    fn drop(&mut self) {
        unsafe { ghostty_key_event_free(self.raw) };
    }
}

pub struct MouseEncoder {
    raw: GhosttyMouseEncoder,
}

impl MouseEncoder {
    pub fn new() -> Result<Self, Error> {
        let mut raw = MaybeUninit::uninit();
        unsafe {
            result(ghostty_mouse_encoder_new(std::ptr::null(), raw.as_mut_ptr()))?;
            Ok(Self { raw: raw.assume_init() })
        }
    }

    pub fn sync_from_terminal(&mut self, terminal: &Terminal) {
        unsafe { ghostty_mouse_encoder_setopt_from_terminal(self.raw, terminal.raw()) };
    }

    pub fn set_size(&mut self, size: &GhosttyMouseEncoderSize) {
        unsafe { ghostty_mouse_encoder_setopt(self.raw, 2, size as *const _ as *const c_void) };
    }

    pub fn encode(&mut self, event: &MouseEvent) -> Result<Vec<u8>, Error> {
        let mut required = 0usize;
        unsafe {
            let rc = ghostty_mouse_encoder_encode(self.raw, event.raw, std::ptr::null_mut(), 0, &mut required);
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
            result(ghostty_mouse_encoder_encode(
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
        unsafe { ghostty_mouse_encoder_free(self.raw) };
    }
}

pub struct MouseEvent {
    raw: GhosttyMouseEvent,
}

pub type GhosttyTerminalWritePtyFn =
    Option<unsafe extern "C" fn(GhosttyTerminal, *mut c_void, *const u8, usize)>;

impl MouseEvent {
    pub fn new() -> Result<Self, Error> {
        let mut raw = MaybeUninit::uninit();
        unsafe {
            result(ghostty_mouse_event_new(std::ptr::null(), raw.as_mut_ptr()))?;
            Ok(Self { raw: raw.assume_init() })
        }
    }

    pub fn set_action(&mut self, action: GhosttyMouseAction) {
        unsafe { ghostty_mouse_event_set_action(self.raw, action) };
    }

    pub fn set_button(&mut self, button: GhosttyMouseButton) {
        unsafe { ghostty_mouse_event_set_button(self.raw, button) };
    }

    pub fn clear_button(&mut self) {
        unsafe { ghostty_mouse_event_clear_button(self.raw) };
    }

    pub fn set_mods(&mut self, mods: GhosttyMods) {
        unsafe { ghostty_mouse_event_set_mods(self.raw, mods) };
    }

    pub fn set_position(&mut self, x: f32, y: f32) {
        let pos = GhosttyMousePosition { x, y };
        unsafe { ghostty_mouse_event_set_position(self.raw, pos) };
    }
}

impl Drop for MouseEvent {
    fn drop(&mut self) {
        unsafe { ghostty_mouse_event_free(self.raw) };
    }
}

unsafe extern "C" {
    fn ghostty_terminal_new(
        allocator: *const c_void,
        terminal: *mut GhosttyTerminal,
        options: GhosttyTerminalOptions,
    ) -> i32;
    fn ghostty_terminal_free(terminal: GhosttyTerminal);
    fn ghostty_terminal_resize(
        terminal: GhosttyTerminal,
        cols: u16,
        rows: u16,
        cell_width_px: u32,
        cell_height_px: u32,
    ) -> i32;
    fn ghostty_terminal_vt_write(
        terminal: GhosttyTerminal,
        data: *const u8,
        len: usize,
    );
    fn ghostty_terminal_set(
        terminal: GhosttyTerminal,
        option: i32,
        value: *const c_void,
    ) -> i32;
    fn ghostty_terminal_get(
        terminal: GhosttyTerminal,
        data: i32,
        out: *mut c_void,
    ) -> i32;
    fn ghostty_terminal_scroll_viewport(
        terminal: GhosttyTerminal,
        behavior: GhosttyTerminalScrollViewport,
    );

    fn ghostty_render_state_new(
        allocator: *const c_void,
        state: *mut GhosttyRenderState,
    ) -> i32;
    fn ghostty_render_state_free(state: GhosttyRenderState);
    fn ghostty_render_state_update(
        state: GhosttyRenderState,
        terminal: GhosttyTerminal,
    ) -> i32;
    fn ghostty_render_state_get(
        state: GhosttyRenderState,
        data: i32,
        out: *mut c_void,
    ) -> i32;
    fn ghostty_render_state_set(
        state: GhosttyRenderState,
        option: i32,
        value: *const c_void,
    ) -> i32;
    fn ghostty_render_state_colors_get(
        state: GhosttyRenderState,
        out: *mut GhosttyRenderStateColors,
    ) -> i32;
    fn ghostty_render_state_row_iterator_new(
        allocator: *const c_void,
        out: *mut GhosttyRenderStateRowIterator,
    ) -> i32;
    fn ghostty_render_state_row_iterator_free(iter: GhosttyRenderStateRowIterator);
    fn ghostty_render_state_row_iterator_next(iter: GhosttyRenderStateRowIterator) -> bool;
    fn ghostty_render_state_row_get(
        iter: GhosttyRenderStateRowIterator,
        data: i32,
        out: *mut c_void,
    ) -> i32;
    fn ghostty_render_state_row_set(
        iter: GhosttyRenderStateRowIterator,
        option: i32,
        value: *const c_void,
    ) -> i32;
    fn ghostty_render_state_row_cells_new(
        allocator: *const c_void,
        out: *mut GhosttyRenderStateRowCells,
    ) -> i32;
    fn ghostty_render_state_row_cells_free(cells: GhosttyRenderStateRowCells);
    fn ghostty_render_state_row_cells_next(cells: GhosttyRenderStateRowCells) -> bool;
    fn ghostty_render_state_row_cells_get(
        cells: GhosttyRenderStateRowCells,
        data: i32,
        out: *mut c_void,
    ) -> i32;

    fn ghostty_key_encoder_new(
        allocator: *const c_void,
        encoder: *mut GhosttyKeyEncoder,
    ) -> i32;
    fn ghostty_key_encoder_free(encoder: GhosttyKeyEncoder);
    fn ghostty_key_encoder_setopt_from_terminal(
        encoder: GhosttyKeyEncoder,
        terminal: GhosttyTerminal,
    );
    fn ghostty_key_encoder_encode(
        encoder: GhosttyKeyEncoder,
        event: GhosttyKeyEvent,
        out_buf: *mut c_char,
        out_buf_size: usize,
        out_len: *mut usize,
    ) -> i32;

    fn ghostty_key_event_new(
        allocator: *const c_void,
        event: *mut GhosttyKeyEvent,
    ) -> i32;
    fn ghostty_key_event_free(event: GhosttyKeyEvent);
    fn ghostty_key_event_set_action(event: GhosttyKeyEvent, action: i32);
    fn ghostty_key_event_set_key(event: GhosttyKeyEvent, key: GhosttyKey);
    fn ghostty_key_event_set_mods(event: GhosttyKeyEvent, mods: GhosttyMods);
    fn ghostty_key_event_set_consumed_mods(event: GhosttyKeyEvent, mods: GhosttyMods);
    fn ghostty_key_event_set_composing(event: GhosttyKeyEvent, composing: bool);
    fn ghostty_key_event_set_utf8(event: GhosttyKeyEvent, utf8: *const c_char, len: usize);
    fn ghostty_key_event_set_unshifted_codepoint(event: GhosttyKeyEvent, codepoint: u32);

    fn ghostty_mouse_encoder_new(
        allocator: *const c_void,
        encoder: *mut GhosttyMouseEncoder,
    ) -> i32;
    fn ghostty_mouse_encoder_free(encoder: GhosttyMouseEncoder);
    fn ghostty_mouse_encoder_setopt(
        encoder: GhosttyMouseEncoder,
        option: i32,
        value: *const c_void,
    );
    fn ghostty_mouse_encoder_setopt_from_terminal(
        encoder: GhosttyMouseEncoder,
        terminal: GhosttyTerminal,
    );
    fn ghostty_mouse_encoder_encode(
        encoder: GhosttyMouseEncoder,
        event: GhosttyMouseEvent,
        out_buf: *mut c_char,
        out_buf_size: usize,
        out_len: *mut usize,
    ) -> i32;

    fn ghostty_mouse_event_new(
        allocator: *const c_void,
        event: *mut GhosttyMouseEvent,
    ) -> i32;
    fn ghostty_mouse_event_free(event: GhosttyMouseEvent);
    fn ghostty_mouse_event_set_action(event: GhosttyMouseEvent, action: GhosttyMouseAction);
    fn ghostty_mouse_event_set_button(event: GhosttyMouseEvent, button: GhosttyMouseButton);
    fn ghostty_mouse_event_clear_button(event: GhosttyMouseEvent);
    fn ghostty_mouse_event_set_mods(event: GhosttyMouseEvent, mods: GhosttyMods);
    fn ghostty_mouse_event_set_position(event: GhosttyMouseEvent, position: GhosttyMousePosition);
}
