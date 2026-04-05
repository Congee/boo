mod backend;
mod bindings;
mod client_gui;
mod cli;
mod config;
mod control;
mod ffi;
mod keymap;
mod launch;
#[cfg(target_os = "macos")]
mod macos_vt_backend;
mod pane;
mod platform;
mod remote;
mod runtime;
mod server;
mod session;
mod splits;
mod tabs;
mod tmux;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod unix_pty;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod vt;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod vt_backend_core;
#[cfg(target_os = "linux")]
mod vt_snapshot;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod vt_terminal_canvas;

use backend::TerminalBackend;
use iced::widget::{container, row, text};
use iced::window;
use iced::{Color, Element, Event, Font, Length, Size, Subscription, Task, Theme, keyboard, mouse};
use pane::PaneHandle;
use std::ffi::{CStr, CString, c_void};
#[cfg(target_os = "linux")]
use std::process::Command;
use std::ptr;

/// Status bar height in points.
const STATUS_BAR_HEIGHT: f64 = 20.0;

static SCROLL_RX: std::sync::OnceLock<
    std::sync::Mutex<std::sync::mpsc::Receiver<platform::ScrollEvent>>,
> = std::sync::OnceLock::new();
static KEY_EVENT_RX: std::sync::OnceLock<
    std::sync::Mutex<std::sync::mpsc::Receiver<platform::KeyEvent>>,
> = std::sync::OnceLock::new();
static TEXT_INPUT_RX: std::sync::OnceLock<
    std::sync::Mutex<std::sync::mpsc::Receiver<platform::TextInputEvent>>,
> = std::sync::OnceLock::new();

#[derive(Clone, Copy)]
struct CommandFinishedEvent {
    exit_code: Option<u8>,
    duration_ns: u64,
}

const DEFAULT_TERMINAL_FONT_SIZE: f32 = 14.0;
const DEFAULT_BACKGROUND_OPACITY: f32 = 1.0;
const HEADLESS_WIDTH: f32 = 1024.0;
const HEADLESS_HEIGHT: f32 = 768.0;
#[derive(Debug)]
struct ResolvedAppearance {
    font_family: Option<&'static str>,
    font_size: f32,
    background_opacity: f32,
    background_opacity_cells: bool,
    #[cfg(target_os = "linux")]
    font_bytes: Option<Vec<u8>>,
}

fn leak_font_family(name: &str) -> &'static str {
    Box::leak(name.to_owned().into_boxed_str())
}

