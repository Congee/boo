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
use std::marker::PhantomData;
use std::mem::MaybeUninit;
use std::os::raw::c_char;
use std::ptr::NonNull;

pub const GHOSTTY_SUCCESS: i32 = ffi::GhosttyResult_GHOSTTY_SUCCESS;
pub const GHOSTTY_OUT_OF_MEMORY: i32 = ffi::GhosttyResult_GHOSTTY_OUT_OF_MEMORY;
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
pub const GHOSTTY_MOUSE_BUTTON_FOUR: GhosttyMouseButton =
    ffi::GhosttyMouseButton_GHOSTTY_MOUSE_BUTTON_FOUR;
pub const GHOSTTY_MOUSE_BUTTON_FIVE: GhosttyMouseButton =
    ffi::GhosttyMouseButton_GHOSTTY_MOUSE_BUTTON_FIVE;
pub const GHOSTTY_MOUSE_BUTTON_SIX: GhosttyMouseButton =
    ffi::GhosttyMouseButton_GHOSTTY_MOUSE_BUTTON_SIX;
pub const GHOSTTY_MOUSE_BUTTON_SEVEN: GhosttyMouseButton =
    ffi::GhosttyMouseButton_GHOSTTY_MOUSE_BUTTON_SEVEN;
pub const GHOSTTY_MOUSE_BUTTON_EIGHT: GhosttyMouseButton =
    ffi::GhosttyMouseButton_GHOSTTY_MOUSE_BUTTON_EIGHT;
pub const GHOSTTY_MOUSE_BUTTON_NINE: GhosttyMouseButton =
    ffi::GhosttyMouseButton_GHOSTTY_MOUSE_BUTTON_NINE;
pub const GHOSTTY_MOUSE_BUTTON_TEN: GhosttyMouseButton =
    ffi::GhosttyMouseButton_GHOSTTY_MOUSE_BUTTON_TEN;
pub const GHOSTTY_MOUSE_BUTTON_ELEVEN: GhosttyMouseButton =
    ffi::GhosttyMouseButton_GHOSTTY_MOUSE_BUTTON_ELEVEN;

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
pub enum Error {
    OutOfMemory,
    InvalidValue,
    OutOfSpace { required: usize },
    Unknown(i32),
}

impl Error {
    pub fn code(self) -> i32 {
        match self {
            Self::OutOfMemory => GHOSTTY_OUT_OF_MEMORY,
            Self::InvalidValue => GHOSTTY_NO_VALUE,
            Self::OutOfSpace { .. } => GHOSTTY_OUT_OF_SPACE,
            Self::Unknown(code) => code,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutOfMemory => write!(f, "libghostty-vt out of memory"),
            Self::InvalidValue => write!(f, "libghostty-vt invalid value"),
            Self::OutOfSpace { required } => {
                write!(f, "libghostty-vt out of space, {required} bytes required")
            }
            Self::Unknown(code) => write!(f, "libghostty-vt error {code}"),
        }
    }
}

impl std::error::Error for Error {}

fn result(code: i32) -> Result<(), Error> {
    match code {
        GHOSTTY_SUCCESS => Ok(()),
        GHOSTTY_OUT_OF_MEMORY => Err(Error::OutOfMemory),
        GHOSTTY_NO_VALUE => Err(Error::InvalidValue),
        GHOSTTY_OUT_OF_SPACE => Err(Error::OutOfSpace { required: 0 }),
        other => Err(Error::Unknown(other)),
    }
}

fn result_with_len(code: i32, len: usize) -> Result<usize, Error> {
    match code {
        GHOSTTY_SUCCESS => Ok(len),
        GHOSTTY_OUT_OF_MEMORY => Err(Error::OutOfMemory),
        GHOSTTY_NO_VALUE => Err(Error::InvalidValue),
        GHOSTTY_OUT_OF_SPACE => Err(Error::OutOfSpace { required: len }),
        other => Err(Error::Unknown(other)),
    }
}

#[derive(Debug)]
struct Handle<T> {
    raw: NonNull<T>,
}

impl<T> Handle<T> {
    fn new(raw: *mut T) -> Result<Self, Error> {
        let raw = NonNull::new(raw).ok_or(Error::OutOfMemory)?;
        Ok(Self { raw })
    }

    fn as_ptr(&self) -> *mut T {
        self.raw.as_ptr()
    }
}

