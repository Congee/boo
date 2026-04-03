mod bindings;
mod config;
mod control;
mod ffi;
mod keymap;
mod platform;
mod session;
mod splits;
mod tabs;
mod tmux;

use iced::widget::{column, container, row, text};
use iced::window;
use iced::{keyboard, mouse, Color, Element, Event, Font, Length, Size, Subscription, Task, Theme};
use std::ffi::{c_void, CStr, CString};
use std::ptr;

/// Status bar height in points.
const STATUS_BAR_HEIGHT: f64 = 20.0;

static SCROLL_RX: std::sync::OnceLock<std::sync::Mutex<std::sync::mpsc::Receiver<platform::ScrollEvent>>> =
    std::sync::OnceLock::new();

/// Shared state accessible from C callbacks via runtime_config.userdata.
struct CallbackState {
    surface: ffi::ghostty_surface_t,
    pending_split: Option<ffi::ghostty_action_split_direction_e>,
    pending_focus: Option<ffi::ghostty_action_goto_split_e>,
    pending_title: Option<String>,
    pending_close: bool,
    scrollbar: Option<ffi::ghostty_action_scrollbar_s>,
    search_total: Option<isize>,
    search_selected: Option<isize>,
    pending_pwd: Option<String>,
    pending_cell_size: Option<(f64, f64)>,
}


static STARTUP_SESSION: std::sync::OnceLock<String> = std::sync::OnceLock::new();

fn main() {
    env_logger::init();

    // Parse --session <name> from CLI args
    let args: Vec<String> = std::env::args().collect();
    if let Some(pos) = args.iter().position(|a| a == "--session") {
        if let Some(name) = args.get(pos + 1) {
            STARTUP_SESSION.set(name.clone()).ok();
        }
    }

    let result = unsafe { ffi::ghostty_init(0, ptr::null_mut()) };
    if result != ffi::GHOSTTY_SUCCESS {
        eprintln!("Failed to initialize ghostty: error code {result}");
        std::process::exit(1);
    }

    log::info!("ghostty initialized");

    let (scroll_tx, scroll_rx) = std::sync::mpsc::channel();
    platform::install_event_monitors(scroll_tx);
    // Store in a static so GhosttyApp::new can pick it up
    SCROLL_RX.set(std::sync::Mutex::new(scroll_rx)).ok();

    iced::application(GhosttyApp::new, GhosttyApp::update, GhosttyApp::view)
        .title("boo")
        .decorations(false)
        .transparent(true)
        .style(|_state, _theme| iced::theme::Style {
            background_color: Color::TRANSPARENT,
            text_color: Color::WHITE,
        })
        .theme(GhosttyApp::theme)
        .subscription(GhosttyApp::subscription)
        .run()
        .unwrap();
}

struct GhosttyApp {
    app: ffi::ghostty_app_t,
    config: ffi::ghostty_config_t,
    tabs: tabs::TabManager,
    parent_view: *mut c_void,
    #[cfg(target_os = "linux")]
    egl_state: Option<platform::EglState>,
    cb_state: Box<CallbackState>,
    ctl_rx: std::sync::mpsc::Receiver<control::ControlCmd>,
    scroll_rx: std::sync::mpsc::Receiver<platform::ScrollEvent>,
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
    copy_mode: Option<CopyModeState>,
    command_prompt: CommandPrompt,
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
    IcedEvent(Event),
}

