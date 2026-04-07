mod app_helpers;
mod backend;
mod bindings;
mod cli;
mod client_gui;
mod command_prompt;
mod config;
mod control;
mod copy_mode;
mod ffi;
mod keymap;
mod launch;
#[cfg(target_os = "macos")]
mod macos_vt_backend;
mod pane;
mod platform;
mod profiling;
mod remote;
mod runtime;
mod runtime_copy;
mod runtime_input;
mod runtime_panes;
mod runtime_server;
mod runtime_ui;
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

use app_helpers::{
    TextInputAction, apply_text_input_event, command_finish_notification,
    control_key_to_keyboard_key, ghostty_mods_to_iced, iced_button_to_ghostty, iced_button_to_vt,
    iced_mods_to_ghostty, key_to_codepoint, native_keycode_to_keyboard_key,
    native_keycode_to_named_key, parse_keyspec, parse_vt_keyspec, shifted_char, shifted_codepoint,
    shifted_codepoint_vt, should_route_macos_vt_key_via_appkit, split_direction_name,
    text_input_command_key, ui_rect_snapshot,
};
use backend::TerminalBackend;
use command_prompt::CommandPrompt;
#[cfg(test)]
use command_prompt::fuzzy_score;
pub use copy_mode::SelectionMode;
use copy_mode::{CopyModeState, JumpKind, WordMoveKind, selection_mode_name};
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
    paste_buffers: Vec<String>,
    marked_pane_id: Option<crate::pane::PaneId>,
    display_panes_active: bool,
    choose_buffer_active: bool,
    choose_buffer_selected: usize,
    choose_tree_active: bool,
    choose_tree_selected: usize,
    find_window_active: bool,
    find_window_query: String,
    find_window_selected: usize,
    copy_mode: Option<CopyModeState>,
    command_prompt: CommandPrompt,
    terminal_font_family: Option<&'static str>,
    terminal_font_size: f32,
    background_opacity: f32,
    background_opacity_cells: bool,
    appearance_revision: u64,
    surface_initialized_once: bool,
    app_focused: bool,
    remote_dirty: bool,
    desktop_notifications_enabled: bool,
    notify_on_command_finish: config::NotifyOnCommandFinish,
    notify_on_command_finish_action: config::NotifyOnCommandFinishAction,
    notify_on_command_finish_after_ns: u64,
    #[cfg(target_os = "linux")]
    pending_font_bytes: Option<Vec<u8>>,
}

#[derive(Debug, Clone)]
enum Message {
    Frame,
    #[cfg(target_os = "linux")]
    FontLoaded,
    #[allow(dead_code)]
    IcedEvent(Event),
}

#[cfg(test)]
pub mod main_tests {
    use super::*;
    use crate::command_prompt::COMMANDS;

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
            row_revisions: vec![1],
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
            row_revisions: vec![1, 1],
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
            row_revisions: vec![1, 1],
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
