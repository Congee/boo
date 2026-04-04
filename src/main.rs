mod backend;
mod bindings;
mod config;
mod control;
mod ffi;
mod keymap;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod vt_terminal_canvas;
#[cfg(target_os = "linux")]
mod linux_vt_backend;
#[cfg(target_os = "macos")]
mod macos_vt_backend;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod vt_backend_core;
mod pane;
mod platform;
mod session;
mod splits;
mod tabs;
mod tmux;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod vt;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod unix_pty;

use iced::widget::{column, container, row, text};
use iced::window;
use iced::{keyboard, mouse, Color, Element, Event, Font, Length, Size, Subscription, Task, Theme};
use backend::TerminalBackend;
use pane::PaneHandle;
use std::ffi::{c_void, CStr, CString};
#[cfg(target_os = "linux")]
use std::process::Command;
use std::ptr;

/// Status bar height in points.
const STATUS_BAR_HEIGHT: f64 = 20.0;

static SCROLL_RX: std::sync::OnceLock<std::sync::Mutex<std::sync::mpsc::Receiver<platform::ScrollEvent>>> =
    std::sync::OnceLock::new();
static KEY_EVENT_RX: std::sync::OnceLock<std::sync::Mutex<std::sync::mpsc::Receiver<platform::KeyEvent>>> =
    std::sync::OnceLock::new();
static TEXT_INPUT_RX: std::sync::OnceLock<std::sync::Mutex<std::sync::mpsc::Receiver<platform::TextInputEvent>>> =
    std::sync::OnceLock::new();

#[derive(Clone, Copy)]
struct CommandFinishedEvent {
    exit_code: Option<u8>,
    duration_ns: u64,
}


static STARTUP_SESSION: std::sync::OnceLock<String> = std::sync::OnceLock::new();

const DEFAULT_TERMINAL_FONT_SIZE: f32 = 14.0;
const DEFAULT_BACKGROUND_OPACITY: f32 = 1.0;

#[derive(Debug)]
struct ResolvedAppearance {
    font_family: Option<&'static str>,
    font_size: f32,
    background_opacity: f32,
    background_opacity_cells: bool,
    #[cfg(target_os = "linux")]
    font_bytes: Option<Vec<u8>>,
}

fn leak_font_family(name: &str) -> &'static str {
    Box::leak(name.to_owned().into_boxed_str())
}