impl GhosttyApp {
    fn new() -> (Self, Task<Message>) {
        let config = unsafe { ffi::ghostty_config_new() };
        assert!(!config.is_null(), "failed to create ghostty config");

        let config_path = config::ghostty_config_path();
        if config_path.exists() {
            let path_cstr = CString::new(config_path.to_str().unwrap()).unwrap();
            unsafe { ffi::ghostty_config_load_file(config, path_cstr.as_ptr()) };
            log::info!("loaded config: {}", config_path.display());
        } else {
            unsafe { ffi::ghostty_config_load_default_files(config) };
        }
        unsafe { ffi::ghostty_config_finalize(config) };

        let diag_count = unsafe { ffi::ghostty_config_diagnostics_count(config) };
        for i in 0..diag_count {
            let diag = unsafe { ffi::ghostty_config_get_diagnostic(config, i) };
            if !diag.message.is_null() {
                let msg = unsafe { std::ffi::CStr::from_ptr(diag.message) };
                log::warn!("config: {}", msg.to_string_lossy());
            }
        }

        let mut cb_state = Box::new(CallbackState {
            surface: ptr::null_mut(),
            pending_split: None,
            pending_focus: None,
            pending_title: None,
            pending_close: false,
            scrollbar: None,
            search_total: None,
            search_selected: None,
            pending_pwd: None,
            pending_cell_size: None,
        });

        let runtime_config = ffi::ghostty_runtime_config_s {
            userdata: &mut *cb_state as *mut CallbackState as *mut c_void,
            supports_selection_clipboard: false,
            wakeup_cb: Some(cb_wakeup),
            action_cb: Some(cb_action),
            read_clipboard_cb: Some(cb_read_clipboard),
            confirm_read_clipboard_cb: Some(cb_confirm_read_clipboard),
            write_clipboard_cb: Some(cb_write_clipboard),
            close_surface_cb: Some(cb_close_surface),
        };

        let app = unsafe { ffi::ghostty_app_new(&runtime_config, config) };
        assert!(!app.is_null(), "failed to create ghostty app");
        log::info!("ghostty app created");

        let boo_config = config::Config::load();
        let ctl_rx = control::start(boo_config.control_socket.as_deref());
        let bindings = bindings::Bindings::from_config(&boo_config);

        (
            Self {
                app,
                config,
                tabs: tabs::TabManager::new(),
                parent_view: ptr::null_mut(),
                #[cfg(target_os = "linux")]
                egl_state: None,
                cb_state,
                ctl_rx,
                scroll_rx: SCROLL_RX
                    .get()
                    .and_then(|m| m.lock().ok())
                    .map(|mut guard| {
                        // Take the receiver out of the mutex
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
                cell_width: 8.0,
                cell_height: 16.0,
                scrollbar: ffi::ghostty_action_scrollbar_s { total: 0, offset: 0, len: 0 },
                scrollbar_layer: ptr::null_mut(),
                search_active: false,
                search_query: String::new(),
                search_total: 0,
                search_selected: 0,
                pwd: String::new(),
                copy_mode: None,
                command_prompt: CommandPrompt::new(),
            },
            Task::none(),
        )
    }

    fn focused_surface(&self) -> ffi::ghostty_surface_t {
        self.tabs.focused_surface()
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        unsafe { ffi::ghostty_app_tick(self.app) };

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
                unsafe {
                    ffi::ghostty_surface_mouse_scroll(surface, scroll.dx, scroll.dy, mods);
                }
            }
        }

        if self.tabs.is_empty() {
            self.init_surface();
            return Task::none();
        }

        // Process pending actions from C callbacks
        if let Some(title) = self.cb_state.pending_title.take() {
            self.tabs.set_active_title(title);
        }
        if self.cb_state.pending_close {
            self.cb_state.pending_close = false;
            self.handle_surface_closed();
        }
        if let Some(dir) = self.cb_state.pending_split.take() {
            self.create_split(dir);
        }
        if let Some(dir) = self.cb_state.pending_focus.take() {
            self.switch_focus(dir);
        }

        // Process search callbacks
        if let Some(total) = self.cb_state.search_total.take() {
            self.search_total = total;
        }
        if let Some(selected) = self.cb_state.search_selected.take() {
            self.search_selected = selected;
        }

        if let Some((cw, ch)) = self.cb_state.pending_cell_size.take() {
            self.cell_width = cw;
            self.cell_height = ch;
        }
        if let Some(pwd) = self.cb_state.pending_pwd.take() {
            self.pwd = pwd;
        }
        if let Some(sb) = self.cb_state.scrollbar.take() {
            self.scrollbar = sb;
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
            Message::IcedEvent(event) => event,
        };

        let surface = self.focused_surface();
        if surface.is_null() {
            return Task::none();
        }

        match event {
            Event::Keyboard(kb_event) => self.handle_keyboard(kb_event),
            Event::Mouse(mouse_event) => self.handle_mouse(mouse_event),
            Event::Window(window::Event::Resized(size)) => {
                self.handle_resize(size);
            }
            Event::Window(window::Event::Focused) => {
                unsafe {
                    ffi::ghostty_surface_set_focus(surface, true);
                    ffi::ghostty_app_set_focus(self.app, true);
                };
            }
            Event::Window(window::Event::Unfocused) => {
                unsafe {
                    ffi::ghostty_surface_set_focus(surface, false);
                    ffi::ghostty_app_set_focus(self.app, false);
                };
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
                let translation_mods = if !surface.is_null() {
                    unsafe { ffi::ghostty_surface_key_translation_mods(surface, mods) }
                } else {
                    mods
                };

                // unshifted_codepoint: character with NO modifiers (matches macOS byApplyingModifiers:[])
                let unshifted_codepoint = key_to_codepoint(&key);

                // consumed_mods: from translation_mods, strip Ctrl and Cmd
                // (matching Swift: translationMods.subtracting([.control, .command]))
                let consumed_mods = translation_mods
                    & !(ffi::GHOSTTY_MODS_CTRL | ffi::GHOSTTY_MODS_SUPER);

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

                let consumed = unsafe { ffi::ghostty_surface_key(self.focused_surface(), key_event) };
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
                let key_event = ffi::ghostty_input_key_s {
                    action: ffi::ghostty_input_action_e::GHOSTTY_ACTION_RELEASE,
                    mods: iced_mods_to_ghostty(&modifiers),
                    consumed_mods: ffi::GHOSTTY_MODS_NONE,
                    keycode,
                    text: ptr::null(),
                    unshifted_codepoint: 0,
                    composing: false,
                };
                unsafe { ffi::ghostty_surface_key(self.focused_surface(), key_event) };
            }
            _ => {}
        }
    }

