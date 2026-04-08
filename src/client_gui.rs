use crate::bindings;
use crate::control;
use crate::AppKeyEvent;
use crate::iced_mods_to_ghostty;
use crate::keymap;
use crate::remote;
use crate::vt;
use crate::vt_backend_core;
use crate::vt_terminal_canvas;
use iced::futures::{SinkExt, StreamExt};
use iced::stream;
use iced::widget::{column, container, row, text};
use iced::window;
use iced::{Color, Element, Event, Font, Length, Size, Subscription, Task, Theme, keyboard, time};
use std::collections::HashMap;
use std::io::Write;
use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixListener;
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

const STATUS_BAR_HEIGHT: f64 = 20.0;
const DEFAULT_FONT_SIZE: f32 = 14.0;
const IDLE_TICK_INTERVAL: Duration = Duration::from_secs(1);
const STREAM_RECONNECT_DELAY: Duration = Duration::from_millis(250);
const PASSIVE_STREAM_BATCH_WINDOW: Duration = Duration::from_millis(24);
const SNAPSHOT_RETRY_TICKS: u8 = 3;
const SNAPSHOT_KEEPALIVE_TICKS: u8 = 30;

#[derive(Debug, Clone)]
pub enum Message {
    Frame,
    FlushPassiveStream,
    IcedEvent(Event),
    StreamReady(std::sync::mpsc::Sender<StreamCommand>),
    StreamEvent(LocalStreamEvent),
    StreamBatch(Vec<LocalStreamEvent>),
    GuiTest(GuiTestCommand),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuiTestCommand {
    Text(String),
    Key(String),
    AppKey(String),
    Command(String),
    Resize { cols: u16, rows: u16 },
    Refresh,
}

#[derive(Debug, Clone, Default)]
struct GuiTestStatus {
    mode: &'static str,
    stream_ready: bool,
    has_terminal: bool,
    active_tab: usize,
    row0_text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClientMode {
    Bootstrapping,
    Attached,
    Recovering,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct ClientUiState {
    tabs: Vec<ClientTabState>,
    active_tab: usize,
    pwd: String,
    pane_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ClientTabState {
    index: usize,
    session_id: Option<u32>,
    active: bool,
    title: String,
    pane_count: usize,
}

pub struct ClientApp {
    socket_path: String,
    client: control::Client,
    stream_tx: Option<std::sync::mpsc::Sender<StreamCommand>>,
    bootstrap_snapshot: Option<control::UiSnapshot>,
    ui_state: ClientUiState,
    mode: ClientMode,
    active_session_id: Option<u32>,
    stream_snapshot: Option<Arc<vt_backend_core::TerminalSnapshot>>,
    last_error: Option<String>,
    cell_width: f64,
    cell_height: f64,
    font_size: f32,
    background_opacity: f32,
    background_opacity_cells: bool,
    tick_counter: u8,
    next_input_seq: u64,
    pending_input_latencies: HashMap<u64, Instant>,
    pending_passive_stream: Option<LocalStreamEvent>,
    passive_flush_scheduled: bool,
    steady_state_snapshot_requests: u64,
    should_exit: bool,
    terminal_snapshot_generation: u64,
    next_full_snapshot_revision: u64,
    next_snapshot_generation: u64,
}

impl ClientApp {
    fn has_paintable_terminal(&self) -> bool {
        self.stream_snapshot.is_some()
            || self
                .bootstrap_snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.terminal.as_ref())
                .is_some()
    }

    fn stream_ready_for_terminal_io(&self) -> bool {
        self.stream_tx.is_some() && matches!(self.mode, ClientMode::Attached)
    }

    pub fn new(socket_path: String) -> (Self, Task<Message>) {
        let client = control::Client::connect(socket_path.clone());
        let snapshot = client.get_ui_snapshot().ok();
        let font_size = snapshot
            .as_ref()
            .map(|snapshot| snapshot.appearance.font_size)
            .unwrap_or(DEFAULT_FONT_SIZE);
        let (cell_width, cell_height) = terminal_metrics(font_size);
        let ui_state = snapshot
            .as_ref()
            .map(ClientUiState::from_snapshot)
            .unwrap_or_default();
        let app = Self {
                socket_path,
                client,
                stream_tx: None,
                bootstrap_snapshot: snapshot,
                ui_state,
                mode: ClientMode::Bootstrapping,
                active_session_id: None,
                stream_snapshot: None,
                last_error: None,
                cell_width,
                cell_height,
                font_size,
                background_opacity: 1.0,
                background_opacity_cells: false,
                tick_counter: 0,
                next_input_seq: 1,
                pending_input_latencies: HashMap::new(),
                pending_passive_stream: None,
                passive_flush_scheduled: false,
                steady_state_snapshot_requests: 0,
                should_exit: false,
                terminal_snapshot_generation: 1,
                next_full_snapshot_revision: 1,
                next_snapshot_generation: 2,
            };
        app.update_gui_test_status();
        (app, Task::none())
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        let mut tasks = Vec::new();
        match message {
            Message::Frame => self.on_tick(),
            Message::FlushPassiveStream => {
                self.passive_flush_scheduled = false;
                self.flush_pending_passive_stream();
            }
            Message::StreamReady(tx) => {
                self.stream_tx = Some(tx);
                self.send_stream_command(StreamCommand::ListSessions);
            }
            Message::StreamEvent(event) => {
                if let Some(task) = self.handle_stream_delivery(event) {
                    tasks.push(task);
                }
            }
            Message::StreamBatch(events) => {
                for event in events {
                    if let Some(task) = self.handle_stream_delivery(event) {
                        tasks.push(task);
                    }
                }
            }
            Message::GuiTest(command) => self.handle_gui_test(command),
            Message::IcedEvent(event) => match event {
                Event::Window(window::Event::Resized(size)) => {
                    self.send_resize(size);
                }
                Event::Keyboard(event) => self.handle_keyboard(event),
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

        if let Some(stream_snapshot) = self.stream_snapshot.as_ref() {
            let terminal_canvas = vt_terminal_canvas::TerminalCanvas::new(
                Arc::clone(stream_snapshot),
                self.cell_width as f32,
                self.cell_height as f32,
                self.font_size,
                None,
                self.terminal_snapshot_generation,
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
        } else if let Some(snapshot) = self.bootstrap_snapshot.as_ref() {
            if let Some(terminal) = snapshot.terminal.as_ref() {
                let terminal_canvas = vt_terminal_canvas::TerminalCanvas::new(
                    Arc::new(ui_terminal_to_vt_snapshot(terminal)),
                    self.cell_width as f32,
                    self.cell_height as f32,
                    self.font_size,
                    None,
                    self.terminal_snapshot_generation,
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

        let (left, right) = build_status(&self.ui_state);
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

        container(main_col)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    pub fn theme(&self) -> Theme {
        Theme::Dark
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let mut subscriptions = vec![
            iced::event::listen().map(Message::IcedEvent),
            iced::Subscription::run_with(self.socket_path.clone(), local_stream_subscription),
            gui_test_subscription(),
        ];
        if !matches!(self.mode, ClientMode::Attached)
            && self.stream_tx.is_none()
            && !self.has_paintable_terminal()
        {
            subscriptions.push(time::every(IDLE_TICK_INTERVAL).map(|_| Message::Frame));
        }
        Subscription::batch(subscriptions)
    }

    pub fn window_style(&self) -> iced::theme::Style {
        iced::theme::Style {
            background_color: Color::TRANSPARENT,
            text_color: Color::WHITE,
        }
    }

    fn refresh_snapshot(&mut self) {
        let _scope =
            crate::profiling::scope("client.control.get_ui_snapshot", crate::profiling::Kind::Io);
        match self.client.get_ui_snapshot() {
            Ok(snapshot) => {
                if matches!(self.mode, ClientMode::Attached) {
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
                self.font_size = snapshot.appearance.font_size.max(8.0);
                self.background_opacity = snapshot.appearance.background_opacity;
                self.background_opacity_cells = snapshot.appearance.background_opacity_cells;
                (self.cell_width, self.cell_height) = terminal_metrics(self.font_size);
                self.ui_state = ClientUiState::from_snapshot(&snapshot);
                self.bootstrap_snapshot = Some(snapshot);
                self.terminal_snapshot_generation = self.allocate_snapshot_generation();
                self.last_error = None;
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

    fn on_tick(&mut self) {
        if matches!(self.mode, ClientMode::Attached)
            || self.stream_tx.is_some()
            || self.has_paintable_terminal()
        {
            return;
        }
        self.tick_counter = self.tick_counter.wrapping_add(1);
        let refresh_ticks = if !self.has_paintable_terminal() || self.stream_tx.is_none() {
            SNAPSHOT_RETRY_TICKS
        } else {
            SNAPSHOT_KEEPALIVE_TICKS
        };
        if self.tick_counter >= refresh_ticks {
            self.tick_counter = 0;
            self.refresh_snapshot();
        }
    }

    fn send_resize(&mut self, size: Size) {
        let cols = ((size.width as f64 / self.cell_width).floor() as u16).max(2);
        let rows = (((size.height as f64 - STATUS_BAR_HEIGHT).max(1.0) / self.cell_height).floor()
            as u16)
            .max(1);
        if self.stream_ready_for_terminal_io() {
            self.send_stream_command(StreamCommand::Resize { cols, rows });
        } else {
            let _ = self
                .client
                .send(&control::Request::ResizeFocused { cols, rows });
            self.refresh_snapshot();
        }
    }

    fn handle_keyboard(&mut self, event: keyboard::Event) {
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
            return;
        };
        let input_seq = self.record_pending_input();
        let app_event = AppKeyEvent {
            keycode,
            mods: iced_mods_to_ghostty(&modifiers),
            text: text.as_ref().map(ToString::to_string).filter(|text| !text.is_empty()),
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
            control::Request::AppKeyEvent {
                event: app_event,
            },
        );
    }

    fn send_stream_or_control(&mut self, stream: StreamCommand, control: control::Request) {
        if self.stream_ready_for_terminal_io() {
            self.send_stream_command(stream);
        } else {
            let _ = self.client.send(&control);
            self.refresh_snapshot();
        }
    }

    fn handle_stream_event(&mut self, event: LocalStreamEvent) {
        match event {
            LocalStreamEvent::SessionList(sessions) => {
                let live_sessions: Vec<_> = sessions
                    .iter()
                    .filter(|session| !session.child_exited)
                    .collect();
                self.apply_remote_sessions(&sessions);
                if let Some(session_id) =
                    session_list_attach_target(self.mode, self.active_session_id, &live_sessions)
                {
                    self.should_exit = false;
                    self.send_stream_command(StreamCommand::Attach(session_id));
                } else if matches!(self.mode, ClientMode::Attached)
                    && self
                        .active_session_id
                        .map(|session_id| live_sessions.iter().any(|session| session.id == session_id))
                        .unwrap_or(false)
                {
                    self.should_exit = false;
                } else if matches!(self.mode, ClientMode::Bootstrapping)
                    && !self.has_paintable_terminal()
                {
                    let _ = self.client.send(&control::Request::NewTab);
                    self.send_stream_command(StreamCommand::ListSessions);
                } else {
                    self.should_exit = true;
                }
            }
            LocalStreamEvent::Attached(session_id) => {
                self.active_session_id = Some(session_id);
                self.ui_state.mark_active_session(Some(session_id));
            }
            LocalStreamEvent::Detached => {
                self.mode = ClientMode::Recovering;
                self.active_session_id = None;
                self.pending_input_latencies.clear();
                self.send_stream_command(StreamCommand::ListSessions);
            }
            LocalStreamEvent::SessionExited(session_id) => {
                if self.active_session_id == Some(session_id) {
                    self.active_session_id = None;
                }
                self.mode = ClientMode::Recovering;
                self.should_exit = false;
                self.last_error = None;
                self.pending_input_latencies.clear();
                self.send_stream_command(StreamCommand::ListSessions);
            }
            LocalStreamEvent::Disconnected => {
                self.stream_tx = None;
                let had_paintable_terminal = self.has_paintable_terminal();
                let lost_active_session = self.active_session_id.take();
                self.mode = ClientMode::Recovering;
                self.pending_input_latencies.clear();
                if should_exit_after_stream_disconnect(lost_active_session, had_paintable_terminal)
                {
                    self.should_exit = true;
                    self.last_error = None;
                } else {
                    self.last_error = Some("boo server stream disconnected".to_string());
                }
            }
            LocalStreamEvent::FullState {
                ack_input_seq,
                state,
            } => {
                let _scope = crate::profiling::scope(
                    "client.stream.apply_full",
                    crate::profiling::Kind::Cpu,
                );
                self.mode = ClientMode::Attached;
                let revision_seed = self.allocate_full_snapshot_revision_seed(state.rows as usize);
                self.terminal_snapshot_generation = self.allocate_snapshot_generation();
                self.stream_snapshot = Some(Arc::new(remote_full_state_to_vt_snapshot(
                    &state,
                    revision_seed,
                )));
                self.bootstrap_snapshot = None;
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
                if let Some(snapshot) = self.stream_snapshot.as_mut() {
                    apply_remote_delta_snapshot(Arc::make_mut(snapshot), &delta);
                }
                self.acknowledge_input_latency("stream_delta", ack_input_seq);
            }
            LocalStreamEvent::Error(error) => {
                self.last_error = Some(error);
            }
        }
    }

    fn flush_pending_passive_stream(&mut self) {
        if let Some(event) = self.pending_passive_stream.take() {
            self.handle_stream_event(event);
        }
    }

    fn handle_stream_delivery(&mut self, event: LocalStreamEvent) -> Option<Task<Message>> {
        if is_passive_screen_event(&event) && matches!(self.mode, ClientMode::Attached) {
            match self.pending_passive_stream.take() {
                Some(pending) if passive_screen_event_supersedes(&pending, &event) => {
                    self.pending_passive_stream = Some(event);
                }
                Some(pending) => {
                    self.handle_stream_event(pending);
                    self.pending_passive_stream = Some(event);
                }
                None => self.pending_passive_stream = Some(event),
            }
            if !self.passive_flush_scheduled {
                self.passive_flush_scheduled = true;
                return Some(Task::perform(
                    async move {
                        std::thread::sleep(PASSIVE_STREAM_BATCH_WINDOW);
                    },
                    |_| Message::FlushPassiveStream,
                ));
            }
            return None;
        }

        if matches!(
            event,
            LocalStreamEvent::FullState { .. } | LocalStreamEvent::Delta { .. }
        ) {
            self.pending_passive_stream = None;
        } else {
            self.flush_pending_passive_stream();
        }
        self.handle_stream_event(event);
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
                    self.refresh_snapshot();
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
                    self.refresh_snapshot();
                }
            }
            GuiTestCommand::AppKey(keyspec) => {
                if let Some(event) = gui_test_app_key_event(&keyspec, self.record_pending_input()) {
                    self.send_stream_or_control(
                        StreamCommand::AppKeyEvent {
                            event: event.clone(),
                        },
                        control::Request::AppKeyEvent { event },
                    );
                }
            }
            GuiTestCommand::Command(input) => self.send_stream_or_control(
                StreamCommand::ExecuteCommand {
                    input: input.clone(),
                },
                control::Request::ExecuteCommand { input },
            ),
            GuiTestCommand::Resize { cols, rows } => self.send_resize_cells(cols, rows),
            GuiTestCommand::Refresh => self.refresh_snapshot(),
        }
    }

    fn send_stream_command(&self, command: StreamCommand) {
        if let Some(tx) = self.stream_tx.as_ref() {
            let _ = tx.send(command);
        }
    }

    fn send_resize_cells(&mut self, cols: u16, rows: u16) {
        if self.stream_ready_for_terminal_io() {
            self.send_stream_command(StreamCommand::Resize { cols, rows });
        } else {
            let _ = self
                .client
                .send(&control::Request::ResizeFocused { cols, rows });
            self.refresh_snapshot();
        }
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
        let mut completed = None;
        self.pending_input_latencies.retain(|seq, started_at| {
            if *seq <= ack_input_seq {
                completed = Some((*seq, *started_at));
                false
            } else {
                true
            }
        });
        if let Some((input_seq, started_at)) = completed {
            log_client_latency(stage, input_seq, started_at);
        }
    }

    fn apply_remote_sessions(&mut self, sessions: &[remote::RemoteSessionInfo]) {
        self.ui_state = ClientUiState::from_remote_sessions(sessions, self.active_session_id);
    }

    fn update_gui_test_status(&self) {
        let status_value = GuiTestStatus {
            mode: match self.mode {
                ClientMode::Bootstrapping => "bootstrapping",
                ClientMode::Attached => "attached",
                ClientMode::Recovering => "recovering",
            },
            stream_ready: self.stream_ready_for_terminal_io(),
            has_terminal: self.has_paintable_terminal(),
            active_tab: self.ui_state.active_tab,
            row0_text: self
                .stream_snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.rows_data.first())
                .map(|row| row.iter().map(|cell| cell.text.as_str()).collect::<String>())
                .or_else(|| {
                    self.bootstrap_snapshot
                        .as_ref()
                        .and_then(|snapshot| snapshot.terminal.as_ref())
                        .and_then(|terminal| terminal.rows_data.first())
                        .map(|row| {
                            row.cells
                                .iter()
                                .map(|cell| cell.text.as_str())
                                .collect::<String>()
                        })
                })
                .unwrap_or_default(),
        };
        if let Some(status) = gui_test_status_handle()
            && let Ok(mut guard) = status.lock()
        {
            *guard = status_value.clone();
        }
        if let Some(path) = gui_test_status_path() {
            let line = format!(
                "mode={} stream_ready={} has_terminal={} active_tab={} row0={:?}\n",
                status_value.mode,
                u8::from(status_value.stream_ready),
                u8::from(status_value.has_terminal),
                status_value.active_tab,
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

fn should_exit_after_stream_disconnect(
    lost_active_session: Option<u32>,
    had_paintable_terminal: bool,
) -> bool {
    lost_active_session.is_some() || had_paintable_terminal
}

fn session_list_attach_target(
    mode: ClientMode,
    active_session_id: Option<u32>,
    live_sessions: &[&remote::RemoteSessionInfo],
) -> Option<u32> {
    if matches!(mode, ClientMode::Attached)
        && active_session_id
            .map(|session_id| live_sessions.iter().any(|session| session.id == session_id))
            .unwrap_or(false)
    {
        return None;
    }

    active_session_id
        .and_then(|session_id| {
            live_sessions
                .iter()
                .copied()
                .find(|session| session.id == session_id)
        })
        .or_else(|| live_sessions.first().copied())
        .map(|session| session.id)
}

impl ClientUiState {
    fn from_snapshot(snapshot: &control::UiSnapshot) -> Self {
        Self {
            tabs: snapshot
                .tabs
                .iter()
                .map(|tab| ClientTabState {
                    index: tab.index,
                    session_id: None,
                    active: tab.active,
                    title: tab.title.clone(),
                    pane_count: tab.pane_count,
                })
                .collect(),
            active_tab: snapshot.active_tab,
            pwd: snapshot.pwd.clone(),
            pane_count: snapshot.visible_panes.len(),
        }
    }

    fn from_remote_sessions(
        sessions: &[remote::RemoteSessionInfo],
        active_session_id: Option<u32>,
    ) -> Self {
        let active_index = active_session_id
            .and_then(|session_id| sessions.iter().position(|session| session.id == session_id))
            .or_else(|| sessions.iter().position(|session| session.attached))
            .unwrap_or(0);
        let tabs = sessions
            .iter()
            .enumerate()
            .map(|(index, session)| ClientTabState {
                index,
                session_id: Some(session.id),
                active: index == active_index,
                title: if session.title.is_empty() {
                    session.name.clone()
                } else {
                    session.title.clone()
                },
                pane_count: 1,
            })
            .collect::<Vec<_>>();
        let pwd = sessions
            .get(active_index)
            .map(|session| session.pwd.clone())
            .unwrap_or_default();
        Self {
            tabs,
            active_tab: active_index.min(sessions.len().saturating_sub(1)),
            pwd,
            pane_count: usize::from(!sessions.is_empty()),
        }
    }

    fn mark_active_session(&mut self, session_id: Option<u32>) {
        let Some(session_id) = session_id else {
            return;
        };
        if let Some(index) = self
            .tabs
            .iter()
            .position(|tab| tab.session_id == Some(session_id))
        {
            self.active_tab = index;
        }
        for (index, tab) in self.tabs.iter_mut().enumerate() {
            tab.active = index == self.active_tab;
        }
    }
}

fn terminal_metrics(font_size: f32) -> (f64, f64) {
    let size = font_size.max(8.0) as f64;
    let cell_width = (size * 0.62).max(6.0).ceil();
    let cell_height = (size * 1.35).max(12.0).ceil();
    (cell_width, cell_height)
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

fn remote_full_state_to_vt_snapshot(
    state: &remote::RemoteFullState,
    revision_seed: u64,
) -> vt_backend_core::TerminalSnapshot {
    let cols = state.cols as usize;
    let rows_data = state
        .cells
        .chunks(cols.max(1))
        .map(|row| row.iter().map(remote_cell_to_snapshot).collect())
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
            x: state.cursor_x,
            y: state.cursor_y,
            style: 1,
        },
        rows_data,
        row_revisions,
        scrollbar: Default::default(),
        colors: vt::GhosttyRenderStateColors {
            foreground: vt::GhosttyColorRgb {
                r: 0xf0,
                g: 0xf0,
                b: 0xf0,
            },
            background: vt::GhosttyColorRgb { r: 0, g: 0, b: 0 },
            cursor: vt::GhosttyColorRgb {
                r: 0xff,
                g: 0xff,
                b: 0xff,
            },
            cursor_has_value: true,
            ..Default::default()
        },
    }
}

fn ui_terminal_to_vt_snapshot(
    snapshot: &control::UiTerminalSnapshot,
) -> vt_backend_core::TerminalSnapshot {
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
        row_revisions: vec![1; snapshot.rows_data.len()],
        scrollbar: Default::default(),
        colors: vt::GhosttyRenderStateColors {
            foreground: vt::GhosttyColorRgb {
                r: 0xf0,
                g: 0xf0,
                b: 0xf0,
            },
            background: vt::GhosttyColorRgb { r: 0, g: 0, b: 0 },
            cursor: vt::GhosttyColorRgb {
                r: 0xff,
                g: 0xff,
                b: 0xff,
            },
            cursor_has_value: true,
            ..Default::default()
        },
    }
}

#[derive(Clone, Debug)]
pub(crate) enum LocalStreamEvent {
    SessionList(Vec<remote::RemoteSessionInfo>),
    Attached(u32),
    Detached,
    SessionExited(u32),
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
    scroll_rows: i16,
    changed_rows: Vec<RemoteRowDelta>,
}

fn stream_batch_window_for_event(event: &LocalStreamEvent) -> Option<Duration> {
    match event {
        LocalStreamEvent::FullState { ack_input_seq, .. }
        | LocalStreamEvent::Delta { ack_input_seq, .. } => {
            if ack_input_seq.is_none() {
                Some(PASSIVE_STREAM_BATCH_WINDOW)
            } else {
                None
            }
        }
        _ => None,
    }
}

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

fn push_coalesced_stream_event(
    batch: &mut Vec<LocalStreamEvent>,
    pending_passive_screen: &mut Option<LocalStreamEvent>,
    event: LocalStreamEvent,
) {
    if is_passive_screen_event(&event) {
        match pending_passive_screen.take() {
            Some(pending) if passive_screen_event_supersedes(&pending, &event) => {
                *pending_passive_screen = Some(event);
            }
            Some(pending) => {
                batch.push(pending);
                *pending_passive_screen = Some(event);
            }
            None => *pending_passive_screen = Some(event),
        }
        return;
    }
    if let Some(pending) = pending_passive_screen.take() {
        batch.push(pending);
    }
    batch.push(event);
}

fn flush_pending_passive_stream_event(
    batch: &mut Vec<LocalStreamEvent>,
    pending_passive_screen: &mut Option<LocalStreamEvent>,
) {
    if let Some(pending) = pending_passive_screen.take() {
        batch.push(pending);
    }
}

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
    ListSessions,
    Attach(u32),
    AppKeyEvent { event: AppKeyEvent },
    ExecuteCommand { input: String },
    Input { input_seq: u64, bytes: Vec<u8> },
    Key { input_seq: u64, keyspec: String },
    Resize { cols: u16, rows: u16 },
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
                            StreamCommand::ListSessions => write_stream_message(
                                &mut write,
                                remote::MessageType::ListSessions,
                                &[],
                            ),
                            StreamCommand::Attach(session_id) => write_stream_message(
                                &mut write,
                                remote::MessageType::Attach,
                                &session_id.to_le_bytes(),
                            ),
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
                            StreamCommand::Resize { cols, rows } => {
                                let mut payload = Vec::with_capacity(4);
                                payload.extend_from_slice(&cols.to_le_bytes());
                                payload.extend_from_slice(&rows.to_le_bytes());
                                write_stream_message(
                                    &mut write,
                                    remote::MessageType::Resize,
                                    &payload,
                                )
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
                    if let Some(batch_window) = stream_batch_window_for_event(&event) {
                        std::thread::sleep(batch_window);
                    }
                    let mut batch = Vec::new();
                    let mut pending_passive_screen = None;
                    push_coalesced_stream_event(&mut batch, &mut pending_passive_screen, event);
                    let mut saw_disconnect = matches!(
                        batch.last().or(pending_passive_screen.as_ref()),
                        Some(LocalStreamEvent::Disconnected)
                    );
                    while batch.len() < 64 {
                        match event_rx.try_recv() {
                            Ok(event) => {
                                saw_disconnect |= matches!(event, LocalStreamEvent::Disconnected);
                                push_coalesced_stream_event(
                                    &mut batch,
                                    &mut pending_passive_screen,
                                    event,
                                );
                                if saw_disconnect {
                                    break;
                                }
                            }
                            Err(_) => break,
                        }
                    }
                    flush_pending_passive_stream_event(&mut batch, &mut pending_passive_screen);
                    let message = if batch.len() == 1 {
                        Message::StreamEvent(batch.pop().expect("single event batch"))
                    } else {
                        Message::StreamBatch(batch)
                    };
                    let _ = output.send(message).await;
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
    if let Some(rest) = trimmed.strip_prefix("appkey ") {
        return Some(GuiTestCommand::AppKey(rest.to_string()));
    }
    if let Some(rest) = trimmed.strip_prefix("command ") {
        return Some(GuiTestCommand::Command(rest.to_string()));
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

fn gui_test_app_key_event(spec: &str, input_seq: u64) -> Option<AppKeyEvent> {
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
        repeat: false,
        input_seq: Some(input_seq),
    })
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
            remote::MessageType::SessionList => {
                decode_remote_session_list(&payload).map(LocalStreamEvent::SessionList)
            }
            remote::MessageType::Attached => decode_u32(&payload).map(LocalStreamEvent::Attached),
            remote::MessageType::Detached => Some(LocalStreamEvent::Detached),
            remote::MessageType::SessionExited => {
                decode_u32(&payload).map(LocalStreamEvent::SessionExited)
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

fn decode_remote_session_list(payload: &[u8]) -> Option<Vec<remote::RemoteSessionInfo>> {
    if payload.len() < 4 {
        return None;
    }
    let count = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]) as usize;
    let mut offset = 4usize;
    let mut sessions = Vec::with_capacity(count);
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
        sessions.push(remote::RemoteSessionInfo {
            id,
            name,
            title,
            pwd,
            attached: (flags & 0x01) != 0,
            child_exited: (flags & 0x02) != 0,
        });
    }
    Some(sessions)
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
    if payload.len() < 20 {
        return None;
    }
    let ack_input_seq = u64::from_le_bytes(payload[..8].try_into().ok()?);
    let rows = u16::from_le_bytes([payload[8], payload[9]]);
    let cols = u16::from_le_bytes([payload[10], payload[11]]);
    let cursor_x = u16::from_le_bytes([payload[12], payload[13]]);
    let cursor_y = u16::from_le_bytes([payload[14], payload[15]]);
    let cursor_visible = payload[16] != 0;
    let cell_count = rows as usize * cols as usize;
    if payload.len() < 20 + cell_count * 12 {
        return None;
    }
    let mut cells = Vec::with_capacity(cell_count);
    let mut offset = 20usize;
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
            rows,
            cols,
            cursor_x,
            cursor_y,
            cursor_visible,
            cells,
        },
    ))
}

fn decode_remote_delta(payload: &[u8]) -> Option<(Option<u64>, RemoteDelta)> {
    if payload.len() < 16 {
        return None;
    }
    let ack_input_seq = u64::from_le_bytes(payload[..8].try_into().ok()?);
    let row_count = u16::from_le_bytes([payload[8], payload[9]]) as usize;
    let cursor_x = u16::from_le_bytes([payload[10], payload[11]]);
    let cursor_y = u16::from_le_bytes([payload[12], payload[13]]);
    let cursor_visible = payload[14] != 0;
    let flags = payload[15];
    let mut offset = 16usize;
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
            scroll_rows,
            changed_rows,
        },
    ))
}

fn apply_remote_delta_snapshot(
    snapshot: &mut vt_backend_core::TerminalSnapshot,
    delta: &RemoteDelta,
) {
    snapshot.cursor.x = delta.cursor_x;
    snapshot.cursor.y = delta.cursor_y;
    snapshot.cursor.visible = delta.cursor_visible;
    let cols = snapshot.cols as usize;
    if snapshot.row_revisions.len() != snapshot.rows_data.len() {
        snapshot.row_revisions.resize(snapshot.rows_data.len(), 1);
    }
    if delta.scroll_rows != 0 {
        apply_snapshot_scroll(snapshot, delta.scroll_rows);
        for revision in &mut snapshot.row_revisions {
            *revision = revision.wrapping_add(1);
        }
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
            target_row[start_col + offset] = remote_cell_to_snapshot(cell);
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
        snapshot.rows_data.drain(0..shift);
        snapshot.row_revisions.drain(0..shift);
        for _ in 0..shift {
            snapshot.rows_data.push(blank_row());
            snapshot.row_revisions.push(1);
        }
    } else {
        let shift = ((-scroll_rows) as usize).min(rows);
        for _ in 0..shift {
            snapshot.rows_data.insert(0, blank_row());
            snapshot.row_revisions.insert(0, 1);
        }
        snapshot.rows_data.truncate(rows);
        snapshot.row_revisions.truncate(rows);
    }
}

fn remote_cell_to_snapshot(cell: &remote::RemoteCell) -> vt_backend_core::CellSnapshot {
    let default_fg = vt::GhosttyColorRgb {
        r: 0xf0,
        g: 0xf0,
        b: 0xf0,
    };
    let default_bg = vt::GhosttyColorRgb { r: 0, g: 0, b: 0 };
    vt_backend_core::CellSnapshot {
        text: if cell.codepoint == 0 {
            String::new()
        } else {
            std::char::from_u32(cell.codepoint)
                .map(|ch| ch.to_string())
                .unwrap_or_default()
        },
        display_width: if cell.wide { 2 } else { 1 },
        fg: if (cell.style_flags & 0x20) != 0 {
            vt::GhosttyColorRgb {
                r: cell.fg[0],
                g: cell.fg[1],
                b: cell.fg[2],
            }
        } else {
            default_fg
        },
        bg: if (cell.style_flags & 0x40) != 0 {
            vt::GhosttyColorRgb {
                r: cell.bg[0],
                g: cell.bg[1],
                b: cell.bg[2],
            }
        } else {
            default_bg
        },
        bold: (cell.style_flags & 0x01) != 0,
        italic: (cell.style_flags & 0x02) != 0,
        underline: 0,
    }
}

fn build_status(ui_state: &ClientUiState) -> (String, String) {
    let left = ui_state
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
    if ui_state.pane_count > 1 {
        right_parts.push(format!("{} panes", ui_state.pane_count));
    }
    if !ui_state.pwd.is_empty() {
        right_parts.push(ui_state.pwd.clone());
    }
    (left, right_parts.join("  "))
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
        let snapshot = remote_cell_to_snapshot(&cell);
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
        let snapshot = remote_cell_to_snapshot(&cell);
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
    fn parse_gui_test_command_command() {
        assert_eq!(
            parse_gui_test_command("command new-tab"),
            Some(GuiTestCommand::Command("new-tab".to_string()))
        );
    }

    #[test]
    fn parse_gui_test_appkey_command() {
        assert_eq!(
            parse_gui_test_command("appkey shift+0x27"),
            Some(GuiTestCommand::AppKey("shift+0x27".to_string()))
        );
    }

    #[test]
    fn handle_keyboard_sends_raw_app_key_event_over_stream() {
        let (mut app, _) = ClientApp::new("/tmp/boo-test.sock".to_string());
        let (tx, rx) = std::sync::mpsc::channel();
        app.stream_tx = Some(tx);
        app.mode = ClientMode::Attached;

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
                assert_eq!(event.keycode, 0x27);
                assert_eq!(event.text.as_deref(), Some("\""));
                assert_eq!(event.modified_text.as_deref(), Some("\""));
                assert_eq!(event.named_key, None);
            }
            other => panic!("unexpected stream command: {other:?}"),
        }
    }

    #[test]
    fn stream_batch_window_only_applies_to_unacked_screen_updates() {
        assert_eq!(
            stream_batch_window_for_event(&LocalStreamEvent::Delta {
                ack_input_seq: None,
                delta: RemoteDelta {
                    cursor_x: 0,
                    cursor_y: 0,
                    cursor_visible: true,
                    scroll_rows: 0,
                    changed_rows: Vec::new(),
                },
            }),
            Some(PASSIVE_STREAM_BATCH_WINDOW)
        );
        assert_eq!(
            stream_batch_window_for_event(&LocalStreamEvent::Delta {
                ack_input_seq: Some(1),
                delta: RemoteDelta {
                    cursor_x: 0,
                    cursor_y: 0,
                    cursor_visible: true,
                    scroll_rows: 0,
                    changed_rows: Vec::new(),
                },
            }),
            None
        );
        assert_eq!(
            stream_batch_window_for_event(&LocalStreamEvent::SessionList(Vec::new())),
            None
        );
    }

    #[test]
    fn coalesces_only_superseded_passive_screen_updates() {
        let passive_a = LocalStreamEvent::Delta {
            ack_input_seq: None,
            delta: RemoteDelta {
                cursor_x: 0,
                cursor_y: 0,
                cursor_visible: true,
                scroll_rows: 0,
                changed_rows: Vec::new(),
            },
        };
        let passive_b = LocalStreamEvent::FullState {
            ack_input_seq: None,
            state: remote::RemoteFullState {
                rows: 1,
                cols: 1,
                cursor_x: 0,
                cursor_y: 0,
                cursor_visible: true,
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
                scroll_rows: 0,
                changed_rows: Vec::new(),
            },
        };
        let barrier = LocalStreamEvent::Attached(7);

        let mut batch = Vec::new();
        let mut pending = None;
        push_coalesced_stream_event(&mut batch, &mut pending, passive_a);
        push_coalesced_stream_event(&mut batch, &mut pending, passive_b);
        push_coalesced_stream_event(&mut batch, &mut pending, acked.clone());
        push_coalesced_stream_event(&mut batch, &mut pending, barrier.clone());
        flush_pending_passive_stream_event(&mut batch, &mut pending);

        assert_eq!(batch.len(), 3);
        assert!(matches!(batch[0], LocalStreamEvent::FullState { ack_input_seq: None, .. }));
        assert!(matches!(batch[1], LocalStreamEvent::Delta { ack_input_seq: Some(1), .. }));
        assert!(matches!(batch[2], LocalStreamEvent::Attached(7)));
    }

    #[test]
    fn preserves_consecutive_passive_deltas_in_order() {
        let passive_a = LocalStreamEvent::Delta {
            ack_input_seq: None,
            delta: RemoteDelta {
                cursor_x: 0,
                cursor_y: 0,
                cursor_visible: true,
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
                scroll_rows: 0,
                changed_rows: vec![RemoteRowDelta {
                    row: 1,
                    start_col: 0,
                    cells: Vec::new(),
                }],
            },
        };

        let mut batch = Vec::new();
        let mut pending = None;
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
    fn apply_remote_delta_scrolls_snapshot_before_replacing_changed_rows() {
        let mut snapshot = vt_backend_core::TerminalSnapshot {
            cols: 1,
            rows: 3,
            cursor: vt_backend_core::CursorSnapshot {
                visible: true,
                x: 0,
                y: 2,
                style: 0,
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
    fn remote_blank_cell_stays_empty() {
        let snapshot = remote_cell_to_snapshot(&remote::RemoteCell {
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
    fn disconnect_exits_when_active_session_is_gone() {
        assert!(should_exit_after_stream_disconnect(Some(7), false));
        assert!(should_exit_after_stream_disconnect(Some(7), true));
        assert!(should_exit_after_stream_disconnect(None, true));
        assert!(!should_exit_after_stream_disconnect(None, false));
    }

    #[test]
    fn attached_mode_session_list_does_not_force_reattach_to_existing_active_session() {
        let sessions = vec![
            remote::RemoteSessionInfo {
                id: 7,
                name: "one".to_string(),
                title: "".to_string(),
                pwd: "".to_string(),
                attached: true,
                child_exited: false,
            },
            remote::RemoteSessionInfo {
                id: 8,
                name: "two".to_string(),
                title: "".to_string(),
                pwd: "".to_string(),
                attached: false,
                child_exited: false,
            },
        ];
        let live = sessions.iter().collect::<Vec<_>>();
        assert_eq!(
            session_list_attach_target(ClientMode::Attached, Some(7), &live),
            None
        );
    }

    #[test]
    fn recovering_mode_session_list_reattaches_to_existing_active_session() {
        let sessions = vec![
            remote::RemoteSessionInfo {
                id: 7,
                name: "one".to_string(),
                title: "".to_string(),
                pwd: "".to_string(),
                attached: true,
                child_exited: false,
            },
            remote::RemoteSessionInfo {
                id: 8,
                name: "two".to_string(),
                title: "".to_string(),
                pwd: "".to_string(),
                attached: false,
                child_exited: false,
            },
        ];
        let live = sessions.iter().collect::<Vec<_>>();
        assert_eq!(
            session_list_attach_target(ClientMode::Recovering, Some(7), &live),
            Some(7)
        );
    }

    #[test]
    fn session_exit_relists_sessions_instead_of_immediately_exiting() {
        let (mut app, _) = ClientApp::new("/tmp/test.sock".to_string());
        let (tx, _rx) = std::sync::mpsc::channel();
        app.stream_tx = Some(tx);
        app.mode = ClientMode::Attached;
        app.active_session_id = Some(7);
        app.should_exit = true;
        app.last_error = Some("stale".to_string());

        app.handle_stream_event(LocalStreamEvent::SessionExited(7));

        assert!(matches!(app.mode, ClientMode::Recovering));
        assert_eq!(app.active_session_id, None);
        assert!(!app.should_exit);
        assert_eq!(app.last_error, None);
    }
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