#[cfg(target_os = "linux")]
fn resolve_linux_font(name: &str) -> (Option<&'static str>, Option<Vec<u8>>) {
    let output = Command::new("fc-match")
        .args(["-f", "%{family[0]}|%{file}\n", name])
        .output();

    let Ok(output) = output else {
        log::warn!("failed to run fc-match for font family {:?}", name);
        return (Some(leak_font_family(name)), None);
    };

    if !output.status.success() {
        log::warn!("fc-match failed for font family {:?}", name);
        return (Some(leak_font_family(name)), None);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout.trim();
    let Some((resolved_family, resolved_file)) = line.split_once('|') else {
        return (Some(leak_font_family(name)), None);
    };

    let resolved_family = resolved_family.trim();
    let resolved_file = resolved_file.trim();

    let family = if resolved_family.is_empty() {
        leak_font_family(name)
    } else {
        if resolved_family != name {
            log::info!(
                "resolved font family {:?} to {:?} via fc-match",
                name,
                resolved_family
            );
        }
        leak_font_family(resolved_family)
    };

    let font_bytes = if resolved_file.is_empty() {
        None
    } else {
        match std::fs::read(resolved_file) {
            Ok(bytes) => Some(bytes),
            Err(error) => {
                log::warn!(
                    "failed to read resolved font file {:?} for family {:?}: {}",
                    resolved_file,
                    family,
                    error
                );
                None
            }
        }
    };

    (Some(family), font_bytes)
}

fn terminal_metrics(font_size: f32) -> (f64, f64) {
    let size = font_size.max(8.0) as f64;
    let cell_width = (size * 0.62).max(6.0).ceil();
    let cell_height = (size * 1.35).max(12.0).ceil();
    (cell_width, cell_height)
}

fn configured_font(family: Option<&'static str>) -> Font {
    family.map(Font::with_name).unwrap_or(Font::MONOSPACE)
}

fn main() {
    env_logger::init();

    // Parse --session <name> from CLI args
    let args: Vec<String> = std::env::args().collect();
    if let Some(pos) = args.iter().position(|a| a == "--session") {
        if let Some(name) = args.get(pos + 1) {
            STARTUP_SESSION.set(name.clone()).ok();
        }
    }

    #[cfg(not(target_os = "linux"))]
    {
        let result = unsafe { ffi::ghostty_init(0, ptr::null_mut()) };
        if result != ffi::GHOSTTY_SUCCESS {
            eprintln!("Failed to initialize ghostty: error code {result}");
            std::process::exit(1);
        }

        log::info!("ghostty initialized");
    }

    let (scroll_tx, scroll_rx) = std::sync::mpsc::channel();
    let (key_event_tx, key_event_rx) = std::sync::mpsc::channel();
    let (text_input_tx, text_input_rx) = std::sync::mpsc::channel();
    platform::install_event_monitors(scroll_tx, key_event_tx, text_input_tx);
    // Store in a static so BooApp::new can pick it up
    SCROLL_RX.set(std::sync::Mutex::new(scroll_rx)).ok();
    KEY_EVENT_RX.set(std::sync::Mutex::new(key_event_rx)).ok();
    TEXT_INPUT_RX.set(std::sync::Mutex::new(text_input_rx)).ok();

    iced::application(BooApp::new, BooApp::update, BooApp::view)
        .title("boo")
        .decorations(false)
        .transparent(true)
        .style(|state, _theme| state.window_style())
        .theme(BooApp::theme)
        .subscription(BooApp::subscription)
        .run()
        .unwrap();
}

struct BooApp {
    backend: backend::Backend,
    tabs: tabs::TabManager,
    parent_view: *mut c_void,
    ctl_rx: std::sync::mpsc::Receiver<control::ControlCmd>,
    scroll_rx: std::sync::mpsc::Receiver<platform::ScrollEvent>,
    key_event_rx: std::sync::mpsc::Receiver<platform::KeyEvent>,
    text_input_rx: std::sync::mpsc::Receiver<platform::TextInputEvent>,
    bindings: bindings::Bindings,
    socket_path: Option<String>,
    dump_keys: bool,
    last_size: Size,
    last_mouse_pos: (f64, f64),
    divider_drag: Option<splits::Direction>,
    scrollbar_drag: bool,
    scrollbar_opacity: f32,
    cell_width: f64,
    cell_height: f64,
    scrollbar: ffi::ghostty_action_scrollbar_s,
    scrollbar_layer: *mut c_void,
    search_active: bool,
    search_query: String,
    search_total: isize,
    search_selected: isize,
    pwd: String,
    preedit_text: String,
    last_clipboard_text: String,
    copy_mode: Option<CopyModeState>,
    command_prompt: CommandPrompt,
    terminal_font_family: Option<&'static str>,
    terminal_font_size: f32,
    background_opacity: f32,
    background_opacity_cells: bool,
    appearance_revision: u64,
    app_focused: bool,
    desktop_notifications_enabled: bool,
    notify_on_command_finish: config::NotifyOnCommandFinish,
    notify_on_command_finish_action: config::NotifyOnCommandFinishAction,
    notify_on_command_finish_after_ns: u64,
    #[cfg(target_os = "linux")]
    pending_font_bytes: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SelectionMode {
    None,
    Char,
    Line,
    Rectangle,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum JumpKind {
    Forward,
    Backward,
    ToForward,
    ToBackward,
}

#[derive(Debug, Clone, Copy)]
enum WordMoveKind {
    NextWord, PrevWord, EndWord,
    NextBigWord, PrevBigWord, EndBigWord,
}

struct CopyModeState {
    cursor_row: i64,
    cursor_col: u32,
    selection: SelectionMode,
    sel_anchor: Option<(i64, u32)>,
    highlight_layers: Vec<*mut c_void>,
    cursor_layer: *mut c_void,
    cell_width: f64,
    cell_height: f64,
    viewport_rows: u32,
    viewport_cols: u32,
    mark: Option<(i64, u32)>,
    last_jump: Option<(char, JumpKind)>,
    last_search_forward: bool,
    pending_jump: Option<JumpKind>,
    show_position: bool,
}

struct CommandDef {
    name: &'static str,
    description: &'static str,
    args: &'static str, // e.g. "<n>" or "" for no args
}

const COMMANDS: &[CommandDef] = &[
    CommandDef { name: "split-right", description: "vertical split", args: "" },
    CommandDef { name: "split-down", description: "horizontal split", args: "" },
    CommandDef { name: "split-left", description: "vertical split (left)", args: "" },
    CommandDef { name: "split-up", description: "horizontal split (up)", args: "" },
    CommandDef { name: "resize-left", description: "resize pane left", args: "<n>" },
    CommandDef { name: "resize-right", description: "resize pane right", args: "<n>" },
    CommandDef { name: "resize-up", description: "resize pane up", args: "<n>" },
    CommandDef { name: "resize-down", description: "resize pane down", args: "<n>" },
    CommandDef { name: "close-pane", description: "close focused pane", args: "" },
    CommandDef { name: "new-tab", description: "create new tab", args: "" },
    CommandDef { name: "next-tab", description: "switch to next tab", args: "" },
    CommandDef { name: "prev-tab", description: "switch to previous tab", args: "" },
    CommandDef { name: "close-tab", description: "close current tab", args: "" },
    CommandDef { name: "goto-tab", description: "go to tab number", args: "<n>" },
    CommandDef { name: "last-tab", description: "go to last tab", args: "" },
    CommandDef { name: "next-pane", description: "focus next pane", args: "" },
    CommandDef { name: "prev-pane", description: "focus previous pane", args: "" },
    CommandDef { name: "copy-mode", description: "enter copy mode", args: "" },
    CommandDef { name: "command-prompt", description: "open command prompt", args: "" },
    CommandDef { name: "search", description: "open search", args: "" },
    CommandDef { name: "paste", description: "paste from clipboard", args: "" },
    CommandDef { name: "zoom", description: "toggle pane zoom", args: "" },
    CommandDef { name: "reload-config", description: "reload configuration", args: "" },
    CommandDef { name: "goto-line", description: "jump to line (copy mode)", args: "<n>" },
    CommandDef { name: "set", description: "set ghostty config value", args: "<key> <value>" },
    CommandDef { name: "load-session", description: "load a session layout", args: "<name>" },
    CommandDef { name: "save-session", description: "save current layout", args: "<name>" },
    CommandDef { name: "list-sessions", description: "list available sessions", args: "" },
];

struct CommandPrompt {
    active: bool,
    input: String,
    history: Vec<String>,
    history_idx: Option<usize>,
    suggestions: Vec<usize>, // indices into COMMANDS
    selected_suggestion: usize,
}

impl CommandPrompt {
    fn new() -> Self {
        CommandPrompt {
            active: false,
            input: String::new(),
            history: Vec::new(),
            history_idx: None,
            suggestions: Vec::new(),
            selected_suggestion: 0,
        }
    }

    fn update_suggestions(&mut self) {
        let query = self.input.split_whitespace().next().unwrap_or("");
        if query.is_empty() {
            self.suggestions = (0..COMMANDS.len()).collect();
        } else {
            let mut scored: Vec<(usize, i32)> = COMMANDS.iter().enumerate()
                .filter_map(|(i, cmd)| {
                    let score = fuzzy_score(query, cmd.name);
                    if score > 0 { Some((i, score)) } else { None }
                })
                .collect();
            scored.sort_by(|a, b| b.1.cmp(&a.1));
            self.suggestions = scored.into_iter().map(|(i, _)| i).take(7).collect();
        }
        self.selected_suggestion = 0;
    }

    fn selected_command(&self) -> Option<&'static CommandDef> {
        self.suggestions.get(self.selected_suggestion).map(|&i| &COMMANDS[i])
    }
}

fn fuzzy_score(query: &str, target: &str) -> i32 {
    if query.is_empty() { return 1; }
    let ql = query.to_lowercase();
    let tl = target.to_lowercase();

    // Exact prefix
    if tl.starts_with(&ql) { return 100 + (100 - target.len() as i32); }

    // Word-initial match: "sr" matches "split-right" via s...r...
    let parts: Vec<&str> = tl.split('-').collect();
    let mut qi = 0;
    let qchars: Vec<char> = ql.chars().collect();
    for part in &parts {
        if qi < qchars.len() && part.starts_with(qchars[qi]) {
            qi += 1;
        }
    }
    if qi == qchars.len() { return 50 + (100 - target.len() as i32); }

    // Subsequence match
    let mut qi = 0;
    for tc in tl.chars() {
        if qi < qchars.len() && tc == qchars[qi] {
            qi += 1;
        }
    }
    if qi == qchars.len() { return 10 + (100 - target.len() as i32); }

    0
}

#[derive(Debug, Clone)]
enum Message {
    Frame,
    #[cfg(target_os = "linux")]
    FontLoaded,
    IcedEvent(Event),
}

impl BooApp {
    fn resolve_appearance_config(config: &config::Config) -> ResolvedAppearance {
        #[cfg(target_os = "linux")]
        let (font_family, font_bytes) = config
            .font_family
            .as_deref()
            .map(resolve_linux_font)
            .unwrap_or((None, None));

        #[cfg(not(target_os = "linux"))]
        let font_family = config.font_family.as_deref().map(leak_font_family);

        ResolvedAppearance {
            font_family,
            font_size: config.font_size.unwrap_or(DEFAULT_TERMINAL_FONT_SIZE),
            background_opacity: config
                .background_opacity
                .unwrap_or(DEFAULT_BACKGROUND_OPACITY)
                .clamp(0.0, 1.0),
            background_opacity_cells: config.background_opacity_cells,
            #[cfg(target_os = "linux")]
            font_bytes,
        }
    }

    fn split_direction_from_str(direction: &str) -> bindings::SplitDirection {
        match direction {
            "right" => bindings::SplitDirection::Right,
            "down" => bindings::SplitDirection::Down,
            "left" => bindings::SplitDirection::Left,
            "up" => bindings::SplitDirection::Up,
            _ => bindings::SplitDirection::Right,
        }
    }

    fn new() -> (Self, Task<Message>) {
        let backend = <backend::Backend as backend::TerminalBackend>::new(ptr::null_mut());

        let boo_config = config::Config::load();
        let ctl_rx = control::start(boo_config.control_socket.as_deref());
        let bindings = bindings::Bindings::from_config(&boo_config);
        let appearance = Self::resolve_appearance_config(&boo_config);
        let (cell_width, cell_height) = terminal_metrics(appearance.font_size);

        #[cfg(target_os = "linux")]
        {
            (
                Self {
                    backend,
                    tabs: tabs::TabManager::new(),
                    parent_view: ptr::null_mut(),
                    ctl_rx,
                    scroll_rx: SCROLL_RX
                        .get()
                        .and_then(|m| m.lock().ok())
                        .map(|mut guard| {
                            let (_, rx) = std::sync::mpsc::channel();
                            std::mem::replace(&mut *guard, rx)
                        })
                        .unwrap_or_else(|| std::sync::mpsc::channel().1),
                    key_event_rx: KEY_EVENT_RX
                        .get()
                        .and_then(|m| m.lock().ok())
                        .map(|mut guard| {
                            let (_, rx) = std::sync::mpsc::channel();
                            std::mem::replace(&mut *guard, rx)
                        })
                        .unwrap_or_else(|| std::sync::mpsc::channel().1),
                    text_input_rx: TEXT_INPUT_RX
                        .get()
                        .and_then(|m| m.lock().ok())
                        .map(|mut guard| {
                            let (_, rx) = std::sync::mpsc::channel();
                            std::mem::replace(&mut *guard, rx)
                        })
                        .unwrap_or_else(|| std::sync::mpsc::channel().1),
                    bindings,
                    socket_path: boo_config.control_socket.clone(),
                    dump_keys: std::env::args().any(|a| a == "--dump-keys"),
                    last_size: Size::new(0.0, 0.0),
                    last_mouse_pos: (0.0, 0.0),
                    divider_drag: None,
                    scrollbar_drag: false,
                    scrollbar_opacity: 0.0,
                    cell_width,
                    cell_height,
                    scrollbar: ffi::ghostty_action_scrollbar_s { total: 0, offset: 0, len: 0 },
                    scrollbar_layer: ptr::null_mut(),
                    search_active: false,
                    search_query: String::new(),
                    search_total: 0,
                    search_selected: 0,
                    pwd: String::new(),
                    preedit_text: String::new(),
                    last_clipboard_text: String::new(),
                    copy_mode: None,
                    command_prompt: CommandPrompt::new(),
                    terminal_font_family: appearance.font_family,
                    terminal_font_size: appearance.font_size,
                    background_opacity: appearance.background_opacity,
                    background_opacity_cells: appearance.background_opacity_cells,
                    appearance_revision: 1,
                    app_focused: true,
                    desktop_notifications_enabled: boo_config.desktop_notifications,
                    notify_on_command_finish: boo_config.notify_on_command_finish,
                    notify_on_command_finish_action: boo_config.notify_on_command_finish_action,
                    notify_on_command_finish_after_ns: boo_config.notify_on_command_finish_after_ns,
                    pending_font_bytes: appearance.font_bytes,
                },
                Task::none(),
            )
        }

        #[cfg(not(target_os = "linux"))]
        {
            (
                Self {
                    backend,
                    tabs: tabs::TabManager::new(),
                    parent_view: ptr::null_mut(),
                    ctl_rx,
                    scroll_rx: SCROLL_RX
                        .get()
                        .and_then(|m| m.lock().ok())
                        .map(|mut guard| {
                            let (_, rx) = std::sync::mpsc::channel();
                            std::mem::replace(&mut *guard, rx)
                        })
                        .unwrap_or_else(|| std::sync::mpsc::channel().1),
                    key_event_rx: KEY_EVENT_RX
                        .get()
                        .and_then(|m| m.lock().ok())
                        .map(|mut guard| {
                            let (_, rx) = std::sync::mpsc::channel();
                            std::mem::replace(&mut *guard, rx)
                        })
                        .unwrap_or_else(|| std::sync::mpsc::channel().1),
                    text_input_rx: TEXT_INPUT_RX
                        .get()
                        .and_then(|m| m.lock().ok())
                        .map(|mut guard| {
                            let (_, rx) = std::sync::mpsc::channel();
                            std::mem::replace(&mut *guard, rx)
                        })
                        .unwrap_or_else(|| std::sync::mpsc::channel().1),
                    bindings,
                    socket_path: boo_config.control_socket.clone(),
                    dump_keys: std::env::args().any(|a| a == "--dump-keys"),
                    last_size: Size::new(0.0, 0.0),
                    last_mouse_pos: (0.0, 0.0),
                    divider_drag: None,
                    scrollbar_drag: false,
                    scrollbar_opacity: 0.0,
                    cell_width,
                    cell_height,
                    scrollbar: ffi::ghostty_action_scrollbar_s { total: 0, offset: 0, len: 0 },
                    scrollbar_layer: ptr::null_mut(),
                    search_active: false,
                    search_query: String::new(),
                    search_total: 0,
                    search_selected: 0,
                    pwd: String::new(),
                    preedit_text: String::new(),
                    last_clipboard_text: String::new(),
                    copy_mode: None,
                    command_prompt: CommandPrompt::new(),
                    terminal_font_family: appearance.font_family,
                    terminal_font_size: appearance.font_size,
                    background_opacity: appearance.background_opacity,
                    background_opacity_cells: appearance.background_opacity_cells,
                    appearance_revision: 1,
                    app_focused: true,
                    desktop_notifications_enabled: boo_config.desktop_notifications,
                    notify_on_command_finish: boo_config.notify_on_command_finish,
                    notify_on_command_finish_action: boo_config.notify_on_command_finish_action,
                    notify_on_command_finish_after_ns: boo_config.notify_on_command_finish_after_ns,
                },
                Task::none(),
            )
        }
    }

    fn focused_surface(&self) -> ffi::ghostty_surface_t {
        self.tabs.focused_pane().surface()
    }

    fn set_pane_focus(&self, pane: PaneHandle, focused: bool) {
        self.backend.set_surface_focus(pane.surface(), focused);
        #[cfg(target_os = "macos")]
        if focused && pane.surface().is_null() {
            platform::focus_view(pane.view());
        }
    }

    fn handle_command_finished(&mut self, event: CommandFinishedEvent) {
        if let Some((title, body)) = command_finish_notification(
            self.desktop_notifications_enabled,
            self.notify_on_command_finish_action.notify,
            self.notify_on_command_finish,
            self.app_focused,
            self.notify_on_command_finish_after_ns,
            event,
        ) {
            platform::send_desktop_notification(title, &body);
        }
    }

    fn forward_text_input_command(&mut self, command: platform::TextInputCommand) {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            let (keycode, unshifted_codepoint) = text_input_command_key(command);
            let _ = self.backend.forward_vt_key(
                self.tabs.focused_pane(),
                vt::GHOSTTY_KEY_ACTION_PRESS,
                keycode,
                ffi::GHOSTTY_MODS_NONE as vt::GhosttyMods,
                ffi::GHOSTTY_MODS_NONE as vt::GhosttyMods,
                None,
                "",
                false,
                unshifted_codepoint,
            );
        }
    }

    fn dispatch_binding_result(&mut self, result: bindings::KeyResult) -> bool {
        match result {
            bindings::KeyResult::Consumed(action) => {
                if let Some(action) = action {
                    self.dispatch_binding_action(action);
                }
                true
            }
            bindings::KeyResult::CopyMode(action) => {
                self.dispatch_copy_mode_action(action);
                true
            }
            bindings::KeyResult::Forward => false,
        }
    }

    fn route_app_key(
        &mut self,
        key_char: Option<char>,
        keycode: u32,
        mods: i32,
        named_key: Option<bindings::NamedKey>,
        keyboard_key: keyboard::Key,
    ) -> bool {
        let text = key_char.map(|ch| ch.to_string());
        let iced_mods = ghostty_mods_to_iced(mods);

        if self.command_prompt.active {
            self.handle_command_key(&keyboard_key, &text, &iced_mods);
            return true;
        }

        if self.search_active {
            self.handle_search_key(&keyboard_key, &text, &iced_mods);
            return true;
        }

        let result = self.bindings.handle_key(key_char, keycode, mods, named_key);
        self.dispatch_binding_result(result)
    }

    fn handle_committed_text(&mut self, committed: String) {
        if self.command_prompt.active {
            let key = keyboard::Key::Character(committed.clone().into());
            self.handle_command_key(&key, &Some(committed), &keyboard::Modifiers::default());
            return;
        }

        if self.search_active {
            let key = keyboard::Key::Character(committed.clone().into());
            self.handle_search_key(&key, &Some(committed), &keyboard::Modifiers::default());
            return;
        }

        if self.bindings.is_prefix_mode() || self.bindings.is_copy_mode() {
            for ch in committed.chars() {
                let result = self.bindings.handle_key(Some(ch), 0, 0, None);
                let _ = self.dispatch_binding_result(result);
            }
            return;
        }

        let _ = self
            .backend
            .write_input(self.tabs.focused_pane(), committed.as_bytes());
    }

    #[cfg(target_os = "macos")]
    fn handle_platform_key_event(&mut self, event: platform::KeyEvent) {
        let keycode = event.keycode;
        let mods = event.mods;
        let key_char = shifted_char(keycode, mods);
        let named_key = native_keycode_to_named_key(keycode);
        let keyboard_key = native_keycode_to_keyboard_key(keycode, key_char);

        if self.route_app_key(key_char, keycode, mods, named_key, keyboard_key) {
            return;
        }

        let Some(vt_keycode) = keymap::native_to_vt_keycode(keycode) else {
            return;
        };
        let unshifted_codepoint = shifted_codepoint_vt(vt_keycode, 0);
        let _ = self.backend.forward_vt_key(
            self.tabs.focused_pane(),
            if event.repeat {
                vt::GHOSTTY_KEY_ACTION_REPEAT
            } else {
                vt::GHOSTTY_KEY_ACTION_PRESS
            },
            vt_keycode,
            mods as vt::GhosttyMods,
            (mods & ffi::GHOSTTY_MODS_SHIFT) as vt::GhosttyMods,
            key_char,
            "",
            false,
            unshifted_codepoint,
        );
    }

    fn resize_pane_backend(&mut self, pane: PaneHandle, scale: f64, width: u32, height: u32) {
        self.backend
            .resize_pane(pane, scale, width, height, self.cell_width, self.cell_height);
    }

    fn free_pane_backend(&mut self, pane: PaneHandle) {
        self.backend.free_pane(pane);
    }

    fn surface_key_translation_mods(
        &self,
        surface: ffi::ghostty_surface_t,
        mods: i32,
    ) -> i32 {
        self.backend.surface_key_translation_mods(surface, mods)
    }

    fn forward_surface_key(&mut self, event: ffi::ghostty_input_key_s) -> bool {
        self.backend.surface_key(self.tabs.focused_pane(), event)
    }

    fn forward_surface_mouse_pos(&mut self, x: f64, y: f64, mods: i32) {
        self.backend
            .surface_mouse_pos(self.tabs.focused_pane(), x, y, mods);
    }

    fn forward_surface_mouse_button(
        &mut self,
        state: ffi::ghostty_input_mouse_state_e,
        button: ffi::ghostty_input_mouse_button_e,
        mods: i32,
    ) {
        self.backend
            .surface_mouse_button(self.tabs.focused_pane(), state, button, mods);
    }

    fn forward_surface_mouse_scroll(&mut self, dx: f64, dy: f64, mods: i32) {
        self.backend
            .surface_mouse_scroll(self.tabs.focused_pane(), dx, dy, mods);
    }

    fn focused_cursor_cell_position(&self) -> Option<(u32, i64, f64)> {
        let scale = self.scale_factor();
        let cell_w_pts = self.cell_width / scale;
        let mut cell_h_pts = self.cell_height / scale;
        if self.focused_surface().is_null() {
            return self.backend.render_snapshot(self.tabs.focused_pane().id()).map(|snapshot| {
                (
                    snapshot.cursor.x as u32,
                    self.scrollbar.offset as i64 + snapshot.cursor.y as i64,
                    cell_h_pts,
                )
            });
        }
        let focused_pane = self.tabs.focused_pane();
        if let Some((x, y, _, h)) = self.backend.ime_point(focused_pane) {
            if h > 0.0 {
                cell_h_pts = h;
            }
            let col = if cell_w_pts > 0.0 {
                (x / cell_w_pts) as u32
            } else {
                0
            };
            let row = if cell_h_pts > 0.0 {
                ((y - cell_h_pts) / cell_h_pts) as i64
            } else {
                0
            };
            Some((col, row, cell_h_pts))
        } else {
            self.backend.render_snapshot(self.tabs.focused_pane().id()).map(|snapshot| {
                (
                    snapshot.cursor.x as u32,
                    self.scrollbar.offset as i64 + snapshot.cursor.y as i64,
                    cell_h_pts,
                )
            })
        }
    }

    fn update_text_input_cursor_rect(&self) {
        #[cfg(target_os = "macos")]
        {
            let rect = if self.focused_surface().is_null() {
                self.backend
                    .ime_point(self.tabs.focused_pane())
                    .map(|(x, y, w, h)| {
                        platform::Rect::new(
                            platform::Point::new(x, y - h),
                            platform::Size::new(w, h),
                        )
                    })
                    .unwrap_or_default()
            } else {
                platform::Rect::default()
            };
            platform::set_text_input_cursor_rect(rect);
        }
    }

    fn poll_backend(&mut self) {
        let active_id = self.tabs.focused_pane().id();
        let active_pane_ids: Vec<pane::PaneId> = self
            .tabs
            .active_tree()
            .map(|tree| tree.all_panes().into_iter().map(|pane| pane.id()).collect())
            .unwrap_or_default();
        let poll = self.backend.poll(
            &active_pane_ids,
            active_id,
            self.scrollbar.len,
            self.cell_width,
            self.cell_height,
        );
        for running_command in poll.running_commands.iter().cloned() {
            self.tabs.set_running_command_for_pane(
                running_command.pane_id,
                Some(tabs::RunningCommand {
                    command: running_command.command,
                }),
            );
        }
        for pane_id in &active_pane_ids {
            if !poll.running_commands.iter().any(|running| running.pane_id == *pane_id) {
                self.tabs.set_running_command_for_pane(*pane_id, None);
            }
        }
        for finished_command in poll.finished_commands {
            self.handle_command_finished(CommandFinishedEvent {
                exit_code: finished_command.exit_code,
                duration_ns: finished_command.duration_ns,
            });
        }
        if let Some(pwd) = poll.active_pwd {
            self.pwd = pwd;
        }
        if let Some(title) = poll.active_title {
            self.tabs.set_active_title(title);
        }
        if let Some(scrollbar) = poll.active_scrollbar {
            self.scrollbar = scrollbar;
        }
        for pane_id in poll.exited_panes {
            self.close_active_pane_by_id(pane_id);
        }
    }

    fn apply_appearance(&mut self, appearance: ResolvedAppearance) {
        self.terminal_font_family = appearance.font_family;
        self.terminal_font_size = appearance.font_size;
        self.background_opacity = appearance.background_opacity;
        self.background_opacity_cells = appearance.background_opacity_cells;
        let (cell_width, cell_height) = terminal_metrics(self.terminal_font_size);
        self.cell_width = cell_width;
        self.cell_height = cell_height;
        self.appearance_revision = self.appearance_revision.wrapping_add(1);
        #[cfg(target_os = "linux")]
        {
            self.pending_font_bytes = appearance.font_bytes;
        }
    }

    fn ui_font(&self) -> Font {
        configured_font(self.terminal_font_family)
    }

    fn panel_alpha(&self, base: f32) -> f32 {
        (base * self.background_opacity.max(0.3)).clamp(0.2, 0.98)
    }

    fn window_style(&self) -> iced::theme::Style {
        #[cfg(target_os = "linux")]
        {
            iced::theme::Style {
                background_color: Color::TRANSPARENT,
                text_color: Color::WHITE,
            }
        }

        #[cfg(not(target_os = "linux"))]
        {
            iced::theme::Style {
                background_color: Color::TRANSPARENT,
                text_color: Color::WHITE,
            }
        }
    }

    fn terminate(&self, code: i32) -> ! {
        control::cleanup(self.socket_path.as_deref());
        std::process::exit(code);
    }

    fn close_active_pane_by_id(&mut self, pane_id: pane::PaneId) {
        let Some(tree) = self.tabs.active_tree_mut() else {
            return;
        };
        let Some(leaf_id) = tree
            .export_panes()
            .into_iter()
            .find(|pane| pane.pane.id() == pane_id)
            .map(|pane| pane.leaf_id)
        else {
            return;
        };
        tree.set_focus(leaf_id);
        self.handle_surface_closed();
    }

    fn ui_snapshot(&self) -> control::UiSnapshot {
        let focused_pane = self.tabs.focused_pane();
        let terminal_frame = self.terminal_frame();
        let visible_panes = self
            .tabs
            .active_tree()
            .map(|tree| {
                tree.export_panes_with_frames(terminal_frame)
                    .into_iter()
                    .enumerate()
                    .map(|(leaf_index, pane)| control::UiPaneSnapshot {
                        leaf_index,
                        leaf_id: pane.leaf_id,
                        pane_id: pane.pane.id(),
                        focused: pane.pane.id() == focused_pane.id(),
                        frame: pane.frame.map_or(ui_rect_snapshot(0.0, 0.0, 0.0, 0.0), |frame| {
                            ui_rect_snapshot(
                                frame.origin.x,
                                frame.origin.y,
                                frame.size.width,
                                frame.size.height,
                            )
                        }),
                        split_direction: pane.split.map(|(direction, _)| split_direction_name(direction).to_string()),
                        split_ratio: pane.split.map(|(_, ratio)| ratio),
                    })
                    .collect()
            })
            .unwrap_or_default();

        let copy_mode_frame = terminal_frame;
        let copy_mode = self.copy_mode.as_ref().map_or(
            control::UiCopyModeSnapshot {
                active: false,
                cursor_row: 0,
                cursor_col: 0,
                selection_mode: "none".to_string(),
                has_selection_anchor: false,
                anchor_row: None,
                anchor_col: None,
                selection_rects: Vec::new(),
                show_position: false,
            },
            |copy_mode| {
                let selection_rects =
                    copy_mode
                        .sel_anchor
                        .map_or_else(Vec::new, |(anchor_row, anchor_col)| {
                            Self::compute_selection_rects_static(
                                copy_mode.selection,
                                copy_mode.cursor_row,
                                copy_mode.cursor_col,
                                anchor_row,
                                anchor_col,
                                self.scrollbar.offset as i64,
                                copy_mode.viewport_cols,
                                copy_mode.cell_width,
                                copy_mode.cell_height,
                                copy_mode_frame.origin.y,
                            )
                            .into_iter()
                            .map(|(x, y, width, height)| ui_rect_snapshot(x, y, width, height))
                            .collect()
                        });
                control::UiCopyModeSnapshot {
                    active: true,
                    cursor_row: copy_mode.cursor_row,
                    cursor_col: copy_mode.cursor_col,
                    selection_mode: selection_mode_name(copy_mode.selection).to_string(),
                    has_selection_anchor: copy_mode.sel_anchor.is_some(),
                    anchor_row: copy_mode.sel_anchor.map(|(row, _)| row),
                    anchor_col: copy_mode.sel_anchor.map(|(_, col)| col),
                    selection_rects,
                    show_position: copy_mode.show_position,
                }
            },
        );

        let tabs = self
            .tabs
            .tab_info()
            .into_iter()
            .map(|tab| control::UiTabSnapshot {
                index: tab.index,
                active: tab.active,
                title: tab.title,
                pane_count: tab.surfaces,
            })
            .collect();

        let command_prompt = control::UiCommandPromptSnapshot {
            active: self.command_prompt.active,
            input: self.command_prompt.input.clone(),
            selected_suggestion: self.command_prompt.selected_suggestion,
            suggestions: self
                .command_prompt
                .suggestions
                .iter()
                .filter_map(|&index| COMMANDS.get(index))
                .map(|command| command.name.to_string())
                .collect(),
        };

        let terminal = {
            self.backend.ui_terminal_snapshot(focused_pane.id())
        };

        control::UiSnapshot {
            active_tab: self.tabs.active_index(),
            focused_pane: focused_pane.id(),
            appearance: control::UiAppearanceSnapshot {
                font_family: self.terminal_font_family.map(str::to_string),
                font_size: self.terminal_font_size,
                background_opacity: self.background_opacity,
                background_opacity_cells: self.background_opacity_cells,
            },
            tabs,
            visible_panes,
            copy_mode,
            search: control::UiSearchSnapshot {
                active: self.search_active,
                query: self.search_query.clone(),
                total: self.search_total,
                selected: self.search_selected,
            },
            command_prompt,
            pwd: self.pwd.clone(),
            scrollbar: control::UiScrollbarSnapshot {
                total: self.scrollbar.total,
                offset: self.scrollbar.offset,
                len: self.scrollbar.len,
            },
            terminal,
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        #[cfg(target_os = "linux")]
        if let Some(bytes) = self.pending_font_bytes.take() {
            return iced::font::load(bytes).map(|_| Message::FontLoaded);
        }

        self.backend.tick();
        self.poll_backend();
        self.update_text_input_cursor_rect();

        // Process one control command per frame
        if let Ok(cmd) = self.ctl_rx.try_recv() {
            self.handle_control_cmd(cmd);
        }

        // Process native scroll events (with precision + momentum)
        while let Ok(scroll) = self.scroll_rx.try_recv() {
            let surface = self.focused_surface();
            if !surface.is_null() {
                // Build scroll mods: bit 0 = precision, bits 1-3 = momentum
                let mods = (scroll.precision as i32) | ((scroll.momentum as i32) << 1);
                if self.scrollbar.total > self.scrollbar.len {
                    self.scrollbar_opacity = 1.0;
                }
                self.forward_surface_mouse_scroll(scroll.dx, scroll.dy, mods);
            } else {
                #[cfg(any(target_os = "linux", target_os = "macos"))]
                {
                    if self.scrollbar.total > self.scrollbar.len {
                        self.scrollbar_opacity = 1.0;
                    }
                    let line_delta = if scroll.dy.abs() >= 1.0 {
                        -scroll.dy.round() as isize
                    } else if scroll.dy > 0.0 {
                        -1
                    } else if scroll.dy < 0.0 {
                        1
                    } else {
                        0
                    };
                    let _ = self
                        .backend
                        .scroll_viewport_delta(self.tabs.focused_pane(), line_delta);
                }
            }
        }

        #[cfg(target_os = "macos")]
        while let Ok(event) = self.key_event_rx.try_recv() {
            if self.focused_surface().is_null() {
                self.handle_platform_key_event(event);
            }
        }

        while let Ok(event) = self.text_input_rx.try_recv() {
            if !self.focused_surface().is_null() {
                continue;
            }
            match apply_text_input_event(&mut self.preedit_text, event) {
                Some(TextInputAction::Commit(committed)) => self.handle_committed_text(committed),
                Some(TextInputAction::Command(command)) => {
                    self.forward_text_input_command(command);
                }
                None => {}
            }
        }

        if self.tabs.is_empty() {
            self.init_surface();
            return Task::none();
        }

        let event = match message {
            Message::Frame => {
                // Fade out scrollbar (but not while dragging)
                if self.scrollbar_opacity > 0.0 && !self.scrollbar_drag {
                    self.scrollbar_opacity = (self.scrollbar_opacity - 0.008).max(0.0);
                }
                self.update_scrollbar_overlay();
                return Task::none();
            }
            #[cfg(target_os = "linux")]
            Message::FontLoaded => {
                self.appearance_revision = self.appearance_revision.wrapping_add(1);
                self.relayout();
                return Task::none();
            }
            Message::IcedEvent(event) => event,
        };

        match event {
            Event::Keyboard(kb_event) => self.handle_keyboard(kb_event),
            Event::Mouse(mouse_event) => self.handle_mouse(mouse_event),
            Event::Window(window::Event::Resized(size)) => {
                self.handle_resize(size);
            }
            Event::Window(window::Event::Focused) => {
                self.app_focused = true;
                self.set_pane_focus(self.tabs.focused_pane(), true);
                self.backend.set_app_focus(true);
            }
            Event::Window(window::Event::Unfocused) => {
                self.app_focused = false;
                self.set_pane_focus(self.tabs.focused_pane(), false);
                self.backend.set_app_focus(false);
            }
            _ => {}
        }

        Task::none()
    }

    fn handle_keyboard(&mut self, event: keyboard::Event) {
        match event {
            keyboard::Event::KeyPressed {
                key,
                modified_key,
                physical_key,
                modifiers,
                text,
                repeat,
                ..
            } => {
                // Skip modifier-only keys — they must not trigger or consume bindings
                if matches!(
                    physical_key,
                    keyboard::key::Physical::Code(
                        keyboard::key::Code::ShiftLeft
                            | keyboard::key::Code::ShiftRight
                            | keyboard::key::Code::ControlLeft
                            | keyboard::key::Code::ControlRight
                            | keyboard::key::Code::AltLeft
                            | keyboard::key::Code::AltRight
                            | keyboard::key::Code::SuperLeft
                            | keyboard::key::Code::SuperRight
                            | keyboard::key::Code::CapsLock
                    )
                ) {
                    return;
                }

                // Search mode — intercept all keys for the search input
                if self.command_prompt.active {
                    self.handle_command_key(&key, &text, &modifiers);
                    return;
                }

                if self.search_active {
                    self.handle_search_key(&key, &text, &modifiers);
                    return;
                }

                let Some(keycode) = keymap::physical_to_native_keycode(&physical_key) else {
                    return;
                };
                let mods = iced_mods_to_ghostty(&modifiers);

                // Get the character produced by this key for binding matching.
                // Compute from keycode+mods (same as control socket) because iced's
                // text/modified_key fields may not include shift effects when no
                // text input widget is focused.
                let key_char = shifted_char(keycode, mods)
                    .or_else(|| text.as_ref().and_then(|t| t.chars().next()))
                    .or_else(|| match &modified_key {
                        keyboard::Key::Character(s) => s.chars().next(),
                        _ => None,
                    });

                // Convert iced Named key to boo's NamedKey for copy mode
                let named_key = match &key {
                    keyboard::Key::Named(n) => {
                        use keyboard::key::Named;
                        match n {
                            Named::ArrowUp => Some(bindings::NamedKey::ArrowUp),
                            Named::ArrowDown => Some(bindings::NamedKey::ArrowDown),
                            Named::ArrowLeft => Some(bindings::NamedKey::ArrowLeft),
                            Named::ArrowRight => Some(bindings::NamedKey::ArrowRight),
                            Named::PageUp => Some(bindings::NamedKey::PageUp),
                            Named::PageDown => Some(bindings::NamedKey::PageDown),
                            Named::Home => Some(bindings::NamedKey::Home),
                            Named::End => Some(bindings::NamedKey::End),
                            Named::Escape => Some(bindings::NamedKey::Escape),
                            _ => None,
                        }
                    }
                    _ => None,
                };

                // If copy mode has a pending jump (f/F/t/T), consume next char as target
                if let Some(ref mut cm) = self.copy_mode {
                    if let Some(kind) = cm.pending_jump.take() {
                        if let Some(ch) = key_char {
                            cm.last_jump = Some((ch, kind));
                            self.copy_mode_execute_jump(ch, kind);
                        }
                        return;
                    }
                }

                // Check boo's own bindings first (prefix key system)
                match self.bindings.handle_key(key_char, keycode, mods, named_key) {
                    bindings::KeyResult::Consumed(action) => {
                        if self.dump_keys {
                            log::info!("boo binding: {action:?}");
                        }
                        if let Some(action) = action {
                            self.dispatch_binding_action(action);
                        }
                        return;
                    }
                    bindings::KeyResult::CopyMode(action) => {
                        self.dispatch_copy_mode_action(action);
                        return;
                    }
                    bindings::KeyResult::Forward => {}
                }

                // Forward to ghostty
                let action = if repeat {
                    ffi::ghostty_input_action_e::GHOSTTY_ACTION_REPEAT
                } else {
                    ffi::ghostty_input_action_e::GHOSTTY_ACTION_PRESS
                };

                // Apply option-as-alt translation
                let surface = self.focused_surface();
                let translation_mods = self.surface_key_translation_mods(surface, mods);

                // unshifted_codepoint: character with NO modifiers (matches macOS byApplyingModifiers:[])
                let unshifted_codepoint = key_to_codepoint(&key);

                // consumed_mods: from translation_mods, strip Ctrl and Cmd
                // (matching Swift: translationMods.subtracting([.control, .command]))
                let consumed_mods = translation_mods
                    & !(ffi::GHOSTTY_MODS_CTRL | ffi::GHOSTTY_MODS_SUPER);

                #[cfg(any(target_os = "linux", target_os = "macos"))]
                if surface.is_null() {
                    let Some(vt_keycode) = keymap::physical_to_vt_keycode(&physical_key) else {
                        return;
                    };
                    #[cfg(target_os = "macos")]
                    if should_route_macos_vt_key_via_appkit(vt_keycode, mods) {
                        return;
                    }
                    let _ = self.backend.forward_vt_key(
                        self.tabs.focused_pane(),
                        if repeat { vt::GHOSTTY_KEY_ACTION_REPEAT } else { vt::GHOSTTY_KEY_ACTION_PRESS },
                        vt_keycode,
                        mods as vt::GhosttyMods,
                        consumed_mods as vt::GhosttyMods,
                        key_char,
                        text.as_deref().unwrap_or(""),
                        false,
                        unshifted_codepoint,
                    );
                    return;
                }

                // text: the produced character. Filter control chars < 0x20 — ghostty handles Ctrl mapping
                let text_cstring = text
                    .as_ref()
                    .filter(|t| t.as_bytes().first().is_some_and(|&b| b >= 0x20))
                    .and_then(|t| CString::new(t.as_str()).ok());
                let text_ptr = text_cstring
                    .as_ref()
                    .map(|c| c.as_ptr())
                    .unwrap_or(ptr::null());

                let key_event = ffi::ghostty_input_key_s {
                    action,
                    mods,
                    consumed_mods,
                    keycode,
                    text: text_ptr,
                    unshifted_codepoint,
                    composing: false,
                };

                let consumed = self.forward_surface_key(key_event);
                if self.dump_keys {
                    log::info!(
                        "→ghostty: keycode=0x{keycode:02x} mods={mods:#x} cp={unshifted_codepoint:#x} text={:?} consumed={consumed}",
                        text.as_deref()
                    );
                }
            }
            keyboard::Event::KeyReleased {
                physical_key,
                modifiers,
                ..
            } => {
                let Some(keycode) = keymap::physical_to_native_keycode(&physical_key) else {
                    return;
                };
                #[cfg(any(target_os = "linux", target_os = "macos"))]
                if self.focused_surface().is_null() {
                    let Some(vt_keycode) = keymap::physical_to_vt_keycode(&physical_key) else {
                        return;
                    };
                    #[cfg(target_os = "macos")]
                    if should_route_macos_vt_key_via_appkit(
                        vt_keycode,
                        iced_mods_to_ghostty(&modifiers),
                    ) && self.preedit_text.is_empty() {
                        return;
                    }
                    let _ = self.backend.forward_vt_key(
                        self.tabs.focused_pane(),
                        vt::GHOSTTY_KEY_ACTION_RELEASE,
                        vt_keycode,
                        iced_mods_to_ghostty(&modifiers) as vt::GhosttyMods,
                        0,
                        None,
                        "",
                        false,
                        0,
                    );
                    return;
                }
                let key_event = ffi::ghostty_input_key_s {
                    action: ffi::ghostty_input_action_e::GHOSTTY_ACTION_RELEASE,
                    mods: iced_mods_to_ghostty(&modifiers),
                    consumed_mods: ffi::GHOSTTY_MODS_NONE,
                    keycode,
                    text: ptr::null(),
                    unshifted_codepoint: 0,
                    composing: false,
                };
                let _ = self.forward_surface_key(key_event);
            }
            _ => {}
        }
    }

    fn handle_control_cmd(&mut self, cmd: control::ControlCmd) {
        match cmd {
            control::ControlCmd::DumpKeysOn => self.dump_keys = true,
            control::ControlCmd::DumpKeysOff => self.dump_keys = false,
            control::ControlCmd::Quit => self.terminate(0),
            control::ControlCmd::ListSurfaces { reply } => {
                let info = if let Some(tree) = self.tabs.active_tree() {
                    tree.surface_info()
                        .into_iter()
                        .map(|(id, focused)| control::SurfaceInfo { index: id, focused })
                        .collect()
                } else {
                    Vec::new()
                };
                let _ = reply.send(control::Response::Surfaces { surfaces: info });
            }
            control::ControlCmd::NewSplit { direction } => {
                self.create_split(Self::split_direction_from_str(&direction));
            }
            control::ControlCmd::FocusSurface { index } => {
                let old = self.tabs.focused_pane();
                if let Some(tree) = self.tabs.active_tree_mut() {
                    tree.set_focus(index);
                }
                let new = self.tabs.focused_pane();
                if old != new {
                    self.set_pane_focus(old, false);
                    self.set_pane_focus(new, true);
                }
            }
            control::ControlCmd::ListTabs { reply } => {
                let _ = reply.send(control::Response::Tabs {
                    tabs: self.tabs.tab_info(),
                });
            }
            control::ControlCmd::GetClipboard { reply } => {
                let _ = reply.send(control::Response::Clipboard {
                    text: if self.last_clipboard_text.is_empty() {
                        platform::clipboard_read().unwrap_or_default()
                    } else {
                        self.last_clipboard_text.clone()
                    },
                });
            }
            control::ControlCmd::GetUiSnapshot { reply } => {
                let _ = reply.send(control::Response::UiSnapshot {
                    snapshot: self.ui_snapshot(),
                });
            }
            control::ControlCmd::ExecuteCommand { input } => {
                self.execute_command(&input);
            }
            control::ControlCmd::SendText { text } => {
                #[cfg(any(target_os = "linux", target_os = "macos"))]
                {
                    let _ = self.backend.write_input(self.tabs.focused_pane(), text.as_bytes());
                }
            }
            control::ControlCmd::SendVt { text } => {
                #[cfg(any(target_os = "linux", target_os = "macos"))]
                {
                    self.backend
                        .write_vt_bytes(self.tabs.focused_pane(), text.as_bytes());
                }
            }
            control::ControlCmd::NewTab => self.new_tab(),
            control::ControlCmd::GotoTab { index } => {
                self.tabs.goto_tab(index);
                self.sync_after_tab_change();
            }
            control::ControlCmd::NextTab => {
                self.tabs.next_tab();
                self.sync_after_tab_change();
            }
            control::ControlCmd::PrevTab => {
                self.tabs.prev_tab();
                self.sync_after_tab_change();
            }
            control::ControlCmd::SendKey { keyspec } => {
                self.inject_key(&keyspec);
            }
            control::ControlCmd::SendNativeKey { keycode, mods, repeat } => {
                #[cfg(target_os = "macos")]
                self.handle_platform_key_event(platform::KeyEvent { keycode, mods, repeat });
                #[cfg(not(target_os = "macos"))]
                let _ = (keycode, mods, repeat);
            }
        }
    }

    fn inject_key(&mut self, keyspec: &str) {
        let (keycode, mods) = match parse_keyspec(keyspec) {
            Some(v) => v,
            None => {
                log::warn!("unknown keyspec: {keyspec}");
                return;
            }
        };

        let key_char = shifted_char(keycode, mods);

        if self.command_prompt.active {
            let key = control_key_to_keyboard_key(keyspec, key_char);
            let text = key_char.map(|ch| ch.to_string());
            let modifiers = ghostty_mods_to_iced(mods);
            self.handle_command_key(&key, &text, &modifiers);
            return;
        }

        if self.search_active {
            let key = control_key_to_keyboard_key(keyspec, key_char);
            let text = key_char.map(|ch| ch.to_string());
            let modifiers = ghostty_mods_to_iced(mods);
            self.handle_search_key(&key, &text, &modifiers);
            return;
        }

        // Route through boo's binding system first (prefix key, etc.)
        let result = self.bindings.handle_key(key_char, keycode, mods, None);
        if self.dispatch_binding_result(result) {
            return;
        }

        let text_str = if key_char.is_some() && mods & ffi::GHOSTTY_MODS_CTRL == 0 {
            key_char.map(|c| c.to_string())
        } else {
            None
        };
        let unshifted = shifted_codepoint(keycode, 0);

        // Forward to ghostty
        let surface = self.focused_surface();
        if surface.is_null() {
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            {
                let Some((vt_keycode, _)) = parse_vt_keyspec(keyspec) else {
                    log::warn!("unknown VT keyspec: {keyspec}");
                    return;
                };
                let consumed_mods = if mods & ffi::GHOSTTY_MODS_SHIFT != 0 {
                    ffi::GHOSTTY_MODS_SHIFT
                } else {
                    ffi::GHOSTTY_MODS_NONE
                };
                let _ = self.backend.forward_vt_key(
                    self.tabs.focused_pane(),
                    vt::GHOSTTY_KEY_ACTION_PRESS,
                    vt_keycode,
                    mods as vt::GhosttyMods,
                    consumed_mods as vt::GhosttyMods,
                    key_char,
                    text_str.as_deref().unwrap_or(""),
                    false,
                    shifted_codepoint_vt(vt_keycode, 0),
                );
            }
            return;
        }
        let ctext = text_str.as_ref().and_then(|t| CString::new(t.as_str()).ok());
        let text_ptr = ctext.as_ref().map(|c| c.as_ptr()).unwrap_or(ptr::null());
        let consumed_mods = if mods & ffi::GHOSTTY_MODS_SHIFT != 0 {
            ffi::GHOSTTY_MODS_SHIFT
        } else {
            ffi::GHOSTTY_MODS_NONE
        };
        let ev = ffi::ghostty_input_key_s {
            action: ffi::ghostty_input_action_e::GHOSTTY_ACTION_PRESS,
            mods,
            consumed_mods,
            keycode,
            text: text_ptr,
            unshifted_codepoint: unshifted,
            composing: false,
        };
        let consumed = self.forward_surface_key(ev);
        if self.dump_keys {
            log::info!("ctl key: keycode=0x{keycode:02x} mods={mods:#x} cp=0x{unshifted:02x} text={text_str:?} consumed={consumed}");
        }
    }

    fn dispatch_copy_mode_action(&mut self, action: bindings::CopyModeAction) {
        use bindings::CopyModeAction::*;

        // If we're waiting for a jump target character, consume it
        if let Some(_kind) = self.copy_mode.as_ref().and_then(|cm| cm.pending_jump) {
            if let JumpForward | JumpBackward | JumpToForward | JumpToBackward = action {
                // These are the jump initiators — they set pending_jump below
            } else {
                // Any other action cancels the pending jump
                if let Some(ref mut cm) = self.copy_mode {
                    cm.pending_jump = None;
                }
            }
        }

        match action {
            Move(dir) => self.copy_mode_move(dir),

            // Word movement
            WordNext => self.copy_mode_word_move(WordMoveKind::NextWord),
            WordBack => self.copy_mode_word_move(WordMoveKind::PrevWord),
            WordEnd => self.copy_mode_word_move(WordMoveKind::EndWord),
            BigWordNext => self.copy_mode_word_move(WordMoveKind::NextBigWord),
            BigWordBack => self.copy_mode_word_move(WordMoveKind::PrevBigWord),
            BigWordEnd => self.copy_mode_word_move(WordMoveKind::EndBigWord),

            // Line position
            LineStart => {
                if let Some(ref mut cm) = self.copy_mode {
                    cm.cursor_col = 0;
                }
                self.update_copy_mode_highlight();
            }
            LineEnd => {
                if let Some(ref mut cm) = self.copy_mode {
                    cm.cursor_col = cm.viewport_cols.saturating_sub(1);
                }
                self.update_copy_mode_highlight();
            }
            FirstNonBlank => self.copy_mode_first_non_blank(),

            // Screen position
            ScreenTop => {
                if let Some(ref mut cm) = self.copy_mode {
                    cm.cursor_row = self.scrollbar.offset as i64;
                    cm.cursor_col = 0;
                }
                self.update_copy_mode_highlight();
            }
            ScreenMiddle => {
                if let Some(ref mut cm) = self.copy_mode {
                    cm.cursor_row = self.scrollbar.offset as i64 + cm.viewport_rows as i64 / 2;
                    cm.cursor_col = 0;
                }
                self.update_copy_mode_highlight();
            }
            ScreenBottom => {
                if let Some(ref mut cm) = self.copy_mode {
                    cm.cursor_row = self.scrollbar.offset as i64 + cm.viewport_rows as i64 - 1;
                    cm.cursor_col = 0;
                }
                self.update_copy_mode_highlight();
            }

            // Scrollback
            HistoryTop => {
                if let Some(ref mut cm) = self.copy_mode {
                    cm.cursor_row = 0;
                    cm.cursor_col = 0;
                }
                self.ghostty_binding_action("scroll_to_top");
                self.update_copy_mode_highlight();
            }
            HistoryBottom => {
                if let Some(ref mut cm) = self.copy_mode {
                    cm.cursor_row = self.scrollbar.total as i64;
                    cm.cursor_col = 0;
                }
                self.ghostty_binding_action("scroll_to_bottom");
                self.update_copy_mode_highlight();
            }

            // Page/scroll
            PageUp => {
                if let Some(ref mut cm) = self.copy_mode {
                    cm.cursor_row = cm.cursor_row.saturating_sub(cm.viewport_rows as i64);
                }
                self.ghostty_binding_action("scroll_page_up");
                self.update_copy_mode_highlight();
            }
            PageDown => {
                if let Some(ref mut cm) = self.copy_mode {
                    cm.cursor_row += cm.viewport_rows as i64;
                }
                self.ghostty_binding_action("scroll_page_down");
                self.update_copy_mode_highlight();
            }
            HalfPageUp => {
                if let Some(ref mut cm) = self.copy_mode {
                    let half = (cm.viewport_rows / 2) as i64;
                    cm.cursor_row = cm.cursor_row.saturating_sub(half);
                }
                self.copy_mode_ensure_visible();
                self.update_copy_mode_highlight();
            }
            HalfPageDown => {
                if let Some(ref mut cm) = self.copy_mode {
                    let half = (cm.viewport_rows / 2) as i64;
                    cm.cursor_row += half;
                }
                self.copy_mode_ensure_visible();
                self.update_copy_mode_highlight();
            }
            ScrollUp => {
                self.ghostty_binding_action("scroll_page_lines:-1");
                self.update_copy_mode_highlight();
            }
            ScrollDown => {
                self.ghostty_binding_action("scroll_page_lines:1");
                self.update_copy_mode_highlight();
            }
            ScrollMiddle => {
                // Scroll so cursor is at middle of screen
                if let Some(ref cm) = self.copy_mode {
                    let target_offset = cm.cursor_row - cm.viewport_rows as i64 / 2;
                    let target_offset = target_offset.max(0) as usize;
                    let current = self.scrollbar.offset;
                    let diff = target_offset as i64 - current as i64;
                    if diff != 0 {
                        let cmd = format!("scroll_page_lines:{diff}");
                        self.ghostty_binding_action(&cmd);
                    }
                }
                self.update_copy_mode_highlight();
            }

            // Selection
            StartCharSelect => {
                if let Some(ref mut cm) = self.copy_mode {
                    if cm.selection == SelectionMode::Char {
                        cm.selection = SelectionMode::None;
                        cm.sel_anchor = None;
                    } else {
                        cm.selection = SelectionMode::Char;
                        cm.sel_anchor = Some((cm.cursor_row, cm.cursor_col));
                    }
                }
                self.update_copy_mode_highlight();
            }
            StartLineSelect => {
                if let Some(ref mut cm) = self.copy_mode {
                    if cm.selection == SelectionMode::Line {
                        cm.selection = SelectionMode::None;
                        cm.sel_anchor = None;
                    } else {
                        cm.selection = SelectionMode::Line;
                        cm.sel_anchor = Some((cm.cursor_row, cm.cursor_col));
                    }
                }
                self.update_copy_mode_highlight();
            }
            StartRectSelect => {
                if let Some(ref mut cm) = self.copy_mode {
                    if cm.selection == SelectionMode::Rectangle {
                        cm.selection = SelectionMode::None;
                        cm.sel_anchor = None;
                    } else {
                        cm.selection = SelectionMode::Rectangle;
                        if cm.sel_anchor.is_none() {
                            cm.sel_anchor = Some((cm.cursor_row, cm.cursor_col));
                        }
                    }
                }
                self.update_copy_mode_highlight();
            }
            ClearSelection => {
                if let Some(ref mut cm) = self.copy_mode {
                    if cm.selection != SelectionMode::None {
                        cm.selection = SelectionMode::None;
                        cm.sel_anchor = None;
                        self.update_copy_mode_highlight();
                    } else {
                        self.exit_copy_mode();
                    }
                }
            }
            SwapAnchor => {
                if let Some(ref mut cm) = self.copy_mode {
                    if let Some((ar, ac)) = cm.sel_anchor {
                        let (cr, cc) = (cm.cursor_row, cm.cursor_col);
                        cm.cursor_row = ar;
                        cm.cursor_col = ac;
                        cm.sel_anchor = Some((cr, cc));
                    }
                }
                self.copy_mode_ensure_visible();
                self.update_copy_mode_highlight();
            }

            // In-line jump — set pending state, next keypress will be the target
            JumpForward => {
                if let Some(ref mut cm) = self.copy_mode {
                    cm.pending_jump = Some(JumpKind::Forward);
                }
            }
            JumpBackward => {
                if let Some(ref mut cm) = self.copy_mode {
                    cm.pending_jump = Some(JumpKind::Backward);
                }
            }
            JumpToForward => {
                if let Some(ref mut cm) = self.copy_mode {
                    cm.pending_jump = Some(JumpKind::ToForward);
                }
            }
            JumpToBackward => {
                if let Some(ref mut cm) = self.copy_mode {
                    cm.pending_jump = Some(JumpKind::ToBackward);
                }
            }
            JumpAgain => self.copy_mode_jump_repeat(false),
            JumpReverse => self.copy_mode_jump_repeat(true),

            // Paragraph/bracket
            NextParagraph => self.copy_mode_paragraph(true),
            PreviousParagraph => self.copy_mode_paragraph(false),
            MatchingBracket => self.copy_mode_matching_bracket(),

            // Marks
            SetMark => {
                if let Some(ref mut cm) = self.copy_mode {
                    cm.mark = Some((cm.cursor_row, cm.cursor_col));
                }
            }
            JumpToMark => {
                if let Some(ref mut cm) = self.copy_mode {
                    if let Some((r, c)) = cm.mark {
                        cm.cursor_row = r;
                        cm.cursor_col = c;
                    }
                }
                self.copy_mode_ensure_visible();
                self.update_copy_mode_highlight();
            }

            // Search
            SearchForward => {
                if let Some(ref mut cm) = self.copy_mode {
                    cm.last_search_forward = true;
                }
                self.search_active = true;
                self.search_query.clear();
                self.search_total = 0;
                self.search_selected = 0;
                self.relayout();
            }
            SearchBackward => {
                if let Some(ref mut cm) = self.copy_mode {
                    cm.last_search_forward = false;
                }
                self.search_active = true;
                self.search_query.clear();
                self.search_total = 0;
                self.search_selected = 0;
                self.relayout();
            }
            SearchAgain => {
                let forward = self.copy_mode.as_ref().map_or(true, |cm| cm.last_search_forward);
                if forward {
                    self.ghostty_binding_action("navigate_search:next");
                } else {
                    self.ghostty_binding_action("navigate_search:previous");
                }
            }
            SearchReverse => {
                let forward = self.copy_mode.as_ref().map_or(true, |cm| cm.last_search_forward);
                if forward {
                    self.ghostty_binding_action("navigate_search:previous");
                } else {
                    self.ghostty_binding_action("navigate_search:next");
                }
            }
            SearchWordForward => {
                if let Some(word) = self.copy_mode_word_under_cursor() {
                    if let Some(ref mut cm) = self.copy_mode {
                        cm.last_search_forward = true;
                    }
                    self.search_active = true;
                    self.search_query = word;
                    self.relayout();
                    self.ghostty_binding_action("navigate_search:next");
                }
            }
            SearchWordBackward => {
                if let Some(word) = self.copy_mode_word_under_cursor() {
                    if let Some(ref mut cm) = self.copy_mode {
                        cm.last_search_forward = false;
                    }
                    self.search_active = true;
                    self.search_query = word;
                    self.relayout();
                    self.ghostty_binding_action("navigate_search:previous");
                }
            }

            // Copy
            CopyAndExit => {
                self.copy_mode_copy();
                self.exit_copy_mode();
            }
            CopyToEndOfLine => {
                // Select from cursor to end of line, copy, exit
                if let Some(ref mut cm) = self.copy_mode {
                    cm.selection = SelectionMode::Char;
                    cm.sel_anchor = Some((cm.cursor_row, cm.cursor_col));
                    cm.cursor_col = cm.viewport_cols.saturating_sub(1);
                }
                self.copy_mode_copy();
                self.exit_copy_mode();
            }
            AppendAndCancel => {
                self.copy_mode_append_copy();
                self.exit_copy_mode();
            }

            // Other
            OpenPrompt => {
                self.command_prompt.active = true;
                self.command_prompt.input.clear();
                self.command_prompt.selected_suggestion = 0;
                self.command_prompt.history_idx = None;
                self.command_prompt.update_suggestions();
            }
            RefreshFromPane => {
                // Re-read terminal state — for boo this is effectively a no-op since
                // we read from ghostty's live buffer on every operation
                self.update_copy_mode_highlight();
            }
            TogglePosition => {
                if let Some(ref mut cm) = self.copy_mode {
                    cm.show_position = !cm.show_position;
                }
            }
            Exit => self.exit_copy_mode(),
        }
    }

    fn copy_mode_move(&mut self, dir: bindings::Direction) {
        let Some(ref mut cm) = self.copy_mode else { return };
        match dir {
            bindings::Direction::Up => cm.cursor_row -= 1,
            bindings::Direction::Down => cm.cursor_row += 1,
            bindings::Direction::Left => {
                if cm.cursor_col > 0 {
                    cm.cursor_col -= 1;
                }
            }
            bindings::Direction::Right => {
                cm.cursor_col = (cm.cursor_col + 1).min(cm.viewport_cols.saturating_sub(1));
            }
        }
        self.copy_mode_ensure_visible();
        self.update_copy_mode_highlight();
    }

    fn copy_mode_ensure_visible(&mut self) {
        let Some(ref cm) = self.copy_mode else { return };
        let viewport_row = cm.cursor_row - self.scrollbar.offset as i64;
        if viewport_row < 0 {
            let lines = -viewport_row;
            let cmd = format!("scroll_page_lines:-{lines}");
            self.ghostty_binding_action(&cmd);
        } else if viewport_row >= cm.viewport_rows as i64 {
            let lines = viewport_row - cm.viewport_rows as i64 + 1;
            let cmd = format!("scroll_page_lines:{lines}");
            self.ghostty_binding_action(&cmd);
        }
    }

    fn copy_mode_first_non_blank(&mut self) {
        if let Some(line) = self.read_viewport_line_for_cursor() {
            let col = line.chars().position(|c| !c.is_whitespace()).unwrap_or(0);
            if let Some(ref mut cm) = self.copy_mode {
                cm.cursor_col = col as u32;
            }
        }
        self.update_copy_mode_highlight();
    }

    fn read_viewport_line_for_cursor(&self) -> Option<String> {
        let cm = self.copy_mode.as_ref()?;
        let viewport_row = (cm.cursor_row - self.scrollbar.offset as i64).max(0) as u32;
        self.read_viewport_line(viewport_row)
    }

    fn read_viewport_line(&self, viewport_row: u32) -> Option<String> {
        let cm = self.copy_mode.as_ref()?;
        let sel = ffi::ghostty_selection_s {
            top_left: ffi::ghostty_point_s {
                tag: ffi::GHOSTTY_POINT_VIEWPORT,
                coord: ffi::GHOSTTY_POINT_COORD_EXACT,
                x: 0,
                y: viewport_row,
            },
            bottom_right: ffi::ghostty_point_s {
                tag: ffi::GHOSTTY_POINT_VIEWPORT,
                coord: ffi::GHOSTTY_POINT_COORD_EXACT,
                x: cm.viewport_cols.saturating_sub(1),
                y: viewport_row,
            },
            rectangle: false,
        };
        self.read_surface_selection_text(sel)
    }

    fn copy_mode_word_move(&mut self, kind: WordMoveKind) {
        let Some(line) = self.read_viewport_line_for_cursor() else { return };
        let Some(ref mut cm) = self.copy_mode else { return };
        let chars: Vec<char> = line.chars().collect();
        let col = cm.cursor_col as usize;
        let len = chars.len();

        let is_word = |c: char, big: bool| -> bool {
            if big { !c.is_whitespace() } else { c.is_alphanumeric() || c == '_' }
        };
        let is_sep = |c: char| -> bool { !c.is_alphanumeric() && c != '_' && !c.is_whitespace() };

        let new_col = match kind {
            WordMoveKind::NextWord | WordMoveKind::NextBigWord => {
                let big = matches!(kind, WordMoveKind::NextBigWord);
                let mut i = col;
                // Skip current word/punct chars
                if i < len && is_word(chars[i], big) {
                    while i < len && is_word(chars[i], big) { i += 1; }
                } else if !big && i < len && is_sep(chars[i]) {
                    while i < len && is_sep(chars[i]) { i += 1; }
                } else {
                    i += 1;
                }
                // Skip whitespace
                while i < len && chars[i].is_whitespace() { i += 1; }
                if i >= len { col } else { i }
            }
            WordMoveKind::PrevWord | WordMoveKind::PrevBigWord => {
                let big = matches!(kind, WordMoveKind::PrevBigWord);
                if col == 0 { 0 } else {
                    let mut i = col - 1;
                    // Skip whitespace backwards
                    while i > 0 && chars[i].is_whitespace() { i -= 1; }
                    // Skip word/punct chars backwards
                    if is_word(chars[i], big) {
                        while i > 0 && is_word(chars[i - 1], big) { i -= 1; }
                    } else if !big && is_sep(chars[i]) {
                        while i > 0 && is_sep(chars[i - 1]) { i -= 1; }
                    }
                    i
                }
            }
            WordMoveKind::EndWord | WordMoveKind::EndBigWord => {
                let big = matches!(kind, WordMoveKind::EndBigWord);
                if col + 1 >= len { col } else {
                    let mut i = col + 1;
                    // Skip whitespace
                    while i < len && chars[i].is_whitespace() { i += 1; }
                    // Advance through word/punct chars
                    if i < len && is_word(chars[i], big) {
                        while i + 1 < len && is_word(chars[i + 1], big) { i += 1; }
                    } else if !big && i < len && is_sep(chars[i]) {
                        while i + 1 < len && is_sep(chars[i + 1]) { i += 1; }
                    }
                    i
                }
            }
        };

        cm.cursor_col = new_col as u32;
        self.copy_mode_ensure_visible();
        self.update_copy_mode_highlight();
    }

    fn copy_mode_execute_jump(&mut self, target: char, kind: JumpKind) {
        let Some(line) = self.read_viewport_line_for_cursor() else { return };
        let Some(ref mut cm) = self.copy_mode else { return };
        let col = cm.cursor_col as usize;
        let chars: Vec<char> = line.chars().collect();

        let new_col = match kind {
            JumpKind::Forward => {
                chars.iter().enumerate().skip(col + 1).find(|(_, c)| **c == target).map(|(i, _)| i)
            }
            JumpKind::Backward => {
                chars.iter().enumerate().take(col).rev().find(|(_, c)| **c == target).map(|(i, _)| i)
            }
            JumpKind::ToForward => {
                chars.iter().enumerate().skip(col + 1).find(|(_, c)| **c == target).map(|(i, _)| i.saturating_sub(1).max(col + 1))
            }
            JumpKind::ToBackward => {
                chars.iter().enumerate().take(col).rev().find(|(_, c)| **c == target).map(|(i, _)| (i + 1).min(col.saturating_sub(1)))
            }
        };

        if let Some(nc) = new_col {
            cm.cursor_col = nc as u32;
        }
        self.update_copy_mode_highlight();
    }

    fn copy_mode_jump_repeat(&mut self, reverse: bool) {
        let Some(ref cm) = self.copy_mode else { return };
        let Some((target, kind)) = cm.last_jump else { return };
        let kind = if reverse {
            match kind {
                JumpKind::Forward => JumpKind::Backward,
                JumpKind::Backward => JumpKind::Forward,
                JumpKind::ToForward => JumpKind::ToBackward,
                JumpKind::ToBackward => JumpKind::ToForward,
            }
        } else {
            kind
        };
        self.copy_mode_execute_jump(target, kind);
    }

    fn copy_mode_paragraph(&mut self, forward: bool) {
        let Some(ref mut cm) = self.copy_mode else { return };
        let offset = self.scrollbar.offset as i64;
        let max_row = self.scrollbar.total as i64;

        if forward {
            let mut r = cm.cursor_row + 1;
            while r <= max_row {
                let vp = (r - offset).max(0) as u32;
                if let Some(line) = self.read_viewport_line(vp) {
                    if line.trim().is_empty() {
                        if let Some(ref mut cm) = self.copy_mode {
                            cm.cursor_row = r;
                            cm.cursor_col = 0;
                        }
                        break;
                    }
                } else {
                    break;
                }
                r += 1;
            }
        } else {
            let mut r = cm.cursor_row - 1;
            while r >= 0 {
                let vp = (r - offset).max(0) as u32;
                if let Some(line) = self.read_viewport_line(vp) {
                    if line.trim().is_empty() {
                        if let Some(ref mut cm) = self.copy_mode {
                            cm.cursor_row = r;
                            cm.cursor_col = 0;
                        }
                        break;
                    }
                } else {
                    break;
                }
                r -= 1;
            }
        }
        self.copy_mode_ensure_visible();
        self.update_copy_mode_highlight();
    }

    fn copy_mode_matching_bracket(&mut self) {
        let Some(line) = self.read_viewport_line_for_cursor() else { return };
        let Some(ref mut cm) = self.copy_mode else { return };
        let chars: Vec<char> = line.chars().collect();
        let col = cm.cursor_col as usize;

        // Find bracket at or after cursor
        let brackets = [('(', ')'), ('[', ']'), ('{', '}')];
        let mut found = None;
        for i in col..chars.len() {
            for &(open, close) in &brackets {
                if chars[i] == open {
                    found = Some((i, open, close, true));
                    break;
                } else if chars[i] == close {
                    found = Some((i, open, close, false));
                    break;
                }
            }
            if found.is_some() { break; }
        }

        let Some((pos, open, close, is_open)) = found else { return };
        // Simple single-line bracket matching
        let mut depth = 0i32;
        if is_open {
            for i in pos..chars.len() {
                if chars[i] == open { depth += 1; }
                if chars[i] == close { depth -= 1; }
                if depth == 0 {
                    cm.cursor_col = i as u32;
                    break;
                }
            }
        } else {
            for i in (0..=pos).rev() {
                if chars[i] == close { depth += 1; }
                if chars[i] == open { depth -= 1; }
                if depth == 0 {
                    cm.cursor_col = i as u32;
                    break;
                }
            }
        }
        self.update_copy_mode_highlight();
    }

    fn copy_mode_word_under_cursor(&self) -> Option<String> {
        let line = self.read_viewport_line_for_cursor()?;
        let cm = self.copy_mode.as_ref()?;
        let chars: Vec<char> = line.chars().collect();
        let col = cm.cursor_col as usize;
        if col >= chars.len() { return None; }

        let is_word = |c: char| c.is_alphanumeric() || c == '_';
        if !is_word(chars[col]) { return None; }

        let mut start = col;
        while start > 0 && is_word(chars[start - 1]) { start -= 1; }
        let mut end = col;
        while end + 1 < chars.len() && is_word(chars[end + 1]) { end += 1; }

        Some(chars[start..=end].iter().collect())
    }

    fn copy_mode_append_copy(&mut self) {
        let existing = if self.last_clipboard_text.is_empty() {
            platform::clipboard_read().unwrap_or_default()
        } else {
            self.last_clipboard_text.clone()
        };

        let Some(ref cm) = self.copy_mode else { return };
        let Some((anchor_row, anchor_col)) = cm.sel_anchor else { return };

        let (r1, c1, r2, c2) = if anchor_row < cm.cursor_row
            || (anchor_row == cm.cursor_row && anchor_col <= cm.cursor_col)
        {
            (anchor_row, anchor_col, cm.cursor_row, cm.cursor_col)
        } else {
            (cm.cursor_row, cm.cursor_col, anchor_row, anchor_col)
        };
        let (c1, c2) = if cm.selection == SelectionMode::Line {
            (0u32, cm.viewport_cols.saturating_sub(1))
        } else { (c1, c2) };

        let sel = ffi::ghostty_selection_s {
            top_left: ffi::ghostty_point_s {
                tag: ffi::GHOSTTY_POINT_VIEWPORT,
                coord: ffi::GHOSTTY_POINT_COORD_EXACT,
                x: c1,
                y: (r1 - self.scrollbar.offset as i64).max(0) as u32,
            },
            bottom_right: ffi::ghostty_point_s {
                tag: ffi::GHOSTTY_POINT_VIEWPORT,
                coord: ffi::GHOSTTY_POINT_COORD_EXACT,
                x: c2,
                y: (r2 - self.scrollbar.offset as i64).max(0) as u32,
            },
            rectangle: cm.selection == SelectionMode::Rectangle,
        };
        if let Some(new_text) = self.read_surface_selection_text(sel) {
            let combined = format!("{existing}{new_text}");
            platform::clipboard_write(&combined);
            self.last_clipboard_text = combined;
            log::info!("copy mode: appended {} bytes to clipboard", new_text.len());
        }
    }

    fn enter_copy_mode(&mut self) {
        let scale = self.scale_factor();
        let cell_w_pts = self.cell_width / scale;
        let Some((col, row, cell_h_pts)) = self.focused_cursor_cell_position() else {
            return;
        };

        let viewport_rows = if cell_h_pts > 0.0 {
            ((self.last_size.height as f64 - STATUS_BAR_HEIGHT) / cell_h_pts) as u32
        } else {
            24
        };

        let frame = self.terminal_frame();
        let viewport_cols = if cell_w_pts > 0.0 {
            (frame.size.width / cell_w_pts) as u32
        } else {
            80
        };

        let cursor_layer = platform::create_highlight_layer();
        self.copy_mode = Some(CopyModeState {
            cursor_row: self.scrollbar.offset as i64 + row,
            cursor_col: col,
            selection: SelectionMode::None,
            sel_anchor: None,
            highlight_layers: Vec::new(),
            cursor_layer,
            cell_width: cell_w_pts,
            cell_height: cell_h_pts,
            viewport_rows,
            viewport_cols,
            mark: None,
            last_jump: None,
            last_search_forward: true,
            pending_jump: None,
            show_position: false,
        });
        self.bindings.enter_copy_mode();
        self.update_copy_mode_highlight();
    }

    fn exit_copy_mode(&mut self) {
        if let Some(cm) = self.copy_mode.take() {
            platform::update_highlight_layer(cm.cursor_layer, 0.0, 0.0, 0.0, 0.0, false, false);
            for layer in &cm.highlight_layers {
                platform::update_highlight_layer(*layer, 0.0, 0.0, 0.0, 0.0, false, false);
            }
        }
        self.bindings.exit_copy_mode();
        self.ghostty_binding_action("scroll_to_bottom");
        self.ghostty_binding_action("end_search");
        self.search_active = false;
    }

    fn update_copy_mode_highlight(&mut self) {
        let Some(ref cm) = self.copy_mode else { return };

        let frame = self.terminal_frame();
        let term_y = frame.origin.y;
        let offset = self.scrollbar.offset as i64;
        let viewport_row = cm.cursor_row - offset;
        let px = cm.cursor_col as f64 * cm.cell_width;
        let py = term_y + viewport_row as f64 * cm.cell_height;

        // Always show cursor bar
        platform::update_highlight_layer(cm.cursor_layer, px, py, 2.0, cm.cell_height, true, false);

        // Compute selection rects (extracted to avoid borrow conflicts)
        let rects = if cm.selection != SelectionMode::None {
            if let Some((anchor_row, anchor_col)) = cm.sel_anchor {
                Self::compute_selection_rects_static(
                    cm.selection, cm.cursor_row, cm.cursor_col,
                    anchor_row, anchor_col, offset,
                    cm.viewport_cols, cm.cell_width, cm.cell_height, term_y,
                )
            } else {
                vec![]
            }
        } else {
            vec![]
        };

        // Grow layer pool if needed
        let cm = self.copy_mode.as_mut().unwrap();
        while cm.highlight_layers.len() < rects.len() {
            cm.highlight_layers.push(platform::create_highlight_layer());
        }
        // Position visible layers
        for (i, &(x, y, w, h)) in rects.iter().enumerate() {
            platform::update_highlight_layer(cm.highlight_layers[i], x, y, w, h, true, true);
        }
        // Hide unused layers
        for i in rects.len()..cm.highlight_layers.len() {
            platform::update_highlight_layer(cm.highlight_layers[i], 0.0, 0.0, 0.0, 0.0, false, true);
        }
    }

    fn compute_selection_rects_static(
        selection: SelectionMode,
        cursor_row: i64, cursor_col: u32,
        anchor_row: i64, anchor_col: u32,
        offset: i64,
        viewport_cols: u32,
        cell_width: f64, cell_height: f64,
        term_y: f64,
    ) -> Vec<(f64, f64, f64, f64)> {
        let (r1, c1, r2, c2) = if anchor_row < cursor_row
            || (anchor_row == cursor_row && anchor_col <= cursor_col)
        {
            (anchor_row, anchor_col, cursor_row, cursor_col)
        } else {
            (cursor_row, cursor_col, anchor_row, anchor_col)
        };
        let full_w = viewport_cols as f64 * cell_width;

        match selection {
            SelectionMode::Char => {
                if r1 == r2 {
                    let x = c1 as f64 * cell_width;
                    let y = term_y + (r1 - offset) as f64 * cell_height;
                    let w = (c2 as f64 - c1 as f64 + 1.0) * cell_width;
                    vec![(x, y, w, cell_height)]
                } else {
                    let mut rects = Vec::new();
                    let y1 = term_y + (r1 - offset) as f64 * cell_height;
                    rects.push((c1 as f64 * cell_width, y1, full_w - c1 as f64 * cell_width, cell_height));
                    for r in (r1 + 1)..r2 {
                        let y = term_y + (r - offset) as f64 * cell_height;
                        rects.push((0.0, y, full_w, cell_height));
                    }
                    let y2 = term_y + (r2 - offset) as f64 * cell_height;
                    rects.push((0.0, y2, (c2 as f64 + 1.0) * cell_width, cell_height));
                    rects
                }
            }
            SelectionMode::Line => {
                (r1..=r2).map(|r| {
                    let y = term_y + (r - offset) as f64 * cell_height;
                    (0.0, y, full_w, cell_height)
                }).collect()
            }
            SelectionMode::Rectangle => {
                let min_c = c1.min(c2);
                let max_c = c1.max(c2);
                let x = min_c as f64 * cell_width;
                let w = (max_c as f64 - min_c as f64 + 1.0) * cell_width;
                (r1..=r2).map(|r| {
                    let y = term_y + (r - offset) as f64 * cell_height;
                    (x, y, w, cell_height)
                }).collect()
            }
            SelectionMode::None => vec![],
        }
    }

    fn copy_mode_copy(&mut self) {
        let Some(ref cm) = self.copy_mode else { return };
        let Some((anchor_row, anchor_col)) = cm.sel_anchor else { return };

        let (r1, c1, r2, c2) = if anchor_row < cm.cursor_row
            || (anchor_row == cm.cursor_row && anchor_col <= cm.cursor_col)
        {
            (anchor_row, anchor_col, cm.cursor_row, cm.cursor_col)
        } else {
            (cm.cursor_row, cm.cursor_col, anchor_row, anchor_col)
        };

        // For line selection, select full lines
        let (c1, c2) = if cm.selection == SelectionMode::Line {
            (0u32, cm.viewport_cols.saturating_sub(1))
        } else {
            (c1, c2)
        };

        let sel = ffi::ghostty_selection_s {
            top_left: ffi::ghostty_point_s {
                tag: ffi::GHOSTTY_POINT_VIEWPORT,
                coord: ffi::GHOSTTY_POINT_COORD_EXACT,
                x: c1,
                y: (r1 - self.scrollbar.offset as i64).max(0) as u32,
            },
            bottom_right: ffi::ghostty_point_s {
                tag: ffi::GHOSTTY_POINT_VIEWPORT,
                coord: ffi::GHOSTTY_POINT_COORD_EXACT,
                x: c2,
                y: (r2 - self.scrollbar.offset as i64).max(0) as u32,
            },
            rectangle: cm.selection == SelectionMode::Rectangle,
        };
        if let Some(text) = self.read_surface_selection_text(sel) {
            platform::clipboard_write(&text);
            self.last_clipboard_text = text.clone();
            log::info!("copy mode: copied {} bytes", text.len());
        }
    }

    fn dispatch_binding_action(&mut self, action: bindings::Action) {
        match action {
            bindings::Action::NewSplit(dir) => self.create_split(dir),
            bindings::Action::GotoSplit(dir) => {
                self.switch_focus(dir);
            }
            bindings::Action::ResizeSplit(dir, amount) => {
                let delta = amount as f64 / 100.0;
                let (axis, sign) = match dir {
                    bindings::Direction::Right => (splits::Direction::Horizontal, 1.0),
                    bindings::Direction::Left => (splits::Direction::Horizontal, -1.0),
                    bindings::Direction::Down => (splits::Direction::Vertical, 1.0),
                    bindings::Direction::Up => (splits::Direction::Vertical, -1.0),
                };
                if let Some(tree) = self.tabs.active_tree_mut() {
                    tree.resize_focused(axis, delta * sign);
                }
                self.relayout();
            }
            bindings::Action::CloseSurface => self.handle_surface_closed(),
            bindings::Action::NewTab => self.new_tab(),
            bindings::Action::NextTab => {
                self.tabs.next_tab();
                self.sync_after_tab_change();
            }
            bindings::Action::PrevTab => {
                self.tabs.prev_tab();
                self.sync_after_tab_change();
            }
            bindings::Action::CloseTab => {
                if self.tabs.len() <= 1 {
                    self.terminate(0);
                }
                let active = self.tabs.active_index();
                let panes = self.tabs.remove_tab(active);
                for pane in panes {
                    self.free_pane_backend(pane);
                }
                self.sync_after_tab_change();
            }
            bindings::Action::GotoTab(target) => {
                let idx = match target {
                    bindings::TabTarget::Index(i) => i,
                    bindings::TabTarget::Last => self.tabs.len().saturating_sub(1),
                };
                self.tabs.goto_tab(idx);
                self.sync_after_tab_change();
            }
            bindings::Action::Search => {
                self.search_active = true;
                self.search_query.clear();
                self.search_total = 0;
                self.search_selected = 0;
                self.relayout();
            }
            bindings::Action::EnterCopyMode => {
                self.enter_copy_mode();
            }
            bindings::Action::Paste => {
                self.ghostty_binding_action("paste_from_clipboard");
            }
            bindings::Action::ToggleZoom => {
                self.ghostty_binding_action("toggle_split_zoom");
                self.relayout();
            }
            bindings::Action::NextPane => {
                if let Some(tree) = self.tabs.active_tree_mut() {
                    tree.focus_next();
                }
                let new = self.tabs.focused_pane();
                self.set_pane_focus(new, true);
            }
            bindings::Action::PreviousPane => {
                if let Some(tree) = self.tabs.active_tree_mut() {
                    tree.focus_prev();
                }
                let new = self.tabs.focused_pane();
                self.set_pane_focus(new, true);
            }
            bindings::Action::PreviousTab => {
                let prev = self.tabs.previous_active();
                self.tabs.goto_tab(prev);
                self.sync_after_tab_change();
            }
            bindings::Action::ReloadConfig => {
                log::info!("reloading config");
                // Reload boo's own config
                let boo_config = config::Config::load();
                self.bindings = bindings::Bindings::from_config(&boo_config);
                self.apply_appearance(Self::resolve_appearance_config(&boo_config));
                self.backend.reload_config();
                self.relayout();
                log::info!("config reloaded");
            }
            bindings::Action::OpenCommandPrompt => {
                self.command_prompt.active = true;
                self.command_prompt.input.clear();
                self.command_prompt.selected_suggestion = 0;
                self.command_prompt.history_idx = None;
                self.command_prompt.update_suggestions();
            }
        }
    }

    fn handle_command_key<S: AsRef<str>>(
        &mut self,
        key: &keyboard::Key,
        text: &Option<S>,
        modifiers: &keyboard::Modifiers,
    ) {
        use keyboard::key::Named;
        match key {
            keyboard::Key::Named(Named::Escape) => {
                self.command_prompt.active = false;
            }
            keyboard::Key::Named(Named::Enter) => {
                let input = self.command_prompt.input.clone();
                if !input.is_empty() {
                    self.command_prompt.history.push(input.clone());
                }
                self.command_prompt.active = false;
                self.execute_command(&input);
            }
            keyboard::Key::Named(Named::Backspace) => {
                if modifiers.control() {
                    // Ctrl-w: delete word backward
                    let trimmed = self.command_prompt.input.trim_end();
                    if let Some(pos) = trimmed.rfind(|c: char| c.is_whitespace()) {
                        self.command_prompt.input.truncate(pos + 1);
                    } else {
                        self.command_prompt.input.clear();
                    }
                } else {
                    self.command_prompt.input.pop();
                }
                self.command_prompt.update_suggestions();
            }
            keyboard::Key::Named(Named::Tab) => {
                // Accept top suggestion
                if let Some(cmd) = self.command_prompt.selected_command() {
                    self.command_prompt.input = cmd.name.to_string();
                    if !cmd.args.is_empty() {
                        self.command_prompt.input.push(' ');
                    }
                    self.command_prompt.update_suggestions();
                }
            }
            keyboard::Key::Named(Named::ArrowUp) => {
                if !self.command_prompt.suggestions.is_empty() {
                    if self.command_prompt.selected_suggestion > 0 {
                        self.command_prompt.selected_suggestion -= 1;
                    }
                } else {
                    // History navigation
                    let hist_len = self.command_prompt.history.len();
                    if hist_len > 0 {
                        let idx = self.command_prompt.history_idx
                            .map(|i| i.saturating_sub(1))
                            .unwrap_or(hist_len - 1);
                        self.command_prompt.history_idx = Some(idx);
                        self.command_prompt.input = self.command_prompt.history[idx].clone();
                    }
                }
            }
            keyboard::Key::Named(Named::ArrowDown) => {
                if !self.command_prompt.suggestions.is_empty() {
                    if self.command_prompt.selected_suggestion + 1 < self.command_prompt.suggestions.len() {
                        self.command_prompt.selected_suggestion += 1;
                    }
                } else {
                    // History navigation
                    if let Some(idx) = self.command_prompt.history_idx {
                        if idx + 1 < self.command_prompt.history.len() {
                            self.command_prompt.history_idx = Some(idx + 1);
                            self.command_prompt.input = self.command_prompt.history[idx + 1].clone();
                        } else {
                            self.command_prompt.history_idx = None;
                            self.command_prompt.input.clear();
                        }
                    }
                }
            }
            keyboard::Key::Named(Named::Home) => {
                if modifiers.control() {
                    // Ctrl-a: start of input (no cursor pos tracking yet)
                }
            }
            _ => {
                if modifiers.control() {
                    // Ctrl-a, Ctrl-e, Ctrl-w handled above
                } else if let Some(t) = text {
                    for ch in t.as_ref().chars() {
                        if ch >= ' ' {
                            self.command_prompt.input.push(ch);
                        }
                    }
                    self.command_prompt.update_suggestions();
                }
            }
        }
    }

    fn execute_command(&mut self, input: &str) {
        let parts: Vec<&str> = input.split_whitespace().collect();
        if parts.is_empty() { return; }

        let cmd = parts[0];
        let arg1 = parts.get(1).copied();

        match cmd {
            "split-right" => self.dispatch_binding_action(bindings::Action::NewSplit(bindings::SplitDirection::Right)),
            "split-down" => self.dispatch_binding_action(bindings::Action::NewSplit(bindings::SplitDirection::Down)),
            "split-left" => self.dispatch_binding_action(bindings::Action::NewSplit(bindings::SplitDirection::Left)),
            "split-up" => self.dispatch_binding_action(bindings::Action::NewSplit(bindings::SplitDirection::Up)),
            "resize-left" => {
                let n: u16 = arg1.and_then(|s| s.parse().ok()).unwrap_or(10);
                self.dispatch_binding_action(bindings::Action::ResizeSplit(bindings::Direction::Left, n));
            }
            "resize-right" => {
                let n: u16 = arg1.and_then(|s| s.parse().ok()).unwrap_or(10);
                self.dispatch_binding_action(bindings::Action::ResizeSplit(bindings::Direction::Right, n));
            }
            "resize-up" => {
                let n: u16 = arg1.and_then(|s| s.parse().ok()).unwrap_or(10);
                self.dispatch_binding_action(bindings::Action::ResizeSplit(bindings::Direction::Up, n));
            }
            "resize-down" => {
                let n: u16 = arg1.and_then(|s| s.parse().ok()).unwrap_or(10);
                self.dispatch_binding_action(bindings::Action::ResizeSplit(bindings::Direction::Down, n));
            }
            "close-pane" => self.dispatch_binding_action(bindings::Action::CloseSurface),
            "new-tab" => self.dispatch_binding_action(bindings::Action::NewTab),
            "next-tab" => self.dispatch_binding_action(bindings::Action::NextTab),
            "prev-tab" => self.dispatch_binding_action(bindings::Action::PrevTab),
            "close-tab" => self.dispatch_binding_action(bindings::Action::CloseTab),
            "goto-tab" => {
                if let Some(n) = arg1.and_then(|s| s.parse::<usize>().ok()) {
                    self.dispatch_binding_action(bindings::Action::GotoTab(bindings::TabTarget::Index(n.saturating_sub(1))));
                }
            }
            "last-tab" => self.dispatch_binding_action(bindings::Action::GotoTab(bindings::TabTarget::Last)),
            "next-pane" => self.dispatch_binding_action(bindings::Action::NextPane),
            "prev-pane" => self.dispatch_binding_action(bindings::Action::PreviousPane),
            "copy-mode" => self.dispatch_binding_action(bindings::Action::EnterCopyMode),
            "command-prompt" => self.dispatch_binding_action(bindings::Action::OpenCommandPrompt),
            "search" => self.dispatch_binding_action(bindings::Action::Search),
            "paste" => self.dispatch_binding_action(bindings::Action::Paste),
            "zoom" => self.dispatch_binding_action(bindings::Action::ToggleZoom),
            "reload-config" => self.dispatch_binding_action(bindings::Action::ReloadConfig),
            "goto-line" => {
                if let Some(n) = arg1.and_then(|s| s.parse::<i64>().ok()) {
                    if let Some(ref mut cm) = self.copy_mode {
                        cm.cursor_row = n;
                        cm.cursor_col = 0;
                    }
                    self.copy_mode_ensure_visible();
                    self.update_copy_mode_highlight();
                }
            }
            "set" => {
                // set <key> <value> — reload config with override via temp file
                if parts.len() >= 3 {
                    let key = parts[1];
                    let val = parts[2..].join(" ");
                    self.backend
                        .apply_config_override(self.focused_surface(), key, &val);
                    log::info!("set: {key} = {val}");
                }
            }
            "load-session" => {
                if let Some(name) = arg1 {
                    self.load_session(name);
                }
            }
            "save-session" => {
                if let Some(name) = arg1 {
                    self.save_current_session(name);
                }
            }
            "list-sessions" => {
                let sessions = session::list_sessions();
                log::info!("sessions: {}", sessions.join(", "));
            }
            _ => {
                // Try as a bare number for goto-line in copy mode
                if let Ok(n) = cmd.parse::<i64>() {
                    if self.bindings.is_copy_mode() {
                        if let Some(ref mut cm) = self.copy_mode {
                            cm.cursor_row = n;
                            cm.cursor_col = 0;
                        }
                        self.copy_mode_ensure_visible();
                        self.update_copy_mode_highlight();
                    }
                } else {
                    log::warn!("unknown command: {cmd}");
                }
            }
        }
    }

    fn handle_search_key<S: AsRef<str>>(
        &mut self,
        key: &keyboard::Key,
        text: &Option<S>,
        modifiers: &keyboard::Modifiers,
    ) {
        use keyboard::key::Named;
        match key {
            keyboard::Key::Named(Named::Escape) => {
                self.search_active = false;
                self.search_query.clear();
                if !self.bindings.is_copy_mode() {
                    self.ghostty_binding_action("end_search");
                }
                self.relayout();
            }
            keyboard::Key::Named(Named::Enter) => {
                if modifiers.shift() {
                    self.ghostty_binding_action("navigate_search:previous");
                } else {
                    self.ghostty_binding_action("navigate_search:next");
                }
            }
            keyboard::Key::Named(Named::Backspace) => {
                self.search_query.pop();
                self.send_search();
            }
            _ => {
                if let Some(t) = text {
                    for ch in t.as_ref().chars() {
                        if ch >= ' ' {
                            self.search_query.push(ch);
                        }
                    }
                    self.send_search();
                }
            }
        }
    }

    fn update_scrollbar_overlay(&self) {
        if self.scrollbar_layer.is_null() {
            return;
        }
        let w = self.last_size.width as f64;
        let h = self.last_size.height as f64 - STATUS_BAR_HEIGHT;
        if h <= 0.0 || self.scrollbar.total == 0 {
            platform::update_scrollbar_layer(self.scrollbar_layer, 0.0, 0.0, 0.0, 0.0, 0.0);
            return;
        }
        let ratio = self.scrollbar.len as f64 / self.scrollbar.total as f64;
        let thumb_h = (ratio * h).max(20.0);
        let scroll_range = self.scrollbar.total.saturating_sub(self.scrollbar.len) as f64;
        let thumb_y = if scroll_range > 0.0 {
            (self.scrollbar.offset as f64 / scroll_range) * (h - thumb_h)
        } else {
            0.0
        };
        let sb_width = 6.0;
        let margin = 2.0;
        platform::update_scrollbar_layer(
            self.scrollbar_layer,
            w - sb_width - margin,
            thumb_y,
            sb_width,
            thumb_h,
            self.scrollbar_opacity,
        );
    }

    fn sync_after_tab_change(&mut self) {
        let focused = self.tabs.focused_pane();
        self.set_pane_focus(focused, true);
        self.relayout();
    }

    fn read_surface_selection_text(&self, selection: ffi::ghostty_selection_s) -> Option<String> {
        self.backend
            .read_selection_text(self.tabs.focused_pane(), selection)
    }

    fn ghostty_binding_action(&mut self, action: &str) {
        self.backend
            .binding_action(self.tabs.focused_pane(), action, self.scrollbar.len);
    }

    fn send_search(&mut self) {
        self.ghostty_binding_action(&format!("search:{}", self.search_query));
    }

    fn scroll_to_mouse_y(&mut self, y: f64) {
        let terminal_h = self.last_size.height as f64 - STATUS_BAR_HEIGHT;
        if terminal_h <= 0.0 || self.scrollbar.total <= self.scrollbar.len {
            return;
        }
        let fraction = (y / terminal_h).clamp(0.0, 1.0);
        let max_offset = self.scrollbar.total.saturating_sub(self.scrollbar.len);
        let target_row = (fraction * max_offset as f64) as u64;
        let surface = self.focused_surface();
        if !surface.is_null() {
            self.ghostty_binding_action(&format!("scroll_to_row:{target_row}"));
            return;
        }
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            let delta = target_row as i64 - self.scrollbar.offset as i64;
            let _ = if target_row == 0 {
                self.backend.scroll_viewport_top(self.tabs.focused_pane())
            } else if target_row >= max_offset {
                self.backend.scroll_viewport_bottom(self.tabs.focused_pane())
            } else {
                self.backend
                    .scroll_viewport_delta(self.tabs.focused_pane(), delta as isize)
            };
        }
    }

    fn terminal_frame(&self) -> platform::Rect {
        let search_offset = if self.search_active { STATUS_BAR_HEIGHT } else { 0.0 };
        platform::Rect::new(
            platform::Point::new(0.0, search_offset),
            platform::Size::new(
                self.last_size.width as f64,
                self.last_size.height as f64 - STATUS_BAR_HEIGHT - search_offset,
            ),
        )
    }

    fn handle_mouse(&mut self, event: mouse::Event) {
        match event {
            mouse::Event::CursorMoved { position } => {
                self.last_mouse_pos = (position.x as f64, position.y as f64);
                if let Some(dir) = self.divider_drag {
                    let frame = self.terminal_frame();
                    let point = (position.x as f64, position.y as f64);
                    if let Some(tree) = self.tabs.active_tree_mut() {
                        tree.resize_drag(frame, dir, point);
                    }
                    self.relayout();
                    return;
                }
                if self.scrollbar_drag {
                    self.scroll_to_mouse_y(position.y as f64);
                    return;
                }
                #[cfg(any(target_os = "linux", target_os = "macos"))]
                if self.focused_surface().is_null() {
                    let _ = self.backend.send_mouse_input(
                        self.tabs.focused_pane(),
                        vt::GHOSTTY_MOUSE_ACTION_MOTION,
                        None,
                        position.x,
                        position.y,
                        ffi::GHOSTTY_MODS_NONE as vt::GhosttyMods,
                    );
                    return;
                }
                self.forward_surface_mouse_pos(
                    position.x as f64,
                    position.y as f64,
                    ffi::GHOSTTY_MODS_NONE,
                );
            }
            mouse::Event::ButtonPressed(button) => {
                if button == mouse::Button::Left {
                    // Check scrollbar area (rightmost 10px)
                    let (mx, my) = self.last_mouse_pos;
                    let terminal_h = self.last_size.height as f64 - STATUS_BAR_HEIGHT;
                    if mx >= self.last_size.width as f64 - 10.0
                        && my < terminal_h
                    {
                        self.scrollbar_drag = true;
                        self.scrollbar_opacity = 1.0;
                        self.scroll_to_mouse_y(my);
                        return;
                    }

                    // Check split dividers
                    let frame = self.terminal_frame();
                    let point = (mx, my);
                    if let Some(tree) = self.tabs.active_tree() {
                        if let Some(dir) = tree.divider_at(frame, point) {
                            self.divider_drag = Some(dir);
                            return;
                        }
                    }

                    // Click to focus split pane
                    let old = self.tabs.focused_pane();
                    if let Some(tree) = self.tabs.active_tree_mut() {
                        if tree.focus_at(frame, point) {
                            let new = self.tabs.focused_pane();
                            self.set_pane_focus(old, false);
                            self.set_pane_focus(new, true);
                        }
                    }
                }
                #[cfg(any(target_os = "linux", target_os = "macos"))]
                if self.focused_surface().is_null() {
                    let (mx, my) = self.last_mouse_pos;
                    let _ = self.backend.send_mouse_input(
                        self.tabs.focused_pane(),
                        vt::GHOSTTY_MOUSE_ACTION_PRESS,
                        Some(iced_button_to_vt(button)),
                        mx as f32,
                        my as f32,
                        ffi::GHOSTTY_MODS_NONE as vt::GhosttyMods,
                    );
                    return;
                }
                self.forward_surface_mouse_button(
                    ffi::ghostty_input_mouse_state_e::GHOSTTY_MOUSE_PRESS,
                    iced_button_to_ghostty(button),
                    ffi::GHOSTTY_MODS_NONE,
                );
            }
            mouse::Event::ButtonReleased(button) => {
                if button == mouse::Button::Left {
                    if self.divider_drag.is_some() {
                        self.divider_drag = None;
                        return;
                    }
                    if self.scrollbar_drag {
                        self.scrollbar_drag = false;
                        return;
                    }
                }
                #[cfg(any(target_os = "linux", target_os = "macos"))]
                if self.focused_surface().is_null() {
                    let (mx, my) = self.last_mouse_pos;
                    let _ = self.backend.send_mouse_input(
                        self.tabs.focused_pane(),
                        vt::GHOSTTY_MOUSE_ACTION_RELEASE,
                        Some(iced_button_to_vt(button)),
                        mx as f32,
                        my as f32,
                        ffi::GHOSTTY_MODS_NONE as vt::GhosttyMods,
                    );
                    return;
                }
                self.forward_surface_mouse_button(
                    ffi::ghostty_input_mouse_state_e::GHOSTTY_MOUSE_RELEASE,
                    iced_button_to_ghostty(button),
                    ffi::GHOSTTY_MODS_NONE,
                );
            }
            mouse::Event::WheelScrolled { delta } => {
                #[cfg(any(target_os = "linux", target_os = "macos"))]
                {
                    let surface = self.focused_surface();
                    let (dx, dy) = match delta {
                        mouse::ScrollDelta::Lines { x, y } => (x as f64, y as f64),
                        mouse::ScrollDelta::Pixels { x, y } => (x as f64, y as f64),
                    };
                    if self.scrollbar.total > self.scrollbar.len {
                        self.scrollbar_opacity = 1.0;
                    }
                    if !surface.is_null() {
                        self.forward_surface_mouse_scroll(dx, dy, 0);
                    } else {
                        let line_delta = if dy.abs() >= 1.0 {
                            -dy.round() as isize
                        } else if dy > 0.0 {
                            -1
                        } else if dy < 0.0 {
                            1
                        } else {
                            0
                        };
                        let _ = self
                            .backend
                            .scroll_viewport_delta(self.tabs.focused_pane(), line_delta);
                    }
                }
            }
            _ => {}
        }
    }

    fn scale_factor(&self) -> f64 {
        platform::scale_factor()
    }


    fn handle_resize(&mut self, size: Size) {
        self.last_size = size;
        self.relayout();
    }

    fn init_surface(&mut self) {
        if !self.tabs.is_empty() {
            return;
        }
        let cv = platform::content_view_handle();
        if cv.is_null() {
            log::debug!("init_surface: content view not ready");
            return;
        }
        self.parent_view = cv;
        platform::set_window_transparent();
        self.scrollbar_layer = platform::create_scrollbar_layer();
        let bounds = platform::view_bounds(cv);
        if (self.last_size.width <= 1.0 || self.last_size.height <= 1.0)
            && bounds.size.width > 1.0
            && bounds.size.height > 1.0
        {
            self.last_size = Size::new(bounds.size.width as f32, bounds.size.height as f32);
        }
        let frame = self.terminal_frame();
        if frame.size.width <= 1.0 || frame.size.height <= 1.0 {
            return;
        }

        let Some(pane) = self.create_pane(
            frame,
            ffi::ghostty_surface_context_e::GHOSTTY_SURFACE_CONTEXT_WINDOW,
        ) else { return };
        self.tabs.add_initial_tab(pane);
        self.set_pane_focus(pane, true);

        // Tell ghostty the initial surface size so it starts rendering.
        {
            let scale = self.scale_factor();
            let (w, h) = if frame.size.width > 0.0 && frame.size.height > 0.0 {
                ((frame.size.width * scale) as u32, (frame.size.height * scale) as u32)
            } else {
                (800, 600) // default until first resize
            };
            self.resize_pane_backend(pane, scale, w, h);
            log::info!("tab 0 created, size {w}x{h}");
        }

        // Load startup session if --session was specified
        if let Some(name) = STARTUP_SESSION.get() {
            self.load_session(name);
        }
    }

    fn create_pane(
        &mut self,
        frame: platform::Rect,
        context: ffi::ghostty_surface_context_e,
    ) -> Option<PaneHandle> {
        self.create_pane_with(frame, context, None, None)
    }

    #[allow(unused_variables)]
    fn create_pane_with(
        &mut self,
        frame: platform::Rect,
        context: ffi::ghostty_surface_context_e,
        command: Option<&CStr>,
        working_directory: Option<&CStr>,
    ) -> Option<PaneHandle> {
        self.backend.create_pane(
            ptr::null_mut(),
            self.parent_view,
            self.scale_factor(),
            frame,
            context,
            command,
            working_directory,
            self.cell_width,
            self.cell_height,
        )
    }

    fn create_split(&mut self, direction: bindings::SplitDirection) {
        if self.parent_view.is_null() || self.tabs.is_empty() {
            return;
        }
        let parent_bounds = platform::view_bounds(self.parent_view);
        let split_dir = match direction {
            bindings::SplitDirection::Right | bindings::SplitDirection::Left => {
                splits::Direction::Horizontal
            }
            _ => splits::Direction::Vertical,
        };

        let Some(pane) = self.create_pane(
            parent_bounds,
            ffi::ghostty_surface_context_e::GHOSTTY_SURFACE_CONTEXT_SPLIT,
        ) else { return };
        let old_focused = self.tabs.focused_pane();
        if let Some(tree) = self.tabs.active_tree_mut() {
            tree.split_focused(split_dir, pane);
        }

        self.set_pane_focus(old_focused, false);
        self.set_pane_focus(pane, true);

        self.relayout();
        log::info!("split created");
    }

    fn switch_focus(&mut self, dir: bindings::PaneFocusDirection) {
        let old = self.tabs.focused_pane();
        if let Some(tree) = self.tabs.active_tree_mut() {
            match dir {
                bindings::PaneFocusDirection::Next => tree.focus_next(),
                bindings::PaneFocusDirection::Previous => tree.focus_prev(),
            }
        }
        let new = self.tabs.focused_pane();
        if old != new {
            self.set_pane_focus(old, false);
            self.set_pane_focus(new, true);
        }
    }

    fn relayout(&mut self) {
        if self.tabs.is_empty() || self.last_size.width == 0.0 {
            return;
        }
        let scale = self.scale_factor();
        let frame = self.terminal_frame();
        let surfaces = self.tabs.layout_active(frame, scale);
        for (pane, w, h) in surfaces {
            self.resize_pane_backend(pane, scale, w, h);
        }
    }

    fn handle_surface_closed(&mut self) {
        // Remove just the focused leaf from the split tree
        let removed = if let Some(tree) = self.tabs.active_tree_mut() {
            tree.remove_focused().map(|pane| (pane, tree.len() == 0))
        } else {
            None
        };
        if let Some((pane, tab_empty)) = removed {
            self.free_pane_backend(pane);

            if tab_empty {
                // Tab is empty — remove it or exit
                if self.tabs.len() <= 1 {
                    self.terminate(0);
                }
                let active = self.tabs.active_index();
                self.tabs.remove_tab(active);
            }

            let focused = self.tabs.focused_pane();
            self.set_pane_focus(focused, true);
            self.relayout();
            log::info!(
                "surface closed, {} surfaces in tab, {} tabs",
                self.tabs.active_tree().map(|t| t.len()).unwrap_or(0),
                self.tabs.len()
            );
            return;
        }

        // Fallback: no active tree or focused leaf not found
        if self.tabs.len() <= 1 {
            self.terminate(0);
        }
        let active = self.tabs.active_index();
        let panes = self.tabs.remove_tab(active);
        for pane in panes {
            self.free_pane_backend(pane);
        }
        let focused = self.tabs.focused_pane();
        self.set_pane_focus(focused, true);
        self.relayout();
    }

    fn new_tab(&mut self) {
        if self.parent_view.is_null() {
            return;
        }
        let frame = platform::view_bounds(self.parent_view);
        let Some(pane) = self.create_pane(
            frame,
            ffi::ghostty_surface_context_e::GHOSTTY_SURFACE_CONTEXT_TAB,
        ) else { return };
        let old = self.tabs.focused_pane();
        self.set_pane_focus(old, false);

        let idx = self.tabs.new_tab(pane);
        self.set_pane_focus(pane, true);
        self.relayout();
        log::info!("new tab {idx} (total: {})", self.tabs.len());
    }

    fn load_session(&mut self, name: &str) {
        let Some(layout) = session::load_session(name) else {
            log::warn!("session not found: {name}");
            return;
        };
        log::info!("loading session: {} ({} tabs)", layout.name, layout.tabs.len());
        if self.parent_view.is_null() {
            return;
        }
        let frame = platform::view_bounds(self.parent_view);

        for (tab_idx, session_tab) in layout.tabs.iter().enumerate() {
            // For named layouts, compute the split sequence automatically
            let auto_splits = if session_tab.layout != session::TabLayout::Manual {
                session::layout_splits(&session_tab.layout, session_tab.panes.len())
            } else {
                vec![]
            };

            for (pane_idx, pane) in session_tab.panes.iter().enumerate() {
                let cmd_cstr = pane.command.as_ref().map(|c| CString::new(c.as_str()).unwrap());
                let wd_cstr = pane.working_directory.as_ref().map(|w| CString::new(w.as_str()).unwrap());

                if pane_idx == 0 {
                    // First pane → create a new tab
                    let Some(pane) = self.create_pane_with(
                        frame,
                        if self.tabs.is_empty() && tab_idx == 0 {
                            ffi::ghostty_surface_context_e::GHOSTTY_SURFACE_CONTEXT_WINDOW
                        } else {
                            ffi::ghostty_surface_context_e::GHOSTTY_SURFACE_CONTEXT_TAB
                        },
                        cmd_cstr.as_deref(),
                        wd_cstr.as_deref(),
                    ) else { continue };
                    let old = self.tabs.focused_pane();
                    self.set_pane_focus(old, false);
                    self.tabs.new_tab(pane);
                    self.set_pane_focus(pane, true);
                } else {
                    // Determine split direction and ratio
                    let (split_dir, ratio) = if !auto_splits.is_empty() {
                        // Named layout: use computed splits
                        let spec = &auto_splits[pane_idx - 1];
                        let dir = match spec.direction {
                            session::SplitDir::Right => splits::Direction::Horizontal,
                            session::SplitDir::Down => splits::Direction::Vertical,
                        };
                        (dir, spec.ratio)
                    } else if let Some(ref spec) = pane.split {
                        // Manual layout: use explicit split
                        let dir = match spec.direction {
                            session::SplitDir::Right => splits::Direction::Horizontal,
                            session::SplitDir::Down => splits::Direction::Vertical,
                        };
                        (dir, spec.ratio)
                    } else {
                        (splits::Direction::Vertical, 0.5)
                    };

                    let Some(pane) = self.create_pane_with(
                        frame,
                        ffi::ghostty_surface_context_e::GHOSTTY_SURFACE_CONTEXT_SPLIT,
                        cmd_cstr.as_deref(),
                        wd_cstr.as_deref(),
                    ) else { continue };
                    if let Some(tree) = self.tabs.active_tree_mut() {
                        tree.split_focused_with_ratio(
                            split_dir,
                            pane,
                            ratio,
                        );
                    }
                    self.set_pane_focus(pane, true);
                }
            }
            // Set tab title
            if !session_tab.title.is_empty() {
                if let Some(tab) = self.tabs.tab_mut(tab_idx) {
                    tab.title = session_tab.title.clone();
                }
            }
        }
        self.relayout();
        log::info!("session loaded: {}", layout.name);
    }

    fn save_current_session(&self, name: &str) {
        let tab_infos = self.tabs.tab_info();
        let tabs: Vec<session::SessionTab> = tab_infos.iter().map(|info| {
            let panes = if let Some(tree) = self.tabs.tab_tree(info.index) {
                tree.export_panes().into_iter().map(|ep| {
                    let split = ep.split.map(|(dir, ratio)| session::SplitSpec {
                        direction: match dir {
                            splits::Direction::Horizontal => session::SplitDir::Right,
                            splits::Direction::Vertical => session::SplitDir::Down,
                        },
                        ratio,
                    });
                    session::SessionPane {
                        command: None, // can't read running command from ghostty
                        working_directory: None,
                        split,
                    }
                }).collect()
            } else {
                vec![session::SessionPane { command: None, working_directory: None, split: None }]
            };
            session::SessionTab {
                title: info.title.clone(),
                layout: session::TabLayout::Manual,
                panes,
            }
        }).collect();

        let layout = session::SessionLayout { name: name.to_string(), tabs };
        if let Err(e) = session::save_session(&layout) {
            log::error!("failed to save session: {e}");
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let ui_font = self.ui_font();
        // Search bar (overlays top of terminal area when active)
        let search_bar: Option<Element<'_, Message>> = if self.search_active {
            let label = if self.search_total > 0 {
                format!(
                    " search: {}  ({}/{})",
                    self.search_query,
                    self.search_selected + 1,
                    self.search_total
                )
            } else if self.search_query.is_empty() {
                " search: _".to_string()
            } else {
                format!(" search: {}  (no matches)", self.search_query)
            };
            Some(
                container(
                    text(label)
                        .font(ui_font)
                        .size(13)
                        .color(Color::from_rgb(0.9, 0.9, 0.9)),
                )
                .style(|_: &Theme| container::Style {
                    background: Some(iced::Background::Color(Color::from_rgba(
                        0.15, 0.15, 0.15, self.panel_alpha(0.95),
                    ))),
                    ..Default::default()
                })
                .width(Length::Fill)
                .height(Length::Fixed(STATUS_BAR_HEIGHT as f32))
                .padding([2, 6])
                .into(),
            )
        } else {
            None
        };

        let mut main_col = column![];
        if let Some(search) = search_bar {
            main_col = main_col.push(search);
        }
        // Terminal area
        if self.focused_surface().is_null() {
            if let Some(snapshot) = self.backend.render_snapshot(self.tabs.focused_pane().id()) {
                let selection_rects = self
                    .copy_mode
                    .as_ref()
                    .and_then(|copy_mode| {
                        copy_mode.sel_anchor.map(|(anchor_row, anchor_col)| {
                            Self::compute_selection_rects_static(
                                copy_mode.selection,
                                copy_mode.cursor_row,
                                copy_mode.cursor_col,
                                anchor_row,
                                anchor_col,
                                self.scrollbar.offset as i64,
                                copy_mode.viewport_cols,
                                copy_mode.cell_width,
                                copy_mode.cell_height,
                                0.0,
                            )
                        })
                    })
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(x, y, width, height)| vt_terminal_canvas::TerminalSelectionRect {
                        x: x as f32,
                        y: y as f32,
                        width: width as f32,
                        height: height as f32,
                    })
                    .collect::<Vec<_>>();

                let terminal_canvas = vt_terminal_canvas::TerminalCanvas::new(
                    snapshot,
                    self.cell_width as f32,
                    self.cell_height as f32,
                    self.terminal_font_size,
                    self.terminal_font_family,
                    self.appearance_revision,
                    self.background_opacity,
                    self.background_opacity_cells,
                    selection_rects,
                    Color::from_rgba(0.65, 0.72, 0.95, 0.35),
                    (!self.preedit_text.is_empty()).then(|| self.preedit_text.clone()),
                );
                main_col = main_col.push(
                    container(
                        iced::widget::canvas(terminal_canvas)
                            .width(Length::Fill)
                            .height(Length::Fill),
                    )
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .style(|_: &Theme| container::Style { ..Default::default() }),
                );
            } else {
                main_col = main_col.push(
                    iced::widget::Space::new().width(Length::Fill).height(Length::Fill),
                );
            }
        } else {
            main_col = main_col.push(
                iced::widget::Space::new().width(Length::Fill).height(Length::Fill),
            );
        }

        if self.command_prompt.active {
            // Suggestion overlay (above prompt)
            let suggestions = &self.command_prompt.suggestions;
            if !suggestions.is_empty() && !self.command_prompt.input.is_empty() {
                let mut suggestion_col = column![];
                for (display_idx, &cmd_idx) in suggestions.iter().enumerate().take(5) {
                    let cmd = &COMMANDS[cmd_idx];
                    let is_selected = display_idx == self.command_prompt.selected_suggestion;
                    let label = if cmd.args.is_empty() {
                        format!("  {:<24} {}", cmd.name, cmd.description)
                    } else {
                        format!("  {:<24} {} {}", cmd.name, cmd.description, cmd.args)
                    };
                    let fg = if is_selected {
                        Color::from_rgb(1.0, 1.0, 1.0)
                    } else {
                        Color::from_rgb(0.6, 0.6, 0.6)
                    };
                    let bg = if is_selected {
                        Color::from_rgba(0.3, 0.3, 0.5, 0.95)
                    } else {
                        Color::from_rgba(0.1, 0.1, 0.1, 0.9)
                    };
                    suggestion_col = suggestion_col.push(
                        container(
                            text(label).font(ui_font).size(13).color(fg),
                        )
                        .style(move |_: &Theme| container::Style {
                            background: Some(iced::Background::Color(bg)),
                            ..Default::default()
                        })
                        .width(Length::Fill)
                        .height(Length::Fixed(STATUS_BAR_HEIGHT as f32))
                        .padding([2, 6]),
                    );
                }
                main_col = main_col.push(suggestion_col);
            }

            // Command prompt line (replaces status bar)
            let prompt_label = format!(": {}_", self.command_prompt.input);
            main_col = main_col.push(
                container(
                    text(prompt_label)
                        .font(ui_font)
                        .size(13)
                        .color(Color::from_rgb(0.9, 0.9, 0.9)),
                )
                .style(|_: &Theme| container::Style {
                    background: Some(iced::Background::Color(Color::from_rgba(
                        0.15, 0.15, 0.15, self.panel_alpha(0.95),
                    ))),
                    ..Default::default()
                })
                .width(Length::Fill)
                .height(Length::Fixed(STATUS_BAR_HEIGHT as f32))
                .padding([2, 6]),
            );
        } else {
            // Normal three-zone status bar
            let (status_left, status_right) = self.build_status_zones();
            main_col = main_col.push(
                container(
                    row![
                        text(status_left)
                            .font(ui_font)
                            .size(13)
                            .color(Color::from_rgb(0.8, 0.8, 0.8)),
                        iced::widget::Space::new().width(Length::Fill),
                        text(status_right)
                            .font(ui_font)
                            .size(13)
                            .color(Color::from_rgb(0.6, 0.6, 0.6)),
                    ]
                    .width(Length::Fill),
                )
                .style(|_: &Theme| container::Style {
                    background: Some(iced::Background::Color(Color::from_rgba(
                        0.12,
                        0.12,
                        0.12,
                        self.panel_alpha(0.92),
                    ))),
                    ..Default::default()
                })
                .width(Length::Fill)
                .height(Length::Fixed(STATUS_BAR_HEIGHT as f32))
                .padding([2, 6]),
            );
        }
        main_col.into()
    }

    /// Build three-zone status bar: (left, right).
    fn build_status_zones(&self) -> (String, String) {
        // Left: tab list
        let spinner_frame = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| (duration.as_millis() / 125) as usize)
            .unwrap_or(0);
        let tabs = self.tabs.tab_info_with_spinner(spinner_frame);
        let mut parts = Vec::new();
        for tab in &tabs {
            let display_idx = tab.index + 1;
            let marker = if tab.active { "*" } else { "" };
            if tab.title.is_empty() {
                parts.push(format!("[{display_idx}{marker}]"));
            } else {
                parts.push(format!("[{display_idx}:{}{marker}]", tab.title));
            }
        }
        let left = parts.join(" ");

        // Right: pane count + mode
        let mut right_parts = Vec::new();
        let active_surfaces = self
            .tabs
            .active_tree()
            .map(|t| t.len())
            .unwrap_or(0);
        if active_surfaces > 1 {
            right_parts.push(format!("{active_surfaces} panes"));
        }
        if self.bindings.is_copy_mode() {
            let mode_str = match self.copy_mode.as_ref().map(|cm| cm.selection) {
                Some(SelectionMode::Char) => "VISUAL",
                Some(SelectionMode::Line) => "V-LINE",
                Some(SelectionMode::Rectangle) => "V-BLOCK",
                _ => "COPY",
            };
            let pos_str = if let Some(ref cm) = self.copy_mode {
                if cm.show_position {
                    format!(" [{}/{}]", cm.cursor_row, self.scrollbar.total)
                } else {
                    String::new()
                }
            } else {
                String::new()
            };
            right_parts.push(format!("{mode_str}{pos_str}"));
        } else if self.bindings.is_prefix_mode() {
            right_parts.push("PREFIX".to_string());
        }
        if !self.preedit_text.is_empty() {
            right_parts.push(format!("IME {}", self.preedit_text));
        }
        if !self.pwd.is_empty() {
            // Show shortened path: ~ for home, last 2 components otherwise
            let home = std::env::var("HOME").unwrap_or_default();
            let display = if let Some(rest) = self.pwd.strip_prefix(&home) {
                format!("~{rest}")
            } else {
                self.pwd.clone()
            };
            right_parts.push(display);
        }
        let right = right_parts.join("  ");

        (left, right)
    }

    fn theme(&self) -> Theme {
        Theme::Dark
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            window::frames().map(|_| Message::Frame),
            iced::event::listen().map(Message::IcedEvent),
        ])
    }
}

