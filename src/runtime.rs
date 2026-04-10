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
    let (wake_tx, wake_rx) = std::sync::mpsc::sync_channel(1);
    crate::install_headless_waker(wake_tx);
    loop {
        {
            let _scope =
                crate::profiling::scope("server.headless.update", crate::profiling::Kind::Cpu);
            let _ = app.update(Message::Frame);
        }
        let sleep_duration = if app.backend.has_pending_terminal_work()
            || app.remote_dirty
            || app.interactive_activity_epoch.elapsed() < INTERACTIVE_ACTIVITY_WINDOW
        {
            std::time::Duration::from_millis(1)
        } else {
            std::time::Duration::from_millis(16)
        };
        {
            let _scope =
                crate::profiling::scope("server.headless.sleep", crate::profiling::Kind::Wait);
            let _ = wake_rx.recv_timeout(sleep_duration);
        }
    }
}

impl BooApp {
    pub(crate) fn resolve_appearance_config(config: &config::Config) -> ResolvedAppearance {
        let mut font_fallbacks = Vec::new();
        let mut seen_families = std::collections::HashSet::new();
        if let Some(primary) = config.font_family.as_deref() {
            seen_families.insert(primary.to_ascii_lowercase());
        }
        for family in &config.font_fallbacks {
            let key = family.to_ascii_lowercase();
            if seen_families.insert(key) {
                font_fallbacks.push(leak_font_family(family));
            }
        }
        for family in platform_default_font_fallbacks(config.font_family.as_deref()) {
            if seen_families.insert(family.to_ascii_lowercase()) {
                font_fallbacks.push(family);
            }
        }

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
            font_fallbacks,
            font_size: config.font_size.unwrap_or(DEFAULT_TERMINAL_FONT_SIZE),
            background_opacity: config
                .background_opacity
                .unwrap_or(DEFAULT_BACKGROUND_OPACITY)
                .clamp(0.0, 1.0),
            background_opacity_cells: config.background_opacity_cells,
            terminal_foreground: config.foreground.unwrap_or(DEFAULT_TERMINAL_FOREGROUND),
            terminal_background: config.background.unwrap_or(DEFAULT_TERMINAL_BACKGROUND),
            terminal_palette: config.palette,
            cursor_color: config.cursor_color.unwrap_or(DEFAULT_CURSOR_COLOR),
            selection_background: config
                .selection_background
                .unwrap_or(DEFAULT_SELECTION_BACKGROUND),
            selection_foreground: config
                .selection_foreground
                .unwrap_or(DEFAULT_SELECTION_FOREGROUND),
            cursor_text_color: config
                .cursor_text_color
                .unwrap_or(DEFAULT_CURSOR_TEXT_COLOR),
            url_color: config.url_color.unwrap_or(DEFAULT_URL_COLOR),
            active_tab_foreground: config
                .active_tab_foreground
                .unwrap_or(DEFAULT_ACTIVE_TAB_FOREGROUND),
            active_tab_background: config
                .active_tab_background
                .unwrap_or(DEFAULT_ACTIVE_TAB_BACKGROUND),
            inactive_tab_foreground: config
                .inactive_tab_foreground
                .unwrap_or(DEFAULT_INACTIVE_TAB_FOREGROUND),
            inactive_tab_background: config
                .inactive_tab_background
                .unwrap_or(DEFAULT_INACTIVE_TAB_BACKGROUND),
            cursor_style: config
                .cursor_style
                .map(config::CursorStyle::vt_visual_style),
            cursor_blink: config.cursor_blink,
            cursor_blink_interval: std::time::Duration::from_nanos(config.cursor_blink_interval_ns),
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
                    paste_buffers: Vec::new(),
                    marked_pane_id: None,
                    display_panes_active: false,
                    choose_buffer_active: false,
                    choose_buffer_selected: 0,
                    choose_tree_active: false,
                    choose_tree_selected: 0,
                    find_window_active: false,
                    find_window_query: String::new(),
                    find_window_selected: 0,
                    copy_mode: None,
                    command_prompt: CommandPrompt::new(),
                    terminal_font_family: appearance.font_family,
                    terminal_font_fallbacks: appearance.font_fallbacks.clone(),
                    terminal_font_size: appearance.font_size,
                    background_opacity: appearance.background_opacity,
                    background_opacity_cells: appearance.background_opacity_cells,
                    terminal_foreground: appearance.terminal_foreground,
                    terminal_background: appearance.terminal_background,
                    terminal_palette: appearance.terminal_palette,
                    cursor_color: appearance.cursor_color,
                    selection_background: appearance.selection_background,
                    selection_foreground: appearance.selection_foreground,
                    cursor_text_color: appearance.cursor_text_color,
                    url_color: appearance.url_color,
                    active_tab_foreground: appearance.active_tab_foreground,
                    active_tab_background: appearance.active_tab_background,
                    inactive_tab_foreground: appearance.inactive_tab_foreground,
                    inactive_tab_background: appearance.inactive_tab_background,
                    cursor_style: appearance.cursor_style,
                    cursor_blink: appearance.cursor_blink,
                    cursor_blink_interval: appearance.cursor_blink_interval,
                    cursor_blink_epoch: std::time::Instant::now(),
                    appearance_revision: 1,
                    surface_initialized_once: false,
                    app_focused: true,
                    remote_dirty: true,
                    interactive_activity_epoch: std::time::Instant::now(),
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
                    paste_buffers: Vec::new(),
                    marked_pane_id: None,
                    display_panes_active: false,
                    choose_buffer_active: false,
                    choose_buffer_selected: 0,
                    choose_tree_active: false,
                    choose_tree_selected: 0,
                    find_window_active: false,
                    find_window_query: String::new(),
                    find_window_selected: 0,
                    copy_mode: None,
                    command_prompt: CommandPrompt::new(),
                    terminal_font_family: appearance.font_family,
                    terminal_font_fallbacks: appearance.font_fallbacks.clone(),
                    terminal_font_size: appearance.font_size,
                    background_opacity: appearance.background_opacity,
                    background_opacity_cells: appearance.background_opacity_cells,
                    terminal_foreground: appearance.terminal_foreground,
                    terminal_background: appearance.terminal_background,
                    terminal_palette: appearance.terminal_palette,
                    cursor_color: appearance.cursor_color,
                    selection_background: appearance.selection_background,
                    selection_foreground: appearance.selection_foreground,
                    cursor_text_color: appearance.cursor_text_color,
                    url_color: appearance.url_color,
                    active_tab_foreground: appearance.active_tab_foreground,
                    active_tab_background: appearance.active_tab_background,
                    inactive_tab_foreground: appearance.inactive_tab_foreground,
                    inactive_tab_background: appearance.inactive_tab_background,
                    cursor_style: appearance.cursor_style,
                    cursor_blink: appearance.cursor_blink,
                    cursor_blink_interval: appearance.cursor_blink_interval,
                    cursor_blink_epoch: std::time::Instant::now(),
                    appearance_revision: 1,
                    surface_initialized_once: false,
                    app_focused: true,
                    remote_dirty: true,
                    interactive_activity_epoch: std::time::Instant::now(),
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

impl BooApp {
    pub(crate) fn focused_surface(&self) -> ffi::ghostty_surface_t {
        self.server.tabs.focused_pane().surface()
    }

    pub(crate) fn set_pane_focus(&self, pane: PaneHandle, focused: bool) {
        self.backend.set_surface_focus(pane.surface(), focused);
        #[cfg(target_os = "macos")]
        if focused && pane.surface().is_null() {
            platform::focus_view(pane.view());
        }
    }

    pub(crate) fn handle_command_finished(&mut self, event: CommandFinishedEvent) {
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

    pub(crate) fn resize_pane_backend(
        &mut self,
        pane: PaneHandle,
        scale: f64,
        width: u32,
        height: u32,
    ) {
        self.backend.resize_pane(
            pane,
            scale,
            width,
            height,
            self.cell_width,
            self.cell_height,
        );
    }

    pub(crate) fn free_pane_backend(&mut self, pane: PaneHandle) {
        self.backend.free_pane(pane);
    }

    pub(crate) fn focused_cursor_cell_position(&self) -> Option<(u32, i64, f64)> {
        let scale = self.scale_factor();
        let cell_w_pts = self.cell_width / scale;
        let mut cell_h_pts = self.cell_height / scale;
        if self.focused_surface().is_null() {
            return self
                .backend
                .render_snapshot(self.server.tabs.focused_pane().id())
                .map(|snapshot| {
                    (
                        snapshot.cursor.x as u32,
                        self.scrollbar.offset as i64 + snapshot.cursor.y as i64,
                        cell_h_pts,
                    )
                });
        }
        let focused_pane = self.server.tabs.focused_pane();
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
            self.backend
                .render_snapshot(self.server.tabs.focused_pane().id())
                .map(|snapshot| {
                    (
                        snapshot.cursor.x as u32,
                        self.scrollbar.offset as i64 + snapshot.cursor.y as i64,
                        cell_h_pts,
                    )
                })
        }
    }

    pub(crate) fn update_text_input_cursor_rect(&self) {
        if self.headless {
            return;
        }
        #[cfg(target_os = "macos")]
        {
            let rect = if self.focused_surface().is_null() {
                self.backend
                    .ime_point(self.server.tabs.focused_pane())
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

    pub(crate) fn poll_backend(&mut self) {
        let mut remote_dirty = false;
        let active_id = self.server.tabs.focused_pane().id();
        let active_pane_ids: Vec<pane::PaneId> = self
            .server
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
        remote_dirty |= poll.terminal_dirty;
        for running_command in poll.running_commands.iter().cloned() {
            remote_dirty |= self.server.tabs.set_running_command_for_pane(
                running_command.pane_id,
                Some(tabs::RunningCommand {
                    command: running_command.command,
                }),
            );
        }
        for pane_id in &active_pane_ids {
            if !poll
                .running_commands
                .iter()
                .any(|running| running.pane_id == *pane_id)
            {
                remote_dirty |= self
                    .server
                    .tabs
                    .set_running_command_for_pane(*pane_id, None);
            }
        }
        for finished_command in poll.finished_commands {
            self.handle_command_finished(CommandFinishedEvent {
                exit_code: finished_command.exit_code,
                duration_ns: finished_command.duration_ns,
            });
        }
        if self.desktop_notifications_enabled {
            for notification in poll.desktop_notifications {
                platform::send_desktop_notification(&notification.title, &notification.body);
            }
        }
        if let Some(pwd) = poll.active_pwd {
            if self.pwd != pwd {
                self.pwd = pwd;
                remote_dirty = true;
            }
        }
        if let Some(title) = poll.active_title {
            if self.server.tabs.active_title() != Some(title.as_str()) {
                self.server.tabs.set_active_title(title);
                remote_dirty = true;
            }
        }
        if let Some(scrollbar) = poll.active_scrollbar {
            if self.scrollbar.total != scrollbar.total
                || self.scrollbar.offset != scrollbar.offset
                || self.scrollbar.len != scrollbar.len
            {
                self.scrollbar = scrollbar;
                remote_dirty = true;
            }
        }
        for pane_id in poll.exited_panes {
            if let Some(session_id) = self.server.tabs.session_id_for_pane_id(pane_id) {
                for server in self.remote_servers() {
                    server.send_session_exited(session_id);
                }
            }
            self.close_pane_by_id(pane_id);
            remote_dirty = true;
        }
        self.remote_dirty |= remote_dirty;
    }

    pub(crate) fn apply_appearance(&mut self, appearance: ResolvedAppearance) {
        self.terminal_font_family = appearance.font_family;
        self.terminal_font_fallbacks = appearance.font_fallbacks;
        self.terminal_font_size = appearance.font_size;
        self.background_opacity = appearance.background_opacity;
        self.background_opacity_cells = appearance.background_opacity_cells;
        self.terminal_foreground = appearance.terminal_foreground;
        self.terminal_background = appearance.terminal_background;
        self.terminal_palette = appearance.terminal_palette;
        self.cursor_color = appearance.cursor_color;
        self.selection_background = appearance.selection_background;
        self.selection_foreground = appearance.selection_foreground;
        self.cursor_text_color = appearance.cursor_text_color;
        self.url_color = appearance.url_color;
        self.active_tab_foreground = appearance.active_tab_foreground;
        self.active_tab_background = appearance.active_tab_background;
        self.inactive_tab_foreground = appearance.inactive_tab_foreground;
        self.inactive_tab_background = appearance.inactive_tab_background;
        self.cursor_style = appearance.cursor_style;
        self.cursor_blink = appearance.cursor_blink;
        self.cursor_blink_interval = appearance.cursor_blink_interval;
        self.cursor_blink_epoch = std::time::Instant::now();
        let (cell_width, cell_height) = terminal_metrics(self.terminal_font_size);
        self.cell_width = cell_width;
        self.cell_height = cell_height;
        self.appearance_revision = self.appearance_revision.wrapping_add(1);
        #[cfg(target_os = "linux")]
        {
            self.pending_font_bytes = appearance.font_bytes;
        }
        self.apply_cursor_defaults_to_all_panes();
    }

    pub(crate) fn terminate(&self, code: i32) -> ! {
        control::cleanup(self.server.socket_path.as_deref());
        std::process::exit(code);
    }

    pub(crate) fn close_pane_by_id(&mut self, pane_id: pane::PaneId) {
        let old_focused = self.server.tabs.focused_pane();
        let Some((tab_index, leaf_id)) = self.server.tabs.find_pane_location(pane_id) else {
            return;
        };
        let old_active = self.server.tabs.active_index();
        if old_active != tab_index {
            self.server.tabs.goto_tab(tab_index);
        }
        let Some(tree) = self.server.tabs.active_tree_mut() else {
            return;
        };
        tree.set_focus(leaf_id);
        let new_focused = self.server.tabs.focused_pane();
        if old_focused != new_focused {
            self.set_pane_focus(old_focused, false);
            self.set_pane_focus(new_focused, true);
        }
        self.handle_surface_closed();
    }

    pub(crate) fn update(&mut self, message: Message) -> Task<Message> {
        #[cfg(target_os = "linux")]
        if let Some(bytes) = self.pending_font_bytes.take() {
            return iced::font::load(bytes).map(|_| Message::FontLoaded);
        }

        {
            let _scope =
                crate::profiling::scope("server.backend.tick", crate::profiling::Kind::Cpu);
            self.backend.tick();
        }
        {
            let _scope =
                crate::profiling::scope("server.backend.poll", crate::profiling::Kind::Cpu);
            self.poll_backend();
        }
        {
            let _scope = crate::profiling::scope(
                "server.text_input_cursor_rect",
                crate::profiling::Kind::Cpu,
            );
            self.update_text_input_cursor_rect();
        }

        let mut saw_interactive_activity = false;
        if let Ok(cmd) = self.server.ctl_rx.try_recv() {
            self.handle_server_cmd(cmd.into());
            saw_interactive_activity = true;
        }
        while let Ok(cmd) = self.server.remote_rx.try_recv() {
            self.handle_server_cmd(cmd.into());
            saw_interactive_activity = true;
        }
        while let Ok(cmd) = self.server.local_gui_rx.try_recv() {
            self.handle_server_cmd(cmd.into());
            saw_interactive_activity = true;
        }
        if saw_interactive_activity {
            self.interactive_activity_epoch = std::time::Instant::now();
        }
        if self.remote_dirty {
            let _scope =
                crate::profiling::scope("server.publish_remote_state", crate::profiling::Kind::Cpu);
            self.publish_remote_state();
            self.remote_dirty = false;
        }

        while let Ok(scroll) = self.scroll_rx.try_recv() {
            let surface = self.focused_surface();
            if !surface.is_null() {
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
                        .scroll_viewport_delta(self.server.tabs.focused_pane(), line_delta);
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

        if self.server.tabs.is_empty() {
            if !self.surface_initialized_once {
                self.init_surface();
            }
            return Task::none();
        }

        let event = match message {
            Message::Frame => {
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
                self.cursor_blink_epoch = std::time::Instant::now();
                self.set_pane_focus(self.server.tabs.focused_pane(), true);
                self.backend.set_app_focus(true);
            }
            Event::Window(window::Event::Unfocused) => {
                self.app_focused = false;
                self.cursor_blink_epoch = std::time::Instant::now();
                self.set_pane_focus(self.server.tabs.focused_pane(), false);
                self.backend.set_app_focus(false);
            }
            _ => {}
        }

        Task::none()
    }
}

impl Drop for BooApp {
    fn drop(&mut self) {
        for pane in self.server.tabs.all_panes() {
            self.free_pane_backend(pane);
        }
        control::cleanup(self.server.socket_path.as_deref());
    }
}
