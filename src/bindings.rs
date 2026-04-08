//! Boo keybinding system with tmux-style prefix key.
//!
//! Keys flow: iced event → Bindings::handle_key → Consumed | CopyMode | Forward to ghostty.
//!
//! Modes: Normal → Prefix → (action) | Normal → CopyMode → Normal

use crate::config::Config;
use crate::ffi;
use std::collections::HashMap;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum Action {
    NewSplit(SplitDirection),
    GotoSplit(PaneFocusDirection),
    ResizeSplit(Direction, u16),
    CloseSurface,
    BreakPane,
    NewTab,
    NextTab,
    PrevTab,
    CloseTab,
    GotoTab(TabTarget),
    Search,
    EnterCopyMode,
    Copy,
    Paste,
    ChooseBuffer,
    ChooseTree,
    FindWindow,
    SetTabTitle,
    DisplayPanes,
    MarkPane,
    ClearMarkedPane,
    JoinMarkedPane(SplitDirection),
    RotatePanesForward,
    RotatePanesBackward,
    SwapPaneNext,
    SwapPanePrevious,
    SelectLayout(crate::session::TabLayout),
    NextLayout,
    PreviousLayout,
    RebalanceLayout,
    ToggleZoom,
    OpenCommandPrompt,
    NextPane,
    PreviousPane,
    PreviousTab,
    ReloadConfig,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum TabTarget {
    Index(usize),
    Last,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SplitDirection {
    Right,
    Down,
    Left,
    Up,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PaneFocusDirection {
    Left,
    Right,
    Up,
    Down,
}

/// Actions emitted while in copy mode (tmux vi copy-mode compatible).
#[derive(Debug, Clone)]
pub enum CopyModeAction {
    // Cursor
    Move(Direction),
    // Word
    WordNext,
    WordBack,
    WordEnd,
    BigWordNext,
    BigWordBack,
    BigWordEnd,
    // Line position
    LineStart,
    LineEnd,
    FirstNonBlank,
    // Screen position
    ScreenTop,
    ScreenMiddle,
    ScreenBottom,
    // Scrollback
    HistoryTop,
    HistoryBottom,
    // Page/scroll
    PageUp,
    PageDown,
    HalfPageUp,
    HalfPageDown,
    ScrollUp,
    ScrollDown,
    ScrollMiddle,
    // Selection
    StartCharSelect,
    StartLineSelect,
    StartRectSelect,
    ClearSelection,
    SwapAnchor,
    // In-line jump
    JumpForward,
    JumpBackward,
    JumpToForward,
    JumpToBackward,
    JumpAgain,
    JumpReverse,
    // Paragraph/bracket
    NextParagraph,
    PreviousParagraph,
    MatchingBracket,
    // Marks
    SetMark,
    JumpToMark,
    // Search
    SearchForward,
    SearchBackward,
    SearchAgain,
    SearchReverse,
    SearchWordForward,
    SearchWordBackward,
    // Copy
    CopyAndExit,
    CopyToEndOfLine,
    AppendAndCancel,
    // Other
    OpenPrompt,
    RefreshFromPane,
    TogglePosition,
    Exit,
}

pub enum KeyResult {
    /// Boo consumed the key. Don't forward to ghostty.
    Consumed(Option<Action>),
    /// Copy mode action.
    CopyMode(CopyModeAction),
    /// Forward the key to ghostty.
    Forward,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Mode {
    Normal,
    Prefix,
    CopyMode,
}

/// Parsed prefix key: a keycode + required modifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PrefixKey {
    keycode: u32,
    mods_mask: i32,
}

pub struct Bindings {
    prefix: Option<PrefixKey>,
    /// Prefix-mode keybinds: matched by produced character after prefix key.
    table: HashMap<String, Action>,
    /// Direct keybinds: matched by (keycode, mods) in normal mode, no prefix needed.
    direct: HashMap<(u32, i32), Action>,
    mode: Mode,
}

impl Bindings {
    pub fn from_config(config: &Config) -> Self {
        let mut prefix = config.prefix_key.as_deref().and_then(parse_prefix_key);
        let mut table = HashMap::new();
        let mut direct = HashMap::new();

        for (key, action_str) in &config.keybinds {
            let Some(action) = parse_action(action_str) else {
                log::warn!("unknown boo action: {action_str}");
                continue;
            };
            if let Some((prefix_spec, trigger_spec)) = key.split_once('>') {
                let Some(parsed_prefix) = parse_prefix_key(prefix_spec.trim()) else {
                    log::warn!("unparseable prefixed keybind prefix: {key}");
                    continue;
                };
                match prefix {
                    Some(existing) if existing != parsed_prefix => {
                        log::warn!(
                            "ignoring prefixed keybind with different prefix: {key} (boo supports one prefix key)"
                        );
                        continue;
                    }
                    None => prefix = Some(parsed_prefix),
                    _ => {}
                }
                if let Some(trigger) = parse_prefix_trigger(trigger_spec.trim()) {
                    table.insert(trigger, action);
                } else {
                    log::warn!("unparseable prefixed keybind trigger: {key}");
                }
            } else if key.contains('+') {
                if let Some(parsed) = parse_prefix_key(key) {
                    direct.insert((parsed.keycode, parsed.mods_mask), action);
                } else {
                    log::warn!("unparseable direct keybind: {key}");
                }
            } else {
                if let Some(trigger) = parse_prefix_trigger(key) {
                    table.insert(trigger, action);
                } else {
                    log::warn!("unparseable prefix keybind trigger: {key}");
                }
            }
        }

        log::info!(
            "bindings: prefix={}, {} prefix keybinds, {} direct keybinds",
            config.prefix_key.as_deref().unwrap_or("none"),
            table.len(),
            direct.len(),
        );

        Bindings {
            prefix,
            table,
            direct,
            mode: Mode::Normal,
        }
    }

    pub fn is_prefix_mode(&self) -> bool {
        self.mode == Mode::Prefix
    }

    pub fn is_copy_mode(&self) -> bool {
        self.mode == Mode::CopyMode
    }

    pub fn enter_copy_mode(&mut self) {
        self.mode = Mode::CopyMode;
    }

    pub fn exit_copy_mode(&mut self) {
        self.mode = Mode::Normal;
    }

    /// Process a key event. `key_char` is the character produced by the key,
    /// `keycode` is the macOS virtual keycode, `mods` is the ghostty modifier bitmask.
    /// `named_key` is the iced Named key (for arrows, page up/down, etc.).
    pub fn handle_key(
        &mut self,
        key_char: Option<char>,
        keycode: u32,
        mods: i32,
        named_key: Option<NamedKey>,
    ) -> KeyResult {
        match self.mode {
            Mode::Normal => {
                if let Some(action) = self.direct.get(&(keycode, mods)) {
                    return KeyResult::Consumed(Some(action.clone()));
                }
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
                        log::info!("prefix match: {key_str:?} → {action:?}");
                        return KeyResult::Consumed(Some(action.clone()));
                    }
                    log::debug!("prefix no match for {key_str:?}");
                }
                KeyResult::Forward
            }
            Mode::CopyMode => self.handle_copy_mode_key(key_char, named_key, mods),
        }
    }

    fn handle_copy_mode_key(
        &mut self,
        key_char: Option<char>,
        named_key: Option<NamedKey>,
        mods: i32,
    ) -> KeyResult {
        use CopyModeAction::*;
        let has_ctrl = mods & ffi::GHOSTTY_MODS_CTRL != 0;
        let has_alt = mods & ffi::GHOSTTY_MODS_ALT != 0;

        // Tier 1: Named keys (arrows, pgup/pgdn, home/end, escape)
        if let Some(named) = named_key {
            let action = match named {
                NamedKey::ArrowUp => Some(Move(Direction::Up)),
                NamedKey::ArrowDown => Some(Move(Direction::Down)),
                NamedKey::ArrowLeft => Some(Move(Direction::Left)),
                NamedKey::ArrowRight => Some(Move(Direction::Right)),
                NamedKey::PageUp => Some(PageUp),
                NamedKey::PageDown => Some(PageDown),
                NamedKey::Home => Some(HistoryTop),
                NamedKey::End => Some(HistoryBottom),
                NamedKey::Escape => Some(ClearSelection),
            };
            if let Some(a) = action {
                return KeyResult::CopyMode(a);
            }
        }

        // Tier 2: Ctrl-modified keys
        if has_ctrl {
            if let Some(ch) = key_char {
                let action = match ch {
                    'b' | '\x02' => Some(PageUp),
                    'f' | '\x06' => Some(PageDown),
                    'u' | '\x15' => Some(HalfPageUp),
                    'd' | '\x04' => Some(HalfPageDown),
                    'y' | '\x19' => Some(ScrollUp),
                    'e' | '\x05' => Some(ScrollDown),
                    'v' | '\x16' => Some(StartRectSelect),
                    _ => None,
                };
                if let Some(a) = action {
                    return KeyResult::CopyMode(a);
                }
            }
        }

        // Tier 2b: Alt-modified keys
        if has_alt {
            if let Some(ch) = key_char {
                let action = match ch {
                    'x' => Some(JumpToMark),
                    _ => None,
                };
                if let Some(a) = action {
                    return KeyResult::CopyMode(a);
                }
            }
        }

        // Tier 3: Plain character keys
        if let Some(ch) = key_char {
            let action = match ch {
                // Cursor
                'h' | '\x08' => Some(Move(Direction::Left)), // h, Backspace
                'j' => Some(Move(Direction::Down)),
                'k' => Some(Move(Direction::Up)),
                'l' => Some(Move(Direction::Right)),
                // Word movement
                'w' => Some(WordNext),
                'b' => Some(WordBack),
                'e' => Some(WordEnd),
                'W' => Some(BigWordNext),
                'B' => Some(BigWordBack),
                'E' => Some(BigWordEnd),
                // Line position
                '0' => Some(LineStart),
                '$' => Some(LineEnd),
                '^' => Some(FirstNonBlank),
                // Screen position
                'H' => Some(ScreenTop),
                'M' => Some(ScreenMiddle),
                'L' => Some(ScreenBottom),
                // Scrollback
                'g' => Some(HistoryTop),
                'G' => Some(HistoryBottom),
                // Scroll (view only)
                'z' => Some(ScrollMiddle),
                // Selection
                ' ' => Some(StartCharSelect),
                'V' => Some(StartLineSelect),
                'v' => Some(StartRectSelect),
                'o' => Some(SwapAnchor),
                // In-line jump
                'f' => Some(JumpForward),
                'F' => Some(JumpBackward),
                't' => Some(JumpToForward),
                'T' => Some(JumpToBackward),
                ';' => Some(JumpAgain),
                ',' => Some(JumpReverse),
                // Paragraph/bracket
                '{' => Some(PreviousParagraph),
                '}' => Some(NextParagraph),
                '%' => Some(MatchingBracket),
                // Marks
                'X' => Some(SetMark),
                // Search
                '/' => Some(SearchForward),
                '?' => Some(SearchBackward),
                'n' => Some(SearchAgain),
                'N' => Some(SearchReverse),
                '*' => Some(SearchWordForward),
                '#' => Some(SearchWordBackward),
                // Copy
                '\r' | 'y' => {
                    self.mode = Mode::Normal;
                    Some(CopyAndExit)
                }
                'D' => {
                    self.mode = Mode::Normal;
                    Some(CopyToEndOfLine)
                }
                'A' => {
                    self.mode = Mode::Normal;
                    Some(AppendAndCancel)
                }
                // Other
                ':' => Some(OpenPrompt),
                'r' => Some(RefreshFromPane),
                'P' => Some(TogglePosition),
                'q' => {
                    self.mode = Mode::Normal;
                    Some(Exit)
                }
                _ => None,
            };
            if let Some(a) = action {
                return KeyResult::CopyMode(a);
            }
        }

        KeyResult::Consumed(None)
    }
}

/// Named keys that copy mode cares about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum NamedKey {
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    PageUp,
    PageDown,
    Home,
    End,
    Escape,
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

fn parse_prefix_trigger(spec: &str) -> Option<String> {
    match spec {
        "space" => Some(" ".to_string()),
        "tab" => Some("\t".to_string()),
        "enter" | "return" => Some("\r".to_string()),
        "escape" | "esc" => Some("\u{1b}".to_string()),
        _ if spec.chars().count() == 1 => Some(spec.to_string()),
        _ => None,
    }
}

/// Map a single-character key name to a platform-native keycode.
/// Must match the values returned by keymap::physical_to_native_keycode().
fn single_char_to_keycode(key: &str) -> Option<u32> {
    #[cfg(target_os = "macos")]
    {
        Some(match key {
            "a" => 0x00,
            "s" => 0x01,
            "d" => 0x02,
            "f" => 0x03,
            "h" => 0x04,
            "g" => 0x05,
            "z" => 0x06,
            "x" => 0x07,
            "c" => 0x08,
            "v" => 0x09,
            "b" => 0x0B,
            "q" => 0x0C,
            "w" => 0x0D,
            "e" => 0x0E,
            "r" => 0x0F,
            "y" => 0x10,
            "t" => 0x11,
            "u" => 0x20,
            "i" => 0x22,
            "o" => 0x1F,
            "p" => 0x23,
            "l" => 0x25,
            "j" => 0x26,
            "k" => 0x28,
            "n" => 0x2D,
            "m" => 0x2E,
            "1" => 0x12,
            "2" => 0x13,
            "3" => 0x14,
            "4" => 0x15,
            "5" => 0x17,
            "6" => 0x16,
            "7" => 0x1A,
            "8" => 0x1C,
            "9" => 0x19,
            "0" => 0x1D,
            "," | "comma" => 0x2B,
            "." | "period" => 0x2F,
            "/" | "slash" => 0x2C,
            ";" | "semicolon" => 0x29,
            "'" | "quote" => 0x27,
            "-" | "minus" => 0x1B,
            "=" | "equal" => 0x18,
            "[" | "left_bracket" => 0x21,
            "]" | "right_bracket" => 0x1E,
            "space" => 0x31,
            "enter" => 0x24,
            "tab" => 0x30,
            "escape" => 0x35,
            _ => return None,
        })
    }

    #[cfg(target_os = "linux")]
    {
        // ghostty_input_key_e enum values (W3C key codes)
        Some(match key {
            "a" => 20,
            "b" => 21,
            "c" => 22,
            "d" => 23,
            "e" => 24,
            "f" => 25,
            "g" => 26,
            "h" => 27,
            "i" => 28,
            "j" => 29,
            "k" => 30,
            "l" => 31,
            "m" => 32,
            "n" => 33,
            "o" => 34,
            "p" => 35,
            "q" => 36,
            "r" => 37,
            "s" => 38,
            "t" => 39,
            "u" => 40,
            "v" => 41,
            "w" => 42,
            "x" => 43,
            "y" => 44,
            "z" => 45,
            "0" => 6,
            "1" => 7,
            "2" => 8,
            "3" => 9,
            "4" => 10,
            "5" => 11,
            "6" => 12,
            "7" => 13,
            "8" => 14,
            "9" => 15,
            "," | "comma" => 55,
            "." | "period" => 56,
            "/" | "slash" => 57,
            ";" | "semicolon" => 51,
            "'" | "quote" => 52,
            "-" | "minus" => 16,
            "=" | "equal" => 17,
            "[" | "left_bracket" => 46,
            "]" | "right_bracket" => 47,
            "space" => 63,
            "enter" => 58,
            "tab" => 64,
            "escape" => 120,
            _ => return None,
        })
    }
}

fn parse_action(s: &str) -> Option<Action> {
    use SplitDirection::*;

    match s {
        "reload_config" => Some(Action::ReloadConfig),
        "close_surface" => Some(Action::CloseSurface),
        "break_pane" => Some(Action::BreakPane),
        "new_tab" => Some(Action::NewTab),
        "next_tab" => Some(Action::NextTab),
        "prev_tab" => Some(Action::PrevTab),
        "close_tab" => Some(Action::CloseTab),
        "search" => Some(Action::Search),
        "enter_copy_mode" => Some(Action::EnterCopyMode),
        "copy" => Some(Action::Copy),
        "paste" => Some(Action::Paste),
        "choose_buffer" => Some(Action::ChooseBuffer),
        "choose_tree" => Some(Action::ChooseTree),
        "find_window" => Some(Action::FindWindow),
        "set_tab_title" => Some(Action::SetTabTitle),
        "display_panes" => Some(Action::DisplayPanes),
        "mark_pane" => Some(Action::MarkPane),
        "clear_marked_pane" => Some(Action::ClearMarkedPane),
        "join_marked_pane:right" | "move_marked_pane:right" => {
            Some(Action::JoinMarkedPane(Right))
        }
        "join_marked_pane:down" | "move_marked_pane:down" => {
            Some(Action::JoinMarkedPane(Down))
        }
        "join_marked_pane:left" | "move_marked_pane:left" => {
            Some(Action::JoinMarkedPane(Left))
        }
        "join_marked_pane:up" | "move_marked_pane:up" => Some(Action::JoinMarkedPane(Up)),
        "rotate_panes_forward" => Some(Action::RotatePanesForward),
        "rotate_panes_backward" => Some(Action::RotatePanesBackward),
        "swap_pane_next" => Some(Action::SwapPaneNext),
        "swap_pane_previous" => Some(Action::SwapPanePrevious),
        "next_layout" => Some(Action::NextLayout),
        "previous_layout" => Some(Action::PreviousLayout),
        "rebalance_layout" => Some(Action::RebalanceLayout),
        "select_layout:manual" => Some(Action::SelectLayout(crate::session::TabLayout::Manual)),
        "select_layout:even-horizontal" => {
            Some(Action::SelectLayout(crate::session::TabLayout::EvenHorizontal))
        }
        "select_layout:even-vertical" => {
            Some(Action::SelectLayout(crate::session::TabLayout::EvenVertical))
        }
        "select_layout:main-horizontal" => {
            Some(Action::SelectLayout(crate::session::TabLayout::MainHorizontal))
        }
        "select_layout:main-vertical" => {
            Some(Action::SelectLayout(crate::session::TabLayout::MainVertical))
        }
        "select_layout:tiled" => Some(Action::SelectLayout(crate::session::TabLayout::Tiled)),
        "toggle_zoom" => Some(Action::ToggleZoom),
        "command_prompt" => Some(Action::OpenCommandPrompt),
        "next_pane" => Some(Action::NextPane),
        "previous_pane" => Some(Action::PreviousPane),
        "previous_tab" => Some(Action::PreviousTab),
        "goto_split:left" => Some(Action::GotoSplit(PaneFocusDirection::Left)),
        "goto_split:right" => Some(Action::GotoSplit(PaneFocusDirection::Right)),
        "goto_split:top" => Some(Action::GotoSplit(PaneFocusDirection::Up)),
        "goto_split:bottom" => Some(Action::GotoSplit(PaneFocusDirection::Down)),
        "new_split:right" => Some(Action::NewSplit(Right)),
        "new_split:down" => Some(Action::NewSplit(Down)),
        "new_split:left" => Some(Action::NewSplit(Left)),
        "new_split:up" => Some(Action::NewSplit(Up)),
        "goto_tab:last" => Some(Action::GotoTab(TabTarget::Last)),
        _ if s.starts_with("goto_tab:") => {
            let n: usize = s["goto_tab:".len()..].parse().ok()?;
            Some(Action::GotoTab(TabTarget::Index(n.saturating_sub(1))))
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_copy_mode_bindings() -> Bindings {
        let config = Config {
            prefix_key: Some("ctrl+s".to_string()),
            keybinds: Default::default(),
            control_socket: None,
            remote_port: None,
            remote_auth_key: None,
            font_family: None,
            font_size: None,
            background_opacity: None,
            background_opacity_cells: false,
            cursor_style: None,
            cursor_blink: false,
            desktop_notifications: true,
            notify_on_command_finish: crate::config::NotifyOnCommandFinish::Never,
            notify_on_command_finish_action: crate::config::NotifyOnCommandFinishAction {
                bell: true,
                notify: false,
            },
            notify_on_command_finish_after_ns: 5 * 1_000_000_000,
        };
        let mut b = Bindings::from_config(&config);
        b.mode = Mode::CopyMode;
        b
    }

    fn send_char(b: &mut Bindings, ch: char) -> KeyResult {
        b.handle_key(Some(ch), 0, 0, None)
    }

    fn send_ctrl(b: &mut Bindings, ch: char) -> KeyResult {
        b.handle_key(Some(ch), 0, ffi::GHOSTTY_MODS_CTRL, None)
    }

    fn send_alt(b: &mut Bindings, ch: char) -> KeyResult {
        b.handle_key(Some(ch), 0, ffi::GHOSTTY_MODS_ALT, None)
    }

    fn send_named(b: &mut Bindings, key: NamedKey) -> KeyResult {
        b.handle_key(None, 0, 0, Some(key))
    }

    fn assert_copy_action(result: KeyResult, expected: &str) {
        match result {
            KeyResult::CopyMode(action) => {
                let actual = format!("{action:?}");
                assert!(
                    actual.contains(expected),
                    "expected CopyMode action containing '{expected}', got '{actual}'"
                );
            }
            KeyResult::Consumed(_) => panic!("expected CopyMode, got Consumed"),
            KeyResult::Forward => panic!("expected CopyMode, got Forward"),
        }
    }

    // --- Cursor movement ---
    #[test]
    fn test_copy_mode_hjkl() {
        let mut b = make_copy_mode_bindings();
        assert_copy_action(send_char(&mut b, 'h'), "Left");
        assert_copy_action(send_char(&mut b, 'j'), "Down");
        assert_copy_action(send_char(&mut b, 'k'), "Up");
        assert_copy_action(send_char(&mut b, 'l'), "Right");
    }

    #[test]
    fn test_copy_mode_arrows() {
        let mut b = make_copy_mode_bindings();
        assert_copy_action(send_named(&mut b, NamedKey::ArrowLeft), "Left");
        assert_copy_action(send_named(&mut b, NamedKey::ArrowDown), "Down");
        assert_copy_action(send_named(&mut b, NamedKey::ArrowUp), "Up");
        assert_copy_action(send_named(&mut b, NamedKey::ArrowRight), "Right");
    }

    #[test]
    fn test_copy_mode_backspace() {
        let mut b = make_copy_mode_bindings();
        assert_copy_action(send_char(&mut b, '\x08'), "Left");
    }

    // --- Word movement ---
    #[test]
    fn test_copy_mode_word_keys() {
        let mut b = make_copy_mode_bindings();
        assert_copy_action(send_char(&mut b, 'w'), "WordNext");
        assert_copy_action(send_char(&mut b, 'b'), "WordBack");
        assert_copy_action(send_char(&mut b, 'e'), "WordEnd");
        assert_copy_action(send_char(&mut b, 'W'), "BigWordNext");
        assert_copy_action(send_char(&mut b, 'B'), "BigWordBack");
        assert_copy_action(send_char(&mut b, 'E'), "BigWordEnd");
    }

    // --- Line position ---
    #[test]
    fn test_copy_mode_line_position() {
        let mut b = make_copy_mode_bindings();
        assert_copy_action(send_char(&mut b, '0'), "LineStart");
        assert_copy_action(send_char(&mut b, '$'), "LineEnd");
        assert_copy_action(send_char(&mut b, '^'), "FirstNonBlank");
    }

    // --- Screen position ---
    #[test]
    fn test_copy_mode_screen_position() {
        let mut b = make_copy_mode_bindings();
        assert_copy_action(send_char(&mut b, 'H'), "ScreenTop");
        assert_copy_action(send_char(&mut b, 'M'), "ScreenMiddle");
        assert_copy_action(send_char(&mut b, 'L'), "ScreenBottom");
    }

    // --- Scrollback ---
    #[test]
    fn test_copy_mode_history() {
        let mut b = make_copy_mode_bindings();
        assert_copy_action(send_char(&mut b, 'g'), "HistoryTop");
        assert_copy_action(send_char(&mut b, 'G'), "HistoryBottom");
        assert_copy_action(send_named(&mut b, NamedKey::Home), "HistoryTop");
        assert_copy_action(send_named(&mut b, NamedKey::End), "HistoryBottom");
    }

    // --- Page/scroll ---
    #[test]
    fn test_copy_mode_page_scroll() {
        let mut b = make_copy_mode_bindings();
        assert_copy_action(send_named(&mut b, NamedKey::PageUp), "PageUp");
        assert_copy_action(send_named(&mut b, NamedKey::PageDown), "PageDown");
        assert_copy_action(send_ctrl(&mut b, 'b'), "PageUp");
        assert_copy_action(send_ctrl(&mut b, 'f'), "PageDown");
        assert_copy_action(send_ctrl(&mut b, 'u'), "HalfPageUp");
        assert_copy_action(send_ctrl(&mut b, 'd'), "HalfPageDown");
        assert_copy_action(send_ctrl(&mut b, 'y'), "ScrollUp");
        assert_copy_action(send_ctrl(&mut b, 'e'), "ScrollDown");
        assert_copy_action(send_char(&mut b, 'z'), "ScrollMiddle");
    }

    // --- Selection ---
    #[test]
    fn test_copy_mode_selection_modes() {
        let mut b = make_copy_mode_bindings();
        assert_copy_action(send_char(&mut b, ' '), "StartCharSelect");
        assert_copy_action(send_char(&mut b, 'V'), "StartLineSelect");
        assert_copy_action(send_char(&mut b, 'v'), "StartRectSelect");
        assert_copy_action(send_ctrl(&mut b, 'v'), "StartRectSelect");
        assert_copy_action(send_char(&mut b, 'o'), "SwapAnchor");
    }

    #[test]
    fn test_copy_mode_escape_clears() {
        let mut b = make_copy_mode_bindings();
        assert_copy_action(send_named(&mut b, NamedKey::Escape), "ClearSelection");
        // Bindings doesn't change mode on ClearSelection — that's handled by dispatch
        assert!(b.is_copy_mode());
    }

    #[test]
    fn test_copy_mode_q_exits() {
        let mut b = make_copy_mode_bindings();
        assert_copy_action(send_char(&mut b, 'q'), "Exit");
        assert!(!b.is_copy_mode());
    }

    // --- In-line jump ---
    #[test]
    fn test_copy_mode_jump_keys() {
        let mut b = make_copy_mode_bindings();
        assert_copy_action(send_char(&mut b, 'f'), "JumpForward");
        assert_copy_action(send_char(&mut b, 'F'), "JumpBackward");
        assert_copy_action(send_char(&mut b, 't'), "JumpToForward");
        assert_copy_action(send_char(&mut b, 'T'), "JumpToBackward");
        assert_copy_action(send_char(&mut b, ';'), "JumpAgain");
        assert_copy_action(send_char(&mut b, ','), "JumpReverse");
    }

    // --- Paragraph/bracket ---
    #[test]
    fn test_copy_mode_paragraph_bracket() {
        let mut b = make_copy_mode_bindings();
        assert_copy_action(send_char(&mut b, '{'), "PreviousParagraph");
        assert_copy_action(send_char(&mut b, '}'), "NextParagraph");
        assert_copy_action(send_char(&mut b, '%'), "MatchingBracket");
    }

    // --- Marks ---
    #[test]
    fn test_copy_mode_marks() {
        let mut b = make_copy_mode_bindings();
        assert_copy_action(send_char(&mut b, 'X'), "SetMark");
        assert_copy_action(send_alt(&mut b, 'x'), "JumpToMark");
    }

    // --- Search ---
    #[test]
    fn test_copy_mode_search() {
        let mut b = make_copy_mode_bindings();
        assert_copy_action(send_char(&mut b, '/'), "SearchForward");
        assert_copy_action(send_char(&mut b, '?'), "SearchBackward");
        assert_copy_action(send_char(&mut b, 'n'), "SearchAgain");
        assert_copy_action(send_char(&mut b, 'N'), "SearchReverse");
        assert_copy_action(send_char(&mut b, '*'), "SearchWordForward");
        assert_copy_action(send_char(&mut b, '#'), "SearchWordBackward");
    }

    // --- Copy ---
    #[test]
    fn test_copy_mode_copy_exits() {
        let mut b = make_copy_mode_bindings();
        assert_copy_action(send_char(&mut b, 'y'), "CopyAndExit");
        assert!(!b.is_copy_mode());
    }

    #[test]
    fn test_copy_mode_enter_copies() {
        let mut b = make_copy_mode_bindings();
        assert_copy_action(send_char(&mut b, '\r'), "CopyAndExit");
        assert!(!b.is_copy_mode());
    }

    #[test]
    fn test_copy_mode_d_copies_eol() {
        let mut b = make_copy_mode_bindings();
        assert_copy_action(send_char(&mut b, 'D'), "CopyToEndOfLine");
        assert!(!b.is_copy_mode());
    }

    #[test]
    fn test_copy_mode_a_appends() {
        let mut b = make_copy_mode_bindings();
        assert_copy_action(send_char(&mut b, 'A'), "AppendAndCancel");
        assert!(!b.is_copy_mode());
    }

    // --- Other ---
    #[test]
    fn test_copy_mode_other() {
        let mut b = make_copy_mode_bindings();
        assert_copy_action(send_char(&mut b, ':'), "OpenPrompt");
        assert_copy_action(send_char(&mut b, 'r'), "RefreshFromPane");
        assert_copy_action(send_char(&mut b, 'P'), "TogglePosition");
    }

    #[test]
    fn test_direct_punctuation_keybinds_parse() {
        let config = Config::parse(
            r#"
prefix-key = ctrl+s
keybind = ctrl+super+, = reload_config
keybind = super+/ = search
"#,
        );
        let bindings = Bindings::from_config(&config);
        assert_eq!(bindings.direct.len(), 2);
    }

    // --- Selection rect computation ---
    #[test]
    fn test_selection_rects_char_single_line() {
        use crate::SelectionMode;
        use crate::main_tests::compute_rects;
        let rects = compute_rects(SelectionMode::Char, 5, 10, 5, 3, 0, 80, 8.0, 16.0, 0.0);
        assert_eq!(rects.len(), 1);
        let (x, y, w, h) = rects[0];
        assert_eq!(x, 3.0 * 8.0); // min col * cell_width
        assert_eq!(y, 5.0 * 16.0); // row * cell_height
        assert_eq!(w, 8.0 * 8.0); // (10 - 3 + 1) * cell_width
        assert_eq!(h, 16.0);
    }

    #[test]
    fn test_selection_rects_char_multi_line() {
        use crate::SelectionMode;
        use crate::main_tests::compute_rects;
        let rects = compute_rects(SelectionMode::Char, 7, 5, 5, 10, 0, 80, 8.0, 16.0, 0.0);
        assert_eq!(rects.len(), 3); // first partial + middle + last partial
    }

    #[test]
    fn test_selection_rects_line() {
        use crate::SelectionMode;
        use crate::main_tests::compute_rects;
        let rects = compute_rects(SelectionMode::Line, 7, 5, 5, 10, 0, 80, 8.0, 16.0, 0.0);
        assert_eq!(rects.len(), 3); // 3 rows
        // All full-width
        for (x, _, w, _) in &rects {
            assert_eq!(*x, 0.0);
            assert_eq!(*w, 80.0 * 8.0);
        }
    }

    #[test]
    fn test_selection_rects_rectangle() {
        use crate::SelectionMode;
        use crate::main_tests::compute_rects;
        let rects = compute_rects(SelectionMode::Rectangle, 7, 15, 5, 5, 0, 80, 8.0, 16.0, 0.0);
        assert_eq!(rects.len(), 3); // 3 rows
        // All same width
        for (x, _, w, _) in &rects {
            assert_eq!(*x, 5.0 * 8.0);
            assert_eq!(*w, 11.0 * 8.0); // (15 - 5 + 1) * 8
        }
    }

    // --- Parse action ---
    #[test]
    fn test_parse_action() {
        assert!(matches!(parse_action("new_tab"), Some(Action::NewTab)));
        assert!(matches!(
            parse_action("close_surface"),
            Some(Action::CloseSurface)
        ));
        assert!(matches!(
            parse_action("enter_copy_mode"),
            Some(Action::EnterCopyMode)
        ));
        assert!(matches!(
            parse_action("goto_tab:3"),
            Some(Action::GotoTab(TabTarget::Index(2)))
        ));
        assert!(matches!(
            parse_action("goto_tab:last"),
            Some(Action::GotoTab(TabTarget::Last))
        ));
        assert!(matches!(
            parse_action("resize_split:left,10"),
            Some(Action::ResizeSplit(Direction::Left, 10))
        ));
        assert!(matches!(
            parse_action("break_pane"),
            Some(Action::BreakPane)
        ));
        assert!(matches!(
            parse_action("display_panes"),
            Some(Action::DisplayPanes)
        ));
        assert!(matches!(
            parse_action("choose_buffer"),
            Some(Action::ChooseBuffer)
        ));
        assert!(matches!(
            parse_action("choose_tree"),
            Some(Action::ChooseTree)
        ));
        assert!(matches!(
            parse_action("find_window"),
            Some(Action::FindWindow)
        ));
        assert!(matches!(
            parse_action("rotate_panes_forward"),
            Some(Action::RotatePanesForward)
        ));
        assert!(matches!(
            parse_action("swap_pane_previous"),
            Some(Action::SwapPanePrevious)
        ));
        assert!(matches!(
            parse_action("select_layout:tiled"),
            Some(Action::SelectLayout(crate::session::TabLayout::Tiled))
        ));
        assert!(matches!(
            parse_action("rebalance_layout"),
            Some(Action::RebalanceLayout)
        ));
        assert!(matches!(parse_action("mark_pane"), Some(Action::MarkPane)));
        assert!(matches!(
            parse_action("join_marked_pane:down"),
            Some(Action::JoinMarkedPane(SplitDirection::Down))
        ));
        assert!(parse_action("nonexistent").is_none());
    }

    // --- Prefix key parsing ---
    #[test]
    fn test_parse_prefix_key() {
        let pk = parse_prefix_key("ctrl+s").unwrap();
        #[cfg(target_os = "macos")]
        assert_eq!(pk.keycode, 0x01); // macOS kVK_ANSI_S
        #[cfg(target_os = "linux")]
        assert_eq!(pk.keycode, 38); // ghostty_input_key_e::GHOSTTY_KEY_S
        assert_eq!(pk.mods_mask, ffi::GHOSTTY_MODS_CTRL);

        let pk = parse_prefix_key("ctrl+shift+a").unwrap();
        assert_eq!(
            pk.mods_mask,
            ffi::GHOSTTY_MODS_CTRL | ffi::GHOSTTY_MODS_SHIFT
        );

        assert!(parse_prefix_key("ctrl+unknown").is_none());
    }

    #[test]
    fn test_prefixed_keybind_uses_implicit_prefix() {
        let mut config = Config::default();
        config
            .keybinds
            .insert("ctrl+a>c".to_string(), "new_tab".to_string());
        let mut bindings = Bindings::from_config(&config);

        assert!(matches!(
            bindings.handle_key(
                Some('a'),
                single_char_to_keycode("a").unwrap(),
                ffi::GHOSTTY_MODS_CTRL,
                None
            ),
            KeyResult::Consumed(None)
        ));
        assert!(matches!(
            bindings.handle_key(
                Some('c'),
                single_char_to_keycode("c").unwrap(),
                ffi::GHOSTTY_MODS_NONE,
                None
            ),
            KeyResult::Consumed(Some(Action::NewTab))
        ));
    }

    #[test]
    fn test_parse_prefix_trigger_named_keys() {
        assert_eq!(parse_prefix_trigger("space").as_deref(), Some(" "));
        assert_eq!(parse_prefix_trigger("tab").as_deref(), Some("\t"));
        assert_eq!(parse_prefix_trigger("enter").as_deref(), Some("\r"));
        assert_eq!(parse_prefix_trigger("esc").as_deref(), Some("\u{1b}"));
    }
}
