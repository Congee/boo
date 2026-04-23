//! Layout manager — declarative configs for predefined tab/pane layouts.
//!
//! Layout files live in `~/.config/boo/layouts/<name>.boo` using key=value format.
//!
//! Supports named layouts (even-horizontal, main-vertical, etc.) that auto-arrange
//! N panes, or explicit split directives with optional ratios.

use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Layout {
    pub name: String,
    pub tabs: Vec<LayoutTab>,
}

#[derive(Debug, Clone)]
pub struct LayoutTab {
    pub title: String,
    pub layout: TabLayout,
    pub panes: Vec<LayoutPane>,
}

/// How panes in a tab are arranged.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum TabLayout {
    /// Manual splits — each pane after the first has an explicit SplitDir.
    Manual,
    /// All panes side by side (equal width).
    EvenHorizontal,
    /// All panes stacked (equal height).
    EvenVertical,
    /// Big pane on top, rest in a row below.
    MainHorizontal,
    /// Big pane on left, rest stacked on right.
    MainVertical,
    /// Grid layout.
    Tiled,
}

#[derive(Debug, Clone)]
pub struct LayoutPane {
    pub command: Option<String>,
    pub working_directory: Option<String>,
    /// For Manual layout: how this pane splits from the previous.
    pub split: Option<SplitSpec>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SplitSpec {
    pub direction: SplitDir,
    pub ratio: f64, // 0.0..1.0, default 0.5
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SplitDir {
    Right,
    Down,
}

pub fn layouts_dir() -> PathBuf {
    crate::config::config_dir().join("layouts")
}

pub fn list_layouts() -> Vec<String> {
    let dir = layouts_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return vec![];
    };
    entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            name.strip_suffix(".boo").map(|s| s.to_string())
        })
        .collect()
}

pub fn load_layout(name: &str) -> Option<Layout> {
    let path = layouts_dir().join(format!("{name}.boo"));
    let content = std::fs::read_to_string(&path).ok()?;
    Some(parse_layout(name, &content))
}

pub fn parse_layout(name: &str, content: &str) -> Layout {
    let mut layout = Layout {
        name: name.to_string(),
        tabs: Vec::new(),
    };
    let mut current_working_dir: Option<String> = None;
    let mut pending_split: Option<SplitSpec> = None;
    let mut current_tab_layout = TabLayout::Manual;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // Bare directives (no = sign)
        if line == "split-right" || line == "split-down" {
            let dir = if line == "split-right" {
                SplitDir::Right
            } else {
                SplitDir::Down
            };
            pending_split = Some(SplitSpec {
                direction: dir,
                ratio: 0.5,
            });
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();

        match key {
            "layout-name" => layout.name = value.to_string(),
            "working-directory" => {
                current_working_dir = Some(shellexpand_home(value));
            }
            "layout" => {
                current_tab_layout = match value {
                    "even-horizontal" => TabLayout::EvenHorizontal,
                    "even-vertical" => TabLayout::EvenVertical,
                    "main-horizontal" => TabLayout::MainHorizontal,
                    "main-vertical" => TabLayout::MainVertical,
                    "tiled" => TabLayout::Tiled,
                    _ => TabLayout::Manual,
                };
                // Apply to current tab if one exists
                if let Some(tab) = layout.tabs.last_mut() {
                    tab.layout = current_tab_layout.clone();
                }
            }
            "tab" => {
                current_tab_layout = TabLayout::Manual;
                layout.tabs.push(LayoutTab {
                    title: value.to_string(),
                    layout: TabLayout::Manual,
                    panes: Vec::new(),
                });
            }
            "split-right" | "split-down" => {
                let dir = if key == "split-right" {
                    SplitDir::Right
                } else {
                    SplitDir::Down
                };
                let ratio = if value.is_empty() {
                    0.5
                } else {
                    parse_ratio(value)
                };
                pending_split = Some(SplitSpec {
                    direction: dir,
                    ratio,
                });
            }
            "pane" => {
                if layout.tabs.is_empty() {
                    layout.tabs.push(LayoutTab {
                        title: String::new(),
                        layout: current_tab_layout.clone(),
                        panes: Vec::new(),
                    });
                }
                let command = if value.is_empty() {
                    None
                } else {
                    Some(value.to_string())
                };
                let tab = layout.tabs.last_mut().unwrap();
                tab.panes.push(LayoutPane {
                    command,
                    working_directory: current_working_dir.clone(),
                    split: pending_split.take(),
                });
            }
            _ => {}
        }
    }

    // Ensure every tab has at least one pane
    for tab in &mut layout.tabs {
        if tab.panes.is_empty() {
            tab.panes.push(LayoutPane {
                command: None,
                working_directory: current_working_dir.clone(),
                split: None,
            });
        }
    }

    layout
}

