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

    pub(crate) fn forward_text_input_command(&mut self, command: platform::TextInputCommand) {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            let (keycode, unshifted_codepoint) = text_input_command_key(command);
            let _ = self.backend.forward_vt_key(
                self.server.tabs.focused_pane(),
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

    pub(crate) fn dispatch_binding_result(&mut self, result: bindings::KeyResult) -> bool {
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

    pub(crate) fn route_app_key(
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

    pub(crate) fn handle_committed_text(&mut self, committed: String) {
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
            .write_input(self.server.tabs.focused_pane(), committed.as_bytes());
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn handle_platform_key_event(&mut self, event: platform::KeyEvent) {
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
            self.server.tabs.focused_pane(),
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

    pub(crate) fn surface_key_translation_mods(&self, surface: ffi::ghostty_surface_t, mods: i32) -> i32 {
        self.backend.surface_key_translation_mods(surface, mods)
    }

    pub(crate) fn forward_surface_key(&mut self, event: ffi::ghostty_input_key_s) -> bool {
        self.backend
            .surface_key(self.server.tabs.focused_pane(), event)
    }

    pub(crate) fn forward_surface_mouse_pos(&mut self, x: f64, y: f64, mods: i32) {
        self.backend
            .surface_mouse_pos(self.server.tabs.focused_pane(), x, y, mods);
    }

    pub(crate) fn forward_surface_mouse_button(
        &mut self,
        state: ffi::ghostty_input_mouse_state_e,
        button: ffi::ghostty_input_mouse_button_e,
        mods: i32,
    ) {
        self.backend
            .surface_mouse_button(self.server.tabs.focused_pane(), state, button, mods);
    }

    pub(crate) fn forward_surface_mouse_scroll(&mut self, dx: f64, dy: f64, mods: i32) {
        self.backend
            .surface_mouse_scroll(self.server.tabs.focused_pane(), dx, dy, mods);
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

    #[allow(dead_code)]
    pub(crate) fn ui_font(&self) -> Font {
        configured_font(self.terminal_font_family)
    }

    #[allow(dead_code)]
    pub(crate) fn panel_alpha(&self, base: f32) -> f32 {
        (base * self.background_opacity.max(0.3)).clamp(0.2, 0.98)
    }

    #[allow(dead_code)]
    pub(crate) fn window_style(&self) -> iced::theme::Style {
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

    pub(crate) fn ui_snapshot(&self) -> control::UiSnapshot {
        let focused_pane = self.server.tabs.focused_pane();
        let terminal_frame = self.terminal_frame();
        let visible_panes = self
            .server
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
                        frame: pane
                            .frame
                            .map_or(ui_rect_snapshot(0.0, 0.0, 0.0, 0.0), |frame| {
                                ui_rect_snapshot(
                                    frame.origin.x,
                                    frame.origin.y,
                                    frame.size.width,
                                    frame.size.height,
                                )
                            }),
                        split_direction: pane
                            .split
                            .map(|(direction, _)| split_direction_name(direction).to_string()),
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
            .server
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

        let terminal = { self.backend.ui_terminal_snapshot(focused_pane.id()) };

        control::UiSnapshot {
            active_tab: self.server.tabs.active_index(),
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

    pub(crate) fn handle_keyboard(&mut self, event: keyboard::Event) {
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
                let key_char = shifted_char(keycode, mods)
                    .or_else(|| text.as_ref().and_then(|t| t.chars().next()))
                    .or_else(|| match &modified_key {
                        keyboard::Key::Character(s) => s.chars().next(),
                        _ => None,
                    });

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

                if let Some(ref mut cm) = self.copy_mode {
                    if let Some(kind) = cm.pending_jump.take() {
                        if let Some(ch) = key_char {
                            cm.last_jump = Some((ch, kind));
                            self.copy_mode_execute_jump(ch, kind);
                        }
                        return;
                    }
                }

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

                let action = if repeat {
                    ffi::ghostty_input_action_e::GHOSTTY_ACTION_REPEAT
                } else {
                    ffi::ghostty_input_action_e::GHOSTTY_ACTION_PRESS
                };
                let surface = self.focused_surface();
                let translation_mods = self.surface_key_translation_mods(surface, mods);
                let unshifted_codepoint = key_to_codepoint(&key);
                let consumed_mods =
                    translation_mods & !(ffi::GHOSTTY_MODS_CTRL | ffi::GHOSTTY_MODS_SUPER);

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
                        self.server.tabs.focused_pane(),
                        if repeat {
                            vt::GHOSTTY_KEY_ACTION_REPEAT
                        } else {
                            vt::GHOSTTY_KEY_ACTION_PRESS
                        },
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
                    ) && self.preedit_text.is_empty()
                    {
                        return;
                    }
                    let _ = self.backend.forward_vt_key(
                        self.server.tabs.focused_pane(),
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
}
