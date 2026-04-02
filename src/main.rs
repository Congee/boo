mod appkit;
mod bindings;
mod config;
mod control;
mod ffi;
mod keymap;
mod splits;
mod tabs;

use iced::window;
use iced::{keyboard, mouse, Element, Event, Length, Size, Subscription, Task, Theme};
use std::ffi::{c_void, CString};
use std::ptr;

/// Shared state accessible from C callbacks via runtime_config.userdata.
struct CallbackState {
    surface: ffi::ghostty_surface_t,
    pending_split: Option<ffi::ghostty_action_split_direction_e>,
    pending_focus: Option<ffi::ghostty_action_goto_split_e>,
    pending_title: Option<String>,
    pending_close: bool,
}


fn main() {
    env_logger::init();

    let result = unsafe { ffi::ghostty_init(0, ptr::null_mut()) };
    if result != ffi::GHOSTTY_SUCCESS {
        eprintln!("Failed to initialize ghostty: error code {result}");
        std::process::exit(1);
    }

    log::info!("ghostty initialized");

    iced::application(GhosttyApp::new, GhosttyApp::update, GhosttyApp::view)
        .title("boo")
        .theme(GhosttyApp::theme)
        .subscription(GhosttyApp::subscription)
        .run()
        .unwrap();
}

struct GhosttyApp {
    app: ffi::ghostty_app_t,
    config: ffi::ghostty_config_t,
    tabs: tabs::TabManager,
    parent_nsview: *mut c_void,
    cb_state: Box<CallbackState>,
    ctl_rx: std::sync::mpsc::Receiver<control::ControlCmd>,
    bindings: bindings::Bindings,
    socket_path: Option<String>,
    dump_keys: bool,
    last_size: Size,
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

        // Load boo-specific config (not the default ghostty config)
        let config_dir = std::env::var("XDG_CONFIG_HOME")
            .unwrap_or_else(|_| format!("{}/.config", std::env::var("HOME").unwrap_or_default()));
        let config_path = std::path::PathBuf::from(config_dir).join("boo/config.ghostty");
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
                parent_nsview: ptr::null_mut(),
                cb_state,
                ctl_rx,
                bindings,
                socket_path: boo_config.control_socket.clone(),
                dump_keys: std::env::args().any(|a| a == "--dump-keys"),
                last_size: Size::new(0.0, 0.0),
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

        let event = match message {
            Message::Frame => return Task::none(),
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
                let Some(keycode) = keymap::physical_to_native_keycode(&physical_key) else {
                    return;
                };
                let mods = iced_mods_to_ghostty(&modifiers);

                // Get the character produced by this key for binding matching
                let key_char = match &modified_key {
                    keyboard::Key::Character(s) => s.chars().next(),
                    _ => None,
                };

                // Check boo's own bindings first (prefix key system)
                match self.bindings.handle_key(key_char, keycode, mods) {
                    bindings::KeyResult::Consumed(action) => {
                        if self.dump_keys {
                            log::info!("boo binding: {action:?}");
                        }
                        if let Some(action) = action {
                            self.dispatch_binding_action(action);
                        }
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
                self.cb_state.surface = self.tabs.focused_surface();
                self.relayout();
            }
            control::ControlCmd::NextTab => {
                self.tabs.next_tab();
                self.cb_state.surface = self.tabs.focused_surface();
                self.relayout();
            }
            control::ControlCmd::PrevTab => {
                self.tabs.prev_tab();
                self.cb_state.surface = self.tabs.focused_surface();
                self.relayout();
            }
            control::ControlCmd::SendKey { keyspec } => {
                self.inject_key(&keyspec);
            }
        }
    }

