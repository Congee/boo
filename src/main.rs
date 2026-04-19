mod app_helpers;
mod app_input;
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
mod remote_client;
mod remote_identity;
mod remote_quic;
mod remote_transport;
mod remote_types;
mod remote_wire;
mod runtime;
mod runtime_copy;
mod runtime_input;
mod runtime_panes;
mod runtime_server;
mod runtime_ui;
mod server;
mod session;
mod splits;
mod status_components;
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

#[cfg(all(target_os = "macos", test))]
use app_helpers::native_keycode_to_keyboard_key;
use app_helpers::{
    TextInputAction, apply_text_input_event, command_finish_notification,
    control_key_to_keyboard_key, cursor_blink_visible, ghostty_mods_to_iced,
    iced_button_to_ghostty, iced_button_to_vt, iced_mods_to_ghostty, native_keycode_to_named_key,
    parse_keyspec, parse_vt_keyspec, shifted_char, shifted_codepoint, shifted_codepoint_vt,
    should_route_macos_vt_key_via_appkit, split_direction_name, text_input_command_key,
    ui_rect_snapshot,
};
pub(crate) use app_input::{AppKeyEvent, AppMouseButton, AppMouseEvent};
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
use status_components::StatusComponentStore;
use std::ffi::{CStr, CString, c_void};
#[cfg(target_os = "linux")]
use std::process::Command;
use std::ptr;

static SCROLL_RX: std::sync::OnceLock<
    std::sync::Mutex<std::sync::mpsc::Receiver<platform::ScrollEvent>>,
> = std::sync::OnceLock::new();
static KEY_EVENT_RX: std::sync::OnceLock<
    std::sync::Mutex<std::sync::mpsc::Receiver<platform::KeyEvent>>,
> = std::sync::OnceLock::new();
static TEXT_INPUT_RX: std::sync::OnceLock<
    std::sync::Mutex<std::sync::mpsc::Receiver<platform::TextInputEvent>>,
> = std::sync::OnceLock::new();
struct HeadlessWakeGate {
    pending: std::sync::Mutex<bool>,
    condvar: std::sync::Condvar,
}

static HEADLESS_WAKE_GATE: std::sync::OnceLock<HeadlessWakeGate> = std::sync::OnceLock::new();

#[derive(Clone, Copy)]
struct CommandFinishedEvent {
    exit_code: Option<u8>,
    duration_ns: u64,
}

const DEFAULT_TERMINAL_FONT_SIZE: f32 = 14.0;
const DEFAULT_BACKGROUND_OPACITY: f32 = 1.0;
const HEADLESS_WIDTH: f32 = 1024.0;
const HEADLESS_HEIGHT: f32 = 768.0;
const DEFAULT_TERMINAL_FOREGROUND: config::RgbColor = [0xF0, 0xF0, 0xF0];
const DEFAULT_TERMINAL_BACKGROUND: config::RgbColor = [0x00, 0x00, 0x00];
const DEFAULT_CURSOR_COLOR: config::RgbColor = [0xFF, 0xFF, 0xFF];
const DEFAULT_SELECTION_BACKGROUND: config::RgbColor = [0xA6, 0xB8, 0xF2];
const DEFAULT_SELECTION_FOREGROUND: config::RgbColor = DEFAULT_TERMINAL_BACKGROUND;
const DEFAULT_CURSOR_TEXT_COLOR: config::RgbColor = DEFAULT_TERMINAL_BACKGROUND;
const DEFAULT_URL_COLOR: config::RgbColor = DEFAULT_TERMINAL_FOREGROUND;
const DEFAULT_ACTIVE_TAB_FOREGROUND: config::RgbColor = [0xEB, 0xEB, 0xEB];
const DEFAULT_ACTIVE_TAB_BACKGROUND: config::RgbColor = [0x3D, 0x52, 0x9E];
const DEFAULT_INACTIVE_TAB_FOREGROUND: config::RgbColor = [0xB8, 0xB8, 0xB8];
const DEFAULT_INACTIVE_TAB_BACKGROUND: config::RgbColor = [0x1A, 0x1A, 0x1A];

