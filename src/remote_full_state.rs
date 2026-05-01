//! Adapters that turn UI / VT snapshots into the wire-level `RemoteFullState`.
//!
//! Used by `runtime_server::remote_full_state_for_pane` and the remote
//! broadcast path to convert internal presentation structs into the
//! fixed-layout cell buffer that the delta encoder and clients understand.
//!
//! Pure mapping code — no server state, no I/O — split out of `remote.rs` so
//! the mapping can be unit-tested alongside the wire encoders.

use crate::remote_wire::{
    RemoteCell, RemoteFullState, STYLE_FLAG_BOLD, STYLE_FLAG_EXPLICIT_BG, STYLE_FLAG_EXPLICIT_FG,
    STYLE_FLAG_HYPERLINK, STYLE_FLAG_ITALIC,
};

pub fn full_state_from_ui(snapshot: &crate::control::UiTerminalSnapshot) -> RemoteFullState {
    let cells = snapshot
        .rows_data
        .iter()
        .flat_map(|row| row.cells.iter())
        .map(|cell| {
            let mut style_flags = 0u8;
            if cell.bold {
                style_flags |= STYLE_FLAG_BOLD;
            }
            if cell.italic {
                style_flags |= STYLE_FLAG_ITALIC;
            }
            if cell.hyperlink {
                style_flags |= STYLE_FLAG_HYPERLINK;
            }
            if cell.fg != [0, 0, 0] {
                style_flags |= STYLE_FLAG_EXPLICIT_FG;
            }
            if !cell.bg_is_default {
                style_flags |= STYLE_FLAG_EXPLICIT_BG;
            }
            RemoteCell {
                codepoint: cell.text.chars().next().map(u32::from).unwrap_or(0),
                fg: cell.fg,
                bg: cell.bg,
                style_flags,
                wide: cell.display_width > 1,
            }
        })
        .collect();
    RemoteFullState {
        epoch: 0,
        viewport_top: 0,
        scrollback_total: snapshot.rows as u64,
        rows: snapshot.rows,
        cols: snapshot.cols,
        cursor_x: snapshot.cursor.x,
        cursor_y: snapshot.cursor.y,
        cursor_visible: snapshot.cursor.visible,
        cursor_blinking: snapshot.cursor.blinking,
        cursor_style: snapshot.cursor.style,
        cells,
    }
}

pub fn full_state_from_terminal(
    snapshot: &crate::vt_backend_core::TerminalSnapshot,
) -> RemoteFullState {
    let cells = snapshot
        .rows_data
        .iter()
        .flat_map(|row| row.iter())
        .map(|cell| {
            let mut style_flags = 0u8;
            if cell.bold {
                style_flags |= STYLE_FLAG_BOLD;
            }
            if cell.italic {
                style_flags |= STYLE_FLAG_ITALIC;
            }
            if cell.hyperlink {
                style_flags |= STYLE_FLAG_HYPERLINK;
            }
            let has_explicit_fg = cell.fg != snapshot.colors.foreground;
            let has_explicit_bg = !cell.bg_is_default;
            if has_explicit_fg {
                style_flags |= STYLE_FLAG_EXPLICIT_FG;
            }
            if has_explicit_bg {
                style_flags |= STYLE_FLAG_EXPLICIT_BG;
            }
            RemoteCell {
                codepoint: cell.text.chars().next().map(u32::from).unwrap_or(0),
                fg: cell.fg.to_array(),
                bg: cell.bg.to_array(),
                style_flags,
                wide: cell.display_width > 1,
            }
        })
        .collect();
    RemoteFullState {
        epoch: 0,
        viewport_top: snapshot.scrollbar.offset,
        scrollback_total: snapshot.scrollbar.total,
        rows: snapshot.rows,
        cols: snapshot.cols,
        cursor_x: snapshot.cursor.x,
        cursor_y: snapshot.cursor.y,
        cursor_visible: snapshot.cursor.visible,
        cursor_blinking: snapshot.cursor.blinking,
        cursor_style: snapshot.cursor.style.raw(),
        cells,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control;

    #[test]
    fn full_state_from_ui_snapshot_flattens_rows() {
        let snapshot = control::UiTerminalSnapshot {
            cols: 2,
            rows: 1,
            title: String::new(),
            pwd: String::new(),
            cursor: control::UiCursorSnapshot {
                visible: true,
                blinking: false,
                x: 1,
                y: 0,
                style: 0,
            },
            rows_data: vec![control::UiTerminalRowSnapshot {
                cells: vec![
                    control::UiTerminalCellSnapshot {
                        text: "a".to_string(),
                        display_width: 1,
                        fg: [1, 1, 1],
                        bg: [0, 0, 0],
                        bg_is_default: true,
                        bold: false,
                        italic: false,
                        underline: 0,
                        hyperlink: false,
                    },
                    control::UiTerminalCellSnapshot {
                        text: "界".to_string(),
                        display_width: 2,
                        fg: [2, 2, 2],
                        bg: [3, 3, 3],
                        bg_is_default: false,
                        bold: true,
                        italic: true,
                        underline: 0,
                        hyperlink: false,
                    },
                ],
            }],
        };
        let state = full_state_from_ui(&snapshot);
        assert_eq!(state.cells.len(), 2);
        assert_eq!(state.cells[0].codepoint, u32::from('a'));
        assert!(!state.cells[0].wide);
        assert_eq!(state.cells[1].codepoint, u32::from('界'));
        assert!(state.cells[1].wide);
        assert_eq!(state.cells[1].style_flags & 0x03, 0x03);
    }

    #[test]
    fn full_state_from_terminal_snapshot_flattens_rows() {
        let snapshot = crate::vt_backend_core::TerminalSnapshot {
            cols: 2,
            rows: 1,
            cursor: crate::vt_backend_core::CursorSnapshot {
                visible: true,
                blinking: true,
                x: 1,
                y: 0,
                style: crate::vt::CursorStyle::Bar,
            },
            rows_data: vec![vec![
                crate::vt_backend_core::CellSnapshot {
                    text: "a".to_string(),
                    display_width: 1,
                    fg: crate::vt::RgbColor { r: 1, g: 1, b: 1 },
                    bg: crate::vt::RgbColor { r: 0, g: 0, b: 0 },
                    bg_is_default: true,
                    bold: false,
                    italic: false,
                    underline: 0,
                    hyperlink: false,
                },
                crate::vt_backend_core::CellSnapshot {
                    text: "界".to_string(),
                    display_width: 2,
                    fg: crate::vt::RgbColor { r: 2, g: 2, b: 2 },
                    bg: crate::vt::RgbColor { r: 3, g: 3, b: 3 },
                    bg_is_default: false,
                    bold: true,
                    italic: true,
                    underline: 0,
                    hyperlink: false,
                },
            ]],
            colors: crate::vt::RenderColors {
                foreground: crate::vt::RgbColor { r: 1, g: 1, b: 1 },
                background: crate::vt::RgbColor { r: 0, g: 0, b: 0 },
                ..Default::default()
            },
            ..Default::default()
        };
        let state = full_state_from_terminal(&snapshot);
        assert_eq!(state.cells.len(), 2);
        assert_eq!(state.cells[0].codepoint, u32::from('a'));
        assert!(!state.cells[0].wide);
        assert_eq!(state.cells[1].codepoint, u32::from('界'));
        assert!(state.cells[1].wide);
        assert_eq!(state.cells[1].style_flags & 0x03, 0x03);
        assert_eq!(state.cells[1].style_flags & 0x60, 0x60);
    }
}