    fn inject_key(&self, keyspec: &str) {
        let surface = self.focused_surface();
        if surface.is_null() {
            return;
        }
        let (keycode, mods) = match parse_keyspec(keyspec) {
            Some(v) => v,
            None => {
                log::warn!("unknown keyspec: {keyspec}");
                return;
            }
        };
        let shifted = shifted_codepoint(keycode, mods);
        let text_str = if shifted > 0 && mods & ffi::GHOSTTY_MODS_CTRL == 0 {
            char::from_u32(shifted).map(|c| c.to_string())
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

    fn dispatch_binding_action(&mut self, action: bindings::Action) {
        match action {
            bindings::Action::NewSplit(dir) => self.create_split(dir),
            bindings::Action::GotoSplit(dir) => {
                self.switch_focus(dir);
            }
            bindings::Action::ResizeSplit(_dir, _amount) => {
                // TODO: implement split resize
                log::info!("resize_split: not yet implemented");
            }
            bindings::Action::NewTab => self.new_tab(),
            bindings::Action::NextTab => { self.tabs.next_tab(); self.relayout(); }
            bindings::Action::PrevTab => { self.tabs.prev_tab(); self.relayout(); }
            bindings::Action::ReloadConfig => {
                log::info!("reload_config triggered");
            }
        }
    }

    fn handle_mouse(&self, event: mouse::Event) {
        match event {
            mouse::Event::CursorMoved { position } => unsafe {
                ffi::ghostty_surface_mouse_pos(
                    self.focused_surface(),
                    position.x as f64,
                    position.y as f64,
                    ffi::GHOSTTY_MODS_NONE,
                );
            },
            mouse::Event::ButtonPressed(button) => unsafe {
                ffi::ghostty_surface_mouse_button(
                    self.focused_surface(),
                    ffi::ghostty_input_mouse_state_e::GHOSTTY_MOUSE_PRESS,
                    iced_button_to_ghostty(button),
                    ffi::GHOSTTY_MODS_NONE,
                );
            },
            mouse::Event::ButtonReleased(button) => unsafe {
                ffi::ghostty_surface_mouse_button(
                    self.focused_surface(),
                    ffi::ghostty_input_mouse_state_e::GHOSTTY_MOUSE_RELEASE,
                    iced_button_to_ghostty(button),
                    ffi::GHOSTTY_MODS_NONE,
                );
            },
            mouse::Event::WheelScrolled { delta } => {
                let (dx, dy) = match delta {
                    mouse::ScrollDelta::Lines { x, y } => (x as f64, y as f64),
                    mouse::ScrollDelta::Pixels { x, y } => (x as f64, y as f64),
                };
                unsafe {
                    ffi::ghostty_surface_mouse_scroll(
                        self.focused_surface(),
                        dx,
                        dy,
                        ffi::GHOSTTY_MODS_NONE,
                    );
                }
            }
            _ => {}
        }
    }

    fn scale_factor(&self) -> f64 {
        appkit::scale_factor()
    }

    fn handle_resize(&mut self, size: Size) {
        self.last_size = size;
        self.relayout();
    }

    fn init_surface(&mut self) {
        if !self.tabs.is_empty() {
            return;
        }
        let Some(cv) = appkit::content_view() else { return };
        self.parent_nsview = objc2::rc::Retained::as_ptr(&cv) as *mut c_void;
        let frame = cv.bounds();
        let (surface, nsview) = self.create_ghostty_surface(frame, true);
        let Some(surface) = surface else { return };
        self.tabs.add_initial_tab(surface, nsview);
        self.cb_state.surface = surface;
        log::info!("tab 0 created");
    }

    fn create_ghostty_surface(
        &self,
        frame: objc2_foundation::NSRect,
        is_first: bool,
    ) -> (Option<ffi::ghostty_surface_t>, *mut c_void) {
        let Some(parent_view) = appkit::content_view() else {
            return (None, ptr::null_mut());
        };
        let child = appkit::create_child_view(&parent_view, frame);
        let child_view = objc2::rc::Retained::as_ptr(&child) as *mut c_void;
        std::mem::forget(child);

        let scale = self.scale_factor();
        let mut config = unsafe { ffi::ghostty_surface_config_new() };
        config.userdata = &*self.cb_state as *const CallbackState as *mut c_void;
        config.platform_tag = ffi::ghostty_platform_e::GHOSTTY_PLATFORM_MACOS as i32;
        config.platform = ffi::ghostty_platform_u {
            macos: ffi::ghostty_platform_macos_s { nsview: child_view },
        };
        config.scale_factor = scale;
        config.context = if is_first {
            ffi::ghostty_surface_context_e::GHOSTTY_SURFACE_CONTEXT_WINDOW
        } else {
            ffi::ghostty_surface_context_e::GHOSTTY_SURFACE_CONTEXT_SPLIT
        };

        let surface = unsafe { ffi::ghostty_surface_new(self.app, &config) };
        if surface.is_null() {
            log::error!("failed to create ghostty surface");
            return (None, child_view);
        }

        (Some(surface), child_view)
    }

    fn create_split(&mut self, direction: ffi::ghostty_action_split_direction_e) {
        if self.parent_nsview.is_null() || self.tabs.is_empty() {
            return;
        }
        let parent_bounds = appkit::view_bounds(self.parent_nsview);
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
        let frame = objc2_foundation::NSRect::new(
            objc2_foundation::NSPoint::new(0.0, 0.0),
            objc2_foundation::NSSize::new(
                self.last_size.width as f64,
                self.last_size.height as f64,
            ),
        );
        let surfaces = self.tabs.layout_active(frame, scale);
        for (surface, w, h) in surfaces {
            unsafe {
                ffi::ghostty_surface_set_content_scale(surface, scale, scale);
                ffi::ghostty_surface_set_size(surface, w, h);
            }
        }
    }

    fn handle_surface_closed(&mut self) {
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
        log::info!("tab closed, {} remaining", self.tabs.len());
    }

    fn new_tab(&mut self) {
        if self.parent_nsview.is_null() {
            return;
        }
        let frame = appkit::view_bounds(self.parent_nsview);
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

    fn view(&self) -> Element<'_, Message> {
        iced::widget::Space::new()
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
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
    appkit::request_redraw();
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
                        appkit::set_window_title(s);
                        let cb = &mut *(ffi::ghostty_app_userdata(_app) as *mut CallbackState);
                        cb.pending_title = Some(s.to_owned());
                    }
                }
                true
            }
            GHOSTTY_ACTION_CELL_SIZE => {
                let payload: ffi::ghostty_action_cell_size_s = action.payload();
                appkit::set_resize_increments(payload.width as f64, payload.height as f64);
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
            GHOSTTY_ACTION_PRESENT_TERMINAL | GHOSTTY_ACTION_OPEN_CONFIG
            | GHOSTTY_ACTION_INITIAL_SIZE | GHOSTTY_ACTION_RENDER
            | GHOSTTY_ACTION_MOUSE_SHAPE | GHOSTTY_ACTION_MOUSE_VISIBILITY
            | GHOSTTY_ACTION_RENDERER_HEALTH | GHOSTTY_ACTION_PWD
            | GHOSTTY_ACTION_SET_TAB_TITLE | GHOSTTY_ACTION_COLOR_CHANGE
            | GHOSTTY_ACTION_RING_BELL | GHOSTTY_ACTION_QUIT_TIMER
            | GHOSTTY_ACTION_SIZE_LIMIT | GHOSTTY_ACTION_SCROLLBAR
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
    unsafe {
        use objc2_app_kit::NSPasteboard;
        use objc2_app_kit::NSPasteboardTypeString;

        let pb = NSPasteboard::generalPasteboard();
        let text = pb.stringForType(NSPasteboardTypeString);
        match text {
            Some(ns_str) => {
                let cstr = CString::new(ns_str.to_string()).unwrap_or_default();
                ffi::ghostty_surface_complete_clipboard_request(
                    surface, cstr.as_ptr(), state, true,
                );
            }
            None => {
                ffi::ghostty_surface_complete_clipboard_request(
                    surface, ptr::null(), state, true,
                );
            }
        }
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

        // Dispatch to main thread — this callback may be called from io thread
        let block = block2::RcBlock::new(move || {
            use objc2_app_kit::NSPasteboard;
            use objc2_foundation::NSString;

            let pb = NSPasteboard::generalPasteboard();
            pb.clearContents();
            let ns_str = NSString::from_str(&owned);
            // writeObjects expects NSArray<NSPasteboardWriting> — NSString conforms
            use objc2::msg_send;
            let array: *mut objc2::runtime::AnyObject = msg_send![
                objc2::runtime::AnyClass::get(c"NSArray").expect("NSArray"),
                arrayWithObject: &*ns_str
            ];
            let _: bool = msg_send![&*pb, writeObjects: array];
        });
        objc2_foundation::NSOperationQueue::mainQueue()
            .addOperationWithBlock(&block);
    }
}

unsafe extern "C" fn cb_close_surface(userdata: *mut c_void, _process_alive: bool) {
    log::info!("close surface");
    let cb = unsafe { &mut *(userdata as *mut CallbackState) };
    cb.pending_close = true;
}
