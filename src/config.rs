//! Boo configuration parsed from ~/.config/boo/config.boo.
//!
//! Single config file using ghostty's key=value format.
//! On Linux, Boo now consumes the visual settings it needs directly because the
//! terminal runtime is `libghostty-vt`, not the full Ghostty surface runtime.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub type RgbColor = [u8; 3];

#[derive(Debug, Clone)]
pub struct Config {
    pub prefix_key: Option<String>,
    pub control_socket: Option<String>,
    pub remote_port: Option<u16>,
    pub remote_auth_key: Option<String>,
    pub keybinds: HashMap<String, String>,
    pub font_family: Option<String>,
    pub font_fallbacks: Vec<String>,
    pub font_size: Option<f32>,
    pub sync_to_monitor: bool,
    pub window_decoration: WindowDecoration,
    pub background_opacity: Option<f32>,
    pub background_opacity_cells: bool,
    pub foreground: Option<RgbColor>,
    pub background: Option<RgbColor>,
    pub palette: [Option<RgbColor>; 16],
    pub cursor_color: Option<RgbColor>,
    pub selection_background: Option<RgbColor>,
    pub selection_foreground: Option<RgbColor>,
    pub cursor_text_color: Option<RgbColor>,
    pub url_color: Option<RgbColor>,
    pub active_tab_foreground: Option<RgbColor>,
    pub active_tab_background: Option<RgbColor>,
    pub inactive_tab_foreground: Option<RgbColor>,
    pub inactive_tab_background: Option<RgbColor>,
    pub cursor_style: Option<CursorStyle>,
    pub cursor_blink: bool,
    pub cursor_blink_interval_ns: u64,
    pub desktop_notifications: bool,
    pub notify_on_command_finish: NotifyOnCommandFinish,
    pub notify_on_command_finish_action: NotifyOnCommandFinishAction,
    pub notify_on_command_finish_after_ns: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorStyle {
    Block,
    Bar,
    Underline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowDecoration {
    None,
    TitleBar,
}

impl WindowDecoration {
    pub fn shows_system_decorations(self) -> bool {
        matches!(self, WindowDecoration::TitleBar)
    }
}

impl CursorStyle {
    pub fn vt_visual_style(self) -> i32 {
        match self {
            CursorStyle::Bar => crate::vt::GHOSTTY_RENDER_STATE_CURSOR_VISUAL_STYLE_BAR,
            CursorStyle::Block => crate::vt::GHOSTTY_RENDER_STATE_CURSOR_VISUAL_STYLE_BLOCK,
            CursorStyle::Underline => crate::vt::GHOSTTY_RENDER_STATE_CURSOR_VISUAL_STYLE_UNDERLINE,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotifyOnCommandFinish {
    Never,
    Unfocused,
    Always,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotifyOnCommandFinishAction {
    pub bell: bool,
    pub notify: bool,
}

impl Config {
    pub fn load() -> Config {
        let path = config_path();
        if !path.exists() {
            log::warn!("boo config not found at {}", path.display());
            return Config::default();
        }
        let content = match load_with_includes(&path, &mut HashSet::new()) {
            Ok(c) => c,
            Err(e) => {
                log::error!("failed to read {}: {e}", path.display());
                return Config::default();
            }
        };
        Self::parse(&content)
    }

    pub fn parse(content: &str) -> Config {
        let mut config = Config::default();
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim();
            let value = strip_quotes(value.trim());
            match key {
                "prefix-key" => config.prefix_key = Some(value.to_string()),
                "control-socket" => config.control_socket = Some(value.to_string()),
                "remote-port" => {
                    if let Ok(port) = value.parse::<u16>() {
                        config.remote_port = Some(port);
                    }
                }
                "remote-auth-key" => {
                    if !value.is_empty() {
                        config.remote_auth_key = Some(value.to_string());
                    }
                }
                "font-family" => {
                    if value.is_empty() {
                        config.font_family = None;
                        config.font_fallbacks.clear();
                    } else if config.font_family.is_none() {
                        config.font_family = Some(value.to_string());
                    } else {
                        config.font_fallbacks.push(value.to_string());
                    }
                }
                "font-size" => {
                    if let Ok(size) = value.parse::<f32>() {
                        config.font_size = Some(size.max(1.0));
                    }
                }
                "sync-to-monitor" => {
                    config.sync_to_monitor = parse_bool(value).unwrap_or(true);
                }
                "window-decoration" => {
                    if let Some(decoration) = parse_window_decoration(value) {
                        config.window_decoration = decoration;
                    }
                }
                "background-opacity" => {
                    if let Ok(opacity) = value.parse::<f32>() {
                        config.background_opacity = Some(opacity.clamp(0.0, 1.0));
                    }
                }
                "background-opacity-cells" => {
                    config.background_opacity_cells = parse_bool(value).unwrap_or(false);
                }
                "foreground" => config.foreground = parse_rgb_color(value),
                "background" => config.background = parse_rgb_color(value),
                "cursor" | "cursor-color" => config.cursor_color = parse_rgb_color(value),
                "cursor_text_color" | "cursor-text-color" => {
                    config.cursor_text_color = parse_rgb_color(value);
                }
                "selection_background" | "selection-background" => {
                    config.selection_background = parse_rgb_color(value);
                }
                "selection_foreground" | "selection-foreground" => {
                    config.selection_foreground = parse_rgb_color(value);
                }
                "url_color" | "url-color" => {
                    config.url_color = parse_rgb_color(value);
                }
                "active_tab_foreground" | "active-tab-foreground" => {
                    config.active_tab_foreground = parse_rgb_color(value);
                }
                "active_tab_background" | "active-tab-background" => {
                    config.active_tab_background = parse_rgb_color(value);
                }
                "inactive_tab_foreground" | "inactive-tab-foreground" => {
                    config.inactive_tab_foreground = parse_rgb_color(value);
                }
                "inactive_tab_background" | "inactive-tab-background" => {
                    config.inactive_tab_background = parse_rgb_color(value);
                }
                "cursor-style" => {
                    config.cursor_style = parse_cursor_style(value);
                }
                "cursor-blink" => {
                    config.cursor_blink = parse_bool(value).unwrap_or(true);
                }
                "cursor-blink-interval" => {
                    if let Some(duration) = parse_duration_ns(value) {
                        config.cursor_blink_interval_ns = duration;
                    }
                }
                "desktop-notifications" => {
                    config.desktop_notifications = parse_bool(value).unwrap_or(true);
                }
                "notify-on-command-finish" => {
                    config.notify_on_command_finish = match value {
                        "never" => NotifyOnCommandFinish::Never,
                        "unfocused" => NotifyOnCommandFinish::Unfocused,
                        "always" => NotifyOnCommandFinish::Always,
                        _ => NotifyOnCommandFinish::Never,
                    };
                }
                "notify-on-command-finish-action" => {
                    config.notify_on_command_finish_action =
                        parse_notify_on_command_finish_action(value);
                }
                "notify-on-command-finish-after" => {
                    if let Some(duration) = parse_duration_ns(value) {
                        config.notify_on_command_finish_after_ns = duration;
                    }
                }
                "keybind" => {
                    // Format: keybind = <key>=<action>
                    if let Some((bind_key, action)) = value.split_once('=') {
                        config.keybinds.insert(
                            strip_quotes(bind_key.trim()).to_string(),
                            strip_quotes(action.trim()).to_string(),
                        );
                    }
                }
                _ => {
                    if let Some(index) = parse_palette_key(key) {
                        config.palette[index] = parse_rgb_color(value);
                    }
                }
            }
        }
        log::info!("loaded boo config from {} lines", content.lines().count());
        config
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            prefix_key: None,
            control_socket: None,
            remote_port: None,
            remote_auth_key: None,
            keybinds: HashMap::new(),
            font_family: None,
            font_fallbacks: Vec::new(),
            font_size: None,
            sync_to_monitor: true,
            window_decoration: WindowDecoration::None,
            background_opacity: None,
            background_opacity_cells: false,
            foreground: None,
            background: None,
            palette: [None; 16],
            cursor_color: None,
            selection_background: None,
            selection_foreground: None,
            cursor_text_color: None,
            url_color: None,
            active_tab_foreground: None,
            active_tab_background: None,
            inactive_tab_foreground: None,
            inactive_tab_background: None,
            cursor_style: None,
            cursor_blink: true,
            cursor_blink_interval_ns: 600_000_000,
            desktop_notifications: true,
            notify_on_command_finish: NotifyOnCommandFinish::Never,
            notify_on_command_finish_action: NotifyOnCommandFinishAction {
                bell: true,
                notify: false,
            },
            notify_on_command_finish_after_ns: 5 * 1_000_000_000,
        }
    }
}

fn load_with_includes(
    path: &Path,
    visited: &mut HashSet<PathBuf>,
) -> Result<String, std::io::Error> {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if !visited.insert(canonical.clone()) {
        return Ok(String::new());
    }
    let content = std::fs::read_to_string(&canonical)?;
    let mut expanded = String::new();
    let base_dir = canonical.parent().unwrap_or_else(|| Path::new("."));
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("include ") {
            let include_path = strip_quotes(rest.trim());
            let candidate = Path::new(include_path);
            let include_path = if candidate.is_absolute() {
                candidate.to_path_buf()
            } else {
                base_dir.join(candidate)
            };
            expanded.push_str(&load_with_includes(&include_path, visited)?);
            if !expanded.ends_with('\n') {
                expanded.push('\n');
            }
            continue;
        }
        expanded.push_str(line);
        expanded.push('\n');
    }
    Ok(expanded)
}

fn strip_quotes(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
        .unwrap_or(value)
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => Some(true),
        "false" | "no" | "off" | "0" => Some(false),
        _ => None,
    }
}

fn parse_rgb_color(value: &str) -> Option<RgbColor> {
    let hex = value.trim().strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    Some([
        u8::from_str_radix(&hex[0..2], 16).ok()?,
        u8::from_str_radix(&hex[2..4], 16).ok()?,
        u8::from_str_radix(&hex[4..6], 16).ok()?,
    ])
}

fn parse_palette_key(key: &str) -> Option<usize> {
    let suffix = key.strip_prefix("color")?;
    let index = suffix.parse::<usize>().ok()?;
    (index < 16).then_some(index)
}

fn parse_cursor_style(value: &str) -> Option<CursorStyle> {
    match value.trim().to_ascii_lowercase().as_str() {
        "block" => Some(CursorStyle::Block),
        "bar" | "beam" => Some(CursorStyle::Bar),
        "underline" => Some(CursorStyle::Underline),
        _ => None,
    }
}

fn parse_window_decoration(value: &str) -> Option<WindowDecoration> {
    match value.trim().to_ascii_lowercase().as_str() {
        "none" => Some(WindowDecoration::None),
        "titlebar" | "title-bar" | "native" | "system" => Some(WindowDecoration::TitleBar),
        _ => None,
    }
}

fn parse_notify_on_command_finish_action(value: &str) -> NotifyOnCommandFinishAction {
    let mut action = NotifyOnCommandFinishAction {
        bell: true,
        notify: false,
    };
    for entry in value
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
    {
        match entry {
            "bell" => action.bell = true,
            "no-bell" => action.bell = false,
            "notify" => action.notify = true,
            "no-notify" => action.notify = false,
            _ => {}
        }
    }
    action
}

fn parse_duration_ns(value: &str) -> Option<u64> {
    let bytes = value.as_bytes();
    let mut i = 0;
    let mut total = 0u64;
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if start == i {
            return None;
        }
        let number = value[start..i].parse::<u64>().ok()?;
        let unit_start = i;
        while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
            i += 1;
        }
        let unit = &value[unit_start..i];
        let multiplier = match unit {
            "h" => 3_600_000_000_000,
            "m" => 60_000_000_000,
            "s" => 1_000_000_000,
            "ms" => 1_000_000,
            "us" => 1_000,
            "ns" => 1,
            _ => return None,
        };
        total = total.checked_add(number.checked_mul(multiplier)?)?;
    }
    Some(total)
}

