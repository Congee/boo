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


    pub(crate) fn resize_pane_backend(&mut self, pane: PaneHandle, scale: f64, width: u32, height: u32) {
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
        for running_command in poll.running_commands.iter().cloned() {
            self.server.tabs.set_running_command_for_pane(
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
                self.server
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
            self.pwd = pwd;
        }
        if let Some(title) = poll.active_title {
            self.server.tabs.set_active_title(title);
        }
        if let Some(scrollbar) = poll.active_scrollbar {
            self.scrollbar = scrollbar;
        }
        for pane_id in poll.exited_panes {
            self.close_active_pane_by_id(pane_id);
        }
    }

    pub(crate) fn apply_appearance(&mut self, appearance: ResolvedAppearance) {
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

    pub(crate) fn terminate(&self, code: i32) -> ! {
        control::cleanup(self.server.socket_path.as_deref());
        std::process::exit(code);
    }

    pub(crate) fn close_active_pane_by_id(&mut self, pane_id: pane::PaneId) {
        let Some(tree) = self.server.tabs.active_tree_mut() else {
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

    pub(crate) fn update(&mut self, message: Message) -> Task<Message> {
        #[cfg(target_os = "linux")]
        if let Some(bytes) = self.pending_font_bytes.take() {
            return iced::font::load(bytes).map(|_| Message::FontLoaded);
        }

        self.backend.tick();
        self.poll_backend();
        self.update_text_input_cursor_rect();

        if let Ok(cmd) = self.server.ctl_rx.try_recv() {
            self.handle_server_cmd(cmd.into());
        }
        while let Ok(cmd) = self.server.remote_rx.try_recv() {
            self.handle_server_cmd(cmd.into());
        }
        self.publish_remote_state();

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
            self.init_surface();
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
                self.set_pane_focus(self.server.tabs.focused_pane(), true);
                self.backend.set_app_focus(true);
            }
            Event::Window(window::Event::Unfocused) => {
                self.app_focused = false;
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
