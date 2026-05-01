use crate::bindings;
use crate::control;
use crate::cursor_blink_visible;
use crate::iced_mods_to_ghostty;
use crate::keymap;
use crate::remote;
use crate::vt;
use crate::vt_backend_core;
use crate::vt_terminal_canvas;
use crate::{AppKeyEvent, AppMouseButton, AppMouseEvent};
use iced::advanced::widget::Tree;
use iced::advanced::{
    Clipboard, InputMethod, Layout, Shell, Widget, input_method, layout, renderer,
};
use iced::futures::{SinkExt, StreamExt};
use iced::stream;
use iced::widget::{column, container, row, stack, text};
use iced::window;
use iced::{
    Color, Element, Event, Font, Length, Point, Rectangle, Size, Subscription, Task, Theme,
    keyboard, mouse, time,
};
use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixListener;
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

const DEFAULT_FONT_SIZE: f32 = 14.0;
const DEFAULT_CURSOR_BLINK_INTERVAL: Duration = Duration::from_millis(600);
const STREAM_RECONNECT_DELAY: Duration = Duration::from_millis(250);
const LOCAL_STREAM_INPUT_SEQ_LEN: usize = 8;
const REMOTE_FULL_STATE_HEADER_LEN: usize = 14;
const LOCAL_FULL_STATE_HEADER_LEN: usize =
    LOCAL_STREAM_INPUT_SEQ_LEN + REMOTE_FULL_STATE_HEADER_LEN;
const REMOTE_DELTA_HEADER_LEN: usize = 13;
const LOCAL_DELTA_HEADER_LEN: usize = LOCAL_STREAM_INPUT_SEQ_LEN + REMOTE_DELTA_HEADER_LEN;
const REMOTE_CELL_ENCODED_LEN: usize = 12;

fn ime_debug_enabled() -> bool {
    std::env::var_os("BOO_IME_DEBUG").is_some()
}

fn remote_debug_status_enabled() -> bool {
    std::env::var_os("BOO_REMOTE_DEBUG_STATUS").is_some()
}

fn ime_debug(args: std::fmt::Arguments<'_>) {
    if ime_debug_enabled() {
        eprintln!("[boo-ime] {args}");
    }
}

macro_rules! ime_debug {
    ($($arg:tt)*) => {
        crate::client_gui::ime_debug(format_args!($($arg)*))
    };
}

#[derive(Debug, Clone)]
pub enum Message {
    Frame,
    RemoteDiagnosticsTick,
    IcedEvent(Event),
    StreamReady(std::sync::mpsc::Sender<StreamCommand>),
    StreamEvent(LocalStreamEvent),
    GuiTest(GuiTestCommand),
    ActivateTab(u32),
}

#[derive(Debug, Clone, PartialEq)]
pub enum GuiTestCommand {
    Text(String),
    Key(String),
    AppKey { keyspec: String, repeat: bool },
    Keyboard { keyspec: String, repeat: bool },
    Command(String),
    ActivateTab(u32),
    Click { x: f64, y: f64 },
    Drag { x1: f64, y1: f64, x2: f64, y2: f64 },
    Resize { cols: u16, rows: u16 },
    Refresh,
}

#[derive(Debug, Clone, Default)]
struct GuiTestStatus {
    mode: &'static str,
    stream_ready: bool,
    has_terminal: bool,
    active_tab: usize,
    stream_seq: u64,
    render_seq: u64,
    keyboard_seq: u64,
    input_method_commit_seq: u64,
    last_stream_ms: u64,
    last_render_ms: u64,
    cursor_row: Option<u16>,
    cursor_col: Option<u16>,
    cursor_row_text: String,
    row0_text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClientMode {
    Bootstrapping,
    Active,
    Recovering,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct ClientUiState {
    tabs: Vec<ClientTabState>,
    active_tab: usize,
    pwd: String,
    pane_count: usize,
    status_bar: crate::status_components::UiStatusBarSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ClientTabState {
    index: usize,
    tab_id: Option<u32>,
    active: bool,
    title: String,
    pane_count: usize,
}

pub struct ClientApp {
    socket_path: String,
    remote_host: Option<String>,
    client: control::Client,
    stream_tx: Option<std::sync::mpsc::Sender<StreamCommand>>,
    bootstrapped: bool,
    ui_state: ClientUiState,
    visible_panes: Vec<control::UiPaneSnapshot>,
    mouse_selection: control::UiMouseSelectionSnapshot,
    mode: ClientMode,
    runtime_view_id: u64,
    runtime_revision: u64,
    view_revision: u64,
    active_remote_tab_id: Option<u32>,
    pane_snapshots: HashMap<u64, Arc<vt_backend_core::TerminalSnapshot>>,
    focused_pane_id: u64,
    last_error: Option<String>,
    font_families: Arc<[&'static str]>,
    cell_width: f64,
    cell_height: f64,
    font_size: f32,
    background_opacity: f32,
    background_opacity_cells: bool,
    terminal_foreground: crate::config::RgbColor,
    terminal_background: crate::config::RgbColor,
    cursor_color: crate::config::RgbColor,
    selection_background: crate::config::RgbColor,
    selection_foreground: crate::config::RgbColor,
    cursor_text_color: crate::config::RgbColor,
    url_color: crate::config::RgbColor,
    active_tab_foreground: crate::config::RgbColor,
    active_tab_background: crate::config::RgbColor,
    inactive_tab_foreground: crate::config::RgbColor,
    inactive_tab_background: crate::config::RgbColor,
    cursor_blink_interval: Duration,
    app_focused: bool,
    cursor_blink_epoch: Instant,
    focused_cursor_position: Option<(u64, u16, u16)>,
    preedit_text: String,
    next_input_seq: u64,
    pending_input_latencies: BTreeMap<u64, Instant>,
    steady_state_snapshot_requests: u64,
    should_exit: bool,
    terminal_snapshot_generation: u64,
    appearance_revision: u64,
    next_full_snapshot_revision: u64,
    next_snapshot_generation: u64,
    last_mouse_pos: Point,
    last_requested_viewport_points: Option<(u32, u32)>,
    remote_debug_enabled: bool,
    remote_debug_summary: Option<String>,
}

#[derive(Clone, Copy)]
enum SnapshotRefreshReason {
    TextFallback,
    StreamFallback,
    GuiTestText,
    GuiTestKey,
    GuiTestManual,
}

impl SnapshotRefreshReason {
    fn profile_path(self) -> &'static str {
        match self {
            Self::TextFallback => "client.control.get_ui_snapshot.text_fallback",
            Self::StreamFallback => "client.control.get_ui_snapshot.stream_fallback",
            Self::GuiTestText => "client.control.get_ui_snapshot.gui_test_text",
            Self::GuiTestKey => "client.control.get_ui_snapshot.gui_test_key",
            Self::GuiTestManual => "client.control.get_ui_snapshot.gui_test_manual",
        }
    }
}

impl ClientApp {
    fn status_bar_height(&self) -> f64 {
        crate::status_bar_metrics(self.font_size, self.font_families.first().copied()).height
    }

    fn status_bar_text_size(&self) -> f32 {
        crate::status_bar_metrics(self.font_size, self.font_families.first().copied()).text_size
    }

    fn apply_ui_appearance(&mut self, appearance: &control::UiAppearanceSnapshot) {
        self.font_size = appearance.font_size.max(8.0);
        self.font_families = appearance
            .font_families
            .iter()
            .map(|family| crate::leak_font_family(family))
            .collect::<Vec<_>>()
            .into();
        self.background_opacity = appearance.background_opacity;
        self.background_opacity_cells = appearance.background_opacity_cells;
        self.terminal_foreground = appearance.terminal_foreground;
        self.terminal_background = appearance.terminal_background;
        self.cursor_color = appearance.cursor_color;
        self.selection_background = appearance.selection_background;
        self.selection_foreground = appearance.selection_foreground;
        self.cursor_text_color = appearance.cursor_text_color;
        self.url_color = appearance.url_color;
        self.active_tab_foreground = appearance.active_tab_foreground;
        self.active_tab_background = appearance.active_tab_background;
        self.inactive_tab_foreground = appearance.inactive_tab_foreground;
        self.inactive_tab_background = appearance.inactive_tab_background;
        self.cursor_blink_interval = Duration::from_nanos(appearance.cursor_blink_interval_ns);
        self.cursor_blink_epoch = Instant::now();
        (self.cell_width, self.cell_height) =
            crate::terminal_metrics(self.font_size, self.font_families.first().copied());
        self.appearance_revision = self.appearance_revision.wrapping_add(1);
    }

    fn apply_ui_runtime_state(&mut self, state: control::UiRuntimeState) {
        if self.view_revision != 0 && state.view_revision != self.view_revision {
            self.pane_snapshots.clear();
        }
        self.runtime_view_id = state.view_id;
        self.runtime_revision = state.runtime_revision;
        self.view_revision = state.view_revision;
        let ui_state = ClientUiState::from_runtime_state(&state);
        if let Some(active_tab_id) = ui_state
            .tabs
            .get(ui_state.active_tab)
            .and_then(|tab| tab.tab_id)
        {
            self.active_remote_tab_id = Some(active_tab_id);
            self.should_exit = false;
        }
        self.ui_state = ui_state;
        self.visible_panes = state.visible_panes;
        self.mouse_selection = state.mouse_selection;
        self.focused_pane_id = state.focused_pane;
        self.observe_focused_cursor_position();
        self.last_error = None;
    }

    fn apply_ui_snapshot(&mut self, snapshot: control::UiSnapshot) {
        self.apply_ui_appearance(&snapshot.appearance);
        self.ui_state = ClientUiState::from_snapshot(&snapshot);
        self.visible_panes = snapshot.visible_panes.clone();
        self.mouse_selection = snapshot.mouse_selection.clone();
        self.focused_pane_id = snapshot.focused_pane;
        self.pane_snapshots = pane_snapshot_map_from_ui_snapshot(&snapshot);
        self.observe_focused_cursor_position();
        self.bootstrapped = true;
        self.terminal_snapshot_generation = self.allocate_snapshot_generation();
        self.last_error = None;
    }

    fn has_paintable_terminal(&self) -> bool {
        !self.pane_snapshots.is_empty()
    }

    fn focused_pane_has_blinking_cursor(&self) -> bool {
        self.pane_snapshots
            .get(&self.focused_pane_id)
            .is_some_and(|snapshot| snapshot.cursor.visible && snapshot.cursor.blinking)
    }

    fn stream_ready_for_terminal_io(&self) -> bool {
        self.stream_tx.is_some() && matches!(self.mode, ClientMode::Active)
    }

    fn theme_color(color: crate::config::RgbColor, alpha: f32) -> Color {
        Color::from_rgba8(color[0], color[1], color[2], alpha.clamp(0.0, 1.0))
    }

    pub fn new_with_remote_host(
        socket_path: String,
        remote_host: Option<String>,
    ) -> (Self, Task<Message>) {
        let client = control::Client::connect(socket_path.clone());
        let font_size = DEFAULT_FONT_SIZE;
        let font_families: Arc<[&'static str]> = Arc::from([]);
        let background_opacity = 1.0;
        let background_opacity_cells = false;
        let cursor_blink_interval = DEFAULT_CURSOR_BLINK_INTERVAL;
        let terminal_foreground = crate::DEFAULT_TERMINAL_FOREGROUND;
        let terminal_background = crate::DEFAULT_TERMINAL_BACKGROUND;
        let cursor_color = crate::DEFAULT_CURSOR_COLOR;
        let selection_background = crate::DEFAULT_SELECTION_BACKGROUND;
        let selection_foreground = crate::DEFAULT_SELECTION_FOREGROUND;
        let cursor_text_color = crate::DEFAULT_CURSOR_TEXT_COLOR;
        let url_color = crate::DEFAULT_URL_COLOR;
        let active_tab_foreground = crate::DEFAULT_ACTIVE_TAB_FOREGROUND;
        let active_tab_background = crate::DEFAULT_ACTIVE_TAB_BACKGROUND;
        let inactive_tab_foreground = crate::DEFAULT_INACTIVE_TAB_FOREGROUND;
        let inactive_tab_background = crate::DEFAULT_INACTIVE_TAB_BACKGROUND;
        let (cell_width, cell_height) =
            crate::terminal_metrics(font_size, font_families.first().copied());
        let ui_state = ClientUiState::default();
        let pane_snapshots = HashMap::new();
        let focused_pane_id = 0;
        let focused_cursor_position = focused_cursor_position(focused_pane_id, &pane_snapshots);
        let app = Self {
            socket_path,
            remote_host,
            client,
            stream_tx: None,
            bootstrapped: false,
            ui_state,
            visible_panes: Vec::new(),
            mouse_selection: control::UiMouseSelectionSnapshot::default(),
            mode: ClientMode::Bootstrapping,
            runtime_view_id: 0,
            runtime_revision: 0,
            view_revision: 0,
            active_remote_tab_id: None,
            pane_snapshots,
            focused_pane_id,
            last_error: None,
            font_families,
            cell_width,
            cell_height,
            font_size,
            background_opacity,
            background_opacity_cells,
            terminal_foreground,
            terminal_background,
            cursor_color,
            selection_background,
            selection_foreground,
            cursor_text_color,
            url_color,
            active_tab_foreground,
            active_tab_background,
            inactive_tab_foreground,
            inactive_tab_background,
            cursor_blink_interval,
            app_focused: true,
            cursor_blink_epoch: Instant::now(),
            focused_cursor_position,
            preedit_text: String::new(),
            next_input_seq: 1,
            pending_input_latencies: BTreeMap::new(),
            steady_state_snapshot_requests: 0,
            should_exit: false,
            terminal_snapshot_generation: 1,
            appearance_revision: 1,
            next_full_snapshot_revision: 1,
            next_snapshot_generation: 2,
            last_mouse_pos: Point::ORIGIN,
            last_requested_viewport_points: None,
            remote_debug_enabled: remote_debug_status_enabled(),
            remote_debug_summary: None,
        };
        app.update_gui_test_status();
        (app, Task::none())
    }

    #[cfg(test)]
    pub fn new(socket_path: String) -> (Self, Task<Message>) {
        Self::new_with_remote_host(socket_path, None)
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        let mut tasks = Vec::new();
        match message {
            Message::Frame => self.on_tick(),
            Message::RemoteDiagnosticsTick => self.refresh_remote_debug_summary(),
            Message::StreamReady(tx) => {
                self.stream_tx = Some(tx);
            }
            Message::StreamEvent(event) => {
                if let Some(task) = self.handle_stream_delivery(event) {
                    tasks.push(task);
                }
            }
            Message::GuiTest(command) => self.handle_gui_test(command),
            Message::ActivateTab(tab_id) => self.activate_tab(tab_id),
            Message::IcedEvent(event) => match event {
                Event::Window(window::Event::Resized(size)) => {
                    self.send_resize(size);
                }
                Event::Window(window::Event::Focused) => {
                    self.app_focused = true;
                    self.cursor_blink_epoch = Instant::now();
                }
                Event::Window(window::Event::Unfocused) => {
                    self.app_focused = false;
                    self.cursor_blink_epoch = Instant::now();
                }
                Event::Keyboard(event) => self.handle_keyboard(event),
                Event::InputMethod(event) => self.handle_input_method(event),
                Event::Mouse(event) => self.handle_mouse(event),
                _ => {}
            },
        }
        self.update_gui_test_status();
        if self.should_exit {
            iced::exit()
        } else {
            Task::batch(tasks)
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        let mut main_col = column![].width(Length::Fill).height(Length::Fill);

        if self.bootstrapped && !self.visible_panes.is_empty() && !self.pane_snapshots.is_empty() {
            main_col = main_col.push(self.render_terminal_scene());
        } else if self.bootstrapped {
            main_col = main_col.push(self.render_terminal_placeholder(None));
        } else {
            let message = self
                .last_error
                .clone()
                .unwrap_or_else(|| "waiting for boo server".to_string());
            main_col = main_col.push(self.render_terminal_placeholder(Some(message)));
        }

        let right = if self.ui_state.status_bar.right.is_empty() {
            build_status_right(
                &self.ui_state,
                self.mode,
                self.active_remote_tab_id,
                self.last_error.as_deref(),
                self.remote_host.as_deref(),
                self.remote_debug_summary.as_deref(),
            )
        } else {
            String::new()
        };
        let status_bar_height = self.status_bar_height() as f32;
        let status_text_size = self.status_bar_text_size();
        let left_segments = render_status_zone(&self.ui_state.status_bar.left, status_text_size);
        let right_segments = if self.ui_state.status_bar.right.is_empty() {
            render_status_text(right, status_text_size)
        } else {
            render_status_zone(&self.ui_state.status_bar.right, status_text_size)
        };
        let mut tabs_row = row![].spacing(0);
        for tab in &self.ui_state.tabs {
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
                iced::widget::button(
                    text(label)
                        .font(Font::MONOSPACE)
                        .size(status_text_size)
                        .color(fg),
                )
                .padding(0)
                .style(move |_: &Theme, _| iced::widget::button::Style {
                    background: Some(iced::Background::Color(bg)),
                    text_color: fg,
                    ..Default::default()
                })
                .on_press_maybe(tab.tab_id.map(Message::ActivateTab)),
            );
        }
        main_col = main_col.push(
            container(
                row![
                    left_segments,
                    iced::widget::Space::new().width(Length::Fill),
                    tabs_row,
                    iced::widget::Space::new().width(Length::Fill),
                    right_segments,
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
            .height(Length::Fixed(status_bar_height))
            .padding(0),
        );

        container(main_col)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    pub fn theme(&self) -> Theme {
        Theme::Dark
    }

    fn terminal_background_color(&self) -> Color {
        Self::theme_color(self.terminal_background, self.background_opacity)
    }

    fn render_terminal_placeholder(&self, message: Option<String>) -> Element<'_, Message> {
        let background = self.terminal_background_color();
        let content: Element<'_, Message> = match message {
            Some(message) => text(message)
                .font(Font::MONOSPACE)
                .size(14)
                .color(Self::theme_color(self.terminal_foreground, 1.0))
                .into(),
            None => iced::widget::Space::new()
                .width(Length::Fill)
                .height(Length::Fill)
                .into(),
        };
        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_: &Theme| container::Style {
                background: Some(iced::Background::Color(background)),
                ..Default::default()
            })
            .into()
    }

    fn render_terminal_scene<'a>(&'a self) -> Element<'a, Message> {
        let _scope = crate::profiling::scope(
            "client.view.render_terminal_scene",
            crate::profiling::Kind::Cpu,
        );
        note_gui_test_render_activity();
        crate::profiling::record_units(
            "client.view.render_terminal_scene.panes",
            crate::profiling::Kind::Cpu,
            self.visible_panes.len() as u64,
        );
        let selection_background = Self::theme_color(self.selection_background, 0.35);
        let selection_foreground = Self::theme_color(self.selection_foreground, 1.0);
        let cursor_text_color = Self::theme_color(self.cursor_text_color, 1.0);
        let url_color = Self::theme_color(self.url_color, 1.0);
        let mut layers: Vec<Element<'a, Message>> = {
            let _scope = crate::profiling::scope(
                "client.view.render_terminal_scene.background",
                crate::profiling::Kind::Cpu,
            );
            vec![
                iced::widget::canvas(vt_terminal_canvas::TerminalBackgroundCanvas {
                    color: Self::theme_color(self.terminal_background, self.background_opacity),
                })
                .width(Length::Fill)
                .height(Length::Fill)
                .into(),
            ]
        };
        for pane in &self.visible_panes {
            crate::profiling::record_units(
                "client.view.render_terminal_scene.pane",
                crate::profiling::Kind::Cpu,
                1,
            );
            let Some(terminal_snapshot) = self.pane_snapshots.get(&pane.pane_id) else {
                continue;
            };
            let cursor_blink_visible = pane_cursor_blink_visible(
                pane.focused,
                terminal_snapshot.cursor.blinking,
                self.app_focused,
                self.cursor_blink_epoch,
                self.cursor_blink_interval,
            );
            let selection_rects: Arc<[vt_terminal_canvas::TerminalSelectionRect]> = if self
                .mouse_selection
                .active
                && self.mouse_selection.pane_id == Some(pane.pane_id)
            {
                self.mouse_selection
                    .selection_rects
                    .iter()
                    .map(|rect| vt_terminal_canvas::TerminalSelectionRect {
                        x: rect.x as f32,
                        y: rect.y as f32,
                        width: rect.width as f32,
                        height: rect.height as f32,
                    })
                    .collect::<Vec<_>>()
                    .into()
            } else {
                Arc::from([])
            };
            let viewport = vt_terminal_canvas::TerminalViewport {
                x: pane.frame.x as f32,
                y: pane.frame.y as f32,
                width: pane.frame.width as f32,
                height: pane.frame.height as f32,
                content_offset_y: 0.0,
            };
            let preedit_text =
                (pane.focused && !self.preedit_text.is_empty()).then(|| self.preedit_text.clone());
            let terminal_canvas = {
                let _scope = crate::profiling::scope(
                    "client.view.render_terminal_scene.pane_canvas",
                    crate::profiling::Kind::Cpu,
                );
                vt_terminal_canvas::TerminalCanvas::new(
                    Arc::clone(terminal_snapshot),
                    self.cell_width as f32,
                    self.cell_height as f32,
                    self.font_size,
                    self.font_families.clone(),
                    self.terminal_snapshot_generation,
                    1,
                    self.background_opacity,
                    self.background_opacity_cells,
                    cursor_blink_visible,
                    Arc::clone(&selection_rects),
                    selection_background,
                    Some(selection_foreground),
                    Some(cursor_text_color),
                    Some(url_color),
                    preedit_text.clone(),
                )
                .without_base_fill()
                .without_text_fill()
                .new_with_viewport(viewport)
            };
            layers.push(
                container(
                    iced::widget::canvas(terminal_canvas)
                        .width(Length::Fill)
                        .height(Length::Fill),
                )
                .width(Length::Fill)
                .height(Length::Fill)
                .into(),
            );
            let text_layer = {
                let _scope = crate::profiling::scope(
                    "client.view.render_terminal_scene.pane_text",
                    crate::profiling::Kind::Cpu,
                );
                Element::new(
                    vt_terminal_canvas::TerminalBodyLayer::new(
                        Arc::clone(terminal_snapshot),
                        self.cell_width as f32,
                        self.cell_height as f32,
                        self.font_size,
                        self.font_families.clone(),
                        self.terminal_snapshot_generation,
                        self.appearance_revision,
                        cursor_blink_visible,
                        selection_rects,
                        Some(selection_foreground),
                        Some(cursor_text_color),
                        Some(url_color),
                        preedit_text,
                    )
                    .new_with_viewport(viewport),
                )
            };
            layers.push(text_layer);
        }
        {
            let _scope = crate::profiling::scope(
                "client.view.render_terminal_scene.borders",
                crate::profiling::Kind::Cpu,
            );
            layers.push(
                iced::widget::canvas(PaneBordersOverlay {
                    panes: self.visible_panes.clone(),
                })
                .width(Length::Fill)
                .height(Length::Fill)
                .into(),
            );
        }
        {
            let _scope = crate::profiling::scope(
                "client.view.render_terminal_scene.ime_layer",
                crate::profiling::Kind::Cpu,
            );
            layers.push(TerminalInputMethodLayer::element(
                self.focused_cursor_rect(),
                self.font_size,
                self.preedit_text.clone(),
                self.app_focused,
            ));
        }
        {
            let _scope = crate::profiling::scope(
                "client.view.render_terminal_scene.stack",
                crate::profiling::Kind::Cpu,
            );
            // The first stack layer owns the terminal background. Do not also
            // paint the outer container or translucent backgrounds compound
            // into an effectively opaque/dark terminal body.
            container(stack(layers).width(Length::Fill).height(Length::Fill))
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        }
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let mut subscriptions = vec![
            window::frames().map(|_| Message::Frame),
            iced::event::listen().map(Message::IcedEvent),
            iced::Subscription::run_with(self.socket_path.clone(), local_stream_subscription),
            gui_test_subscription(),
        ];
        if self.remote_debug_enabled {
            subscriptions
                .push(time::every(Duration::from_secs(1)).map(|_| Message::RemoteDiagnosticsTick));
        }
        if !self.cursor_blink_interval.is_zero()
            && self.focused_pane_has_blinking_cursor()
            && self.has_paintable_terminal()
        {
            subscriptions.push(time::every(self.cursor_blink_interval).map(|_| Message::Frame));
        }
        Subscription::batch(subscriptions)
    }

    pub fn window_style(&self) -> iced::theme::Style {
        iced::theme::Style {
            background_color: Color::TRANSPARENT,
            text_color: Color::WHITE,
        }
    }

    fn refresh_snapshot(&mut self, reason: SnapshotRefreshReason) {
        let _scope =
            crate::profiling::scope("client.control.get_ui_snapshot", crate::profiling::Kind::Io);
        crate::profiling::record_units(reason.profile_path(), crate::profiling::Kind::Io, 1);
        match self.client.get_ui_snapshot() {
            Ok(snapshot) => {
                if matches!(self.mode, ClientMode::Active) {
                    self.steady_state_snapshot_requests =
                        self.steady_state_snapshot_requests.saturating_add(1);
                    crate::profiling::record_units(
                        "client.control.get_ui_snapshot.steady_state",
                        crate::profiling::Kind::Io,
                        1,
                    );
                }
                crate::profiling::record_units(
                    "client.control.get_ui_snapshot.ok",
                    crate::profiling::Kind::Io,
                    1,
                );
                self.apply_ui_snapshot(snapshot);
            }
            Err(error) => {
                crate::profiling::record_units(
                    "client.control.get_ui_snapshot.err",
                    crate::profiling::Kind::Io,
                    1,
                );
                self.last_error = Some(error);
            }
        }
    }

    fn refresh_remote_debug_summary(&mut self) {
        if !self.remote_debug_enabled {
            return;
        }
        self.remote_debug_summary = match self.client.get_remote_clients() {
            Ok(snapshot) => {
                let viewing = snapshot
                    .clients
                    .iter()
                    .filter(|client| client.subscribed_to_runtime)
                    .count();
                let pending = snapshot
                    .clients
                    .iter()
                    .filter(|client| client.challenge_pending)
                    .count();
                let stale_heartbeats = snapshot
                    .clients
                    .iter()
                    .filter(|client| client.heartbeat_overdue)
                    .count();
                Some(format!(
                    "diag s={} c={} v={} p={} h={}",
                    snapshot.servers.len(),
                    snapshot.clients.len(),
                    viewing,
                    pending,
                    stale_heartbeats
                ))
            }
            Err(error) => Some(format!("diag error: {error}")),
        };
    }

    fn on_tick(&mut self) {
        if matches!(self.mode, ClientMode::Active)
            || self.stream_tx.is_some()
            || self.has_paintable_terminal()
        {}
    }

    fn send_text_input(&mut self, text: String) {
        if self.stream_ready_for_terminal_io() {
            ime_debug!("client send committed text via stream");
            let input_seq = self.record_pending_input();
            self.send_stream_command(StreamCommand::Input {
                input_seq,
                bytes: text.into_bytes(),
            });
        } else if matches!(self.mode, ClientMode::Bootstrapping) {
            ime_debug!("client drop committed text while bootstrapping");
        } else {
            ime_debug!("client send committed text via control");
            let _ = self.client.send(&control::Request::SendText { text });
            self.refresh_snapshot(SnapshotRefreshReason::TextFallback);
        }
    }

    fn handle_input_method(&mut self, event: input_method::Event) {
        ime_debug!(
            "client iced input method event: {}",
            iced_input_method_event_name(&event)
        );
        match event {
            input_method::Event::Opened => {
                self.preedit_text.clear();
            }
            input_method::Event::Preedit(text, _selection) => {
                if text.is_empty() {
                    self.preedit_text.clear();
                } else {
                    self.preedit_text = text;
                }
            }
            input_method::Event::Commit(text) => {
                note_gui_test_input_method_commit();
                self.preedit_text.clear();
                self.send_text_input(text);
            }
            input_method::Event::Closed => {
                self.preedit_text.clear();
            }
        }
    }

    fn activate_tab(&mut self, tab_id: u32) {
        if let Some(index) = self
            .ui_state
            .tabs
            .iter()
            .position(|tab| tab.tab_id == Some(tab_id))
        {
            self.ui_state.active_tab = index;
            for tab in &mut self.ui_state.tabs {
                tab.active = tab.tab_id == Some(tab_id);
            }
        }
        self.active_remote_tab_id = Some(tab_id);
        self.send_runtime_action(remote::RuntimeAction::SetViewedTab {
            view_id: self.runtime_view_id,
            tab_id,
        });
    }

    fn send_resize(&mut self, size: Size) {
        let width = size.width.max(1.0).round() as u32;
        let height = size.height.max(1.0).round() as u32;
        if self.last_requested_viewport_points == Some((width, height)) {
            return;
        }
        self.last_requested_viewport_points = Some((width, height));
        let _ = self.client.send(&control::Request::ResizeViewportPoints {
            width: width as f64,
            height: height as f64,
        });
    }

    fn handle_keyboard(&mut self, event: keyboard::Event) {
        ime_debug!("client iced keyboard event");
        note_gui_test_keyboard_event();
        let keyboard::Event::KeyPressed {
            key,
            modified_key,
            physical_key,
            text,
            modifiers,
            repeat,
            ..
        } = event
        else {
            return;
        };

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
            ime_debug!("client iced keyboard missing native keycode");
            return;
        };
        ime_debug!("client iced keyboard sends app key keycode={keycode}");
        let input_seq = self.record_pending_input();
        let app_event = AppKeyEvent {
            keycode,
            mods: iced_mods_to_ghostty(&modifiers),
            text: text
                .as_ref()
                .map(ToString::to_string)
                .filter(|text| !text.is_empty()),
            modified_text: match &modified_key {
                keyboard::Key::Character(chars) if !chars.is_empty() => Some(chars.to_string()),
                _ => None,
            },
            named_key: named_key_from_iced_key(&key),
            repeat,
            input_seq: Some(input_seq),
        };
        self.send_stream_or_control(
            StreamCommand::AppKeyEvent {
                event: app_event.clone(),
            },
            control::Request::AppKeyEvent { event: app_event },
        );
    }

