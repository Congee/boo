use crate::control;
use crate::remote;
use crate::vt;
use crate::vt_backend_core;
use crate::vt_terminal_canvas;
use iced::futures::{SinkExt, StreamExt};
use iced::stream;
use iced::widget::{column, container, row, text};
use iced::window;
use iced::{keyboard, time, Color, Element, Event, Font, Length, Size, Subscription, Task, Theme};
use std::collections::HashMap;
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::time::{Duration, Instant};

const STATUS_BAR_HEIGHT: f64 = 20.0;
const DEFAULT_FONT_SIZE: f32 = 14.0;
const IDLE_TICK_INTERVAL: Duration = Duration::from_secs(1);
const SNAPSHOT_RETRY_TICKS: u8 = 3;
const SNAPSHOT_KEEPALIVE_TICKS: u8 = 30;

#[derive(Debug, Clone)]
pub enum Message {
    Frame,
    IcedEvent(Event),
    StreamReady(std::sync::mpsc::Sender<StreamCommand>),
    StreamEvent(LocalStreamEvent),
}

pub struct ClientApp {
    socket_path: String,
    client: control::Client,
    stream_tx: Option<std::sync::mpsc::Sender<StreamCommand>>,
    snapshot: Option<control::UiSnapshot>,
    stream_state: Option<remote::RemoteFullState>,
    stream_snapshot: Option<Arc<vt_backend_core::TerminalSnapshot>>,
    last_error: Option<String>,
    cell_width: f64,
    cell_height: f64,
    font_size: f32,
    background_opacity: f32,
    background_opacity_cells: bool,
    tick_counter: u8,
    active_tab_index: usize,
    render_revision: u64,
    next_input_seq: u64,
    pending_input_latencies: HashMap<u64, Instant>,
    should_exit: bool,
}

impl ClientApp {
    fn stream_ready_for_terminal_io(&self) -> bool {
        self.stream_tx.is_some() && self.stream_state.is_some()
    }

