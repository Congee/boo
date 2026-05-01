use super::*;
use crate::command_prompt::COMMANDS;
use iced::alignment;
use iced::widget::canvas;
use iced::widget::stack;
use iced::{Pixels, Point, Rectangle, Renderer};

impl BooApp {
    pub(crate) fn pane_ids_for_tab(&self, tab_id: u32) -> Vec<u64> {
        self.server
            .tabs
            .find_index_by_tab_id(tab_id)
            .and_then(|tab_index| self.server.tabs.tab_tree(tab_index))
            .map(|tree| {
                tree.export_panes()
                    .into_iter()
                    .map(|pane| pane.pane.id())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub(crate) fn default_focused_pane_for_tab(&self, tab_id: u32) -> Option<u64> {
        self.server
            .tabs
            .find_index_by_tab_id(tab_id)
            .and_then(|tab_index| self.server.tabs.tab_tree(tab_index))
            .map(|tree| tree.focused_pane().id())
    }

    pub(crate) fn visible_pane_snapshots_for(
        &self,
        tab_id: u32,
        focused_pane_id: u64,
        viewport_cols: Option<u16>,
        viewport_rows: Option<u16>,
    ) -> Vec<control::UiPaneSnapshot> {
        let terminal_frame = match viewport_cols.zip(viewport_rows) {
            Some((cols, rows)) => {
                let (width, height) = self.tab_size_pixels(cols, rows);
                platform::Rect::new(
                    platform::Point::new(0.0, 0.0),
                    platform::Size::new(width as f64, height as f64),
                )
            }
            None => self.terminal_frame(),
        };
        self.server
            .tabs
            .find_index_by_tab_id(tab_id)
            .and_then(|tab_index| self.server.tabs.tab_tree(tab_index))
            .map(|tree| {
                tree.export_panes_with_frames(terminal_frame)
                    .into_iter()
                    .enumerate()
                    .map(|(leaf_index, pane)| control::UiPaneSnapshot {
                        leaf_index,
                        leaf_id: pane.leaf_id,
                        pane_id: pane.pane.id(),
                        focused: pane.pane.id() == focused_pane_id,
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
            .unwrap_or_default()
    }

    pub(crate) fn runtime_tab_snapshots_for(
        &self,
        viewed_tab_id: Option<u32>,
    ) -> Vec<control::UiTabSnapshot> {
        self.server
            .tabs
            .tab_info()
            .into_iter()
            .map(|tab| {
                let tab_id = self
                    .server
                    .tabs
                    .tab_id_for_index(tab.index)
                    .expect("tab_info index should resolve to tab id");
                control::UiTabSnapshot {
                    pane_ids: self.pane_ids_for_tab(tab_id),
                    focused_pane: self.default_focused_pane_for_tab(tab_id),
                    tab_id,
                    index: tab.index,
                    active: Some(tab_id) == viewed_tab_id,
                    title: tab.title,
                    pane_count: tab.surfaces,
                }
            })
            .collect()
    }

    pub(crate) fn ui_mouse_selection_snapshot(&self) -> control::UiMouseSelectionSnapshot {
        let Some(selection) = self
            .mouse_selection
            .filter(|selection| selection.has_range())
        else {
            return control::UiMouseSelectionSnapshot::default();
        };
        let rects = self
            .mouse_selection_rects(selection, 0.0)
            .into_iter()
            .map(|(x, y, width, height)| ui_rect_snapshot(x, y, width, height))
            .collect();
        control::UiMouseSelectionSnapshot {
            active: true,
            pane_id: Some(selection.pane_id),
            selection_rects: rects,
        }
    }

    pub(crate) fn find_window_entries(&self) -> Vec<ChooseTreeEntry> {
        let query = self.find_window_query.trim().to_lowercase();
        self.choose_tree_entries()
            .into_iter()
            .filter(|entry| {
                if query.is_empty() {
                    return true;
                }
                let haystacks = [
                    entry.tab_title.to_lowercase(),
                    entry.cwd.to_lowercase(),
                    entry.preview.to_lowercase(),
                    format!("{}.{}", entry.tab_index + 1, entry.pane_index + 1),
                ];
                haystacks.iter().any(|value| value.contains(&query))
            })
            .collect()
    }

    pub(crate) fn choose_tree_entries(&self) -> Vec<ChooseTreeEntry> {
        let active_pane_id = self.server.tabs.focused_pane().id();
        let mut entries = Vec::new();
        for tab in self.server.tabs.tab_identity_info() {
            let Some(tree) = self.server.tabs.tab_tree(tab.index) else {
                continue;
            };
            for (pane_index, pane) in tree.export_panes().into_iter().enumerate() {
                let terminal = self.backend.ui_terminal_snapshot(pane.pane.id());
                let title = self.server.tabs.display_title(tab.index, None);
                let cwd = terminal
                    .as_ref()
                    .map(|snapshot| snapshot.pwd.clone())
                    .unwrap_or_default();
                let preview = terminal
                    .and_then(|snapshot| {
                        snapshot.rows_data.into_iter().find_map(|row| {
                            let line = row
                                .cells
                                .into_iter()
                                .map(|cell| cell.text)
                                .collect::<String>()
                                .trim()
                                .to_string();
                            (!line.is_empty()).then_some(line)
                        })
                    })
                    .unwrap_or_default();
                entries.push(ChooseTreeEntry {
                    tab_index: tab.index,
                    pane_id: pane.pane.id(),
                    pane_index,
                    focused: pane.pane.id() == active_pane_id,
                    tab_title: title,
                    cwd,
                    preview,
                });
            }
        }
        entries
    }

    pub(crate) fn visible_pane_snapshots(&self) -> Vec<control::UiPaneSnapshot> {
        self.server
            .tabs
            .active_tab_id()
            .map(|tab_id| {
                self.visible_pane_snapshots_for(
                    tab_id,
                    self.server.tabs.focused_pane().id(),
                    None,
                    None,
                )
            })
            .unwrap_or_default()
    }

    pub(crate) fn visible_pane_terminal_snapshots(&self) -> Vec<control::UiPaneTerminalSnapshot> {
        self.server
            .tabs
            .active_tree()
            .map(|tree| {
                tree.export_panes()
                    .into_iter()
                    .filter_map(|pane| {
                        self.backend
                            .ui_terminal_snapshot(pane.pane.id())
                            .map(|terminal| control::UiPaneTerminalSnapshot {
                                pane_id: pane.pane.id(),
                                terminal,
                            })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub(crate) fn ui_text_snapshot(&self) -> control::UiTextSnapshot {
        let visible_panes = self.visible_pane_snapshots();
        let pane_texts = visible_panes
            .iter()
            .filter_map(|pane| {
                self.backend
                    .ui_terminal_snapshot(pane.pane_id)
                    .map(|terminal| control::UiPaneTextSnapshot {
                        pane_id: pane.pane_id,
                        text: Self::terminal_text(&terminal),
                    })
            })
            .collect();

        control::UiTextSnapshot {
            active_tab: self.server.tabs.active_index(),
            focused_pane: self.server.tabs.focused_pane().id(),
            tabs: self.runtime_tab_snapshots_for(self.server.tabs.active_tab_id()),
            visible_panes,
            status_bar: self.status_components.snapshot(),
            pane_texts,
        }
    }

    fn terminal_text(terminal: &control::UiTerminalSnapshot) -> String {
        terminal
            .rows_data
            .iter()
            .map(|row| {
                row.cells
                    .iter()
                    .map(|cell| {
                        if cell.text.is_empty() {
                            " "
                        } else {
                            cell.text.as_str()
                        }
                    })
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[allow(dead_code)]
    pub(crate) fn ui_font(&self) -> Font {
        configured_font(self.terminal_font_families.first().copied())
    }

    #[allow(dead_code)]
    pub(crate) fn panel_alpha(&self, base: f32) -> f32 {
        (base * self.background_opacity.max(0.3)).clamp(0.2, 0.98)
    }

    fn theme_color(color: crate::config::RgbColor, alpha: f32) -> Color {
        Color::from_rgba8(color[0], color[1], color[2], alpha.clamp(0.0, 1.0))
    }

    #[allow(dead_code)]
    pub(crate) fn window_style(&self) -> iced::theme::Style {
        iced::theme::Style {
            background_color: Color::TRANSPARENT,
            text_color: Color::WHITE,
        }
    }

    pub(crate) fn ui_snapshot(&self) -> control::UiSnapshot {
        let focused_pane = self.server.tabs.focused_pane();
        let terminal_frame = self.terminal_frame();
        let visible_panes = self.visible_pane_snapshots();
        let pane_terminals = self.visible_pane_terminal_snapshots();
        let status_bar = self.status_components.snapshot();

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
                pane_ids: self
                    .server
                    .tabs
                    .tab_id_for_index(tab.index)
                    .map(|tab_id| self.pane_ids_for_tab(tab_id))
                    .unwrap_or_default(),
                focused_pane: self
                    .server
                    .tabs
                    .tab_id_for_index(tab.index)
                    .and_then(|tab_id| self.default_focused_pane_for_tab(tab_id)),
                tab_id: self
                    .server
                    .tabs
                    .tab_id_for_index(tab.index)
                    .expect("tab_info index should resolve to tab id"),
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

        let terminal = self.backend.ui_terminal_snapshot(focused_pane.id());

        control::UiSnapshot {
            active_tab: self.server.tabs.active_index(),
            focused_pane: focused_pane.id(),
            appearance: self.ui_appearance_snapshot(),
            tabs,
            visible_panes: visible_panes.clone(),
            pane_terminals,
            copy_mode,
            mouse_selection: self.ui_mouse_selection_snapshot(),
            search: control::UiSearchSnapshot {
                active: self.search_active,
                query: self.search_query.clone(),
                total: self.search_total,
                selected: self.search_selected,
            },
            command_prompt,
            status_bar,
            pwd: self.pwd.clone(),
            scrollbar: control::UiScrollbarSnapshot {
                total: self.scrollbar.total,
                offset: self.scrollbar.offset,
                len: self.scrollbar.len,
            },
            terminal,
        }
    }

    pub(crate) fn ui_runtime_state(&self) -> control::UiRuntimeState {
        let visible_panes = self.visible_pane_snapshots();
        control::UiRuntimeState {
            active_tab: self.server.tabs.active_index(),
            focused_pane: self.server.tabs.focused_pane().id(),
            tabs: self.runtime_tab_snapshots_for(self.server.tabs.active_tab_id()),
            visible_panes: visible_panes.clone(),
            mouse_selection: self.ui_mouse_selection_snapshot(),
            status_bar: self.status_components.snapshot(),
            pwd: self.pwd.clone(),
            runtime_revision: 1,
            view_revision: 1,
            view_id: 0,
            viewed_tab_id: self.server.tabs.active_tab_id(),
            viewport_cols: None,
            viewport_rows: None,
            visible_pane_ids: visible_panes.iter().map(|pane| pane.pane_id).collect(),
            acked_client_action_id: None,
        }
    }

    pub(crate) fn ui_appearance_snapshot(&self) -> control::UiAppearanceSnapshot {
        control::UiAppearanceSnapshot {
            font_families: self
                .terminal_font_families
                .iter()
                .map(|family| (*family).to_string())
                .collect(),
            font_size: self.terminal_font_size,
            background_opacity: self.background_opacity,
            background_opacity_cells: self.background_opacity_cells,
            terminal_foreground: self.terminal_foreground,
            terminal_background: self.terminal_background,
            cursor_color: self.cursor_color,
            selection_background: self.selection_background,
            selection_foreground: self.selection_foreground,
            cursor_text_color: self.cursor_text_color,
            url_color: self.url_color,
            active_tab_foreground: self.active_tab_foreground,
            active_tab_background: self.active_tab_background,
            inactive_tab_foreground: self.inactive_tab_foreground,
            inactive_tab_background: self.inactive_tab_background,
            cursor_style: self.cursor_style,
            cursor_blink: self.cursor_blink,
            cursor_blink_interval_ns: self.cursor_blink_interval.as_nanos() as u64,
        }
    }

    #[allow(dead_code)]
    pub(crate) fn view(&self) -> Element<'_, Message> {
        let ui_font = self.ui_font();
        let status_bar_height = self.status_bar_height() as f32;
        let status_text_size = self.status_bar_text_size();
        let status_bar = self.status_components.snapshot();
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
                        .font(ui_font)
                        .size(status_text_size)
                        .color(Color::from_rgb(0.9, 0.9, 0.9)),
                )
                .style(|_: &Theme| container::Style {
                    background: Some(iced::Background::Color(Color::from_rgba(
                        0.15,
                        0.15,
                        0.15,
                        self.panel_alpha(0.95),
                    ))),
                    ..Default::default()
                })
                .width(Length::Fill)
                .height(Length::Fixed(status_bar_height))
                .padding(0)
                .into(),
            )
        } else {
            None
        };

        let mut main_col = iced::widget::column![];
        if let Some(search) = search_bar {
            main_col = main_col.push(search);
        }
        if self.focused_surface().is_null() {
            if let Some(snapshot) = self
                .backend
                .render_snapshot(self.server.tabs.focused_pane().id())
            {
                let selection_rects = self
                    .copy_mode
                    .as_ref()
                    .and_then(|copy_mode| {
                        copy_mode.sel_anchor.map(|(anchor_row, anchor_col)| {
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
                                0.0,
                            )
                        })
                    })
                    .or_else(|| {
                        self.mouse_selection
                            .filter(|selection| {
                                selection.pane_id == self.server.tabs.focused_pane().id()
                                    && selection.has_range()
                            })
                            .map(|selection| self.mouse_selection_rects(selection, 0.0))
                    })
                    .unwrap_or_default()
                    .into_iter()
                    .map(
                        |(x, y, width, height)| vt_terminal_canvas::TerminalSelectionRect {
                            x: x as f32,
                            y: y as f32,
                            width: width as f32,
                            height: height as f32,
                        },
                    )
                    .collect::<Vec<_>>();

                let cursor_blink_visible = !snapshot.cursor.blinking
                    || self.cursor_blink_interval.is_zero()
                    || !self.app_focused
                    || crate::app_helpers::cursor_blink_visible(
                        self.cursor_blink_epoch,
                        self.cursor_blink_interval,
                    );
                let terminal_canvas = vt_terminal_canvas::TerminalCanvas::new(
                    std::sync::Arc::new(snapshot),
                    self.cell_width as f32,
                    self.cell_height as f32,
                    self.terminal_font_size,
                    self.terminal_font_families.clone().into(),
                    self.appearance_revision,
                    self.appearance_revision,
                    self.background_opacity,
                    self.background_opacity_cells,
                    cursor_blink_visible,
                    selection_rects.into(),
                    Self::theme_color(self.selection_background, 0.35),
                    Some(Self::theme_color(self.selection_foreground, 1.0)),
                    Some(Self::theme_color(self.cursor_text_color, 1.0)),
                    Some(Self::theme_color(self.url_color, 1.0)),
                    (!self.preedit_text.is_empty()).then(|| self.preedit_text.clone()),
                )
                .without_base_fill()
                .with_content_offset_y(self.smooth_scroll_content_offset_y());
                main_col = main_col.push(
                    container(
                        iced::widget::canvas(terminal_canvas)
                            .width(Length::Fill)
                            .height(Length::Fill),
                    )
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .style(|_: &Theme| container::Style {
                        ..Default::default()
                    }),
                );
            } else {
                main_col = main_col.push(
                    iced::widget::Space::new()
                        .width(Length::Fill)
                        .height(Length::Fill),
                );
            }
        } else {
            main_col = main_col.push(
                iced::widget::Space::new()
                    .width(Length::Fill)
                    .height(Length::Fill),
            );
        }

        if self.command_prompt.active {
            let suggestions = &self.command_prompt.suggestions;
            if !suggestions.is_empty() && !self.command_prompt.input.is_empty() {
                let mut suggestion_col = iced::widget::column![];
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
                        container(text(label).font(ui_font).size(status_text_size).color(fg))
                            .style(move |_: &Theme| container::Style {
                                background: Some(iced::Background::Color(bg)),
                                ..Default::default()
                            })
                            .width(Length::Fill)
                            .height(Length::Fixed(status_bar_height))
                            .padding(0),
                    );
                }
                main_col = main_col.push(suggestion_col);
            }

            let prompt_label = format!(": {}_", self.command_prompt.input);
            main_col = main_col.push(
                container(
                    text(prompt_label)
                        .font(ui_font)
                        .size(status_text_size)
                        .color(Color::from_rgb(0.9, 0.9, 0.9)),
                )
                .style(|_: &Theme| container::Style {
                    background: Some(iced::Background::Color(Color::from_rgba(
                        0.15,
                        0.15,
                        0.15,
                        self.panel_alpha(0.95),
                    ))),
                    ..Default::default()
                })
                .width(Length::Fill)
                .height(Length::Fixed(status_bar_height))
                .padding(0),
            );
        } else {
            let spinner_frame = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| (duration.as_millis() / 125) as usize)
                .unwrap_or(0);
            let tabs = self.server.tabs.tab_info_with_spinner(spinner_frame);
            let status_left = self.render_status_zone(status_bar.left.clone(), ui_font);
            let status_right = if status_bar.right.is_empty() {
                self.render_status_fallback(self.build_status_right(), ui_font)
            } else {
                self.render_status_zone(status_bar.right.clone(), ui_font)
            };
            let mut tabs_row = row![].spacing(0);
            for tab in &tabs {
                let display_idx = tab.index + 1;
                let marker = if tab.active { "*" } else { "" };
                let label = if tab.title.is_empty() {
                    format!("[{display_idx}{marker}]")
                } else {
                    format!("[{display_idx}:{}{marker}]", tab.title)
                };
                let fg = if tab.active {
                    Self::theme_color(self.active_tab_foreground, 1.0)
                } else {
                    Self::theme_color(self.inactive_tab_foreground, 1.0)
                };
                let bg = if tab.active {
                    Self::theme_color(self.active_tab_background, 0.94)
                } else {
                    Self::theme_color(self.inactive_tab_background, 0.88)
                };
                tabs_row = tabs_row.push(
                    iced::widget::button(text(label).font(ui_font).size(status_text_size).color(fg))
                        .padding(0)
                        .style(move |_: &Theme, _| iced::widget::button::Style {
                            background: Some(iced::Background::Color(bg)),
                            text_color: fg,
                            ..Default::default()
                        })
                        .on_press(Message::ActivateTab(tab.index)),
                );
            }
            main_col = main_col.push(
                container(
                    row![
                        status_left,
                        iced::widget::Space::new().width(Length::Fill),
                        tabs_row,
                        iced::widget::Space::new().width(Length::Fill),
                        status_right,
                    ]
                    .width(Length::Fill),
                )
                .style(|_: &Theme| container::Style {
                    background: Some(iced::Background::Color(Color::from_rgba(
                        0.12,
                        0.12,
                        0.12,
                        self.panel_alpha(0.92),
                    ))),
                    ..Default::default()
                })
                .width(Length::Fill)
                .height(Length::Fixed(status_bar_height))
                .padding(0),
            );
        }
        let background = iced::widget::canvas(vt_terminal_canvas::TerminalBackgroundCanvas {
            color: Self::theme_color(self.terminal_background, self.background_opacity),
        })
        .width(Length::Fill)
        .height(Length::Fill);
        let base: Element<'_, Message> = stack([
            background.into(),
            container(main_col)
                .width(Length::Fill)
                .height(Length::Fill)
                .into(),
        ])
        .width(Length::Fill)
        .height(Length::Fill)
        .into();
        if self.display_panes_active {
            let overlay = iced::widget::canvas(DisplayPanesOverlay {
                panes: self.visible_pane_snapshots(),
                font: ui_font,
            })
            .width(Length::Fill)
            .height(Length::Fill);
            stack([base, overlay.into()])
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else if self.find_window_active {
            let entries = self.find_window_entries();
            let mut list = iced::widget::column![].spacing(6).width(Length::Fill);
            for (index, entry) in entries.iter().take(12).enumerate() {
                let is_selected = index == self.find_window_selected;
                let label = format!(
                    "{}.{}  {}  {}",
                    entry.tab_index + 1,
                    entry.pane_index + 1,
                    if entry.tab_title.is_empty() {
                        "(untitled)"
                    } else {
                        entry.tab_title.as_str()
                    },
                    if entry.cwd.is_empty() {
                        entry.preview.as_str()
                    } else {
                        entry.cwd.as_str()
                    }
                );
                let preview = if entry.preview.is_empty() {
                    String::new()
                } else {
                    format!("    {}", entry.preview.replace('\n', "\\n"))
                };
                list = list.push(
                    container(
                        iced::widget::column![
                            text(label).font(ui_font).size(14).color(if is_selected {
                                Color::WHITE
                            } else {
                                Color::from_rgb(0.86, 0.86, 0.86)
                            }),
                            text(preview)
                                .font(ui_font)
                                .size(12)
                                .color(Color::from_rgb(0.68, 0.68, 0.68))
                        ]
                        .spacing(2),
                    )
                    .padding([6, 10])
                    .width(Length::Fill)
                    .style(move |_: &Theme| container::Style {
                        background: Some(iced::Background::Color(if is_selected {
                            Color::from_rgba(0.24, 0.32, 0.62, 0.94)
                        } else {
                            Color::from_rgba(0.10, 0.10, 0.10, 0.88)
                        })),
                        ..Default::default()
                    }),
                );
            }
            let overlay: Element<'_, Message> = container(
                iced::widget::column![
                    text("find-window")
                        .font(ui_font)
                        .size(16)
                        .color(Color::from_rgb(0.92, 0.92, 0.92)),
                    text(format!("query: {}_", self.find_window_query))
                        .font(ui_font)
                        .size(13)
                        .color(Color::from_rgb(0.82, 0.82, 0.82)),
                    text("type to search   enter: select   arrows: move   backspace: delete   esc: close")
                        .font(ui_font)
                        .size(12)
                        .color(Color::from_rgb(0.72, 0.72, 0.72)),
                    list
                ]
                .spacing(8)
                .width(Length::Fill),
            )
            .padding(16)
            .width(Length::FillPortion(3))
            .style(|_: &Theme| container::Style {
                background: Some(iced::Background::Color(Color::from_rgba(
                    0.06, 0.06, 0.06, 0.96,
                ))),
                ..Default::default()
            })
            .into();
            stack([
                base,
                container(overlay)
                    .center_x(Length::Fill)
                    .center_y(Length::Fill)
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into(),
            ])
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else if self.choose_tree_active {
            let entries = self.choose_tree_entries();
            let mut list = iced::widget::column![].spacing(6).width(Length::Fill);
            for (index, entry) in entries.iter().take(12).enumerate() {
                let is_selected = index == self.choose_tree_selected;
                let label = format!(
                    "{}{}.{}  {}  {}",
                    if entry.focused { "*" } else { " " },
                    entry.tab_index + 1,
                    entry.pane_index + 1,
                    if entry.tab_title.is_empty() {
                        "(untitled)"
                    } else {
                        entry.tab_title.as_str()
                    },
                    if entry.cwd.is_empty() {
                        entry.preview.as_str()
                    } else {
                        entry.cwd.as_str()
                    }
                );
                let preview = if entry.preview.is_empty() {
                    String::new()
                } else {
                    format!("    {}", entry.preview.replace('\n', "\\n"))
                };
                list = list.push(
                    container(
                        iced::widget::column![
                            text(label).font(ui_font).size(14).color(if is_selected {
                                Color::WHITE
                            } else {
                                Color::from_rgb(0.86, 0.86, 0.86)
                            }),
                            text(preview)
                                .font(ui_font)
                                .size(12)
                                .color(Color::from_rgb(0.68, 0.68, 0.68))
                        ]
                        .spacing(2),
                    )
                    .padding([6, 10])
                    .width(Length::Fill)
                    .style(move |_: &Theme| container::Style {
                        background: Some(iced::Background::Color(if is_selected {
                            Color::from_rgba(0.24, 0.32, 0.62, 0.94)
                        } else {
                            Color::from_rgba(0.10, 0.10, 0.10, 0.88)
                        })),
                        ..Default::default()
                    }),
                );
            }
            let overlay: Element<'_, Message> = container(
                iced::widget::column![
                    text("choose-tree")
                        .font(ui_font)
                        .size(16)
                        .color(Color::from_rgb(0.92, 0.92, 0.92)),
                    text("enter: select   j/k or arrows: move   esc: close")
                        .font(ui_font)
                        .size(12)
                        .color(Color::from_rgb(0.72, 0.72, 0.72)),
                    list
                ]
                .spacing(8)
                .width(Length::Fill),
            )
            .padding(16)
            .width(Length::FillPortion(3))
            .style(|_: &Theme| container::Style {
                background: Some(iced::Background::Color(Color::from_rgba(
                    0.06, 0.06, 0.06, 0.96,
                ))),
                ..Default::default()
            })
            .into();
            stack([
                base,
                container(overlay)
                    .center_x(Length::Fill)
                    .center_y(Length::Fill)
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into(),
            ])
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else if self.choose_buffer_active {
            let preview_limit = 48usize;
            let mut list = iced::widget::column![].spacing(6).width(Length::Fill);
            for (index, buffer) in self.paste_buffers.iter().take(8).enumerate() {
                let is_selected = index == self.choose_buffer_selected;
                let mut preview = buffer.replace('\n', "\\n");
                if preview.chars().count() > preview_limit {
                    preview = preview.chars().take(preview_limit).collect::<String>() + "...";
                }
                let label = format!("{:>2}. {}", index + 1, preview);
                list = list.push(
                    container(text(label).font(ui_font).size(14).color(if is_selected {
                        Color::WHITE
                    } else {
                        Color::from_rgb(0.82, 0.82, 0.82)
                    }))
                    .padding([6, 10])
                    .width(Length::Fill)
                    .style(move |_: &Theme| container::Style {
                        background: Some(iced::Background::Color(if is_selected {
                            Color::from_rgba(0.24, 0.32, 0.62, 0.94)
                        } else {
                            Color::from_rgba(0.10, 0.10, 0.10, 0.88)
                        })),
                        ..Default::default()
                    }),
                );
            }
            let overlay: Element<'_, Message> = container(
                iced::widget::column![
                    text("choose-buffer")
                        .font(ui_font)
                        .size(16)
                        .color(Color::from_rgb(0.92, 0.92, 0.92)),
                    text("enter/p: paste   j/k or arrows: move   d/backspace: delete   esc: close")
                        .font(ui_font)
                        .size(12)
                        .color(Color::from_rgb(0.72, 0.72, 0.72)),
                    list
                ]
                .spacing(8)
                .width(Length::Fill),
            )
            .padding(16)
            .width(Length::FillPortion(3))
            .style(|_: &Theme| container::Style {
                background: Some(iced::Background::Color(Color::from_rgba(
                    0.06, 0.06, 0.06, 0.96,
                ))),
                ..Default::default()
            })
            .into();
            stack([
                base,
                container(overlay)
                    .center_x(Length::Fill)
                    .center_y(Length::Fill)
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into(),
            ])
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else {
            base
        }
    }

    #[allow(dead_code)]
    pub(crate) fn build_status_right(&self) -> String {
        let mut right_parts = Vec::new();
        let active_surfaces = self.server.tabs.active_tree().map(|t| t.len()).unwrap_or(0);
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
        if !self.preedit_text.is_empty() {
            right_parts.push(format!("IME {}", self.preedit_text));
        }
        if !self.pwd.is_empty() {
            let home = std::env::var("HOME").unwrap_or_default();
            let display = if let Some(rest) = self.pwd.strip_prefix(&home) {
                format!("~{rest}")
            } else {
                self.pwd.clone()
            };
            right_parts.push(display);
        }
        right_parts.join("  ")
    }

    fn render_status_zone(
        &self,
        segments: Vec<crate::status_components::UiStatusComponent>,
        font: Font,
    ) -> Element<'static, Message> {
        let status_text_size = self.status_bar_text_size();
        let mut row = row![].spacing(0);
        for segment in segments {
            let fg = segment
                .style
                .fg
                .as_deref()
                .and_then(crate::status_components::parse_status_color)
                .unwrap_or_else(|| Color::from_rgb(0.82, 0.82, 0.82));
            let bg = segment
                .style
                .bg
                .as_deref()
                .and_then(crate::status_components::parse_status_color);
            row = row.push(
                container(
                    text(segment.text.clone())
                        .font(font)
                        .size(status_text_size)
                        .color(fg),
                )
                .padding(0)
                .style(move |_: &Theme| container::Style {
                    background: bg.map(iced::Background::Color),
                    ..Default::default()
                }),
            );
        }
        row.into()
    }

    fn render_status_fallback<'a>(&self, text_value: String, font: Font) -> Element<'a, Message> {
        let status_text_size = self.status_bar_text_size();
        text(text_value)
            .font(font)
            .size(status_text_size)
            .color(Color::from_rgb(0.6, 0.6, 0.6))
            .into()
    }

    #[allow(dead_code)]
    pub(crate) fn theme(&self) -> Theme {
        Theme::Dark
    }

    #[allow(dead_code)]
    pub(crate) fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            window::frames().map(|_| Message::Frame),
            iced::event::listen().map(Message::IcedEvent),
        ])
    }
}

#[derive(Debug)]
struct DisplayPanesOverlay {
    panes: Vec<control::UiPaneSnapshot>,
    font: Font,
}

#[derive(Debug, Clone)]
pub(crate) struct ChooseTreeEntry {
    pub(crate) tab_index: usize,
    pub(crate) pane_id: crate::pane::PaneId,
    pub(crate) pane_index: usize,
    pub(crate) focused: bool,
    pub(crate) tab_title: String,
    pub(crate) cwd: String,
    pub(crate) preview: String,
}

impl<Message> canvas::Program<Message> for DisplayPanesOverlay {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry<Renderer>> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());

        for (index, pane) in self.panes.iter().enumerate().take(9) {
            let width = pane.frame.width as f32;
            let height = pane.frame.height as f32;
            if width <= 0.0 || height <= 0.0 {
                continue;
            }

            let badge_size = 40.0f32.min(width.max(0.0)).min(height.max(0.0)).max(28.0);
            let x = pane.frame.x as f32 + (width - badge_size) * 0.5;
            let y = pane.frame.y as f32 + (height - badge_size) * 0.5;
            let badge = canvas::Path::rounded_rectangle(
                Point::new(x, y),
                Size::new(badge_size, badge_size),
                iced::border::Radius::from(8.0),
            );

            frame.fill(
                &badge,
                if pane.focused {
                    Color::from_rgba(0.28, 0.38, 0.72, 0.92)
                } else {
                    Color::from_rgba(0.08, 0.08, 0.08, 0.88)
                },
            );
            frame.stroke(
                &badge,
                canvas::Stroke::default()
                    .with_width(2.0)
                    .with_color(Color::from_rgba(0.92, 0.92, 0.92, 0.95)),
            );
            frame.fill_text(canvas::Text {
                content: (index + 1).to_string(),
                position: Point::new(x + badge_size * 0.5, y + badge_size * 0.5),
                color: Color::WHITE,
                size: Pixels((badge_size * 0.56).round()),
                line_height: iced::widget::text::LineHeight::Relative(1.0),
                font: self.font,
                align_x: iced::widget::text::Alignment::Center,
                align_y: alignment::Vertical::Center,
                shaping: iced::widget::text::Shaping::Basic,
                max_width: badge_size,
            });
        }

        vec![frame.into_geometry()]
    }
}