    fn handle_mouse(&mut self, event: mouse::Event) {
        match event {
            mouse::Event::CursorMoved { position } => {
                self.last_mouse_pos = position;
                self.send_mouse_event(AppMouseEvent::CursorMoved {
                    x: position.x as f64,
                    y: position.y as f64,
                    mods: 0,
                });
            }
            mouse::Event::ButtonPressed(button) => {
                self.send_mouse_event(AppMouseEvent::ButtonPressed {
                    button: app_mouse_button(button),
                    x: self.last_mouse_pos.x as f64,
                    y: self.last_mouse_pos.y as f64,
                    mods: 0,
                });
            }
            mouse::Event::ButtonReleased(button) => {
                self.send_mouse_event(AppMouseEvent::ButtonReleased {
                    button: app_mouse_button(button),
                    x: self.last_mouse_pos.x as f64,
                    y: self.last_mouse_pos.y as f64,
                    mods: 0,
                });
            }
            mouse::Event::WheelScrolled { delta } => {
                let event = match delta {
                    mouse::ScrollDelta::Lines { x, y } => AppMouseEvent::WheelScrolledLines {
                        x: x as f64,
                        y: y as f64,
                        mods: 0,
                    },
                    mouse::ScrollDelta::Pixels { x, y } => AppMouseEvent::WheelScrolledPixels {
                        x: x as f64,
                        y: y as f64,
                        mods: 0,
                    },
                };
                self.send_mouse_event(event);
            }
            _ => {}
        }
    }

    fn send_mouse_event(&mut self, event: AppMouseEvent) {
        self.send_stream_or_control(
            StreamCommand::AppMouseEvent {
                event: event.clone(),
            },
            control::Request::AppMouseEvent { event },
        );
    }

    fn send_stream_or_control(&mut self, stream: StreamCommand, control: control::Request) {
        if self.stream_ready_for_terminal_io() {
            self.send_stream_command(stream);
        } else if matches!(self.mode, ClientMode::Bootstrapping) {
            let _ = stream;
            let _ = control;
        } else {
            let _ = self.client.send(&control);
            self.refresh_snapshot(SnapshotRefreshReason::StreamFallback);
        }
    }

    fn handle_stream_event(&mut self, event: LocalStreamEvent) {
        match event {
            LocalStreamEvent::TabList(tabs) => {
                let live_tabs: Vec<_> = tabs.iter().filter(|tab| !tab.child_exited).collect();
                self.apply_remote_tabs(&tabs);
                if matches!(self.mode, ClientMode::Active)
                    && self
                        .active_remote_tab_id
                        .map(|tab_id| live_tabs.iter().any(|tab| tab.id == tab_id))
                        .unwrap_or(false)
                {
                    self.should_exit = false;
                } else if matches!(self.mode, ClientMode::Bootstrapping)
                    && !self.has_paintable_terminal()
                    && live_tabs.is_empty()
                {
                    self.send_runtime_action(remote::RuntimeAction::NewTab {
                        view_id: self.runtime_view_id,
                        cols: None,
                        rows: None,
                    });
                } else {
                    self.should_exit = live_tabs.is_empty();
                }
            }
            LocalStreamEvent::UiRuntimeState(state) => {
                self.apply_ui_runtime_state(state);
                if matches!(self.mode, ClientMode::Bootstrapping)
                    && !self.has_paintable_terminal()
                    && self.ui_state.tabs.is_empty()
                {
                    self.send_runtime_action(remote::RuntimeAction::NewTab {
                        view_id: self.runtime_view_id,
                        cols: None,
                        rows: None,
                    });
                }
            }
            LocalStreamEvent::UiAppearance(appearance) => {
                self.apply_ui_appearance(&appearance);
            }
            LocalStreamEvent::UiPaneFullState {
                pane_id,
                runtime_revision,
                state,
            } => {
                if runtime_revision != 0 && runtime_revision < self.runtime_revision {
                    return;
                }
                let revision_seed = self.allocate_full_snapshot_revision_seed(state.rows as usize);
                self.terminal_snapshot_generation = self.allocate_snapshot_generation();
                self.pane_snapshots.insert(
                    pane_id,
                    Arc::new(remote_full_state_to_vt_snapshot(
                        &state,
                        revision_seed,
                        self.terminal_foreground,
                        self.terminal_background,
                        self.cursor_color,
                    )),
                );
                self.bootstrapped = true;
                self.last_error = None;
                self.observe_focused_cursor_position();
            }
            LocalStreamEvent::UiPaneDelta {
                pane_id,
                runtime_revision,
                delta,
            } => {
                if runtime_revision != 0 && runtime_revision != self.runtime_revision {
                    return;
                }
                if let Some(snapshot) = self.pane_snapshots.get_mut(&pane_id) {
                    apply_remote_delta_snapshot(Arc::make_mut(snapshot), &delta);
                }
                self.observe_focused_cursor_position();
            }
            LocalStreamEvent::TabExited => {}
            LocalStreamEvent::Disconnected => {
                self.stream_tx = None;
                self.mode = ClientMode::Recovering;
                self.pending_input_latencies.clear();
                self.should_exit = false;
                self.last_error = Some("boo server stream disconnected".to_string());
            }
            LocalStreamEvent::FullState {
                ack_input_seq,
                state,
            } => {
                let _scope = crate::profiling::scope(
                    "client.stream.apply_full",
                    crate::profiling::Kind::Cpu,
                );
                self.mode = ClientMode::Active;
                let revision_seed = self.allocate_full_snapshot_revision_seed(state.rows as usize);
                self.terminal_snapshot_generation = self.allocate_snapshot_generation();
                if self.focused_pane_id != 0 {
                    self.pane_snapshots.insert(
                        self.focused_pane_id,
                        Arc::new(remote_full_state_to_vt_snapshot(
                            &state,
                            revision_seed,
                            self.terminal_foreground,
                            self.terminal_background,
                            self.cursor_color,
                        )),
                    );
                }
                self.bootstrapped = true;
                self.last_error = None;
                self.observe_focused_cursor_position();
                self.acknowledge_input_latency("stream_full_state", ack_input_seq);
            }
            LocalStreamEvent::Delta {
                ack_input_seq,
                delta,
            } => {
                let _scope = crate::profiling::scope(
                    "client.stream.apply_delta",
                    crate::profiling::Kind::Cpu,
                );
                if let Some(snapshot) = self.pane_snapshots.get_mut(&self.focused_pane_id) {
                    apply_remote_delta_snapshot(Arc::make_mut(snapshot), &delta);
                }
                self.observe_focused_cursor_position();
                self.acknowledge_input_latency("stream_delta", ack_input_seq);
            }
            LocalStreamEvent::Error(error) => {
                self.last_error = Some(error);
            }
        }
    }

