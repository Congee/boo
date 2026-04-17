use super::*;

impl BooApp {
    pub(crate) fn status_bar_height(&self) -> f64 {
        crate::status_bar_metrics(
            self.terminal_font_size,
            self.terminal_font_families.first().copied(),
        )
        .height
    }

    pub(crate) fn status_bar_text_size(&self) -> f32 {
        crate::status_bar_metrics(
            self.terminal_font_size,
            self.terminal_font_families.first().copied(),
        )
        .text_size
    }

    fn append_osc_color(seq: &mut Vec<u8>, code: &str, color: crate::config::RgbColor) {
        let value = format!(
            "\x1b]{code};#{:02X}{:02X}{:02X}\x07",
            color[0], color[1], color[2]
        );
        seq.extend_from_slice(value.as_bytes());
    }

    fn append_palette_osc(seq: &mut Vec<u8>, index: usize, color: crate::config::RgbColor) {
        let value = format!(
            "\x1b]4;{index};#{:02X}{:02X}{:02X}\x07",
            color[0], color[1], color[2]
        );
        seq.extend_from_slice(value.as_bytes());
    }

    fn default_theme_vt_sequence(&self) -> Vec<u8> {
        let mut seq = Vec::new();
        for (index, color) in self.terminal_palette.iter().enumerate() {
            if let Some(color) = color {
                Self::append_palette_osc(&mut seq, index, *color);
            }
        }
        Self::append_osc_color(&mut seq, "10", self.terminal_foreground);
        Self::append_osc_color(&mut seq, "11", self.terminal_background);
        Self::append_osc_color(&mut seq, "12", self.cursor_color);
        seq
    }

    fn default_cursor_vt_sequence(&self) -> Option<Vec<u8>> {
        match self.cursor_style {
            Some(style) => {
                let blink = self.cursor_blink;
                let param = match style {
                    crate::vt::GHOSTTY_RENDER_STATE_CURSOR_VISUAL_STYLE_BAR => {
                        if blink {
                            5
                        } else {
                            6
                        }
                    }
                    crate::vt::GHOSTTY_RENDER_STATE_CURSOR_VISUAL_STYLE_UNDERLINE => {
                        if blink {
                            3
                        } else {
                            4
                        }
                    }
                    _ => {
                        if blink {
                            1
                        } else {
                            2
                        }
                    }
                };
                Some(format!("\x1b[{param} q").into_bytes())
            }
            None if !self.cursor_blink => Some(b"\x1b[?12l".to_vec()),
            None => None,
        }
    }

    fn apply_cursor_defaults_to_pane(&mut self, pane: PaneHandle) {
        let mut bytes = self.default_theme_vt_sequence();
        if let Some(cursor_bytes) = self.default_cursor_vt_sequence() {
            bytes.extend_from_slice(&cursor_bytes);
        }
        if bytes.is_empty() {
            return;
        }
        self.backend.write_vt_bytes(pane, &bytes);
    }

    pub(crate) fn apply_cursor_defaults_to_all_panes(&mut self) {
        for pane in self.server.tabs.all_panes() {
            self.apply_cursor_defaults_to_pane(pane);
        }
    }

    pub(crate) fn terminal_frame(&self) -> platform::Rect {
        let search_offset = if self.search_active {
            self.status_bar_height()
        } else {
            0.0
        };
        platform::Rect::new(
            platform::Point::new(0.0, search_offset),
            platform::Size::new(
                self.last_size.width as f64,
                self.last_size.height as f64 - self.status_bar_height() - search_offset,
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

    pub(crate) fn resize_viewport_cells(&mut self, cols: u16, rows: u16) -> bool {
        let (width, terminal_height) = self.session_size_pixels(cols, rows);
        self.resize_viewport_points(
            width as f64,
            terminal_height as f64 + self.status_bar_height(),
        )
    }

    pub(crate) fn resize_viewport_points(&mut self, width: f64, height: f64) -> bool {
        let next_size = Size::new(width.max(1.0) as f32, height.max(1.0) as f32);
        if self.last_size == next_size {
            return false;
        }
        self.last_size = next_size;
        self.relayout();
        true
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
        self.surface_initialized_once = true;
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
        let pane = self.backend.create_pane(
            ptr::null_mut(),
            self.parent_view,
            self.scale_factor(),
            frame,
            context,
            command,
            working_directory,
            self.cell_width,
            self.cell_height,
        );
        if let Some(pane) = pane {
            self.apply_cursor_defaults_to_pane(pane);
            Some(pane)
        } else {
            None
        }
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
        let frame = self.terminal_frame();
        if let Some(tree) = self.server.tabs.active_tree_mut() {
            tree.focus_direction(frame, dir);
        }
        let new = self.server.tabs.focused_pane();
        if old != new {
            self.set_pane_focus(old, false);
            self.set_pane_focus(new, true);
        }
    }

    pub(crate) fn focus_pane_by_id(&mut self, pane_id: crate::pane::PaneId) -> bool {
        let old = self.server.tabs.focused_pane();
        let mut changed = false;
        if let Some(tree) = self.server.tabs.active_tree_mut() {
            changed = tree.set_focus_to_pane(pane_id);
        }
        let new = self.server.tabs.focused_pane();
        if old != new {
            self.set_pane_focus(old, false);
            self.set_pane_focus(new, true);
            return true;
        }
        changed
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
                let active = self.server.tabs.active_index();
                self.server.tabs.remove_tab(active);
            }

            if !self.server.tabs.is_empty() {
                let focused = self.server.tabs.focused_pane();
                self.set_pane_focus(focused, true);
                self.relayout();
            }
            log::info!(
                "surface closed, {} surfaces in tab, {} tabs",
                self.server.tabs.active_tree().map(|t| t.len()).unwrap_or(0),
                self.server.tabs.len()
            );
            return;
        }

        let active = self.server.tabs.active_index();
        let panes = self.server.tabs.remove_tab(active);
        for pane in panes {
            self.free_pane_backend(pane);
        }
        if !self.server.tabs.is_empty() {
            let focused = self.server.tabs.focused_pane();
            self.set_pane_focus(focused, true);
            self.relayout();
        }
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
        log::info!(
            "loading session: {} ({} tabs)",
            layout.name,
            layout.tabs.len()
        );
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
                    tab.layout = session_tab.layout.clone();
                }
            } else if let Some(tab) = self.server.tabs.tab_mut(tab_idx) {
                tab.layout = session_tab.layout.clone();
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
                    layout: self
                        .server
                        .tabs
                        .tab_layout(info.index)
                        .unwrap_or(session::TabLayout::Manual),
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
