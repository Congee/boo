use super::*;

fn take_global_receiver<T>(
    cell: &std::sync::OnceLock<std::sync::Mutex<std::sync::mpsc::Receiver<T>>>,
) -> std::sync::mpsc::Receiver<T> {
    cell.get()
        .and_then(|mutex| mutex.lock().ok())
        .map(|mut guard| {
            let (_, rx) = std::sync::mpsc::channel();
            std::mem::replace(&mut *guard, rx)
        })
        .unwrap_or_else(|| std::sync::mpsc::channel().1)
}

pub fn run_headless() {
    let mut app = BooApp::new_headless();
    loop {
        let _ = app.update(Message::Frame);
        std::thread::sleep(std::time::Duration::from_millis(16));
    }
}

impl BooApp {
    pub(crate) fn resolve_appearance_config(config: &config::Config) -> ResolvedAppearance {
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

    pub(crate) fn split_direction_from_str(direction: &str) -> bindings::SplitDirection {
        match direction {
            "right" => bindings::SplitDirection::Right,
            "down" => bindings::SplitDirection::Down,
            "left" => bindings::SplitDirection::Left,
            "up" => bindings::SplitDirection::Up,
            _ => bindings::SplitDirection::Right,
        }
    }

    fn new_headless() -> Self {
        Self::new_with_mode(true).0
    }

    fn new_with_mode(headless: bool) -> (Self, Task<Message>) {
        let backend = <backend::Backend as backend::TerminalBackend>::new(ptr::null_mut());

        let boo_config = launch::load_startup_config();
        let server = server::State::new(
            boo_config.control_socket.clone(),
            boo_config.remote_port,
            boo_config.remote_auth_key.clone(),
        );
        let bindings = bindings::Bindings::from_config(&boo_config);
        let appearance = Self::resolve_appearance_config(&boo_config);
        let (cell_width, cell_height) = terminal_metrics(appearance.font_size);

        #[cfg(target_os = "linux")]
        {
            (
                Self {
                    backend,
                    headless,
                    server,
                    parent_view: ptr::null_mut(),
                    scroll_rx: take_global_receiver(&SCROLL_RX),
                    key_event_rx: take_global_receiver(&KEY_EVENT_RX),
                    text_input_rx: take_global_receiver(&TEXT_INPUT_RX),
                    bindings,
                    dump_keys: std::env::args().any(|a| a == "--dump-keys"),
                    last_size: if headless {
                        Size::new(HEADLESS_WIDTH, HEADLESS_HEIGHT)
                    } else {
                        Size::new(0.0, 0.0)
                    },
                    last_mouse_pos: (0.0, 0.0),
                    divider_drag: None,
                    scrollbar_drag: false,
                    scrollbar_opacity: 0.0,
                    cell_width,
                    cell_height,
                    scrollbar: ffi::ghostty_action_scrollbar_s {
                        total: 0,
                        offset: 0,
                        len: 0,
                    },
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
                    headless,
                    server,
                    parent_view: ptr::null_mut(),
                    scroll_rx: take_global_receiver(&SCROLL_RX),
                    key_event_rx: take_global_receiver(&KEY_EVENT_RX),
                    text_input_rx: take_global_receiver(&TEXT_INPUT_RX),
                    bindings,
                    dump_keys: std::env::args().any(|a| a == "--dump-keys"),
                    last_size: if headless {
                        Size::new(HEADLESS_WIDTH, HEADLESS_HEIGHT)
                    } else {
                        Size::new(0.0, 0.0)
                    },
                    last_mouse_pos: (0.0, 0.0),
                    divider_drag: None,
                    scrollbar_drag: false,
                    scrollbar_opacity: 0.0,
                    cell_width,
                    cell_height,
                    scrollbar: ffi::ghostty_action_scrollbar_s {
                        total: 0,
                        offset: 0,
                        len: 0,
                    },
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
}