    fn observe_focused_cursor_position(&mut self) {
        let current = focused_cursor_position(self.focused_pane_id, &self.pane_snapshots);
        if current.is_some() && self.focused_cursor_position != current {
            self.cursor_blink_epoch = Instant::now();
        }
        self.focused_cursor_position = current;
    }

    fn focused_cursor_rect(&self) -> Option<Rectangle> {
        let snapshot = self.pane_snapshots.get(&self.focused_pane_id)?;
        let pane = self
            .visible_panes
            .iter()
            .find(|pane| pane.pane_id == self.focused_pane_id)?;
        let x = pane.frame.x + snapshot.cursor.x as f64 * self.cell_width;
        let y = pane.frame.y + snapshot.cursor.y as f64 * self.cell_height;
        Some(Rectangle::new(
            Point::new(x as f32, y as f32),
            Size::new(self.cell_width as f32, self.cell_height as f32),
        ))
    }

    fn handle_stream_delivery(&mut self, event: LocalStreamEvent) -> Option<Task<Message>> {
        let track_stream = matches!(
            &event,
            LocalStreamEvent::TabList(_)
                | LocalStreamEvent::UiRuntimeState(_)
                | LocalStreamEvent::UiAppearance(_)
                | LocalStreamEvent::FullState { .. }
                | LocalStreamEvent::Delta { .. }
                | LocalStreamEvent::UiPaneFullState { .. }
                | LocalStreamEvent::UiPaneDelta { .. }
                | LocalStreamEvent::TabExited
        ) || matches!(
            &event,
            LocalStreamEvent::UiPaneFullState { pane_id, .. }
                | LocalStreamEvent::UiPaneDelta { pane_id, .. }
                if *pane_id == self.focused_pane_id
        );
        self.handle_stream_event(event);
        if track_stream {
            note_gui_test_stream_activity();
            return Some(Task::done(Message::Frame));
        }
        None
    }

    fn handle_gui_test(&mut self, command: GuiTestCommand) {
        match command {
            GuiTestCommand::Text(text) => {
                if self.stream_ready_for_terminal_io() {
                    let input_seq = self.record_pending_input();
                    self.send_stream_command(StreamCommand::Input {
                        input_seq,
                        bytes: text.into_bytes(),
                    });
                } else {
                    let _ = self.client.send(&control::Request::SendText { text });
                    self.refresh_snapshot(SnapshotRefreshReason::GuiTestText);
                }
            }
            GuiTestCommand::Key(keyspec) => {
                if self.stream_ready_for_terminal_io() {
                    let input_seq = self.record_pending_input();
                    self.send_stream_command(StreamCommand::Key { input_seq, keyspec });
                } else {
                    let _ = self
                        .client
                        .send(&control::Request::SendKey { key: keyspec });
                    self.refresh_snapshot(SnapshotRefreshReason::GuiTestKey);
                }
            }
            GuiTestCommand::AppKey { keyspec, repeat } => {
                if let Some(event) =
                    gui_test_app_key_event(&keyspec, self.record_pending_input(), repeat)
                {
                    self.send_stream_or_control(
                        StreamCommand::AppKeyEvent {
                            event: event.clone(),
                        },
                        control::Request::AppKeyEvent { event },
                    );
                }
            }
            GuiTestCommand::Keyboard { keyspec, repeat } => {
                if let Some(event) = gui_test_keyboard_event(&keyspec, repeat) {
                    self.handle_keyboard(event);
                }
            }
            GuiTestCommand::Command(input) => self.send_stream_or_control(
                StreamCommand::ExecuteCommand {
                    input: input.clone(),
                },
                control::Request::ExecuteCommand { input },
            ),
            GuiTestCommand::ActivateTab(tab_id) => self.activate_tab(tab_id),
            GuiTestCommand::Click { x, y } => {
                self.last_mouse_pos = Point::new(x as f32, y as f32);
                self.send_mouse_event(AppMouseEvent::ButtonPressed {
                    button: AppMouseButton::Left,
                    x,
                    y,
                    mods: 0,
                });
                self.send_mouse_event(AppMouseEvent::ButtonReleased {
                    button: AppMouseButton::Left,
                    x,
                    y,
                    mods: 0,
                });
            }
            GuiTestCommand::Drag { x1, y1, x2, y2 } => {
                self.last_mouse_pos = Point::new(x1 as f32, y1 as f32);
                self.send_mouse_event(AppMouseEvent::ButtonPressed {
                    button: AppMouseButton::Left,
                    x: x1,
                    y: y1,
                    mods: 0,
                });
                self.last_mouse_pos = Point::new(x2 as f32, y2 as f32);
                self.send_mouse_event(AppMouseEvent::CursorMoved {
                    x: x2,
                    y: y2,
                    mods: 0,
                });
                self.send_mouse_event(AppMouseEvent::ButtonReleased {
                    button: AppMouseButton::Left,
                    x: x2,
                    y: y2,
                    mods: 0,
                });
            }
            GuiTestCommand::Resize { cols, rows } => self.send_resize_cells(cols, rows),
            GuiTestCommand::Refresh => self.refresh_snapshot(SnapshotRefreshReason::GuiTestManual),
        }
    }

    fn send_stream_command(&self, command: StreamCommand) {
        if let Some(tx) = self.stream_tx.as_ref() {
            let _ = tx.send(command);
        }
    }

    fn send_runtime_action(&self, action: remote::RuntimeAction) {
        self.send_stream_command(StreamCommand::RuntimeAction { action });
    }

    fn send_resize_cells(&mut self, cols: u16, rows: u16) {
        self.send_resize_viewport_cells(cols, rows);
    }

    fn send_resize_viewport_cells(&mut self, cols: u16, rows: u16) {
        let _ = self
            .client
            .send(&control::Request::ResizeViewport { cols, rows });
    }

    fn record_pending_input(&mut self) -> u64 {
        let input_seq = self.next_input_seq;
        self.next_input_seq = self.next_input_seq.wrapping_add(1);
        self.pending_input_latencies
            .insert(input_seq, Instant::now());
        input_seq
    }

    fn allocate_full_snapshot_revision_seed(&mut self, row_count: usize) -> u64 {
        let seed = self.next_full_snapshot_revision;
        self.next_full_snapshot_revision = self
            .next_full_snapshot_revision
            .wrapping_add(row_count.max(1) as u64);
        seed
    }

    fn allocate_snapshot_generation(&mut self) -> u64 {
        let generation = self.next_snapshot_generation;
        self.next_snapshot_generation = self.next_snapshot_generation.wrapping_add(1);
        generation
    }

    fn acknowledge_input_latency(&mut self, stage: &str, ack_input_seq: Option<u64>) {
        let Some(ack_input_seq) = ack_input_seq else {
            return;
        };
        for (input_seq, started_at) in
            take_acknowledged_input_latencies(&mut self.pending_input_latencies, ack_input_seq)
        {
            log_client_latency(stage, input_seq, started_at);
        }
    }

    fn apply_remote_tabs(&mut self, tabs: &[remote::RemoteTabInfo]) {
        self.ui_state = ClientUiState::from_remote_tabs(tabs, self.active_remote_tab_id);
    }

    fn update_gui_test_status(&self) {
        let focused_snapshot = self.pane_snapshots.get(&self.focused_pane_id);
        let status_value = GuiTestStatus {
            mode: match self.mode {
                ClientMode::Bootstrapping => "bootstrapping",
                ClientMode::Active => "active",
                ClientMode::Recovering => "recovering",
            },
            stream_ready: self.stream_ready_for_terminal_io(),
            has_terminal: self.has_paintable_terminal(),
            active_tab: self.ui_state.active_tab,
            stream_seq: gui_test_stream_seq().load(std::sync::atomic::Ordering::Relaxed),
            render_seq: gui_test_render_seq().load(std::sync::atomic::Ordering::Relaxed),
            keyboard_seq: gui_test_keyboard_seq().load(std::sync::atomic::Ordering::Relaxed),
            input_method_commit_seq: gui_test_input_method_commit_seq()
                .load(std::sync::atomic::Ordering::Relaxed),
            last_stream_ms: gui_test_last_stream_ms().load(std::sync::atomic::Ordering::Relaxed),
            last_render_ms: gui_test_last_render_ms().load(std::sync::atomic::Ordering::Relaxed),
            cursor_row: focused_snapshot.map(|snapshot| snapshot.cursor.y),
            cursor_col: focused_snapshot.map(|snapshot| snapshot.cursor.x),
            cursor_row_text: focused_snapshot
                .and_then(|snapshot| snapshot.rows_data.get(snapshot.cursor.y as usize))
                .map(|row| render_snapshot_row_text(row))
                .unwrap_or_default(),
            row0_text: focused_snapshot
                .and_then(|snapshot| snapshot.rows_data.first())
                .map(|row| render_snapshot_row_text(row))
                .unwrap_or_default(),
        };
        if let Some(status) = gui_test_status_handle()
            && let Ok(mut guard) = status.lock()
        {
            *guard = status_value.clone();
        }
        if let Some(path) = gui_test_status_path() {
            let line = format!(
                "mode={} stream_ready={} has_terminal={} active_tab={} stream_seq={} render_seq={} keyboard_seq={} input_method_commit_seq={} last_stream_ms={} last_render_ms={} cursor_row={} cursor_col={} cursor_row_text={:?} row0={:?}\n",
                status_value.mode,
                u8::from(status_value.stream_ready),
                u8::from(status_value.has_terminal),
                status_value.active_tab,
                status_value.stream_seq,
                status_value.render_seq,
                status_value.keyboard_seq,
                status_value.input_method_commit_seq,
                status_value.last_stream_ms,
                status_value.last_render_ms,
                status_value
                    .cursor_row
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "-".to_string()),
                status_value
                    .cursor_col
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "-".to_string()),
                status_value.cursor_row_text,
                status_value.row0_text
            );
            if let Ok(mut last_written) = gui_test_status_last_written().lock()
                && last_written.as_deref() != Some(line.as_str())
            {
                let _ = std::fs::write(path, &line);
                *last_written = Some(line);
            }
        }
    }
}

fn gui_test_status_handle() -> Option<&'static Arc<Mutex<GuiTestStatus>>> {
    static STATUS: OnceLock<Option<Arc<Mutex<GuiTestStatus>>>> = OnceLock::new();
    STATUS
        .get_or_init(|| {
            std::env::var_os("BOO_GUI_TEST_SOCKET")
                .map(|_| Arc::new(Mutex::new(GuiTestStatus::default())))
        })
        .as_ref()
}

fn gui_test_status_path() -> Option<&'static str> {
    static PATH: OnceLock<Option<String>> = OnceLock::new();
    PATH.get_or_init(|| {
        std::env::var_os("BOO_GUI_TEST_STATUS_PATH").and_then(|path| path.into_string().ok())
    })
    .as_deref()
}

fn gui_test_status_last_written() -> &'static Mutex<Option<String>> {
    static LAST: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    LAST.get_or_init(|| Mutex::new(None))
}

fn render_snapshot_row_text(row: &[vt_backend_core::CellSnapshot]) -> String {
    row.iter()
        .map(|cell| cell.text.as_str())
        .collect::<String>()
}

fn gui_test_stream_seq() -> &'static std::sync::atomic::AtomicU64 {
    static VALUE: OnceLock<std::sync::atomic::AtomicU64> = OnceLock::new();
    VALUE.get_or_init(|| std::sync::atomic::AtomicU64::new(0))
}

fn gui_test_render_seq() -> &'static std::sync::atomic::AtomicU64 {
    static VALUE: OnceLock<std::sync::atomic::AtomicU64> = OnceLock::new();
    VALUE.get_or_init(|| std::sync::atomic::AtomicU64::new(0))
}

fn gui_test_keyboard_seq() -> &'static std::sync::atomic::AtomicU64 {
    static VALUE: OnceLock<std::sync::atomic::AtomicU64> = OnceLock::new();
    VALUE.get_or_init(|| std::sync::atomic::AtomicU64::new(0))
}

fn gui_test_input_method_commit_seq() -> &'static std::sync::atomic::AtomicU64 {
    static VALUE: OnceLock<std::sync::atomic::AtomicU64> = OnceLock::new();
    VALUE.get_or_init(|| std::sync::atomic::AtomicU64::new(0))
}

fn gui_test_last_stream_ms() -> &'static std::sync::atomic::AtomicU64 {
    static VALUE: OnceLock<std::sync::atomic::AtomicU64> = OnceLock::new();
    VALUE.get_or_init(|| std::sync::atomic::AtomicU64::new(0))
}

fn gui_test_last_render_ms() -> &'static std::sync::atomic::AtomicU64 {
    static VALUE: OnceLock<std::sync::atomic::AtomicU64> = OnceLock::new();
    VALUE.get_or_init(|| std::sync::atomic::AtomicU64::new(0))
}

fn gui_test_now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn note_gui_test_stream_activity() {
    gui_test_stream_seq().fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    gui_test_last_stream_ms().store(gui_test_now_ms(), std::sync::atomic::Ordering::Relaxed);
}

fn note_gui_test_render_activity() {
    gui_test_render_seq().fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    gui_test_last_render_ms().store(gui_test_now_ms(), std::sync::atomic::Ordering::Relaxed);
}

fn note_gui_test_keyboard_event() {
    gui_test_keyboard_seq().fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

fn note_gui_test_input_method_commit() {
    gui_test_input_method_commit_seq().fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

impl ClientUiState {
    fn from_snapshot(snapshot: &control::UiSnapshot) -> Self {
        Self {
            tabs: snapshot
                .tabs
                .iter()
                .map(|tab| ClientTabState {
                    index: tab.index,
                    tab_id: None,
                    active: tab.active,
                    title: tab.title.clone(),
                    pane_count: tab.pane_count,
                })
                .collect(),
            active_tab: snapshot.active_tab,
            pwd: snapshot.pwd.clone(),
            pane_count: snapshot.visible_panes.len(),
            status_bar: snapshot.status_bar.clone(),
        }
    }

    fn from_remote_tabs(tabs: &[remote::RemoteTabInfo], active_remote_tab_id: Option<u32>) -> Self {
        let remote_tabs = tabs;
        let active_index = active_remote_tab_id
            .and_then(|tab_id| remote_tabs.iter().position(|tab| tab.id == tab_id))
            .unwrap_or(0);
        let tabs = remote_tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| ClientTabState {
                index,
                tab_id: Some(tab.id),
                active: index == active_index,
                title: if tab.title.is_empty() {
                    tab.name.clone()
                } else {
                    tab.title.clone()
                },
                pane_count: 1,
            })
            .collect::<Vec<_>>();
        let pwd = remote_tabs
            .get(active_index)
            .map(|tab| tab.pwd.clone())
            .unwrap_or_default();
        let pane_count = usize::from(!tabs.is_empty());
        let active_tab = active_index.min(tabs.len().saturating_sub(1));
        Self {
            tabs,
            active_tab,
            pwd,
            pane_count,
            status_bar: crate::status_components::UiStatusBarSnapshot::default(),
        }
    }

    fn from_runtime_state(state: &control::UiRuntimeState) -> Self {
        Self {
            tabs: state
                .tabs
                .iter()
                .map(|tab| ClientTabState {
                    index: tab.index,
                    tab_id: Some(tab.tab_id),
                    active: tab.active,
                    title: tab.title.clone(),
                    pane_count: tab.pane_count,
                })
                .collect(),
            active_tab: state.active_tab,
            pwd: state.pwd.clone(),
            pane_count: state.visible_panes.len(),
            status_bar: state.status_bar.clone(),
        }
    }
}

fn latency_debug_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("BOO_LATENCY_DEBUG").is_some())
}

fn log_client_latency(stage: &str, input_seq: u64, started_at: Instant) {
    crate::profiling::record(
        match stage {
            "stream_full_state" => "client.latency.stream_full_state",
            "stream_delta" => "client.latency.stream_delta",
            _ => "client.latency.other",
        },
        crate::profiling::Kind::Cpu,
        started_at.elapsed(),
    );
    if !latency_debug_enabled() {
        return;
    }
    log::info!(
        "boo_latency stage={stage} seq={input_seq} ms={:.3}",
        started_at.elapsed().as_secs_f64() * 1000.0
    );
}

fn take_acknowledged_input_latencies(
    pending: &mut BTreeMap<u64, Instant>,
    ack_input_seq: u64,
) -> Vec<(u64, Instant)> {
    let remaining = pending.split_off(&ack_input_seq.wrapping_add(1));
    let acknowledged = std::mem::replace(pending, remaining);
    acknowledged.into_iter().collect()
}

