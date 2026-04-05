use crate::control;
use crate::vt;
use crate::vt_backend_core;
use crate::vt_terminal_canvas;
use iced::widget::{column, container, row, text};
use iced::window;
use iced::{keyboard, time, Color, Element, Event, Font, Length, Size, Subscription, Task, Theme};
use std::time::Duration;

const STATUS_BAR_HEIGHT: f64 = 20.0;
const DEFAULT_FONT_SIZE: f32 = 14.0;
const SNAPSHOT_TICK_INTERVAL: Duration = Duration::from_millis(16);
const BACKGROUND_POLL_TICKS: u8 = 8;
const FAST_POLL_BURST_TICKS: u8 = 3;

#[derive(Debug, Clone)]
pub enum Message {
    Frame,
    IcedEvent(Event),
}

pub struct ClientApp {
    client: control::Client,
    snapshot: Option<control::UiSnapshot>,
    last_error: Option<String>,
    cell_width: f64,
    cell_height: f64,
    font_size: f32,
    background_opacity: f32,
    background_opacity_cells: bool,
    tick_counter: u8,
    fast_poll_ticks_remaining: u8,
}

impl ClientApp {
    pub fn new(socket_path: String) -> (Self, Task<Message>) {
        let client = control::Client::connect(socket_path);
        let snapshot = client.get_ui_snapshot().ok();
        let font_size = snapshot
            .as_ref()
            .map(|snapshot| snapshot.appearance.font_size)
            .unwrap_or(DEFAULT_FONT_SIZE);
        let (cell_width, cell_height) = terminal_metrics(font_size);
        (
            Self {
                client,
                snapshot,
                last_error: None,
                cell_width,
                cell_height,
                font_size,
                background_opacity: 1.0,
                background_opacity_cells: false,
                tick_counter: 0,
                fast_poll_ticks_remaining: FAST_POLL_BURST_TICKS,
            },
            Task::none(),
        )
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Frame => self.on_tick(),
            Message::IcedEvent(event) => match event {
                Event::Window(window::Event::Resized(size)) => {
                    self.send_resize(size);
                    self.refresh_snapshot();
                }
                Event::Keyboard(event) => self.handle_keyboard(event),
                _ => {}
            },
        }
        Task::none()
    }

    pub fn view(&self) -> Element<'_, Message> {
        let mut main_col = column![];

        if let Some(snapshot) = self.snapshot.as_ref() {
            if let Some(terminal) = snapshot.terminal.as_ref() {
                let terminal_canvas = vt_terminal_canvas::TerminalCanvas::new(
                    ui_terminal_to_vt_snapshot(terminal),
                    self.cell_width as f32,
                    self.cell_height as f32,
                    self.font_size,
                    None,
                    1,
                    self.background_opacity,
                    self.background_opacity_cells,
                    Vec::new(),
                    Color::from_rgba(0.65, 0.72, 0.95, 0.35),
                    None,
                );
                main_col = main_col.push(
                    container(
                        iced::widget::canvas(terminal_canvas)
                            .width(Length::Fill)
                            .height(Length::Fill),
                    )
                    .width(Length::Fill)
                    .height(Length::Fill),
                );
            } else {
                main_col = main_col.push(
                    iced::widget::Space::new()
                        .width(Length::Fill)
                        .height(Length::Fill),
                );
            }

            let (left, right) = build_status(snapshot);
            main_col = main_col.push(
                container(
                    row![
                        text(left)
                            .font(Font::MONOSPACE)
                            .size(13)
                            .color(Color::from_rgb(0.8, 0.8, 0.8)),
                        iced::widget::Space::new().width(Length::Fill),
                        text(right)
                            .font(Font::MONOSPACE)
                            .size(13)
                            .color(Color::from_rgb(0.6, 0.6, 0.6)),
                    ]
                    .width(Length::Fill),
                )
                .style(|_: &Theme| container::Style {
                    background: Some(iced::Background::Color(Color::from_rgba(
                        0.12, 0.12, 0.12, 0.92,
                    ))),
                    ..Default::default()
                })
                .width(Length::Fill)
                .height(Length::Fixed(STATUS_BAR_HEIGHT as f32))
                .padding([2, 6]),
            );
        } else {
            let message = self
                .last_error
                .clone()
                .unwrap_or_else(|| "waiting for boo server".to_string());
            main_col = main_col.push(
                container(text(message).font(Font::MONOSPACE).size(14))
                    .width(Length::Fill)
                    .height(Length::Fill),
            );
        }

        main_col.into()
    }

    pub fn theme(&self) -> Theme {
        Theme::Dark
    }

    pub fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            time::every(SNAPSHOT_TICK_INTERVAL).map(|_| Message::Frame),
            iced::event::listen().map(Message::IcedEvent),
        ])
    }

    pub fn window_style(&self) -> iced::theme::Style {
        iced::theme::Style {
            background_color: Color::TRANSPARENT,
            text_color: Color::WHITE,
        }
    }

    fn refresh_snapshot(&mut self) {
        match self.client.get_ui_snapshot() {
            Ok(snapshot) => {
                self.font_size = snapshot.appearance.font_size.max(8.0);
                self.background_opacity = snapshot.appearance.background_opacity;
                self.background_opacity_cells = snapshot.appearance.background_opacity_cells;
                (self.cell_width, self.cell_height) = terminal_metrics(self.font_size);
                self.snapshot = Some(snapshot);
                self.last_error = None;
            }
            Err(error) => {
                self.last_error = Some(error);
            }
        }
    }

    fn on_tick(&mut self) {
        self.tick_counter = self.tick_counter.wrapping_add(1);
        let should_refresh = if self.fast_poll_ticks_remaining > 0 {
            self.fast_poll_ticks_remaining -= 1;
            true
        } else {
            self.tick_counter % BACKGROUND_POLL_TICKS == 0
        };
        if should_refresh {
            self.refresh_snapshot();
        }
    }

    fn send_resize(&mut self, size: Size) {
        let cols = ((size.width as f64 / self.cell_width).floor() as u16).max(2);
        let rows = (((size.height as f64 - STATUS_BAR_HEIGHT).max(1.0) / self.cell_height).floor()
            as u16)
            .max(1);
        let _ = self.client.send(&control::Request::ResizeFocused { cols, rows });
    }

    fn handle_keyboard(&mut self, event: keyboard::Event) {
        let keyboard::Event::KeyPressed { key, text, modifiers, .. } = event else {
            return;
        };

        if let Some(committed) = text
            .as_ref()
            .map(ToString::to_string)
            .filter(|text| !text.is_empty())
            .filter(|_| !(modifiers.control() || modifiers.alt() || modifiers.logo()))
        {
            let _ = self.client.send(&control::Request::SendText { text: committed });
            self.fast_poll_ticks_remaining = FAST_POLL_BURST_TICKS;
            return;
        }

        if let Some(keyspec) = keyspec_from_key(&key, modifiers, text.as_deref()) {
            let _ = self.client.send(&control::Request::SendKey { key: keyspec });
            self.fast_poll_ticks_remaining = FAST_POLL_BURST_TICKS;
        }
    }
}

