//! Boo configuration parsed from ~/.config/boo/boo.toml (or $XDG_CONFIG_HOME/boo/boo.toml).

use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Config {
    pub prefix_key: Option<String>,
    #[serde(default)]
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
        match toml::from_str(&content) {
            Ok(c) => {
                log::info!("loaded boo config: {}", path.display());
                c
            }
            Err(e) => {
                log::error!("failed to parse {}: {e}", path.display());
                Config::default()
            }
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            prefix_key: None,
            keybinds: HashMap::new(),
        }
    }
}

fn config_path() -> PathBuf {
    let config_dir = std::env::var("XDG_CONFIG_HOME")
        .unwrap_or_else(|_| format!("{}/.config", std::env::var("HOME").unwrap_or_default()));
    PathBuf::from(config_dir).join("boo/boo.toml")
}