fn remote_full_state_to_vt_snapshot(
    state: &remote::RemoteFullState,
    revision_seed: u64,
    terminal_foreground: crate::config::RgbColor,
    terminal_background: crate::config::RgbColor,
    cursor_color: crate::config::RgbColor,
) -> vt_backend_core::TerminalSnapshot {
    let cols = state.cols as usize;
    let rows_data = state
        .cells
        .chunks(cols.max(1))
        .map(|row| {
            row.iter()
                .map(|cell| remote_cell_to_snapshot(cell, terminal_foreground, terminal_background))
                .collect()
        })
        .collect();
    let row_revisions = (0..state.rows as usize)
        .map(|index| revision_seed.wrapping_add(index as u64))
        .collect();
    vt_backend_core::TerminalSnapshot {
        cols: state.cols,
        rows: state.rows,
        title: String::new(),
        pwd: String::new(),
        cursor: vt_backend_core::CursorSnapshot {
            visible: state.cursor_visible,
            blinking: state.cursor_blinking,
            x: state.cursor_x,
            y: state.cursor_y,
            style: vt::CursorStyle::from(state.cursor_style),
        },
        rows_data,
        row_revisions,
        scrollbar: Default::default(),
        colors: vt::RenderColors {
            foreground: vt::RgbColor::from_array(terminal_foreground),
            background: vt::RgbColor::from_array(terminal_background),
            cursor: vt::RgbColor::from_array(cursor_color),
            cursor_has_value: true,
            ..Default::default()
        },
    }
}

fn ui_terminal_to_vt_snapshot(
    snapshot: &control::UiTerminalSnapshot,
    terminal_foreground: crate::config::RgbColor,
    terminal_background: crate::config::RgbColor,
    cursor_color: crate::config::RgbColor,
) -> vt_backend_core::TerminalSnapshot {
    vt_backend_core::TerminalSnapshot {
        cols: snapshot.cols,
        rows: snapshot.rows,
        title: snapshot.title.clone(),
        pwd: snapshot.pwd.clone(),
        cursor: vt_backend_core::CursorSnapshot {
            visible: snapshot.cursor.visible,
            blinking: snapshot.cursor.blinking,
            x: snapshot.cursor.x,
            y: snapshot.cursor.y,
            style: vt::CursorStyle::from(snapshot.cursor.style),
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
                        fg: vt::RgbColor::from_array(cell.fg),
                        bg: vt::RgbColor::from_array(cell.bg),
                        bg_is_default: cell.bg_is_default,
                        bold: cell.bold,
                        italic: cell.italic,
                        underline: cell.underline,
                        hyperlink: cell.hyperlink,
                    })
                    .collect()
            })
            .collect(),
        row_revisions: vec![1; snapshot.rows_data.len()],
        scrollbar: Default::default(),
        colors: vt::RenderColors {
            foreground: vt::RgbColor::from_array(terminal_foreground),
            background: vt::RgbColor::from_array(terminal_background),
            cursor: vt::RgbColor::from_array(cursor_color),
            cursor_has_value: true,
            ..Default::default()
        },
    }
}

fn pane_snapshot_map_from_ui_snapshot(
    snapshot: &control::UiSnapshot,
) -> HashMap<u64, Arc<vt_backend_core::TerminalSnapshot>> {
    snapshot
        .pane_terminals
        .iter()
        .map(|pane| {
            (
                pane.pane_id,
                Arc::new(ui_terminal_to_vt_snapshot(
                    &pane.terminal,
                    snapshot.appearance.terminal_foreground,
                    snapshot.appearance.terminal_background,
                    snapshot.appearance.cursor_color,
                )),
            )
        })
        .collect()
}

#[derive(Debug, Clone)]
struct PaneBordersOverlay {
    panes: Vec<control::UiPaneSnapshot>,
}

impl<Message> iced::widget::canvas::Program<Message> for PaneBordersOverlay {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &iced::Renderer,
        _theme: &Theme,
        bounds: iced::Rectangle,
        _cursor: iced::mouse::Cursor,
    ) -> Vec<iced::widget::canvas::Geometry<iced::Renderer>> {
        let cache = iced::widget::canvas::Cache::new();
        let geometry = cache.draw(renderer, bounds.size(), |frame| {
            let stroke = iced::widget::canvas::Stroke::default()
                .with_color(Color::from_rgba(0.72, 0.72, 0.72, 0.35))
                .with_width(1.0);
            for divider in pane_dividers(&self.panes) {
                let path = iced::widget::canvas::Path::line(divider.start, divider.end);
                frame.stroke(&path, stroke);
            }
        });
        vec![geometry]
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct PaneDivider {
    start: iced::Point,
    end: iced::Point,
}

fn pane_dividers(panes: &[control::UiPaneSnapshot]) -> Vec<PaneDivider> {
    let mut dividers = Vec::new();
    for (index, first) in panes.iter().enumerate() {
        let first_rect = pane_rect(first);
        for second in panes.iter().skip(index + 1) {
            let second_rect = pane_rect(second);
            if let Some(divider) = vertical_divider(first_rect, second_rect)
                .or_else(|| vertical_divider(second_rect, first_rect))
                .or_else(|| horizontal_divider(first_rect, second_rect))
                .or_else(|| horizontal_divider(second_rect, first_rect))
                && !dividers
                    .iter()
                    .any(|existing| same_divider(*existing, divider))
            {
                dividers.push(divider);
            }
        }
    }
    dividers
}

fn pane_rect(pane: &control::UiPaneSnapshot) -> iced::Rectangle {
    iced::Rectangle {
        x: pane.frame.x as f32,
        y: pane.frame.y as f32,
        width: pane.frame.width as f32,
        height: pane.frame.height as f32,
    }
}

fn vertical_divider(left: iced::Rectangle, right: iced::Rectangle) -> Option<PaneDivider> {
    let left_edge = left.x + left.width;
    let gap = right.x - left_edge;
    if !(0.0..=2.0).contains(&gap) {
        return None;
    }
    let top = left.y.max(right.y);
    let bottom = (left.y + left.height).min(right.y + right.height);
    if bottom - top <= 1.0 {
        return None;
    }
    let x = left_edge + gap * 0.5;
    Some(PaneDivider {
        start: iced::Point::new(x, top),
        end: iced::Point::new(x, bottom),
    })
}

fn horizontal_divider(top: iced::Rectangle, bottom: iced::Rectangle) -> Option<PaneDivider> {
    let top_edge = top.y + top.height;
    let gap = bottom.y - top_edge;
    if !(0.0..=2.0).contains(&gap) {
        return None;
    }
    let left = top.x.max(bottom.x);
    let right = (top.x + top.width).min(bottom.x + bottom.width);
    if right - left <= 1.0 {
        return None;
    }
    let y = top_edge + gap * 0.5;
    Some(PaneDivider {
        start: iced::Point::new(left, y),
        end: iced::Point::new(right, y),
    })
}

fn same_divider(a: PaneDivider, b: PaneDivider) -> bool {
    (a.start.x - b.start.x).abs() < 0.5
        && (a.start.y - b.start.y).abs() < 0.5
        && (a.end.x - b.end.x).abs() < 0.5
        && (a.end.y - b.end.y).abs() < 0.5
}

fn focused_cursor_position(
    focused_pane_id: u64,
    pane_snapshots: &HashMap<u64, Arc<vt_backend_core::TerminalSnapshot>>,
) -> Option<(u64, u16, u16)> {
    let cursor = &pane_snapshots.get(&focused_pane_id)?.cursor;
    Some((focused_pane_id, cursor.x, cursor.y))
}

fn pane_cursor_blink_visible(
    pane_focused: bool,
    cursor_blinking: bool,
    app_focused: bool,
    epoch: Instant,
    interval: Duration,
) -> bool {
    !pane_focused
        || !cursor_blinking
        || interval.is_zero()
        || !app_focused
        || cursor_blink_visible(epoch, interval)
}

fn iced_input_method_event_name(event: &input_method::Event) -> &'static str {
    match event {
        input_method::Event::Opened => "opened",
        input_method::Event::Preedit(_, _) => "preedit",
        input_method::Event::Commit(_) => "commit",
        input_method::Event::Closed => "closed",
    }
}

struct TerminalInputMethodLayer {
    cursor: Option<Rectangle>,
    text_size: f32,
    preedit: String,
    enabled: bool,
}

impl TerminalInputMethodLayer {
    fn element<'a>(
        cursor: Option<Rectangle>,
        text_size: f32,
        preedit: String,
        enabled: bool,
    ) -> Element<'a, Message> {
        Element::new(Self {
            cursor,
            text_size,
            preedit,
            enabled,
        })
    }
}

