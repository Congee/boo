use iced::Color;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum StatusBarZone {
    #[default]
    Left,
    Right,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct StatusStyle {
    #[serde(default)]
    pub fg: Option<String>,
    #[serde(default)]
    pub bg: Option<String>,
    #[serde(default)]
    pub bold: bool,
    #[serde(default)]
    pub italic: bool,
    #[serde(default)]
    pub underline: Option<String>,
    #[serde(default)]
    pub strikethrough: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StatusClick {
    pub id: String,
    #[serde(default)]
    pub action: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StatusComponent {
    pub text: String,
    #[serde(default)]
    pub style: StatusStyle,
    #[serde(default)]
    pub click: Option<StatusClick>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StatusComponentsUpdate {
    pub zone: StatusBarZone,
    pub source: String,
    pub components: Vec<StatusComponent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct UiStatusBarSnapshot {
    #[serde(default)]
    pub left: Vec<UiStatusComponent>,
    #[serde(default)]
    pub right: Vec<UiStatusComponent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UiStatusComponent {
    pub source: String,
    pub text: String,
    #[serde(default)]
    pub style: StatusStyle,
    #[serde(default)]
    pub click: Option<StatusClick>,
}

#[derive(Debug, Clone, Default)]
pub struct StatusComponentStore {
    entries: Vec<StatusComponentsUpdate>,
}

impl StatusComponentStore {
    pub fn set(&mut self, update: StatusComponentsUpdate) -> bool {
        if update.source.is_empty() {
            return false;
        }
        if update.components.is_empty() {
            return self.clear(&update.source, Some(update.zone));
        }
        if let Some(existing) = self
            .entries
            .iter_mut()
            .find(|entry| entry.source == update.source && entry.zone == update.zone)
        {
            if *existing == update {
                return false;
            }
            *existing = update;
            return true;
        }
        self.entries.push(update);
        true
    }

    pub fn clear(&mut self, source: &str, zone: Option<StatusBarZone>) -> bool {
        let old_len = self.entries.len();
        self.entries.retain(|entry| {
            !(entry.source == source && zone.map(|zone| zone == entry.zone).unwrap_or(true))
        });
        self.entries.len() != old_len
    }

    pub fn snapshot(&self) -> UiStatusBarSnapshot {
        let mut snapshot = UiStatusBarSnapshot::default();
        for entry in &self.entries {
            let target = match entry.zone {
                StatusBarZone::Left => &mut snapshot.left,
                StatusBarZone::Right => &mut snapshot.right,
            };
            target.extend(entry.components.iter().cloned().map(|component| UiStatusComponent {
                source: entry.source.clone(),
                text: component.text,
                style: component.style,
                click: component.click,
            }));
        }
        snapshot
    }

    pub fn click_action(&self, source: &str, click_id: &str) -> Option<String> {
        self.entries.iter().find_map(|entry| {
            if entry.source != source {
                return None;
            }
            entry.components.iter().find_map(|component| {
                component.click.as_ref().and_then(|click| {
                    if click.id == click_id {
                        click.action.clone()
                    } else {
                        None
                    }
                })
            })
        })
    }
}

pub fn osc_source_for_pane(pane_id: u64) -> String {
    format!("pane:{pane_id}:osc")
}

pub fn parse_status_color(value: &str) -> Option<Color> {
    let value = value.trim();
    if let Some(hex) = value.strip_prefix('#') {
        if hex.len() == 6 {
            let rgb = u32::from_str_radix(hex, 16).ok()?;
            return Some(Color::from_rgb8(
                ((rgb >> 16) & 0xff) as u8,
                ((rgb >> 8) & 0xff) as u8,
                (rgb & 0xff) as u8,
            ));
        }
        return None;
    }
    match value.to_ascii_lowercase().as_str() {
        "black" => Some(Color::from_rgb8(0x00, 0x00, 0x00)),
        "red" => Some(Color::from_rgb8(0xcc, 0x24, 0x1d)),
        "green" => Some(Color::from_rgb8(0x98, 0xc3, 0x79)),
        "yellow" => Some(Color::from_rgb8(0xd7, 0xba, 0x7d)),
        "blue" => Some(Color::from_rgb8(0x61, 0xaf, 0xef)),
        "magenta" => Some(Color::from_rgb8(0xc6, 0x78, 0xdd)),
        "cyan" => Some(Color::from_rgb8(0x56, 0xb6, 0xc2)),
        "white" => Some(Color::from_rgb8(0xe5, 0xe5, 0xe5)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_replaces_and_clears_by_source_and_zone() {
        let mut store = StatusComponentStore::default();
        assert!(store.set(StatusComponentsUpdate {
            zone: StatusBarZone::Left,
            source: "nvim-1".to_string(),
            components: vec![StatusComponent {
                text: "A".to_string(),
                style: StatusStyle::default(),
                click: None,
            }],
        }));
        assert!(store.set(StatusComponentsUpdate {
            zone: StatusBarZone::Right,
            source: "nvim-1".to_string(),
            components: vec![StatusComponent {
                text: "B".to_string(),
                style: StatusStyle::default(),
                click: None,
            }],
        }));
        assert!(store.set(StatusComponentsUpdate {
            zone: StatusBarZone::Left,
            source: "nvim-1".to_string(),
            components: vec![StatusComponent {
                text: "C".to_string(),
                style: StatusStyle::default(),
                click: None,
            }],
        }));

        let snapshot = store.snapshot();
        assert_eq!(snapshot.left[0].text, "C");
        assert_eq!(snapshot.right[0].text, "B");

        assert!(store.clear("nvim-1", Some(StatusBarZone::Left)));
        let snapshot = store.snapshot();
        assert!(snapshot.left.is_empty());
        assert_eq!(snapshot.right.len(), 1);

        assert!(store.clear("nvim-1", None));
        let snapshot = store.snapshot();
        assert!(snapshot.left.is_empty());
        assert!(snapshot.right.is_empty());
    }

    #[test]
    fn click_action_returns_matching_action() {
        let mut store = StatusComponentStore::default();
        store.set(StatusComponentsUpdate {
            zone: StatusBarZone::Left,
            source: "nvim-1".to_string(),
            components: vec![StatusComponent {
                text: "Run".to_string(),
                style: StatusStyle::default(),
                click: Some(StatusClick {
                    id: "run".to_string(),
                    action: Some("new-tab".to_string()),
                }),
            }],
        });

        assert_eq!(store.click_action("nvim-1", "run"), Some("new-tab".to_string()));
        assert_eq!(store.click_action("nvim-1", "missing"), None);
    }
}
