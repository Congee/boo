use super::*;

impl BooApp {
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

    fn dispatch_app_key(
        &mut self,
        event: &AppKeyEvent,
        key_char: Option<char>,
        keyboard_key: keyboard::Key,
    ) -> bool {
        let text = event
            .text
            .clone()
            .or_else(|| key_char.map(|ch| ch.to_string()));
        let iced_mods = ghostty_mods_to_iced(event.mods);

        if self.find_window_active && self.handle_find_window_key(&keyboard_key, key_char) {
            return true;
        }

        if self.choose_tree_active && self.handle_choose_tree_key(&keyboard_key, key_char) {
            return true;
        }

        if self.choose_buffer_active && self.handle_choose_buffer_key(&keyboard_key, key_char) {
            return true;
        }

        if self.display_panes_active && self.handle_display_panes_key(&keyboard_key, key_char) {
            return true;
        }

        if self.command_prompt.active {
            self.handle_command_key(&keyboard_key, &text, &iced_mods);
            return true;
        }

        if self.search_active {
            self.handle_search_key(&keyboard_key, &text, &iced_mods);
            return true;
        }

        let result = self
            .bindings
            .handle_key(key_char, event.keycode, event.mods, event.named_key);
        self.dispatch_binding_result(result)
    }

    pub(crate) fn handle_app_key_event(&mut self, event: AppKeyEvent) -> bool {
        let key_char = event.key_char();
        let keyboard_key = event.keyboard_key();

        if self.dispatch_app_key(&event, key_char, keyboard_key) {
            return true;
        }

        let surface = self.focused_surface();
        let unshifted_codepoint = event
            .key_char()
            .map(|ch| ch as u32)
            .unwrap_or_else(|| shifted_codepoint(event.keycode, 0));

        if surface.is_null() {
            if event.mods
                & (ffi::GHOSTTY_MODS_CTRL | ffi::GHOSTTY_MODS_ALT | ffi::GHOSTTY_MODS_SUPER)
                == 0
            {
                if let Some(committed) = event
                    .text
                    .clone()
                    .or_else(|| event.modified_text.clone())
                    .filter(|text| !text.is_empty())
                {
                    self.handle_committed_text(committed);
                    return false;
                }
            }
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            {
                let Some(vt_keycode) = keymap::native_to_vt_keycode(event.keycode) else {
                    return false;
                };
                #[cfg(target_os = "macos")]
                if should_route_macos_vt_key_via_appkit(vt_keycode, event.mods) {
                    return false;
                }
                let _ = self.backend.forward_vt_key(
                    self.server.tabs.focused_pane(),
                    if event.repeat {
                        vt::GHOSTTY_KEY_ACTION_REPEAT
                    } else {
                        vt::GHOSTTY_KEY_ACTION_PRESS
                    },
                    vt_keycode,
                    event.mods as vt::GhosttyMods,
                    (event.mods & ffi::GHOSTTY_MODS_SHIFT) as vt::GhosttyMods,
                    key_char,
                    event.text.as_deref().unwrap_or(""),
                    false,
                    shifted_codepoint_vt(vt_keycode, 0),
                );
            }
            return false;
        }

        let translation_mods = self.surface_key_translation_mods(surface, event.mods);
        let consumed_mods = translation_mods & !(ffi::GHOSTTY_MODS_CTRL | ffi::GHOSTTY_MODS_SUPER);
        let text_cstring = event
            .text
            .as_ref()
            .filter(|t| t.as_bytes().first().is_some_and(|&b| b >= 0x20))
            .and_then(|t| CString::new(t.as_str()).ok());
        let text_ptr = text_cstring
            .as_ref()
            .map(|c| c.as_ptr())
            .unwrap_or(ptr::null());
        let key_event = ffi::ghostty_input_key_s {
            action: if event.repeat {
                ffi::ghostty_input_action_e::GHOSTTY_ACTION_REPEAT
            } else {
                ffi::ghostty_input_action_e::GHOSTTY_ACTION_PRESS
            },
            mods: event.mods,
            consumed_mods,
            keycode: event.keycode,
            text: text_ptr,
            unshifted_codepoint,
            composing: false,
        };
        let consumed = self.forward_surface_key(key_event);
        if self.dump_keys {
            log::info!(
                "→ghostty: keycode=0x{:02x} mods={:#x} cp={:#x} text={:?} consumed={consumed}",
                event.keycode,
                event.mods,
                unshifted_codepoint,
                event.text.as_deref()
            );
        }
        false
    }