impl Widget<Message, Theme, iced::Renderer> for TerminalInputMethodLayer {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }

    fn layout(
        &mut self,
        _tree: &mut Tree,
        _renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        layout::atomic(limits, Length::Fill, Length::Fill)
    }

    fn draw(
        &self,
        _tree: &Tree,
        _renderer: &mut iced::Renderer,
        _theme: &Theme,
        _style: &renderer::Style,
        _layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
    }

    fn update(
        &mut self,
        _tree: &mut Tree,
        event: &Event,
        _layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _renderer: &iced::Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        if let Event::Window(window::Event::RedrawRequested(_)) = event
            && let Some(cursor) = self.cursor.filter(|_| self.enabled)
        {
            let preedit = (!self.preedit.is_empty()).then_some(input_method::Preedit {
                content: self.preedit.as_str(),
                selection: None,
                text_size: Some(iced::Pixels(self.text_size)),
            });
            shell.request_input_method(&InputMethod::Enabled {
                cursor,
                purpose: input_method::Purpose::Terminal,
                preedit,
            });
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum LocalStreamEvent {
    TabList(Vec<remote::RemoteTabInfo>),
    UiRuntimeState(control::UiRuntimeState),
    UiAppearance(control::UiAppearanceSnapshot),
    UiPaneFullState {
        pane_id: u64,
        runtime_revision: u64,
        state: remote::RemoteFullState,
    },
    UiPaneDelta {
        pane_id: u64,
        runtime_revision: u64,
        delta: RemoteDelta,
    },
    TabExited,
    Disconnected,
    FullState {
        ack_input_seq: Option<u64>,
        state: remote::RemoteFullState,
    },
    Delta {
        ack_input_seq: Option<u64>,
        delta: RemoteDelta,
    },
    Error(String),
}

#[derive(Clone, Debug)]
pub(crate) struct RemoteDelta {
    cursor_x: u16,
    cursor_y: u16,
    cursor_visible: bool,
    cursor_blinking: bool,
    cursor_style: i32,
    scroll_rows: i16,
    changed_rows: Vec<RemoteRowDelta>,
}

#[cfg(test)]
fn is_passive_screen_event(event: &LocalStreamEvent) -> bool {
    matches!(
        event,
        LocalStreamEvent::FullState {
            ack_input_seq: None,
            ..
        } | LocalStreamEvent::Delta {
            ack_input_seq: None,
            ..
        }
    )
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CoalescedStreamKind {
    TabList,
    UiAppearance,
    UiRuntimeState,
    PassiveScreen,
}

#[cfg(test)]
#[derive(Default)]
struct PendingCoalescedStreamEvents {
    coalesce_passive_screen: bool,
    order: Vec<CoalescedStreamKind>,
    tab_list: Option<LocalStreamEvent>,
    ui_appearance: Option<LocalStreamEvent>,
    ui_runtime_state: Option<LocalStreamEvent>,
    passive_screen: Option<LocalStreamEvent>,
}

#[cfg(test)]
impl PendingCoalescedStreamEvents {
    fn with_passive_screen_coalescing(coalesce_passive_screen: bool) -> Self {
        Self {
            coalesce_passive_screen,
            ..Default::default()
        }
    }

    fn push_kind_once(&mut self, kind: CoalescedStreamKind) {
        if !self.order.contains(&kind) {
            self.order.push(kind);
        }
    }

    fn set_tab_list(&mut self, event: LocalStreamEvent) {
        self.push_kind_once(CoalescedStreamKind::TabList);
        self.tab_list = Some(event);
    }

    fn set_ui_appearance(&mut self, event: LocalStreamEvent) {
        self.push_kind_once(CoalescedStreamKind::UiAppearance);
        self.ui_appearance = Some(event);
    }

    fn set_ui_runtime_state(&mut self, event: LocalStreamEvent) {
        self.push_kind_once(CoalescedStreamKind::UiRuntimeState);
        self.ui_runtime_state = Some(event);
    }

    fn set_passive_screen(&mut self, event: LocalStreamEvent) {
        self.push_kind_once(CoalescedStreamKind::PassiveScreen);
        self.passive_screen = Some(event);
    }

    fn take_in_order(&mut self) -> Vec<LocalStreamEvent> {
        let mut flushed = Vec::with_capacity(self.order.len());
        for kind in self.order.drain(..) {
            let event = match kind {
                CoalescedStreamKind::TabList => self.tab_list.take(),
                CoalescedStreamKind::UiAppearance => self.ui_appearance.take(),
                CoalescedStreamKind::UiRuntimeState => self.ui_runtime_state.take(),
                CoalescedStreamKind::PassiveScreen => self.passive_screen.take(),
            };
            if let Some(event) = event {
                flushed.push(event);
            }
        }
        flushed
    }
}

#[cfg(test)]
fn push_coalesced_stream_event(
    batch: &mut Vec<LocalStreamEvent>,
    pending: &mut PendingCoalescedStreamEvents,
    event: LocalStreamEvent,
) {
    match &event {
        LocalStreamEvent::TabList(_) => {
            pending.set_tab_list(event);
            return;
        }
        LocalStreamEvent::UiAppearance(_) => {
            pending.set_ui_appearance(event);
            return;
        }
        LocalStreamEvent::UiRuntimeState(_) => {
            pending.set_ui_runtime_state(event);
            return;
        }
        _ => {}
    }
    if pending.coalesce_passive_screen && is_passive_screen_event(&event) {
        match pending.passive_screen.take() {
            Some(previous) if passive_screen_event_supersedes(&previous, &event) => {
                pending.set_passive_screen(event);
            }
            Some(previous) => {
                batch.extend(pending.take_in_order());
                batch.push(previous);
                pending.set_passive_screen(event);
            }
            None => pending.set_passive_screen(event),
        }
        return;
    }
    batch.extend(pending.take_in_order());
    batch.push(event);
}

#[cfg(test)]
fn flush_pending_passive_stream_event(
    batch: &mut Vec<LocalStreamEvent>,
    pending: &mut PendingCoalescedStreamEvents,
) {
    batch.extend(pending.take_in_order());
}

#[cfg(test)]
fn passive_screen_event_supersedes(previous: &LocalStreamEvent, next: &LocalStreamEvent) -> bool {
    is_passive_screen_event(previous)
        && matches!(
            next,
            LocalStreamEvent::FullState {
                ack_input_seq: None,
                ..
            }
        )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RemoteRowDelta {
    row: u16,
    start_col: u16,
    cells: Vec<remote::RemoteCell>,
}

#[derive(Clone, Debug)]
pub(crate) enum StreamCommand {
    RuntimeAction { action: remote::RuntimeAction },
    AppKeyEvent { event: AppKeyEvent },
    AppMouseEvent { event: AppMouseEvent },
    ExecuteCommand { input: String },
    Input { input_seq: u64, bytes: Vec<u8> },
    Key { input_seq: u64, keyspec: String },
}

fn write_stream_message(
    write: &mut UnixStream,
    ty: remote::MessageType,
    payload: &[u8],
) -> std::io::Result<()> {
    let frame = remote::encode_message(ty, payload);
    let mut scope = crate::profiling::scope("client.stream.write", crate::profiling::Kind::Io);
    scope.add_bytes(frame.len() as u64);
    write.write_all(&frame)?;
    write.flush()
}

fn local_stream_subscription(
    socket_path: &String,
) -> iced::futures::stream::BoxStream<'static, Message> {
    let socket_path = format!("{socket_path}.stream");
    Box::pin(stream::channel(
        100,
        move |mut output: iced::futures::channel::mpsc::Sender<Message>| async move {
            loop {
                let Ok(write) = UnixStream::connect(&socket_path) else {
                    std::thread::sleep(STREAM_RECONNECT_DELAY);
                    continue;
                };
                let Ok(read) = write.try_clone() else {
                    std::thread::sleep(STREAM_RECONNECT_DELAY);
                    continue;
                };
                let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<StreamCommand>();
                let _ = output.send(Message::StreamReady(cmd_tx)).await;

                let (event_tx, mut event_rx) =
                    iced::futures::channel::mpsc::unbounded::<LocalStreamEvent>();

                let writer_event_tx = event_tx.clone();
                std::thread::spawn(move || {
                    let mut write = write;
                    while let Ok(command) = cmd_rx.recv() {
                        let result = match command {
                            StreamCommand::RuntimeAction { action } => {
                                let Ok(payload) = serde_json::to_vec(&action) else {
                                    let _ = writer_event_tx
                                        .unbounded_send(LocalStreamEvent::Disconnected);
                                    break;
                                };
                                write_stream_message(
                                    &mut write,
                                    remote::MessageType::RuntimeAction,
                                    &payload,
                                )
                            }
                            StreamCommand::AppKeyEvent { event } => {
                                let Ok(payload) = serde_json::to_vec(&event) else {
                                    let _ = writer_event_tx
                                        .unbounded_send(LocalStreamEvent::Disconnected);
                                    break;
                                };
                                write_stream_message(
                                    &mut write,
                                    remote::MessageType::AppKeyEvent,
                                    &payload,
                                )
                            }
                            StreamCommand::AppMouseEvent { event } => {
                                let Ok(payload) = serde_json::to_vec(&event) else {
                                    let _ = writer_event_tx
                                        .unbounded_send(LocalStreamEvent::Disconnected);
                                    break;
                                };
                                write_stream_message(
                                    &mut write,
                                    remote::MessageType::AppMouseEvent,
                                    &payload,
                                )
                            }
                            StreamCommand::ExecuteCommand { input } => write_stream_message(
                                &mut write,
                                remote::MessageType::ExecuteCommand,
                                input.as_bytes(),
                            ),
                            StreamCommand::Input { input_seq, bytes } => {
                                let mut payload = Vec::with_capacity(8 + bytes.len());
                                payload.extend_from_slice(&input_seq.to_le_bytes());
                                payload.extend_from_slice(&bytes);
                                write_stream_message(
                                    &mut write,
                                    remote::MessageType::Input,
                                    &payload,
                                )
                            }
                            StreamCommand::Key { input_seq, keyspec } => {
                                let mut payload = Vec::with_capacity(8 + keyspec.len());
                                payload.extend_from_slice(&input_seq.to_le_bytes());
                                payload.extend_from_slice(keyspec.as_bytes());
                                write_stream_message(&mut write, remote::MessageType::Key, &payload)
                            }
                        };
                        if result.is_err() {
                            let _ = writer_event_tx.unbounded_send(LocalStreamEvent::Disconnected);
                            break;
                        }
                    }
                });

                std::thread::spawn(move || {
                    read_local_stream_loop(read, move |event| {
                        let _ = event_tx.unbounded_send(event);
                    })
                });

                while let Some(event) = event_rx.next().await {
                    let saw_disconnect = matches!(event, LocalStreamEvent::Disconnected);
                    let _ = output.send(Message::StreamEvent(event)).await;
                    if saw_disconnect {
                        break;
                    }
                }
                std::thread::sleep(STREAM_RECONNECT_DELAY);
            }
        },
    ))
}

fn gui_test_subscription() -> Subscription<Message> {
    let Some(socket_path) =
        std::env::var_os("BOO_GUI_TEST_SOCKET").and_then(|path| path.into_string().ok())
    else {
        return Subscription::none();
    };
    iced::Subscription::run_with(socket_path, |socket_path| {
        let socket_path = socket_path.clone();
        let status = gui_test_status_handle().cloned();
        Box::pin(stream::channel(
            100,
            move |mut output: iced::futures::channel::mpsc::Sender<Message>| async move {
                let (event_tx, mut event_rx) =
                    iced::futures::channel::mpsc::unbounded::<GuiTestCommand>();
                std::thread::spawn(move || {
                    let _ = std::fs::remove_file(&socket_path);
                    let Ok(listener) = UnixListener::bind(&socket_path) else {
                        return;
                    };
                    while let Ok((stream, _addr)) = listener.accept() {
                        let mut writer = stream;
                        let Ok(read_stream) = writer.try_clone() else {
                            continue;
                        };
                        let mut reader = BufReader::new(read_stream);
                        let mut line = String::new();
                        loop {
                            line.clear();
                            let Ok(bytes) = reader.read_line(&mut line) else {
                                break;
                            };
                            if bytes == 0 {
                                break;
                            }
                            let line = line.trim_end_matches(['\r', '\n']);
                            if line == "status" {
                                if let Some(status) = status.as_ref()
                                    && let Ok(guard) = status.lock()
                                {
                                    let _ = writeln!(
                                        writer,
                                        "mode={} stream_ready={} has_terminal={} active_tab={} row0={:?}",
                                        guard.mode,
                                        u8::from(guard.stream_ready),
                                        u8::from(guard.has_terminal),
                                        guard.active_tab,
                                        guard.row0_text
                                    );
                                }
                                continue;
                            }
                            if let Some(command) = parse_gui_test_command(line) {
                                let _ = event_tx.unbounded_send(command);
                            }
                        }
                    }
                    let _ = std::fs::remove_file(&socket_path);
                });

                while let Some(command) = event_rx.next().await {
                    if output.send(Message::GuiTest(command)).await.is_err() {
                        break;
                    }
                }
            },
        ))
    })
}

fn parse_gui_test_command(line: &str) -> Option<GuiTestCommand> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(rest) = trimmed.strip_prefix("text ") {
        return Some(GuiTestCommand::Text(decode_gui_test_text(rest)));
    }
    if let Some(rest) = trimmed.strip_prefix("key ") {
        return Some(GuiTestCommand::Key(rest.to_string()));
    }
    if let Some(rest) = trimmed.strip_prefix("appkey-repeat ") {
        return Some(GuiTestCommand::AppKey {
            keyspec: rest.to_string(),
            repeat: true,
        });
    }
    if let Some(rest) = trimmed.strip_prefix("appkey ") {
        return Some(GuiTestCommand::AppKey {
            keyspec: rest.to_string(),
            repeat: false,
        });
    }
    if let Some(rest) = trimmed.strip_prefix("keyboard-repeat ") {
        return Some(GuiTestCommand::Keyboard {
            keyspec: rest.to_string(),
            repeat: true,
        });
    }
    if let Some(rest) = trimmed.strip_prefix("keyboard ") {
        return Some(GuiTestCommand::Keyboard {
            keyspec: rest.to_string(),
            repeat: false,
        });
    }
    if let Some(rest) = trimmed.strip_prefix("command ") {
        return Some(GuiTestCommand::Command(rest.to_string()));
    }
    if let Some(rest) = trimmed.strip_prefix("activate-tab ") {
        let tab_id = rest.parse().ok()?;
        return Some(GuiTestCommand::ActivateTab(tab_id));
    }
    if let Some(rest) = trimmed.strip_prefix("click ") {
        let mut parts = rest.split_whitespace();
        let x = parts.next()?.parse().ok()?;
        let y = parts.next()?.parse().ok()?;
        return Some(GuiTestCommand::Click { x, y });
    }
    if let Some(rest) = trimmed.strip_prefix("drag ") {
        let mut parts = rest.split_whitespace();
        let x1 = parts.next()?.parse().ok()?;
        let y1 = parts.next()?.parse().ok()?;
        let x2 = parts.next()?.parse().ok()?;
        let y2 = parts.next()?.parse().ok()?;
        return Some(GuiTestCommand::Drag { x1, y1, x2, y2 });
    }
    if let Some(rest) = trimmed.strip_prefix("resize ") {
        let mut parts = rest.split_whitespace();
        let cols = parts.next()?.parse().ok()?;
        let rows = parts.next()?.parse().ok()?;
        return Some(GuiTestCommand::Resize { cols, rows });
    }
    if trimmed == "refresh" {
        return Some(GuiTestCommand::Refresh);
    }
    None
}

fn decode_gui_test_text(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('r') => out.push('\r'),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('\\') => out.push('\\'),
            Some('0') => out.push('\0'),
            Some('x') => {
                let hi = chars.next();
                let lo = chars.next();
                if let (Some(hi), Some(lo)) = (hi, lo)
                    && let Ok(value) = u8::from_str_radix(&format!("{hi}{lo}"), 16)
                {
                    out.push(value as char);
                } else {
                    out.push('\\');
                    out.push('x');
                    if let Some(hi) = hi {
                        out.push(hi);
                    }
                    if let Some(lo) = lo {
                        out.push(lo);
                    }
                }
            }
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

fn gui_test_app_key_event(spec: &str, input_seq: u64, repeat: bool) -> Option<AppKeyEvent> {
    let (keycode, mods) = crate::parse_keyspec(spec)?;
    let key_char = crate::shifted_char(keycode, mods);
    let text = if mods & (crate::ffi::GHOSTTY_MODS_CTRL | crate::ffi::GHOSTTY_MODS_ALT) == 0 {
        key_char.map(|ch| ch.to_string())
    } else {
        None
    };
    Some(AppKeyEvent {
        keycode,
        mods,
        text: text.clone(),
        modified_text: text,
        named_key: None,
        repeat,
        input_seq: Some(input_seq),
    })
}

fn gui_test_keyboard_event(spec: &str, repeat: bool) -> Option<keyboard::Event> {
    let mut chars = spec.chars();
    let ch = chars.next()?;
    if chars.next().is_some() || !ch.is_ascii_alphabetic() {
        return None;
    }
    let code = match ch.to_ascii_lowercase() {
        'a' => keyboard::key::Code::KeyA,
        'b' => keyboard::key::Code::KeyB,
        'c' => keyboard::key::Code::KeyC,
        'd' => keyboard::key::Code::KeyD,
        'e' => keyboard::key::Code::KeyE,
        'f' => keyboard::key::Code::KeyF,
        'g' => keyboard::key::Code::KeyG,
        'h' => keyboard::key::Code::KeyH,
        'i' => keyboard::key::Code::KeyI,
        'j' => keyboard::key::Code::KeyJ,
        'k' => keyboard::key::Code::KeyK,
        'l' => keyboard::key::Code::KeyL,
        'm' => keyboard::key::Code::KeyM,
        'n' => keyboard::key::Code::KeyN,
        'o' => keyboard::key::Code::KeyO,
        'p' => keyboard::key::Code::KeyP,
        'q' => keyboard::key::Code::KeyQ,
        'r' => keyboard::key::Code::KeyR,
        's' => keyboard::key::Code::KeyS,
        't' => keyboard::key::Code::KeyT,
        'u' => keyboard::key::Code::KeyU,
        'v' => keyboard::key::Code::KeyV,
        'w' => keyboard::key::Code::KeyW,
        'x' => keyboard::key::Code::KeyX,
        'y' => keyboard::key::Code::KeyY,
        'z' => keyboard::key::Code::KeyZ,
        _ => return None,
    };
    let text = ch.to_string();
    Some(keyboard::Event::KeyPressed {
        key: keyboard::Key::Character(text.clone().into()),
        modified_key: keyboard::Key::Character(text.clone().into()),
        physical_key: keyboard::key::Physical::Code(code),
        location: keyboard::Location::Standard,
        modifiers: keyboard::Modifiers::default(),
        text: Some(text.into()),
        repeat,
    })
}

fn app_mouse_button(button: mouse::Button) -> AppMouseButton {
    match button {
        mouse::Button::Left => AppMouseButton::Left,
        mouse::Button::Right => AppMouseButton::Right,
        mouse::Button::Middle => AppMouseButton::Middle,
        _ => AppMouseButton::Other,
    }
}

fn read_local_stream_loop(mut read: UnixStream, mut emit: impl FnMut(LocalStreamEvent)) {
    loop {
        let mut scope =
            crate::profiling::scope("client.stream.read_message", crate::profiling::Kind::Io);
        let Ok((ty, payload)) = remote::read_message(&mut read) else {
            break;
        };
        scope.add_bytes(payload.len() as u64);
        let event = match ty {
            remote::MESSAGE_TYPE_TAB_LIST => {
                decode_remote_tab_list(&payload).map(LocalStreamEvent::TabList)
            }
            remote::MessageType::UiRuntimeState => serde_json::from_slice(&payload)
                .ok()
                .map(LocalStreamEvent::UiRuntimeState),
            remote::MessageType::UiAppearance => serde_json::from_slice(&payload)
                .ok()
                .map(LocalStreamEvent::UiAppearance),
            remote::MessageType::UiPaneFullState => decode_remote_pane_full_state(&payload).map(
                |(pane_id, runtime_revision, state)| LocalStreamEvent::UiPaneFullState {
                    pane_id,
                    runtime_revision,
                    state,
                },
            ),
            remote::MessageType::UiPaneDelta => decode_remote_pane_delta(&payload).map(
                |(pane_id, runtime_revision, delta)| LocalStreamEvent::UiPaneDelta {
                    pane_id,
                    runtime_revision,
                    delta,
                },
            ),
            remote::MESSAGE_TYPE_TAB_EXITED => {
                decode_u32(&payload).map(|_| LocalStreamEvent::TabExited)
            }
            remote::MessageType::FullState => {
                decode_remote_full_state(&payload).map(|(ack_input_seq, state)| {
                    LocalStreamEvent::FullState {
                        ack_input_seq,
                        state,
                    }
                })
            }
            remote::MessageType::Delta => {
                decode_remote_delta(&payload).map(|(ack_input_seq, delta)| {
                    LocalStreamEvent::Delta {
                        ack_input_seq,
                        delta,
                    }
                })
            }
            remote::MessageType::ErrorMsg => Some(LocalStreamEvent::Error(
                String::from_utf8_lossy(&payload).to_string(),
            )),
            _ => None,
        };
        if let Some(event) = event {
            emit(event);
        }
    }
    emit(LocalStreamEvent::Disconnected);
}

fn decode_u32(payload: &[u8]) -> Option<u32> {
    (payload.len() >= 4)
        .then(|| u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]))
}

fn decode_remote_tab_list(payload: &[u8]) -> Option<Vec<remote::RemoteTabInfo>> {
    if payload.len() < 4 {
        return None;
    }
    let count = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]) as usize;
    let mut offset = 4usize;
    let mut tabs = Vec::with_capacity(count);
    for _ in 0..count {
        if offset + 4 > payload.len() {
            return None;
        }
        let id = u32::from_le_bytes([
            payload[offset],
            payload[offset + 1],
            payload[offset + 2],
            payload[offset + 3],
        ]);
        offset += 4;
        let name = decode_remote_string(payload, &mut offset)?;
        let title = decode_remote_string(payload, &mut offset)?;
        let pwd = decode_remote_string(payload, &mut offset)?;
        if offset >= payload.len() {
            return None;
        }
        let flags = payload[offset];
        offset += 1;
        tabs.push(remote::RemoteTabInfo {
            id,
            name,
            title,
            pwd,
            active: (flags & 0x01) != 0,
            child_exited: (flags & 0x02) != 0,
        });
    }
    Some(tabs)
}

fn decode_remote_string(payload: &[u8], offset: &mut usize) -> Option<String> {
    if *offset + 2 > payload.len() {
        return None;
    }
    let len = u16::from_le_bytes([payload[*offset], payload[*offset + 1]]) as usize;
    *offset += 2;
    if *offset + len > payload.len() {
        return None;
    }
    let value = String::from_utf8(payload[*offset..*offset + len].to_vec()).ok()?;
    *offset += len;
    Some(value)
}

fn decode_remote_full_state(payload: &[u8]) -> Option<(Option<u64>, remote::RemoteFullState)> {
    if payload.len() < LOCAL_FULL_STATE_HEADER_LEN {
        return None;
    }
    let ack_input_seq = u64::from_le_bytes(payload[..8].try_into().ok()?);
    let rows = u16::from_le_bytes([payload[8], payload[9]]);
    let cols = u16::from_le_bytes([payload[10], payload[11]]);
    let cursor_x = u16::from_le_bytes([payload[12], payload[13]]);
    let cursor_y = u16::from_le_bytes([payload[14], payload[15]]);
    let cursor_visible = payload[16] != 0;
    let cursor_blinking = payload[17] != 0;
    let cursor_style = i32::from_le_bytes([payload[18], payload[19], payload[20], payload[21]]);
    let cell_count = rows as usize * cols as usize;
    if payload.len() < LOCAL_FULL_STATE_HEADER_LEN + cell_count * REMOTE_CELL_ENCODED_LEN {
        return None;
    }
    let mut cells = Vec::with_capacity(cell_count);
    let mut offset = LOCAL_FULL_STATE_HEADER_LEN;
    for _ in 0..cell_count {
        cells.push(remote::RemoteCell {
            codepoint: u32::from_le_bytes([
                payload[offset],
                payload[offset + 1],
                payload[offset + 2],
                payload[offset + 3],
            ]),
            fg: [
                payload[offset + 4],
                payload[offset + 5],
                payload[offset + 6],
            ],
            bg: [
                payload[offset + 7],
                payload[offset + 8],
                payload[offset + 9],
            ],
            style_flags: payload[offset + 10],
            wide: payload[offset + 11] != 0,
        });
        offset += 12;
    }
    crate::profiling::record_bytes_and_units(
        "client.stream.decode_full_state",
        crate::profiling::Kind::Cpu,
        std::time::Duration::ZERO,
        payload.len() as u64,
        cell_count as u64,
    );
    Some((
        (ack_input_seq != 0).then_some(ack_input_seq),
        remote::RemoteFullState {
            epoch: 0,
            viewport_top: 0,
            scrollback_total: 0,
            rows,
            cols,
            cursor_x,
            cursor_y,
            cursor_visible,
            cursor_blinking,
            cursor_style,
            cells,
        },
    ))
}

fn decode_remote_pane_full_state(payload: &[u8]) -> Option<(u64, u64, remote::RemoteFullState)> {
    if payload.len() < crate::remote_wire::UI_PANE_UPDATE_HEADER_LEN {
        return None;
    }
    let pane_id = u64::from_le_bytes(payload[4..12].try_into().ok()?);
    let runtime_revision = u64::from_le_bytes(payload[20..28].try_into().ok()?);
    let (_, state) =
        decode_remote_full_state(&payload[crate::remote_wire::UI_PANE_UPDATE_HEADER_LEN..])?;
    Some((pane_id, runtime_revision, state))
}