/// Parse a ratio value: "0.6", "60%", or "60" (treated as percentage).
fn parse_ratio(s: &str) -> f64 {
    if let Some(pct) = s.strip_suffix('%') {
        pct.parse::<f64>().unwrap_or(50.0) / 100.0
    } else {
        let v: f64 = s.parse().unwrap_or(0.5);
        if v > 1.0 { v / 100.0 } else { v }
    }
}

/// Compute the split sequence for a named layout with N panes.
/// Returns a list of (SplitDir, ratio) for panes 1..N (pane 0 is the root).
pub fn layout_splits(layout: &TabLayout, n: usize) -> Vec<SplitSpec> {
    if n <= 1 {
        return vec![];
    }
    match layout {
        TabLayout::Manual => {
            // Default: all vertical splits, equal
            (1..n)
                .map(|_| SplitSpec {
                    direction: SplitDir::Down,
                    ratio: 0.5,
                })
                .collect()
        }
        TabLayout::EvenHorizontal => {
            // Cascading right-splits: each split gives 1/(remaining) to the first child
            (1..n)
                .map(|i| {
                    let remaining = n - i + 1;
                    SplitSpec {
                        direction: SplitDir::Right,
                        ratio: 1.0 / remaining as f64,
                    }
                })
                .collect()
        }
        TabLayout::EvenVertical => (1..n)
            .map(|i| {
                let remaining = n - i + 1;
                SplitSpec {
                    direction: SplitDir::Down,
                    ratio: 1.0 / remaining as f64,
                }
            })
            .collect(),
        TabLayout::MainVertical => {
            // First split: big pane left (60%), rest right
            let mut splits = vec![SplitSpec {
                direction: SplitDir::Right,
                ratio: 0.6,
            }];
            // Remaining panes split vertically on the right side
            for i in 2..n {
                let remaining = n - i + 1;
                splits.push(SplitSpec {
                    direction: SplitDir::Down,
                    ratio: 1.0 / remaining as f64,
                });
            }
            splits
        }
        TabLayout::MainHorizontal => {
            // First split: big pane top (60%), rest below
            let mut splits = vec![SplitSpec {
                direction: SplitDir::Down,
                ratio: 0.6,
            }];
            // Remaining panes split horizontally below
            for i in 2..n {
                let remaining = n - i + 1;
                splits.push(SplitSpec {
                    direction: SplitDir::Right,
                    ratio: 1.0 / remaining as f64,
                });
            }
            splits
        }
        TabLayout::Tiled => {
            // Alternate horizontal and vertical splits
            (1..n)
                .map(|i| {
                    let remaining = n - i + 1;
                    let dir = if i % 2 == 1 {
                        SplitDir::Right
                    } else {
                        SplitDir::Down
                    };
                    SplitSpec {
                        direction: dir,
                        ratio: 1.0 / remaining as f64,
                    }
                })
                .collect()
        }
    }
}

