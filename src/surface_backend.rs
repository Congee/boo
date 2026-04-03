#![cfg_attr(target_os = "linux", allow(dead_code))]

use crate::ffi;
#[cfg(not(target_os = "linux"))]
use crate::pane::PaneHandle;
#[cfg(not(target_os = "linux"))]
use crate::platform;
#[cfg(not(target_os = "linux"))]
use std::ffi::{c_void, CStr, CString};

#[cfg(not(target_os = "linux"))]
pub fn set_focus(surface: ffi::ghostty_surface_t, focused: bool) {
    if !surface.is_null() {
        unsafe { ffi::ghostty_surface_set_focus(surface, focused) };
    }
}

#[cfg(target_os = "linux")]
pub fn set_focus(_surface: ffi::ghostty_surface_t, _focused: bool) {}

#[cfg(not(target_os = "linux"))]
pub fn resize(surface: ffi::ghostty_surface_t, scale: f64, width: u32, height: u32) {
    if !surface.is_null() {
        unsafe {
            ffi::ghostty_surface_set_content_scale(surface, scale, scale);
            ffi::ghostty_surface_set_size(surface, width, height);
        }
    }
}

#[cfg(target_os = "linux")]
pub fn resize(_surface: ffi::ghostty_surface_t, _scale: f64, _width: u32, _height: u32) {}

#[cfg(not(target_os = "linux"))]
pub fn free(surface: ffi::ghostty_surface_t) {
    if !surface.is_null() {
        unsafe { ffi::ghostty_surface_free(surface) };
    }
}

#[cfg(target_os = "linux")]
pub fn free(_surface: ffi::ghostty_surface_t) {}

#[cfg(not(target_os = "linux"))]
pub fn key_translation_mods(surface: ffi::ghostty_surface_t, mods: i32) -> i32 {
    if surface.is_null() {
        mods
    } else {
        unsafe { ffi::ghostty_surface_key_translation_mods(surface, mods) }
    }
}

#[cfg(target_os = "linux")]
pub fn key_translation_mods(_surface: ffi::ghostty_surface_t, mods: i32) -> i32 {
    mods
}

#[cfg(not(target_os = "linux"))]
pub fn key(surface: ffi::ghostty_surface_t, event: ffi::ghostty_input_key_s) -> bool {
    if surface.is_null() {
        false
    } else {
        unsafe { ffi::ghostty_surface_key(surface, event) }
    }
}

#[cfg(target_os = "linux")]
pub fn key(_surface: ffi::ghostty_surface_t, _event: ffi::ghostty_input_key_s) -> bool {
    false
}

#[cfg(not(target_os = "linux"))]
pub fn mouse_pos(surface: ffi::ghostty_surface_t, x: f64, y: f64, mods: i32) {
    if !surface.is_null() {
        unsafe { ffi::ghostty_surface_mouse_pos(surface, x, y, mods) };
    }
}

#[cfg(target_os = "linux")]
pub fn mouse_pos(_surface: ffi::ghostty_surface_t, _x: f64, _y: f64, _mods: i32) {}

#[cfg(not(target_os = "linux"))]
pub fn mouse_button(
    surface: ffi::ghostty_surface_t,
    state: ffi::ghostty_input_mouse_state_e,
    button: ffi::ghostty_input_mouse_button_e,
    mods: i32,
) {
    if !surface.is_null() {
        unsafe { ffi::ghostty_surface_mouse_button(surface, state, button, mods) };
    }
}

#[cfg(target_os = "linux")]
pub fn mouse_button(
    _surface: ffi::ghostty_surface_t,
    _state: ffi::ghostty_input_mouse_state_e,
    _button: ffi::ghostty_input_mouse_button_e,
    _mods: i32,
) {
}

#[cfg(not(target_os = "linux"))]
pub fn mouse_scroll(surface: ffi::ghostty_surface_t, dx: f64, dy: f64, mods: i32) {
    if !surface.is_null() {
        unsafe { ffi::ghostty_surface_mouse_scroll(surface, dx, dy, mods) };
    }
}

#[cfg(target_os = "linux")]
pub fn mouse_scroll(_surface: ffi::ghostty_surface_t, _dx: f64, _dy: f64, _mods: i32) {}

#[cfg(not(target_os = "linux"))]
pub fn update_config(surface: ffi::ghostty_surface_t, config: ffi::ghostty_config_t) {
    if !surface.is_null() {
        unsafe { ffi::ghostty_surface_update_config(surface, config) };
    }
}

#[cfg(target_os = "linux")]
pub fn update_config(_surface: ffi::ghostty_surface_t, _config: ffi::ghostty_config_t) {}

