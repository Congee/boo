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

                if self.route_app_key(key_char, keycode, mods, named_key, key.clone()) {
                    return;
                }

                let surface = self.focused_surface();
                let translation_mods = self.surface_key_translation_mods(surface, mods);
                let unshifted_codepoint = key_to_codepoint(&key);
                let consumed_mods =
                    translation_mods & !(ffi::GHOSTTY_MODS_CTRL | ffi::GHOSTTY_MODS_SUPER);

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
                    action: if repeat {
                        ffi::ghostty_input_action_e::GHOSTTY_ACTION_REPEAT
                    } else {
                        ffi::ghostty_input_action_e::GHOSTTY_ACTION_PRESS
                    },
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
                let delta = amount as f64 / 100.0;
                let (axis, sign) = match dir {
                    bindings::Direction::Right => (splits::Direction::Horizontal, 1.0),
                    bindings::Direction::Left => (splits::Direction::Horizontal, -1.0),
                    bindings::Direction::Down => (splits::Direction::Vertical, 1.0),
                    bindings::Direction::Up => (splits::Direction::Vertical, -1.0),
                };
                if let Some(tree) = self.server.tabs.active_tree_mut() {
                    tree.resize_focused(axis, delta * sign);
                }
                self.relayout();
            }
            bindings::Action::CloseSurface => self.handle_surface_closed(),
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
            bindings::Action::Paste => {
                self.ghostty_binding_action("paste_from_clipboard");
            }
            bindings::Action::ToggleZoom => {
                self.ghostty_binding_action("toggle_split_zoom");
                self.relayout();
            }
            bindings::Action::NextPane => {
                if let Some(tree) = self.server.tabs.active_tree_mut() {
                    tree.focus_next();
                }
                let new = self.server.tabs.focused_pane();
                self.set_pane_focus(new, true);
            }
            bindings::Action::PreviousPane => {
                if let Some(tree) = self.server.tabs.active_tree_mut() {
                    tree.focus_prev();
                }
                let new = self.server.tabs.focused_pane();
                self.set_pane_focus(new, true);
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
}