pub fn save_layout(layout: &Layout) -> std::io::Result<()> {
    let dir = layouts_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.boo", layout.name));
    let mut out = String::new();
    out.push_str(&format!("layout-name = {}\n\n", layout.name));
    for tab in &layout.tabs {
        out.push_str(&format!("tab = {}\n", tab.title));
        if tab.layout != TabLayout::Manual {
            let layout_str = match tab.layout {
                TabLayout::EvenHorizontal => "even-horizontal",
                TabLayout::EvenVertical => "even-vertical",
                TabLayout::MainHorizontal => "main-horizontal",
                TabLayout::MainVertical => "main-vertical",
                TabLayout::Tiled => "tiled",
                TabLayout::Manual => unreachable!(),
            };
            out.push_str(&format!("layout = {layout_str}\n"));
        }
        for (i, pane) in tab.panes.iter().enumerate() {
            if let Some(ref wd) = pane.working_directory {
                out.push_str(&format!("working-directory = {wd}\n"));
            }
            if i > 0 {
                if let Some(ref spec) = pane.split {
                    let dir_str = if spec.direction == SplitDir::Right {
                        "split-right"
                    } else {
                        "split-down"
                    };
                    if (spec.ratio - 0.5).abs() < 0.01 {
                        out.push_str(&format!("{dir_str}\n"));
                    } else {
                        out.push_str(&format!("{dir_str} = {:.0}%\n", spec.ratio * 100.0));
                    }
                } else {
                    out.push_str("split-down\n");
                }
            }
            let cmd = pane.command.as_deref().unwrap_or("");
            out.push_str(&format!("pane = {cmd}\n"));
        }
        out.push('\n');
    }
    std::fs::write(&path, out)?;
    log::info!("saved layout: {}", path.display());
    Ok(())
}