fn install_headless_waker() {
    let _ = HEADLESS_WAKE_GATE.set(HeadlessWakeGate {
        pending: std::sync::Mutex::new(false),
        condvar: std::sync::Condvar::new(),
    });
}

pub(crate) fn wait_for_headless_wakeup() {
    let Some(gate) = HEADLESS_WAKE_GATE.get() else {
        return;
    };
    let Ok(mut pending) = gate.pending.lock() else {
        return;
    };
    while !*pending {
        let Ok(next_pending) = gate.condvar.wait(pending) else {
            return;
        };
        pending = next_pending;
    }
    *pending = false;
}

pub(crate) fn notify_headless_wakeup() {
    if let Some(gate) = HEADLESS_WAKE_GATE.get()
        && let Ok(mut pending) = gate.pending.lock()
    {
        *pending = true;
        gate.condvar.notify_one();
    }
}

#[derive(Debug)]
struct ResolvedAppearance {
    font_families: Vec<&'static str>,
    font_size: f32,
    background_opacity: f32,
    background_opacity_cells: bool,
    terminal_foreground: config::RgbColor,
    terminal_background: config::RgbColor,
    terminal_palette: [Option<config::RgbColor>; 16],
    cursor_color: config::RgbColor,
    selection_background: config::RgbColor,
    selection_foreground: config::RgbColor,
    cursor_text_color: config::RgbColor,
    url_color: config::RgbColor,
    active_tab_foreground: config::RgbColor,
    active_tab_background: config::RgbColor,
    inactive_tab_foreground: config::RgbColor,
    inactive_tab_background: config::RgbColor,
    cursor_style: Option<i32>,
    cursor_blink: bool,
    cursor_blink_interval: std::time::Duration,
    #[cfg(target_os = "linux")]
    font_bytes: Option<Vec<u8>>,
}

pub(crate) fn leak_font_family(name: &str) -> &'static str {
    static FONT_FAMILY_INTERNER: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, &'static str>>,
    > = std::sync::OnceLock::new();
    let interner = FONT_FAMILY_INTERNER
        .get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    let mut interner = interner.lock().expect("font family interner poisoned");
    if let Some(existing) = interner.get(name) {
        return existing;
    }
    let leaked = Box::leak(name.to_owned().into_boxed_str());
    interner.insert(name.to_owned(), leaked);
    leaked
}

fn platform_default_font_fallbacks(primary_family: Option<&str>) -> Vec<&'static str> {
    #[cfg(target_os = "macos")]
    {
        crate::platform::default_font_fallbacks(primary_family)
            .into_iter()
            .map(|family| leak_font_family(&family))
            .collect()
    }

    #[cfg(target_os = "linux")]
    {
        resolve_linux_font_fallbacks(primary_family.unwrap_or("monospace"))
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = primary_family;
        Vec::new()
    }
}