    fn handle_control_cmd(&mut self, cmd: control::ControlCmd) {
        match cmd {
            control::ControlCmd::DumpKeysOn => self.dump_keys = true,
            control::ControlCmd::DumpKeysOff => self.dump_keys = false,
            control::ControlCmd::Quit => std::process::exit(0),
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
                use ffi::ghostty_action_split_direction_e::*;
                let dir = match direction.as_str() {
                    "right" => GHOSTTY_SPLIT_DIRECTION_RIGHT,
                    "down" => GHOSTTY_SPLIT_DIRECTION_DOWN,
                    "left" => GHOSTTY_SPLIT_DIRECTION_LEFT,
                    "up" => GHOSTTY_SPLIT_DIRECTION_UP,
                    _ => GHOSTTY_SPLIT_DIRECTION_RIGHT,
                };
                self.create_split(dir);
            }
            control::ControlCmd::FocusSurface { index } => {
                let old = self.tabs.focused_surface();
                if let Some(tree) = self.tabs.active_tree_mut() {
                    tree.set_focus(index);
                }
                let new = self.tabs.focused_surface();
                if old != new {
                    unsafe {
                        if !old.is_null() { ffi::ghostty_surface_set_focus(old, false); }
                        if !new.is_null() { ffi::ghostty_surface_set_focus(new, true); }
                    }
                    self.cb_state.surface = new;
                }
            }
            control::ControlCmd::ListTabs { reply } => {
                let _ = reply.send(control::Response::Tabs {
                    tabs: self.tabs.tab_info(),
                });
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

        // Route through boo's binding system first (prefix key, etc.)
        match self.bindings.handle_key(key_char, keycode, mods, None) {
            bindings::KeyResult::Consumed(action) => {
                log::info!("ctl key consumed by boo: {action:?}");
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
        let surface = self.focused_surface();
        if surface.is_null() {
            return;
        }
        let text_str = if key_char.is_some() && mods & ffi::GHOSTTY_MODS_CTRL == 0 {
            key_char.map(|c| c.to_string())
        } else {
            None
        };
        let ctext = text_str.as_ref().and_then(|t| CString::new(t.as_str()).ok());
        let text_ptr = ctext.as_ref().map(|c| c.as_ptr()).unwrap_or(ptr::null());
        let unshifted = shifted_codepoint(keycode, 0);
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
        let consumed = unsafe { ffi::ghostty_surface_key(surface, ev) };
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
        let surface = self.focused_surface();
        if surface.is_null() {
            return None;
        }
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
        let mut text = ffi::ghostty_text_s {
            tl_px_x: 0.0, tl_px_y: 0.0,
            offset_start: 0, offset_len: 0,
            text: ptr::null(), text_len: 0,
        };
        let ok = unsafe { ffi::ghostty_surface_read_text(surface, sel, &mut text) };
        if ok && !text.text.is_null() && text.text_len > 0 {
            let slice = unsafe { std::slice::from_raw_parts(text.text as *const u8, text.text_len) };
            let s = std::str::from_utf8(slice).ok().map(|s| s.to_string());
            unsafe { ffi::ghostty_surface_free_text(surface, &mut text) };
            s
        } else {
            None
        }
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

    fn copy_mode_append_copy(&self) {
        let existing = platform::clipboard_read().unwrap_or_default();

        let Some(ref cm) = self.copy_mode else { return };
        let Some((anchor_row, anchor_col)) = cm.sel_anchor else { return };
        let surface = self.focused_surface();
        if surface.is_null() { return; }

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
        let mut text = ffi::ghostty_text_s {
            tl_px_x: 0.0, tl_px_y: 0.0,
            offset_start: 0, offset_len: 0,
            text: ptr::null(), text_len: 0,
        };
        let ok = unsafe { ffi::ghostty_surface_read_text(surface, sel, &mut text) };
        if ok && !text.text.is_null() && text.text_len > 0 {
            let slice = unsafe { std::slice::from_raw_parts(text.text as *const u8, text.text_len) };
            if let Ok(new_text) = std::str::from_utf8(slice) {
                let combined = format!("{existing}{new_text}");
                platform::clipboard_write(&combined);
                log::info!("copy mode: appended {} bytes to clipboard", text.text_len);
            }
            unsafe { ffi::ghostty_surface_free_text(surface, &mut text) };
        }
    }

    fn enter_copy_mode(&mut self) {
        let surface = self.focused_surface();
        if surface.is_null() {
            return;
        }
        // Get cursor position (returns screen points, y = bottom of cursor cell)
        let mut x: f64 = 0.0;
        let mut y: f64 = 0.0;
        let mut _w: f64 = 0.0;
        let mut h: f64 = 0.0;
        unsafe { ffi::ghostty_surface_ime_point(surface, &mut x, &mut y, &mut _w, &mut h) };

        // h from ime_point = cell_height in screen points
        let scale = self.scale_factor();
        let cell_w_pts = self.cell_width / scale;
        let cell_h_pts = if h > 0.0 { h } else { self.cell_height / scale };

        // ime_point x includes padding and is at cell midpoint; y is at cell bottom
        let col = if cell_w_pts > 0.0 { (x / cell_w_pts) as u32 } else { 0 };
        let row = if cell_h_pts > 0.0 { ((y - cell_h_pts) / cell_h_pts) as i64 } else { 0 };
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

    fn copy_mode_copy(&self) {
        let Some(ref cm) = self.copy_mode else { return };
        let Some((anchor_row, anchor_col)) = cm.sel_anchor else { return };
        let surface = self.focused_surface();
        if surface.is_null() {
            return;
        }

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

        let mut text = ffi::ghostty_text_s {
            tl_px_x: 0.0,
            tl_px_y: 0.0,
            offset_start: 0,
            offset_len: 0,
            text: ptr::null(),
            text_len: 0,
        };
        let ok = unsafe { ffi::ghostty_surface_read_text(surface, sel, &mut text) };
        if ok && !text.text.is_null() && text.text_len > 0 {
            let slice = unsafe { std::slice::from_raw_parts(text.text as *const u8, text.text_len) };
            if let Ok(s) = std::str::from_utf8(slice) {
                platform::clipboard_write(s);
                log::info!("copy mode: copied {} bytes", text.text_len);
            }
            unsafe { ffi::ghostty_surface_free_text(surface, &mut text) };
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
                    std::process::exit(0);
                }
                let active = self.tabs.active_index();
                let surfaces = self.tabs.remove_tab(active);
                for s in surfaces {
                    unsafe { ffi::ghostty_surface_free(s) };
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
                let new = self.tabs.focused_surface();
                self.cb_state.surface = new;
                unsafe { if !new.is_null() { ffi::ghostty_surface_set_focus(new, true); } }
            }
            bindings::Action::PreviousPane => {
                if let Some(tree) = self.tabs.active_tree_mut() {
                    tree.focus_prev();
                }
                let new = self.tabs.focused_surface();
                self.cb_state.surface = new;
                unsafe { if !new.is_null() { ffi::ghostty_surface_set_focus(new, true); } }
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
                // Reload ghostty config
                let new_config = unsafe { ffi::ghostty_config_new() };
                if !new_config.is_null() {
                    let path = config::ghostty_config_path();
                    if path.exists() {
                        let cstr = CString::new(path.to_str().unwrap_or_default()).unwrap();
                        unsafe { ffi::ghostty_config_load_file(new_config, cstr.as_ptr()) };
                    }
                    unsafe { ffi::ghostty_config_finalize(new_config) };
                    unsafe { ffi::ghostty_app_update_config(self.app, new_config) };
                    unsafe { ffi::ghostty_config_free(self.config) };
                    self.config = new_config;
                }
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

        use ffi::ghostty_action_split_direction_e::*;

        match cmd {
            "split-right" => self.dispatch_binding_action(bindings::Action::NewSplit(GHOSTTY_SPLIT_DIRECTION_RIGHT)),
            "split-down" => self.dispatch_binding_action(bindings::Action::NewSplit(GHOSTTY_SPLIT_DIRECTION_DOWN)),
            "split-left" => self.dispatch_binding_action(bindings::Action::NewSplit(GHOSTTY_SPLIT_DIRECTION_LEFT)),
            "split-up" => self.dispatch_binding_action(bindings::Action::NewSplit(GHOSTTY_SPLIT_DIRECTION_UP)),
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
                    let kv = format!("{key} = {val}\n");
                    let tmp = std::env::temp_dir().join("boo-set-config.tmp");
                    if std::fs::write(&tmp, &kv).is_ok() {
                        let new_config = unsafe { ffi::ghostty_config_new() };
                        if !new_config.is_null() {
                            // Load base config first
                            let base = config::ghostty_config_path();
                            if base.exists() {
                                let cstr = CString::new(base.to_str().unwrap_or_default()).unwrap();
                                unsafe { ffi::ghostty_config_load_file(new_config, cstr.as_ptr()) };
                            }
                            // Then load override
                            let cstr = CString::new(tmp.to_str().unwrap_or_default()).unwrap();
                            unsafe { ffi::ghostty_config_load_file(new_config, cstr.as_ptr()) };
                            unsafe { ffi::ghostty_config_finalize(new_config) };
                            let surface = self.focused_surface();
                            if !surface.is_null() {
                                unsafe { ffi::ghostty_surface_update_config(surface, new_config) };
                            }
                            unsafe { ffi::ghostty_config_free(self.config) };
                            self.config = new_config;
                        }
                        let _ = std::fs::remove_file(&tmp);
                    }
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
        self.cb_state.surface = self.tabs.focused_surface();
        self.relayout();
    }

    fn ghostty_binding_action(&self, action: &str) {
        let surface = self.focused_surface();
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

    fn send_search(&self) {
        self.ghostty_binding_action(&format!("search:{}", self.search_query));
    }

    fn scroll_to_mouse_y(&self, y: f64) {
        let terminal_h = self.last_size.height as f64 - STATUS_BAR_HEIGHT;
        if terminal_h <= 0.0 || self.scrollbar.total <= self.scrollbar.len {
            return;
        }
        let fraction = (y / terminal_h).clamp(0.0, 1.0);
        let max_offset = self.scrollbar.total.saturating_sub(self.scrollbar.len);
        let target_row = (fraction * max_offset as f64) as usize;
        self.ghostty_binding_action(&format!("scroll_to_row:{target_row}"));
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
                unsafe {
                    ffi::ghostty_surface_mouse_pos(
                        self.focused_surface(),
                        position.x as f64,
                        position.y as f64,
                        ffi::GHOSTTY_MODS_NONE,
                    );
                }
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
                    let old = self.focused_surface();
                    if let Some(tree) = self.tabs.active_tree_mut() {
                        if tree.focus_at(frame, point) {
                            let new = self.tabs.focused_surface();
                            unsafe {
                                if !old.is_null() { ffi::ghostty_surface_set_focus(old, false); }
                                if !new.is_null() { ffi::ghostty_surface_set_focus(new, true); }
                            }
                            self.cb_state.surface = new;
                        }
                    }
                }
                unsafe {
                    ffi::ghostty_surface_mouse_button(
                        self.focused_surface(),
                        ffi::ghostty_input_mouse_state_e::GHOSTTY_MOUSE_PRESS,
                        iced_button_to_ghostty(button),
                        ffi::GHOSTTY_MODS_NONE,
                    );
                }
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
                unsafe {
                    ffi::ghostty_surface_mouse_button(
                        self.focused_surface(),
                        ffi::ghostty_input_mouse_state_e::GHOSTTY_MOUSE_RELEASE,
                        iced_button_to_ghostty(button),
                        ffi::GHOSTTY_MODS_NONE,
                    );
                }
            }
            mouse::Event::WheelScrolled { delta } => {
                // macOS: handled via native NSEvent monitor for precision + momentum.
                // Linux: use iced's scroll event directly.
                #[cfg(target_os = "linux")]
                {
                    let surface = self.focused_surface();
                    if !surface.is_null() {
                        let (dx, dy) = match delta {
                            mouse::ScrollDelta::Lines { x, y } => (x as f64, y as f64),
                            mouse::ScrollDelta::Pixels { x, y } => (x as f64, y as f64),
                        };
                        if self.scrollbar.total > self.scrollbar.len {
                            self.scrollbar_opacity = 1.0;
                        }
                        unsafe {
                            ffi::ghostty_surface_mouse_scroll(surface, dx, dy, 0);
                        }
                    }
                }
                #[cfg(target_os = "macos")]
                let _ = delta; // suppress unused warning
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
            return;
        }
        self.parent_view = cv;
        platform::set_window_transparent();
        self.scrollbar_layer = platform::create_scrollbar_layer();
        let frame = platform::view_bounds(cv);
        let (surface, child_view) = self.create_ghostty_surface(frame, true);
        let Some(surface) = surface else { return };
        self.tabs.add_initial_tab(surface, child_view);
        self.cb_state.surface = surface;
        log::info!("tab 0 created");

        // Load startup session if --session was specified
        if let Some(name) = STARTUP_SESSION.get() {
            self.load_session(name);
        }
    }

    fn create_ghostty_surface(
        &mut self,
        frame: platform::Rect,
        is_first: bool,
    ) -> (Option<ffi::ghostty_surface_t>, *mut c_void) {
        self.create_ghostty_surface_with(frame, is_first, None, None)
    }

    #[allow(unused_variables)]
    fn create_ghostty_surface_with(
        &mut self,
        frame: platform::Rect,
        is_first: bool,
        command: Option<&CStr>,
        working_directory: Option<&CStr>,
    ) -> (Option<ffi::ghostty_surface_t>, *mut c_void) {
        let scale = self.scale_factor();
        let mut config = unsafe { ffi::ghostty_surface_config_new() };
        config.userdata = &*self.cb_state as *const CallbackState as *mut c_void;
        config.platform_tag = platform::platform_tag();
        config.scale_factor = scale;
        config.context = if is_first {
            ffi::ghostty_surface_context_e::GHOSTTY_SURFACE_CONTEXT_WINDOW
        } else {
            ffi::ghostty_surface_context_e::GHOSTTY_SURFACE_CONTEXT_SPLIT
        };
        if let Some(cmd) = command {
            config.command = cmd.as_ptr();
        }
        if let Some(wd) = working_directory {
            config.working_directory = wd.as_ptr();
        }

        // Platform-specific: set up the native view / EGL context
        #[cfg(target_os = "macos")]
        let child_view = {
            let parent_handle = platform::content_view_handle();
            if parent_handle.is_null() {
                return (None, ptr::null_mut());
            }
            let cv = platform::create_child_view(parent_handle, frame);
            config.platform = platform::platform_config(cv);
            cv
        };

        #[cfg(target_os = "linux")]
        let child_view = {
            let egl = self.egl_state.get_or_insert_with(|| {
                platform::EglState::new().expect("failed to create EGL context")
            });
            config.platform = platform::platform_config(egl);
            ptr::null_mut() // no native child view on Linux
        };

        let surface = unsafe { ffi::ghostty_surface_new(self.app, &config) };
        if surface.is_null() {
            log::error!("failed to create ghostty surface");
            return (None, child_view);
        }
        platform::set_view_layer_transparent(child_view);

        // Release EGL from main thread so renderer thread can claim it
        #[cfg(target_os = "linux")]
        if let Some(ref egl) = self.egl_state {
            egl.release_current();
        }

        (Some(surface), child_view)
    }

    fn create_split(&mut self, direction: ffi::ghostty_action_split_direction_e) {
        if self.parent_view.is_null() || self.tabs.is_empty() {
            return;
        }
        let parent_bounds = platform::view_bounds(self.parent_view);
        let split_dir = match direction {
            ffi::ghostty_action_split_direction_e::GHOSTTY_SPLIT_DIRECTION_RIGHT
            | ffi::ghostty_action_split_direction_e::GHOSTTY_SPLIT_DIRECTION_LEFT => {
                splits::Direction::Horizontal
            }
            _ => splits::Direction::Vertical,
        };

        let (surface, nsview) = self.create_ghostty_surface(parent_bounds, false);
        let Some(surface) = surface else { return };

        let old_focused = self.tabs.focused_surface();
        if let Some(tree) = self.tabs.active_tree_mut() {
            tree.split_focused(split_dir, surface, nsview);
        }
        self.cb_state.surface = surface;

        if !old_focused.is_null() {
            unsafe { ffi::ghostty_surface_set_focus(old_focused, false) };
        }

        self.relayout();
        log::info!("split created");
    }

    fn switch_focus(&mut self, dir: ffi::ghostty_action_goto_split_e) {
        let old = self.tabs.focused_surface();
        if let Some(tree) = self.tabs.active_tree_mut() {
            match dir {
                ffi::ghostty_action_goto_split_e::GHOSTTY_GOTO_SPLIT_NEXT => tree.focus_next(),
                ffi::ghostty_action_goto_split_e::GHOSTTY_GOTO_SPLIT_PREVIOUS => tree.focus_prev(),
            }
        }
        let new = self.tabs.focused_surface();
        if old != new {
            unsafe {
                if !old.is_null() { ffi::ghostty_surface_set_focus(old, false); }
                if !new.is_null() { ffi::ghostty_surface_set_focus(new, true); }
            }
            self.cb_state.surface = new;
        }
    }

    fn relayout(&self) {
        if self.tabs.is_empty() || self.last_size.width == 0.0 {
            return;
        }
        let scale = self.scale_factor();
        let frame = self.terminal_frame();
        let surfaces = self.tabs.layout_active(frame, scale);
        for (surface, w, h) in surfaces {
            unsafe {
                ffi::ghostty_surface_set_content_scale(surface, scale, scale);
                ffi::ghostty_surface_set_size(surface, w, h);
            }
        }
    }

    fn handle_surface_closed(&mut self) {
        // Remove just the focused leaf from the split tree
        if let Some(tree) = self.tabs.active_tree_mut() {
            if let Some((surface, nsview)) = tree.remove_focused() {
                unsafe { ffi::ghostty_surface_free(surface) };
                platform::remove_view(nsview);

                if tree.len() == 0 {
                    // Tab is empty — remove it or exit
                    if self.tabs.len() <= 1 {
                        std::process::exit(0);
                    }
                    let active = self.tabs.active_index();
                    self.tabs.remove_tab(active);
                }

                self.cb_state.surface = self.tabs.focused_surface();
                if !self.cb_state.surface.is_null() {
                    unsafe { ffi::ghostty_surface_set_focus(self.cb_state.surface, true) };
                }
                self.relayout();
                log::info!(
                    "surface closed, {} surfaces in tab, {} tabs",
                    self.tabs.active_tree().map(|t| t.len()).unwrap_or(0),
                    self.tabs.len()
                );
                return;
            }
        }

        // Fallback: no active tree or focused leaf not found
        if self.tabs.len() <= 1 {
            std::process::exit(0);
        }
        let active = self.tabs.active_index();
        let surfaces = self.tabs.remove_tab(active);
        for s in surfaces {
            unsafe { ffi::ghostty_surface_free(s) };
        }
        self.cb_state.surface = self.tabs.focused_surface();
        self.relayout();
    }

    fn new_tab(&mut self) {
        if self.parent_view.is_null() {
            return;
        }
        let frame = platform::view_bounds(self.parent_view);
        let (surface, nsview) = self.create_ghostty_surface(frame, false);
        let Some(surface) = surface else { return };

        let old = self.tabs.focused_surface();
        if !old.is_null() {
            unsafe { ffi::ghostty_surface_set_focus(old, false) };
        }

        let idx = self.tabs.new_tab(surface, nsview);
        self.cb_state.surface = surface;
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
                    let (surface, nsview) = self.create_ghostty_surface_with(
                        frame, false,
                        cmd_cstr.as_deref(),
                        wd_cstr.as_deref(),
                    );
                    let Some(surface) = surface else { continue };
                    let old = self.tabs.focused_surface();
                    if !old.is_null() {
                        unsafe { ffi::ghostty_surface_set_focus(old, false) };
                    }
                    self.tabs.new_tab(surface, nsview);
                    self.cb_state.surface = surface;
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

                    let (surface, nsview) = self.create_ghostty_surface_with(
                        frame, false,
                        cmd_cstr.as_deref(),
                        wd_cstr.as_deref(),
                    );
                    let Some(surface) = surface else { continue };
                    if let Some(tree) = self.tabs.active_tree_mut() {
                        tree.split_focused_with_ratio(split_dir, surface, nsview, ratio);
                    }
                    self.cb_state.surface = surface;
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
                        .font(Font::MONOSPACE)
                        .size(13)
                        .color(Color::from_rgb(0.9, 0.9, 0.9)),
                )
                .style(|_: &Theme| container::Style {
                    background: Some(iced::Background::Color(Color::from_rgba(
                        0.15, 0.15, 0.15, 0.95,
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
        // Terminal area — NSViews and scrollbar CALayer sit on top
        main_col = main_col.push(
            iced::widget::Space::new()
                .width(Length::Fill)
                .height(Length::Fill),
        );

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
                            text(label).font(Font::MONOSPACE).size(13).color(fg),
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
                        .font(Font::MONOSPACE)
                        .size(13)
                        .color(Color::from_rgb(0.9, 0.9, 0.9)),
                )
                .style(|_: &Theme| container::Style {
                    background: Some(iced::Background::Color(Color::from_rgba(
                        0.15, 0.15, 0.15, 0.95,
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
                            .font(Font::MONOSPACE)
                            .size(13)
                            .color(Color::from_rgb(0.8, 0.8, 0.8)),
                        iced::widget::Space::new().width(Length::Fill),
                        text(status_right)
                            .font(Font::MONOSPACE)
                            .size(13)
                            .color(Color::from_rgb(0.6, 0.6, 0.6)),
                    ]
                    .width(Length::Fill),
                )
                .style(|_: &Theme| container::Style {
                    background: Some(iced::Background::Color(Color::from_rgb(0.12, 0.12, 0.12))),
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
        let tabs = self.tabs.tab_info();
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

impl Drop for GhosttyApp {
    fn drop(&mut self) {
        for surface in self.tabs.all_surfaces() {
            unsafe { ffi::ghostty_surface_free(surface) };
        }
        unsafe {
            ffi::ghostty_app_free(self.app);
            ffi::ghostty_config_free(self.config);
        }
        control::cleanup(self.socket_path.as_deref());
    }
}

/// For control pipe injection: compute the character for a keycode+mods combo.
fn shifted_codepoint(keycode: u32, mods: i32) -> u32 {
    let has_shift = mods & ffi::GHOSTTY_MODS_SHIFT != 0;
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
    Some((keycode, mods))
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

// --- Runtime callbacks ---

unsafe extern "C" fn cb_wakeup(_userdata: *mut c_void) {
    platform::request_redraw();
}

unsafe extern "C" fn cb_action(
    _app: ffi::ghostty_app_t,
    _target: ffi::ghostty_target_s,
    action: ffi::ghostty_action_s,
) -> bool {
    use ffi::ghostty_action_tag_e::*;
    log::debug!("action: {:?}", action.tag);
    unsafe {
        match action.tag {
            GHOSTTY_ACTION_SET_TITLE => {
                let payload: ffi::ghostty_action_set_title_s = action.payload();
                if !payload.title.is_null() {
                    let title = std::ffi::CStr::from_ptr(payload.title);
                    if let Ok(s) = title.to_str() {
                        platform::set_window_title(s);
                        let cb = &mut *(ffi::ghostty_app_userdata(_app) as *mut CallbackState);
                        cb.pending_title = Some(s.to_owned());
                    }
                }
                true
            }
            GHOSTTY_ACTION_CELL_SIZE => {
                let payload: ffi::ghostty_action_cell_size_s = action.payload();
                platform::set_resize_increments(payload.width as f64, payload.height as f64);
                let cb = &mut *(ffi::ghostty_app_userdata(_app) as *mut CallbackState);
                cb.pending_cell_size = Some((payload.width as f64, payload.height as f64));
                true
            }
            GHOSTTY_ACTION_QUIT => {
                std::process::exit(0);
            }
            GHOSTTY_ACTION_NEW_SPLIT => {
                let cb = &mut *(ffi::ghostty_app_userdata(_app) as *mut CallbackState);
                cb.pending_split = Some(action.payload());
                true
            }
            GHOSTTY_ACTION_GOTO_SPLIT => {
                let cb = &mut *(ffi::ghostty_app_userdata(_app) as *mut CallbackState);
                cb.pending_focus = Some(action.payload());
                true
            }
            GHOSTTY_ACTION_SCROLLBAR => {
                let cb = &mut *(ffi::ghostty_app_userdata(_app) as *mut CallbackState);
                cb.scrollbar = Some(action.payload());
                true
            }
            GHOSTTY_ACTION_SEARCH_TOTAL => {
                let cb = &mut *(ffi::ghostty_app_userdata(_app) as *mut CallbackState);
                let payload: ffi::ghostty_action_search_total_s = action.payload();
                cb.search_total = Some(payload.total);
                true
            }
            GHOSTTY_ACTION_SEARCH_SELECTED => {
                let cb = &mut *(ffi::ghostty_app_userdata(_app) as *mut CallbackState);
                let payload: ffi::ghostty_action_search_selected_s = action.payload();
                cb.search_selected = Some(payload.selected);
                true
            }
            GHOSTTY_ACTION_PWD => {
                let cb = &mut *(ffi::ghostty_app_userdata(_app) as *mut CallbackState);
                let payload: ffi::ghostty_action_pwd_s = action.payload();
                if !payload.pwd.is_null() {
                    if let Ok(s) = std::ffi::CStr::from_ptr(payload.pwd).to_str() {
                        cb.pending_pwd = Some(s.to_owned());
                    }
                }
                true
            }
            GHOSTTY_ACTION_START_SEARCH | GHOSTTY_ACTION_END_SEARCH => true,
            GHOSTTY_ACTION_PRESENT_TERMINAL | GHOSTTY_ACTION_OPEN_CONFIG
            | GHOSTTY_ACTION_INITIAL_SIZE | GHOSTTY_ACTION_RENDER
            | GHOSTTY_ACTION_MOUSE_SHAPE | GHOSTTY_ACTION_MOUSE_VISIBILITY
            | GHOSTTY_ACTION_RENDERER_HEALTH
            | GHOSTTY_ACTION_SET_TAB_TITLE | GHOSTTY_ACTION_COLOR_CHANGE
            | GHOSTTY_ACTION_RING_BELL | GHOSTTY_ACTION_QUIT_TIMER
            | GHOSTTY_ACTION_SIZE_LIMIT
            | GHOSTTY_ACTION_SECURE_INPUT | GHOSTTY_ACTION_KEY_SEQUENCE
            | GHOSTTY_ACTION_KEY_TABLE | GHOSTTY_ACTION_CLOSE_TAB
            | GHOSTTY_ACTION_FLOAT_WINDOW | GHOSTTY_ACTION_RESET_WINDOW_SIZE => true,
            other => {
                log::debug!("unhandled action: {:?}", other);
                true // acknowledge all actions to prevent ghostty from taking fallback paths
            }
        }
    }
}

unsafe extern "C" fn cb_read_clipboard(
    userdata: *mut c_void,
    _clipboard: ffi::ghostty_clipboard_e,
    state: *mut c_void,
) -> bool {
    let cb = unsafe { &*(userdata as *const CallbackState) };
    let surface = cb.surface;
    if surface.is_null() {
        return false;
    }
    match platform::clipboard_read() {
        Some(text) => {
            let cstr = CString::new(text).unwrap_or_default();
            unsafe {
                ffi::ghostty_surface_complete_clipboard_request(
                    surface, cstr.as_ptr(), state, true,
                );
            }
        }
        None => unsafe {
            ffi::ghostty_surface_complete_clipboard_request(
                surface, ptr::null(), state, true,
            );
        },
    }
    true
}

unsafe extern "C" fn cb_confirm_read_clipboard(
    userdata: *mut c_void,
    content: *const std::os::raw::c_char,
    state: *mut c_void,
    _request: ffi::ghostty_clipboard_request_e,
) {
    let cb = unsafe { &*(userdata as *const CallbackState) };
    if !cb.surface.is_null() && !content.is_null() {
        unsafe {
            ffi::ghostty_surface_complete_clipboard_request(cb.surface, content, state, true);
        }
    }
}

unsafe extern "C" fn cb_write_clipboard(
    _userdata: *mut c_void,
    _clipboard: ffi::ghostty_clipboard_e,
    content: *const ffi::ghostty_clipboard_content_s,
    count: usize,
    _confirm: bool,
) {
    if count == 0 || content.is_null() {
        return;
    }
    unsafe {
        let first = &*content;
        if first.data.is_null() {
            return;
        }
        let text = std::ffi::CStr::from_ptr(first.data);
        let Ok(owned) = text.to_str().map(|s| s.to_owned()) else { return };

        // May be called from io thread — use thread-safe clipboard write
        platform::clipboard_write_from_thread(owned);
    }
}

unsafe extern "C" fn cb_close_surface(userdata: *mut c_void, _process_alive: bool) {
    log::info!("close surface");
    let cb = unsafe { &mut *(userdata as *mut CallbackState) };
    cb.pending_close = true;
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
        GhosttyApp::compute_selection_rects_static(
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
}
