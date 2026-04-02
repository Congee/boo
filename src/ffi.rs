//! Hand-written FFI bindings for libghostty's C API (ghostty.h).
//! Covers the minimal surface needed for the iced prototype.

#![allow(non_camel_case_types)]
#![allow(dead_code)]

use std::ffi::c_void;
use std::os::raw::c_char;

// --- Opaque types ---

pub type ghostty_app_t = *mut c_void;
pub type ghostty_config_t = *mut c_void;
pub type ghostty_surface_t = *mut c_void;

// --- Constants ---

pub const GHOSTTY_SUCCESS: i32 = 0;

// --- Enums ---

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub enum ghostty_platform_e {
    GHOSTTY_PLATFORM_INVALID = 0,
    GHOSTTY_PLATFORM_MACOS = 1,
    GHOSTTY_PLATFORM_IOS = 2,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub enum ghostty_clipboard_e {
    GHOSTTY_CLIPBOARD_STANDARD = 0,
    GHOSTTY_CLIPBOARD_SELECTION = 1,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub enum ghostty_clipboard_request_e {
    GHOSTTY_CLIPBOARD_REQUEST_PASTE = 0,
    GHOSTTY_CLIPBOARD_REQUEST_OSC_52_READ = 1,
    GHOSTTY_CLIPBOARD_REQUEST_OSC_52_WRITE = 2,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub enum ghostty_input_action_e {
    GHOSTTY_ACTION_RELEASE = 0,
    GHOSTTY_ACTION_PRESS = 1,
    GHOSTTY_ACTION_REPEAT = 2,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub enum ghostty_input_mouse_state_e {
    GHOSTTY_MOUSE_RELEASE = 0,
    GHOSTTY_MOUSE_PRESS = 1,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub enum ghostty_input_mouse_button_e {
    GHOSTTY_MOUSE_UNKNOWN = 0,
    GHOSTTY_MOUSE_LEFT = 1,
    GHOSTTY_MOUSE_RIGHT = 2,
    GHOSTTY_MOUSE_MIDDLE = 3,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub enum ghostty_color_scheme_e {
    GHOSTTY_COLOR_SCHEME_LIGHT = 0,
    GHOSTTY_COLOR_SCHEME_DARK = 1,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub enum ghostty_action_split_direction_e {
    GHOSTTY_SPLIT_DIRECTION_RIGHT = 0,
    GHOSTTY_SPLIT_DIRECTION_DOWN = 1,
    GHOSTTY_SPLIT_DIRECTION_LEFT = 2,
    GHOSTTY_SPLIT_DIRECTION_UP = 3,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub enum ghostty_action_goto_split_e {
    GHOSTTY_GOTO_SPLIT_PREVIOUS = 0,
    GHOSTTY_GOTO_SPLIT_NEXT = 1,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub enum ghostty_surface_context_e {
    GHOSTTY_SURFACE_CONTEXT_WINDOW = 0,
    GHOSTTY_SURFACE_CONTEXT_TAB = 1,
    GHOSTTY_SURFACE_CONTEXT_SPLIT = 2,
}

// Modifier bitmask
pub type ghostty_input_mods_e = i32;
pub const GHOSTTY_MODS_NONE: i32 = 0;
pub const GHOSTTY_MODS_SHIFT: i32 = 1 << 0;
pub const GHOSTTY_MODS_CTRL: i32 = 1 << 1;
pub const GHOSTTY_MODS_ALT: i32 = 1 << 2;
pub const GHOSTTY_MODS_SUPER: i32 = 1 << 3;

pub type ghostty_input_scroll_mods_t = i32;

// --- Structs ---

#[repr(C)]
pub struct ghostty_clipboard_content_s {
    pub mime: *const c_char,
    pub data: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_platform_macos_s {
    pub nsview: *mut c_void,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_platform_ios_s {
    pub uiview: *mut c_void,
}

#[repr(C)]
pub union ghostty_platform_u {
    pub macos: ghostty_platform_macos_s,
    pub ios: ghostty_platform_ios_s,
}

#[repr(C)]
pub struct ghostty_surface_config_s {
    pub platform_tag: i32,
    pub platform: ghostty_platform_u,
    pub userdata: *mut c_void,
    pub scale_factor: f64,
    pub font_size: f32,
    pub working_directory: *const c_char,
    pub command: *const c_char,
    pub env_vars: *mut c_void,
    pub env_var_count: usize,
    pub initial_input: *const c_char,
    pub wait_after_command: bool,
    pub context: ghostty_surface_context_e,
}

#[repr(C)]
pub struct ghostty_surface_size_s {
    pub columns: u16,
    pub rows: u16,
    pub width_px: u32,
    pub height_px: u32,
    pub cell_width_px: u32,
    pub cell_height_px: u32,
}

#[repr(C)]
pub struct ghostty_input_key_s {
    pub action: ghostty_input_action_e,
    pub mods: ghostty_input_mods_e,
    pub consumed_mods: ghostty_input_mods_e,
    pub keycode: u32,
    pub text: *const c_char,
    pub unshifted_codepoint: u32,
    pub composing: bool,
}

#[repr(C)]
pub struct ghostty_info_s {
    pub build_mode: i32,
    pub version: *const c_char,
    pub version_len: usize,
}

#[repr(C)]
pub struct ghostty_diagnostic_s {
    pub message: *const c_char,
}

// --- Action types (for the action callback) ---

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ghostty_action_tag_e {
    GHOSTTY_ACTION_QUIT = 0,
    GHOSTTY_ACTION_NEW_WINDOW = 1,
    GHOSTTY_ACTION_NEW_TAB = 2,
    GHOSTTY_ACTION_CLOSE_TAB = 3,
    GHOSTTY_ACTION_NEW_SPLIT = 4,
    GHOSTTY_ACTION_CLOSE_ALL_WINDOWS = 5,
    GHOSTTY_ACTION_TOGGLE_MAXIMIZE = 6,
    GHOSTTY_ACTION_TOGGLE_FULLSCREEN = 7,
    GHOSTTY_ACTION_TOGGLE_TAB_OVERVIEW = 8,
    GHOSTTY_ACTION_TOGGLE_WINDOW_DECORATIONS = 9,
    GHOSTTY_ACTION_TOGGLE_QUICK_TERMINAL = 10,
    GHOSTTY_ACTION_TOGGLE_COMMAND_PALETTE = 11,
    GHOSTTY_ACTION_TOGGLE_VISIBILITY = 12,
    GHOSTTY_ACTION_TOGGLE_BACKGROUND_OPACITY = 13,
    GHOSTTY_ACTION_MOVE_TAB = 14,
    GHOSTTY_ACTION_GOTO_TAB = 15,
    GHOSTTY_ACTION_GOTO_SPLIT = 16,
    GHOSTTY_ACTION_GOTO_WINDOW = 17,
    GHOSTTY_ACTION_RESIZE_SPLIT = 18,
    GHOSTTY_ACTION_EQUALIZE_SPLITS = 19,
    GHOSTTY_ACTION_TOGGLE_SPLIT_ZOOM = 20,
    GHOSTTY_ACTION_PRESENT_TERMINAL = 21,
    GHOSTTY_ACTION_SIZE_LIMIT = 22,
    GHOSTTY_ACTION_RESET_WINDOW_SIZE = 23,
    GHOSTTY_ACTION_INITIAL_SIZE = 24,
    GHOSTTY_ACTION_CELL_SIZE = 25,
    GHOSTTY_ACTION_SCROLLBAR = 26,
    GHOSTTY_ACTION_RENDER = 27,
    GHOSTTY_ACTION_INSPECTOR = 28,
    GHOSTTY_ACTION_SHOW_GTK_INSPECTOR = 29,
    GHOSTTY_ACTION_RENDER_INSPECTOR = 30,
    GHOSTTY_ACTION_DESKTOP_NOTIFICATION = 31,
    GHOSTTY_ACTION_SET_TITLE = 32,
    GHOSTTY_ACTION_SET_TAB_TITLE = 33,
    GHOSTTY_ACTION_PROMPT_TITLE = 34,
    GHOSTTY_ACTION_PWD = 35,
    GHOSTTY_ACTION_MOUSE_SHAPE = 36,
    GHOSTTY_ACTION_MOUSE_VISIBILITY = 37,
    GHOSTTY_ACTION_MOUSE_OVER_LINK = 38,
    GHOSTTY_ACTION_RENDERER_HEALTH = 39,
    GHOSTTY_ACTION_OPEN_CONFIG = 40,
    GHOSTTY_ACTION_QUIT_TIMER = 41,
    GHOSTTY_ACTION_FLOAT_WINDOW = 42,
    GHOSTTY_ACTION_SECURE_INPUT = 43,
    GHOSTTY_ACTION_KEY_SEQUENCE = 44,
    GHOSTTY_ACTION_KEY_TABLE = 45,
    GHOSTTY_ACTION_COLOR_CHANGE = 46,
    GHOSTTY_ACTION_RELOAD_CONFIG = 47,
    GHOSTTY_ACTION_CONFIG_CHANGE = 48,
    GHOSTTY_ACTION_CLOSE_WINDOW = 49,
    GHOSTTY_ACTION_RING_BELL = 50,
    GHOSTTY_ACTION_UNDO = 51,
    GHOSTTY_ACTION_REDO = 52,
    GHOSTTY_ACTION_CHECK_FOR_UPDATES = 53,
    GHOSTTY_ACTION_OPEN_URL = 54,
    GHOSTTY_ACTION_SHOW_CHILD_EXITED = 55,
    GHOSTTY_ACTION_PROGRESS_REPORT = 56,
    GHOSTTY_ACTION_SHOW_ON_SCREEN_KEYBOARD = 57,
    GHOSTTY_ACTION_COMMAND_FINISHED = 58,
    GHOSTTY_ACTION_START_SEARCH = 59,
    GHOSTTY_ACTION_END_SEARCH = 60,
    GHOSTTY_ACTION_SEARCH_TOTAL = 61,
    GHOSTTY_ACTION_SEARCH_SELECTED = 62,
    GHOSTTY_ACTION_READONLY = 63,
    GHOSTTY_ACTION_COPY_TITLE_TO_CLIPBOARD = 64,
}

// Target for action dispatch
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ghostty_target_tag_e {
    GHOSTTY_TARGET_APP = 0,
    GHOSTTY_TARGET_SURFACE = 1,
}

#[repr(C)]
pub union ghostty_target_u {
    pub surface: ghostty_surface_t,
}

#[repr(C)]
pub struct ghostty_target_s {
    pub tag: ghostty_target_tag_e,
    pub target: ghostty_target_u,
}

// Action payload structs (for the variants we care about)
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_action_set_title_s {
    pub title: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_action_cell_size_s {
    pub width: u32,
    pub height: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_action_initial_size_s {
    pub width: u32,
    pub height: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_action_pwd_s {
    pub pwd: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_action_scrollbar_s {
    pub total: u64,
    pub offset: u64,
    pub len: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_action_search_total_s {
    pub total: isize,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_action_search_selected_s {
    pub selected: isize,
}

// --- Copy mode types ---

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_point_s {
    pub tag: u32,  // ghostty_point_tag_e
    pub coord: u32, // ghostty_point_coord_e
    pub x: u32,
    pub y: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_selection_s {
    pub top_left: ghostty_point_s,
    pub bottom_right: ghostty_point_s,
    pub rectangle: bool,
}

#[repr(C)]
pub struct ghostty_text_s {
    pub tl_px_x: f64,
    pub tl_px_y: f64,
    pub offset_start: u32,
    pub offset_len: u32,
    pub text: *const c_char,
    pub text_len: usize,
}

// Point tag values
pub const GHOSTTY_POINT_VIEWPORT: u32 = 1;
// Point coord values
pub const GHOSTTY_POINT_COORD_EXACT: u32 = 0;

// ghostty_action_s: { tag (4 bytes), pad (4 bytes), union (24 bytes) } = 32 bytes total.
// The union is 24 bytes with 8-byte alignment (contains pointers).
#[repr(C)]
pub struct ghostty_action_s {
    pub tag: ghostty_action_tag_e,
    pub _payload: [u64; 3], // 24 bytes, 8-byte aligned (matches ghostty_action_u)
}

impl ghostty_action_s {
    /// Read the payload as a specific type. Caller must ensure the tag matches.
    pub unsafe fn payload<T: Copy>(&self) -> T {
        unsafe { std::ptr::read_unaligned(self._payload.as_ptr() as *const T) }
    }
}

// --- Callback types ---

pub type ghostty_runtime_wakeup_cb = Option<unsafe extern "C" fn(*mut c_void)>;

pub type ghostty_runtime_action_cb = Option<
    unsafe extern "C" fn(ghostty_app_t, ghostty_target_s, ghostty_action_s) -> bool,
>;

pub type ghostty_runtime_read_clipboard_cb =
    Option<unsafe extern "C" fn(*mut c_void, ghostty_clipboard_e, *mut c_void) -> bool>;

pub type ghostty_runtime_confirm_read_clipboard_cb = Option<
    unsafe extern "C" fn(
        *mut c_void,
        *const c_char,
        *mut c_void,
        ghostty_clipboard_request_e,
    ),
>;

pub type ghostty_runtime_write_clipboard_cb = Option<
    unsafe extern "C" fn(
        *mut c_void,
        ghostty_clipboard_e,
        *const ghostty_clipboard_content_s,
        usize,
        bool,
    ),
>;

pub type ghostty_runtime_close_surface_cb =
    Option<unsafe extern "C" fn(*mut c_void, bool)>;

#[repr(C)]
pub struct ghostty_runtime_config_s {
    pub userdata: *mut c_void,
    pub supports_selection_clipboard: bool,
    pub wakeup_cb: ghostty_runtime_wakeup_cb,
    pub action_cb: ghostty_runtime_action_cb,
    pub read_clipboard_cb: ghostty_runtime_read_clipboard_cb,
    pub confirm_read_clipboard_cb: ghostty_runtime_confirm_read_clipboard_cb,
    pub write_clipboard_cb: ghostty_runtime_write_clipboard_cb,
    pub close_surface_cb: ghostty_runtime_close_surface_cb,
}

// --- Extern functions ---

unsafe extern "C" {
    // Init
    pub fn ghostty_init(argc: usize, argv: *mut *mut c_char) -> i32;
    pub fn ghostty_info() -> ghostty_info_s;

    // Config
    pub fn ghostty_config_new() -> ghostty_config_t;
    pub fn ghostty_config_free(config: ghostty_config_t);
    pub fn ghostty_config_load_default_files(config: ghostty_config_t);
    pub fn ghostty_config_load_recursive_files(config: ghostty_config_t);
    pub fn ghostty_config_load_file(config: ghostty_config_t, path: *const c_char);
    pub fn ghostty_config_finalize(config: ghostty_config_t);
    pub fn ghostty_config_diagnostics_count(config: ghostty_config_t) -> u32;
    pub fn ghostty_config_get_diagnostic(
        config: ghostty_config_t,
        index: u32,
    ) -> ghostty_diagnostic_s;

    // App
    pub fn ghostty_app_new(
        runtime_config: *const ghostty_runtime_config_s,
        config: ghostty_config_t,
    ) -> ghostty_app_t;
    pub fn ghostty_app_free(app: ghostty_app_t);
    pub fn ghostty_app_tick(app: ghostty_app_t);
    pub fn ghostty_app_set_focus(app: ghostty_app_t, focused: bool);
    pub fn ghostty_app_userdata(app: ghostty_app_t) -> *mut c_void;
    pub fn ghostty_app_set_color_scheme(app: ghostty_app_t, scheme: ghostty_color_scheme_e);
    pub fn ghostty_app_update_config(app: ghostty_app_t, config: ghostty_config_t);
    pub fn ghostty_surface_update_config(surface: ghostty_surface_t, config: ghostty_config_t);

    // Surface
    pub fn ghostty_surface_config_new() -> ghostty_surface_config_s;
    pub fn ghostty_surface_new(
        app: ghostty_app_t,
        config: *const ghostty_surface_config_s,
    ) -> ghostty_surface_t;
    pub fn ghostty_surface_free(surface: ghostty_surface_t);
    pub fn ghostty_surface_draw(surface: ghostty_surface_t);
    pub fn ghostty_surface_refresh(surface: ghostty_surface_t);
    pub fn ghostty_surface_set_size(surface: ghostty_surface_t, width: u32, height: u32);
    pub fn ghostty_surface_set_focus(surface: ghostty_surface_t, focused: bool);
    pub fn ghostty_surface_set_content_scale(surface: ghostty_surface_t, x: f64, y: f64);
    pub fn ghostty_surface_size(surface: ghostty_surface_t) -> ghostty_surface_size_s;
    pub fn ghostty_surface_key_translation_mods(
        surface: ghostty_surface_t,
        mods: ghostty_input_mods_e,
    ) -> ghostty_input_mods_e;
    pub fn ghostty_surface_key(
        surface: ghostty_surface_t,
        event: ghostty_input_key_s,
    ) -> bool;
    pub fn ghostty_surface_binding_action(
        surface: ghostty_surface_t,
        action: *const c_char,
        action_len: usize,
    ) -> bool;
    pub fn ghostty_surface_ime_point(
        surface: ghostty_surface_t,
        x: *mut f64,
        y: *mut f64,
        w: *mut f64,
        h: *mut f64,
    );
    pub fn ghostty_surface_read_text(
        surface: ghostty_surface_t,
        sel: ghostty_selection_s,
        text: *mut ghostty_text_s,
    ) -> bool;
    pub fn ghostty_surface_free_text(
        surface: ghostty_surface_t,
        text: *mut ghostty_text_s,
    );
    pub fn ghostty_surface_mouse_button(
        surface: ghostty_surface_t,
        state: ghostty_input_mouse_state_e,
        button: ghostty_input_mouse_button_e,
        mods: ghostty_input_mods_e,
    ) -> bool;
    pub fn ghostty_surface_mouse_pos(
        surface: ghostty_surface_t,
        x: f64,
        y: f64,
        mods: ghostty_input_mods_e,
    );
    pub fn ghostty_surface_mouse_scroll(
        surface: ghostty_surface_t,
        x: f64,
        y: f64,
        mods: ghostty_input_scroll_mods_t,
    );
    pub fn ghostty_surface_request_close(surface: ghostty_surface_t);
    pub fn ghostty_surface_split(
        surface: ghostty_surface_t,
        direction: ghostty_action_split_direction_e,
    );
    pub fn ghostty_surface_split_focus(
        surface: ghostty_surface_t,
        direction: ghostty_action_goto_split_e,
    );
    pub fn ghostty_surface_set_color_scheme(
        surface: ghostty_surface_t,
        scheme: ghostty_color_scheme_e,
    );
    pub fn ghostty_surface_complete_clipboard_request(
        surface: ghostty_surface_t,
        data: *const c_char,
        state: *mut c_void,
        confirmed: bool,
    );
}