    pub fn new(socket_path: String) -> (Self, Task<Message>) {
        let client = control::Client::connect(socket_path.clone());
        let snapshot = client.get_ui_snapshot().ok();
        let active_tab_index = snapshot.as_ref().map(|snapshot| snapshot.active_tab).unwrap_or(0);
        let font_size = snapshot
            .as_ref()
            .map(|snapshot| snapshot.appearance.font_size)
            .unwrap_or(DEFAULT_FONT_SIZE);
        let (cell_width, cell_height) = terminal_metrics(font_size);
        (
            Self {
                socket_path,
                client,
                stream_tx: None,
                snapshot,
                stream_state: None,
                stream_snapshot: None,
                last_error: None,
                cell_width,
                cell_height,
                font_size,
                background_opacity: 1.0,
                background_opacity_cells: false,
                tick_counter: 0,
                active_tab_index,
                render_revision: 1,
                next_input_seq: 1,
                pending_input_latencies: HashMap::new(),
                should_exit: false,
            },
            Task::none(),
        )
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Frame => self.on_tick(),
            Message::StreamReady(tx) => {
                self.stream_tx = Some(tx);
                self.send_stream_command(StreamCommand::ListSessions);
            }
            Message::StreamEvent(event) => self.handle_stream_event(event),
            Message::IcedEvent(event) => match event {
                Event::Window(window::Event::Resized(size)) => {
                    self.send_resize(size);
                }
                Event::Keyboard(event) => self.handle_keyboard(event),
                _ => {}
            },
        }
        if self.should_exit {
            iced::exit()
        } else {
            Task::none()
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        let mut main_col = column![];

        if let Some(snapshot) = self.snapshot.as_ref() {
            if let Some(stream_snapshot) = self.stream_snapshot.as_ref() {
                let terminal_canvas = vt_terminal_canvas::TerminalCanvas::new(
                    Arc::clone(stream_snapshot),
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
            } else if let Some(terminal) = snapshot.terminal.as_ref() {
                let terminal_canvas = vt_terminal_canvas::TerminalCanvas::new(
                    Arc::new(ui_terminal_to_vt_snapshot(terminal)),
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
            time::every(IDLE_TICK_INTERVAL).map(|_| Message::Frame),
            iced::event::listen().map(Message::IcedEvent),
            iced::Subscription::run_with(self.socket_path.clone(), local_stream_subscription),
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
                self.active_tab_index = snapshot.active_tab;
                self.font_size = snapshot.appearance.font_size.max(8.0);
                self.background_opacity = snapshot.appearance.background_opacity;
                self.background_opacity_cells = snapshot.appearance.background_opacity_cells;
                (self.cell_width, self.cell_height) = terminal_metrics(self.font_size);
                self.snapshot = Some(snapshot);
                self.render_revision = self.render_revision.wrapping_add(1);
                self.last_error = None;
            }
            Err(error) => {
                self.last_error = Some(error);
            }
        }
    }

    fn on_tick(&mut self) {
        self.tick_counter = self.tick_counter.wrapping_add(1);
        let refresh_ticks = if self.snapshot.is_none() || self.stream_tx.is_none() {
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
            let _ = self.client.send(&control::Request::ResizeFocused { cols, rows });
        }
    }

    fn handle_keyboard(&mut self, event: keyboard::Event) {
        let keyboard::Event::KeyPressed { key, text, modifiers, .. } = event else {
            return;
        };

        let committed = text
            .as_ref()
            .map(ToString::to_string)
            .filter(|text| !text.is_empty())
            .or_else(|| committed_text_from_key(&key, modifiers));

        if let Some(committed) = committed
            .filter(|_| !(modifiers.control() || modifiers.alt() || modifiers.logo()))
        {
            if self.stream_ready_for_terminal_io() {
                let input_seq = self.record_pending_input();
                self.send_stream_command(StreamCommand::Input {
                    input_seq,
                    bytes: committed.into_bytes(),
                });
            } else {
                let _ = self.client.send(&control::Request::SendText { text: committed });
            }
            return;
        }

        if let Some(keyspec) = keyspec_from_key(&key, modifiers, text.as_deref()) {
            if self.stream_ready_for_terminal_io() {
                let input_seq = self.record_pending_input();
                self.send_stream_command(StreamCommand::Key { input_seq, keyspec });
            } else {
                let _ = self.client.send(&control::Request::SendKey { key: keyspec });
            }
        }
    }

    fn handle_stream_event(&mut self, event: LocalStreamEvent) {
        match event {
                LocalStreamEvent::SessionList(sessions) => {
                    let live_sessions: Vec<_> = sessions
                        .iter()
                        .filter(|session| !session.child_exited)
                        .collect();
                    if let Some(session) = live_sessions
                        .get(self.active_tab_index)
                        .or_else(|| live_sessions.first())
                    {
                        self.should_exit = false;
                        self.send_stream_command(StreamCommand::Attach(session.id));
                    } else {
                        self.should_exit = true;
                    }
                }
                LocalStreamEvent::Attached(session_id) => {
                    let _ = session_id;
                }
                LocalStreamEvent::Detached => {
                    self.stream_state = None;
                    self.stream_snapshot = None;
                    self.pending_input_latencies.clear();
                    self.render_revision = self.render_revision.wrapping_add(1);
                    self.send_stream_command(StreamCommand::ListSessions);
                }
                LocalStreamEvent::SessionExited(session_id) => {
                    let _ = session_id;
                    self.stream_state = None;
                    self.stream_snapshot = None;
                    self.pending_input_latencies.clear();
                    self.render_revision = self.render_revision.wrapping_add(1);
                    self.send_stream_command(StreamCommand::ListSessions);
                }
                LocalStreamEvent::Disconnected => {
                    self.stream_state = None;
                    self.stream_snapshot = None;
                    self.pending_input_latencies.clear();
                    self.last_error = Some("boo server disconnected".to_string());
                    self.should_exit = true;
                }
                LocalStreamEvent::FullState { ack_input_seq, state } => {
                    self.stream_snapshot = Some(Arc::new(remote_full_state_to_vt_snapshot(&state)));
                    self.stream_state = Some(state);
                    self.render_revision = self.render_revision.wrapping_add(1);
                    self.acknowledge_input_latency("stream_full_state", ack_input_seq);
                }
                LocalStreamEvent::Delta { ack_input_seq, delta } => {
                    if let Some(state) = self.stream_state.as_mut() {
                        apply_remote_delta(state, &delta);
                    }
                    if let Some(snapshot) = self.stream_snapshot.as_mut() {
                        apply_remote_delta_snapshot(Arc::make_mut(snapshot), &delta);
                    }
                    self.render_revision = self.render_revision.wrapping_add(1);
                    self.acknowledge_input_latency("stream_delta", ack_input_seq);
                }
                LocalStreamEvent::Error(error) => {
                    self.last_error = Some(error);
                }
        }
    }

    fn send_stream_command(&self, command: StreamCommand) {
        if let Some(tx) = self.stream_tx.as_ref() {
            let _ = tx.send(command);
        }
    }

    fn record_pending_input(&mut self) -> u64 {
        let input_seq = self.next_input_seq;
        self.next_input_seq = self.next_input_seq.wrapping_add(1);
        self.pending_input_latencies
            .insert(input_seq, Instant::now());
        input_seq
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
    if !latency_debug_enabled() {
        return;
    }
    log::info!(
        "boo_latency stage={stage} seq={input_seq} ms={:.3}",
        started_at.elapsed().as_secs_f64() * 1000.0
    );
}

fn remote_full_state_to_vt_snapshot(state: &remote::RemoteFullState) -> vt_backend_core::TerminalSnapshot {
    let cols = state.cols as usize;
    let rows_data = state
        .cells
        .chunks(cols.max(1))
        .map(|row| {
            row.iter().map(remote_cell_to_snapshot).collect()
        })
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
    changed_rows: Vec<(u16, Vec<remote::RemoteCell>)>,
}

#[derive(Clone, Debug)]
pub(crate) enum StreamCommand {
    ListSessions,
    Attach(u32),
    Input { input_seq: u64, bytes: Vec<u8> },
    Key { input_seq: u64, keyspec: String },
    Resize { cols: u16, rows: u16 },
}

fn write_stream_message(write: &mut UnixStream, ty: remote::MessageType, payload: &[u8]) -> std::io::Result<()> {
        let frame = remote::encode_message(ty, payload);
        write.write_all(&frame)?;
        write.flush()
}

fn local_stream_subscription(
    socket_path: &String,
) -> iced::futures::stream::BoxStream<'static, Message> {
    let socket_path = socket_path.clone();
    Box::pin(stream::channel(
        100,
        move |mut output: iced::futures::channel::mpsc::Sender<Message>| async move {
        let Ok(write) = UnixStream::connect(&socket_path) else {
            let _ = output.send(Message::StreamEvent(LocalStreamEvent::Disconnected)).await;
            return;
        };
        let Ok(read) = write.try_clone() else {
            let _ = output.send(Message::StreamEvent(LocalStreamEvent::Disconnected)).await;
            return;
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
                    StreamCommand::ListSessions => {
                        write_stream_message(&mut write, remote::MessageType::ListSessions, &[])
                    }
                    StreamCommand::Attach(session_id) => {
                        write_stream_message(&mut write, remote::MessageType::Attach, &session_id.to_le_bytes())
                    }
                    StreamCommand::Input { input_seq, bytes } => {
                        let mut payload = Vec::with_capacity(8 + bytes.len());
                        payload.extend_from_slice(&input_seq.to_le_bytes());
                        payload.extend_from_slice(&bytes);
                        write_stream_message(&mut write, remote::MessageType::Input, &payload)
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
                        write_stream_message(&mut write, remote::MessageType::Resize, &payload)
                    }
                };
                if result.is_err() {
                    let _ = writer_event_tx.unbounded_send(LocalStreamEvent::Disconnected);
                    break;
                }
            }
        });

        std::thread::spawn(move || read_local_stream_loop(read, move |event| {
            let _ = event_tx.unbounded_send(event);
        }));

        while let Some(event) = event_rx.next().await {
            let _ = output.send(Message::StreamEvent(event)).await;
        }
        },
    ))
}