fn shellexpand_home(path: &str) -> String {
    if path.starts_with("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{home}{}", &path[1..]);
        }
    }
    path.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_basic_layout() {
        let content = r#"
layout-name = dev

tab = editor
pane = nvim .

tab = servers
pane = cargo watch
split-right
pane = tail -f log
"#;
        let layout = parse_layout("dev", content);
        assert_eq!(layout.name, "dev");
        assert_eq!(layout.tabs.len(), 2);
        assert_eq!(layout.tabs[0].panes.len(), 1);
        assert!(layout.tabs[0].panes[0].split.is_none());
        assert_eq!(layout.tabs[1].panes.len(), 2);
        assert_eq!(
            layout.tabs[1].panes[1].split.as_ref().unwrap().direction,
            SplitDir::Right
        );
    }

    #[test]
    fn test_parse_named_layout() {
        let content = r#"
tab = dev
layout = main-vertical
pane = nvim
pane = cargo watch
pane = htop
"#;
        let layout = parse_layout("test", content);
        assert_eq!(layout.tabs[0].layout, TabLayout::MainVertical);
        assert_eq!(layout.tabs[0].panes.len(), 3);
    }

    #[test]
    fn test_parse_ratio() {
        assert!((parse_ratio("0.6") - 0.6).abs() < 0.001);
        assert!((parse_ratio("60%") - 0.6).abs() < 0.001);
        assert!((parse_ratio("60") - 0.6).abs() < 0.001);
        assert!((parse_ratio("0.5") - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_parse_split_with_ratio() {
        let content = "tab = dev\npane = vim\nsplit-right = 70%\npane = term\n";
        let layout = parse_layout("r", content);
        let spec = layout.tabs[0].panes[1].split.as_ref().unwrap();
        assert_eq!(spec.direction, SplitDir::Right);
        assert!((spec.ratio - 0.7).abs() < 0.001);
    }

    #[test]
    fn test_layout_splits_even_horizontal() {
        let splits = layout_splits(&TabLayout::EvenHorizontal, 3);
        assert_eq!(splits.len(), 2);
        assert_eq!(splits[0].direction, SplitDir::Right);
        assert_eq!(splits[1].direction, SplitDir::Right);
        // First split: 1/3 for pane 0, 2/3 remaining
        assert!((splits[0].ratio - 1.0 / 3.0).abs() < 0.01);
        // Second split: 1/2 of remaining
        assert!((splits[1].ratio - 0.5).abs() < 0.01);
    }

    #[test]
    fn test_layout_splits_even_vertical() {
        let splits = layout_splits(&TabLayout::EvenVertical, 4);
        assert_eq!(splits.len(), 3);
        for s in &splits {
            assert_eq!(s.direction, SplitDir::Down);
        }
    }

    #[test]
    fn test_layout_splits_main_vertical() {
        let splits = layout_splits(&TabLayout::MainVertical, 3);
        assert_eq!(splits.len(), 2);
        assert_eq!(splits[0].direction, SplitDir::Right);
        assert!((splits[0].ratio - 0.6).abs() < 0.01); // main pane 60%
        assert_eq!(splits[1].direction, SplitDir::Down);
    }

    #[test]
    fn test_layout_splits_main_horizontal() {
        let splits = layout_splits(&TabLayout::MainHorizontal, 3);
        assert_eq!(splits[0].direction, SplitDir::Down);
        assert!((splits[0].ratio - 0.6).abs() < 0.01);
        assert_eq!(splits[1].direction, SplitDir::Right);
    }

    #[test]
    fn test_layout_splits_single_pane() {
        let splits = layout_splits(&TabLayout::EvenHorizontal, 1);
        assert!(splits.is_empty());
    }

    #[test]
    fn test_layout_splits_tiled() {
        let splits = layout_splits(&TabLayout::Tiled, 4);
        assert_eq!(splits.len(), 3);
        assert_eq!(splits[0].direction, SplitDir::Right);
        assert_eq!(splits[1].direction, SplitDir::Down);
        assert_eq!(splits[2].direction, SplitDir::Right);
    }

    #[test]
    fn test_parse_working_directory() {
        let content = "working-directory = ~/dev\ntab = x\npane = nvim\n";
        let layout = parse_layout("wd", content);
        assert!(
            layout.tabs[0].panes[0]
                .working_directory
                .as_ref()
                .unwrap()
                .contains("/dev")
        );
    }

    #[test]
    fn test_parse_empty_pane() {
        let content = "tab = shell\npane =\n";
        let layout = parse_layout("e", content);
        assert!(layout.tabs[0].panes[0].command.is_none());
    }

    #[test]
    fn test_parse_implicit_tab() {
        let content = "pane = htop\n";
        let layout = parse_layout("bare", content);
        assert_eq!(layout.tabs.len(), 1);
    }

    #[test]
    fn test_save_roundtrip() {
        let layout = Layout {
            name: "rt".to_string(),
            tabs: vec![LayoutTab {
                title: "main".to_string(),
                layout: TabLayout::MainVertical,
                panes: vec![
                    LayoutPane {
                        command: Some("nvim".to_string()),
                        working_directory: None,
                        split: None,
                    },
                    LayoutPane {
                        command: Some("cargo run".to_string()),
                        working_directory: None,
                        split: Some(SplitSpec {
                            direction: SplitDir::Right,
                            ratio: 0.6,
                        }),
                    },
                ],
            }],
        };
        let dir = std::env::temp_dir().join("boo-test-sess");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rt.boo");

        // Serialize
        let mut out = String::new();
        out.push_str(&format!("layout-name = {}\n\n", layout.name));
        for tab in &layout.tabs {
            out.push_str(&format!("tab = {}\n", tab.title));
            if tab.layout != TabLayout::Manual {
                out.push_str("layout = main-vertical\n");
            }
            for (i, pane) in tab.panes.iter().enumerate() {
                if i > 0 {
                    if let Some(ref spec) = pane.split {
                        let d = if spec.direction == SplitDir::Right {
                            "split-right"
                        } else {
                            "split-down"
                        };
                        out.push_str(&format!("{d} = {:.0}%\n", spec.ratio * 100.0));
                    }
                }
                out.push_str(&format!(
                    "pane = {}\n",
                    pane.command.as_deref().unwrap_or("")
                ));
            }
            out.push('\n');
        }
        std::fs::write(&path, &out).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let parsed = parse_layout("rt", &content);
        assert_eq!(parsed.tabs[0].layout, TabLayout::MainVertical);
        assert_eq!(parsed.tabs[0].panes.len(), 2);
        let spec = parsed.tabs[0].panes[1].split.as_ref().unwrap();
        assert!((spec.ratio - 0.6).abs() < 0.01);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