fn decode_remote_delta(payload: &[u8]) -> Option<(Option<u64>, RemoteDelta)> {
    if payload.len() < LOCAL_DELTA_HEADER_LEN {
        return None;
    }
    let ack_input_seq = u64::from_le_bytes(payload[..8].try_into().ok()?);
    let row_count = u16::from_le_bytes([payload[8], payload[9]]) as usize;
    let cursor_x = u16::from_le_bytes([payload[10], payload[11]]);
    let cursor_y = u16::from_le_bytes([payload[12], payload[13]]);
    let cursor_visible = payload[14] != 0;
    let cursor_blinking = payload[15] != 0;
    let flags = payload[16];
    let cursor_style = i32::from_le_bytes([payload[17], payload[18], payload[19], payload[20]]);
    let mut offset = LOCAL_DELTA_HEADER_LEN;
    let scroll_rows = if (flags & 0x01) != 0 {
        if offset + 2 > payload.len() {
            return None;
        }
        let value = i16::from_le_bytes([payload[offset], payload[offset + 1]]);
        offset += 2;
        value
    } else {
        0
    };
    let mut changed_rows = Vec::with_capacity(row_count);
    let mut changed_cells = 0u64;
    for _ in 0..row_count {
        if offset + 6 > payload.len() {
            return None;
        }
        let row = u16::from_le_bytes([payload[offset], payload[offset + 1]]);
        let start_col = u16::from_le_bytes([payload[offset + 2], payload[offset + 3]]);
        let cols = u16::from_le_bytes([payload[offset + 4], payload[offset + 5]]) as usize;
        offset += 6;
        let mut cells = Vec::with_capacity(cols);
        for _ in 0..cols {
            if offset + 12 > payload.len() {
                return None;
            }
            cells.push(remote::RemoteCell {
                codepoint: u32::from_le_bytes([
                    payload[offset],
                    payload[offset + 1],
                    payload[offset + 2],
                    payload[offset + 3],
                ]),
                fg: [
                    payload[offset + 4],
                    payload[offset + 5],
                    payload[offset + 6],
                ],
                bg: [
                    payload[offset + 7],
                    payload[offset + 8],
                    payload[offset + 9],
                ],
                style_flags: payload[offset + 10],
                wide: payload[offset + 11] != 0,
            });
            offset += 12;
        }
        changed_cells += cells.len() as u64;
        changed_rows.push(RemoteRowDelta {
            row,
            start_col,
            cells,
        });
    }
    crate::profiling::record_bytes_and_units(
        "client.stream.decode_delta",
        crate::profiling::Kind::Cpu,
        std::time::Duration::ZERO,
        payload.len() as u64,
        changed_cells,
    );
    crate::profiling::record_units(
        "client.stream.decode_delta_rows",
        crate::profiling::Kind::Cpu,
        changed_rows.len() as u64,
    );
    Some((
        (ack_input_seq != 0).then_some(ack_input_seq),
        RemoteDelta {
            cursor_x,
            cursor_y,
            cursor_visible,
            cursor_blinking,
            cursor_style,
            scroll_rows,
            changed_rows,
        },
    ))
}

fn decode_remote_pane_delta(payload: &[u8]) -> Option<(u64, u64, RemoteDelta)> {
    if payload.len() < crate::remote_wire::UI_PANE_UPDATE_HEADER_LEN {
        return None;
    }
    let pane_id = u64::from_le_bytes(payload[4..12].try_into().ok()?);
    let runtime_revision = u64::from_le_bytes(payload[20..28].try_into().ok()?);
    let (_, delta) =
        decode_remote_delta(&payload[crate::remote_wire::UI_PANE_UPDATE_HEADER_LEN..])?;
    Some((pane_id, runtime_revision, delta))
}

fn apply_remote_delta_snapshot(
    snapshot: &mut vt_backend_core::TerminalSnapshot,
    delta: &RemoteDelta,
) {
    snapshot.cursor.x = delta.cursor_x;
    snapshot.cursor.y = delta.cursor_y;
    snapshot.cursor.visible = delta.cursor_visible;
    snapshot.cursor.blinking = delta.cursor_blinking;
    snapshot.cursor.style = vt::CursorStyle::from(delta.cursor_style);
    let cols = snapshot.cols as usize;
    if snapshot.row_revisions.len() != snapshot.rows_data.len() {
        snapshot.row_revisions.resize(snapshot.rows_data.len(), 1);
    }
    if delta.scroll_rows != 0 {
        apply_snapshot_scroll(snapshot, delta.scroll_rows);
    }
    let mut applied_rows = 0u64;
    let mut applied_cells = 0u64;
    for row_delta in &delta.changed_rows {
        let row_index = row_delta.row as usize;
        if row_index >= snapshot.rows_data.len() {
            continue;
        }
        let target_row = &mut snapshot.rows_data[row_index];
        if target_row.len() < cols {
            target_row.resize_with(cols, Default::default);
        }
        let start_col = row_delta.start_col as usize;
        if start_col >= cols {
            continue;
        }
        for (offset, cell) in row_delta
            .cells
            .iter()
            .enumerate()
            .take(cols.saturating_sub(start_col))
        {
            target_row[start_col + offset] = remote_cell_to_snapshot_default(cell);
            applied_cells += 1;
        }
        applied_rows += 1;
        if let Some(revision) = snapshot.row_revisions.get_mut(row_index) {
            *revision = revision.wrapping_add(1);
        }
    }
    crate::profiling::record_units(
        "client.stream.apply_delta_rows",
        crate::profiling::Kind::Cpu,
        applied_rows,
    );
    crate::profiling::record_units(
        "client.stream.apply_delta_cells",
        crate::profiling::Kind::Cpu,
        applied_cells,
    );
}

fn apply_snapshot_scroll(snapshot: &mut vt_backend_core::TerminalSnapshot, scroll_rows: i16) {
    let rows = snapshot.rows_data.len();
    if rows == 0 {
        return;
    }
    let cols = snapshot.cols as usize;
    if snapshot.row_revisions.len() != rows {
        snapshot.row_revisions.resize(rows, 1);
    }
    let blank_row = || vec![vt_backend_core::CellSnapshot::default(); cols];
    if scroll_rows > 0 {
        let shift = (scroll_rows as usize).min(rows);
        snapshot.rows_data.rotate_left(shift);
        snapshot.row_revisions.rotate_left(shift);
        for row_index in rows - shift..rows {
            snapshot.rows_data[row_index] = blank_row();
            snapshot.row_revisions[row_index] = snapshot.row_revisions[row_index].wrapping_add(1);
        }
    } else {
        let shift = ((-scroll_rows) as usize).min(rows);
        snapshot.rows_data.rotate_right(shift);
        snapshot.row_revisions.rotate_right(shift);
        for row_index in 0..shift {
            snapshot.rows_data[row_index] = blank_row();
            snapshot.row_revisions[row_index] = snapshot.row_revisions[row_index].wrapping_add(1);
        }
    }
}

fn remote_cell_to_snapshot(
    cell: &remote::RemoteCell,
    default_foreground: crate::config::RgbColor,
    default_background: crate::config::RgbColor,
) -> vt_backend_core::CellSnapshot {
    const REMOTE_STYLE_FLAG_BOLD: u8 = 0x01;
    const REMOTE_STYLE_FLAG_ITALIC: u8 = 0x02;
    const REMOTE_STYLE_FLAG_HYPERLINK: u8 = 0x04;
    const REMOTE_STYLE_FLAG_EXPLICIT_FG: u8 = 0x20;
    const REMOTE_STYLE_FLAG_EXPLICIT_BG: u8 = 0x40;
    let default_fg = vt::RgbColor::from_array(default_foreground);
    let default_bg = vt::RgbColor::from_array(default_background);
    vt_backend_core::CellSnapshot {
        text: if cell.codepoint == 0 {
            String::new()
        } else {
            std::char::from_u32(cell.codepoint)
                .map(|ch| ch.to_string())
                .unwrap_or_default()
        },
        display_width: if cell.wide { 2 } else { 1 },
        fg: if (cell.style_flags & REMOTE_STYLE_FLAG_EXPLICIT_FG) != 0 {
            vt::RgbColor::from_array(cell.fg)
        } else {
            default_fg
        },
        bg: if (cell.style_flags & REMOTE_STYLE_FLAG_EXPLICIT_BG) != 0 {
            vt::RgbColor::from_array(cell.bg)
        } else {
            default_bg
        },
        bg_is_default: (cell.style_flags & REMOTE_STYLE_FLAG_EXPLICIT_BG) == 0,
        bold: (cell.style_flags & REMOTE_STYLE_FLAG_BOLD) != 0,
        italic: (cell.style_flags & REMOTE_STYLE_FLAG_ITALIC) != 0,
        underline: 0,
        hyperlink: (cell.style_flags & REMOTE_STYLE_FLAG_HYPERLINK) != 0,
    }
}

fn remote_cell_to_snapshot_default(cell: &remote::RemoteCell) -> vt_backend_core::CellSnapshot {
    remote_cell_to_snapshot(
        cell,
        crate::DEFAULT_TERMINAL_FOREGROUND,
        crate::DEFAULT_TERMINAL_BACKGROUND,
    )
}

fn build_status_right(
    ui_state: &ClientUiState,
    mode: ClientMode,
    active_remote_tab_id: Option<u32>,
    last_error: Option<&str>,
    remote_host: Option<&str>,
    remote_debug_summary: Option<&str>,
) -> String {
    let mut right_parts = Vec::new();
    let remote_prefix = match remote_host.filter(|value| !value.is_empty()) {
        Some(host) => format!("remote:{host}"),
        None => "remote".to_string(),
    };
    if let Some(error) = last_error.filter(|value| !value.is_empty()) {
        right_parts.push(format!("{remote_prefix}: error {error}"));
    } else {
        match mode {
            ClientMode::Bootstrapping => {
                right_parts.push(format!("{remote_prefix}: bootstrapping"))
            }
            ClientMode::Recovering => {
                if let Some(tab_id) = active_remote_tab_id {
                    right_parts.push(format!("{remote_prefix}: recovering tab {tab_id}"))
                } else {
                    right_parts.push(format!("{remote_prefix}: recovering"))
                }
            }
            ClientMode::Active => right_parts.push(format!("{remote_prefix}: connected")),
        }
    }
    if ui_state.pane_count > 1 {
        right_parts.push(format!("{} panes", ui_state.pane_count));
    }
    if !ui_state.pwd.is_empty() {
        right_parts.push(ui_state.pwd.clone());
    }
    if let Some(summary) = remote_debug_summary.filter(|value| !value.is_empty()) {
        right_parts.push(summary.to_string());
    }
    right_parts.join("  ")
}

fn render_status_text<'a>(value: String, status_text_size: f32) -> Element<'a, Message> {
    text(value)
        .font(Font::MONOSPACE)
        .size(status_text_size)
        .color(Color::from_rgb(0.6, 0.6, 0.6))
        .into()
}

