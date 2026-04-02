//! Boo keybinding system with tmux-style prefix key.
//!
//! Keys flow: iced event → Bindings::handle_key → Consumed | Forward to ghostty.

use crate::config::Config;
use crate::ffi;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub enum Action {
    NewSplit(ffi::ghostty_action_split_direction_e),
    GotoSplit(ffi::ghostty_action_goto_split_e),
    ResizeSplit(Direction, u16),
    NewTab,
    NextTab,
    PrevTab,
    ReloadConfig,
}

#[derive(Debug, Clone, Copy)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

pub enum KeyResult {
    /// Boo consumed the key. Don't forward to ghostty.
    Consumed(Option<Action>),
    /// Forward the key to ghostty.
    Forward,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Mode {
    Normal,
    Prefix,
}

/// Parsed prefix key: a keycode + required modifiers.
struct PrefixKey {
    keycode: u32,
    mods_mask: i32,
}

pub struct Bindings {
    prefix: Option<PrefixKey>,
    table: HashMap<String, Action>,
    mode: Mode,
}

impl Bindings {
    pub fn from_config(config: &Config) -> Self {
        let prefix = config.prefix_key.as_deref().and_then(parse_prefix_key);
        let mut table = HashMap::new();

        for (key, action_str) in &config.keybinds {
            if let Some(action) = parse_action(action_str) {
                table.insert(key.clone(), action);
            } else {
                log::warn!("unknown boo action: {action_str}");
            }
        }

        log::info!(
            "bindings: prefix={}, {} keybinds",
            config.prefix_key.as_deref().unwrap_or("none"),
            table.len()
        );

        Bindings {
            prefix,
            table,
            mode: Mode::Normal,
        }
    }

    /// Process a key event. `key_char` is the character produced by the key
    /// (from iced's modified_key), `keycode` is the macOS virtual keycode,
    /// `mods` is the ghostty modifier bitmask.
    pub fn handle_key(&mut self, key_char: Option<char>, keycode: u32, mods: i32) -> KeyResult {
        match self.mode {
            Mode::Normal => {
                if let Some(ref prefix) = self.prefix {
                    if keycode == prefix.keycode && (mods & prefix.mods_mask) == prefix.mods_mask {
                        self.mode = Mode::Prefix;
                        return KeyResult::Consumed(None);
                    }
                }
                KeyResult::Forward
            }
            Mode::Prefix => {
                self.mode = Mode::Normal;
                if let Some(ch) = key_char {
                    let key_str = ch.to_string();
                    if let Some(action) = self.table.get(&key_str) {
                        return KeyResult::Consumed(Some(action.clone()));
                    }
                }
                // No match — forward to ghostty
                KeyResult::Forward
            }
        }
    }

    pub fn in_prefix_mode(&self) -> bool {
        self.mode == Mode::Prefix
    }
}

/// Parse "ctrl+s" into a PrefixKey with keycode and mods.
fn parse_prefix_key(spec: &str) -> Option<PrefixKey> {
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

    let keycode = single_char_to_keycode(key_part)?;
    Some(PrefixKey {
        keycode,
        mods_mask: mods,
    })
}

/// Map a single-character key name to a macOS virtual keycode.
fn single_char_to_keycode(key: &str) -> Option<u32> {
    Some(match key {
        "a" => 0x00, "s" => 0x01, "d" => 0x02, "f" => 0x03,
        "h" => 0x04, "g" => 0x05, "z" => 0x06, "x" => 0x07,
        "c" => 0x08, "v" => 0x09, "b" => 0x0B, "q" => 0x0C,
        "w" => 0x0D, "e" => 0x0E, "r" => 0x0F, "y" => 0x10,
        "t" => 0x11, "u" => 0x20, "i" => 0x22, "o" => 0x1F,
        "p" => 0x23, "l" => 0x25, "j" => 0x26, "k" => 0x28,
        "n" => 0x2D, "m" => 0x2E,
        "space" => 0x31,
        "enter" => 0x24,
        "tab" => 0x30,
        "escape" => 0x35,
        _ => return None,
    })
}

fn parse_action(s: &str) -> Option<Action> {
    use ffi::ghostty_action_goto_split_e::*;
    use ffi::ghostty_action_split_direction_e::*;

    match s {
        "reload_config" => Some(Action::ReloadConfig),
        "new_tab" => Some(Action::NewTab),
        "next_tab" => Some(Action::NextTab),
        "prev_tab" => Some(Action::PrevTab),
        "goto_split:left" => Some(Action::GotoSplit(GHOSTTY_GOTO_SPLIT_PREVIOUS)),
        "goto_split:right" | "goto_split:bottom" => {
            Some(Action::GotoSplit(GHOSTTY_GOTO_SPLIT_NEXT))
        }
        "goto_split:top" => Some(Action::GotoSplit(GHOSTTY_GOTO_SPLIT_PREVIOUS)),
        "new_split:right" => Some(Action::NewSplit(GHOSTTY_SPLIT_DIRECTION_RIGHT)),
        "new_split:down" => Some(Action::NewSplit(GHOSTTY_SPLIT_DIRECTION_DOWN)),
        "new_split:left" => Some(Action::NewSplit(GHOSTTY_SPLIT_DIRECTION_LEFT)),
        "new_split:up" => Some(Action::NewSplit(GHOSTTY_SPLIT_DIRECTION_UP)),
        _ if s.starts_with("resize_split:") => {
            let rest = &s["resize_split:".len()..];
            let (dir_str, amount_str) = rest.split_once(',')?;
            let amount: u16 = amount_str.parse().ok()?;
            let dir = match dir_str {
                "left" => Direction::Left,
                "right" => Direction::Right,
                "up" => Direction::Up,
                "down" => Direction::Down,
                _ => return None,
            };
            Some(Action::ResizeSplit(dir, amount))
        }
        _ => None,
    }
}