#[cfg(not(target_os = "linux"))]
pub fn complete_clipboard_request(
    surface: ffi::ghostty_surface_t,
    content: *const std::os::raw::c_char,
    state: *mut std::ffi::c_void,
    confirmed: bool,
) -> bool {
    if surface.is_null() {
        false
    } else {
        unsafe {
            ffi::ghostty_surface_complete_clipboard_request(surface, content, state, confirmed);
        }
        true
    }
}

#[cfg(target_os = "linux")]
pub fn complete_clipboard_request(
    _surface: ffi::ghostty_surface_t,
    _content: *const std::os::raw::c_char,
    _state: *mut std::ffi::c_void,
    _confirmed: bool,
) -> bool {
    false
}

#[cfg(not(target_os = "linux"))]
pub fn ime_point(surface: ffi::ghostty_surface_t) -> Option<(f64, f64, f64, f64)> {
    if surface.is_null() {
        None
    } else {
        let mut x = 0.0;
        let mut y = 0.0;
        let mut w = 0.0;
        let mut h = 0.0;
        unsafe { ffi::ghostty_surface_ime_point(surface, &mut x, &mut y, &mut w, &mut h) };
        Some((x, y, w, h))
    }
}

#[cfg(target_os = "linux")]
pub fn ime_point(_surface: ffi::ghostty_surface_t) -> Option<(f64, f64, f64, f64)> {
    None
}

#[cfg(not(target_os = "linux"))]
pub fn read_text(
    surface: ffi::ghostty_surface_t,
    selection: ffi::ghostty_selection_s,
) -> Option<String> {
    if surface.is_null() {
        return None;
    }

    let mut text = ffi::ghostty_text_s {
        tl_px_x: 0.0,
        tl_px_y: 0.0,
        offset_start: 0,
        offset_len: 0,
        text: std::ptr::null(),
        text_len: 0,
    };
    let ok = unsafe { ffi::ghostty_surface_read_text(surface, selection, &mut text) };
    if !ok || text.text.is_null() || text.text_len == 0 {
        return None;
    }

    let slice = unsafe { std::slice::from_raw_parts(text.text as *const u8, text.text_len) };
    let result = std::str::from_utf8(slice).ok().map(|s| s.to_string());
    unsafe { ffi::ghostty_surface_free_text(surface, &mut text) };
    result
}

#[cfg(target_os = "linux")]
pub fn read_text(
    _surface: ffi::ghostty_surface_t,
    _selection: ffi::ghostty_selection_s,
) -> Option<String> {
    None
}

#[cfg(not(target_os = "linux"))]
pub fn binding_action(surface: ffi::ghostty_surface_t, action: &str) {
    if !surface.is_null() {
        unsafe {
            ffi::ghostty_surface_binding_action(
                surface,
                action.as_ptr() as *const _,
                action.len(),
            );
        }
    }
}

#[cfg(target_os = "linux")]
pub fn binding_action(_surface: ffi::ghostty_surface_t, _action: &str) {}

#[cfg(not(target_os = "linux"))]
pub fn create_pane(
    app: ffi::ghostty_app_t,
    cb_userdata: *mut c_void,
    parent_view: *mut c_void,
    scale: f64,
    frame: platform::Rect,
    context: ffi::ghostty_surface_context_e,
    command: Option<&CStr>,
    working_directory: Option<&CStr>,
) -> Option<PaneHandle> {
    let mut config = ffi::ghostty_surface_config_s {
        platform_tag: platform::platform_tag(),
        platform: unsafe { std::mem::zeroed() },
        userdata: cb_userdata,
        scale_factor: scale,
        font_size: 0.0,
        working_directory: std::ptr::null(),
        command: std::ptr::null(),
        env_vars: std::ptr::null_mut(),
        env_var_count: 0,
        initial_input: std::ptr::null(),
        wait_after_command: false,
        context,
    };
    let cwd_storage: Option<CString> = std::env::var_os("HOME")
        .and_then(|home| CString::new(home.to_string_lossy().as_bytes()).ok());
    if let Some(cmd) = command {
        config.command = cmd.as_ptr();
    }
    if let Some(wd) = working_directory {
        config.working_directory = wd.as_ptr();
    } else if let Some(cwd) = cwd_storage.as_ref() {
        config.working_directory = cwd.as_ptr();
    }

    #[cfg(target_os = "macos")]
    let child_view = {
        if parent_view.is_null() {
            log::warn!("create_pane: parent view is null");
            return None;
        }
        let cv = platform::create_child_view(parent_view, frame);
        config.platform = platform::platform_config(cv);
        cv
    };

    let surface = unsafe { ffi::ghostty_surface_new(app, &config) };
    if surface.is_null() {
        log::error!("failed to create ghostty surface");
        return None;
    }

    #[cfg(target_os = "macos")]
    platform::set_view_layer_transparent(child_view);

    Some(PaneHandle::new(surface, child_view))
}