fn render_status_zone<'a>(
    segments: &'a [crate::status_components::UiStatusComponent],
    status_text_size: f32,
) -> Element<'a, Message> {
    let mut segments_row = row![].spacing(0);
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
        segments_row = segments_row.push(
            container(
                text(segment.text.clone())
                    .font(Font::MONOSPACE)
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
    segments_row.into()
}

fn named_key_from_iced_key(key: &keyboard::Key) -> Option<bindings::NamedKey> {
    use keyboard::key::Named;

    match key {
        keyboard::Key::Named(Named::ArrowUp) => Some(bindings::NamedKey::ArrowUp),
        keyboard::Key::Named(Named::ArrowDown) => Some(bindings::NamedKey::ArrowDown),
        keyboard::Key::Named(Named::ArrowLeft) => Some(bindings::NamedKey::ArrowLeft),
        keyboard::Key::Named(Named::ArrowRight) => Some(bindings::NamedKey::ArrowRight),
        keyboard::Key::Named(Named::PageUp) => Some(bindings::NamedKey::PageUp),
        keyboard::Key::Named(Named::PageDown) => Some(bindings::NamedKey::PageDown),
        keyboard::Key::Named(Named::Home) => Some(bindings::NamedKey::Home),
        keyboard::Key::Named(Named::End) => Some(bindings::NamedKey::End),
        keyboard::Key::Named(Named::Escape) => Some(bindings::NamedKey::Escape),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_cell_defaults_to_terminal_colors_when_not_explicit() {
        let cell = remote::RemoteCell {
            codepoint: u32::from('x'),
            fg: [0, 0, 0],
            bg: [0, 0, 0],
            style_flags: 0,
            wide: false,
        };
        let snapshot = remote_cell_to_snapshot_default(&cell);
        assert_eq!(snapshot.fg.r, 0xf0);
        assert_eq!(snapshot.fg.g, 0xf0);
        assert_eq!(snapshot.fg.b, 0xf0);
        assert_eq!(snapshot.bg.r, 0);
        assert_eq!(snapshot.bg.g, 0);
        assert_eq!(snapshot.bg.b, 0);
    }

    #[test]
    fn remote_cell_preserves_explicit_colors() {
        let cell = remote::RemoteCell {
            codepoint: u32::from('x'),
            fg: [1, 2, 3],
            bg: [4, 5, 6],
            style_flags: 0x60,
            wide: false,
        };
        let snapshot = remote_cell_to_snapshot_default(&cell);
        assert_eq!((snapshot.fg.r, snapshot.fg.g, snapshot.fg.b), (1, 2, 3));
        assert_eq!((snapshot.bg.r, snapshot.bg.g, snapshot.bg.b), (4, 5, 6));
    }

    #[test]
    fn parse_gui_test_text_command() {
        assert_eq!(
            parse_gui_test_command("text hello"),
            Some(GuiTestCommand::Text("hello".to_string()))
        );
    }

    #[test]
    fn parse_gui_test_text_command_decodes_escapes() {
        assert_eq!(
            parse_gui_test_command(r"text line1\rline2\n\t\\\x41"),
            Some(GuiTestCommand::Text("line1\rline2\n\t\\A".to_string()))
        );
    }

    #[test]
    fn parse_gui_test_resize_command() {
        assert_eq!(
            parse_gui_test_command("resize 120 40"),
            Some(GuiTestCommand::Resize {
                cols: 120,
                rows: 40
            })
        );
    }

    #[test]
    fn parse_gui_test_refresh_command() {
        assert_eq!(
            parse_gui_test_command("refresh"),
            Some(GuiTestCommand::Refresh)
        );
    }

    #[test]
    fn pane_dividers_draw_only_internal_vertical_split() {
        let panes = vec![
            test_pane(0.0, 0.0, 49.5, 80.0),
            test_pane(50.5, 0.0, 49.5, 80.0),
        ];
        let dividers = pane_dividers(&panes);
        assert_eq!(dividers.len(), 1);
        assert!((dividers[0].start.x - 50.0).abs() < 0.01);
        assert_eq!(dividers[0].start.y, 0.0);
        assert_eq!(dividers[0].end.y, 80.0);
    }

    #[test]
    fn pane_dividers_do_not_draw_outer_pane_rectangles() {
        let panes = vec![
            test_pane(0.0, 0.0, 49.5, 40.0),
            test_pane(50.5, 0.0, 49.5, 40.0),
            test_pane(0.0, 41.0, 100.0, 39.0),
        ];
        let dividers = pane_dividers(&panes);
        assert_eq!(dividers.len(), 3);
        assert!(
            !dividers
                .iter()
                .any(|divider| divider.start.x == 0.0 && divider.end.x == 0.0)
        );
        assert!(
            !dividers
                .iter()
                .any(|divider| divider.start.y == 0.0 && divider.end.y == 0.0)
        );
    }

    #[test]
    fn focused_pane_cursor_uses_blink_phase() {
        let interval = Duration::from_millis(500);
        let hidden_epoch = Instant::now() - Duration::from_millis(750);
        assert!(!pane_cursor_blink_visible(
            true,
            true,
            true,
            hidden_epoch,
            interval
        ));
    }

    #[test]
    fn unfocused_pane_cursor_stays_visible_when_blink_phase_is_hidden() {
        let interval = Duration::from_millis(500);
        let hidden_epoch = Instant::now() - Duration::from_millis(750);
        assert!(pane_cursor_blink_visible(
            false,
            true,
            true,
            hidden_epoch,
            interval
        ));
    }

    #[test]
    fn focused_pane_cursor_stays_visible_when_app_is_unfocused() {
        let interval = Duration::from_millis(500);
        let hidden_epoch = Instant::now() - Duration::from_millis(750);
        assert!(pane_cursor_blink_visible(
            true,
            true,
            false,
            hidden_epoch,
            interval
        ));
    }

    #[test]
    fn focused_pane_blink_scheduler_ignores_unfocused_blinking_cursors() {
        let (mut app, _) = ClientApp::new("/tmp/boo-test.sock".to_string());
        app.focused_pane_id = 1;
        app.pane_snapshots
            .insert(1, Arc::new(test_terminal_snapshot_with_cursor(0, 0, false)));
        app.pane_snapshots
            .insert(2, Arc::new(test_terminal_snapshot_with_cursor(1, 0, true)));

        assert!(!app.focused_pane_has_blinking_cursor());
    }

    #[test]
    fn focused_pane_blink_scheduler_tracks_focused_blinking_cursor() {
        let (mut app, _) = ClientApp::new("/tmp/boo-test.sock".to_string());
        app.focused_pane_id = 2;
        app.pane_snapshots
            .insert(1, Arc::new(test_terminal_snapshot_with_cursor(0, 0, false)));
        app.pane_snapshots
            .insert(2, Arc::new(test_terminal_snapshot_with_cursor(1, 0, true)));

        assert!(app.focused_pane_has_blinking_cursor());
    }

    #[test]
    fn focused_cursor_movement_resets_blink_epoch() {
        let (mut app, _) = ClientApp::new("/tmp/boo-test.sock".to_string());
        app.focused_pane_id = 42;
        app.pane_snapshots
            .insert(42, Arc::new(test_terminal_snapshot_with_cursor(0, 0, true)));
        app.observe_focused_cursor_position();

        let hidden_epoch = Instant::now() - Duration::from_millis(750);
        app.cursor_blink_epoch = hidden_epoch;
        app.pane_snapshots
            .insert(42, Arc::new(test_terminal_snapshot_with_cursor(1, 0, true)));

        app.observe_focused_cursor_position();

        assert!(app.cursor_blink_epoch > hidden_epoch);
        assert_eq!(app.focused_cursor_position, Some((42, 1, 0)));
        assert!(pane_cursor_blink_visible(
            true,
            true,
            true,
            app.cursor_blink_epoch,
            Duration::from_millis(500)
        ));
    }

    #[test]
    fn unchanged_focused_cursor_does_not_reset_blink_epoch() {
        let (mut app, _) = ClientApp::new("/tmp/boo-test.sock".to_string());
        app.focused_pane_id = 42;
        app.pane_snapshots
            .insert(42, Arc::new(test_terminal_snapshot_with_cursor(0, 0, true)));
        app.observe_focused_cursor_position();

        let hidden_epoch = Instant::now() - Duration::from_millis(750);
        app.cursor_blink_epoch = hidden_epoch;

        app.observe_focused_cursor_position();

        assert_eq!(app.cursor_blink_epoch, hidden_epoch);
        assert_eq!(app.focused_cursor_position, Some((42, 0, 0)));
    }

    #[test]
    fn iced_input_method_commit_sends_text_and_clears_preedit() {
        let (mut app, _) = ClientApp::new("/tmp/boo-test.sock".to_string());
        let (tx, rx) = std::sync::mpsc::channel();
        app.stream_tx = Some(tx);
        app.mode = ClientMode::Active;

        app.handle_input_method(input_method::Event::Preedit("かな".to_string(), Some(0..6)));
        assert_eq!(app.preedit_text, "かな");

        app.handle_input_method(input_method::Event::Commit("仮名".to_string()));

        assert!(app.preedit_text.is_empty());
        match rx.recv().unwrap() {
            StreamCommand::Input { bytes, .. } => assert_eq!(bytes, "仮名".as_bytes()),
            other => panic!("unexpected stream command: {other:?}"),
        }
    }

    #[test]
    fn apply_ui_runtime_state_tracks_mouse_selection() {
        let (mut app, _) = ClientApp::new("/tmp/boo-test.sock".to_string());
        app.active_remote_tab_id = Some(7);
        app.mode = ClientMode::Active;
        app.apply_ui_runtime_state(control::UiRuntimeState {
            active_tab: 0,
            focused_pane: 7,
            tabs: vec![],
            visible_panes: vec![test_pane_with_id(7, 0.0, 0.0, 80.0, 25.0)],
            mouse_selection: control::UiMouseSelectionSnapshot {
                active: true,
                pane_id: Some(7),
                selection_rects: vec![control::UiRectSnapshot {
                    x: 10.0,
                    y: 12.0,
                    width: 30.0,
                    height: 16.0,
                }],
            },
            status_bar: crate::status_components::UiStatusBarSnapshot::default(),
            pwd: "/tmp".to_string(),
            runtime_revision: 1,
            view_revision: 1,
            view_id: 1,
            viewed_tab_id: Some(7),
            viewport_cols: None,
            viewport_rows: None,
            visible_pane_ids: vec![7],
            acked_client_action_id: None,
        });

        assert!(app.mouse_selection.active);
        assert_eq!(app.mouse_selection.pane_id, Some(7));
        assert_eq!(app.mouse_selection.selection_rects.len(), 1);
    }

    #[test]
    fn full_state_bootstraps_without_ui_pane_terminals() {
        let (mut app, _) = ClientApp::new("/tmp/boo-test.sock".to_string());

        app.handle_stream_event(LocalStreamEvent::UiRuntimeState(control::UiRuntimeState {
            active_tab: 0,
            focused_pane: 7,
            tabs: vec![control::UiTabSnapshot {
                tab_id: 7,
                index: 0,
                active: true,
                title: "shell".to_string(),
                pane_count: 1,
                focused_pane: Some(7),
                pane_ids: vec![7],
            }],
            visible_panes: vec![test_pane_with_id(7, 0.0, 0.0, 80.0, 25.0)],
            mouse_selection: control::UiMouseSelectionSnapshot::default(),
            status_bar: crate::status_components::UiStatusBarSnapshot::default(),
            pwd: String::new(),
            runtime_revision: 1,
            view_revision: 1,
            view_id: 1,
            viewed_tab_id: Some(7),
            viewport_cols: None,
            viewport_rows: None,
            visible_pane_ids: vec![7],
            acked_client_action_id: None,
        }));
        app.handle_stream_event(LocalStreamEvent::FullState {
            ack_input_seq: None,
            state: remote::RemoteFullState {
                epoch: 0,
                viewport_top: 0,
                scrollback_total: 0,
                rows: 1,
                cols: 2,
                cursor_x: 1,
                cursor_y: 0,
                cursor_visible: true,
                cursor_blinking: false,
                cursor_style: 1,
                cells: vec![
                    remote::RemoteCell {
                        codepoint: 'o' as u32,
                        fg: [255, 255, 255],
                        bg: [0, 0, 0],
                        style_flags: 0,
                        wide: false,
                    },
                    remote::RemoteCell {
                        codepoint: 'k' as u32,
                        fg: [255, 255, 255],
                        bg: [0, 0, 0],
                        style_flags: 0,
                        wide: false,
                    },
                ],
            },
        });

        assert!(app.bootstrapped);
        assert!(matches!(app.mode, ClientMode::Active));
        assert!(app.has_paintable_terminal());
    }

    #[test]
    fn parse_gui_test_command_command() {
        assert_eq!(
            parse_gui_test_command("command new-tab"),
            Some(GuiTestCommand::Command("new-tab".to_string()))
        );
    }

    #[test]
    fn parse_gui_test_activate_tab_command() {
        assert_eq!(
            parse_gui_test_command("activate-tab 42"),
            Some(GuiTestCommand::ActivateTab(42))
        );
    }

    #[test]
    fn parse_gui_test_click_command() {
        assert_eq!(
            parse_gui_test_command("click 120.5 33.25"),
            Some(GuiTestCommand::Click { x: 120.5, y: 33.25 })
        );
    }

    #[test]
    fn parse_gui_test_drag_command() {
        assert_eq!(
            parse_gui_test_command("drag 10 20 30 40"),
            Some(GuiTestCommand::Drag {
                x1: 10.0,
                y1: 20.0,
                x2: 30.0,
                y2: 40.0,
            })
        );
    }

    #[test]
    fn parse_gui_test_appkey_command() {
        assert_eq!(
            parse_gui_test_command("appkey shift+0x27"),
            Some(GuiTestCommand::AppKey {
                keyspec: "shift+0x27".to_string(),
                repeat: false,
            })
        );
        assert_eq!(
            parse_gui_test_command("appkey-repeat j"),
            Some(GuiTestCommand::AppKey {
                keyspec: "j".to_string(),
                repeat: true,
            })
        );
        assert_eq!(
            parse_gui_test_command("keyboard-repeat j"),
            Some(GuiTestCommand::Keyboard {
                keyspec: "j".to_string(),
                repeat: true,
            })
        );
    }

    #[test]
    fn handle_keyboard_sends_raw_app_key_event_over_stream() {
        let (mut app, _) = ClientApp::new("/tmp/boo-test.sock".to_string());
        let (tx, rx) = std::sync::mpsc::channel();
        app.stream_tx = Some(tx);
        app.mode = ClientMode::Active;

        app.handle_keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Character("'".into()),
            modified_key: keyboard::Key::Character("\"".into()),
            physical_key: keyboard::key::Physical::Code(keyboard::key::Code::Quote),
            location: keyboard::Location::Standard,
            modifiers: keyboard::Modifiers::SHIFT,
            text: Some("\"".into()),
            repeat: false,
        });

        match rx.recv().unwrap() {
            StreamCommand::AppKeyEvent { event } => {
                assert_eq!(
                    event.keycode,
                    crate::keymap::physical_to_native_keycode(&keyboard::key::Physical::Code(
                        keyboard::key::Code::Quote
                    ))
                    .expect("quote key should map to a native keycode")
                );
                assert_eq!(event.text.as_deref(), Some("\""));
                assert_eq!(event.modified_text.as_deref(), Some("\""));
                assert_eq!(event.named_key, None);
            }
            other => panic!("unexpected stream command: {other:?}"),
        }
    }

    fn test_terminal_snapshot_with_cursor(
        x: u16,
        y: u16,
        blinking: bool,
    ) -> vt_backend_core::TerminalSnapshot {
        vt_backend_core::TerminalSnapshot {
            cols: 2,
            rows: 1,
            cursor: vt_backend_core::CursorSnapshot {
                visible: true,
                blinking,
                x,
                y,
                style: vt::CursorStyle::Block,
            },
            rows_data: vec![vec![vt_backend_core::CellSnapshot::default(); 2]],
            row_revisions: vec![0],
            ..Default::default()
        }
    }

    fn test_pane(x: f64, y: f64, width: f64, height: f64) -> control::UiPaneSnapshot {
        test_pane_with_id(0, x, y, width, height)
    }

    fn test_pane_with_id(
        pane_id: u64,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    ) -> control::UiPaneSnapshot {
        control::UiPaneSnapshot {
            leaf_index: 0,
            leaf_id: 0,
            pane_id,
            focused: false,
            frame: control::UiRectSnapshot {
                x,
                y,
                width,
                height,
            },
            split_direction: None,
            split_ratio: None,
        }
    }

    #[test]
    fn coalesces_only_superseded_passive_screen_updates() {
        let passive_a = LocalStreamEvent::Delta {
            ack_input_seq: None,
            delta: RemoteDelta {
                cursor_x: 0,
                cursor_y: 0,
                cursor_visible: true,
                cursor_blinking: false,
                cursor_style: 1,
                scroll_rows: 0,
                changed_rows: Vec::new(),
            },
        };
        let passive_b = LocalStreamEvent::FullState {
            ack_input_seq: None,
            state: remote::RemoteFullState {
                epoch: 0,
                viewport_top: 0,
                scrollback_total: 0,
                rows: 1,
                cols: 1,
                cursor_x: 0,
                cursor_y: 0,
                cursor_visible: true,
                cursor_blinking: false,
                cursor_style: 1,
                cells: vec![remote::RemoteCell {
                    codepoint: 0,
                    fg: [0, 0, 0],
                    bg: [0, 0, 0],
                    style_flags: 0,
                    wide: false,
                }],
            },
        };
        let acked = LocalStreamEvent::Delta {
            ack_input_seq: Some(1),
            delta: RemoteDelta {
                cursor_x: 1,
                cursor_y: 0,
                cursor_visible: true,
                cursor_blinking: false,
                cursor_style: 1,
                scroll_rows: 0,
                changed_rows: Vec::new(),
            },
        };
        let barrier = LocalStreamEvent::TabExited;

        let mut batch = Vec::new();
        let mut pending = PendingCoalescedStreamEvents::with_passive_screen_coalescing(true);
        push_coalesced_stream_event(&mut batch, &mut pending, passive_a);
        push_coalesced_stream_event(&mut batch, &mut pending, passive_b);
        push_coalesced_stream_event(&mut batch, &mut pending, acked.clone());
        push_coalesced_stream_event(&mut batch, &mut pending, barrier.clone());
        flush_pending_passive_stream_event(&mut batch, &mut pending);

        assert_eq!(batch.len(), 3);
        assert!(matches!(
            batch[0],
            LocalStreamEvent::FullState {
                ack_input_seq: None,
                ..
            }
        ));
        assert!(matches!(
            batch[1],
            LocalStreamEvent::Delta {
                ack_input_seq: Some(1),
                ..
            }
        ));
        assert!(matches!(batch[2], LocalStreamEvent::TabExited));
    }

    #[test]
    fn preserves_consecutive_passive_deltas_in_order() {
        let passive_a = LocalStreamEvent::Delta {
            ack_input_seq: None,
            delta: RemoteDelta {
                cursor_x: 0,
                cursor_y: 0,
                cursor_visible: true,
                cursor_blinking: false,
                cursor_style: 1,
                scroll_rows: 0,
                changed_rows: vec![RemoteRowDelta {
                    row: 0,
                    start_col: 0,
                    cells: Vec::new(),
                }],
            },
        };
        let passive_b = LocalStreamEvent::Delta {
            ack_input_seq: None,
            delta: RemoteDelta {
                cursor_x: 1,
                cursor_y: 1,
                cursor_visible: true,
                cursor_blinking: false,
                cursor_style: 1,
                scroll_rows: 0,
                changed_rows: vec![RemoteRowDelta {
                    row: 1,
                    start_col: 0,
                    cells: Vec::new(),
                }],
            },
        };

        let mut batch = Vec::new();
        let mut pending = PendingCoalescedStreamEvents::with_passive_screen_coalescing(true);
        push_coalesced_stream_event(&mut batch, &mut pending, passive_a.clone());
        push_coalesced_stream_event(&mut batch, &mut pending, passive_b.clone());
        flush_pending_passive_stream_event(&mut batch, &mut pending);

        assert_eq!(batch.len(), 2);
        match &batch[0] {
            LocalStreamEvent::Delta { delta, .. } => {
                assert_eq!(delta.cursor_x, 0);
                assert_eq!(delta.cursor_y, 0);
            }
            other => panic!("expected first batch entry to be delta, got {other:?}"),
        }
        match &batch[1] {
            LocalStreamEvent::Delta { delta, .. } => {
                assert_eq!(delta.cursor_x, 1);
                assert_eq!(delta.cursor_y, 1);
            }
            other => panic!("expected second batch entry to be delta, got {other:?}"),
        }
    }

    #[test]
    fn coalesces_ui_runtime_and_appearance_within_burst() {
        let appearance = |family: &str| control::UiAppearanceSnapshot {
            font_families: vec![family.into()],
            font_size: 14.0,
            background_opacity: 1.0,
            background_opacity_cells: false,
            terminal_foreground: [0, 0, 0],
            terminal_background: [0, 0, 0],
            cursor_color: [0, 0, 0],
            selection_background: [0, 0, 0],
            selection_foreground: [0, 0, 0],
            cursor_text_color: [0, 0, 0],
            url_color: [0, 0, 0],
            active_tab_foreground: [0, 0, 0],
            active_tab_background: [0, 0, 0],
            inactive_tab_foreground: [0, 0, 0],
            inactive_tab_background: [0, 0, 0],
            cursor_style: None,
            cursor_blink: true,
            cursor_blink_interval_ns: 600_000_000,
        };
        let runtime_a = LocalStreamEvent::UiRuntimeState(control::UiRuntimeState {
            tabs: vec![],
            active_tab: 0,
            pwd: "a".into(),
            visible_panes: vec![],
            focused_pane: 1,
            mouse_selection: control::UiMouseSelectionSnapshot::default(),
            status_bar: crate::status_components::UiStatusBarSnapshot::default(),
            runtime_revision: 1,
            view_revision: 1,
            view_id: 1,
            viewed_tab_id: Some(1),
            viewport_cols: None,
            viewport_rows: None,
            visible_pane_ids: vec![],
            acked_client_action_id: None,
        });
        let runtime_b = LocalStreamEvent::UiRuntimeState(control::UiRuntimeState {
            tabs: vec![],
            active_tab: 0,
            pwd: "b".into(),
            visible_panes: vec![],
            focused_pane: 2,
            mouse_selection: control::UiMouseSelectionSnapshot::default(),
            status_bar: crate::status_components::UiStatusBarSnapshot::default(),
            runtime_revision: 2,
            view_revision: 2,
            view_id: 1,
            viewed_tab_id: Some(2),
            viewport_cols: None,
            viewport_rows: None,
            visible_pane_ids: vec![],
            acked_client_action_id: None,
        });
        let appearance_a = LocalStreamEvent::UiAppearance(appearance("A"));
        let appearance_b = LocalStreamEvent::UiAppearance(appearance("B"));
        let barrier = LocalStreamEvent::TabExited;

        let mut batch = Vec::new();
        let mut pending = PendingCoalescedStreamEvents::with_passive_screen_coalescing(true);
        push_coalesced_stream_event(&mut batch, &mut pending, runtime_a);
        push_coalesced_stream_event(&mut batch, &mut pending, appearance_a);
        push_coalesced_stream_event(&mut batch, &mut pending, runtime_b.clone());
        push_coalesced_stream_event(&mut batch, &mut pending, appearance_b.clone());
        push_coalesced_stream_event(&mut batch, &mut pending, barrier.clone());
        flush_pending_passive_stream_event(&mut batch, &mut pending);

        assert_eq!(batch.len(), 3);
        match &batch[0] {
            LocalStreamEvent::UiRuntimeState(state) => {
                assert_eq!(state.pwd, "b");
                assert_eq!(state.focused_pane, 2);
            }
            other => panic!("expected coalesced runtime state, got {other:?}"),
        }
        match &batch[1] {
            LocalStreamEvent::UiAppearance(appearance) => {
                assert_eq!(appearance.font_families, vec!["B"]);
            }
            other => panic!("expected coalesced appearance, got {other:?}"),
        }
        assert!(matches!(batch[2], LocalStreamEvent::TabExited));
    }

    #[test]
    fn acknowledged_input_latencies_drain_in_sequence_order() {
        let now = Instant::now();
        let mut pending = BTreeMap::from([
            (4_u64, now),
            (5_u64, now + Duration::from_millis(1)),
            (7_u64, now + Duration::from_millis(2)),
        ]);

        let acknowledged = take_acknowledged_input_latencies(&mut pending, 5);

        assert_eq!(
            acknowledged.iter().map(|(seq, _)| *seq).collect::<Vec<_>>(),
            vec![4, 5]
        );
        assert_eq!(pending.keys().copied().collect::<Vec<_>>(), vec![7]);
    }

    #[test]
    fn apply_remote_delta_scrolls_snapshot_before_replacing_changed_rows() {
        let mut snapshot = vt_backend_core::TerminalSnapshot {
            cols: 1,
            rows: 3,
            cursor: vt_backend_core::CursorSnapshot {
                visible: true,
                blinking: false,
                x: 0,
                y: 2,
                style: vt::CursorStyle::Bar,
            },
            rows_data: vec![
                vec![vt_backend_core::CellSnapshot {
                    text: "a".to_string(),
                    ..Default::default()
                }],
                vec![vt_backend_core::CellSnapshot {
                    text: "b".to_string(),
                    ..Default::default()
                }],
                vec![vt_backend_core::CellSnapshot {
                    text: "c".to_string(),
                    ..Default::default()
                }],
            ],
            ..Default::default()
        };

        apply_remote_delta_snapshot(
            &mut snapshot,
            &RemoteDelta {
                cursor_x: 0,
                cursor_y: 2,
                cursor_visible: true,
                cursor_blinking: false,
                cursor_style: 0,
                scroll_rows: 1,
                changed_rows: vec![RemoteRowDelta {
                    row: 2,
                    start_col: 0,
                    cells: vec![remote::RemoteCell {
                        codepoint: u32::from('d'),
                        fg: [0, 0, 0],
                        bg: [0, 0, 0],
                        style_flags: 0,
                        wide: false,
                    }],
                }],
            },
        );

        let texts = snapshot
            .rows_data
            .iter()
            .map(|row| {
                row.first()
                    .map(|cell| cell.text.clone())
                    .unwrap_or_default()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            texts,
            vec!["b".to_string(), "c".to_string(), "d".to_string()]
        );
    }

    #[test]
    fn apply_snapshot_scroll_rotates_row_revisions_with_content() {
        let mut snapshot = vt_backend_core::TerminalSnapshot {
            cols: 1,
            rows: 3,
            rows_data: vec![
                vec![vt_backend_core::CellSnapshot {
                    text: "a".to_string(),
                    ..Default::default()
                }],
                vec![vt_backend_core::CellSnapshot {
                    text: "b".to_string(),
                    ..Default::default()
                }],
                vec![vt_backend_core::CellSnapshot {
                    text: "c".to_string(),
                    ..Default::default()
                }],
            ],
            row_revisions: vec![10, 20, 30],
            ..Default::default()
        };

        apply_snapshot_scroll(&mut snapshot, 1);

        let texts = snapshot
            .rows_data
            .iter()
            .map(|row| {
                row.first()
                    .map(|cell| cell.text.clone())
                    .unwrap_or_default()
            })
            .collect::<Vec<_>>();
        assert_eq!(texts, vec!["b".to_string(), "c".to_string(), String::new()]);
        assert_eq!(snapshot.row_revisions, vec![20, 30, 11]);
    }

    #[test]
    fn decode_remote_pane_full_state_uses_prefixed_tab_and_pane_header() {
        let full_state = remote::RemoteFullState {
            epoch: 0,
            viewport_top: 0,
            scrollback_total: 0,
            rows: 1,
            cols: 1,
            cursor_x: 0,
            cursor_y: 0,
            cursor_visible: true,
            cursor_blinking: false,
            cursor_style: 0,
            cells: vec![remote::RemoteCell {
                codepoint: u32::from('x'),
                fg: [1, 2, 3],
                bg: [4, 5, 6],
                style_flags: 0,
                wide: false,
            }],
        };
        let payload = crate::remote_wire::encode_ui_pane_update_payload(
            77,
            99,
            3,
            4,
            &remote::encode_full_state(&full_state, None, true),
        );

        let (pane_id, runtime_revision, decoded) =
            decode_remote_pane_full_state(&payload).expect("pane full state");
        assert_eq!(pane_id, 99);
        assert_eq!(runtime_revision, 4);
        assert_eq!(decoded.rows, 1);
        assert_eq!(decoded.cols, 1);
        assert_eq!(decoded.cells.len(), 1);
        assert_eq!(decoded.cells[0].codepoint, u32::from('x'));
    }

    #[test]
    fn decode_remote_pane_delta_uses_prefixed_tab_and_pane_header() {
        let mut delta = Vec::new();
        delta.extend_from_slice(&0_u64.to_le_bytes());
        delta.extend_from_slice(&1_u16.to_le_bytes());
        delta.extend_from_slice(&0_u16.to_le_bytes());
        delta.extend_from_slice(&0_u16.to_le_bytes());
        delta.push(1);
        delta.push(0);
        delta.push(0);
        delta.extend_from_slice(&0_i32.to_le_bytes());
        delta.extend_from_slice(&0_u16.to_le_bytes());
        delta.extend_from_slice(&0_u16.to_le_bytes());
        delta.extend_from_slice(&1_u16.to_le_bytes());
        delta.extend_from_slice(&u32::from('b').to_le_bytes());
        delta.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0]);
        let payload =
            crate::remote_wire::encode_ui_pane_update_payload(77, 101, 5, 6, &delta);

        let (pane_id, runtime_revision, decoded) =
            decode_remote_pane_delta(&payload).expect("pane delta");
        assert_eq!(pane_id, 101);
        assert_eq!(runtime_revision, 6);
        assert_eq!(decoded.changed_rows.len(), 1);
        assert_eq!(decoded.changed_rows[0].cells[0].codepoint, u32::from('b'));
    }

    #[test]
    fn remote_blank_cell_stays_empty() {
        let snapshot = remote_cell_to_snapshot_default(&remote::RemoteCell {
            codepoint: 0,
            fg: [0, 0, 0],
            bg: [0, 0, 0],
            style_flags: 0,
            wide: false,
        });
        assert_eq!(snapshot.text, "");
        assert_eq!(snapshot.display_width, 1);
    }

    #[test]
    fn apply_remote_delta_updates_only_changed_segment() {
        let mut snapshot = vt_backend_core::TerminalSnapshot {
            cols: 5,
            rows: 1,
            rows_data: vec![vec![
                vt_backend_core::CellSnapshot {
                    text: "a".to_string(),
                    ..Default::default()
                },
                vt_backend_core::CellSnapshot {
                    text: "b".to_string(),
                    ..Default::default()
                },
                vt_backend_core::CellSnapshot {
                    text: "c".to_string(),
                    ..Default::default()
                },
                vt_backend_core::CellSnapshot {
                    text: "d".to_string(),
                    ..Default::default()
                },
                vt_backend_core::CellSnapshot {
                    text: "e".to_string(),
                    ..Default::default()
                },
            ]],
            row_revisions: vec![1],
            ..Default::default()
        };

        apply_remote_delta_snapshot(
            &mut snapshot,
            &RemoteDelta {
                cursor_x: 2,
                cursor_y: 0,
                cursor_visible: true,
                cursor_blinking: false,
                cursor_style: 1,
                scroll_rows: 0,
                changed_rows: vec![RemoteRowDelta {
                    row: 0,
                    start_col: 2,
                    cells: vec![remote::RemoteCell {
                        codepoint: u32::from('X'),
                        fg: [0, 0, 0],
                        bg: [0, 0, 0],
                        style_flags: 0,
                        wide: false,
                    }],
                }],
            },
        );

        let texts = snapshot.rows_data[0]
            .iter()
            .map(|cell| cell.text.clone())
            .collect::<Vec<_>>();
        assert_eq!(texts, vec!["a", "b", "X", "d", "e"]);
    }

    #[test]
    fn runtime_state_bootstrap_uses_active_tab() {
        let (mut app, _rx) = ClientApp::new("/tmp/test.sock".to_string());
        let (stream_tx, stream_rx) = std::sync::mpsc::channel();
        app.stream_tx = Some(stream_tx);

        app.handle_stream_event(LocalStreamEvent::UiRuntimeState(control::UiRuntimeState {
            active_tab: 0,
            focused_pane: 7,
            tabs: vec![control::UiTabSnapshot {
                tab_id: 7,
                index: 0,
                active: true,
                title: "shell".to_string(),
                pane_count: 1,
                focused_pane: Some(7),
                pane_ids: vec![7],
            }],
            visible_panes: vec![test_pane_with_id(7, 0.0, 0.0, 80.0, 25.0)],
            mouse_selection: control::UiMouseSelectionSnapshot::default(),
            status_bar: crate::status_components::UiStatusBarSnapshot::default(),
            pwd: "/tmp".to_string(),
            runtime_revision: 1,
            view_revision: 1,
            view_id: 1,
            viewed_tab_id: Some(7),
            viewport_cols: None,
            viewport_rows: None,
            visible_pane_ids: vec![7],
            acked_client_action_id: None,
        }));

        assert_eq!(app.active_remote_tab_id, Some(7));
        assert!(stream_rx.try_recv().is_err());
    }

    #[test]
    fn activate_tab_updates_status_state_and_sends_runtime_action() {
        let (mut app, _) = ClientApp::new("/tmp/test.sock".to_string());
        let (stream_tx, stream_rx) = std::sync::mpsc::channel();
        app.stream_tx = Some(stream_tx);
        app.runtime_view_id = 44;
        app.ui_state.tabs = vec![
            ClientTabState {
                index: 0,
                tab_id: Some(7),
                active: true,
                title: String::new(),
                pane_count: 1,
            },
            ClientTabState {
                index: 1,
                tab_id: Some(8),
                active: false,
                title: String::new(),
                pane_count: 1,
            },
        ];

        app.activate_tab(8);

        assert_eq!(app.active_remote_tab_id, Some(8));
        assert_eq!(app.ui_state.active_tab, 1);
        assert!(!app.ui_state.tabs[0].active);
        assert!(app.ui_state.tabs[1].active);
        match stream_rx.recv().expect("runtime action") {
            StreamCommand::RuntimeAction {
                action: remote::RuntimeAction::SetViewedTab { view_id, tab_id },
            } => {
                assert_eq!(view_id, 44);
                assert_eq!(tab_id, 8);
            }
            other => panic!("unexpected stream command: {other:?}"),
        }
    }

    #[test]
    fn stale_pane_delta_is_rejected_by_runtime_revision() {
        let (mut app, _) = ClientApp::new("/tmp/test.sock".to_string());
        app.apply_ui_runtime_state(control::UiRuntimeState {
            active_tab: 0,
            focused_pane: 7,
            tabs: vec![control::UiTabSnapshot {
                tab_id: 7,
                index: 0,
                active: true,
                title: "shell".to_string(),
                pane_count: 1,
                focused_pane: Some(7),
                pane_ids: vec![7],
            }],
            visible_panes: vec![test_pane_with_id(7, 0.0, 0.0, 80.0, 25.0)],
            mouse_selection: control::UiMouseSelectionSnapshot::default(),
            status_bar: crate::status_components::UiStatusBarSnapshot::default(),
            pwd: "/tmp".to_string(),
            runtime_revision: 3,
            view_revision: 1,
            view_id: 1,
            viewed_tab_id: Some(7),
            viewport_cols: None,
            viewport_rows: None,
            visible_pane_ids: vec![7],
            acked_client_action_id: None,
        });
        app.handle_stream_event(LocalStreamEvent::UiPaneFullState {
            pane_id: 7,
            runtime_revision: 3,
            state: remote::RemoteFullState {
                epoch: 0,
                viewport_top: 0,
                scrollback_total: 0,
                rows: 1,
                cols: 1,
                cursor_x: 0,
                cursor_y: 0,
                cursor_visible: true,
                cursor_blinking: false,
                cursor_style: 0,
                cells: vec![remote::RemoteCell {
                    codepoint: u32::from('a'),
                    fg: [0, 0, 0],
                    bg: [0, 0, 0],
                    style_flags: 0,
                    wide: false,
                }],
            },
        });

        app.handle_stream_event(LocalStreamEvent::UiPaneDelta {
            pane_id: 7,
            runtime_revision: 2,
            delta: RemoteDelta {
                cursor_x: 0,
                cursor_y: 0,
                cursor_visible: true,
                cursor_blinking: false,
                cursor_style: 0,
                scroll_rows: 0,
                changed_rows: vec![RemoteRowDelta {
                    row: 0,
                    start_col: 0,
                    cells: vec![remote::RemoteCell {
                        codepoint: u32::from('z'),
                        fg: [0, 0, 0],
                        bg: [0, 0, 0],
                        style_flags: 0,
                        wide: false,
                    }],
                }],
            },
        });

        let snapshot = app.pane_snapshots.get(&7).expect("pane snapshot");
        assert_eq!(snapshot.rows_data[0][0].text, "a");
    }

    #[test]
    fn changed_view_revision_clears_cached_pane_snapshots() {
        let (mut app, _) = ClientApp::new("/tmp/test.sock".to_string());
        app.pane_snapshots.insert(
            7,
            Arc::new(vt_backend_core::TerminalSnapshot {
                cols: 1,
                rows: 1,
                rows_data: vec![vec![vt_backend_core::CellSnapshot {
                    text: "a".to_string(),
                    ..Default::default()
                }]],
                row_revisions: vec![1],
                ..Default::default()
            }),
        );
        app.view_revision = 1;

        app.apply_ui_runtime_state(control::UiRuntimeState {
            active_tab: 0,
            focused_pane: 7,
            tabs: vec![control::UiTabSnapshot {
                tab_id: 7,
                index: 0,
                active: true,
                title: "shell".to_string(),
                pane_count: 1,
                focused_pane: Some(7),
                pane_ids: vec![7],
            }],
            visible_panes: vec![test_pane_with_id(7, 0.0, 0.0, 80.0, 25.0)],
            mouse_selection: control::UiMouseSelectionSnapshot::default(),
            status_bar: crate::status_components::UiStatusBarSnapshot::default(),
            pwd: "/tmp".to_string(),
            runtime_revision: 2,
            view_revision: 2,
            view_id: 1,
            viewed_tab_id: Some(7),
            viewport_cols: None,
            viewport_rows: None,
            visible_pane_ids: vec![7],
            acked_client_action_id: None,
        });

        assert!(app.pane_snapshots.is_empty());
    }

    #[test]
    fn tab_list_bootstrap_does_not_pick_a_target_without_runtime_state() {
        let (mut app, _rx) = ClientApp::new("/tmp/test.sock".to_string());
        let (stream_tx, stream_rx) = std::sync::mpsc::channel();
        app.stream_tx = Some(stream_tx);

        app.handle_stream_event(LocalStreamEvent::TabList(vec![remote::RemoteTabInfo {
            id: 7,
            name: "shell".to_string(),
            title: "shell".to_string(),
            pwd: "/tmp".to_string(),
            active: false,
            child_exited: false,
        }]));

        assert_eq!(app.active_remote_tab_id, None);
        assert!(stream_rx.try_recv().is_err());
        assert!(!app.should_exit);
    }

    #[test]
    fn tab_exit_does_not_recover_a_client_owned_target() {
        let (mut app, _) = ClientApp::new("/tmp/test.sock".to_string());
        let (tx, _rx) = std::sync::mpsc::channel();
        app.stream_tx = Some(tx);
        app.mode = ClientMode::Active;
        app.active_remote_tab_id = Some(7);
        app.should_exit = true;
        app.last_error = Some("stale".to_string());

        app.handle_stream_event(LocalStreamEvent::TabExited);

        assert!(matches!(app.mode, ClientMode::Active));
        assert_eq!(app.active_remote_tab_id, Some(7));
        assert!(app.should_exit);
        assert_eq!(app.last_error.as_deref(), Some("stale"));
    }

    #[test]
    fn disconnect_enters_recovering_without_dropping_active_tab() {
        let (mut app, _) = ClientApp::new("/tmp/test.sock".to_string());
        app.mode = ClientMode::Active;
        app.active_remote_tab_id = Some(7);
        app.should_exit = true;

        app.handle_stream_event(LocalStreamEvent::Disconnected);

        assert!(matches!(app.mode, ClientMode::Recovering));
        assert_eq!(app.active_remote_tab_id, Some(7));
        assert!(!app.should_exit);
        assert_eq!(
            app.last_error.as_deref(),
            Some("boo server stream disconnected")
        );
    }

    #[test]
    fn stream_ready_does_not_emit_legacy_recovery_command() {
        let (mut app, _) = ClientApp::new("/tmp/test.sock".to_string());
        let (tx, rx) = std::sync::mpsc::channel();
        app.mode = ClientMode::Recovering;

        let task = app.update(Message::StreamReady(tx));
        drop(task);

        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn fallback_status_right_shows_bootstrap_state() {
        let ui_state = ClientUiState::default();
        assert_eq!(
            build_status_right(&ui_state, ClientMode::Bootstrapping, None, None, None, None),
            "remote: bootstrapping"
        );
    }

    #[test]
    fn fallback_status_right_shows_recovering_state_and_context() {
        let ui_state = ClientUiState {
            pwd: "/tmp".to_string(),
            pane_count: 2,
            ..ClientUiState::default()
        };
        assert_eq!(
            build_status_right(&ui_state, ClientMode::Recovering, None, None, None, None),
            "remote: recovering  2 panes  /tmp"
        );
    }

    #[test]
    fn fallback_status_right_shows_recovering_active_tab() {
        let ui_state = ClientUiState {
            pwd: "/tmp".to_string(),
            ..ClientUiState::default()
        };
        assert_eq!(
            build_status_right(&ui_state, ClientMode::Recovering, Some(7), None, None, None),
            "remote: recovering tab 7  /tmp"
        );
    }

    #[test]
    fn fallback_status_right_prefers_error_text() {
        let ui_state = ClientUiState {
            pwd: "/work".to_string(),
            ..ClientUiState::default()
        };
        assert_eq!(
            build_status_right(
                &ui_state,
                ClientMode::Recovering,
                None,
                Some("boo server stream disconnected"),
                None,
                None,
            ),
            "remote: error boo server stream disconnected  /work"
        );
    }

    #[test]
    fn fallback_status_right_shows_active_state_and_context() {
        let ui_state = ClientUiState {
            pane_count: 2,
            pwd: "/repo".to_string(),
            ..ClientUiState::default()
        };
        assert_eq!(
            build_status_right(&ui_state, ClientMode::Active, None, None, None, None),
            "remote: connected  2 panes  /repo"
        );
    }

    #[test]
    fn fallback_status_right_includes_remote_host_context() {
        let ui_state = ClientUiState {
            pwd: "/repo".to_string(),
            ..ClientUiState::default()
        };
        assert_eq!(
            build_status_right(
                &ui_state,
                ClientMode::Recovering,
                None,
                Some("boo server stream disconnected"),
                Some("example-mbp.local"),
                None,
            ),
            "remote:example-mbp.local: error boo server stream disconnected  /repo"
        );
    }

    #[test]
    fn fallback_status_right_appends_remote_debug_summary() {
        let ui_state = ClientUiState {
            pwd: "/repo".to_string(),
            ..ClientUiState::default()
        };
        assert_eq!(
            build_status_right(
                &ui_state,
                ClientMode::Active,
                None,
                None,
                Some("example-mbp.local"),
                Some("diag s=1 c=2 v=1 p=1 h=0 r=1"),
            ),
            "remote:example-mbp.local: connected  /repo  diag s=1 c=2 v=1 p=1 h=0 r=1"
        );
    }
}
