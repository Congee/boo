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

    pub(crate) fn handle_server_cmd(&mut self, cmd: server::Command) {
        match cmd {
            server::Command::DumpKeys(enabled) => self.dump_keys = enabled,
            server::Command::Quit => self.terminate(0),
            server::Command::ListSurfaces { reply } => {
                let info = if let Some(tree) = self.server.tabs.active_tree() {
                    tree.surface_info()
                        .into_iter()
                        .map(|(id, focused)| control::SurfaceInfo { index: id, focused })
                        .collect()
                } else {
                    Vec::new()
                };
                let _ = reply.send(control::Response::Surfaces { surfaces: info });
            }
            server::Command::NewSplit { direction } => {
                self.create_split(Self::split_direction_from_str(&direction));
            }
            server::Command::FocusSurface { index } => {
                let old = self.server.tabs.focused_pane();
                if let Some(tree) = self.server.tabs.active_tree_mut() {
                    tree.set_focus(index);
                }
                let new = self.server.tabs.focused_pane();
                if old != new {
                    self.set_pane_focus(old, false);
                    self.set_pane_focus(new, true);
                }
            }
            server::Command::ListTabs { reply } => {
                let _ = reply.send(control::Response::Tabs {
                    tabs: self.server.tabs.tab_info(),
                });
            }
            server::Command::GetClipboard { reply } => {
                let _ = reply.send(control::Response::Clipboard {
                    text: if self.last_clipboard_text.is_empty() {
                        platform::clipboard_read().unwrap_or_default()
                    } else {
                        self.last_clipboard_text.clone()
                    },
                });
            }
            server::Command::GetUiSnapshot { reply } => {
                let _ = reply.send(control::Response::UiSnapshot {
                    snapshot: self.ui_snapshot(),
                });
            }
            server::Command::ExecuteCommand { input } => {
                self.execute_command(&input);
            }
            server::Command::SendText { text } => {
                #[cfg(any(target_os = "linux", target_os = "macos"))]
                {
                    let _ = self
                        .backend
                        .write_input(self.server.tabs.focused_pane(), text.as_bytes());
                }
            }
            server::Command::SendVt { text } => {
                #[cfg(any(target_os = "linux", target_os = "macos"))]
                {
                    self.backend
                        .write_vt_bytes(self.server.tabs.focused_pane(), text.as_bytes());
                }
            }
            server::Command::NewTab => {
                let _ = self.new_tab();
            }
            server::Command::GotoTab { index } => {
                self.server.tabs.goto_tab(index);
                self.sync_after_tab_change();
            }
            server::Command::NextTab => {
                self.server.tabs.next_tab();
                self.sync_after_tab_change();
            }
            server::Command::PrevTab => {
                self.server.tabs.prev_tab();
                self.sync_after_tab_change();
            }
            server::Command::ResizeFocused { cols, rows } => {
                let pane = self.server.tabs.focused_pane();
                let (width, height) = self.session_size_pixels(cols, rows);
                self.resize_pane_backend(pane, self.scale_factor(), width, height);
            }
            server::Command::SendKey { keyspec } => {
                self.inject_key(&keyspec);
            }
            server::Command::RemoteListSessions { client_id } => {
                if let Some(server) = self.server.remote_server.as_ref() {
                    server.send_session_list(client_id, &self.remote_sessions());
                }
            }
            server::Command::RemoteAttach {
                client_id,
                session_id,
            } => {
                if self.pane_for_session(session_id).is_some() {
                    if let Some(server) = self.server.remote_server.as_ref() {
                        server.send_attached(client_id, session_id);
                    }
                    self.publish_remote_session(session_id);
                } else if let Some(server) = self.server.remote_server.as_ref() {
                    server.send_error(client_id, "unknown session");
                }
            }
            server::Command::RemoteDetach { client_id } => {
                if let Some(server) = self.server.remote_server.as_ref() {
                    server.send_detached(client_id);
                }
            }
            server::Command::RemoteCreate {
                client_id,
                cols,
                rows,
            } => {
                let created = self.new_tab();
                let Some(session_id) = created else {
                    if let Some(server) = self.server.remote_server.as_ref() {
                        server.send_error(client_id, "failed to create session");
                    }
                    return;
                };
                if let Some(pane) = self.pane_for_session(session_id) {
                    let (width, height) = self.session_size_pixels(cols, rows);
                    self.resize_pane_backend(pane, self.scale_factor(), width, height);
                }
                if let Some(server) = self.server.remote_server.as_ref() {
                    server.send_session_created(client_id, session_id);
                }
            }
            server::Command::RemoteInput { client_id, bytes } => {
                let Some(session_id) = self
                    .server
                    .remote_server
                    .as_ref()
                    .and_then(|server| server.client_session(client_id))
                else {
                    if let Some(server) = self.server.remote_server.as_ref() {
                        server.send_error(client_id, "not attached");
                    }
                    return;
                };
                let Some(pane) = self.pane_for_session(session_id) else {
                    if let Some(server) = self.server.remote_server.as_ref() {
                        server.send_session_exited(session_id);
                    }
                    return;
                };
                let _ = self.backend.write_input(pane, &bytes);
            }
            server::Command::RemoteResize {
                client_id,
                cols,
                rows,
            } => {
                let Some(session_id) = self
                    .server
                    .remote_server
                    .as_ref()
                    .and_then(|server| server.client_session(client_id))
                else {
                    if let Some(server) = self.server.remote_server.as_ref() {
                        server.send_error(client_id, "not attached");
                    }
                    return;
                };
                let Some(pane) = self.pane_for_session(session_id) else {
                    if let Some(server) = self.server.remote_server.as_ref() {
                        server.send_session_exited(session_id);
                    }
                    return;
                };
                let (width, height) = self.session_size_pixels(cols, rows);
                self.resize_pane_backend(pane, self.scale_factor(), width, height);
            }
            server::Command::RemoteDestroy {
                client_id,
                session_id,
            } => {
                let target = session_id.or_else(|| {
                    self.server
                        .remote_server
                        .as_ref()
                        .and_then(|server| server.client_session(client_id))
                });
                let Some(target) = target else {
                    if let Some(server) = self.server.remote_server.as_ref() {
                        server.send_error(client_id, "unknown session");
                    }
                    return;
                };
                let Some(tab_index) = self.server.tabs.find_index_by_session_id(target) else {
                    if let Some(server) = self.server.remote_server.as_ref() {
                        server.send_session_exited(target);
                    }
                    return;
                };
                if self.server.tabs.len() <= 1 {
                    if let Some(server) = self.server.remote_server.as_ref() {
                        server.send_error(client_id, "cannot destroy last session");
                    }
                    return;
                }
                let was_active = tab_index == self.server.tabs.active_index();
                let panes = self.server.tabs.remove_tab(tab_index);
                for pane in panes {
                    self.backend.free_pane(pane);
                }
                if was_active && !self.server.tabs.is_empty() {
                    self.sync_after_tab_change();
                }
                if let Some(server) = self.server.remote_server.as_ref() {
                    server.send_session_exited(target);
                }
            }
        }
    }

    pub(crate) fn remote_sessions(&self) -> Vec<remote::RemoteSessionInfo> {
        self.server
            .tabs
            .tab_session_info()
            .into_iter()
            .map(|tab| {
                let pane = self
                    .server
                    .tabs
                    .tab_tree(tab.index)
                    .map(|tree| tree.focused_pane())
                    .unwrap_or(PaneHandle::null());
                let terminal = self.backend.ui_terminal_snapshot(pane.id());
                remote::RemoteSessionInfo {
                    id: tab.id,
                    name: format!("Tab {}", tab.index + 1),
                    title: tab.title,
                    pwd: terminal
                        .as_ref()
                        .map(|snapshot| snapshot.pwd.clone())
                        .unwrap_or_default(),
                    attached: self
                        .server
                        .remote_server
                        .as_ref()
                        .is_some_and(|server| server.attached_to_session(tab.id)),
                    child_exited: pane.id() == 0 || terminal.is_none(),
                }
            })
            .collect()
    }

    pub(crate) fn pane_for_session(&self, session_id: u32) -> Option<PaneHandle> {
        let tab_index = self.server.tabs.find_index_by_session_id(session_id)?;
        self.server
            .tabs
            .tab_tree(tab_index)
            .map(|tree| tree.focused_pane())
    }

    pub(crate) fn session_size_pixels(&self, cols: u16, rows: u16) -> (u32, u32) {
        let width = (cols as f64 * self.cell_width).round().max(1.0) as u32;
        let height = (rows as f64 * self.cell_height).round().max(1.0) as u32;
        (width, height)
    }

    pub(crate) fn publish_remote_session(&self, session_id: u32) {
        let Some(server) = self.server.remote_server.as_ref() else {
            return;
        };
        let Some(pane) = self.pane_for_session(session_id) else {
            server.send_session_exited(session_id);
            return;
        };
        let Some(snapshot) = self.backend.ui_terminal_snapshot(pane.id()) else {
            server.send_session_exited(session_id);
            return;
        };
        let state = remote::full_state_from_ui(&snapshot);
        server.send_full_state_to_attached(session_id, &state);
    }

    pub(crate) fn publish_remote_state(&self) {
        let Some(server) = self.server.remote_server.as_ref() else {
            return;
        };
        for session_id in server.attached_sessions() {
            self.publish_remote_session(session_id);
        }
    }

    pub(crate) fn terminal_frame(&self) -> platform::Rect {
        let search_offset = if self.search_active {
            STATUS_BAR_HEIGHT
        } else {
            0.0
        };
        platform::Rect::new(
            platform::Point::new(0.0, search_offset),
            platform::Size::new(
                self.last_size.width as f64,
                self.last_size.height as f64 - STATUS_BAR_HEIGHT - search_offset,
            ),
        )
    }

    pub(crate) fn scale_factor(&self) -> f64 {
        if self.headless {
            return 1.0;
        }
        platform::scale_factor()
    }

    pub(crate) fn pane_parent_frame(&self) -> Option<platform::Rect> {
        if self.headless {
            return Some(self.terminal_frame());
        }
        if self.parent_view.is_null() {
            return None;
        }
        Some(platform::view_bounds(self.parent_view))
    }

    pub(crate) fn handle_resize(&mut self, size: Size) {
        self.last_size = size;
        self.relayout();
    }

    pub(crate) fn init_surface(&mut self) {
        if !self.server.tabs.is_empty() {
            return;
        }
        if self.headless {
            self.parent_view = ptr::null_mut();
            self.scrollbar_layer = ptr::null_mut();
            if self.last_size.width <= 1.0 || self.last_size.height <= 1.0 {
                self.last_size = Size::new(HEADLESS_WIDTH, HEADLESS_HEIGHT);
            }
        } else {
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
        }
        let frame = self.terminal_frame();
        if frame.size.width <= 1.0 || frame.size.height <= 1.0 {
            return;
        }

        let Some(pane) = self.create_pane(
            frame,
            ffi::ghostty_surface_context_e::GHOSTTY_SURFACE_CONTEXT_WINDOW,
        ) else {
            return;
        };
        self.server.tabs.add_initial_tab(pane);
        self.set_pane_focus(pane, true);

        let scale = self.scale_factor();
        let (w, h) = if frame.size.width > 0.0 && frame.size.height > 0.0 {
            (
                (frame.size.width * scale) as u32,
                (frame.size.height * scale) as u32,
            )
        } else {
            (800, 600)
        };
        self.resize_pane_backend(pane, scale, w, h);
        log::info!("tab 0 created, size {w}x{h}");

        if let Some(name) = launch::startup_session() {
            self.load_session(name);
        }
    }

    pub(crate) fn create_pane(
        &mut self,
        frame: platform::Rect,
        context: ffi::ghostty_surface_context_e,
    ) -> Option<PaneHandle> {
        self.create_pane_with(frame, context, None, None)
    }

    #[allow(unused_variables)]
    pub(crate) fn create_pane_with(
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

    pub(crate) fn create_split(&mut self, direction: bindings::SplitDirection) {
        if self.server.tabs.is_empty() {
            return;
        }
        let Some(parent_bounds) = self.pane_parent_frame() else {
            return;
        };
        let split_dir = match direction {
            bindings::SplitDirection::Right | bindings::SplitDirection::Left => {
                splits::Direction::Horizontal
            }
            _ => splits::Direction::Vertical,
        };

        let Some(pane) = self.create_pane(
            parent_bounds,
            ffi::ghostty_surface_context_e::GHOSTTY_SURFACE_CONTEXT_SPLIT,
        ) else {
            return;
        };
        let old_focused = self.server.tabs.focused_pane();
        if let Some(tree) = self.server.tabs.active_tree_mut() {
            tree.split_focused(split_dir, pane);
        }

        self.set_pane_focus(old_focused, false);
        self.set_pane_focus(pane, true);

        self.relayout();
        log::info!("split created");
    }

    pub(crate) fn switch_focus(&mut self, dir: bindings::PaneFocusDirection) {
        let old = self.server.tabs.focused_pane();
        if let Some(tree) = self.server.tabs.active_tree_mut() {
            match dir {
                bindings::PaneFocusDirection::Next => tree.focus_next(),
                bindings::PaneFocusDirection::Previous => tree.focus_prev(),
            }
        }
        let new = self.server.tabs.focused_pane();
        if old != new {
            self.set_pane_focus(old, false);
            self.set_pane_focus(new, true);
        }
    }

    pub(crate) fn relayout(&mut self) {
        if self.server.tabs.is_empty() || self.last_size.width == 0.0 {
            return;
        }
        let scale = self.scale_factor();
        let frame = self.terminal_frame();
        let surfaces = self.server.tabs.layout_active(frame, scale);
        for (pane, w, h) in surfaces {
            self.resize_pane_backend(pane, scale, w, h);
        }
    }

    pub(crate) fn handle_surface_closed(&mut self) {
        let removed = if let Some(tree) = self.server.tabs.active_tree_mut() {
            tree.remove_focused().map(|pane| (pane, tree.len() == 0))
        } else {
            None
        };
        if let Some((pane, tab_empty)) = removed {
            self.free_pane_backend(pane);

            if tab_empty {
                if self.server.tabs.len() <= 1 {
                    self.terminate(0);
                }
                let active = self.server.tabs.active_index();
                self.server.tabs.remove_tab(active);
            }

            let focused = self.server.tabs.focused_pane();
            self.set_pane_focus(focused, true);
            self.relayout();
            log::info!(
                "surface closed, {} surfaces in tab, {} tabs",
                self.server.tabs.active_tree().map(|t| t.len()).unwrap_or(0),
                self.server.tabs.len()
            );
            return;
        }

        if self.server.tabs.len() <= 1 {
            self.terminate(0);
        }
        let active = self.server.tabs.active_index();
        let panes = self.server.tabs.remove_tab(active);
        for pane in panes {
            self.free_pane_backend(pane);
        }
        let focused = self.server.tabs.focused_pane();
        self.set_pane_focus(focused, true);
        self.relayout();
    }

    pub(crate) fn new_tab(&mut self) -> Option<u32> {
        let Some(frame) = self.pane_parent_frame() else {
            return None;
        };
        let Some(pane) = self.create_pane(
            frame,
            ffi::ghostty_surface_context_e::GHOSTTY_SURFACE_CONTEXT_TAB,
        ) else {
            return None;
        };
        let old = self.server.tabs.focused_pane();
        self.set_pane_focus(old, false);

        let idx = self.server.tabs.new_tab(pane);
        self.set_pane_focus(pane, true);
        self.relayout();
        log::info!("new tab {idx} (total: {})", self.server.tabs.len());
        self.server.tabs.session_id_for_index(idx)
    }

    pub(crate) fn load_session(&mut self, name: &str) {
        let Some(layout) = session::load_session(name) else {
            log::warn!("session not found: {name}");
            return;
        };
        log::info!("loading session: {} ({} tabs)", layout.name, layout.tabs.len());
        let Some(frame) = self.pane_parent_frame() else {
            return;
        };

        for (tab_idx, session_tab) in layout.tabs.iter().enumerate() {
            let auto_splits = if session_tab.layout != session::TabLayout::Manual {
                session::layout_splits(&session_tab.layout, session_tab.panes.len())
            } else {
                vec![]
            };

            for (pane_idx, pane) in session_tab.panes.iter().enumerate() {
                let cmd_cstr = pane
                    .command
                    .as_ref()
                    .map(|c| CString::new(c.as_str()).unwrap());
                let wd_cstr = pane
                    .working_directory
                    .as_ref()
                    .map(|w| CString::new(w.as_str()).unwrap());

                if pane_idx == 0 {
                    let Some(pane) = self.create_pane_with(
                        frame,
                        if self.server.tabs.is_empty() && tab_idx == 0 {
                            ffi::ghostty_surface_context_e::GHOSTTY_SURFACE_CONTEXT_WINDOW
                        } else {
                            ffi::ghostty_surface_context_e::GHOSTTY_SURFACE_CONTEXT_TAB
                        },
                        cmd_cstr.as_deref(),
                        wd_cstr.as_deref(),
                    ) else {
                        continue;
                    };
                    let old = self.server.tabs.focused_pane();
                    self.set_pane_focus(old, false);
                    self.server.tabs.new_tab(pane);
                    self.set_pane_focus(pane, true);
                } else {
                    let (split_dir, ratio) = if !auto_splits.is_empty() {
                        let spec = &auto_splits[pane_idx - 1];
                        let dir = match spec.direction {
                            session::SplitDir::Right => splits::Direction::Horizontal,
                            session::SplitDir::Down => splits::Direction::Vertical,
                        };
                        (dir, spec.ratio)
                    } else if let Some(ref spec) = pane.split {
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
                    ) else {
                        continue;
                    };
                    if let Some(tree) = self.server.tabs.active_tree_mut() {
                        tree.split_focused_with_ratio(split_dir, pane, ratio);
                    }
                    self.set_pane_focus(pane, true);
                }
            }
            if !session_tab.title.is_empty() {
                if let Some(tab) = self.server.tabs.tab_mut(tab_idx) {
                    tab.title = session_tab.title.clone();
                }
            }
        }
        self.relayout();
        log::info!("session loaded: {}", layout.name);
    }

    pub(crate) fn save_current_session(&self, name: &str) {
        let tab_infos = self.server.tabs.tab_info();
        let tabs: Vec<session::SessionTab> = tab_infos
            .iter()
            .map(|info| {
                let panes = if let Some(tree) = self.server.tabs.tab_tree(info.index) {
                    tree.export_panes()
                        .into_iter()
                        .map(|ep| {
                            let split = ep.split.map(|(dir, ratio)| session::SplitSpec {
                                direction: match dir {
                                    splits::Direction::Horizontal => session::SplitDir::Right,
                                    splits::Direction::Vertical => session::SplitDir::Down,
                                },
                                ratio,
                            });
                            session::SessionPane {
                                command: None,
                                working_directory: None,
                                split,
                            }
                        })
                        .collect()
                } else {
                    vec![session::SessionPane {
                        command: None,
                        working_directory: None,
                        split: None,
                    }]
                };
                session::SessionTab {
                    title: info.title.clone(),
                    layout: session::TabLayout::Manual,
                    panes,
                }
            })
            .collect();

        let layout = session::SessionLayout {
            name: name.to_string(),
            tabs,
        };
        if let Err(e) = session::save_session(&layout) {
            log::error!("failed to save session: {e}");
        }
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