    pub(crate) fn handle_committed_text(&mut self, committed: String) {
        if self.display_panes_active {
            let mut consumed = false;
            for ch in committed.chars() {
                consumed |= self.handle_display_panes_char(ch);
            }
            if consumed {
                return;
            }
        }

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

    pub(crate) fn handle_app_mouse_event(&mut self, event: AppMouseEvent) -> bool {
        let old_focus = self.server.tabs.focused_pane();
        let old_divider_drag = self.divider_drag;
        let old_scrollbar_drag = self.scrollbar_drag;
        match event {
            AppMouseEvent::CursorMoved { x, y, .. } => {
                self.handle_mouse(mouse::Event::CursorMoved {
                    position: iced::Point::new(x as f32, y as f32),
                });
            }
            AppMouseEvent::ButtonPressed { button, x, y, .. } => {
                self.last_mouse_pos = (x, y);
                self.handle_mouse(mouse::Event::ButtonPressed(button.to_iced()));
            }
            AppMouseEvent::ButtonReleased { button, x, y, .. } => {
                self.last_mouse_pos = (x, y);
                self.handle_mouse(mouse::Event::ButtonReleased(button.to_iced()));
            }
            AppMouseEvent::WheelScrolledLines { x, y, .. } => {
                self.handle_mouse(mouse::Event::WheelScrolled {
                    delta: mouse::ScrollDelta::Lines {
                        x: x as f32,
                        y: y as f32,
                    },
                });
            }
            AppMouseEvent::WheelScrolledPixels { x, y, .. } => {
                self.handle_mouse(mouse::Event::WheelScrolled {
                    delta: mouse::ScrollDelta::Pixels {
                        x: x as f32,
                        y: y as f32,
                    },
                });
            }
        }
        old_focus != self.server.tabs.focused_pane()
            || old_divider_drag != self.divider_drag
            || old_scrollbar_drag != self.scrollbar_drag
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn handle_platform_key_event(&mut self, event: platform::KeyEvent) {
        self.handle_app_key_event(AppKeyEvent {
            keycode: event.keycode,
            mods: event.mods,
            text: None,
            modified_text: None,
            named_key: native_keycode_to_named_key(event.keycode),
            repeat: event.repeat,
            input_seq: None,
        });
    }

    pub(crate) fn surface_key_translation_mods(
        &self,
        surface: ffi::ghostty_surface_t,
        mods: i32,
    ) -> i32 {
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

                let Some(keycode) = keymap::physical_to_native_keycode(&physical_key) else {
                    return;
                };
                let mods = iced_mods_to_ghostty(&modifiers);
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

                self.handle_app_key_event(AppKeyEvent {
                    keycode,
                    mods,
                    text: text
                        .as_ref()
                        .map(ToString::to_string)
                        .filter(|t| !t.is_empty()),
                    modified_text: match &modified_key {
                        keyboard::Key::Character(s) if !s.is_empty() => Some(s.to_string()),
                        _ => None,
                    },
                    named_key,
                    repeat,
                    input_seq: None,
                });
            }
            keyboard::Event::KeyReleased {
                physical_key,
                modifiers,
                ..
            } => {
                let Some(keycode) = keymap::physical_to_native_keycode(&physical_key) else {
                    return;
                };
                if self.focused_surface().is_null() {
                    let Some(vt_keycode) = keymap::physical_to_vt_keycode(&physical_key) else {
                        return;
                    };
                    #[cfg(target_os = "macos")]
                    if should_route_macos_vt_key_via_appkit(
                        vt_keycode,
                        iced_mods_to_ghostty(&modifiers),
                    ) {
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

    pub(crate) fn inject_key(&mut self, keyspec: &str) {
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
                    self.server.tabs.focused_pane(),
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
        let ctext = text_str
            .as_ref()
            .and_then(|t| CString::new(t.as_str()).ok());
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
            log::info!(
                "ctl key: keycode=0x{keycode:02x} mods={mods:#x} cp=0x{unshifted:02x} text={text_str:?} consumed={consumed}"
            );
        }
    }

    #[allow(dead_code)]
    pub(crate) fn dispatch_binding_action(&mut self, action: bindings::Action) {
        match action {
            bindings::Action::NewSplit(dir) => self.create_split(dir),
            bindings::Action::GotoSplit(dir) => {
                self.switch_focus(dir);
            }
            bindings::Action::ResizeSplit(dir, amount) => {
                let (axis, sign) = match dir {
                    bindings::Direction::Right => (splits::Direction::Horizontal, 1),
                    bindings::Direction::Left => (splits::Direction::Horizontal, -1),
                    bindings::Direction::Down => (splits::Direction::Vertical, 1),
                    bindings::Direction::Up => (splits::Direction::Vertical, -1),
                };
                let frame = self.terminal_frame();
                let cell_extent = match axis {
                    splits::Direction::Horizontal => self.cell_width,
                    splits::Direction::Vertical => self.cell_height,
                };
                if let Some(tree) = self.server.tabs.active_tree_mut() {
                    tree.resize_focused_by_cells(
                        frame,
                        axis,
                        sign * i32::from(amount),
                        cell_extent,
                    );
                }
                self.relayout();
            }
            bindings::Action::CloseSurface => self.handle_surface_closed(),
            bindings::Action::BreakPane => {
                if self.server.tabs.break_active_pane_to_tab().is_some() {
                    let pane = self.server.tabs.focused_pane();
                    self.set_pane_focus(pane, true);
                    self.relayout();
                }
            }
            bindings::Action::NewTab => {
                let _ = self.new_tab();
            }
            bindings::Action::NextTab => {
                self.server.tabs.next_tab();
                self.sync_after_tab_change();
            }
            bindings::Action::PrevTab => {
                self.server.tabs.prev_tab();
                self.sync_after_tab_change();
            }
            bindings::Action::CloseTab => {
                let active = self.server.tabs.active_index();
                let panes = self.server.tabs.remove_tab(active);
                for pane in panes {
                    self.free_pane_backend(pane);
                }
                if !self.server.tabs.is_empty() {
                    self.sync_after_tab_change();
                }
            }
            bindings::Action::GotoTab(target) => {
                let idx = match target {
                    bindings::TabTarget::Index(i) => i,
                    bindings::TabTarget::Last => self.server.tabs.len().saturating_sub(1),
                };
                self.server.tabs.goto_tab(idx);
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
            bindings::Action::Copy => {
                self.copy_mode_copy();
            }
            bindings::Action::ChooseBuffer => {
                self.choose_buffer_active = !self.paste_buffers.is_empty();
                self.choose_buffer_selected = 0;
            }
            bindings::Action::ChooseTree => {
                self.choose_tree_active = !self.choose_tree_entries().is_empty();
                self.choose_tree_selected = 0;
            }
            bindings::Action::FindWindow => {
                self.find_window_active = true;
                self.find_window_query.clear();
                self.find_window_selected = 0;
            }
            bindings::Action::DisplayPanes => {
                self.display_panes_active = self
                    .server
                    .tabs
                    .active_tree()
                    .is_some_and(|tree| tree.len() > 1);
            }
            bindings::Action::Paste => {
                self.ghostty_binding_action("paste_from_clipboard");
            }
            bindings::Action::SetTabTitle => {
                self.command_prompt.active = true;
                self.command_prompt.input = "set-tab-title ".to_string();
                self.command_prompt.selected_suggestion = 0;
                self.command_prompt.history_idx = None;
                self.command_prompt.update_suggestions();
            }
            bindings::Action::MarkPane => {
                self.marked_pane_id = Some(self.server.tabs.focused_pane().id());
            }
            bindings::Action::ClearMarkedPane => {
                self.marked_pane_id = None;
            }
            bindings::Action::JoinMarkedPane(direction) => {
                let Some(marked_pane_id) = self.marked_pane_id else {
                    return;
                };
                let focused_pane_id = self.server.tabs.focused_pane().id();
                if marked_pane_id == 0 || marked_pane_id == focused_pane_id {
                    return;
                }
                let Some(pane) = self.server.tabs.remove_pane_by_id(marked_pane_id) else {
                    self.marked_pane_id = None;
                    return;
                };
                let old = self.server.tabs.focused_pane();
                if let Some(tree) = self.server.tabs.active_tree_mut() {
                    let split_dir = match direction {
                        bindings::SplitDirection::Right | bindings::SplitDirection::Left => {
                            splits::Direction::Horizontal
                        }
                        bindings::SplitDirection::Down | bindings::SplitDirection::Up => {
                            splits::Direction::Vertical
                        }
                    };
                    let _ = tree.split_focused(split_dir, pane);
                }
                self.set_pane_focus(old, false);
                self.set_pane_focus(pane, true);
                self.marked_pane_id = None;
                self.relayout();
            }
            bindings::Action::ToggleZoom => {
                self.ghostty_binding_action("toggle_split_zoom");
                self.relayout();
            }
            bindings::Action::NextPane => {
                let old = self.server.tabs.focused_pane();
                if let Some(tree) = self.server.tabs.active_tree_mut() {
                    tree.focus_next();
                }
                let new = self.server.tabs.focused_pane();
                if old != new {
                    self.set_pane_focus(old, false);
                    self.set_pane_focus(new, true);
                }
            }
            bindings::Action::PreviousPane => {
                let old = self.server.tabs.focused_pane();
                if let Some(tree) = self.server.tabs.active_tree_mut() {
                    tree.focus_prev();
                }
                let new = self.server.tabs.focused_pane();
                if old != new {
                    self.set_pane_focus(old, false);
                    self.set_pane_focus(new, true);
                }
            }
            bindings::Action::SwapPaneNext => {
                if self.server.tabs.swap_active_pane_with_adjacent(true) {
                    self.relayout();
                }
            }
            bindings::Action::SwapPanePrevious => {
                if self.server.tabs.swap_active_pane_with_adjacent(false) {
                    self.relayout();
                }
            }
            bindings::Action::RotatePanesForward => {
                if self.server.tabs.rotate_active_panes(true) {
                    self.relayout();
                }
            }
            bindings::Action::RotatePanesBackward => {
                if self.server.tabs.rotate_active_panes(false) {
                    self.relayout();
                }
            }
            bindings::Action::SelectLayout(layout) => {
                if self.server.tabs.apply_layout_to_active(layout) {
                    self.relayout();
                }
            }
            bindings::Action::NextLayout => {
                if self.server.tabs.cycle_active_layout(true) {
                    self.relayout();
                }
            }
            bindings::Action::PreviousLayout => {
                if self.server.tabs.cycle_active_layout(false) {
                    self.relayout();
                }
            }
            bindings::Action::RebalanceLayout => {
                if self.server.tabs.rebalance_active_layout() {
                    self.relayout();
                }
            }
            bindings::Action::PreviousTab => {
                let prev = self.server.tabs.previous_active();
                self.server.tabs.goto_tab(prev);
                self.sync_after_tab_change();
            }
            bindings::Action::ReloadConfig => {
                log::info!("reloading config");
                let boo_config = config::Config::load();
                self.bindings = bindings::Bindings::from_config(&boo_config);
                self.apply_appearance(Self::resolve_appearance_config(&boo_config));
                if let Some(server) = self.server.local_gui_server.as_ref() {
                    server.send_ui_appearance_to_local_clients(&self.ui_appearance_snapshot());
                }
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

    pub(crate) fn handle_command_key<S: AsRef<str>>(
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
                    let hist_len = self.command_prompt.history.len();
                    if hist_len > 0 {
                        let idx = self
                            .command_prompt
                            .history_idx
                            .map(|i| i.saturating_sub(1))
                            .unwrap_or(hist_len - 1);
                        self.command_prompt.history_idx = Some(idx);
                        self.command_prompt.input = self.command_prompt.history[idx].clone();
                    }
                }
            }
            keyboard::Key::Named(Named::ArrowDown) => {
                if !self.command_prompt.suggestions.is_empty() {
                    if self.command_prompt.selected_suggestion + 1
                        < self.command_prompt.suggestions.len()
                    {
                        self.command_prompt.selected_suggestion += 1;
                    }
                } else if let Some(idx) = self.command_prompt.history_idx {
                    if idx + 1 < self.command_prompt.history.len() {
                        self.command_prompt.history_idx = Some(idx + 1);
                        self.command_prompt.input = self.command_prompt.history[idx + 1].clone();
                    } else {
                        self.command_prompt.history_idx = None;
                        self.command_prompt.input.clear();
                    }
                }
            }
            keyboard::Key::Named(Named::Home) => if modifiers.control() {},
            _ => {
                if modifiers.control() {
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

    pub(crate) fn execute_command(&mut self, input: &str) {
        let parts: Vec<&str> = input.split_whitespace().collect();
        if parts.is_empty() {
            return;
        }

        let cmd = parts[0];
        let arg1 = parts.get(1).copied();

        match cmd {
            "split-right" => self.dispatch_binding_action(bindings::Action::NewSplit(
                bindings::SplitDirection::Right,
            )),
            "split-down" => self.dispatch_binding_action(bindings::Action::NewSplit(
                bindings::SplitDirection::Down,
            )),
            "split-left" => self.dispatch_binding_action(bindings::Action::NewSplit(
                bindings::SplitDirection::Left,
            )),
            "split-up" => self
                .dispatch_binding_action(bindings::Action::NewSplit(bindings::SplitDirection::Up)),
            "resize-left" => {
                let n: u16 = arg1.and_then(|s| s.parse().ok()).unwrap_or(10);
                self.dispatch_binding_action(bindings::Action::ResizeSplit(
                    bindings::Direction::Left,
                    n,
                ));
            }
            "resize-right" => {
                let n: u16 = arg1.and_then(|s| s.parse().ok()).unwrap_or(10);
                self.dispatch_binding_action(bindings::Action::ResizeSplit(
                    bindings::Direction::Right,
                    n,
                ));
            }
            "resize-up" => {
                let n: u16 = arg1.and_then(|s| s.parse().ok()).unwrap_or(10);
                self.dispatch_binding_action(bindings::Action::ResizeSplit(
                    bindings::Direction::Up,
                    n,
                ));
            }
            "resize-down" => {
                let n: u16 = arg1.and_then(|s| s.parse().ok()).unwrap_or(10);
                self.dispatch_binding_action(bindings::Action::ResizeSplit(
                    bindings::Direction::Down,
                    n,
                ));
            }
            "close-pane" => self.dispatch_binding_action(bindings::Action::CloseSurface),
            "break-pane" => self.dispatch_binding_action(bindings::Action::BreakPane),
            "new-tab" => self.dispatch_binding_action(bindings::Action::NewTab),
            "next-tab" => self.dispatch_binding_action(bindings::Action::NextTab),
            "prev-tab" => self.dispatch_binding_action(bindings::Action::PrevTab),
            "close-tab" => self.dispatch_binding_action(bindings::Action::CloseTab),
            "goto-tab" => {
                if let Some(n) = arg1.and_then(|s| s.parse::<usize>().ok()) {
                    self.dispatch_binding_action(bindings::Action::GotoTab(
                        bindings::TabTarget::Index(n.saturating_sub(1)),
                    ));
                }
            }
            "last-tab" => {
                self.dispatch_binding_action(bindings::Action::GotoTab(bindings::TabTarget::Last))
            }
            "next-layout" => self.dispatch_binding_action(bindings::Action::NextLayout),
            "prev-layout" => self.dispatch_binding_action(bindings::Action::PreviousLayout),
            "select-layout" => {
                if let Some(layout) = arg1.and_then(parse_tab_layout_name) {
                    self.dispatch_binding_action(bindings::Action::SelectLayout(layout));
                }
            }
            "rebalance-layout" => self.dispatch_binding_action(bindings::Action::RebalanceLayout),
            "next-pane" => self.dispatch_binding_action(bindings::Action::NextPane),
            "prev-pane" => self.dispatch_binding_action(bindings::Action::PreviousPane),
            "swap-pane-next" => self.dispatch_binding_action(bindings::Action::SwapPaneNext),
            "swap-pane-prev" => self.dispatch_binding_action(bindings::Action::SwapPanePrevious),
            "rotate-panes-forward" => {
                self.dispatch_binding_action(bindings::Action::RotatePanesForward)
            }
            "rotate-panes-backward" => {
                self.dispatch_binding_action(bindings::Action::RotatePanesBackward)
            }
            "copy-mode" => self.dispatch_binding_action(bindings::Action::EnterCopyMode),
            "copy" => self.dispatch_binding_action(bindings::Action::Copy),
            "choose-buffer" => self.dispatch_binding_action(bindings::Action::ChooseBuffer),
            "choose-tree" => self.dispatch_binding_action(bindings::Action::ChooseTree),
            "find-window" => self.dispatch_binding_action(bindings::Action::FindWindow),
            "display-panes" => self.dispatch_binding_action(bindings::Action::DisplayPanes),
            "command-prompt" => self.dispatch_binding_action(bindings::Action::OpenCommandPrompt),
            "search" => self.dispatch_binding_action(bindings::Action::Search),
            "paste" => self.dispatch_binding_action(bindings::Action::Paste),
            "mark-pane" => self.dispatch_binding_action(bindings::Action::MarkPane),
            "clear-marked-pane" => self.dispatch_binding_action(bindings::Action::ClearMarkedPane),
            "join-pane" | "move-pane" => {
                if let Some(direction) = arg1.and_then(parse_split_direction_name) {
                    self.dispatch_binding_action(bindings::Action::JoinMarkedPane(direction));
                }
            }
            "set-tab-title" => {
                if parts.len() >= 2 {
                    self.server.tabs.set_active_title(parts[1..].join(" "));
                    self.remote_dirty = true;
                    self.relayout();
                }
            }
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

    pub(crate) fn handle_search_key<S: AsRef<str>>(
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

    pub(crate) fn update_scrollbar_overlay(&self) {
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

    pub(crate) fn sync_after_tab_change(&mut self) {
        self.find_window_active = false;
        self.find_window_query.clear();
        self.find_window_selected = 0;
        self.choose_tree_active = false;
        self.choose_buffer_active = false;
        self.display_panes_active = false;
        let focused = self.server.tabs.focused_pane();
        self.set_pane_focus(focused, true);
        self.relayout();
    }

    pub(crate) fn read_surface_selection_text(
        &self,
        selection: ffi::ghostty_selection_s,
    ) -> Option<String> {
        self.backend
            .read_selection_text(self.server.tabs.focused_pane(), selection)
    }

    pub(crate) fn ghostty_binding_action(&mut self, action: &str) {
        self.backend
            .binding_action(self.server.tabs.focused_pane(), action, self.scrollbar.len);
    }

    pub(crate) fn send_search(&mut self) {
        self.ghostty_binding_action(&format!("search:{}", self.search_query));
    }

    pub(crate) fn scroll_to_mouse_y(&mut self, y: f64) {
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
                self.backend
                    .scroll_viewport_top(self.server.tabs.focused_pane())
            } else if target_row >= max_offset {
                self.backend
                    .scroll_viewport_bottom(self.server.tabs.focused_pane())
            } else {
                self.backend
                    .scroll_viewport_delta(self.server.tabs.focused_pane(), delta as isize)
            };
        }
    }

    pub(crate) fn handle_mouse(&mut self, event: mouse::Event) {
        match event {
            mouse::Event::CursorMoved { position } => {
                self.last_mouse_pos = (position.x as f64, position.y as f64);
                if let Some(dir) = self.divider_drag {
                    let frame = self.terminal_frame();
                    let point = (position.x as f64, position.y as f64);
                    if let Some(tree) = self.server.tabs.active_tree_mut() {
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
                        self.server.tabs.focused_pane(),
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
                    let (mx, my) = self.last_mouse_pos;
                    let terminal_h = self.last_size.height as f64 - STATUS_BAR_HEIGHT;
                    if mx >= self.last_size.width as f64 - 10.0 && my < terminal_h {
                        self.scrollbar_drag = true;
                        self.scrollbar_opacity = 1.0;
                        self.scroll_to_mouse_y(my);
                        return;
                    }

                    let frame = self.terminal_frame();
                    let point = (mx, my);
                    if let Some(tree) = self.server.tabs.active_tree() {
                        if let Some(dir) = tree.divider_at(frame, point) {
                            self.divider_drag = Some(dir);
                            return;
                        }
                    }

                    let old = self.server.tabs.focused_pane();
                    if let Some(tree) = self.server.tabs.active_tree_mut() {
                        if tree.focus_at(frame, point) {
                            let new = self.server.tabs.focused_pane();
                            self.set_pane_focus(old, false);
                            self.set_pane_focus(new, true);
                        }
                    }

                    if let Some(url) = self.hyperlink_at_point(point) {
                        std::thread::spawn(move || {
                            let _ = open::that(url);
                        });
                        return;
                    }
                }
                #[cfg(any(target_os = "linux", target_os = "macos"))]
                if self.focused_surface().is_null() {
                    let (mx, my) = self.last_mouse_pos;
                    let _ = self.backend.send_mouse_input(
                        self.server.tabs.focused_pane(),
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
                        self.server.tabs.focused_pane(),
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
                            .scroll_viewport_delta(self.server.tabs.focused_pane(), line_delta);
                    }
                }
            }
            _ => {}
        }
    }

    fn hyperlink_at_point(&self, point: (f64, f64)) -> Option<String> {
        let frame = self.terminal_frame();
        let tree = self.server.tabs.active_tree()?;
        let pane = tree
            .export_panes_with_frames(frame)
            .into_iter()
            .find(|pane| pane.pane.id() == self.server.tabs.focused_pane().id())?;
        let pane_frame = pane.frame?;
        if point.0 < pane_frame.origin.x
            || point.1 < pane_frame.origin.y
            || point.0 >= pane_frame.origin.x + pane_frame.size.width
            || point.1 >= pane_frame.origin.y + pane_frame.size.height
        {
            return None;
        }
        let local_x = point.0 - pane_frame.origin.x;
        let local_y = point.1 - pane_frame.origin.y;
        let col = (local_x / self.cell_width).floor().max(0.0) as u16;
        let row = (local_y / self.cell_height).floor().max(0.0) as u16;
        self.backend
            .hyperlink_at(self.server.tabs.focused_pane(), row, col)
    }
}

fn parse_tab_layout_name(name: &str) -> Option<session::TabLayout> {
    match name {
        "manual" => Some(session::TabLayout::Manual),
        "even-horizontal" => Some(session::TabLayout::EvenHorizontal),
        "even-vertical" => Some(session::TabLayout::EvenVertical),
        "main-horizontal" => Some(session::TabLayout::MainHorizontal),
        "main-vertical" => Some(session::TabLayout::MainVertical),
        "tiled" => Some(session::TabLayout::Tiled),
        _ => None,
    }
}

fn parse_split_direction_name(name: &str) -> Option<bindings::SplitDirection> {
    match name {
        "right" => Some(bindings::SplitDirection::Right),
        "down" => Some(bindings::SplitDirection::Down),
        "left" => Some(bindings::SplitDirection::Left),
        "up" => Some(bindings::SplitDirection::Up),
        _ => None,
    }
}

impl BooApp {
    fn handle_find_window_key(&mut self, key: &keyboard::Key, key_char: Option<char>) -> bool {
        use keyboard::key::Named;

        match key {
            keyboard::Key::Named(Named::Escape) => {
                self.find_window_active = false;
                self.find_window_query.clear();
                self.find_window_selected = 0;
                true
            }
            keyboard::Key::Named(Named::Enter) => {
                self.select_find_window_entry();
                true
            }
            keyboard::Key::Named(Named::ArrowUp) => {
                self.move_find_window_selection(false);
                true
            }
            keyboard::Key::Named(Named::ArrowDown) => {
                self.move_find_window_selection(true);
                true
            }
            keyboard::Key::Named(Named::Backspace) => {
                self.find_window_query.pop();
                self.find_window_selected = 0;
                true
            }
            _ => {
                if let Some(ch) = key_char {
                    if ch >= ' ' {
                        self.find_window_query.push(ch);
                        self.find_window_selected = 0;
                    } else {
                        self.find_window_active = false;
                    }
                } else {
                    self.find_window_active = false;
                }
                true
            }
        }
    }

    fn move_find_window_selection(&mut self, forward: bool) {
        let len = self.find_window_entries().len();
        if len == 0 {
            self.find_window_selected = 0;
            return;
        }
        if forward {
            self.find_window_selected = (self.find_window_selected + 1) % len;
        } else {
            self.find_window_selected = (self.find_window_selected + len - 1) % len;
        }
    }

    fn select_find_window_entry(&mut self) {
        let Some(entry) = self
            .find_window_entries()
            .get(self.find_window_selected)
            .cloned()
        else {
            self.find_window_active = false;
            self.find_window_query.clear();
            self.find_window_selected = 0;
            return;
        };
        self.find_window_active = false;
        self.find_window_query.clear();
        self.find_window_selected = 0;
        let old = self.server.tabs.focused_pane();
        self.server.tabs.goto_tab(entry.tab_index);
        if self.server.tabs.focus_active_pane_by_id(entry.pane_id) {
            let new = self.server.tabs.focused_pane();
            if old != new {
                self.set_pane_focus(old, false);
                self.set_pane_focus(new, true);
            }
        }
        self.sync_after_tab_change();
    }

    fn handle_choose_tree_key(&mut self, key: &keyboard::Key, key_char: Option<char>) -> bool {
        use keyboard::key::Named;

        match key {
            keyboard::Key::Named(Named::Escape) => {
                self.choose_tree_active = false;
                true
            }
            keyboard::Key::Named(Named::Enter) => {
                self.select_choose_tree_entry();
                true
            }
            keyboard::Key::Named(Named::ArrowUp) => {
                self.move_choose_tree_selection(false);
                true
            }
            keyboard::Key::Named(Named::ArrowDown) => {
                self.move_choose_tree_selection(true);
                true
            }
            _ => {
                match key_char {
                    Some('k') => self.move_choose_tree_selection(false),
                    Some('j') => self.move_choose_tree_selection(true),
                    Some('\r') => self.select_choose_tree_entry(),
                    _ => self.choose_tree_active = false,
                }
                true
            }
        }
    }

    fn move_choose_tree_selection(&mut self, forward: bool) {
        let len = self.choose_tree_entries().len();
        if len == 0 {
            self.choose_tree_active = false;
            self.choose_tree_selected = 0;
            return;
        }
        if forward {
            self.choose_tree_selected = (self.choose_tree_selected + 1) % len;
        } else {
            self.choose_tree_selected = (self.choose_tree_selected + len - 1) % len;
        }
    }

    fn select_choose_tree_entry(&mut self) {
        let Some(entry) = self
            .choose_tree_entries()
            .get(self.choose_tree_selected)
            .cloned()
        else {
            self.choose_tree_active = false;
            self.choose_tree_selected = 0;
            return;
        };
        self.choose_tree_active = false;
        let old = self.server.tabs.focused_pane();
        self.server.tabs.goto_tab(entry.tab_index);
        if self.server.tabs.focus_active_pane_by_id(entry.pane_id) {
            let new = self.server.tabs.focused_pane();
            if old != new {
                self.set_pane_focus(old, false);
                self.set_pane_focus(new, true);
            }
        }
        self.sync_after_tab_change();
    }

    fn handle_choose_buffer_key(&mut self, key: &keyboard::Key, key_char: Option<char>) -> bool {
        use keyboard::key::Named;

        match key {
            keyboard::Key::Named(Named::Escape) => {
                self.choose_buffer_active = false;
                true
            }
            keyboard::Key::Named(Named::Enter) => {
                self.paste_selected_buffer();
                true
            }
            keyboard::Key::Named(Named::ArrowUp) => {
                self.move_choose_buffer_selection(false);
                true
            }
            keyboard::Key::Named(Named::ArrowDown) => {
                self.move_choose_buffer_selection(true);
                true
            }
            keyboard::Key::Named(Named::Backspace) | keyboard::Key::Named(Named::Delete) => {
                self.delete_selected_buffer();
                true
            }
            _ => {
                match key_char {
                    Some('k') => self.move_choose_buffer_selection(false),
                    Some('j') => self.move_choose_buffer_selection(true),
                    Some('d') => self.delete_selected_buffer(),
                    Some('p') | Some('\r') => self.paste_selected_buffer(),
                    _ => self.choose_buffer_active = false,
                }
                true
            }
        }
    }

    fn move_choose_buffer_selection(&mut self, forward: bool) {
        if self.paste_buffers.is_empty() {
            self.choose_buffer_active = false;
            self.choose_buffer_selected = 0;
            return;
        }
        let len = self.paste_buffers.len();
        if forward {
            self.choose_buffer_selected = (self.choose_buffer_selected + 1) % len;
        } else {
            self.choose_buffer_selected = (self.choose_buffer_selected + len - 1) % len;
        }
    }

    fn delete_selected_buffer(&mut self) {
        if self.paste_buffers.is_empty() {
            self.choose_buffer_active = false;
            self.choose_buffer_selected = 0;
            return;
        }
        let index = self
            .choose_buffer_selected
            .min(self.paste_buffers.len() - 1);
        self.paste_buffers.remove(index);
        if self.paste_buffers.is_empty() {
            self.choose_buffer_active = false;
            self.choose_buffer_selected = 0;
        } else if self.choose_buffer_selected >= self.paste_buffers.len() {
            self.choose_buffer_selected = self.paste_buffers.len() - 1;
        }
    }

    fn paste_selected_buffer(&mut self) {
        let Some(text) = self.paste_buffers.get(self.choose_buffer_selected).cloned() else {
            self.choose_buffer_active = false;
            self.choose_buffer_selected = 0;
            return;
        };
        self.choose_buffer_active = false;
        self.last_clipboard_text = text.clone();
        platform::clipboard_write(&text);
        let _ = self
            .backend
            .write_input(self.server.tabs.focused_pane(), text.as_bytes());
    }

    fn handle_display_panes_key(&mut self, key: &keyboard::Key, key_char: Option<char>) -> bool {
        use keyboard::key::Named;

        match key {
            keyboard::Key::Named(Named::Escape) => {
                self.display_panes_active = false;
                true
            }
            keyboard::Key::Named(Named::Enter)
            | keyboard::Key::Named(Named::Backspace)
            | keyboard::Key::Named(Named::Tab)
            | keyboard::Key::Named(Named::ArrowUp)
            | keyboard::Key::Named(Named::ArrowDown)
            | keyboard::Key::Named(Named::ArrowLeft)
            | keyboard::Key::Named(Named::ArrowRight) => {
                self.display_panes_active = false;
                true
            }
            _ => {
                if let Some(ch) = key_char {
                    return self.handle_display_panes_char(ch);
                }
                self.display_panes_active = false;
                true
            }
        }
    }

    fn handle_display_panes_char(&mut self, ch: char) -> bool {
        let Some(index) = display_panes_digit_index(ch) else {
            self.display_panes_active = false;
            return true;
        };
        let visible_panes = self.visible_pane_snapshots();
        let Some(target) = visible_panes.get(index) else {
            self.display_panes_active = false;
            return true;
        };
        self.display_panes_active = false;
        let old = self.server.tabs.focused_pane();
        if self.server.tabs.focus_active_pane_by_id(target.pane_id) {
            let new = self.server.tabs.focused_pane();
            if old != new {
                self.set_pane_focus(old, false);
                self.set_pane_focus(new, true);
            }
        }
        true
    }
}

fn display_panes_digit_index(ch: char) -> Option<usize> {
    match ch {
        '1'..='9' => Some(ch as usize - '1' as usize),
        _ => None,
    }
}