fn read_local_stream_loop(mut read: UnixStream, mut emit: impl FnMut(LocalStreamEvent)) {
    while let Ok((ty, payload)) = remote::read_message(&mut read) {
        let event = match ty {
            remote::MessageType::SessionList => {
                decode_remote_session_list(&payload).map(LocalStreamEvent::SessionList)
            }
            remote::MessageType::Attached => decode_u32(&payload).map(LocalStreamEvent::Attached),
            remote::MessageType::Detached => Some(LocalStreamEvent::Detached),
            remote::MessageType::SessionExited => {
                decode_u32(&payload).map(LocalStreamEvent::SessionExited)
            }
            remote::MessageType::FullState => decode_remote_full_state(&payload).map(
                |(ack_input_seq, state)| LocalStreamEvent::FullState { ack_input_seq, state },
            ),
            remote::MessageType::Delta => decode_remote_delta(&payload)
                .map(|(ack_input_seq, delta)| LocalStreamEvent::Delta { ack_input_seq, delta }),
            remote::MessageType::ErrorMsg => {
                Some(LocalStreamEvent::Error(String::from_utf8_lossy(&payload).to_string()))
            }
            _ => None,
        };
        if let Some(event) = event {
            emit(event);
        }
    }
    emit(LocalStreamEvent::Disconnected);
}

