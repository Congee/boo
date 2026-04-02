//! Boo configuration parsed from ~/.config/boo/config.boo.
//!
//! Single config file using ghostty's key=value format.
//! Ghostty-native keys (font-family, background-opacity, etc.) are handled by libghostty.
//! Boo-specific keys (prefix-key, control-socket, keybind) are parsed here.

use std::collections::HashMap;
use std::path::PathBuf;

pub struct Config {
    pub prefix_key: Option<String>,
    pub control_socket: Option<String>,
    pub keybinds: HashMap<String, String>,
}

impl Config {
    pub fn load() -> Config {
        let path = config_path();
        if !path.exists() {
            log::warn!("boo config not found at {}", path.display());
            return Config::default();
        }
        let content = match std::fs::read_to_string(&path) {
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
            let Some((key, value)) = line.split_once('=') else { continue };
            let key = key.trim();
            let value = value.trim();
            match key {
                "prefix-key" => config.prefix_key = Some(value.to_string()),
                "control-socket" => config.control_socket = Some(value.to_string()),
                "keybind" => {
                    // Format: keybind = <key>=<action>
                    if let Some((bind_key, action)) = value.split_once('=') {
                        config.keybinds.insert(
                            bind_key.trim().to_string(),
                            action.trim().to_string(),
                        );
                    }
                }
                _ => {} // ghostty-native keys — ignored here, handled by libghostty
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
            keybinds: HashMap::new(),
        }
    }
}

pub fn config_dir() -> PathBuf {
    let dir = std::env::var("XDG_CONFIG_HOME")
        .unwrap_or_else(|_| format!("{}/.config", std::env::var("HOME").unwrap_or_default()));
    PathBuf::from(dir).join("boo")
}

fn config_path() -> PathBuf {
    config_dir().join("config.boo")
}

/// Path to the config file for loading into ghostty.
/// Same file as boo's config — ghostty ignores boo- prefixed keys.
pub fn ghostty_config_path() -> PathBuf {
    config_path()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_config() {
        let content = r#"
# ghostty settings (ignored by boo parser)
font-family = Fira Code
font-size = 14
background-opacity = 0.9

# boo settings
prefix-key = ctrl+s
control-socket = /tmp/boo.sock

keybind = " = new_split:right
keybind = % = new_split:down
keybind = super+1 = goto_tab:1
"#;
        let config = Config::parse(content);
        assert_eq!(config.prefix_key.as_deref(), Some("ctrl+s"));
        assert_eq!(config.control_socket.as_deref(), Some("/tmp/boo.sock"));
        assert_eq!(config.keybinds.len(), 3);
        assert_eq!(config.keybinds.get("\"").map(|s| s.as_str()), Some("new_split:right"));
        assert_eq!(config.keybinds.get("%").map(|s| s.as_str()), Some("new_split:down"));
        assert_eq!(config.keybinds.get("super+1").map(|s| s.as_str()), Some("goto_tab:1"));
    }

    #[test]
    fn test_ghostty_keys_ignored() {
        let content = "font-size = 14\nwindow-decoration = none\n";
        let config = Config::parse(content);
        assert!(config.prefix_key.is_none());
        assert!(config.keybinds.is_empty());
    }

    #[test]
    fn test_empty_config() {
        let config = Config::parse("");
        assert!(config.prefix_key.is_none());
        assert!(config.keybinds.is_empty());
    }

    #[test]
    fn test_comments_and_blanks() {
        let content = "# comment\n\nprefix-key = ctrl+a\n# another\n";
        let config = Config::parse(content);
        assert_eq!(config.prefix_key.as_deref(), Some("ctrl+a"));
    }
}