impl Drop for BooApp {
    fn drop(&mut self) {
        for pane in self.tabs.all_panes() {
            self.free_pane_backend(pane);
        }
        control::cleanup(self.socket_path.as_deref());
    }
}

/// For control pipe injection: compute the character for a keycode+mods combo.
fn shifted_codepoint(keycode: u32, mods: i32) -> u32 {
    let has_shift = mods & ffi::GHOSTTY_MODS_SHIFT != 0;
    #[cfg(target_os = "macos")]
    let base = match keycode {
        0x00 => 'a', 0x01 => 's', 0x02 => 'd', 0x03 => 'f', 0x04 => 'h',
        0x05 => 'g', 0x06 => 'z', 0x07 => 'x', 0x08 => 'c', 0x09 => 'v',
        0x0B => 'b', 0x0C => 'q', 0x0D => 'w', 0x0E => 'e', 0x0F => 'r',
        0x10 => 'y', 0x11 => 't', 0x20 => 'u', 0x22 => 'i', 0x1F => 'o',
        0x23 => 'p', 0x25 => 'l', 0x26 => 'j', 0x28 => 'k', 0x2D => 'n',
        0x2E => 'm', 0x31 => ' ', 0x24 => '\r', 0x30 => '\t',
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
fn shifted_codepoint_vt(keycode: u32, mods: i32) -> u32 {
    let has_shift = mods & ffi::GHOSTTY_MODS_SHIFT != 0;
    let base = match keycode {
        20 => 'a', 21 => 'b', 22 => 'c', 23 => 'd', 24 => 'e',
        25 => 'f', 26 => 'g', 27 => 'h', 28 => 'i', 29 => 'j',
        30 => 'k', 31 => 'l', 32 => 'm', 33 => 'n', 34 => 'o',
        35 => 'p', 36 => 'q', 37 => 'r', 38 => 's', 39 => 't',
        40 => 'u', 41 => 'v', 42 => 'w', 43 => 'x', 44 => 'y',
        45 => 'z', 63 => ' ', 58 => '\r', 64 => '\t',
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

/// Convert keycode+mods to the character it produces, or None.
fn shifted_char(keycode: u32, mods: i32) -> Option<char> {
    match shifted_codepoint(keycode, mods) {
        0 => None,
        cp => char::from_u32(cp),
    }
}

/// Parse "ctrl+c", "shift+0x27", "a" into (keycode, mods).
fn parse_keyspec(spec: &str) -> Option<(u32, i32)> {
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
        "a" => 0x00, "s" => 0x01, "d" => 0x02, "f" => 0x03,
        "h" => 0x04, "g" => 0x05, "z" => 0x06, "x" => 0x07,
        "c" => 0x08, "v" => 0x09, "b" => 0x0B, "q" => 0x0C,
        "w" => 0x0D, "e" => 0x0E, "r" => 0x0F, "y" => 0x10,
        "t" => 0x11, "u" => 0x20, "i" => 0x22, "o" => 0x1F,
        "p" => 0x23, "l" => 0x25, "j" => 0x26, "k" => 0x28,
        "n" => 0x2D, "m" => 0x2E,
        "enter" | "return" => 0x24,
        "tab" => 0x30, "space" => 0x31, "escape" | "esc" => 0x35,
        "backspace" => 0x33,
        _ if key_part.starts_with("0x") => {
            u32::from_str_radix(&key_part[2..], 16).ok()?
        }
        _ => return None,
    };
    #[cfg(target_os = "linux")]
    let keycode = match key_part {
        "a" => 20, "b" => 21, "c" => 22, "d" => 23,
        "e" => 24, "f" => 25, "g" => 26, "h" => 27,
        "i" => 28, "j" => 29, "k" => 30, "l" => 31,
        "m" => 32, "n" => 33, "o" => 34, "p" => 35,
        "q" => 36, "r" => 37, "s" => 38, "t" => 39,
        "u" => 40, "v" => 41, "w" => 42, "x" => 43,
        "y" => 44, "z" => 45,
        "0" => 6, "1" => 7, "2" => 8, "3" => 9,
        "4" => 10, "5" => 11, "6" => 12, "7" => 13,
        "8" => 14, "9" => 15,
        "enter" | "return" => 58,
        "tab" => 64,
        "space" => 63,
        "escape" | "esc" => 120,
        "backspace" => 53,
        _ if key_part.starts_with("0x") => {
            u32::from_str_radix(&key_part[2..], 16).ok()?
        }
        _ => return None,
    };
    Some((keycode, mods))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn parse_vt_keyspec(spec: &str) -> Option<(u32, i32)> {
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
        "a" => 20, "b" => 21, "c" => 22, "d" => 23,
        "e" => 24, "f" => 25, "g" => 26, "h" => 27,
        "i" => 28, "j" => 29, "k" => 30, "l" => 31,
        "m" => 32, "n" => 33, "o" => 34, "p" => 35,
        "q" => 36, "r" => 37, "s" => 38, "t" => 39,
        "u" => 40, "v" => 41, "w" => 42, "x" => 43,
        "y" => 44, "z" => 45,
        "0" => 6, "1" => 7, "2" => 8, "3" => 9,
        "4" => 10, "5" => 11, "6" => 12, "7" => 13,
        "8" => 14, "9" => 15,
        "enter" | "return" => 58,
        "tab" => 64,
        "space" => 63,
        "escape" | "esc" => 120,
        "backspace" => 53,
        "delete" => 68,
        _ if key_part.starts_with("0x") => {
            u32::from_str_radix(&key_part[2..], 16).ok()?
        }
        _ => return None,
    };
    Some((keycode, mods))
}

fn control_key_to_keyboard_key(spec: &str, key_char: Option<char>) -> keyboard::Key {
    use keyboard::key::Named;

    let key_name = spec
        .rsplit_once('+')
        .map(|(_, key)| key)
        .unwrap_or(spec);

    match key_name {
        "enter" | "return" => keyboard::Key::Named(Named::Enter),
        "tab" => keyboard::Key::Named(Named::Tab),
        "space" => keyboard::Key::Named(Named::Space),
        "escape" | "esc" => keyboard::Key::Named(Named::Escape),
        "backspace" => keyboard::Key::Named(Named::Backspace),
        _ => keyboard::Key::Character(key_char.unwrap_or_default().to_string().into()),
    }
}

#[cfg(target_os = "macos")]
fn should_route_macos_vt_key_via_appkit(vt_keycode: u32, mods: i32) -> bool {
    if mods & (ffi::GHOSTTY_MODS_CTRL | ffi::GHOSTTY_MODS_SUPER) != 0 {
        return false;
    }
    matches!(
        vt_keycode,
        1..=16 | 20..=50 | 63
    )
}

#[cfg(target_os = "macos")]
fn native_keycode_to_named_key(keycode: u32) -> Option<bindings::NamedKey> {
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
fn native_keycode_to_keyboard_key(keycode: u32, key_char: Option<char>) -> keyboard::Key {
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

fn ghostty_mods_to_iced(mods: i32) -> keyboard::Modifiers {
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

fn key_to_codepoint(key: &keyboard::Key) -> u32 {
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

fn iced_mods_to_ghostty(mods: &keyboard::Modifiers) -> ffi::ghostty_input_mods_e {
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

fn iced_button_to_ghostty(button: mouse::Button) -> ffi::ghostty_input_mouse_button_e {
    match button {
        mouse::Button::Left => ffi::ghostty_input_mouse_button_e::GHOSTTY_MOUSE_LEFT,
        mouse::Button::Right => ffi::ghostty_input_mouse_button_e::GHOSTTY_MOUSE_RIGHT,
        mouse::Button::Middle => ffi::ghostty_input_mouse_button_e::GHOSTTY_MOUSE_MIDDLE,
        _ => ffi::ghostty_input_mouse_button_e::GHOSTTY_MOUSE_UNKNOWN,
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn iced_button_to_vt(button: mouse::Button) -> vt::GhosttyMouseButton {
    match button {
        mouse::Button::Left => vt::GHOSTTY_MOUSE_BUTTON_LEFT,
        mouse::Button::Right => vt::GHOSTTY_MOUSE_BUTTON_RIGHT,
        mouse::Button::Middle => vt::GHOSTTY_MOUSE_BUTTON_MIDDLE,
        _ => vt::GHOSTTY_MOUSE_BUTTON_UNKNOWN,
    }
}

#[cfg(target_os = "linux")]
fn linux_snapshot_selection_text(
    snapshot: &linux_vt_backend::TerminalSnapshot,
    selection: ffi::ghostty_selection_s,
) -> String {
    let start_row = selection.top_left.y.min(selection.bottom_right.y) as usize;
    let end_row = selection.top_left.y.max(selection.bottom_right.y) as usize;
    let start_col = selection.top_left.x.min(selection.bottom_right.x) as usize;
    let end_col = selection.top_left.x.max(selection.bottom_right.x) as usize;
    let max_row = snapshot.rows_data.len().saturating_sub(1);

    let mut lines = Vec::new();
    for row_index in start_row.min(max_row)..=end_row.min(max_row) {
        let row = snapshot.rows_data.get(row_index).map(Vec::as_slice).unwrap_or(&[]);
        let line_start = if selection.rectangle || row_index == start_row {
            start_col
        } else {
            0
        };
        let line_end = if selection.rectangle || row_index == end_row {
            end_col
        } else {
            snapshot.cols.saturating_sub(1) as usize
        };
        let text = linux_snapshot_row_text(row, line_start, line_end, selection.rectangle);
        lines.push(text);
    }

    lines.join("\n")
}

#[cfg(target_os = "linux")]
fn linux_snapshot_row_text(
    row: &[linux_vt_backend::CellSnapshot],
    start_col: usize,
    end_col: usize,
    preserve_trailing_spaces: bool,
) -> String {
    if row.is_empty() || start_col > end_col {
        return String::new();
    }

    let mut out = String::new();
    for col in start_col..=end_col {
        let text = row
            .get(col)
            .map(|cell| cell.text.as_str())
            .filter(|text| !text.is_empty() && *text != "\0")
            .unwrap_or(" ");
        out.push_str(text);
    }

    if preserve_trailing_spaces {
        out
    } else {
        out.trim_end_matches(' ').to_string()
    }
}

fn split_direction_name(direction: splits::Direction) -> &'static str {
    match direction {
        splits::Direction::Horizontal => "horizontal",
        splits::Direction::Vertical => "vertical",
    }
}

fn selection_mode_name(selection: SelectionMode) -> &'static str {
    match selection {
        SelectionMode::None => "none",
        SelectionMode::Char => "character",
        SelectionMode::Line => "line",
        SelectionMode::Rectangle => "rectangle",
    }
}

fn ui_rect_snapshot(x: f64, y: f64, width: f64, height: f64) -> control::UiRectSnapshot {
    control::UiRectSnapshot { x, y, width, height }
}

#[cfg(target_os = "linux")]
fn ui_terminal_snapshot_from_linux(
    snapshot: &linux_vt_backend::TerminalSnapshot,
) -> control::UiTerminalSnapshot {
    control::UiTerminalSnapshot {
        cols: snapshot.cols,
        rows: snapshot.rows,
        title: snapshot.title.clone(),
        pwd: snapshot.pwd.clone(),
        cursor: control::UiCursorSnapshot {
            visible: snapshot.cursor.visible,
            x: snapshot.cursor.x,
            y: snapshot.cursor.y,
            style: snapshot.cursor.style,
        },
        rows_data: snapshot
            .rows_data
            .iter()
            .map(|row| control::UiTerminalRowSnapshot {
                cells: row
                    .iter()
                    .map(|cell| control::UiTerminalCellSnapshot {
                        text: cell.text.clone(),
                        display_width: cell.display_width,
                        fg: [cell.fg.r, cell.fg.g, cell.fg.b],
                        bg: [cell.bg.r, cell.bg.g, cell.bg.b],
                        bold: cell.bold,
                        italic: cell.italic,
                        underline: cell.underline,
                    })
                    .collect(),
            })
            .collect(),
    }
}

fn format_command_finished_message(exit_code: Option<u8>, duration_ns: u64) -> String {
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

fn command_finish_notification(
    desktop_notifications_enabled: bool,
    notify_action_enabled: bool,
    notify_on_command_finish: config::NotifyOnCommandFinish,
    app_focused: bool,
    notify_on_command_finish_after_ns: u64,
    event: CommandFinishedEvent,
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

enum TextInputAction {
    Commit(String),
    Command(platform::TextInputCommand),
}

fn apply_text_input_event(
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

fn text_input_command_key(command: platform::TextInputCommand) -> (u32, u32) {
    match command {
        platform::TextInputCommand::Backspace => (53, 0x08),
        platform::TextInputCommand::DeleteForward => (68, 0x7f),
        platform::TextInputCommand::InsertNewline => (58, '\r' as u32),
        platform::TextInputCommand::InsertTab => (64, '\t' as u32),
    }
}

#[cfg(test)]
pub mod main_tests {
    use super::*;

    pub fn compute_rects(
        selection: SelectionMode,
        cursor_row: i64, cursor_col: u32,
        anchor_row: i64, anchor_col: u32,
        offset: i64,
        viewport_cols: u32,
        cell_width: f64, cell_height: f64,
        term_y: f64,
    ) -> Vec<(f64, f64, f64, f64)> {
        BooApp::compute_selection_rects_static(
            selection, cursor_row, cursor_col,
            anchor_row, anchor_col, offset,
            viewport_cols, cell_width, cell_height, term_y,
        )
    }

    #[test]
    fn test_fuzzy_score_prefix() {
        assert!(fuzzy_score("split", "split-right") > fuzzy_score("sr", "split-right"));
        assert!(fuzzy_score("split", "split-right") > 50);
    }

    #[test]
    fn test_fuzzy_score_initials() {
        // "sr" matches "split-right" via word initials (s...r)
        assert!(fuzzy_score("sr", "split-right") > 0);
        assert!(fuzzy_score("sd", "split-down") > 0);
        assert!(fuzzy_score("nt", "new-tab") > 0);
    }

    #[test]
    fn test_fuzzy_score_subsequence() {
        assert!(fuzzy_score("srt", "split-right") > 0);
    }

    #[test]
    fn test_fuzzy_score_no_match() {
        assert_eq!(fuzzy_score("xyz", "split-right"), 0);
        assert_eq!(fuzzy_score("zzz", "new-tab"), 0);
    }

    #[test]
    fn test_fuzzy_score_empty_query() {
        assert!(fuzzy_score("", "anything") > 0);
    }

    #[test]
    fn test_command_finish_notification_respects_disabled_settings() {
        let event = CommandFinishedEvent {
            exit_code: Some(0),
            duration_ns: 10_000_000_000,
        };
        assert!(command_finish_notification(
            false,
            true,
            config::NotifyOnCommandFinish::Always,
            false,
            5_000_000_000,
            event,
        )
        .is_none());
        assert!(command_finish_notification(
            true,
            false,
            config::NotifyOnCommandFinish::Always,
            false,
            5_000_000_000,
            event,
        )
        .is_none());
        assert!(command_finish_notification(
            true,
            true,
            config::NotifyOnCommandFinish::Never,
            false,
            5_000_000_000,
            event,
        )
        .is_none());
    }

    #[test]
    fn test_command_finish_notification_respects_focus_and_threshold() {
        let event = CommandFinishedEvent {
            exit_code: Some(1),
            duration_ns: 8_000_000_000,
        };
        assert!(command_finish_notification(
            true,
            true,
            config::NotifyOnCommandFinish::Unfocused,
            true,
            5_000_000_000,
            event,
        )
        .is_none());
        assert!(command_finish_notification(
            true,
            true,
            config::NotifyOnCommandFinish::Always,
            false,
            8_000_000_000,
            event,
        )
        .is_none());
        let notification = command_finish_notification(
            true,
            true,
            config::NotifyOnCommandFinish::Unfocused,
            false,
            5_000_000_000,
            event,
        )
        .expect("notification should be emitted");
        assert_eq!(notification.0, "Command Failed");
        assert!(notification.1.contains("exited with code 1"));
    }

    #[test]
    fn test_apply_text_input_event_tracks_preedit_and_commit() {
        let mut preedit = String::new();
        assert!(apply_text_input_event(
            &mut preedit,
            platform::TextInputEvent::Preedit("kana".to_string()),
        )
        .is_none());
        assert_eq!(preedit, "kana");

        let committed = apply_text_input_event(
            &mut preedit,
            platform::TextInputEvent::Commit("かな".to_string()),
        );
        match committed {
            Some(TextInputAction::Commit(text)) => assert_eq!(text, "かな"),
            _ => panic!("expected committed text"),
        }
        assert!(preedit.is_empty());

        assert!(apply_text_input_event(&mut preedit, platform::TextInputEvent::PreeditClear).is_none());
        assert!(preedit.is_empty());
    }

    #[test]
    fn test_apply_text_input_event_forwards_text_commands() {
        let mut preedit = String::from("kana");
        let command = apply_text_input_event(
            &mut preedit,
            platform::TextInputEvent::Command(platform::TextInputCommand::Backspace),
        );
        assert_eq!(preedit, "kana");
        match command {
            Some(TextInputAction::Command(platform::TextInputCommand::Backspace)) => {}
            _ => panic!("expected backspace command"),
        }
    }

    #[test]
    fn test_parse_vt_keyspec_uses_portable_backspace_keycode() {
        let (keycode, mods) = parse_vt_keyspec("backspace").expect("backspace should parse");
        assert_eq!(keycode, 53);
        assert_eq!(mods, 0);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_should_route_macos_vt_key_via_appkit_only_for_plain_text_keys() {
        assert!(should_route_macos_vt_key_via_appkit(20, 0));
        assert!(should_route_macos_vt_key_via_appkit(63, 0));
        assert!(!should_route_macos_vt_key_via_appkit(53, 0));
        assert!(!should_route_macos_vt_key_via_appkit(64, 0));
        assert!(!should_route_macos_vt_key_via_appkit(58, 0));
        assert!(!should_route_macos_vt_key_via_appkit(20, ffi::GHOSTTY_MODS_CTRL));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_native_keycode_mapping_keeps_bindings_and_vt_in_sync() {
        assert!(matches!(
            native_keycode_to_named_key(0x7B),
            Some(bindings::NamedKey::ArrowLeft)
        ));
        assert!(matches!(
            native_keycode_to_keyboard_key(0x33, None),
            keyboard::Key::Named(keyboard::key::Named::Backspace)
        ));
        assert_eq!(crate::keymap::native_to_vt_keycode(0x33), Some(53));
        assert_eq!(crate::keymap::native_to_vt_keycode(0x24), Some(58));
    }

    #[test]
    fn test_fuzzy_score_ranking() {
        // Prefix match should beat initials
        let prefix = fuzzy_score("split", "split-right");
        let initials = fuzzy_score("sr", "split-right");
        let subseq = fuzzy_score("srt", "split-right");
        assert!(prefix > initials);
        assert!(initials > subseq);
    }

    #[test]
    fn test_command_prompt_suggestions() {
        let mut cp = CommandPrompt::new();
        cp.input = "split".to_string();
        cp.update_suggestions();
        // Should find split-right, split-down, split-left, split-up
        assert!(cp.suggestions.len() >= 4);
        // First suggestion should be a split command
        let first_cmd = &COMMANDS[cp.suggestions[0]];
        assert!(first_cmd.name.starts_with("split"));
    }

    #[test]
    fn test_command_prompt_suggestions_initials() {
        let mut cp = CommandPrompt::new();
        cp.input = "nt".to_string();
        cp.update_suggestions();
        // Should find "new-tab" and "next-tab"
        let names: Vec<&str> = cp.suggestions.iter().map(|&i| COMMANDS[i].name).collect();
        assert!(names.contains(&"new-tab") || names.contains(&"next-tab"));
    }

    #[test]
    fn test_command_prompt_empty_shows_all() {
        let mut cp = CommandPrompt::new();
        cp.input = "".to_string();
        cp.update_suggestions();
        assert_eq!(cp.suggestions.len(), COMMANDS.len());
    }

    #[test]
    fn test_command_prompt_no_match() {
        let mut cp = CommandPrompt::new();
        cp.input = "xyzxyz".to_string();
        cp.update_suggestions();
        assert!(cp.suggestions.is_empty());
    }

    #[test]
    fn test_command_prompt_tab_accepts() {
        let mut cp = CommandPrompt::new();
        cp.input = "split-r".to_string();
        cp.update_suggestions();
        if let Some(cmd) = cp.selected_command() {
            cp.input = cmd.name.to_string();
            if !cmd.args.is_empty() {
                cp.input.push(' ');
            }
        }
        assert_eq!(cp.input, "split-right");
    }

    #[test]
    fn test_command_prompt_history() {
        let mut cp = CommandPrompt::new();
        cp.history.push("split-right".to_string());
        cp.history.push("new-tab".to_string());

        // Navigate up
        cp.history_idx = Some(cp.history.len() - 1);
        cp.input = cp.history[cp.history_idx.unwrap()].clone();
        assert_eq!(cp.input, "new-tab");

        // Navigate up again
        cp.history_idx = Some(0);
        cp.input = cp.history[0].clone();
        assert_eq!(cp.input, "split-right");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_ui_terminal_snapshot_from_linux_preserves_cells_and_cursor() {
        let snapshot = linux_vt_backend::TerminalSnapshot {
            cols: 2,
            rows: 1,
            title: "shell".to_string(),
            pwd: "/tmp".to_string(),
            cursor: linux_vt_backend::CursorSnapshot {
                visible: true,
                x: 1,
                y: 0,
                style: 2,
            },
            rows_data: vec![vec![
                linux_vt_backend::CellSnapshot {
                    text: "a".to_string(),
                    display_width: 1,
                    fg: vt::GhosttyColorRgb { r: 1, g: 2, b: 3 },
                    bg: vt::GhosttyColorRgb { r: 4, g: 5, b: 6 },
                    bold: true,
                    italic: false,
                    underline: 1,
                },
                linux_vt_backend::CellSnapshot {
                    text: "b".to_string(),
                    display_width: 1,
                    fg: vt::GhosttyColorRgb { r: 7, g: 8, b: 9 },
                    bg: vt::GhosttyColorRgb { r: 10, g: 11, b: 12 },
                    bold: false,
                    italic: true,
                    underline: 0,
                },
            ]],
            scrollbar: vt::GhosttyTerminalScrollbar {
                total: 1,
                offset: 0,
                len: 1,
            },
            colors: vt::GhosttyRenderStateColors::default(),
        };

        let ui = ui_terminal_snapshot_from_linux(&snapshot);

        assert_eq!(ui.cols, 2);
        assert_eq!(ui.rows, 1);
        assert_eq!(ui.cursor.x, 1);
        assert_eq!(ui.rows_data[0].cells[0].text, "a");
        assert_eq!(ui.rows_data[0].cells[0].display_width, 1);
        assert_eq!(ui.rows_data[0].cells[0].fg, [1, 2, 3]);
        assert!(ui.rows_data[0].cells[0].bold);
        assert!(ui.rows_data[0].cells[1].italic);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_linux_snapshot_selection_text_trims_non_rectangular_lines() {
        let snapshot = linux_vt_backend::TerminalSnapshot {
            cols: 4,
            rows: 2,
            title: String::new(),
            pwd: String::new(),
            cursor: linux_vt_backend::CursorSnapshot::default(),
            rows_data: vec![
                vec![
                    linux_vt_backend::CellSnapshot { text: "a".into(), ..Default::default() },
                    linux_vt_backend::CellSnapshot { text: "b".into(), ..Default::default() },
                    linux_vt_backend::CellSnapshot::default(),
                    linux_vt_backend::CellSnapshot::default(),
                ],
                vec![
                    linux_vt_backend::CellSnapshot { text: "c".into(), ..Default::default() },
                    linux_vt_backend::CellSnapshot::default(),
                    linux_vt_backend::CellSnapshot::default(),
                    linux_vt_backend::CellSnapshot { text: "d".into(), ..Default::default() },
                ],
            ],
            scrollbar: vt::GhosttyTerminalScrollbar::default(),
            colors: vt::GhosttyRenderStateColors::default(),
        };

        let text = linux_snapshot_selection_text(
            &snapshot,
            ffi::ghostty_selection_s {
                top_left: ffi::ghostty_point_s {
                    tag: ffi::GHOSTTY_POINT_VIEWPORT,
                    coord: ffi::GHOSTTY_POINT_COORD_EXACT,
                    x: 0,
                    y: 0,
                },
                bottom_right: ffi::ghostty_point_s {
                    tag: ffi::GHOSTTY_POINT_VIEWPORT,
                    coord: ffi::GHOSTTY_POINT_COORD_EXACT,
                    x: 3,
                    y: 1,
                },
                rectangle: false,
            },
        );

        assert_eq!(text, "ab\nc  d");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_linux_snapshot_selection_text_preserves_rectangle_width() {
        let snapshot = linux_vt_backend::TerminalSnapshot {
            cols: 3,
            rows: 2,
            title: String::new(),
            pwd: String::new(),
            cursor: linux_vt_backend::CursorSnapshot::default(),
            rows_data: vec![
                vec![
                    linux_vt_backend::CellSnapshot { text: "a".into(), ..Default::default() },
                    linux_vt_backend::CellSnapshot::default(),
                    linux_vt_backend::CellSnapshot { text: "b".into(), ..Default::default() },
                ],
                vec![
                    linux_vt_backend::CellSnapshot::default(),
                    linux_vt_backend::CellSnapshot { text: "c".into(), ..Default::default() },
                    linux_vt_backend::CellSnapshot::default(),
                ],
            ],
            scrollbar: vt::GhosttyTerminalScrollbar::default(),
            colors: vt::GhosttyRenderStateColors::default(),
        };

        let text = linux_snapshot_selection_text(
            &snapshot,
            ffi::ghostty_selection_s {
                top_left: ffi::ghostty_point_s {
                    tag: ffi::GHOSTTY_POINT_VIEWPORT,
                    coord: ffi::GHOSTTY_POINT_COORD_EXACT,
                    x: 0,
                    y: 0,
                },
                bottom_right: ffi::ghostty_point_s {
                    tag: ffi::GHOSTTY_POINT_VIEWPORT,
                    coord: ffi::GHOSTTY_POINT_COORD_EXACT,
                    x: 2,
                    y: 1,
                },
                rectangle: true,
            },
        );

        assert_eq!(text, "a b\n c ");
    }
}
