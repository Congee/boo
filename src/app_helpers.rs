use crate::{bindings, config, control, ffi, mouse, platform, splits, vt};
use iced::keyboard;

pub(crate) enum TextInputAction {
    Commit(String),
    Command(platform::TextInputCommand),
}

pub(crate) fn shifted_codepoint(keycode: u32, mods: i32) -> u32 {
    let has_shift = mods & ffi::GHOSTTY_MODS_SHIFT != 0;
    #[cfg(target_os = "macos")]
    let base = match keycode {
        0x00 => 'a',
        0x01 => 's',
        0x02 => 'd',
        0x03 => 'f',
        0x04 => 'h',
        0x05 => 'g',
        0x06 => 'z',
        0x07 => 'x',
        0x08 => 'c',
        0x09 => 'v',
        0x0B => 'b',
        0x0C => 'q',
        0x0D => 'w',
        0x0E => 'e',
        0x0F => 'r',
        0x10 => 'y',
        0x11 => 't',
        0x20 => 'u',
        0x22 => 'i',
        0x1F => 'o',
        0x23 => 'p',
        0x25 => 'l',
        0x26 => 'j',
        0x28 => 'k',
        0x2D => 'n',
        0x2E => 'm',
        0x31 => ' ',
        0x24 => '\r',
        0x30 => '\t',
        0x12 => if has_shift { '!' } else { '1' },
        0x13 => if has_shift { '@' } else { '2' },
        0x14 => if has_shift { '#' } else { '3' },
        0x15 => if has_shift { '$' } else { '4' },
        0x17 => if has_shift { '%' } else { '5' },
        0x16 => if has_shift { '^' } else { '6' },
        0x1A => if has_shift { '&' } else { '7' },
        0x1C => if has_shift { '*' } else { '8' },
        0x19 => if has_shift { '(' } else { '9' },
        0x1D => if has_shift { ')' } else { '0' },
        0x27 => if has_shift { '"' } else { '\'' },
        0x2A => if has_shift { '|' } else { '\\' },
        0x2B => if has_shift { '<' } else { ',' },
        0x2F => if has_shift { '>' } else { '.' },
        0x2C => if has_shift { '?' } else { '/' },
        0x29 => if has_shift { ':' } else { ';' },
        0x1B => if has_shift { '_' } else { '-' },
        0x18 => if has_shift { '+' } else { '=' },
        0x21 => if has_shift { '{' } else { '[' },
        0x1E => if has_shift { '}' } else { ']' },
        0x32 => if has_shift { '~' } else { '`' },
        _ => return 0,
    };
    #[cfg(target_os = "linux")]
    return shifted_codepoint_vt(keycode, mods);
    if has_shift && base.is_ascii_lowercase() {
        base.to_ascii_uppercase() as u32
    } else {
        base as u32
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn shifted_codepoint_vt(keycode: u32, mods: i32) -> u32 {
    let has_shift = mods & ffi::GHOSTTY_MODS_SHIFT != 0;
    let base = match keycode {
        20 => 'a',
        21 => 'b',
        22 => 'c',
        23 => 'd',
        24 => 'e',
        25 => 'f',
        26 => 'g',
        27 => 'h',
        28 => 'i',
        29 => 'j',
        30 => 'k',
        31 => 'l',
        32 => 'm',
        33 => 'n',
        34 => 'o',
        35 => 'p',
        36 => 'q',
        37 => 'r',
        38 => 's',
        39 => 't',
        40 => 'u',
        41 => 'v',
        42 => 'w',
        43 => 'x',
        44 => 'y',
        45 => 'z',
        63 => ' ',
        58 => '\r',
        64 => '\t',
        7 => if has_shift { '!' } else { '1' },
        8 => if has_shift { '@' } else { '2' },
        9 => if has_shift { '#' } else { '3' },
        10 => if has_shift { '$' } else { '4' },
        11 => if has_shift { '%' } else { '5' },
        12 => if has_shift { '^' } else { '6' },
        13 => if has_shift { '&' } else { '7' },
        14 => if has_shift { '*' } else { '8' },
        15 => if has_shift { '(' } else { '9' },
        6 => if has_shift { ')' } else { '0' },
        48 => if has_shift { '"' } else { '\'' },
        2 => if has_shift { '|' } else { '\\' },
        5 => if has_shift { '<' } else { ',' },
        47 => if has_shift { '>' } else { '.' },
        50 => if has_shift { '?' } else { '/' },
        49 => if has_shift { ':' } else { ';' },
        46 => if has_shift { '_' } else { '-' },
        16 => if has_shift { '+' } else { '=' },
        3 => if has_shift { '{' } else { '[' },
        4 => if has_shift { '}' } else { ']' },
        1 => if has_shift { '~' } else { '`' },
        _ => return 0,
    };
    if has_shift && base.is_ascii_lowercase() {
        base.to_ascii_uppercase() as u32
    } else {
        base as u32
    }
}

pub(crate) fn shifted_char(keycode: u32, mods: i32) -> Option<char> {
    match shifted_codepoint(keycode, mods) {
        0 => None,
        cp => char::from_u32(cp),
    }
}

pub(crate) fn parse_keyspec(spec: &str) -> Option<(u32, i32)> {
    let mut mods: i32 = 0;
    let mut key_part = spec;
    loop {
        if let Some(rest) = key_part.strip_prefix("ctrl+") {
            mods |= ffi::GHOSTTY_MODS_CTRL;
            key_part = rest;
        } else if let Some(rest) = key_part.strip_prefix("shift+") {
            mods |= ffi::GHOSTTY_MODS_SHIFT;
            key_part = rest;
        } else if let Some(rest) = key_part.strip_prefix("alt+") {
            mods |= ffi::GHOSTTY_MODS_ALT;
            key_part = rest;
        } else if let Some(rest) = key_part.strip_prefix("super+") {
            mods |= ffi::GHOSTTY_MODS_SUPER;
            key_part = rest;
        } else {
            break;
        }
    }
    #[cfg(target_os = "macos")]
    let keycode = match key_part {
        "a" => 0x00, "s" => 0x01, "d" => 0x02, "f" => 0x03, "h" => 0x04, "g" => 0x05,
        "z" => 0x06, "x" => 0x07, "c" => 0x08, "v" => 0x09, "b" => 0x0B, "q" => 0x0C,
        "w" => 0x0D, "e" => 0x0E, "r" => 0x0F, "y" => 0x10, "t" => 0x11, "u" => 0x20,
        "i" => 0x22, "o" => 0x1F, "p" => 0x23, "l" => 0x25, "j" => 0x26, "k" => 0x28,
        "n" => 0x2D, "m" => 0x2E, "enter" | "return" => 0x24, "tab" => 0x30,
        "space" => 0x31, "escape" | "esc" => 0x35, "backspace" => 0x33, "delete" => 0x75,
        "up" => 0x7E, "down" => 0x7D, "left" => 0x7B, "right" => 0x7C, "pageup" => 0x74,
        "pagedown" => 0x79, "home" => 0x73, "end" => 0x77,
        _ if key_part.starts_with("0x") => u32::from_str_radix(&key_part[2..], 16).ok()?,
        _ => return None,
    };
    #[cfg(target_os = "linux")]
    let keycode = match key_part {
        "a" => 20, "b" => 21, "c" => 22, "d" => 23, "e" => 24, "f" => 25, "g" => 26, "h" => 27,
        "i" => 28, "j" => 29, "k" => 30, "l" => 31, "m" => 32, "n" => 33, "o" => 34, "p" => 35,
        "q" => 36, "r" => 37, "s" => 38, "t" => 39, "u" => 40, "v" => 41, "w" => 42, "x" => 43,
        "y" => 44, "z" => 45, "0" => 6, "1" => 7, "2" => 8, "3" => 9, "4" => 10, "5" => 11,
        "6" => 12, "7" => 13, "8" => 14, "9" => 15, "enter" | "return" => 58, "tab" => 64,
        "space" => 63, "escape" | "esc" => 120, "backspace" => 53, "delete" => 119,
        "up" => 111, "down" => 116, "left" => 113, "right" => 114, "pageup" => 112,
        "pagedown" => 117, "home" => 110, "end" => 115,
        _ if key_part.starts_with("0x") => u32::from_str_radix(&key_part[2..], 16).ok()?,
        _ => return None,
    };
    Some((keycode, mods))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn parse_vt_keyspec(spec: &str) -> Option<(u32, i32)> {
    let mut mods: i32 = 0;
    let mut key_part = spec;
    loop {
        if let Some(rest) = key_part.strip_prefix("ctrl+") {
            mods |= ffi::GHOSTTY_MODS_CTRL;
            key_part = rest;
        } else if let Some(rest) = key_part.strip_prefix("shift+") {
            mods |= ffi::GHOSTTY_MODS_SHIFT;
            key_part = rest;
        } else if let Some(rest) = key_part.strip_prefix("alt+") {
            mods |= ffi::GHOSTTY_MODS_ALT;
            key_part = rest;
        } else if let Some(rest) = key_part.strip_prefix("super+") {
            mods |= ffi::GHOSTTY_MODS_SUPER;
            key_part = rest;
        } else {
            break;
        }
    }
    let keycode = match key_part {
        "a" => 20, "b" => 21, "c" => 22, "d" => 23, "e" => 24, "f" => 25, "g" => 26, "h" => 27,
        "i" => 28, "j" => 29, "k" => 30, "l" => 31, "m" => 32, "n" => 33, "o" => 34, "p" => 35,
        "q" => 36, "r" => 37, "s" => 38, "t" => 39, "u" => 40, "v" => 41, "w" => 42, "x" => 43,
        "y" => 44, "z" => 45, "0" => 6, "1" => 7, "2" => 8, "3" => 9, "4" => 10, "5" => 11,
        "6" => 12, "7" => 13, "8" => 14, "9" => 15, "enter" | "return" => 58, "tab" => 64,
        "space" => 63, "escape" | "esc" => 120, "backspace" => 53, "delete" => 68,
        "up" => 73, "down" => 74, "left" => 71, "right" => 72, "pageup" => 75, "pagedown" => 78,
        "home" => 69, "end" => 70,
        _ if key_part.starts_with("0x") => u32::from_str_radix(&key_part[2..], 16).ok()?,
        _ => return None,
    };
    Some((keycode, mods))
}

pub(crate) fn control_key_to_keyboard_key(spec: &str, key_char: Option<char>) -> keyboard::Key {
    use keyboard::key::Named;
    let key_name = spec.rsplit_once('+').map(|(_, key)| key).unwrap_or(spec);
    match key_name {
        "enter" | "return" => keyboard::Key::Named(Named::Enter),
        "tab" => keyboard::Key::Named(Named::Tab),
        "space" => keyboard::Key::Named(Named::Space),
        "escape" | "esc" => keyboard::Key::Named(Named::Escape),
        "backspace" => keyboard::Key::Named(Named::Backspace),
        "delete" => keyboard::Key::Named(Named::Delete),
        "up" => keyboard::Key::Named(Named::ArrowUp),
        "down" => keyboard::Key::Named(Named::ArrowDown),
        "left" => keyboard::Key::Named(Named::ArrowLeft),
        "right" => keyboard::Key::Named(Named::ArrowRight),
        "pageup" => keyboard::Key::Named(Named::PageUp),
        "pagedown" => keyboard::Key::Named(Named::PageDown),
        "home" => keyboard::Key::Named(Named::Home),
        "end" => keyboard::Key::Named(Named::End),
        _ => keyboard::Key::Character(key_char.unwrap_or_default().to_string().into()),
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn should_route_macos_vt_key_via_appkit(vt_keycode: u32, mods: i32) -> bool {
    if mods & (ffi::GHOSTTY_MODS_CTRL | ffi::GHOSTTY_MODS_SUPER) != 0 {
        return false;
    }
    matches!(vt_keycode, 1..=16 | 20..=50 | 63)
}

#[cfg(target_os = "macos")]
pub(crate) fn native_keycode_to_named_key(keycode: u32) -> Option<bindings::NamedKey> {
    Some(match keycode {
        0x7E => bindings::NamedKey::ArrowUp,
        0x7D => bindings::NamedKey::ArrowDown,
        0x7B => bindings::NamedKey::ArrowLeft,
        0x7C => bindings::NamedKey::ArrowRight,
        0x74 => bindings::NamedKey::PageUp,
        0x79 => bindings::NamedKey::PageDown,
        0x73 => bindings::NamedKey::Home,
        0x77 => bindings::NamedKey::End,
        0x35 => bindings::NamedKey::Escape,
        _ => return None,
    })
}

#[cfg(target_os = "macos")]
pub(crate) fn native_keycode_to_keyboard_key(keycode: u32, key_char: Option<char>) -> keyboard::Key {
    use keyboard::key::Named;
    match keycode {
        0x24 => keyboard::Key::Named(Named::Enter),
        0x30 => keyboard::Key::Named(Named::Tab),
        0x31 => keyboard::Key::Named(Named::Space),
        0x33 => keyboard::Key::Named(Named::Backspace),
        0x35 => keyboard::Key::Named(Named::Escape),
        0x75 => keyboard::Key::Named(Named::Delete),
        0x7E => keyboard::Key::Named(Named::ArrowUp),
        0x7D => keyboard::Key::Named(Named::ArrowDown),
        0x7B => keyboard::Key::Named(Named::ArrowLeft),
        0x7C => keyboard::Key::Named(Named::ArrowRight),
        0x74 => keyboard::Key::Named(Named::PageUp),
        0x79 => keyboard::Key::Named(Named::PageDown),
        0x73 => keyboard::Key::Named(Named::Home),
        0x77 => keyboard::Key::Named(Named::End),
        _ => keyboard::Key::Character(key_char.unwrap_or_default().to_string().into()),
    }
}

pub(crate) fn ghostty_mods_to_iced(mods: i32) -> keyboard::Modifiers {
    use iced::keyboard::Modifiers;
    let mut result = Modifiers::empty();
    if mods & ffi::GHOSTTY_MODS_SHIFT != 0 {
        result.insert(Modifiers::SHIFT);
    }
    if mods & ffi::GHOSTTY_MODS_CTRL != 0 {
        result.insert(Modifiers::CTRL);
    }
    if mods & ffi::GHOSTTY_MODS_ALT != 0 {
        result.insert(Modifiers::ALT);
    }
    if mods & ffi::GHOSTTY_MODS_SUPER != 0 {
        result.insert(Modifiers::LOGO);
    }
    result
}

pub(crate) fn key_to_codepoint(key: &keyboard::Key) -> u32 {
    match key {
        keyboard::Key::Character(s) => s.chars().next().map(|c| c as u32).unwrap_or(0),
        keyboard::Key::Named(named) => {
            use keyboard::key::Named;
            match named {
                Named::Enter => '\r' as u32,
                Named::Tab => '\t' as u32,
                Named::Space => ' ' as u32,
                Named::Backspace => 0x08,
                Named::Escape => 0x1b,
                Named::Delete => 0x7f,
                _ => 0,
            }
        }
        _ => 0,
    }
}

pub(crate) fn iced_mods_to_ghostty(mods: &keyboard::Modifiers) -> ffi::ghostty_input_mods_e {
    let mut result = ffi::GHOSTTY_MODS_NONE;
    if mods.shift() {
        result |= ffi::GHOSTTY_MODS_SHIFT;
    }
    if mods.control() {
        result |= ffi::GHOSTTY_MODS_CTRL;
    }
    if mods.alt() {
        result |= ffi::GHOSTTY_MODS_ALT;
    }
    if mods.logo() {
        result |= ffi::GHOSTTY_MODS_SUPER;
    }
    result
}

pub(crate) fn iced_button_to_ghostty(button: mouse::Button) -> ffi::ghostty_input_mouse_button_e {
    match button {
        mouse::Button::Left => ffi::ghostty_input_mouse_button_e::GHOSTTY_MOUSE_LEFT,
        mouse::Button::Right => ffi::ghostty_input_mouse_button_e::GHOSTTY_MOUSE_RIGHT,
        mouse::Button::Middle => ffi::ghostty_input_mouse_button_e::GHOSTTY_MOUSE_MIDDLE,
        _ => ffi::ghostty_input_mouse_button_e::GHOSTTY_MOUSE_UNKNOWN,
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn iced_button_to_vt(button: mouse::Button) -> vt::GhosttyMouseButton {
    match button {
        mouse::Button::Left => vt::GHOSTTY_MOUSE_BUTTON_LEFT,
        mouse::Button::Right => vt::GHOSTTY_MOUSE_BUTTON_RIGHT,
        mouse::Button::Middle => vt::GHOSTTY_MOUSE_BUTTON_MIDDLE,
        _ => vt::GHOSTTY_MOUSE_BUTTON_UNKNOWN,
    }
}

pub(crate) fn split_direction_name(direction: splits::Direction) -> &'static str {
    match direction {
        splits::Direction::Horizontal => "horizontal",
        splits::Direction::Vertical => "vertical",
    }
}

pub(crate) fn ui_rect_snapshot(x: f64, y: f64, width: f64, height: f64) -> control::UiRectSnapshot {
    control::UiRectSnapshot { x, y, width, height }
}

pub(crate) fn format_command_finished_message(exit_code: Option<u8>, duration_ns: u64) -> String {
    let duration_ms = duration_ns / 1_000_000;
    if let Some(exit_code) = exit_code {
        format!(
            "Command took {}.{:03}s and exited with code {}.",
            duration_ms / 1000,
            duration_ms % 1000,
            exit_code
        )
    } else {
        format!(
            "Command took {}.{:03}s.",
            duration_ms / 1000,
            duration_ms % 1000
        )
    }
}

pub(crate) fn command_finish_notification(
    desktop_notifications_enabled: bool,
    notify_action_enabled: bool,
    notify_on_command_finish: config::NotifyOnCommandFinish,
    app_focused: bool,
    notify_on_command_finish_after_ns: u64,
    event: crate::CommandFinishedEvent,
) -> Option<(&'static str, String)> {
    if !desktop_notifications_enabled || !notify_action_enabled {
        return None;
    }
    match notify_on_command_finish {
        config::NotifyOnCommandFinish::Never => return None,
        config::NotifyOnCommandFinish::Unfocused if app_focused => return None,
        config::NotifyOnCommandFinish::Unfocused | config::NotifyOnCommandFinish::Always => {}
    }
    if event.duration_ns <= notify_on_command_finish_after_ns {
        return None;
    }

    let title = match event.exit_code {
        Some(0) => "Command Succeeded",
        Some(_) => "Command Failed",
        None => "Command Finished",
    };
    Some((title, format_command_finished_message(event.exit_code, event.duration_ns)))
}

pub(crate) fn apply_text_input_event(
    preedit_text: &mut String,
    event: platform::TextInputEvent,
) -> Option<TextInputAction> {
    match event {
        platform::TextInputEvent::Commit(text) => {
            preedit_text.clear();
            Some(TextInputAction::Commit(text))
        }
        platform::TextInputEvent::Preedit(text) => {
            *preedit_text = text;
            None
        }
        platform::TextInputEvent::PreeditClear => {
            preedit_text.clear();
            None
        }
        platform::TextInputEvent::Command(command) => Some(TextInputAction::Command(command)),
    }
}

pub(crate) fn text_input_command_key(command: platform::TextInputCommand) -> (u32, u32) {
    match command {
        platform::TextInputCommand::Backspace => (53, 0x08),
        platform::TextInputCommand::DeleteForward => (68, 0x7f),
        platform::TextInputCommand::InsertNewline => (58, '\r' as u32),
        platform::TextInputCommand::InsertTab => (64, '\t' as u32),
    }
}