#[cfg(target_os = "linux")]
fn resolve_linux_font(name: &str) -> (Option<&'static str>, Option<Vec<u8>>) {
    fn query_font(pattern: &str) -> Option<(String, String, i32)> {
        let output = Command::new("fc-match")
            .args(["-f", "%{family[0]}|%{file}|%{spacing}\n", pattern])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let line = stdout.trim();
        let mut parts = line.split('|');
        let family = parts.next()?.trim().to_string();
        let file = parts.next()?.trim().to_string();
        let spacing = parts
            .next()
            .and_then(|value| value.trim().parse::<i32>().ok())
            .unwrap_or_default();
        Some((family, file, spacing))
    }

    let primary = query_font(name);
    let mono_pattern = format!("{name}:spacing=100");
    let monospace = query_font(&mono_pattern).or_else(|| query_font("monospace"));

    let Some((resolved_family, resolved_file, resolved_spacing)) = primary else {
        log::warn!("failed to run fc-match for font family {:?}", name);
        return (Some(leak_font_family(name)), None);
    };

    let (resolved_family, resolved_file, resolved_spacing) = if resolved_spacing >= 100 {
        (resolved_family, resolved_file, resolved_spacing)
    } else if let Some((mono_family, mono_file, mono_spacing)) = monospace {
        log::warn!(
            "resolved font family {:?} to proportional {:?}; using monospace fallback {:?}",
            name,
            resolved_family,
            mono_family
        );
        (mono_family, mono_file, mono_spacing)
    } else {
        (resolved_family, resolved_file, resolved_spacing)
    };

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
        leak_font_family(&resolved_family)
    };

    if resolved_spacing > 0 && resolved_spacing < 100 {
        log::warn!(
            "resolved font family {:?} is not monospace (spacing={})",
            family,
            resolved_spacing
        );
    }

    let font_bytes = if resolved_file.is_empty() {
        None
    } else {
        match std::fs::read(&resolved_file) {
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

#[cfg(target_os = "linux")]
fn resolve_linux_font_fallbacks(name: &str) -> Vec<&'static str> {
    let output = Command::new("fc-match")
        .args(["--sort", "-f", "%{family[0]}\n", name])
        .output();

    let Ok(output) = output else {
        log::warn!(
            "failed to run fc-match --sort for font fallbacks {:?}",
            name
        );
        return Vec::new();
    };

    if !output.status.success() {
        log::warn!("fc-match --sort failed for font fallbacks {:?}", name);
        return Vec::new();
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut seen = std::collections::HashSet::new();
    let mut families = Vec::new();
    seen.insert(name.to_ascii_lowercase());
    for line in stdout.lines() {
        let family = line.trim();
        if family.is_empty() {
            continue;
        }
        let key = family.to_ascii_lowercase();
        if seen.insert(key) {
            families.push(leak_font_family(family));
        }
    }
    families
}

#[cfg(target_os = "linux")]
fn measured_linux_terminal_metrics(primary_family: Option<&str>, font_size: f32) -> Option<(f64, f64)> {
    use std::collections::HashMap;
    static CACHE: std::sync::OnceLock<std::sync::Mutex<HashMap<(String, u32), (f64, f64)>>> =
        std::sync::OnceLock::new();
    let family = primary_family.unwrap_or("monospace");
    let size_key = font_size.to_bits();
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
    if let Ok(cache) = cache.lock()
        && let Some(metrics) = cache.get(&(family.to_string(), size_key)).copied()
    {
        return Some(metrics);
    }

    let pattern = format!("{family}:spacing=100");
    let output = Command::new("fc-match")
        .args(["-f", "%{file}\n", &pattern])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let font_path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if font_path.is_empty() {
        return None;
    }
    let bytes = std::fs::read(&font_path).ok()?;
    let face = ttf_parser::Face::parse(&bytes, 0).ok()?;
    let units_per_em = f64::from(face.units_per_em());
    if units_per_em <= 0.0 {
        return None;
    }

    let glyph = ['M', 'W', '0']
        .into_iter()
        .find_map(|ch| face.glyph_index(ch).and_then(|id| face.glyph_hor_advance(id)))
        .map(f64::from)?;
    let height_units =
        f64::from(face.ascender()) - f64::from(face.descender()) + f64::from(face.line_gap());
    let size = font_size.max(1.0) as f64;
    let metrics = (
        (glyph * size / units_per_em).round().max(1.0),
        (height_units * size / units_per_em).round().max(1.0),
    );
    if let Ok(mut cache) = cache.lock() {
        cache.insert((family.to_string(), size_key), metrics);
    }
    Some(metrics)
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct StatusBarMetrics {
    pub cell_width: f64,
    pub height: f64,
    pub text_size: f32,
}

pub(crate) fn terminal_metrics(font_size: f32, primary_family: Option<&str>) -> (f64, f64) {
    #[cfg(target_os = "linux")]
    if let Some(metrics) = measured_linux_terminal_metrics(primary_family, font_size) {
        return metrics;
    }

    let _ = primary_family;
    let size = font_size.max(1.0) as f64;
    let cell_width = size.ceil().max(1.0);
    let cell_height = size.ceil().max(1.0);
    (cell_width, cell_height)
}

pub(crate) fn status_bar_metrics(font_size: f32, primary_family: Option<&str>) -> StatusBarMetrics {
    let (cell_width, cell_height) = terminal_metrics(font_size, primary_family);
    StatusBarMetrics {
        cell_width,
        height: cell_height,
        text_size: font_size.max(1.0),
    }
}

#[allow(dead_code)]
fn configured_font(family: Option<&'static str>) -> Font {
    family.map(Font::with_name).unwrap_or(Font::MONOSPACE)
}

fn main() {
    env_logger::init();
    // Install the ring-based rustls crypto provider as the process-wide default
    // before any rustls code path runs. See remote::install_default_crypto_provider
    // for the full reasoning around future rustls 0.24+ behavior.
    remote_identity::install_default_crypto_provider();

    let cli = cli::Cli::parse_args();
    let server_mode = launch::parse_startup_args(&cli);
    let startup_config = launch::load_startup_config();
    match cli::handle_command(&cli, &startup_config, launch::ensure_server_running) {
        cli::Outcome::Continue => {}
        cli::Outcome::Exit(code) => std::process::exit(code),
    }
    let headless = server_mode || cli.global.headless;
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
    mouse_selection: Option<MouseSelectionState>,
    mouse_selection_drag_active: bool,
    command_prompt: CommandPrompt,
    terminal_font_families: Vec<&'static str>,
    terminal_font_size: f32,
    background_opacity: f32,
    background_opacity_cells: bool,
    terminal_foreground: config::RgbColor,
    terminal_background: config::RgbColor,
    terminal_palette: [Option<config::RgbColor>; 16],
    cursor_color: config::RgbColor,
    selection_background: config::RgbColor,
    selection_foreground: config::RgbColor,
    cursor_text_color: config::RgbColor,
    url_color: config::RgbColor,
    active_tab_foreground: config::RgbColor,
    active_tab_background: config::RgbColor,
    inactive_tab_foreground: config::RgbColor,
    inactive_tab_background: config::RgbColor,
    cursor_style: Option<i32>,
    cursor_blink: bool,
    cursor_blink_interval: std::time::Duration,
    cursor_blink_epoch: std::time::Instant,
    appearance_revision: u64,
    surface_initialized_once: bool,
    app_focused: bool,
    dirty_remote_sessions: Vec<u32>,
    cached_remote_sessions: Option<std::sync::Arc<[remote::RemoteSessionInfo]>>,
    desktop_notifications_enabled: bool,
    notify_on_command_finish: config::NotifyOnCommandFinish,
    notify_on_command_finish_action: config::NotifyOnCommandFinishAction,
    notify_on_command_finish_after_ns: u64,
    status_components: StatusComponentStore,
    #[cfg(target_os = "linux")]
    pending_font_bytes: Option<Vec<u8>>,
}

#[derive(Clone, Copy)]
struct MouseSelectionState {
    pane_id: u64,
    anchor_row: i64,
    anchor_col: u32,
    cursor_row: i64,
    cursor_col: u32,
}

impl MouseSelectionState {
    fn has_range(self) -> bool {
        self.anchor_row != self.cursor_row || self.anchor_col != self.cursor_col
    }

    fn same_as(self, other: Self) -> bool {
        self.pane_id == other.pane_id
            && self.anchor_row == other.anchor_row
            && self.anchor_col == other.anchor_col
            && self.cursor_row == other.cursor_row
            && self.cursor_col == other.cursor_col
    }
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
                blinking: false,
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
                    bg_is_default: false,
                    bold: true,
                    italic: false,
                    underline: 1,
                    hyperlink: false,
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
                    bg_is_default: false,
                    bold: false,
                    italic: true,
                    underline: 0,
                    hyperlink: false,
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