#[cfg(target_os = "linux")]
fn resolve_linux_font(name: &str) -> (Option<&'static str>, Option<Vec<u8>>) {
    let output = Command::new("fc-match")
        .args(["-f", "%{family[0]}|%{file}\n", name])
        .output();

    let Ok(output) = output else {
        log::warn!("failed to run fc-match for font family {:?}", name);
        return (Some(leak_font_family(name)), None);
    };

    if !output.status.success() {
        log::warn!("fc-match failed for font family {:?}", name);
        return (Some(leak_font_family(name)), None);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout.trim();
    let Some((resolved_family, resolved_file)) = line.split_once('|') else {
        return (Some(leak_font_family(name)), None);
    };

    let resolved_family = resolved_family.trim();
    let resolved_file = resolved_file.trim();

    let family = if resolved_family.is_empty() {
        leak_font_family(name)
    } else {
        if resolved_family != name {
            log::info!(
                "resolved font family {:?} to {:?} via fc-match",
                name,
                resolved_family
            );
        }
        leak_font_family(resolved_family)
    };

    let font_bytes = if resolved_file.is_empty() {
        None
    } else {
        match std::fs::read(resolved_file) {
            Ok(bytes) => Some(bytes),
            Err(error) => {
                log::warn!(
                    "failed to read resolved font file {:?} for family {:?}: {}",
                    resolved_file,
                    family,
                    error
                );
                None
            }
        }
    };

    (Some(family), font_bytes)
}

fn terminal_metrics(font_size: f32) -> (f64, f64) {
    let size = font_size.max(8.0) as f64;
    let cell_width = (size * 0.62).max(6.0).ceil();
    let cell_height = (size * 1.35).max(12.0).ceil();
    (cell_width, cell_height)
}

#[allow(dead_code)]
fn configured_font(family: Option<&'static str>) -> Font {
    family.map(Font::with_name).unwrap_or(Font::MONOSPACE)
}

fn main() {
    env_logger::init();

    let args: Vec<String> = std::env::args().collect();
    let server_mode = launch::parse_startup_args(&args);
    let startup_config = launch::load_startup_config();
    match cli::handle_command(&args, &startup_config, launch::ensure_server_running) {
        cli::Outcome::Continue => {}
        cli::Outcome::Exit(code) => std::process::exit(code),
    }
    let headless = server_mode || args.iter().any(|a| a == "--headless");
    if headless {
        runtime::run_headless();
        return;
    }
    launch::run_gui_client();
}

struct BooApp {
    backend: backend::Backend,
    headless: bool,
    server: server::State,
    parent_view: *mut c_void,
    scroll_rx: std::sync::mpsc::Receiver<platform::ScrollEvent>,
    key_event_rx: std::sync::mpsc::Receiver<platform::KeyEvent>,
    text_input_rx: std::sync::mpsc::Receiver<platform::TextInputEvent>,
    bindings: bindings::Bindings,
    dump_keys: bool,
    last_size: Size,
    last_mouse_pos: (f64, f64),
    divider_drag: Option<splits::Direction>,
    scrollbar_drag: bool,
    scrollbar_opacity: f32,
    cell_width: f64,
    cell_height: f64,
    scrollbar: ffi::ghostty_action_scrollbar_s,
    scrollbar_layer: *mut c_void,
    search_active: bool,
    search_query: String,
    search_total: isize,
    search_selected: isize,
    pwd: String,
    preedit_text: String,
    last_clipboard_text: String,
    copy_mode: Option<CopyModeState>,
    command_prompt: CommandPrompt,
    terminal_font_family: Option<&'static str>,
    terminal_font_size: f32,
    background_opacity: f32,
    background_opacity_cells: bool,
    appearance_revision: u64,
    app_focused: bool,
    desktop_notifications_enabled: bool,
    notify_on_command_finish: config::NotifyOnCommandFinish,
    notify_on_command_finish_action: config::NotifyOnCommandFinishAction,
    notify_on_command_finish_after_ns: u64,
    #[cfg(target_os = "linux")]
    pending_font_bytes: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SelectionMode {
    None,
    Char,
    Line,
    Rectangle,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum JumpKind {
    Forward,
    Backward,
    ToForward,
    ToBackward,
}

#[derive(Debug, Clone, Copy)]
enum WordMoveKind {
    NextWord,
    PrevWord,
    EndWord,
    NextBigWord,
    PrevBigWord,
    EndBigWord,
}

struct CopyModeState {
    cursor_row: i64,
    cursor_col: u32,
    selection: SelectionMode,
    sel_anchor: Option<(i64, u32)>,
    highlight_layers: Vec<*mut c_void>,
    cursor_layer: *mut c_void,
    cell_width: f64,
    cell_height: f64,
    viewport_rows: u32,
    viewport_cols: u32,
    mark: Option<(i64, u32)>,
    last_jump: Option<(char, JumpKind)>,
    last_search_forward: bool,
    pending_jump: Option<JumpKind>,
    show_position: bool,
}

struct CommandDef {
    name: &'static str,
    #[allow(dead_code)]
    description: &'static str,
    args: &'static str, // e.g. "<n>" or "" for no args
}

const COMMANDS: &[CommandDef] = &[
    CommandDef {
        name: "split-right",
        description: "vertical split",
        args: "",
    },
    CommandDef {
        name: "split-down",
        description: "horizontal split",
        args: "",
    },
    CommandDef {
        name: "split-left",
        description: "vertical split (left)",
        args: "",
    },
    CommandDef {
        name: "split-up",
        description: "horizontal split (up)",
        args: "",
    },
    CommandDef {
        name: "resize-left",
        description: "resize pane left",
        args: "<n>",
    },
    CommandDef {
        name: "resize-right",
        description: "resize pane right",
        args: "<n>",
    },
    CommandDef {
        name: "resize-up",
        description: "resize pane up",
        args: "<n>",
    },
    CommandDef {
        name: "resize-down",
        description: "resize pane down",
        args: "<n>",
    },
    CommandDef {
        name: "close-pane",
        description: "close focused pane",
        args: "",
    },
    CommandDef {
        name: "new-tab",
        description: "create new tab",
        args: "",
    },
    CommandDef {
        name: "next-tab",
        description: "switch to next tab",
        args: "",
    },
    CommandDef {
        name: "prev-tab",
        description: "switch to previous tab",
        args: "",
    },
    CommandDef {
        name: "close-tab",
        description: "close current tab",
        args: "",
    },
    CommandDef {
        name: "goto-tab",
        description: "go to tab number",
        args: "<n>",
    },
    CommandDef {
        name: "last-tab",
        description: "go to last tab",
        args: "",
    },
    CommandDef {
        name: "next-pane",
        description: "focus next pane",
        args: "",
    },
    CommandDef {
        name: "prev-pane",
        description: "focus previous pane",
        args: "",
    },
    CommandDef {
        name: "copy-mode",
        description: "enter copy mode",
        args: "",
    },
    CommandDef {
        name: "command-prompt",
        description: "open command prompt",
        args: "",
    },
    CommandDef {
        name: "search",
        description: "open search",
        args: "",
    },
    CommandDef {
        name: "paste",
        description: "paste from clipboard",
        args: "",
    },
    CommandDef {
        name: "zoom",
        description: "toggle pane zoom",
        args: "",
    },
    CommandDef {
        name: "reload-config",
        description: "reload configuration",
        args: "",
    },
    CommandDef {
        name: "goto-line",
        description: "jump to line (copy mode)",
        args: "<n>",
    },
    CommandDef {
        name: "set",
        description: "set ghostty config value",
        args: "<key> <value>",
    },
    CommandDef {
        name: "load-session",
        description: "load a session layout",
        args: "<name>",
    },
    CommandDef {
        name: "save-session",
        description: "save current layout",
        args: "<name>",
    },
    CommandDef {
        name: "list-sessions",
        description: "list available sessions",
        args: "",
    },
];

struct CommandPrompt {
    active: bool,
    input: String,
    history: Vec<String>,
    history_idx: Option<usize>,
    suggestions: Vec<usize>, // indices into COMMANDS
    selected_suggestion: usize,
}

impl CommandPrompt {
    fn new() -> Self {
        CommandPrompt {
            active: false,
            input: String::new(),
            history: Vec::new(),
            history_idx: None,
            suggestions: Vec::new(),
            selected_suggestion: 0,
        }
    }

    fn update_suggestions(&mut self) {
        let query = self.input.split_whitespace().next().unwrap_or("");
        if query.is_empty() {
            self.suggestions = (0..COMMANDS.len()).collect();
        } else {
            let mut scored: Vec<(usize, i32)> = COMMANDS
                .iter()
                .enumerate()
                .filter_map(|(i, cmd)| {
                    let score = fuzzy_score(query, cmd.name);
                    if score > 0 { Some((i, score)) } else { None }
                })
                .collect();
            scored.sort_by(|a, b| b.1.cmp(&a.1));
            self.suggestions = scored.into_iter().map(|(i, _)| i).take(7).collect();
        }
        self.selected_suggestion = 0;
    }

    fn selected_command(&self) -> Option<&'static CommandDef> {
        self.suggestions
            .get(self.selected_suggestion)
            .map(|&i| &COMMANDS[i])
    }
}

fn fuzzy_score(query: &str, target: &str) -> i32 {
    if query.is_empty() {
        return 1;
    }
    let ql = query.to_lowercase();
    let tl = target.to_lowercase();

    // Exact prefix
    if tl.starts_with(&ql) {
        return 100 + (100 - target.len() as i32);
    }

    // Word-initial match: "sr" matches "split-right" via s...r...
    let parts: Vec<&str> = tl.split('-').collect();
    let mut qi = 0;
    let qchars: Vec<char> = ql.chars().collect();
    for part in &parts {
        if qi < qchars.len() && part.starts_with(qchars[qi]) {
            qi += 1;
        }
    }
    if qi == qchars.len() {
        return 50 + (100 - target.len() as i32);
    }

    // Subsequence match
    let mut qi = 0;
    for tc in tl.chars() {
        if qi < qchars.len() && tc == qchars[qi] {
            qi += 1;
        }
    }
    if qi == qchars.len() {
        return 10 + (100 - target.len() as i32);
    }

    0
}

#[derive(Debug, Clone)]
enum Message {
    Frame,
    #[cfg(target_os = "linux")]
    FontLoaded,
    #[allow(dead_code)]
    IcedEvent(Event),
}

impl BooApp {
    fn dispatch_copy_mode_action(&mut self, action: bindings::CopyModeAction) {
        use bindings::CopyModeAction::*;

        // If we're waiting for a jump target character, consume it
        if let Some(_kind) = self.copy_mode.as_ref().and_then(|cm| cm.pending_jump) {
            if let JumpForward | JumpBackward | JumpToForward | JumpToBackward = action {
                // These are the jump initiators — they set pending_jump below
            } else {
                // Any other action cancels the pending jump
                if let Some(ref mut cm) = self.copy_mode {
                    cm.pending_jump = None;
                }
            }
        }

        match action {
            Move(dir) => self.copy_mode_move(dir),

            // Word movement
            WordNext => self.copy_mode_word_move(WordMoveKind::NextWord),
            WordBack => self.copy_mode_word_move(WordMoveKind::PrevWord),
            WordEnd => self.copy_mode_word_move(WordMoveKind::EndWord),
            BigWordNext => self.copy_mode_word_move(WordMoveKind::NextBigWord),
            BigWordBack => self.copy_mode_word_move(WordMoveKind::PrevBigWord),
            BigWordEnd => self.copy_mode_word_move(WordMoveKind::EndBigWord),

            // Line position
            LineStart => {
                if let Some(ref mut cm) = self.copy_mode {
                    cm.cursor_col = 0;
                }
                self.update_copy_mode_highlight();
            }
            LineEnd => {
                if let Some(ref mut cm) = self.copy_mode {
                    cm.cursor_col = cm.viewport_cols.saturating_sub(1);
                }
                self.update_copy_mode_highlight();
            }
            FirstNonBlank => self.copy_mode_first_non_blank(),

            // Screen position
            ScreenTop => {
                if let Some(ref mut cm) = self.copy_mode {
                    cm.cursor_row = self.scrollbar.offset as i64;
                    cm.cursor_col = 0;
                }
                self.update_copy_mode_highlight();
            }
            ScreenMiddle => {
                if let Some(ref mut cm) = self.copy_mode {
                    cm.cursor_row = self.scrollbar.offset as i64 + cm.viewport_rows as i64 / 2;
                    cm.cursor_col = 0;
                }
                self.update_copy_mode_highlight();
            }
            ScreenBottom => {
                if let Some(ref mut cm) = self.copy_mode {
                    cm.cursor_row = self.scrollbar.offset as i64 + cm.viewport_rows as i64 - 1;
                    cm.cursor_col = 0;
                }
                self.update_copy_mode_highlight();
            }

            // Scrollback
            HistoryTop => {
                if let Some(ref mut cm) = self.copy_mode {
                    cm.cursor_row = 0;
                    cm.cursor_col = 0;
                }
                self.ghostty_binding_action("scroll_to_top");
                self.update_copy_mode_highlight();
            }
            HistoryBottom => {
                if let Some(ref mut cm) = self.copy_mode {
                    cm.cursor_row = self.scrollbar.total as i64;
                    cm.cursor_col = 0;
                }
                self.ghostty_binding_action("scroll_to_bottom");
                self.update_copy_mode_highlight();
            }

            // Page/scroll
            PageUp => {
                if let Some(ref mut cm) = self.copy_mode {
                    cm.cursor_row = cm.cursor_row.saturating_sub(cm.viewport_rows as i64);
                }
                self.ghostty_binding_action("scroll_page_up");
                self.update_copy_mode_highlight();
            }
            PageDown => {
                if let Some(ref mut cm) = self.copy_mode {
                    cm.cursor_row += cm.viewport_rows as i64;
                }
                self.ghostty_binding_action("scroll_page_down");
                self.update_copy_mode_highlight();
            }
            HalfPageUp => {
                if let Some(ref mut cm) = self.copy_mode {
                    let half = (cm.viewport_rows / 2) as i64;
                    cm.cursor_row = cm.cursor_row.saturating_sub(half);
                }
                self.copy_mode_ensure_visible();
                self.update_copy_mode_highlight();
            }
            HalfPageDown => {
                if let Some(ref mut cm) = self.copy_mode {
                    let half = (cm.viewport_rows / 2) as i64;
                    cm.cursor_row += half;
                }
                self.copy_mode_ensure_visible();
                self.update_copy_mode_highlight();
            }
            ScrollUp => {
                self.ghostty_binding_action("scroll_page_lines:-1");
                self.update_copy_mode_highlight();
            }
            ScrollDown => {
                self.ghostty_binding_action("scroll_page_lines:1");
                self.update_copy_mode_highlight();
            }
            ScrollMiddle => {
                // Scroll so cursor is at middle of screen
                if let Some(ref cm) = self.copy_mode {
                    let target_offset = cm.cursor_row - cm.viewport_rows as i64 / 2;
                    let target_offset = target_offset.max(0) as usize;
                    let current = self.scrollbar.offset;
                    let diff = target_offset as i64 - current as i64;
                    if diff != 0 {
                        let cmd = format!("scroll_page_lines:{diff}");
                        self.ghostty_binding_action(&cmd);
                    }
                }
                self.update_copy_mode_highlight();
            }

            // Selection
            StartCharSelect => {
                if let Some(ref mut cm) = self.copy_mode {
                    if cm.selection == SelectionMode::Char {
                        cm.selection = SelectionMode::None;
                        cm.sel_anchor = None;
                    } else {
                        cm.selection = SelectionMode::Char;
                        cm.sel_anchor = Some((cm.cursor_row, cm.cursor_col));
                    }
                }
                self.update_copy_mode_highlight();
            }
            StartLineSelect => {
                if let Some(ref mut cm) = self.copy_mode {
                    if cm.selection == SelectionMode::Line {
                        cm.selection = SelectionMode::None;
                        cm.sel_anchor = None;
                    } else {
                        cm.selection = SelectionMode::Line;
                        cm.sel_anchor = Some((cm.cursor_row, cm.cursor_col));
                    }
                }
                self.update_copy_mode_highlight();
            }
            StartRectSelect => {
                if let Some(ref mut cm) = self.copy_mode {
                    if cm.selection == SelectionMode::Rectangle {
                        cm.selection = SelectionMode::None;
                        cm.sel_anchor = None;
                    } else {
                        cm.selection = SelectionMode::Rectangle;
                        if cm.sel_anchor.is_none() {
                            cm.sel_anchor = Some((cm.cursor_row, cm.cursor_col));
                        }
                    }
                }
                self.update_copy_mode_highlight();
            }
            ClearSelection => {
                if let Some(ref mut cm) = self.copy_mode {
                    if cm.selection != SelectionMode::None {
                        cm.selection = SelectionMode::None;
                        cm.sel_anchor = None;
                        self.update_copy_mode_highlight();
                    } else {
                        self.exit_copy_mode();
                    }
                }
            }
            SwapAnchor => {
                if let Some(ref mut cm) = self.copy_mode {
                    if let Some((ar, ac)) = cm.sel_anchor {
                        let (cr, cc) = (cm.cursor_row, cm.cursor_col);
                        cm.cursor_row = ar;
                        cm.cursor_col = ac;
                        cm.sel_anchor = Some((cr, cc));
                    }
                }
                self.copy_mode_ensure_visible();
                self.update_copy_mode_highlight();
            }

            // In-line jump — set pending state, next keypress will be the target
            JumpForward => {
                if let Some(ref mut cm) = self.copy_mode {
                    cm.pending_jump = Some(JumpKind::Forward);
                }
            }
            JumpBackward => {
                if let Some(ref mut cm) = self.copy_mode {
                    cm.pending_jump = Some(JumpKind::Backward);
                }
            }
            JumpToForward => {
                if let Some(ref mut cm) = self.copy_mode {
                    cm.pending_jump = Some(JumpKind::ToForward);
                }
            }
            JumpToBackward => {
                if let Some(ref mut cm) = self.copy_mode {
                    cm.pending_jump = Some(JumpKind::ToBackward);
                }
            }
            JumpAgain => self.copy_mode_jump_repeat(false),
            JumpReverse => self.copy_mode_jump_repeat(true),

            // Paragraph/bracket
            NextParagraph => self.copy_mode_paragraph(true),
            PreviousParagraph => self.copy_mode_paragraph(false),
            MatchingBracket => self.copy_mode_matching_bracket(),

            // Marks
            SetMark => {
                if let Some(ref mut cm) = self.copy_mode {
                    cm.mark = Some((cm.cursor_row, cm.cursor_col));
                }
            }
            JumpToMark => {
                if let Some(ref mut cm) = self.copy_mode {
                    if let Some((r, c)) = cm.mark {
                        cm.cursor_row = r;
                        cm.cursor_col = c;
                    }
                }
                self.copy_mode_ensure_visible();
                self.update_copy_mode_highlight();
            }

            // Search
            SearchForward => {
                if let Some(ref mut cm) = self.copy_mode {
                    cm.last_search_forward = true;
                }
                self.search_active = true;
                self.search_query.clear();
                self.search_total = 0;
                self.search_selected = 0;
                self.relayout();
            }
            SearchBackward => {
                if let Some(ref mut cm) = self.copy_mode {
                    cm.last_search_forward = false;
                }
                self.search_active = true;
                self.search_query.clear();
                self.search_total = 0;
                self.search_selected = 0;
                self.relayout();
            }
            SearchAgain => {
                let forward = self
                    .copy_mode
                    .as_ref()
                    .map_or(true, |cm| cm.last_search_forward);
                if forward {
                    self.ghostty_binding_action("navigate_search:next");
                } else {
                    self.ghostty_binding_action("navigate_search:previous");
                }
            }
            SearchReverse => {
                let forward = self
                    .copy_mode
                    .as_ref()
                    .map_or(true, |cm| cm.last_search_forward);
                if forward {
                    self.ghostty_binding_action("navigate_search:previous");
                } else {
                    self.ghostty_binding_action("navigate_search:next");
                }
            }
            SearchWordForward => {
                if let Some(word) = self.copy_mode_word_under_cursor() {
                    if let Some(ref mut cm) = self.copy_mode {
                        cm.last_search_forward = true;
                    }
                    self.search_active = true;
                    self.search_query = word;
                    self.relayout();
                    self.ghostty_binding_action("navigate_search:next");
                }
            }
            SearchWordBackward => {
                if let Some(word) = self.copy_mode_word_under_cursor() {
                    if let Some(ref mut cm) = self.copy_mode {
                        cm.last_search_forward = false;
                    }
                    self.search_active = true;
                    self.search_query = word;
                    self.relayout();
                    self.ghostty_binding_action("navigate_search:previous");
                }
            }

            // Copy
            CopyAndExit => {
                self.copy_mode_copy();
                self.exit_copy_mode();
            }
            CopyToEndOfLine => {
                // Select from cursor to end of line, copy, exit
                if let Some(ref mut cm) = self.copy_mode {
                    cm.selection = SelectionMode::Char;
                    cm.sel_anchor = Some((cm.cursor_row, cm.cursor_col));
                    cm.cursor_col = cm.viewport_cols.saturating_sub(1);
                }
                self.copy_mode_copy();
                self.exit_copy_mode();
            }
            AppendAndCancel => {
                self.copy_mode_append_copy();
                self.exit_copy_mode();
            }

            // Other
            OpenPrompt => {
                self.command_prompt.active = true;
                self.command_prompt.input.clear();
                self.command_prompt.selected_suggestion = 0;
                self.command_prompt.history_idx = None;
                self.command_prompt.update_suggestions();
            }
            RefreshFromPane => {
                // Re-read terminal state — for boo this is effectively a no-op since
                // we read from ghostty's live buffer on every operation
                self.update_copy_mode_highlight();
            }
            TogglePosition => {
                if let Some(ref mut cm) = self.copy_mode {
                    cm.show_position = !cm.show_position;
                }
            }
            Exit => self.exit_copy_mode(),
        }
    }

    fn copy_mode_move(&mut self, dir: bindings::Direction) {
        let Some(ref mut cm) = self.copy_mode else {
            return;
        };
        match dir {
            bindings::Direction::Up => cm.cursor_row -= 1,
            bindings::Direction::Down => cm.cursor_row += 1,
            bindings::Direction::Left => {
                if cm.cursor_col > 0 {
                    cm.cursor_col -= 1;
                }
            }
            bindings::Direction::Right => {
                cm.cursor_col = (cm.cursor_col + 1).min(cm.viewport_cols.saturating_sub(1));
            }
        }
        self.copy_mode_ensure_visible();
        self.update_copy_mode_highlight();
    }

    fn copy_mode_ensure_visible(&mut self) {
        let Some(ref cm) = self.copy_mode else { return };
        let viewport_row = cm.cursor_row - self.scrollbar.offset as i64;
        if viewport_row < 0 {
            let lines = -viewport_row;
            let cmd = format!("scroll_page_lines:-{lines}");
            self.ghostty_binding_action(&cmd);
        } else if viewport_row >= cm.viewport_rows as i64 {
            let lines = viewport_row - cm.viewport_rows as i64 + 1;
            let cmd = format!("scroll_page_lines:{lines}");
            self.ghostty_binding_action(&cmd);
        }
    }

    fn copy_mode_first_non_blank(&mut self) {
        if let Some(line) = self.read_viewport_line_for_cursor() {
            let col = line.chars().position(|c| !c.is_whitespace()).unwrap_or(0);
            if let Some(ref mut cm) = self.copy_mode {
                cm.cursor_col = col as u32;
            }
        }
        self.update_copy_mode_highlight();
    }

    fn read_viewport_line_for_cursor(&self) -> Option<String> {
        let cm = self.copy_mode.as_ref()?;
        let viewport_row = (cm.cursor_row - self.scrollbar.offset as i64).max(0) as u32;
        self.read_viewport_line(viewport_row)
    }

    fn read_viewport_line(&self, viewport_row: u32) -> Option<String> {
        let cm = self.copy_mode.as_ref()?;
        let sel = ffi::ghostty_selection_s {
            top_left: ffi::ghostty_point_s {
                tag: ffi::GHOSTTY_POINT_VIEWPORT,
                coord: ffi::GHOSTTY_POINT_COORD_EXACT,
                x: 0,
                y: viewport_row,
            },
            bottom_right: ffi::ghostty_point_s {
                tag: ffi::GHOSTTY_POINT_VIEWPORT,
                coord: ffi::GHOSTTY_POINT_COORD_EXACT,
                x: cm.viewport_cols.saturating_sub(1),
                y: viewport_row,
            },
            rectangle: false,
        };
        self.read_surface_selection_text(sel)
    }

    fn copy_mode_word_move(&mut self, kind: WordMoveKind) {
        let Some(line) = self.read_viewport_line_for_cursor() else {
            return;
        };
        let Some(ref mut cm) = self.copy_mode else {
            return;
        };
        let chars: Vec<char> = line.chars().collect();
        let col = cm.cursor_col as usize;
        let len = chars.len();

        let is_word = |c: char, big: bool| -> bool {
            if big {
                !c.is_whitespace()
            } else {
                c.is_alphanumeric() || c == '_'
            }
        };
        let is_sep = |c: char| -> bool { !c.is_alphanumeric() && c != '_' && !c.is_whitespace() };

        let new_col = match kind {
            WordMoveKind::NextWord | WordMoveKind::NextBigWord => {
                let big = matches!(kind, WordMoveKind::NextBigWord);
                let mut i = col;
                // Skip current word/punct chars
                if i < len && is_word(chars[i], big) {
                    while i < len && is_word(chars[i], big) {
                        i += 1;
                    }
                } else if !big && i < len && is_sep(chars[i]) {
                    while i < len && is_sep(chars[i]) {
                        i += 1;
                    }
                } else {
                    i += 1;
                }
                // Skip whitespace
                while i < len && chars[i].is_whitespace() {
                    i += 1;
                }
                if i >= len { col } else { i }
            }
            WordMoveKind::PrevWord | WordMoveKind::PrevBigWord => {
                let big = matches!(kind, WordMoveKind::PrevBigWord);
                if col == 0 {
                    0
                } else {
                    let mut i = col - 1;
                    // Skip whitespace backwards
                    while i > 0 && chars[i].is_whitespace() {
                        i -= 1;
                    }
                    // Skip word/punct chars backwards
                    if is_word(chars[i], big) {
                        while i > 0 && is_word(chars[i - 1], big) {
                            i -= 1;
                        }
                    } else if !big && is_sep(chars[i]) {
                        while i > 0 && is_sep(chars[i - 1]) {
                            i -= 1;
                        }
                    }
                    i
                }
            }
            WordMoveKind::EndWord | WordMoveKind::EndBigWord => {
                let big = matches!(kind, WordMoveKind::EndBigWord);
                if col + 1 >= len {
                    col
                } else {
                    let mut i = col + 1;
                    // Skip whitespace
                    while i < len && chars[i].is_whitespace() {
                        i += 1;
                    }
                    // Advance through word/punct chars
                    if i < len && is_word(chars[i], big) {
                        while i + 1 < len && is_word(chars[i + 1], big) {
                            i += 1;
                        }
                    } else if !big && i < len && is_sep(chars[i]) {
                        while i + 1 < len && is_sep(chars[i + 1]) {
                            i += 1;
                        }
                    }
                    i
                }
            }
        };

        cm.cursor_col = new_col as u32;
        self.copy_mode_ensure_visible();
        self.update_copy_mode_highlight();
    }

    fn copy_mode_execute_jump(&mut self, target: char, kind: JumpKind) {
        let Some(line) = self.read_viewport_line_for_cursor() else {
            return;
        };
        let Some(ref mut cm) = self.copy_mode else {
            return;
        };
        let col = cm.cursor_col as usize;
        let chars: Vec<char> = line.chars().collect();

        let new_col = match kind {
            JumpKind::Forward => chars
                .iter()
                .enumerate()
                .skip(col + 1)
                .find(|(_, c)| **c == target)
                .map(|(i, _)| i),
            JumpKind::Backward => chars
                .iter()
                .enumerate()
                .take(col)
                .rev()
                .find(|(_, c)| **c == target)
                .map(|(i, _)| i),
            JumpKind::ToForward => chars
                .iter()
                .enumerate()
                .skip(col + 1)
                .find(|(_, c)| **c == target)
                .map(|(i, _)| i.saturating_sub(1).max(col + 1)),
            JumpKind::ToBackward => chars
                .iter()
                .enumerate()
                .take(col)
                .rev()
                .find(|(_, c)| **c == target)
                .map(|(i, _)| (i + 1).min(col.saturating_sub(1))),
        };

        if let Some(nc) = new_col {
            cm.cursor_col = nc as u32;
        }
        self.update_copy_mode_highlight();
    }

    fn copy_mode_jump_repeat(&mut self, reverse: bool) {
        let Some(ref cm) = self.copy_mode else { return };
        let Some((target, kind)) = cm.last_jump else {
            return;
        };
        let kind = if reverse {
            match kind {
                JumpKind::Forward => JumpKind::Backward,
                JumpKind::Backward => JumpKind::Forward,
                JumpKind::ToForward => JumpKind::ToBackward,
                JumpKind::ToBackward => JumpKind::ToForward,
            }
        } else {
            kind
        };
        self.copy_mode_execute_jump(target, kind);
    }

    fn copy_mode_paragraph(&mut self, forward: bool) {
        let Some(ref mut cm) = self.copy_mode else {
            return;
        };
        let offset = self.scrollbar.offset as i64;
        let max_row = self.scrollbar.total as i64;

        if forward {
            let mut r = cm.cursor_row + 1;
            while r <= max_row {
                let vp = (r - offset).max(0) as u32;
                if let Some(line) = self.read_viewport_line(vp) {
                    if line.trim().is_empty() {
                        if let Some(ref mut cm) = self.copy_mode {
                            cm.cursor_row = r;
                            cm.cursor_col = 0;
                        }
                        break;
                    }
                } else {
                    break;
                }
                r += 1;
            }
        } else {
            let mut r = cm.cursor_row - 1;
            while r >= 0 {
                let vp = (r - offset).max(0) as u32;
                if let Some(line) = self.read_viewport_line(vp) {
                    if line.trim().is_empty() {
                        if let Some(ref mut cm) = self.copy_mode {
                            cm.cursor_row = r;
                            cm.cursor_col = 0;
                        }
                        break;
                    }
                } else {
                    break;
                }
                r -= 1;
            }
        }
        self.copy_mode_ensure_visible();
        self.update_copy_mode_highlight();
    }

    fn copy_mode_matching_bracket(&mut self) {
        let Some(line) = self.read_viewport_line_for_cursor() else {
            return;
        };
        let Some(ref mut cm) = self.copy_mode else {
            return;
        };
        let chars: Vec<char> = line.chars().collect();
        let col = cm.cursor_col as usize;

        // Find bracket at or after cursor
        let brackets = [('(', ')'), ('[', ']'), ('{', '}')];
        let mut found = None;
        for i in col..chars.len() {
            for &(open, close) in &brackets {
                if chars[i] == open {
                    found = Some((i, open, close, true));
                    break;
                } else if chars[i] == close {
                    found = Some((i, open, close, false));
                    break;
                }
            }
            if found.is_some() {
                break;
            }
        }

        let Some((pos, open, close, is_open)) = found else {
            return;
        };
        // Simple single-line bracket matching
        let mut depth = 0i32;
        if is_open {
            for i in pos..chars.len() {
                if chars[i] == open {
                    depth += 1;
                }
                if chars[i] == close {
                    depth -= 1;
                }
                if depth == 0 {
                    cm.cursor_col = i as u32;
                    break;
                }
            }
        } else {
            for i in (0..=pos).rev() {
                if chars[i] == close {
                    depth += 1;
                }
                if chars[i] == open {
                    depth -= 1;
                }
                if depth == 0 {
                    cm.cursor_col = i as u32;
                    break;
                }
            }
        }
        self.update_copy_mode_highlight();
    }

    fn copy_mode_word_under_cursor(&self) -> Option<String> {
        let line = self.read_viewport_line_for_cursor()?;
        let cm = self.copy_mode.as_ref()?;
        let chars: Vec<char> = line.chars().collect();
        let col = cm.cursor_col as usize;
        if col >= chars.len() {
            return None;
        }

        let is_word = |c: char| c.is_alphanumeric() || c == '_';
        if !is_word(chars[col]) {
            return None;
        }

        let mut start = col;
        while start > 0 && is_word(chars[start - 1]) {
            start -= 1;
        }
        let mut end = col;
        while end + 1 < chars.len() && is_word(chars[end + 1]) {
            end += 1;
        }

        Some(chars[start..=end].iter().collect())
    }

    fn copy_mode_append_copy(&mut self) {
        let existing = if self.last_clipboard_text.is_empty() {
            platform::clipboard_read().unwrap_or_default()
        } else {
            self.last_clipboard_text.clone()
        };

        let Some(ref cm) = self.copy_mode else { return };
        let Some((anchor_row, anchor_col)) = cm.sel_anchor else {
            return;
        };

        let (r1, c1, r2, c2) = if anchor_row < cm.cursor_row
            || (anchor_row == cm.cursor_row && anchor_col <= cm.cursor_col)
        {
            (anchor_row, anchor_col, cm.cursor_row, cm.cursor_col)
        } else {
            (cm.cursor_row, cm.cursor_col, anchor_row, anchor_col)
        };
        let (c1, c2) = if cm.selection == SelectionMode::Line {
            (0u32, cm.viewport_cols.saturating_sub(1))
        } else {
            (c1, c2)
        };

        let sel = ffi::ghostty_selection_s {
            top_left: ffi::ghostty_point_s {
                tag: ffi::GHOSTTY_POINT_VIEWPORT,
                coord: ffi::GHOSTTY_POINT_COORD_EXACT,
                x: c1,
                y: (r1 - self.scrollbar.offset as i64).max(0) as u32,
            },
            bottom_right: ffi::ghostty_point_s {
                tag: ffi::GHOSTTY_POINT_VIEWPORT,
                coord: ffi::GHOSTTY_POINT_COORD_EXACT,
                x: c2,
                y: (r2 - self.scrollbar.offset as i64).max(0) as u32,
            },
            rectangle: cm.selection == SelectionMode::Rectangle,
        };
        if let Some(new_text) = self.read_surface_selection_text(sel) {
            let combined = format!("{existing}{new_text}");
            platform::clipboard_write(&combined);
            self.last_clipboard_text = combined;
            log::info!("copy mode: appended {} bytes to clipboard", new_text.len());
        }
    }

    fn enter_copy_mode(&mut self) {
        let scale = self.scale_factor();
        let cell_w_pts = self.cell_width / scale;
        let Some((col, row, cell_h_pts)) = self.focused_cursor_cell_position() else {
            return;
        };

        let viewport_rows = if cell_h_pts > 0.0 {
            ((self.last_size.height as f64 - STATUS_BAR_HEIGHT) / cell_h_pts) as u32
        } else {
            24
        };

        let frame = self.terminal_frame();
        let viewport_cols = if cell_w_pts > 0.0 {
            (frame.size.width / cell_w_pts) as u32
        } else {
            80
        };

        let cursor_layer = platform::create_highlight_layer();
        self.copy_mode = Some(CopyModeState {
            cursor_row: self.scrollbar.offset as i64 + row,
            cursor_col: col,
            selection: SelectionMode::None,
            sel_anchor: None,
            highlight_layers: Vec::new(),
            cursor_layer,
            cell_width: cell_w_pts,
            cell_height: cell_h_pts,
            viewport_rows,
            viewport_cols,
            mark: None,
            last_jump: None,
            last_search_forward: true,
            pending_jump: None,
            show_position: false,
        });
        self.bindings.enter_copy_mode();
        self.update_copy_mode_highlight();
    }

    fn exit_copy_mode(&mut self) {
        if let Some(cm) = self.copy_mode.take() {
            platform::update_highlight_layer(cm.cursor_layer, 0.0, 0.0, 0.0, 0.0, false, false);
            for layer in &cm.highlight_layers {
                platform::update_highlight_layer(*layer, 0.0, 0.0, 0.0, 0.0, false, false);
            }
        }
        self.bindings.exit_copy_mode();
        self.ghostty_binding_action("scroll_to_bottom");
        self.ghostty_binding_action("end_search");
        self.search_active = false;
    }

    fn update_copy_mode_highlight(&mut self) {
        let Some(ref cm) = self.copy_mode else { return };

        let frame = self.terminal_frame();
        let term_y = frame.origin.y;
        let offset = self.scrollbar.offset as i64;
        let viewport_row = cm.cursor_row - offset;
        let px = cm.cursor_col as f64 * cm.cell_width;
        let py = term_y + viewport_row as f64 * cm.cell_height;

        // Always show cursor bar
        platform::update_highlight_layer(cm.cursor_layer, px, py, 2.0, cm.cell_height, true, false);

        // Compute selection rects (extracted to avoid borrow conflicts)
        let rects = if cm.selection != SelectionMode::None {
            if let Some((anchor_row, anchor_col)) = cm.sel_anchor {
                Self::compute_selection_rects_static(
                    cm.selection,
                    cm.cursor_row,
                    cm.cursor_col,
                    anchor_row,
                    anchor_col,
                    offset,
                    cm.viewport_cols,
                    cm.cell_width,
                    cm.cell_height,
                    term_y,
                )
            } else {
                vec![]
            }
        } else {
            vec![]
        };

        // Grow layer pool if needed
        let cm = self.copy_mode.as_mut().unwrap();
        while cm.highlight_layers.len() < rects.len() {
            cm.highlight_layers.push(platform::create_highlight_layer());
        }
        // Position visible layers
        for (i, &(x, y, w, h)) in rects.iter().enumerate() {
            platform::update_highlight_layer(cm.highlight_layers[i], x, y, w, h, true, true);
        }
        // Hide unused layers
        for i in rects.len()..cm.highlight_layers.len() {
            platform::update_highlight_layer(
                cm.highlight_layers[i],
                0.0,
                0.0,
                0.0,
                0.0,
                false,
                true,
            );
        }
    }

    fn compute_selection_rects_static(
        selection: SelectionMode,
        cursor_row: i64,
        cursor_col: u32,
        anchor_row: i64,
        anchor_col: u32,
        offset: i64,
        viewport_cols: u32,
        cell_width: f64,
        cell_height: f64,
        term_y: f64,
    ) -> Vec<(f64, f64, f64, f64)> {
        let (r1, c1, r2, c2) =
            if anchor_row < cursor_row || (anchor_row == cursor_row && anchor_col <= cursor_col) {
                (anchor_row, anchor_col, cursor_row, cursor_col)
            } else {
                (cursor_row, cursor_col, anchor_row, anchor_col)
            };
        let full_w = viewport_cols as f64 * cell_width;

        match selection {
            SelectionMode::Char => {
                if r1 == r2 {
                    let x = c1 as f64 * cell_width;
                    let y = term_y + (r1 - offset) as f64 * cell_height;
                    let w = (c2 as f64 - c1 as f64 + 1.0) * cell_width;
                    vec![(x, y, w, cell_height)]
                } else {
                    let mut rects = Vec::new();
                    let y1 = term_y + (r1 - offset) as f64 * cell_height;
                    rects.push((
                        c1 as f64 * cell_width,
                        y1,
                        full_w - c1 as f64 * cell_width,
                        cell_height,
                    ));
                    for r in (r1 + 1)..r2 {
                        let y = term_y + (r - offset) as f64 * cell_height;
                        rects.push((0.0, y, full_w, cell_height));
                    }
                    let y2 = term_y + (r2 - offset) as f64 * cell_height;
                    rects.push((0.0, y2, (c2 as f64 + 1.0) * cell_width, cell_height));
                    rects
                }
            }
            SelectionMode::Line => (r1..=r2)
                .map(|r| {
                    let y = term_y + (r - offset) as f64 * cell_height;
                    (0.0, y, full_w, cell_height)
                })
                .collect(),
            SelectionMode::Rectangle => {
                let min_c = c1.min(c2);
                let max_c = c1.max(c2);
                let x = min_c as f64 * cell_width;
                let w = (max_c as f64 - min_c as f64 + 1.0) * cell_width;
                (r1..=r2)
                    .map(|r| {
                        let y = term_y + (r - offset) as f64 * cell_height;
                        (x, y, w, cell_height)
                    })
                    .collect()
            }
            SelectionMode::None => vec![],
        }
    }

    fn copy_mode_copy(&mut self) {
        let Some(ref cm) = self.copy_mode else { return };
        let Some((anchor_row, anchor_col)) = cm.sel_anchor else {
            return;
        };

        let (r1, c1, r2, c2) = if anchor_row < cm.cursor_row
            || (anchor_row == cm.cursor_row && anchor_col <= cm.cursor_col)
        {
            (anchor_row, anchor_col, cm.cursor_row, cm.cursor_col)
        } else {
            (cm.cursor_row, cm.cursor_col, anchor_row, anchor_col)
        };

        // For line selection, select full lines
        let (c1, c2) = if cm.selection == SelectionMode::Line {
            (0u32, cm.viewport_cols.saturating_sub(1))
        } else {
            (c1, c2)
        };

        let sel = ffi::ghostty_selection_s {
            top_left: ffi::ghostty_point_s {
                tag: ffi::GHOSTTY_POINT_VIEWPORT,
                coord: ffi::GHOSTTY_POINT_COORD_EXACT,
                x: c1,
                y: (r1 - self.scrollbar.offset as i64).max(0) as u32,
            },
            bottom_right: ffi::ghostty_point_s {
                tag: ffi::GHOSTTY_POINT_VIEWPORT,
                coord: ffi::GHOSTTY_POINT_COORD_EXACT,
                x: c2,
                y: (r2 - self.scrollbar.offset as i64).max(0) as u32,
            },
            rectangle: cm.selection == SelectionMode::Rectangle,
        };
        if let Some(text) = self.read_surface_selection_text(sel) {
            platform::clipboard_write(&text);
            self.last_clipboard_text = text.clone();
            log::info!("copy mode: copied {} bytes", text.len());
        }
    }

}

/// For control pipe injection: compute the character for a keycode+mods combo.
fn shifted_codepoint(keycode: u32, mods: i32) -> u32 {
    let has_shift = mods & ffi::GHOSTTY_MODS_SHIFT != 0;
    #[cfg(target_os = "macos")]
    let base = match keycode {
        0x00 => 'a',
        0x01 => 's',
        0x02 => 'd',
        0x03 => 'f',
        0x04 => 'h',
        0x05 => 'g',
        0x06 => 'z',
        0x07 => 'x',
        0x08 => 'c',
        0x09 => 'v',
        0x0B => 'b',
        0x0C => 'q',
        0x0D => 'w',
        0x0E => 'e',
        0x0F => 'r',
        0x10 => 'y',
        0x11 => 't',
        0x20 => 'u',
        0x22 => 'i',
        0x1F => 'o',
        0x23 => 'p',
        0x25 => 'l',
        0x26 => 'j',
        0x28 => 'k',
        0x2D => 'n',
        0x2E => 'm',
        0x31 => ' ',
        0x24 => '\r',
        0x30 => '\t',
        0x12 => {
            if has_shift {
                '!'
            } else {
                '1'
            }
        }
        0x13 => {
            if has_shift {
                '@'
            } else {
                '2'
            }
        }
        0x14 => {
            if has_shift {
                '#'
            } else {
                '3'
            }
        }
        0x15 => {
            if has_shift {
                '$'
            } else {
                '4'
            }
        }
        0x17 => {
            if has_shift {
                '%'
            } else {
                '5'
            }
        }
        0x16 => {
            if has_shift {
                '^'
            } else {
                '6'
            }
        }
        0x1A => {
            if has_shift {
                '&'
            } else {
                '7'
            }
        }
        0x1C => {
            if has_shift {
                '*'
            } else {
                '8'
            }
        }
        0x19 => {
            if has_shift {
                '('
            } else {
                '9'
            }
        }
        0x1D => {
            if has_shift {
                ')'
            } else {
                '0'
            }
        }
        0x27 => {
            if has_shift {
                '"'
            } else {
                '\''
            }
        }
        0x2A => {
            if has_shift {
                '|'
            } else {
                '\\'
            }
        }
        0x2B => {
            if has_shift {
                '<'
            } else {
                ','
            }
        }
        0x2F => {
            if has_shift {
                '>'
            } else {
                '.'
            }
        }
        0x2C => {
            if has_shift {
                '?'
            } else {
                '/'
            }
        }
        0x29 => {
            if has_shift {
                ':'
            } else {
                ';'
            }
        }
        0x1B => {
            if has_shift {
                '_'
            } else {
                '-'
            }
        }
        0x18 => {
            if has_shift {
                '+'
            } else {
                '='
            }
        }
        0x21 => {
            if has_shift {
                '{'
            } else {
                '['
            }
        }
        0x1E => {
            if has_shift {
                '}'
            } else {
                ']'
            }
        }
        0x32 => {
            if has_shift {
                '~'
            } else {
                '`'
            }
        }
        _ => return 0,
    };
    #[cfg(target_os = "linux")]
    return shifted_codepoint_vt(keycode, mods);
    if has_shift && base.is_ascii_lowercase() {
        base.to_ascii_uppercase() as u32
    } else {
        base as u32
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn shifted_codepoint_vt(keycode: u32, mods: i32) -> u32 {
    let has_shift = mods & ffi::GHOSTTY_MODS_SHIFT != 0;
    let base = match keycode {
        20 => 'a',
        21 => 'b',
        22 => 'c',
        23 => 'd',
        24 => 'e',
        25 => 'f',
        26 => 'g',
        27 => 'h',
        28 => 'i',
        29 => 'j',
        30 => 'k',
        31 => 'l',
        32 => 'm',
        33 => 'n',
        34 => 'o',
        35 => 'p',
        36 => 'q',
        37 => 'r',
        38 => 's',
        39 => 't',
        40 => 'u',
        41 => 'v',
        42 => 'w',
        43 => 'x',
        44 => 'y',
        45 => 'z',
        63 => ' ',
        58 => '\r',
        64 => '\t',
        7 => {
            if has_shift {
                '!'
            } else {
                '1'
            }
        }
        8 => {
            if has_shift {
                '@'
            } else {
                '2'
            }
        }
        9 => {
            if has_shift {
                '#'
            } else {
                '3'
            }
        }
        10 => {
            if has_shift {
                '$'
            } else {
                '4'
            }
        }
        11 => {
            if has_shift {
                '%'
            } else {
                '5'
            }
        }
        12 => {
            if has_shift {
                '^'
            } else {
                '6'
            }
        }
        13 => {
            if has_shift {
                '&'
            } else {
                '7'
            }
        }
        14 => {
            if has_shift {
                '*'
            } else {
                '8'
            }
        }
        15 => {
            if has_shift {
                '('
            } else {
                '9'
            }
        }
        6 => {
            if has_shift {
                ')'
            } else {
                '0'
            }
        }
        48 => {
            if has_shift {
                '"'
            } else {
                '\''
            }
        }
        2 => {
            if has_shift {
                '|'
            } else {
                '\\'
            }
        }
        5 => {
            if has_shift {
                '<'
            } else {
                ','
            }
        }
        47 => {
            if has_shift {
                '>'
            } else {
                '.'
            }
        }
        50 => {
            if has_shift {
                '?'
            } else {
                '/'
            }
        }
        49 => {
            if has_shift {
                ':'
            } else {
                ';'
            }
        }
        46 => {
            if has_shift {
                '_'
            } else {
                '-'
            }
        }
        16 => {
            if has_shift {
                '+'
            } else {
                '='
            }
        }
        3 => {
            if has_shift {
                '{'
            } else {
                '['
            }
        }
        4 => {
            if has_shift {
                '}'
            } else {
                ']'
            }
        }
        1 => {
            if has_shift {
                '~'
            } else {
                '`'
            }
        }
        _ => return 0,
    };
    if has_shift && base.is_ascii_lowercase() {
        base.to_ascii_uppercase() as u32
    } else {
        base as u32
    }
}

/// Convert keycode+mods to the character it produces, or None.
fn shifted_char(keycode: u32, mods: i32) -> Option<char> {
    match shifted_codepoint(keycode, mods) {
        0 => None,
        cp => char::from_u32(cp),
    }
}

/// Parse "ctrl+c", "shift+0x27", "a" into (keycode, mods).
fn parse_keyspec(spec: &str) -> Option<(u32, i32)> {
    let mut mods: i32 = 0;
    let mut key_part = spec;
    loop {
        if let Some(rest) = key_part.strip_prefix("ctrl+") {
            mods |= ffi::GHOSTTY_MODS_CTRL;
            key_part = rest;
        } else if let Some(rest) = key_part.strip_prefix("shift+") {
            mods |= ffi::GHOSTTY_MODS_SHIFT;
            key_part = rest;
        } else if let Some(rest) = key_part.strip_prefix("alt+") {
            mods |= ffi::GHOSTTY_MODS_ALT;
            key_part = rest;
        } else if let Some(rest) = key_part.strip_prefix("super+") {
            mods |= ffi::GHOSTTY_MODS_SUPER;
            key_part = rest;
        } else {
            break;
        }
    }
    #[cfg(target_os = "macos")]
    let keycode = match key_part {
        "a" => 0x00,
        "s" => 0x01,
        "d" => 0x02,
        "f" => 0x03,
        "h" => 0x04,
        "g" => 0x05,
        "z" => 0x06,
        "x" => 0x07,
        "c" => 0x08,
        "v" => 0x09,
        "b" => 0x0B,
        "q" => 0x0C,
        "w" => 0x0D,
        "e" => 0x0E,
        "r" => 0x0F,
        "y" => 0x10,
        "t" => 0x11,
        "u" => 0x20,
        "i" => 0x22,
        "o" => 0x1F,
        "p" => 0x23,
        "l" => 0x25,
        "j" => 0x26,
        "k" => 0x28,
        "n" => 0x2D,
        "m" => 0x2E,
        "enter" | "return" => 0x24,
        "tab" => 0x30,
        "space" => 0x31,
        "escape" | "esc" => 0x35,
        "backspace" => 0x33,
        "delete" => 0x75,
        "up" => 0x7E,
        "down" => 0x7D,
        "left" => 0x7B,
        "right" => 0x7C,
        "pageup" => 0x74,
        "pagedown" => 0x79,
        "home" => 0x73,
        "end" => 0x77,
        _ if key_part.starts_with("0x") => u32::from_str_radix(&key_part[2..], 16).ok()?,
        _ => return None,
    };
    #[cfg(target_os = "linux")]
    let keycode = match key_part {
        "a" => 20,
        "b" => 21,
        "c" => 22,
        "d" => 23,
        "e" => 24,
        "f" => 25,
        "g" => 26,
        "h" => 27,
        "i" => 28,
        "j" => 29,
        "k" => 30,
        "l" => 31,
        "m" => 32,
        "n" => 33,
        "o" => 34,
        "p" => 35,
        "q" => 36,
        "r" => 37,
        "s" => 38,
        "t" => 39,
        "u" => 40,
        "v" => 41,
        "w" => 42,
        "x" => 43,
        "y" => 44,
        "z" => 45,
        "0" => 6,
        "1" => 7,
        "2" => 8,
        "3" => 9,
        "4" => 10,
        "5" => 11,
        "6" => 12,
        "7" => 13,
        "8" => 14,
        "9" => 15,
        "enter" | "return" => 58,
        "tab" => 64,
        "space" => 63,
        "escape" | "esc" => 120,
        "backspace" => 53,
        "delete" => 119,
        "up" => 111,
        "down" => 116,
        "left" => 113,
        "right" => 114,
        "pageup" => 112,
        "pagedown" => 117,
        "home" => 110,
        "end" => 115,
        _ if key_part.starts_with("0x") => u32::from_str_radix(&key_part[2..], 16).ok()?,
        _ => return None,
    };
    Some((keycode, mods))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn parse_vt_keyspec(spec: &str) -> Option<(u32, i32)> {
    let mut mods: i32 = 0;
    let mut key_part = spec;
    loop {
        if let Some(rest) = key_part.strip_prefix("ctrl+") {
            mods |= ffi::GHOSTTY_MODS_CTRL;
            key_part = rest;
        } else if let Some(rest) = key_part.strip_prefix("shift+") {
            mods |= ffi::GHOSTTY_MODS_SHIFT;
            key_part = rest;
        } else if let Some(rest) = key_part.strip_prefix("alt+") {
            mods |= ffi::GHOSTTY_MODS_ALT;
            key_part = rest;
        } else if let Some(rest) = key_part.strip_prefix("super+") {
            mods |= ffi::GHOSTTY_MODS_SUPER;
            key_part = rest;
        } else {
            break;
        }
    }
    let keycode = match key_part {
        "a" => 20,
        "b" => 21,
        "c" => 22,
        "d" => 23,
        "e" => 24,
        "f" => 25,
        "g" => 26,
        "h" => 27,
        "i" => 28,
        "j" => 29,
        "k" => 30,
        "l" => 31,
        "m" => 32,
        "n" => 33,
        "o" => 34,
        "p" => 35,
        "q" => 36,
        "r" => 37,
        "s" => 38,
        "t" => 39,
        "u" => 40,
        "v" => 41,
        "w" => 42,
        "x" => 43,
        "y" => 44,
        "z" => 45,
        "0" => 6,
        "1" => 7,
        "2" => 8,
        "3" => 9,
        "4" => 10,
        "5" => 11,
        "6" => 12,
        "7" => 13,
        "8" => 14,
        "9" => 15,
        "enter" | "return" => 58,
        "tab" => 64,
        "space" => 63,
        "escape" | "esc" => 120,
        "backspace" => 53,
        "delete" => 68,
        "up" => 73,
        "down" => 74,
        "left" => 71,
        "right" => 72,
        "pageup" => 75,
        "pagedown" => 78,
        "home" => 69,
        "end" => 70,
        _ if key_part.starts_with("0x") => u32::from_str_radix(&key_part[2..], 16).ok()?,
        _ => return None,
    };
    Some((keycode, mods))
}

fn control_key_to_keyboard_key(spec: &str, key_char: Option<char>) -> keyboard::Key {
    use keyboard::key::Named;

    let key_name = spec.rsplit_once('+').map(|(_, key)| key).unwrap_or(spec);

    match key_name {
        "enter" | "return" => keyboard::Key::Named(Named::Enter),
        "tab" => keyboard::Key::Named(Named::Tab),
        "space" => keyboard::Key::Named(Named::Space),
        "escape" | "esc" => keyboard::Key::Named(Named::Escape),
        "backspace" => keyboard::Key::Named(Named::Backspace),
        "delete" => keyboard::Key::Named(Named::Delete),
        "up" => keyboard::Key::Named(Named::ArrowUp),
        "down" => keyboard::Key::Named(Named::ArrowDown),
        "left" => keyboard::Key::Named(Named::ArrowLeft),
        "right" => keyboard::Key::Named(Named::ArrowRight),
        "pageup" => keyboard::Key::Named(Named::PageUp),
        "pagedown" => keyboard::Key::Named(Named::PageDown),
        "home" => keyboard::Key::Named(Named::Home),
        "end" => keyboard::Key::Named(Named::End),
        _ => keyboard::Key::Character(key_char.unwrap_or_default().to_string().into()),
    }
}

#[cfg(target_os = "macos")]
fn should_route_macos_vt_key_via_appkit(vt_keycode: u32, mods: i32) -> bool {
    if mods & (ffi::GHOSTTY_MODS_CTRL | ffi::GHOSTTY_MODS_SUPER) != 0 {
        return false;
    }
    matches!(
        vt_keycode,
        1..=16 | 20..=50 | 63
    )
}

#[cfg(target_os = "macos")]
fn native_keycode_to_named_key(keycode: u32) -> Option<bindings::NamedKey> {
    Some(match keycode {
        0x7E => bindings::NamedKey::ArrowUp,
        0x7D => bindings::NamedKey::ArrowDown,
        0x7B => bindings::NamedKey::ArrowLeft,
        0x7C => bindings::NamedKey::ArrowRight,
        0x74 => bindings::NamedKey::PageUp,
        0x79 => bindings::NamedKey::PageDown,
        0x73 => bindings::NamedKey::Home,
        0x77 => bindings::NamedKey::End,
        0x35 => bindings::NamedKey::Escape,
        _ => return None,
    })
}

#[cfg(target_os = "macos")]
fn native_keycode_to_keyboard_key(keycode: u32, key_char: Option<char>) -> keyboard::Key {
    use keyboard::key::Named;
    match keycode {
        0x24 => keyboard::Key::Named(Named::Enter),
        0x30 => keyboard::Key::Named(Named::Tab),
        0x31 => keyboard::Key::Named(Named::Space),
        0x33 => keyboard::Key::Named(Named::Backspace),
        0x35 => keyboard::Key::Named(Named::Escape),
        0x75 => keyboard::Key::Named(Named::Delete),
        0x7E => keyboard::Key::Named(Named::ArrowUp),
        0x7D => keyboard::Key::Named(Named::ArrowDown),
        0x7B => keyboard::Key::Named(Named::ArrowLeft),
        0x7C => keyboard::Key::Named(Named::ArrowRight),
        0x74 => keyboard::Key::Named(Named::PageUp),
        0x79 => keyboard::Key::Named(Named::PageDown),
        0x73 => keyboard::Key::Named(Named::Home),
        0x77 => keyboard::Key::Named(Named::End),
        _ => keyboard::Key::Character(key_char.unwrap_or_default().to_string().into()),
    }
}

fn ghostty_mods_to_iced(mods: i32) -> keyboard::Modifiers {
    use iced::keyboard::Modifiers;

    let mut result = Modifiers::empty();
    if mods & ffi::GHOSTTY_MODS_SHIFT != 0 {
        result.insert(Modifiers::SHIFT);
    }
    if mods & ffi::GHOSTTY_MODS_CTRL != 0 {
        result.insert(Modifiers::CTRL);
    }
    if mods & ffi::GHOSTTY_MODS_ALT != 0 {
        result.insert(Modifiers::ALT);
    }
    if mods & ffi::GHOSTTY_MODS_SUPER != 0 {
        result.insert(Modifiers::LOGO);
    }
    result
}

fn key_to_codepoint(key: &keyboard::Key) -> u32 {
    match key {
        keyboard::Key::Character(s) => s.chars().next().map(|c| c as u32).unwrap_or(0),
        keyboard::Key::Named(named) => {
            use keyboard::key::Named;
            match named {
                Named::Enter => '\r' as u32,
                Named::Tab => '\t' as u32,
                Named::Space => ' ' as u32,
                Named::Backspace => 0x08,
                Named::Escape => 0x1b,
                Named::Delete => 0x7f,
                _ => 0,
            }
        }
        _ => 0,
    }
}

fn iced_mods_to_ghostty(mods: &keyboard::Modifiers) -> ffi::ghostty_input_mods_e {
    let mut result = ffi::GHOSTTY_MODS_NONE;
    if mods.shift() {
        result |= ffi::GHOSTTY_MODS_SHIFT;
    }
    if mods.control() {
        result |= ffi::GHOSTTY_MODS_CTRL;
    }
    if mods.alt() {
        result |= ffi::GHOSTTY_MODS_ALT;
    }
    if mods.logo() {
        result |= ffi::GHOSTTY_MODS_SUPER;
    }
    result
}

fn iced_button_to_ghostty(button: mouse::Button) -> ffi::ghostty_input_mouse_button_e {
    match button {
        mouse::Button::Left => ffi::ghostty_input_mouse_button_e::GHOSTTY_MOUSE_LEFT,
        mouse::Button::Right => ffi::ghostty_input_mouse_button_e::GHOSTTY_MOUSE_RIGHT,
        mouse::Button::Middle => ffi::ghostty_input_mouse_button_e::GHOSTTY_MOUSE_MIDDLE,
        _ => ffi::ghostty_input_mouse_button_e::GHOSTTY_MOUSE_UNKNOWN,
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn iced_button_to_vt(button: mouse::Button) -> vt::GhosttyMouseButton {
    match button {
        mouse::Button::Left => vt::GHOSTTY_MOUSE_BUTTON_LEFT,
        mouse::Button::Right => vt::GHOSTTY_MOUSE_BUTTON_RIGHT,
        mouse::Button::Middle => vt::GHOSTTY_MOUSE_BUTTON_MIDDLE,
        _ => vt::GHOSTTY_MOUSE_BUTTON_UNKNOWN,
    }
}

fn split_direction_name(direction: splits::Direction) -> &'static str {
    match direction {
        splits::Direction::Horizontal => "horizontal",
        splits::Direction::Vertical => "vertical",
    }
}

fn selection_mode_name(selection: SelectionMode) -> &'static str {
    match selection {
        SelectionMode::None => "none",
        SelectionMode::Char => "character",
        SelectionMode::Line => "line",
        SelectionMode::Rectangle => "rectangle",
    }
}

fn ui_rect_snapshot(x: f64, y: f64, width: f64, height: f64) -> control::UiRectSnapshot {
    control::UiRectSnapshot {
        x,
        y,
        width,
        height,
    }
}

fn format_command_finished_message(exit_code: Option<u8>, duration_ns: u64) -> String {
    let duration_ms = duration_ns / 1_000_000;
    if let Some(exit_code) = exit_code {
        format!(
            "Command took {}.{:03}s and exited with code {}.",
            duration_ms / 1000,
            duration_ms % 1000,
            exit_code
        )
    } else {
        format!(
            "Command took {}.{:03}s.",
            duration_ms / 1000,
            duration_ms % 1000
        )
    }
}

fn command_finish_notification(
    desktop_notifications_enabled: bool,
    notify_action_enabled: bool,
    notify_on_command_finish: config::NotifyOnCommandFinish,
    app_focused: bool,
    notify_on_command_finish_after_ns: u64,
    event: CommandFinishedEvent,
) -> Option<(&'static str, String)> {
    if !desktop_notifications_enabled || !notify_action_enabled {
        return None;
    }
    match notify_on_command_finish {
        config::NotifyOnCommandFinish::Never => return None,
        config::NotifyOnCommandFinish::Unfocused if app_focused => return None,
        config::NotifyOnCommandFinish::Unfocused | config::NotifyOnCommandFinish::Always => {}
    }
    if event.duration_ns <= notify_on_command_finish_after_ns {
        return None;
    }

    let title = match event.exit_code {
        Some(0) => "Command Succeeded",
        Some(_) => "Command Failed",
        None => "Command Finished",
    };
    Some((
        title,
        format_command_finished_message(event.exit_code, event.duration_ns),
    ))
}

enum TextInputAction {
    Commit(String),
    Command(platform::TextInputCommand),
}

fn apply_text_input_event(
    preedit_text: &mut String,
    event: platform::TextInputEvent,
) -> Option<TextInputAction> {
    match event {
        platform::TextInputEvent::Commit(text) => {
            preedit_text.clear();
            Some(TextInputAction::Commit(text))
        }
        platform::TextInputEvent::Preedit(text) => {
            *preedit_text = text;
            None
        }
        platform::TextInputEvent::PreeditClear => {
            preedit_text.clear();
            None
        }
        platform::TextInputEvent::Command(command) => Some(TextInputAction::Command(command)),
    }
}

fn text_input_command_key(command: platform::TextInputCommand) -> (u32, u32) {
    match command {
        platform::TextInputCommand::Backspace => (53, 0x08),
        platform::TextInputCommand::DeleteForward => (68, 0x7f),
        platform::TextInputCommand::InsertNewline => (58, '\r' as u32),
        platform::TextInputCommand::InsertTab => (64, '\t' as u32),
    }
}

#[cfg(test)]
pub mod main_tests {
    use super::*;

    pub fn compute_rects(
        selection: SelectionMode,
        cursor_row: i64,
        cursor_col: u32,
        anchor_row: i64,
        anchor_col: u32,
        offset: i64,
        viewport_cols: u32,
        cell_width: f64,
        cell_height: f64,
        term_y: f64,
    ) -> Vec<(f64, f64, f64, f64)> {
        BooApp::compute_selection_rects_static(
            selection,
            cursor_row,
            cursor_col,
            anchor_row,
            anchor_col,
            offset,
            viewport_cols,
            cell_width,
            cell_height,
            term_y,
        )
    }

    #[test]
    fn test_fuzzy_score_prefix() {
        assert!(fuzzy_score("split", "split-right") > fuzzy_score("sr", "split-right"));
        assert!(fuzzy_score("split", "split-right") > 50);
    }

    #[test]
    fn test_fuzzy_score_initials() {
        // "sr" matches "split-right" via word initials (s...r)
        assert!(fuzzy_score("sr", "split-right") > 0);
        assert!(fuzzy_score("sd", "split-down") > 0);
        assert!(fuzzy_score("nt", "new-tab") > 0);
    }

    #[test]
    fn test_fuzzy_score_subsequence() {
        assert!(fuzzy_score("srt", "split-right") > 0);
    }

    #[test]
    fn test_fuzzy_score_no_match() {
        assert_eq!(fuzzy_score("xyz", "split-right"), 0);
        assert_eq!(fuzzy_score("zzz", "new-tab"), 0);
    }

    #[test]
    fn test_fuzzy_score_empty_query() {
        assert!(fuzzy_score("", "anything") > 0);
    }

    #[test]
    fn test_command_finish_notification_respects_disabled_settings() {
        let event = CommandFinishedEvent {
            exit_code: Some(0),
            duration_ns: 10_000_000_000,
        };
        assert!(
            command_finish_notification(
                false,
                true,
                config::NotifyOnCommandFinish::Always,
                false,
                5_000_000_000,
                event,
            )
            .is_none()
        );
        assert!(
            command_finish_notification(
                true,
                false,
                config::NotifyOnCommandFinish::Always,
                false,
                5_000_000_000,
                event,
            )
            .is_none()
        );
        assert!(
            command_finish_notification(
                true,
                true,
                config::NotifyOnCommandFinish::Never,
                false,
                5_000_000_000,
                event,
            )
            .is_none()
        );
    }

    #[test]
    fn test_command_finish_notification_respects_focus_and_threshold() {
        let event = CommandFinishedEvent {
            exit_code: Some(1),
            duration_ns: 8_000_000_000,
        };
        assert!(
            command_finish_notification(
                true,
                true,
                config::NotifyOnCommandFinish::Unfocused,
                true,
                5_000_000_000,
                event,
            )
            .is_none()
        );
        assert!(
            command_finish_notification(
                true,
                true,
                config::NotifyOnCommandFinish::Always,
                false,
                8_000_000_000,
                event,
            )
            .is_none()
        );
        let notification = command_finish_notification(
            true,
            true,
            config::NotifyOnCommandFinish::Unfocused,
            false,
            5_000_000_000,
            event,
        )
        .expect("notification should be emitted");
        assert_eq!(notification.0, "Command Failed");
        assert!(notification.1.contains("exited with code 1"));
    }

    #[test]
    fn test_apply_text_input_event_tracks_preedit_and_commit() {
        let mut preedit = String::new();
        assert!(
            apply_text_input_event(
                &mut preedit,
                platform::TextInputEvent::Preedit("kana".to_string()),
            )
            .is_none()
        );
        assert_eq!(preedit, "kana");

        let committed = apply_text_input_event(
            &mut preedit,
            platform::TextInputEvent::Commit("かな".to_string()),
        );
        match committed {
            Some(TextInputAction::Commit(text)) => assert_eq!(text, "かな"),
            _ => panic!("expected committed text"),
        }
        assert!(preedit.is_empty());

        assert!(
            apply_text_input_event(&mut preedit, platform::TextInputEvent::PreeditClear).is_none()
        );
        assert!(preedit.is_empty());
    }

    #[test]
    fn test_apply_text_input_event_forwards_text_commands() {
        let mut preedit = String::from("kana");
        let command = apply_text_input_event(
            &mut preedit,
            platform::TextInputEvent::Command(platform::TextInputCommand::Backspace),
        );
        assert_eq!(preedit, "kana");
        match command {
            Some(TextInputAction::Command(platform::TextInputCommand::Backspace)) => {}
            _ => panic!("expected backspace command"),
        }
    }

    #[test]
    fn test_parse_vt_keyspec_uses_portable_backspace_keycode() {
        let (keycode, mods) = parse_vt_keyspec("backspace").expect("backspace should parse");
        assert_eq!(keycode, 53);
        assert_eq!(mods, 0);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_should_route_macos_vt_key_via_appkit_only_for_plain_text_keys() {
        assert!(should_route_macos_vt_key_via_appkit(20, 0));
        assert!(should_route_macos_vt_key_via_appkit(63, 0));
        assert!(!should_route_macos_vt_key_via_appkit(53, 0));
        assert!(!should_route_macos_vt_key_via_appkit(64, 0));
        assert!(!should_route_macos_vt_key_via_appkit(58, 0));
        assert!(!should_route_macos_vt_key_via_appkit(
            20,
            ffi::GHOSTTY_MODS_CTRL
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_native_keycode_mapping_keeps_bindings_and_vt_in_sync() {
        assert!(matches!(
            native_keycode_to_named_key(0x7B),
            Some(bindings::NamedKey::ArrowLeft)
        ));
        assert!(matches!(
            native_keycode_to_keyboard_key(0x33, None),
            keyboard::Key::Named(keyboard::key::Named::Backspace)
        ));
        assert_eq!(crate::keymap::native_to_vt_keycode(0x33), Some(53));
        assert_eq!(crate::keymap::native_to_vt_keycode(0x24), Some(58));
    }

    #[test]
    fn test_fuzzy_score_ranking() {
        // Prefix match should beat initials
        let prefix = fuzzy_score("split", "split-right");
        let initials = fuzzy_score("sr", "split-right");
        let subseq = fuzzy_score("srt", "split-right");
        assert!(prefix > initials);
        assert!(initials > subseq);
    }

    #[test]
    fn test_command_prompt_suggestions() {
        let mut cp = CommandPrompt::new();
        cp.input = "split".to_string();
        cp.update_suggestions();
        // Should find split-right, split-down, split-left, split-up
        assert!(cp.suggestions.len() >= 4);
        // First suggestion should be a split command
        let first_cmd = &COMMANDS[cp.suggestions[0]];
        assert!(first_cmd.name.starts_with("split"));
    }

    #[test]
    fn test_command_prompt_suggestions_initials() {
        let mut cp = CommandPrompt::new();
        cp.input = "nt".to_string();
        cp.update_suggestions();
        // Should find "new-tab" and "next-tab"
        let names: Vec<&str> = cp.suggestions.iter().map(|&i| COMMANDS[i].name).collect();
        assert!(names.contains(&"new-tab") || names.contains(&"next-tab"));
    }

    #[test]
    fn test_command_prompt_empty_shows_all() {
        let mut cp = CommandPrompt::new();
        cp.input = "".to_string();
        cp.update_suggestions();
        assert_eq!(cp.suggestions.len(), COMMANDS.len());
    }

    #[test]
    fn test_command_prompt_no_match() {
        let mut cp = CommandPrompt::new();
        cp.input = "xyzxyz".to_string();
        cp.update_suggestions();
        assert!(cp.suggestions.is_empty());
    }

    #[test]
    fn test_command_prompt_tab_accepts() {
        let mut cp = CommandPrompt::new();
        cp.input = "split-r".to_string();
        cp.update_suggestions();
        if let Some(cmd) = cp.selected_command() {
            cp.input = cmd.name.to_string();
            if !cmd.args.is_empty() {
                cp.input.push(' ');
            }
        }
        assert_eq!(cp.input, "split-right");
    }

    #[test]
    fn test_command_prompt_history() {
        let mut cp = CommandPrompt::new();
        cp.history.push("split-right".to_string());
        cp.history.push("new-tab".to_string());

        // Navigate up
        cp.history_idx = Some(cp.history.len() - 1);
        cp.input = cp.history[cp.history_idx.unwrap()].clone();
        assert_eq!(cp.input, "new-tab");

        // Navigate up again
        cp.history_idx = Some(0);
        cp.input = cp.history[0].clone();
        assert_eq!(cp.input, "split-right");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_ui_terminal_snapshot_from_linux_preserves_cells_and_cursor() {
        let snapshot = vt_backend_core::TerminalSnapshot {
            cols: 2,
            rows: 1,
            title: "shell".to_string(),
            pwd: "/tmp".to_string(),
            cursor: vt_backend_core::CursorSnapshot {
                visible: true,
                x: 1,
                y: 0,
                style: 2,
            },
            rows_data: vec![vec![
                vt_backend_core::CellSnapshot {
                    text: "a".to_string(),
                    display_width: 1,
                    fg: vt::GhosttyColorRgb { r: 1, g: 2, b: 3 },
                    bg: vt::GhosttyColorRgb { r: 4, g: 5, b: 6 },
                    bold: true,
                    italic: false,
                    underline: 1,
                },
                vt_backend_core::CellSnapshot {
                    text: "b".to_string(),
                    display_width: 1,
                    fg: vt::GhosttyColorRgb { r: 7, g: 8, b: 9 },
                    bg: vt::GhosttyColorRgb {
                        r: 10,
                        g: 11,
                        b: 12,
                    },
                    bold: false,
                    italic: true,
                    underline: 0,
                },
            ]],
            scrollbar: vt::GhosttyTerminalScrollbar {
                total: 1,
                offset: 0,
                len: 1,
            },
            colors: vt::GhosttyRenderStateColors::default(),
        };

        let ui = vt_snapshot::ui_terminal_snapshot(&snapshot);

        assert_eq!(ui.cols, 2);
        assert_eq!(ui.rows, 1);
        assert_eq!(ui.cursor.x, 1);
        assert_eq!(ui.rows_data[0].cells[0].text, "a");
        assert_eq!(ui.rows_data[0].cells[0].display_width, 1);
        assert_eq!(ui.rows_data[0].cells[0].fg, [1, 2, 3]);
        assert!(ui.rows_data[0].cells[0].bold);
        assert!(ui.rows_data[0].cells[1].italic);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_linux_snapshot_selection_text_trims_non_rectangular_lines() {
        let snapshot = vt_backend_core::TerminalSnapshot {
            cols: 4,
            rows: 2,
            title: String::new(),
            pwd: String::new(),
            cursor: vt_backend_core::CursorSnapshot::default(),
            rows_data: vec![
                vec![
                    vt_backend_core::CellSnapshot {
                        text: "a".into(),
                        ..Default::default()
                    },
                    vt_backend_core::CellSnapshot {
                        text: "b".into(),
                        ..Default::default()
                    },
                    vt_backend_core::CellSnapshot::default(),
                    vt_backend_core::CellSnapshot::default(),
                ],
                vec![
                    vt_backend_core::CellSnapshot {
                        text: "c".into(),
                        ..Default::default()
                    },
                    vt_backend_core::CellSnapshot::default(),
                    vt_backend_core::CellSnapshot::default(),
                    vt_backend_core::CellSnapshot {
                        text: "d".into(),
                        ..Default::default()
                    },
                ],
            ],
            scrollbar: vt::GhosttyTerminalScrollbar::default(),
            colors: vt::GhosttyRenderStateColors::default(),
        };

        let text = vt_snapshot::selection_text(
            &snapshot,
            ffi::ghostty_selection_s {
                top_left: ffi::ghostty_point_s {
                    tag: ffi::GHOSTTY_POINT_VIEWPORT,
                    coord: ffi::GHOSTTY_POINT_COORD_EXACT,
                    x: 0,
                    y: 0,
                },
                bottom_right: ffi::ghostty_point_s {
                    tag: ffi::GHOSTTY_POINT_VIEWPORT,
                    coord: ffi::GHOSTTY_POINT_COORD_EXACT,
                    x: 3,
                    y: 1,
                },
                rectangle: false,
            },
        );

        assert_eq!(text, "ab\nc  d");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_linux_snapshot_selection_text_preserves_rectangle_width() {
        let snapshot = vt_backend_core::TerminalSnapshot {
            cols: 3,
            rows: 2,
            title: String::new(),
            pwd: String::new(),
            cursor: vt_backend_core::CursorSnapshot::default(),
            rows_data: vec![
                vec![
                    vt_backend_core::CellSnapshot {
                        text: "a".into(),
                        ..Default::default()
                    },
                    vt_backend_core::CellSnapshot::default(),
                    vt_backend_core::CellSnapshot {
                        text: "b".into(),
                        ..Default::default()
                    },
                ],
                vec![
                    vt_backend_core::CellSnapshot::default(),
                    vt_backend_core::CellSnapshot {
                        text: "c".into(),
                        ..Default::default()
                    },
                    vt_backend_core::CellSnapshot::default(),
                ],
            ],
            scrollbar: vt::GhosttyTerminalScrollbar::default(),
            colors: vt::GhosttyRenderStateColors::default(),
        };

        let text = vt_snapshot::selection_text(
            &snapshot,
            ffi::ghostty_selection_s {
                top_left: ffi::ghostty_point_s {
                    tag: ffi::GHOSTTY_POINT_VIEWPORT,
                    coord: ffi::GHOSTTY_POINT_COORD_EXACT,
                    x: 0,
                    y: 0,
                },
                bottom_right: ffi::ghostty_point_s {
                    tag: ffi::GHOSTTY_POINT_VIEWPORT,
                    coord: ffi::GHOSTTY_POINT_COORD_EXACT,
                    x: 2,
                    y: 1,
                },
                rectangle: true,
            },
        );

        assert_eq!(text, "a b\n c ");
    }
}