fn terminal_metrics(font_size: f32) -> (f64, f64) {
    let size = font_size.max(8.0) as f64;
    let cell_width = (size * 0.62).max(6.0).ceil();
    let cell_height = (size * 1.35).max(12.0).ceil();
    (cell_width, cell_height)
}

fn ui_terminal_to_vt_snapshot(snapshot: &control::UiTerminalSnapshot) -> vt_backend_core::TerminalSnapshot {
    vt_backend_core::TerminalSnapshot {
        cols: snapshot.cols,
        rows: snapshot.rows,
        title: snapshot.title.clone(),
        pwd: snapshot.pwd.clone(),
        cursor: vt_backend_core::CursorSnapshot {
            visible: snapshot.cursor.visible,
            x: snapshot.cursor.x,
            y: snapshot.cursor.y,
            style: snapshot.cursor.style,
        },
        rows_data: snapshot
            .rows_data
            .iter()
            .map(|row| {
                row.cells
                    .iter()
                    .map(|cell| vt_backend_core::CellSnapshot {
                        text: cell.text.clone(),
                        display_width: cell.display_width,
                        fg: vt::GhosttyColorRgb {
                            r: cell.fg[0],
                            g: cell.fg[1],
                            b: cell.fg[2],
                        },
                        bg: vt::GhosttyColorRgb {
                            r: cell.bg[0],
                            g: cell.bg[1],
                            b: cell.bg[2],
                        },
                        bold: cell.bold,
                        italic: cell.italic,
                        underline: cell.underline,
                    })
                    .collect()
            })
            .collect(),
        scrollbar: Default::default(),
        colors: Default::default(),
    }
}

fn build_status(snapshot: &control::UiSnapshot) -> (String, String) {
    let left = snapshot
        .tabs
        .iter()
        .map(|tab| {
            let display_idx = tab.index + 1;
            let marker = if tab.active { "*" } else { "" };
            if tab.title.is_empty() {
                format!("[{display_idx}{marker}]")
            } else {
                format!("[{display_idx}:{}{marker}]", tab.title)
            }
        })
        .collect::<Vec<_>>()
        .join(" ");

    let mut right_parts = Vec::new();
    if snapshot.visible_panes.len() > 1 {
        right_parts.push(format!("{} panes", snapshot.visible_panes.len()));
    }
    if !snapshot.pwd.is_empty() {
        right_parts.push(snapshot.pwd.clone());
    }
    (left, right_parts.join("  "))
}

fn keyspec_from_key(
    key: &keyboard::Key,
    modifiers: keyboard::Modifiers,
    text: Option<&str>,
) -> Option<String> {
    use keyboard::key::Named;

    let mut parts = Vec::new();
    if modifiers.control() {
        parts.push("ctrl");
    }
    if modifiers.alt() {
        parts.push("alt");
    }
    if modifiers.shift() {
        parts.push("shift");
    }
    if modifiers.logo() {
        parts.push("super");
    }

    let base = match key {
        keyboard::Key::Named(Named::Enter) => "enter".to_string(),
        keyboard::Key::Named(Named::Tab) => "tab".to_string(),
        keyboard::Key::Named(Named::Space) => "space".to_string(),
        keyboard::Key::Named(Named::Escape) => "escape".to_string(),
        keyboard::Key::Named(Named::Backspace) => "backspace".to_string(),
        keyboard::Key::Named(Named::Delete) => "delete".to_string(),
        keyboard::Key::Named(Named::ArrowUp) => "up".to_string(),
        keyboard::Key::Named(Named::ArrowDown) => "down".to_string(),
        keyboard::Key::Named(Named::ArrowLeft) => "left".to_string(),
        keyboard::Key::Named(Named::ArrowRight) => "right".to_string(),
        keyboard::Key::Named(Named::PageUp) => "pageup".to_string(),
        keyboard::Key::Named(Named::PageDown) => "pagedown".to_string(),
        keyboard::Key::Named(Named::Home) => "home".to_string(),
        keyboard::Key::Named(Named::End) => "end".to_string(),
        keyboard::Key::Character(chars) => chars
            .chars()
            .next()
            .map(|ch| ch.to_ascii_lowercase().to_string())
            .or_else(|| text.and_then(|text| text.chars().next().map(|ch| ch.to_string())))?,
        _ => return None,
    };

    parts.push(base.as_str());
    Some(parts.join("+"))
}