fn new_handle<T>(init: impl FnOnce(*mut *mut T) -> i32) -> Result<Handle<T>, Error> {
    let mut raw = std::ptr::null_mut();
    result(init(&mut raw))?;
    Handle::new(raw)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorStyle {
    Bar,
    Block,
    Underline,
    BlockHollow,
    Unknown(i32),
}

impl CursorStyle {
    pub fn raw(self) -> i32 {
        match self {
            Self::Bar => GHOSTTY_RENDER_STATE_CURSOR_VISUAL_STYLE_BAR,
            Self::Block => GHOSTTY_RENDER_STATE_CURSOR_VISUAL_STYLE_BLOCK,
            Self::Underline => GHOSTTY_RENDER_STATE_CURSOR_VISUAL_STYLE_UNDERLINE,
            Self::BlockHollow => GHOSTTY_RENDER_STATE_CURSOR_VISUAL_STYLE_BLOCK_HOLLOW,
            Self::Unknown(style) => style,
        }
    }
}

impl From<i32> for CursorStyle {
    fn from(style: i32) -> Self {
        match style {
            GHOSTTY_RENDER_STATE_CURSOR_VISUAL_STYLE_BAR => Self::Bar,
            GHOSTTY_RENDER_STATE_CURSOR_VISUAL_STYLE_BLOCK => Self::Block,
            GHOSTTY_RENDER_STATE_CURSOR_VISUAL_STYLE_UNDERLINE => Self::Underline,
            GHOSTTY_RENDER_STATE_CURSOR_VISUAL_STYLE_BLOCK_HOLLOW => Self::BlockHollow,
            other => Self::Unknown(other),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderCursor {
    pub visible: bool,
    pub blinking: bool,
    pub x: u16,
    pub y: u16,
    pub style: CursorStyle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyAction {
    Release,
    Press,
    Repeat,
    Unknown(i32),
}

impl KeyAction {
    fn raw(self) -> ffi::GhosttyKeyAction {
        match self {
            Self::Release => ffi::GhosttyKeyAction_GHOSTTY_KEY_ACTION_RELEASE,
            Self::Press => ffi::GhosttyKeyAction_GHOSTTY_KEY_ACTION_PRESS,
            Self::Repeat => ffi::GhosttyKeyAction_GHOSTTY_KEY_ACTION_REPEAT,
            Self::Unknown(action) => action as ffi::GhosttyKeyAction,
        }
    }
}

impl From<i32> for KeyAction {
    fn from(action: i32) -> Self {
        match action {
            GHOSTTY_KEY_ACTION_RELEASE => Self::Release,
            GHOSTTY_KEY_ACTION_PRESS => Self::Press,
            GHOSTTY_KEY_ACTION_REPEAT => Self::Repeat,
            other => Self::Unknown(other),
        }
    }
}

impl From<KeyAction> for i32 {
    fn from(action: KeyAction) -> Self {
        action.raw() as i32
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct KeyMods(GhosttyMods);

impl KeyMods {
    pub const NONE: Self = Self(0);
    pub const SHIFT: Self = Self(ffi::GHOSTTY_MODS_SHIFT as GhosttyMods);
    pub const CTRL: Self = Self(ffi::GHOSTTY_MODS_CTRL as GhosttyMods);
    pub const ALT: Self = Self(ffi::GHOSTTY_MODS_ALT as GhosttyMods);
    pub const SUPER: Self = Self(ffi::GHOSTTY_MODS_SUPER as GhosttyMods);
    pub const CAPS_LOCK: Self = Self(ffi::GHOSTTY_MODS_CAPS_LOCK as GhosttyMods);
    pub const NUM_LOCK: Self = Self(ffi::GHOSTTY_MODS_NUM_LOCK as GhosttyMods);
    pub const SHIFT_SIDE: Self = Self(ffi::GHOSTTY_MODS_SHIFT_SIDE as GhosttyMods);
    pub const CTRL_SIDE: Self = Self(ffi::GHOSTTY_MODS_CTRL_SIDE as GhosttyMods);
    pub const ALT_SIDE: Self = Self(ffi::GHOSTTY_MODS_ALT_SIDE as GhosttyMods);
    pub const SUPER_SIDE: Self = Self(ffi::GHOSTTY_MODS_SUPER_SIDE as GhosttyMods);

    pub fn empty() -> Self {
        Self::NONE
    }

    pub fn from_bits_retain(bits: GhosttyMods) -> Self {
        Self(bits)
    }

    pub fn bits(self) -> GhosttyMods {
        self.0
    }

    pub fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }
}

impl From<GhosttyMods> for KeyMods {
    fn from(bits: GhosttyMods) -> Self {
        Self::from_bits_retain(bits)
    }
}

impl From<KeyMods> for GhosttyMods {
    fn from(mods: KeyMods) -> Self {
        mods.bits()
    }
}

impl std::ops::BitOr for KeyMods {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl std::ops::BitOrAssign for KeyMods {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct KittyKeyFlags(u8);

impl KittyKeyFlags {
    pub const DISABLED: Self = Self(ffi::GHOSTTY_KITTY_KEY_DISABLED as u8);
    pub const DISAMBIGUATE: Self = Self(ffi::GHOSTTY_KITTY_KEY_DISAMBIGUATE as u8);
    pub const REPORT_EVENTS: Self = Self(ffi::GHOSTTY_KITTY_KEY_REPORT_EVENTS as u8);
    pub const REPORT_ALTERNATES: Self = Self(ffi::GHOSTTY_KITTY_KEY_REPORT_ALTERNATES as u8);
    pub const REPORT_ALL: Self = Self(ffi::GHOSTTY_KITTY_KEY_REPORT_ALL as u8);
    pub const REPORT_ASSOCIATED: Self = Self(ffi::GHOSTTY_KITTY_KEY_REPORT_ASSOCIATED as u8);
    pub const ALL: Self = Self(ffi::GHOSTTY_KITTY_KEY_ALL as u8);

    pub fn bits(self) -> u8 {
        self.0
    }
}

impl From<u8> for KittyKeyFlags {
    fn from(bits: u8) -> Self {
        Self(bits)
    }
}

impl From<KittyKeyFlags> for u8 {
    fn from(flags: KittyKeyFlags) -> Self {
        flags.bits()
    }
}

impl std::ops::BitOr for KittyKeyFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OptionAsAlt {
    False,
    True,
    Left,
    Right,
    Unknown(ffi::GhosttyOptionAsAlt),
}

impl OptionAsAlt {
    fn raw(self) -> ffi::GhosttyOptionAsAlt {
        match self {
            Self::False => ffi::GhosttyOptionAsAlt_GHOSTTY_OPTION_AS_ALT_FALSE,
            Self::True => ffi::GhosttyOptionAsAlt_GHOSTTY_OPTION_AS_ALT_TRUE,
            Self::Left => ffi::GhosttyOptionAsAlt_GHOSTTY_OPTION_AS_ALT_LEFT,
            Self::Right => ffi::GhosttyOptionAsAlt_GHOSTTY_OPTION_AS_ALT_RIGHT,
            Self::Unknown(value) => value,
        }
    }
}

impl From<ffi::GhosttyOptionAsAlt> for OptionAsAlt {
    fn from(value: ffi::GhosttyOptionAsAlt) -> Self {
        match value {
            ffi::GhosttyOptionAsAlt_GHOSTTY_OPTION_AS_ALT_FALSE => Self::False,
            ffi::GhosttyOptionAsAlt_GHOSTTY_OPTION_AS_ALT_TRUE => Self::True,
            ffi::GhosttyOptionAsAlt_GHOSTTY_OPTION_AS_ALT_LEFT => Self::Left,
            ffi::GhosttyOptionAsAlt_GHOSTTY_OPTION_AS_ALT_RIGHT => Self::Right,
            other => Self::Unknown(other),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseAction {
    Press,
    Release,
    Motion,
    Unknown(GhosttyMouseAction),
}

impl MouseAction {
    fn raw(self) -> GhosttyMouseAction {
        match self {
            Self::Press => GHOSTTY_MOUSE_ACTION_PRESS,
            Self::Release => GHOSTTY_MOUSE_ACTION_RELEASE,
            Self::Motion => GHOSTTY_MOUSE_ACTION_MOTION,
            Self::Unknown(action) => action,
        }
    }
}

impl From<GhosttyMouseAction> for MouseAction {
    fn from(action: GhosttyMouseAction) -> Self {
        match action {
            GHOSTTY_MOUSE_ACTION_PRESS => Self::Press,
            GHOSTTY_MOUSE_ACTION_RELEASE => Self::Release,
            GHOSTTY_MOUSE_ACTION_MOTION => Self::Motion,
            other => Self::Unknown(other),
        }
    }
}

impl From<MouseAction> for GhosttyMouseAction {
    fn from(action: MouseAction) -> Self {
        action.raw()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Unknown,
    Left,
    Right,
    Middle,
    Four,
    Five,
    Six,
    Seven,
    Eight,
    Nine,
    Ten,
    Eleven,
    Other(GhosttyMouseButton),
}

impl MouseButton {
    fn raw(self) -> GhosttyMouseButton {
        match self {
            Self::Unknown => GHOSTTY_MOUSE_BUTTON_UNKNOWN,
            Self::Left => GHOSTTY_MOUSE_BUTTON_LEFT,
            Self::Right => GHOSTTY_MOUSE_BUTTON_RIGHT,
            Self::Middle => GHOSTTY_MOUSE_BUTTON_MIDDLE,
            Self::Four => GHOSTTY_MOUSE_BUTTON_FOUR,
            Self::Five => GHOSTTY_MOUSE_BUTTON_FIVE,
            Self::Six => GHOSTTY_MOUSE_BUTTON_SIX,
            Self::Seven => GHOSTTY_MOUSE_BUTTON_SEVEN,
            Self::Eight => GHOSTTY_MOUSE_BUTTON_EIGHT,
            Self::Nine => GHOSTTY_MOUSE_BUTTON_NINE,
            Self::Ten => GHOSTTY_MOUSE_BUTTON_TEN,
            Self::Eleven => GHOSTTY_MOUSE_BUTTON_ELEVEN,
            Self::Other(button) => button,
        }
    }
}

impl From<GhosttyMouseButton> for MouseButton {
    fn from(button: GhosttyMouseButton) -> Self {
        match button {
            GHOSTTY_MOUSE_BUTTON_UNKNOWN => Self::Unknown,
            GHOSTTY_MOUSE_BUTTON_LEFT => Self::Left,
            GHOSTTY_MOUSE_BUTTON_RIGHT => Self::Right,
            GHOSTTY_MOUSE_BUTTON_MIDDLE => Self::Middle,
            GHOSTTY_MOUSE_BUTTON_FOUR => Self::Four,
            GHOSTTY_MOUSE_BUTTON_FIVE => Self::Five,
            GHOSTTY_MOUSE_BUTTON_SIX => Self::Six,
            GHOSTTY_MOUSE_BUTTON_SEVEN => Self::Seven,
            GHOSTTY_MOUSE_BUTTON_EIGHT => Self::Eight,
            GHOSTTY_MOUSE_BUTTON_NINE => Self::Nine,
            GHOSTTY_MOUSE_BUTTON_TEN => Self::Ten,
            GHOSTTY_MOUSE_BUTTON_ELEVEN => Self::Eleven,
            other => Self::Other(other),
        }
    }
}

impl From<MouseButton> for GhosttyMouseButton {
    fn from(button: MouseButton) -> Self {
        button.raw()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseTrackingMode {
    None,
    X10,
    Normal,
    Button,
    Any,
    Unknown(GhosttyMouseTrackingMode),
}

impl MouseTrackingMode {
    fn raw(self) -> GhosttyMouseTrackingMode {
        match self {
            Self::None => ffi::GhosttyMouseTrackingMode_GHOSTTY_MOUSE_TRACKING_NONE,
            Self::X10 => ffi::GhosttyMouseTrackingMode_GHOSTTY_MOUSE_TRACKING_X10,
            Self::Normal => ffi::GhosttyMouseTrackingMode_GHOSTTY_MOUSE_TRACKING_NORMAL,
            Self::Button => ffi::GhosttyMouseTrackingMode_GHOSTTY_MOUSE_TRACKING_BUTTON,
            Self::Any => ffi::GhosttyMouseTrackingMode_GHOSTTY_MOUSE_TRACKING_ANY,
            Self::Unknown(mode) => mode,
        }
    }
}

impl From<GhosttyMouseTrackingMode> for MouseTrackingMode {
    fn from(mode: GhosttyMouseTrackingMode) -> Self {
        match mode {
            ffi::GhosttyMouseTrackingMode_GHOSTTY_MOUSE_TRACKING_NONE => Self::None,
            ffi::GhosttyMouseTrackingMode_GHOSTTY_MOUSE_TRACKING_X10 => Self::X10,
            ffi::GhosttyMouseTrackingMode_GHOSTTY_MOUSE_TRACKING_NORMAL => Self::Normal,
            ffi::GhosttyMouseTrackingMode_GHOSTTY_MOUSE_TRACKING_BUTTON => Self::Button,
            ffi::GhosttyMouseTrackingMode_GHOSTTY_MOUSE_TRACKING_ANY => Self::Any,
            other => Self::Unknown(other),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseFormat {
    X10,
    Utf8,
    Sgr,
    Urxvt,
    SgrPixels,
    Unknown(GhosttyMouseFormat),
}

impl MouseFormat {
    fn raw(self) -> GhosttyMouseFormat {
        match self {
            Self::X10 => ffi::GhosttyMouseFormat_GHOSTTY_MOUSE_FORMAT_X10,
            Self::Utf8 => ffi::GhosttyMouseFormat_GHOSTTY_MOUSE_FORMAT_UTF8,
            Self::Sgr => ffi::GhosttyMouseFormat_GHOSTTY_MOUSE_FORMAT_SGR,
            Self::Urxvt => ffi::GhosttyMouseFormat_GHOSTTY_MOUSE_FORMAT_URXVT,
            Self::SgrPixels => ffi::GhosttyMouseFormat_GHOSTTY_MOUSE_FORMAT_SGR_PIXELS,
            Self::Unknown(format) => format,
        }
    }
}

impl From<GhosttyMouseFormat> for MouseFormat {
    fn from(format: GhosttyMouseFormat) -> Self {
        match format {
            ffi::GhosttyMouseFormat_GHOSTTY_MOUSE_FORMAT_X10 => Self::X10,
            ffi::GhosttyMouseFormat_GHOSTTY_MOUSE_FORMAT_UTF8 => Self::Utf8,
            ffi::GhosttyMouseFormat_GHOSTTY_MOUSE_FORMAT_SGR => Self::Sgr,
            ffi::GhosttyMouseFormat_GHOSTTY_MOUSE_FORMAT_URXVT => Self::Urxvt,
            ffi::GhosttyMouseFormat_GHOSTTY_MOUSE_FORMAT_SGR_PIXELS => Self::SgrPixels,
            other => Self::Unknown(other),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MouseGeometry {
    pub screen_width: u32,
    pub screen_height: u32,
    pub cell_width: u32,
    pub cell_height: u32,
    pub padding_top: u32,
    pub padding_bottom: u32,
    pub padding_right: u32,
    pub padding_left: u32,
}

impl MouseGeometry {
    pub fn terminal_grid(cols: u16, rows: u16, cell_width: u32, cell_height: u32) -> Self {
        Self {
            screen_width: cols as u32 * cell_width,
            screen_height: rows as u32 * cell_height,
            cell_width,
            cell_height,
            padding_top: 0,
            padding_bottom: 0,
            padding_right: 0,
            padding_left: 0,
        }
    }
}

impl From<MouseGeometry> for GhosttyMouseEncoderSize {
    fn from(geometry: MouseGeometry) -> Self {
        Self {
            size: std::mem::size_of::<Self>(),
            screen_width: geometry.screen_width,
            screen_height: geometry.screen_height,
            cell_width: geometry.cell_width,
            cell_height: geometry.cell_height,
            padding_top: geometry.padding_top,
            padding_bottom: geometry.padding_bottom,
            padding_right: geometry.padding_right,
            padding_left: geometry.padding_left,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct RgbColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl From<GhosttyColorRgb> for RgbColor {
    fn from(color: GhosttyColorRgb) -> Self {
        Self {
            r: color.r,
            g: color.g,
            b: color.b,
        }
    }
}

impl From<RgbColor> for GhosttyColorRgb {
    fn from(color: RgbColor) -> Self {
        Self {
            r: color.r,
            g: color.g,
            b: color.b,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PaletteIndex(pub u8);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StyleColor {
    None,
    Palette(PaletteIndex),
    Rgb(RgbColor),
    Unknown(ffi::GhosttyStyleColorTag),
}

impl From<ffi::GhosttyStyleColor> for StyleColor {
    fn from(color: ffi::GhosttyStyleColor) -> Self {
        match color.tag {
            ffi::GhosttyStyleColorTag_GHOSTTY_STYLE_COLOR_NONE => Self::None,
            ffi::GhosttyStyleColorTag_GHOSTTY_STYLE_COLOR_PALETTE => {
                Self::Palette(PaletteIndex(unsafe { color.value.palette }))
            }
            ffi::GhosttyStyleColorTag_GHOSTTY_STYLE_COLOR_RGB => {
                Self::Rgb(unsafe { color.value.rgb }.into())
            }
            other => Self::Unknown(other),
        }
    }
}

impl From<StyleColor> for ffi::GhosttyStyleColor {
    fn from(color: StyleColor) -> Self {
        match color {
            StyleColor::None => Self {
                tag: ffi::GhosttyStyleColorTag_GHOSTTY_STYLE_COLOR_NONE,
                value: ffi::GhosttyStyleColorValue::default(),
            },
            StyleColor::Palette(PaletteIndex(palette)) => Self {
                tag: ffi::GhosttyStyleColorTag_GHOSTTY_STYLE_COLOR_PALETTE,
                value: ffi::GhosttyStyleColorValue { palette },
            },
            StyleColor::Rgb(rgb) => Self {
                tag: ffi::GhosttyStyleColorTag_GHOSTTY_STYLE_COLOR_RGB,
                value: ffi::GhosttyStyleColorValue { rgb: rgb.into() },
            },
            StyleColor::Unknown(tag) => Self {
                tag,
                value: ffi::GhosttyStyleColorValue::default(),
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Underline {
    None,
    Single,
    Double,
    Curly,
    Dotted,
    Dashed,
    Unknown(i32),
}

impl Underline {
    pub fn raw(self) -> i32 {
        match self {
            Self::None => ffi::GhosttySgrUnderline_GHOSTTY_SGR_UNDERLINE_NONE as i32,
            Self::Single => ffi::GhosttySgrUnderline_GHOSTTY_SGR_UNDERLINE_SINGLE as i32,
            Self::Double => ffi::GhosttySgrUnderline_GHOSTTY_SGR_UNDERLINE_DOUBLE as i32,
            Self::Curly => ffi::GhosttySgrUnderline_GHOSTTY_SGR_UNDERLINE_CURLY as i32,
            Self::Dotted => ffi::GhosttySgrUnderline_GHOSTTY_SGR_UNDERLINE_DOTTED as i32,
            Self::Dashed => ffi::GhosttySgrUnderline_GHOSTTY_SGR_UNDERLINE_DASHED as i32,
            Self::Unknown(raw) => raw,
        }
    }
}

impl From<i32> for Underline {
    fn from(value: i32) -> Self {
        match value as ffi::GhosttySgrUnderline {
            ffi::GhosttySgrUnderline_GHOSTTY_SGR_UNDERLINE_NONE => Self::None,
            ffi::GhosttySgrUnderline_GHOSTTY_SGR_UNDERLINE_SINGLE => Self::Single,
            ffi::GhosttySgrUnderline_GHOSTTY_SGR_UNDERLINE_DOUBLE => Self::Double,
            ffi::GhosttySgrUnderline_GHOSTTY_SGR_UNDERLINE_CURLY => Self::Curly,
            ffi::GhosttySgrUnderline_GHOSTTY_SGR_UNDERLINE_DOTTED => Self::Dotted,
            ffi::GhosttySgrUnderline_GHOSTTY_SGR_UNDERLINE_DASHED => Self::Dashed,
            _ => Self::Unknown(value),
        }
    }
}

#[derive(Clone, Copy)]
pub struct CellStyle {
    raw: GhosttyStyle,
}

impl CellStyle {
    pub fn foreground_color(self) -> StyleColor {
        self.raw.fg_color.into()
    }

    pub fn background_color(self) -> StyleColor {
        self.raw.bg_color.into()
    }

    pub fn underline_color(self) -> StyleColor {
        self.raw.underline_color.into()
    }

    pub fn bold(self) -> bool {
        self.raw.bold
    }

    pub fn italic(self) -> bool {
        self.raw.italic
    }

    pub fn faint(self) -> bool {
        self.raw.faint
    }

    pub fn blink(self) -> bool {
        self.raw.blink
    }

    pub fn inverse(self) -> bool {
        self.raw.inverse
    }

    pub fn invisible(self) -> bool {
        self.raw.invisible
    }

    pub fn strikethrough(self) -> bool {
        self.raw.strikethrough
    }

    pub fn overline(self) -> bool {
        self.raw.overline
    }

    pub fn underline_style(self) -> Underline {
        self.raw.underline.into()
    }

    pub fn underline(self) -> i32 {
        self.raw.underline
    }

    pub fn background_is_default(self) -> bool {
        self.raw.bg_color.tag == GHOSTTY_STYLE_COLOR_NONE
    }

    pub fn is_default(self) -> bool {
        unsafe { ffi::ghostty_style_is_default(&self.raw) }
    }

    pub fn raw(self) -> GhosttyStyle {
        self.raw
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OptimizeMode {
    Debug,
    ReleaseSafe,
    ReleaseSmall,
    ReleaseFast,
    Unknown(ffi::GhosttyOptimizeMode),
}

impl From<ffi::GhosttyOptimizeMode> for OptimizeMode {
    fn from(mode: ffi::GhosttyOptimizeMode) -> Self {
        match mode {
            ffi::GhosttyOptimizeMode_GHOSTTY_OPTIMIZE_DEBUG => Self::Debug,
            ffi::GhosttyOptimizeMode_GHOSTTY_OPTIMIZE_RELEASE_SAFE => Self::ReleaseSafe,
            ffi::GhosttyOptimizeMode_GHOSTTY_OPTIMIZE_RELEASE_SMALL => Self::ReleaseSmall,
            ffi::GhosttyOptimizeMode_GHOSTTY_OPTIMIZE_RELEASE_FAST => Self::ReleaseFast,
            other => Self::Unknown(other),
        }
    }
}

pub fn supports_simd() -> Result<bool, Error> {
    build_info(ffi::GhosttyBuildInfo_GHOSTTY_BUILD_INFO_SIMD)
}

pub fn supports_kitty_graphics() -> Result<bool, Error> {
    build_info(ffi::GhosttyBuildInfo_GHOSTTY_BUILD_INFO_KITTY_GRAPHICS)
}

pub fn supports_tmux_control_mode() -> Result<bool, Error> {
    build_info(ffi::GhosttyBuildInfo_GHOSTTY_BUILD_INFO_TMUX_CONTROL_MODE)
}

pub fn optimize_mode() -> Result<OptimizeMode, Error> {
    build_info::<ffi::GhosttyOptimizeMode>(ffi::GhosttyBuildInfo_GHOSTTY_BUILD_INFO_OPTIMIZE)
        .map(Into::into)
}

fn build_info<T>(data: ffi::GhosttyBuildInfo) -> Result<T, Error> {
    let mut value = MaybeUninit::<T>::zeroed();
    unsafe {
        result(ffi::ghostty_build_info(
            data,
            value.as_mut_ptr().cast::<c_void>(),
        ))?;
        Ok(value.assume_init())
    }
}

pub fn paste_is_safe(data: &str) -> bool {
    unsafe { ffi::ghostty_paste_is_safe(data.as_ptr() as *const c_char, data.len()) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusEvent {
    Gained,
    Lost,
    Unknown(ffi::GhosttyFocusEvent),
}

impl FocusEvent {
    fn raw(self) -> ffi::GhosttyFocusEvent {
        match self {
            Self::Gained => ffi::GhosttyFocusEvent_GHOSTTY_FOCUS_GAINED,
            Self::Lost => ffi::GhosttyFocusEvent_GHOSTTY_FOCUS_LOST,
            Self::Unknown(event) => event,
        }
    }

    pub fn encode(self, buf: &mut [u8]) -> Result<usize, Error> {
        encode_focus_event(self, buf)
    }

    pub fn encode_to_vec(self, vec: &mut Vec<u8>) -> Result<(), Error> {
        encode_focus_event_to_vec(self, vec)
    }
}

impl From<ffi::GhosttyFocusEvent> for FocusEvent {
    fn from(event: ffi::GhosttyFocusEvent) -> Self {
        match event {
            ffi::GhosttyFocusEvent_GHOSTTY_FOCUS_GAINED => Self::Gained,
            ffi::GhosttyFocusEvent_GHOSTTY_FOCUS_LOST => Self::Lost,
            other => Self::Unknown(other),
        }
    }
}

pub fn encode_focus_event(event: FocusEvent, buf: &mut [u8]) -> Result<usize, Error> {
    let mut written = 0usize;
    unsafe {
        result_with_len(
            ffi::ghostty_focus_encode(
                event.raw(),
                buf.as_mut_ptr() as *mut c_char,
                buf.len(),
                &mut written,
            ),
            written,
        )
    }
}

pub fn encode_focus_event_to_vec(event: FocusEvent, vec: &mut Vec<u8>) -> Result<(), Error> {
    let remaining = vec.capacity().saturating_sub(vec.len());
    let mut written = match encode_focus_event_to_uninit_buf(event, vec.spare_capacity_mut()) {
        Ok(written) => written,
        Err(Error::OutOfSpace { required }) => {
            vec.reserve(required.saturating_sub(remaining));
            encode_focus_event_to_uninit_buf(event, vec.spare_capacity_mut())?
        }
        Err(err) => return Err(err),
    };
    let old_len = vec.len();
    if written > vec.capacity().saturating_sub(old_len) {
        written = vec.capacity().saturating_sub(old_len);
    }
    unsafe { vec.set_len(old_len + written) };
    Ok(())
}

fn encode_focus_event_to_uninit_buf(
    event: FocusEvent,
    buf: &mut [MaybeUninit<u8>],
) -> Result<usize, Error> {
    let mut written = 0usize;
    let out_ptr = if buf.is_empty() {
        std::ptr::null_mut()
    } else {
        buf.as_mut_ptr().cast::<c_char>()
    };
    unsafe {
        result_with_len(
            ffi::ghostty_focus_encode(event.raw(), out_ptr, buf.len(), &mut written),
            written,
        )
    }
}

pub struct Terminal {
    raw: Handle<ffi::GhosttyTerminal>,
}

impl Terminal {
    pub fn new(cols: u16, rows: u16, max_scrollback: usize) -> Result<Self, Error> {
        let options = GhosttyTerminalOptions {
            cols,
            rows,
            max_scrollback,
        };
        let raw =
            new_handle(|out| unsafe { ffi::ghostty_terminal_new(std::ptr::null(), out, options) })?;
        Ok(Self { raw })
    }

    pub fn raw(&self) -> GhosttyTerminal {
        self.raw.as_ptr()
    }

    pub fn write(&mut self, bytes: &[u8]) {
        unsafe { ffi::ghostty_terminal_vt_write(self.raw(), bytes.as_ptr(), bytes.len()) };
    }

    pub fn set_userdata(&mut self, userdata: *mut c_void) -> Result<(), Error> {
        unsafe {
            result(ffi::ghostty_terminal_set(
                self.raw(),
                ffi::GhosttyTerminalOption_GHOSTTY_TERMINAL_OPT_USERDATA,
                userdata.cast_const(),
            ))
        }
    }

    pub fn set_write_pty(&mut self, callback: GhosttyTerminalWritePtyFn) -> Result<(), Error> {
        unsafe {
            result(ffi::ghostty_terminal_set(
                self.raw(),
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
                self.raw(),
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
                self.raw(),
                ffi::GhosttyTerminalData_GHOSTTY_TERMINAL_DATA_SCROLLBAR,
                &mut out as *mut _ as *mut c_void,
            ))?
        };
        Ok(out)
    }

    pub fn scroll_viewport_delta(&mut self, delta: isize) {
        unsafe {
            ffi::ghostty_terminal_scroll_viewport(
                self.raw(),
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
                self.raw(),
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
                self.raw(),
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
                self.raw(),
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
        unsafe { ffi::ghostty_terminal_free(self.raw()) };
    }
}

pub struct Formatter {
    raw: Handle<ffi::GhosttyFormatter>,
}

impl Formatter {
    pub fn for_terminal_hyperlinks(terminal: &Terminal) -> Result<Self, Error> {
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
        let raw = new_handle(|out| unsafe {
            ffi::ghostty_formatter_terminal_new(std::ptr::null(), out, terminal.raw(), options)
        })?;
        Ok(Self { raw })
    }

    pub fn format_alloc(&self) -> Result<Vec<u8>, Error> {
        let mut ptr = std::ptr::null_mut();
        let mut len = 0usize;
        unsafe {
            result(ffi::ghostty_formatter_format_alloc(
                self.raw.as_ptr(),
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

    pub fn format_len(&self) -> Result<usize, Error> {
        let mut required = 0usize;
        let rc = unsafe {
            ffi::ghostty_formatter_format_buf(
                self.raw.as_ptr(),
                std::ptr::null_mut(),
                0,
                &mut required,
            )
        };
        match rc {
            GHOSTTY_SUCCESS | GHOSTTY_OUT_OF_SPACE => Ok(required),
            other => result_with_len(other, required),
        }
    }

    pub fn format_buf(&self, buf: &mut [u8]) -> Result<usize, Error> {
        let mut written = 0usize;
        unsafe {
            result_with_len(
                ffi::ghostty_formatter_format_buf(
                    self.raw.as_ptr(),
                    buf.as_mut_ptr(),
                    buf.len(),
                    &mut written,
                ),
                written,
            )
        }
    }
}

impl Drop for Formatter {
    fn drop(&mut self) {
        unsafe { ffi::ghostty_formatter_free(self.raw.as_ptr()) };
    }
}

pub struct RenderState {
    raw: Handle<ffi::GhosttyRenderState>,
}

impl RenderState {
    pub fn new() -> Result<Self, Error> {
        let raw =
            new_handle(|out| unsafe { ffi::ghostty_render_state_new(std::ptr::null(), out) })?;
        Ok(Self { raw })
    }

    pub fn update(&mut self, terminal: &Terminal) -> Result<RenderSnapshot<'_>, Error> {
        unsafe {
            result(ffi::ghostty_render_state_update(
                self.raw.as_ptr(),
                terminal.raw(),
            ))
        }?;
        Ok(RenderSnapshot { state: self })
    }
}

impl Drop for RenderState {
    fn drop(&mut self) {
        unsafe { ffi::ghostty_render_state_free(self.raw.as_ptr()) };
    }
}

pub struct RenderSnapshot<'state> {
    state: &'state mut RenderState,
}

impl RenderSnapshot<'_> {
    pub fn cols(&self) -> Result<u16, Error> {
        self.get_u16(ffi::GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_COLS)
    }

    pub fn rows(&self) -> Result<u16, Error> {
        self.get_u16(ffi::GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_ROWS)
    }

    pub fn cursor(&self) -> Result<RenderCursor, Error> {
        let visible =
            self.get_bool(ffi::GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_CURSOR_VISIBLE)?;
        let blinking = self
            .get_bool(ffi::GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_CURSOR_BLINKING)
            .unwrap_or(false);
        let has_viewport = self
            .get_bool(
                ffi::GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_CURSOR_VIEWPORT_HAS_VALUE,
            )
            .unwrap_or(false);
        let (x, y) = if has_viewport {
            (
                self.get_u16(
                    ffi::GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_CURSOR_VIEWPORT_X,
                )
                .unwrap_or(0),
                self.get_u16(
                    ffi::GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_CURSOR_VIEWPORT_Y,
                )
                .unwrap_or(0),
            )
        } else {
            (0, 0)
        };
        let style = self
            .get_i32(ffi::GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_CURSOR_VISUAL_STYLE)
            .unwrap_or(0)
            .into();
        Ok(RenderCursor {
            visible,
            blinking,
            x,
            y,
            style,
        })
    }

    pub fn colors(&self) -> Result<GhosttyRenderStateColors, Error> {
        let mut colors = ffi::sized!(GhosttyRenderStateColors);
        unsafe {
            result(ffi::ghostty_render_state_colors_get(
                self.state.raw.as_ptr(),
                &mut colors,
            ))?;
        }
        Ok(colors)
    }

    fn get_u16(&self, data: ffi::GhosttyRenderStateData) -> Result<u16, Error> {
        let mut out = 0u16;
        unsafe {
            result(ffi::ghostty_render_state_get(
                self.state.raw.as_ptr(),
                data,
                &mut out as *mut _ as *mut c_void,
            ))?
        };
        Ok(out)
    }

    fn get_bool(&self, data: ffi::GhosttyRenderStateData) -> Result<bool, Error> {
        let mut out = false;
        unsafe {
            result(ffi::ghostty_render_state_get(
                self.state.raw.as_ptr(),
                data,
                &mut out as *mut _ as *mut c_void,
            ))?
        };
        Ok(out)
    }

    fn get_i32(&self, data: ffi::GhosttyRenderStateData) -> Result<i32, Error> {
        let mut out = 0i32;
        unsafe {
            result(ffi::ghostty_render_state_get(
                self.state.raw.as_ptr(),
                data,
                &mut out as *mut _ as *mut c_void,
            ))?
        };
        Ok(out)
    }
}

pub struct RowIterator {
    raw: Handle<ffi::GhosttyRenderStateRowIterator>,
}

impl RowIterator {
    pub fn new() -> Result<Self, Error> {
        let raw = new_handle(|out| unsafe {
            ffi::ghostty_render_state_row_iterator_new(std::ptr::null(), out)
        })?;
        Ok(Self { raw })
    }

    pub fn update<'snapshot>(
        &'snapshot mut self,
        snapshot: &'snapshot RenderSnapshot<'_>,
    ) -> Result<RowIteration<'snapshot>, Error> {
        let mut raw = self.raw.as_ptr();
        unsafe {
            result(ffi::ghostty_render_state_get(
                snapshot.state.raw.as_ptr(),
                ffi::GhosttyRenderStateData_GHOSTTY_RENDER_STATE_DATA_ROW_ITERATOR,
                &mut raw as *mut _ as *mut c_void,
            ))?;
        }
        self.raw = Handle::new(raw)?;
        Ok(RowIteration {
            iter: self,
            _snapshot: PhantomData,
        })
    }
}

impl Drop for RowIterator {
    fn drop(&mut self) {
        unsafe { ffi::ghostty_render_state_row_iterator_free(self.raw.as_ptr()) };
    }
}

pub struct RowIteration<'snapshot> {
    iter: &'snapshot mut RowIterator,
    _snapshot: PhantomData<&'snapshot RenderSnapshot<'snapshot>>,
}

impl RowIteration<'_> {
    pub fn next(&mut self) -> Option<&Self> {
        if unsafe { ffi::ghostty_render_state_row_iterator_next(self.iter.raw.as_ptr()) } {
            Some(self)
        } else {
            None
        }
    }

    pub fn dirty(&self) -> Result<bool, Error> {
        let mut out = false;
        unsafe {
            result(ffi::ghostty_render_state_row_get(
                self.iter.raw.as_ptr(),
                ffi::GhosttyRenderStateRowData_GHOSTTY_RENDER_STATE_ROW_DATA_DIRTY,
                &mut out as *mut _ as *mut c_void,
            ))?
        };
        Ok(out)
    }

    pub fn clear_dirty(&self) -> Result<(), Error> {
        let clean = false;
        unsafe {
            result(ffi::ghostty_render_state_row_set(
                self.iter.raw.as_ptr(),
                ffi::GhosttyRenderStateRowOption_GHOSTTY_RENDER_STATE_ROW_OPTION_DIRTY,
                &clean as *const _ as *const c_void,
            ))
        }
    }
}

pub struct CellIterator {
    raw: Handle<ffi::GhosttyRenderStateRowCells>,
}

impl CellIterator {
    pub fn new() -> Result<Self, Error> {
        let raw = new_handle(|out| unsafe {
            ffi::ghostty_render_state_row_cells_new(std::ptr::null(), out)
        })?;
        Ok(Self { raw })
    }

    pub fn update<'row>(
        &'row mut self,
        row: &'row RowIteration<'_>,
    ) -> Result<CellIteration<'row>, Error> {
        let mut raw = self.raw.as_ptr();
        unsafe {
            result(ffi::ghostty_render_state_row_get(
                row.iter.raw.as_ptr(),
                ffi::GhosttyRenderStateRowData_GHOSTTY_RENDER_STATE_ROW_DATA_CELLS,
                &mut raw as *mut _ as *mut c_void,
            ))?;
        }
        self.raw = Handle::new(raw)?;
        Ok(CellIteration {
            iter: self,
            _row: PhantomData,
        })
    }
}

impl Drop for CellIterator {
    fn drop(&mut self) {
        unsafe { ffi::ghostty_render_state_row_cells_free(self.raw.as_ptr()) };
    }
}

pub struct CellIteration<'row> {
    iter: &'row mut CellIterator,
    _row: PhantomData<&'row RowIteration<'row>>,
}

impl CellIteration<'_> {
    pub fn next(&mut self) -> Option<&Self> {
        if unsafe { ffi::ghostty_render_state_row_cells_next(self.iter.raw.as_ptr()) } {
            Some(self)
        } else {
            None
        }
    }

    pub fn style(&self) -> Result<CellStyle, Error> {
        let mut style = ffi::sized!(GhosttyStyle);
        unsafe {
            result(ffi::ghostty_render_state_row_cells_get(
                self.iter.raw.as_ptr(),
                ffi::GhosttyRenderStateRowCellsData_GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_STYLE,
                &mut style as *mut _ as *mut c_void,
            ))?;
        }
        Ok(CellStyle { raw: style })
    }

    pub fn grapheme_len(&self) -> Result<u32, Error> {
        let mut out = 0u32;
        unsafe {
            result(ffi::ghostty_render_state_row_cells_get(
                self.iter.raw.as_ptr(),
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
                self.iter.raw.as_ptr(),
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
                self.iter.raw.as_ptr(),
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
                self.iter.raw.as_ptr(),
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
                self.iter.raw.as_ptr(),
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

pub struct KeyEncoder {
    raw: Handle<ffi::GhosttyKeyEncoder>,
}

impl KeyEncoder {
    pub fn new() -> Result<Self, Error> {
        let raw = new_handle(|out| unsafe { ffi::ghostty_key_encoder_new(std::ptr::null(), out) })?;
        Ok(Self { raw })
    }

    pub fn sync_from_terminal(&mut self, terminal: &Terminal) {
        unsafe { ffi::ghostty_key_encoder_setopt_from_terminal(self.raw.as_ptr(), terminal.raw()) };
    }

    pub fn set_cursor_key_application(&mut self, value: bool) -> &mut Self {
        self.setopt_bool(
            ffi::GhosttyKeyEncoderOption_GHOSTTY_KEY_ENCODER_OPT_CURSOR_KEY_APPLICATION,
            value,
        )
    }

    pub fn set_keypad_key_application(&mut self, value: bool) -> &mut Self {
        self.setopt_bool(
            ffi::GhosttyKeyEncoderOption_GHOSTTY_KEY_ENCODER_OPT_KEYPAD_KEY_APPLICATION,
            value,
        )
    }

    pub fn set_ignore_keypad_with_numlock(&mut self, value: bool) -> &mut Self {
        self.setopt_bool(
            ffi::GhosttyKeyEncoderOption_GHOSTTY_KEY_ENCODER_OPT_IGNORE_KEYPAD_WITH_NUMLOCK,
            value,
        )
    }

    pub fn set_alt_esc_prefix(&mut self, value: bool) -> &mut Self {
        self.setopt_bool(
            ffi::GhosttyKeyEncoderOption_GHOSTTY_KEY_ENCODER_OPT_ALT_ESC_PREFIX,
            value,
        )
    }

    pub fn set_modify_other_keys_state_2(&mut self, value: bool) -> &mut Self {
        self.setopt_bool(
            ffi::GhosttyKeyEncoderOption_GHOSTTY_KEY_ENCODER_OPT_MODIFY_OTHER_KEYS_STATE_2,
            value,
        )
    }

    pub fn set_kitty_flags(&mut self, value: KittyKeyFlags) -> &mut Self {
        let value = value.bits();
        self.setopt(
            ffi::GhosttyKeyEncoderOption_GHOSTTY_KEY_ENCODER_OPT_KITTY_FLAGS,
            &value as *const _ as *const c_void,
        )
    }

    pub fn set_macos_option_as_alt(&mut self, value: OptionAsAlt) -> &mut Self {
        let value = value.raw();
        self.setopt(
            ffi::GhosttyKeyEncoderOption_GHOSTTY_KEY_ENCODER_OPT_MACOS_OPTION_AS_ALT,
            &value as *const _ as *const c_void,
        )
    }

    pub fn encode(&mut self, event: &KeyEvent) -> Result<Vec<u8>, Error> {
        let mut out = Vec::with_capacity(64);
        self.encode_to_vec(event, &mut out)?;
        Ok(out)
    }

    pub fn encode_buf(&mut self, event: &KeyEvent, buf: &mut [u8]) -> Result<usize, Error> {
        let buf = unsafe { std::slice::from_raw_parts_mut(buf.as_mut_ptr().cast(), buf.len()) };
        self.encode_to_uninit_buf(event, buf)
    }

    pub fn encode_to_vec(&mut self, event: &KeyEvent, vec: &mut Vec<u8>) -> Result<(), Error> {
        let remaining = vec.capacity().saturating_sub(vec.len());
        let mut written = match self.encode_to_uninit_buf(event, vec.spare_capacity_mut()) {
            Ok(written) => written,
            Err(Error::OutOfSpace { required }) => {
                vec.reserve(required.saturating_sub(remaining));
                self.encode_to_uninit_buf(event, vec.spare_capacity_mut())?
            }
            Err(err) => return Err(err),
        };

        let old_len = vec.len();
        if written > vec.capacity().saturating_sub(old_len) {
            written = vec.capacity().saturating_sub(old_len);
        }
        unsafe { vec.set_len(old_len + written) };
        Ok(())
    }

    fn encode_to_uninit_buf(
        &mut self,
        event: &KeyEvent,
        buf: &mut [MaybeUninit<u8>],
    ) -> Result<usize, Error> {
        let mut written = 0usize;
        let out_ptr = if buf.is_empty() {
            std::ptr::null_mut()
        } else {
            buf.as_mut_ptr().cast::<c_char>()
        };
        unsafe {
            result_with_len(
                ffi::ghostty_key_encoder_encode(
                    self.raw.as_ptr(),
                    event.raw.as_ptr(),
                    out_ptr,
                    buf.len(),
                    &mut written,
                ),
                written,
            )
        }
    }

    fn setopt_bool(&mut self, option: ffi::GhosttyKeyEncoderOption, value: bool) -> &mut Self {
        self.setopt(option, &value as *const _ as *const c_void)
    }

    fn setopt(&mut self, option: ffi::GhosttyKeyEncoderOption, value: *const c_void) -> &mut Self {
        unsafe { ffi::ghostty_key_encoder_setopt(self.raw.as_ptr(), option, value) };
        self
    }
}

impl Drop for KeyEncoder {
    fn drop(&mut self) {
        unsafe { ffi::ghostty_key_encoder_free(self.raw.as_ptr()) };
    }
}

pub struct KeyEvent {
    raw: Handle<ffi::GhosttyKeyEvent>,
    text: Option<String>,
}

impl KeyEvent {
    pub fn new() -> Result<Self, Error> {
        let raw = new_handle(|out| unsafe { ffi::ghostty_key_event_new(std::ptr::null(), out) })?;
        Ok(Self { raw, text: None })
    }

    pub fn set_action(&mut self, action: impl Into<KeyAction>) {
        unsafe { ffi::ghostty_key_event_set_action(self.raw.as_ptr(), action.into().raw()) };
    }

    pub fn set_key(&mut self, key: GhosttyKey) {
        unsafe { ffi::ghostty_key_event_set_key(self.raw.as_ptr(), key) };
    }

    pub fn set_mods(&mut self, mods: impl Into<KeyMods>) {
        unsafe { ffi::ghostty_key_event_set_mods(self.raw.as_ptr(), mods.into().bits()) };
    }

    pub fn set_consumed_mods(&mut self, mods: impl Into<KeyMods>) {
        unsafe { ffi::ghostty_key_event_set_consumed_mods(self.raw.as_ptr(), mods.into().bits()) };
    }

    pub fn set_composing(&mut self, composing: bool) {
        unsafe { ffi::ghostty_key_event_set_composing(self.raw.as_ptr(), composing) };
    }

    pub fn set_utf8(&mut self, utf8: &str) {
        self.text = Some(utf8.to_string());
        let utf8 = self.text.as_deref().unwrap_or_default();
        unsafe {
            ffi::ghostty_key_event_set_utf8(
                self.raw.as_ptr(),
                utf8.as_ptr() as *const c_char,
                utf8.len(),
            )
        };
    }

    pub fn clear_utf8(&mut self) {
        self.text = None;
        unsafe { ffi::ghostty_key_event_set_utf8(self.raw.as_ptr(), std::ptr::null(), 0) };
    }

    pub fn utf8(&self) -> Option<&str> {
        self.text.as_deref()
    }

    pub fn set_unshifted_codepoint(&mut self, codepoint: u32) {
        unsafe { ffi::ghostty_key_event_set_unshifted_codepoint(self.raw.as_ptr(), codepoint) };
    }
}

impl Drop for KeyEvent {
    fn drop(&mut self) {
        unsafe { ffi::ghostty_key_event_free(self.raw.as_ptr()) };
    }
}

pub struct MouseEncoder {
    raw: Handle<ffi::GhosttyMouseEncoder>,
}

impl MouseEncoder {
    pub fn new() -> Result<Self, Error> {
        let raw =
            new_handle(|out| unsafe { ffi::ghostty_mouse_encoder_new(std::ptr::null(), out) })?;
        Ok(Self { raw })
    }

    pub fn sync_from_terminal(&mut self, terminal: &Terminal) {
        unsafe {
            ffi::ghostty_mouse_encoder_setopt_from_terminal(self.raw.as_ptr(), terminal.raw())
        };
    }

    pub fn set_size(&mut self, size: &GhosttyMouseEncoderSize) {
        unsafe {
            ffi::ghostty_mouse_encoder_setopt(
                self.raw.as_ptr(),
                ffi::GhosttyMouseEncoderOption_GHOSTTY_MOUSE_ENCODER_OPT_SIZE,
                size as *const _ as *const c_void,
            )
        };
    }

    pub fn set_geometry(&mut self, geometry: MouseGeometry) {
        let size = geometry.into();
        self.set_size(&size);
    }

    pub fn set_tracking_mode(&mut self, mode: MouseTrackingMode) -> &mut Self {
        let mode = mode.raw();
        self.setopt(
            ffi::GhosttyMouseEncoderOption_GHOSTTY_MOUSE_ENCODER_OPT_EVENT,
            &mode as *const _ as *const c_void,
        )
    }

    pub fn set_format(&mut self, format: MouseFormat) -> &mut Self {
        let format = format.raw();
        self.setopt(
            ffi::GhosttyMouseEncoderOption_GHOSTTY_MOUSE_ENCODER_OPT_FORMAT,
            &format as *const _ as *const c_void,
        )
    }

    pub fn set_any_button_pressed(&mut self, pressed: bool) -> &mut Self {
        self.setopt(
            ffi::GhosttyMouseEncoderOption_GHOSTTY_MOUSE_ENCODER_OPT_ANY_BUTTON_PRESSED,
            &pressed as *const _ as *const c_void,
        )
    }

    pub fn set_track_last_cell(&mut self, track: bool) -> &mut Self {
        self.setopt(
            ffi::GhosttyMouseEncoderOption_GHOSTTY_MOUSE_ENCODER_OPT_TRACK_LAST_CELL,
            &track as *const _ as *const c_void,
        )
    }

    pub fn reset(&mut self) {
        unsafe { ffi::ghostty_mouse_encoder_reset(self.raw.as_ptr()) };
    }

    pub fn encode(&mut self, event: &MouseEvent) -> Result<Vec<u8>, Error> {
        let mut out = Vec::with_capacity(64);
        self.encode_to_vec(event, &mut out)?;
        Ok(out)
    }

    pub fn encode_buf(&mut self, event: &MouseEvent, buf: &mut [u8]) -> Result<usize, Error> {
        let buf = unsafe { std::slice::from_raw_parts_mut(buf.as_mut_ptr().cast(), buf.len()) };
        self.encode_to_uninit_buf(event, buf)
    }

    pub fn encode_to_vec(&mut self, event: &MouseEvent, vec: &mut Vec<u8>) -> Result<(), Error> {
        let remaining = vec.capacity().saturating_sub(vec.len());
        let mut written = match self.encode_to_uninit_buf(event, vec.spare_capacity_mut()) {
            Ok(written) => written,
            Err(Error::OutOfSpace { required }) => {
                vec.reserve(required.saturating_sub(remaining));
                self.encode_to_uninit_buf(event, vec.spare_capacity_mut())?
            }
            Err(err) => return Err(err),
        };

        let old_len = vec.len();
        if written > vec.capacity().saturating_sub(old_len) {
            written = vec.capacity().saturating_sub(old_len);
        }
        unsafe { vec.set_len(old_len + written) };
        Ok(())
    }

    fn encode_to_uninit_buf(
        &mut self,
        event: &MouseEvent,
        buf: &mut [MaybeUninit<u8>],
    ) -> Result<usize, Error> {
        let mut written = 0usize;
        let out_ptr = if buf.is_empty() {
            std::ptr::null_mut()
        } else {
            buf.as_mut_ptr().cast::<c_char>()
        };
        unsafe {
            result_with_len(
                ffi::ghostty_mouse_encoder_encode(
                    self.raw.as_ptr(),
                    event.raw.as_ptr(),
                    out_ptr,
                    buf.len(),
                    &mut written,
                ),
                written,
            )
        }
    }

    fn setopt(
        &mut self,
        option: ffi::GhosttyMouseEncoderOption,
        value: *const c_void,
    ) -> &mut Self {
        unsafe { ffi::ghostty_mouse_encoder_setopt(self.raw.as_ptr(), option, value) };
        self
    }
}

impl Drop for MouseEncoder {
    fn drop(&mut self) {
        unsafe { ffi::ghostty_mouse_encoder_free(self.raw.as_ptr()) };
    }
}

pub struct MouseEvent {
    raw: Handle<ffi::GhosttyMouseEvent>,
}

pub type GhosttyTerminalWritePtyFn = ffi::GhosttyTerminalWritePtyFn;

impl MouseEvent {
    pub fn new() -> Result<Self, Error> {
        let raw = new_handle(|out| unsafe { ffi::ghostty_mouse_event_new(std::ptr::null(), out) })?;
        Ok(Self { raw })
    }

    pub fn set_action(&mut self, action: impl Into<MouseAction>) {
        unsafe { ffi::ghostty_mouse_event_set_action(self.raw.as_ptr(), action.into().raw()) };
    }

    pub fn set_button(&mut self, button: impl Into<MouseButton>) {
        unsafe { ffi::ghostty_mouse_event_set_button(self.raw.as_ptr(), button.into().raw()) };
    }

    pub fn clear_button(&mut self) {
        unsafe { ffi::ghostty_mouse_event_clear_button(self.raw.as_ptr()) };
    }

    pub fn set_mods(&mut self, mods: impl Into<KeyMods>) {
        unsafe { ffi::ghostty_mouse_event_set_mods(self.raw.as_ptr(), mods.into().bits()) };
    }

    pub fn set_position(&mut self, x: f32, y: f32) {
        let pos = GhosttyMousePosition { x, y };
        unsafe { ffi::ghostty_mouse_event_set_position(self.raw.as_ptr(), pos) };
    }
}

impl Drop for MouseEvent {
    fn drop(&mut self) {
        unsafe { ffi::ghostty_mouse_event_free(self.raw.as_ptr()) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn result_mapping_preserves_typed_errors() {
        assert_eq!(result(GHOSTTY_SUCCESS), Ok(()));
        assert_eq!(result(GHOSTTY_OUT_OF_MEMORY), Err(Error::OutOfMemory));
        assert_eq!(result(GHOSTTY_NO_VALUE), Err(Error::InvalidValue));
        assert_eq!(
            result_with_len(GHOSTTY_OUT_OF_SPACE, 7),
            Err(Error::OutOfSpace { required: 7 })
        );
        assert_eq!(result(-99), Err(Error::Unknown(-99)));
    }

    #[test]
    fn null_handle_maps_to_out_of_memory() {
        let handle = Handle::<ffi::GhosttyTerminal>::new(std::ptr::null_mut());
        assert!(matches!(handle, Err(Error::OutOfMemory)));
    }

    #[test]
    fn style_color_converts_none_palette_and_rgb() {
        let none = ffi::GhosttyStyleColor {
            tag: ffi::GhosttyStyleColorTag_GHOSTTY_STYLE_COLOR_NONE,
            value: ffi::GhosttyStyleColorValue::default(),
        };
        assert_eq!(StyleColor::from(none), StyleColor::None);

        let palette = ffi::GhosttyStyleColor {
            tag: ffi::GhosttyStyleColorTag_GHOSTTY_STYLE_COLOR_PALETTE,
            value: ffi::GhosttyStyleColorValue { palette: 42 },
        };
        assert_eq!(
            StyleColor::from(palette),
            StyleColor::Palette(PaletteIndex(42))
        );

        let rgb = ffi::GhosttyStyleColor {
            tag: ffi::GhosttyStyleColorTag_GHOSTTY_STYLE_COLOR_RGB,
            value: ffi::GhosttyStyleColorValue {
                rgb: GhosttyColorRgb { r: 1, g: 2, b: 3 },
            },
        };
        assert_eq!(
            StyleColor::from(rgb),
            StyleColor::Rgb(RgbColor { r: 1, g: 2, b: 3 })
        );
    }

    #[test]
    fn cell_style_exposes_typed_flags_and_colors() {
        let mut raw = ffi::sized!(GhosttyStyle);
        raw.bold = true;
        raw.italic = true;
        raw.faint = true;
        raw.blink = true;
        raw.inverse = true;
        raw.invisible = true;
        raw.strikethrough = true;
        raw.overline = true;
        raw.underline = ffi::GhosttySgrUnderline_GHOSTTY_SGR_UNDERLINE_CURLY as i32;
        raw.fg_color = StyleColor::Rgb(RgbColor {
            r: 10,
            g: 20,
            b: 30,
        })
        .into();

        let style = CellStyle { raw };
        assert!(style.bold());
        assert!(style.italic());
        assert!(style.faint());
        assert!(style.blink());
        assert!(style.inverse());
        assert!(style.invisible());
        assert!(style.strikethrough());
        assert!(style.overline());
        assert_eq!(style.underline_style(), Underline::Curly);
        assert_eq!(
            style.foreground_color(),
            StyleColor::Rgb(RgbColor {
                r: 10,
                g: 20,
                b: 30
            })
        );
    }

    #[test]
    fn mouse_button_maps_extended_buttons() {
        assert_eq!(
            MouseButton::from(ffi::GhosttyMouseButton_GHOSTTY_MOUSE_BUTTON_FOUR),
            MouseButton::Four
        );
        assert_eq!(
            MouseButton::from(ffi::GhosttyMouseButton_GHOSTTY_MOUSE_BUTTON_ELEVEN),
            MouseButton::Eleven
        );
    }

    #[test]
    fn focus_event_encodes_with_typed_out_of_space() {
        let mut empty = [];
        assert_eq!(
            FocusEvent::Gained.encode(&mut empty),
            Err(Error::OutOfSpace { required: 3 })
        );

        let mut buf = [0u8; 8];
        let written = FocusEvent::Gained.encode(&mut buf).unwrap();
        assert_eq!(&buf[..written], b"\x1b[I");
    }

    #[test]
    fn focus_event_appends_to_reused_vec() {
        let mut buf = Vec::with_capacity(1);
        FocusEvent::Lost.encode_to_vec(&mut buf).unwrap();
        assert_eq!(buf, b"\x1b[O");
    }

    #[test]
    fn paste_safety_rejects_newline_and_bracketed_paste_end() {
        assert!(paste_is_safe("hello"));
        assert!(!paste_is_safe("echo hello\n"));
        assert!(!paste_is_safe("safe prefix\x1b[201~unsafe suffix"));
    }

    #[test]
    fn key_encoder_reuses_vec_and_fixed_buffers() {
        let mut event = KeyEvent::new().unwrap();
        event.set_action(KeyAction::Press);
        event.set_key(ffi::GhosttyKey_GHOSTTY_KEY_A);
        event.set_mods(KeyMods::empty());
        event.set_consumed_mods(KeyMods::empty());
        event.set_composing(false);
        event.set_unshifted_codepoint('a' as u32);
        event.set_utf8("a");

        let mut encoder = KeyEncoder::new().unwrap();
        let allocated = encoder.encode(&event).unwrap();

        let mut reused = Vec::with_capacity(1);
        encoder.encode_to_vec(&event, &mut reused).unwrap();
        assert_eq!(reused, allocated);

        let mut fixed = [0u8; 8];
        let written = encoder.encode_buf(&event, &mut fixed).unwrap();
        assert_eq!(&fixed[..written], allocated.as_slice());
    }
}
