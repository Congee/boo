use super::*;
use crate::command_prompt::COMMANDS;
use iced::alignment;
use iced::widget::canvas;
use iced::widget::stack;
use iced::{Pixels, Point, Rectangle, Renderer};

impl BooApp {
    pub(crate) fn visible_pane_snapshots(&self) -> Vec<control::UiPaneSnapshot> {
        let focused_pane = self.server.tabs.focused_pane();
        let terminal_frame = self.terminal_frame();
        self.server
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
            .unwrap_or_default()
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
        iced::theme::Style {
            background_color: Color::TRANSPARENT,
            text_color: Color::WHITE,
        }
    }

    pub(crate) fn ui_snapshot(&self) -> control::UiSnapshot {
        let focused_pane = self.server.tabs.focused_pane();
        let terminal_frame = self.terminal_frame();
        let visible_panes = self.visible_pane_snapshots();

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

        let terminal = self.backend.ui_terminal_snapshot(focused_pane.id());

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

    #[allow(dead_code)]
    pub(crate) fn view(&self) -> Element<'_, Message> {
        let ui_font = self.ui_font();
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
                        .size(13)
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
                .height(Length::Fixed(STATUS_BAR_HEIGHT as f32))
                .padding([2, 6])
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

                let terminal_canvas = vt_terminal_canvas::TerminalCanvas::new(
                    std::sync::Arc::new(snapshot),
                    self.cell_width as f32,
                    self.cell_height as f32,
                    self.terminal_font_size,
                    self.terminal_font_family,
                    self.appearance_revision,
                    self.background_opacity,
                    self.background_opacity_cells,
                    selection_rects,
                    Color::from_rgba(0.65, 0.72, 0.95, 0.35),
                    (!self.preedit_text.is_empty()).then(|| self.preedit_text.clone()),
                );
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
                        container(text(label).font(ui_font).size(13).color(fg))
                            .style(move |_: &Theme| container::Style {
                                background: Some(iced::Background::Color(bg)),
                                ..Default::default()
                            })
                            .width(Length::Fill)
                            .height(Length::Fixed(STATUS_BAR_HEIGHT as f32))
                            .padding([2, 6]),
                    );
                }
                main_col = main_col.push(suggestion_col);
            }

            let prompt_label = format!(": {}_", self.command_prompt.input);
            main_col = main_col.push(
                container(
                    text(prompt_label)
                        .font(ui_font)
                        .size(13)
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
                .height(Length::Fixed(STATUS_BAR_HEIGHT as f32))
                .padding([2, 6]),
            );
        } else {
            let (status_left, status_right) = self.build_status_zones();
            main_col = main_col.push(
                container(
                    row![
                        text(status_left)
                            .font(ui_font)
                            .size(13)
                            .color(Color::from_rgb(0.8, 0.8, 0.8)),
                        iced::widget::Space::new().width(Length::Fill),
                        text(status_right)
                            .font(ui_font)
                            .size(13)
                            .color(Color::from_rgb(0.6, 0.6, 0.6)),
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
                .height(Length::Fixed(STATUS_BAR_HEIGHT as f32))
                .padding([2, 6]),
            );
        }
        let base: Element<'_, Message> = main_col.into();
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
        } else {
            base
        }
    }

    #[allow(dead_code)]
    pub(crate) fn build_status_zones(&self) -> (String, String) {
        let spinner_frame = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| (duration.as_millis() / 125) as usize)
            .unwrap_or(0);
        let tabs = self.server.tabs.tab_info_with_spinner(spinner_frame);
        let mut parts = Vec::new();
        for tab in &tabs {
            let display_idx = tab.index + 1;
            let marker = if tab.active { "*" } else { "" };
            if tab.title.is_empty() {
                parts.push(format!("[{display_idx}{marker}]"));
            } else {
                parts.push(format!("[{display_idx}:{}{marker}]", tab.title));
            }
        }
        let left = parts.join(" ");

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
        let right = right_parts.join("  ");

        (left, right)
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