pub fn config_dir() -> PathBuf {
    let dir = std::env::var("XDG_CONFIG_HOME")
        .unwrap_or_else(|_| format!("{}/.config", std::env::var("HOME").unwrap_or_default()));
    PathBuf::from(dir).join("boo")
}

fn config_path() -> PathBuf {
    config_dir().join("config.boo")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_config() {
        let content = r#"
# visual settings
font-family = "Fira Code"
font-family = "Apple Color Emoji"
font-family = "Hiragino Sans GB"
font-size = 14
sync-to-monitor = false
background-opacity = 0.9
background-opacity-cells = true
cursor-style = underline
cursor-blink = true
cursor-blink-interval = 750ms

# boo settings
prefix-key = ctrl+s
control-socket = /tmp/boo.sock
remote-port = 7337
remote-auth-key = "secret"

keybind = " = new_split:right
keybind = % = new_split:down
keybind = super+1 = goto_tab:1
"#;
        let config = Config::parse(content);
        assert_eq!(config.prefix_key.as_deref(), Some("ctrl+s"));
        assert_eq!(config.control_socket.as_deref(), Some("/tmp/boo.sock"));
        assert_eq!(config.remote_port, Some(7337));
        assert_eq!(config.remote_auth_key.as_deref(), Some("secret"));
        assert_eq!(config.font_family.as_deref(), Some("Fira Code"));
        assert_eq!(
            config.font_fallbacks,
            vec!["Apple Color Emoji".to_string(), "Hiragino Sans GB".to_string()]
        );
        assert_eq!(config.font_size, Some(14.0));
        assert!(!config.sync_to_monitor);
        assert_eq!(config.background_opacity, Some(0.9));
        assert!(config.background_opacity_cells);
        assert_eq!(config.cursor_style, Some(CursorStyle::Underline));
        assert!(config.cursor_blink);
        assert_eq!(config.cursor_blink_interval_ns, 750_000_000);
        assert_eq!(config.keybinds.len(), 3);
        assert_eq!(
            config.keybinds.get("\"").map(|s| s.as_str()),
            Some("new_split:right")
        );
        assert_eq!(
            config.keybinds.get("%").map(|s| s.as_str()),
            Some("new_split:down")
        );
        assert_eq!(
            config.keybinds.get("super+1").map(|s| s.as_str()),
            Some("goto_tab:1")
        );
    }

    #[test]
    fn test_visual_keys_parse_without_boo_keys() {
        let content = "font-size = 14\nwindow-decoration = none\n";
        let config = Config::parse(content);
        assert!(config.prefix_key.is_none());
        assert!(config.keybinds.is_empty());
        assert_eq!(config.font_size, Some(14.0));
        assert_eq!(config.window_decoration, WindowDecoration::None);
    }

    #[test]
    fn test_empty_config() {
        let config = Config::parse("");
        assert!(config.prefix_key.is_none());
        assert!(config.keybinds.is_empty());
        assert!(config.font_family.is_none());
        assert!(config.font_fallbacks.is_empty());
        assert!(config.sync_to_monitor);
        assert!(config.background_opacity.is_none());
    }

    #[test]
    fn test_comments_and_blanks() {
        let content = "# comment\n\nprefix-key = ctrl+a\n# another\n";
        let config = Config::parse(content);
        assert_eq!(config.prefix_key.as_deref(), Some("ctrl+a"));
    }

    #[test]
    fn test_parse_bool_forms() {
        let config = Config::parse("background-opacity-cells = yes\n");
        assert!(config.background_opacity_cells);

        let config = Config::parse("background-opacity-cells = off\n");
        assert!(!config.background_opacity_cells);
    }

    #[test]
    fn test_parse_cursor_style() {
        assert_eq!(parse_cursor_style("block"), Some(CursorStyle::Block));
        assert_eq!(parse_cursor_style("beam"), Some(CursorStyle::Bar));
        assert_eq!(
            parse_cursor_style("underline"),
            Some(CursorStyle::Underline)
        );
        assert_eq!(parse_cursor_style("weird"), None);
    }

    #[test]
    fn test_cursor_style_maps_to_ghostty_visual_style() {
        assert_eq!(
            CursorStyle::Block.vt_visual_style(),
            crate::vt::GHOSTTY_RENDER_STATE_CURSOR_VISUAL_STYLE_BLOCK
        );
        assert_eq!(
            CursorStyle::Bar.vt_visual_style(),
            crate::vt::GHOSTTY_RENDER_STATE_CURSOR_VISUAL_STYLE_BAR
        );
        assert_eq!(
            CursorStyle::Underline.vt_visual_style(),
            crate::vt::GHOSTTY_RENDER_STATE_CURSOR_VISUAL_STYLE_UNDERLINE
        );
    }

    #[test]
    fn test_parse_window_decoration() {
        assert_eq!(
            parse_window_decoration("none"),
            Some(WindowDecoration::None)
        );
        assert_eq!(
            parse_window_decoration("titlebar"),
            Some(WindowDecoration::TitleBar)
        );
        assert_eq!(
            parse_window_decoration("native"),
            Some(WindowDecoration::TitleBar)
        );
        assert_eq!(parse_window_decoration("weird"), None);
    }

    #[test]
    fn test_parse_notify_on_command_finish_settings() {
        let config = Config::parse(
            "notify-on-command-finish = always\nnotify-on-command-finish-action = no-bell,notify\nnotify-on-command-finish-after = 1m30s\n",
        );
        assert_eq!(
            config.notify_on_command_finish,
            NotifyOnCommandFinish::Always
        );
        assert!(!config.notify_on_command_finish_action.bell);
        assert!(config.notify_on_command_finish_action.notify);
        assert_eq!(config.notify_on_command_finish_after_ns, 90_000_000_000);
    }
}