fn decode_u32(payload: &[u8]) -> Option<u32> {
    (payload.len() >= 4).then(|| u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]))
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
            fg: [payload[offset + 4], payload[offset + 5], payload[offset + 6]],
            bg: [payload[offset + 7], payload[offset + 8], payload[offset + 9]],
            style_flags: payload[offset + 10],
            wide: payload[offset + 11] != 0,
        });
        offset += 12;
    }
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
    let mut offset = 16usize;
    let mut changed_rows = Vec::with_capacity(row_count);
    for _ in 0..row_count {
        if offset + 4 > payload.len() {
            return None;
        }
        let row = u16::from_le_bytes([payload[offset], payload[offset + 1]]);
        let cols = u16::from_le_bytes([payload[offset + 2], payload[offset + 3]]) as usize;
        offset += 4;
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
                fg: [payload[offset + 4], payload[offset + 5], payload[offset + 6]],
                bg: [payload[offset + 7], payload[offset + 8], payload[offset + 9]],
                style_flags: payload[offset + 10],
                wide: payload[offset + 11] != 0,
            });
            offset += 12;
        }
        changed_rows.push((row, cells));
    }
    Some((
        (ack_input_seq != 0).then_some(ack_input_seq),
        RemoteDelta {
            cursor_x,
            cursor_y,
            cursor_visible,
            changed_rows,
        },
    ))
}

fn apply_remote_delta(state: &mut remote::RemoteFullState, delta: &RemoteDelta) {
    state.cursor_x = delta.cursor_x;
    state.cursor_y = delta.cursor_y;
    state.cursor_visible = delta.cursor_visible;
    let cols = state.cols as usize;
    for (row, row_cells) in &delta.changed_rows {
        let start = *row as usize * cols;
        let end = start + row_cells.len().min(cols);
        if end <= state.cells.len() {
            state.cells[start..end].clone_from_slice(&row_cells[..(end - start)]);
        }
    }
}

fn apply_remote_delta_snapshot(
    snapshot: &mut vt_backend_core::TerminalSnapshot,
    delta: &RemoteDelta,
) {
    snapshot.cursor.x = delta.cursor_x;
    snapshot.cursor.y = delta.cursor_y;
    snapshot.cursor.visible = delta.cursor_visible;
    let cols = snapshot.cols as usize;
    for (row, row_cells) in &delta.changed_rows {
        let row_index = *row as usize;
        if row_index >= snapshot.rows_data.len() {
            continue;
        }
        let target_row = &mut snapshot.rows_data[row_index];
        if target_row.len() < cols {
            target_row.resize_with(cols, Default::default);
        }
        for (col_index, cell) in row_cells.iter().enumerate().take(cols) {
            target_row[col_index] = remote_cell_to_snapshot(cell);
        }
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
        text: std::char::from_u32(cell.codepoint)
            .map(|ch| ch.to_string())
            .unwrap_or_else(|| " ".to_string()),
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
    fn committed_text_from_character_key_without_text_payload() {
        let key = keyboard::Key::Character("a".into());
        assert_eq!(
            committed_text_from_key(&key, keyboard::Modifiers::default()),
            Some("a".to_string())
        );
    }

    #[test]
    fn committed_text_from_key_ignores_control_modified_input() {
        let key = keyboard::Key::Character("d".into());
        assert_eq!(
            committed_text_from_key(&key, keyboard::Modifiers::CTRL),
            None
        );
    }
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

fn committed_text_from_key(
    key: &keyboard::Key,
    modifiers: keyboard::Modifiers,
) -> Option<String> {
    if modifiers.control() || modifiers.alt() || modifiers.logo() {
        return None;
    }
    match key {
        keyboard::Key::Character(chars) if !chars.is_empty() => Some(chars.to_string()),
        _ => None,
    }
}
