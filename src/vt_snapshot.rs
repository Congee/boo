#![cfg(target_os = "linux")]

use crate::control;
use crate::ffi;
use crate::vt_backend_core::{CellSnapshot, TerminalSnapshot};

pub fn selection_text(snapshot: &TerminalSnapshot, selection: ffi::ghostty_selection_s) -> String {
    let start_row = selection.top_left.y.min(selection.bottom_right.y) as usize;
    let end_row = selection.top_left.y.max(selection.bottom_right.y) as usize;
    let start_col = selection.top_left.x.min(selection.bottom_right.x) as usize;
    let end_col = selection.top_left.x.max(selection.bottom_right.x) as usize;
    let max_row = snapshot.rows_data.len().saturating_sub(1);

    let mut lines = Vec::new();
    for row_index in start_row.min(max_row)..=end_row.min(max_row) {
        let row = snapshot
            .rows_data
            .get(row_index)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let line_start = if selection.rectangle || row_index == start_row {
            start_col
        } else {
            0
        };
        let line_end = if selection.rectangle || row_index == end_row {
            end_col
        } else {
            snapshot.cols.saturating_sub(1) as usize
        };
        let text = row_text(row, line_start, line_end, selection.rectangle);
        lines.push(text);
    }

    lines.join("\n")
}

pub fn row_text(
    row: &[CellSnapshot],
    start_col: usize,
    end_col: usize,
    preserve_trailing_spaces: bool,
) -> String {
    if row.is_empty() || start_col > end_col {
        return String::new();
    }

    let mut out = String::new();
    for col in start_col..=end_col {
        let text = row
            .get(col)
            .map(|cell| cell.text.as_str())
            .filter(|text| !text.is_empty() && *text != "\0")
            .unwrap_or(" ");
        out.push_str(text);
    }

    if preserve_trailing_spaces {
        out
    } else {
        out.trim_end_matches(' ').to_string()
    }
}

pub fn ui_terminal_snapshot(snapshot: &TerminalSnapshot) -> control::UiTerminalSnapshot {
    control::UiTerminalSnapshot {
        cols: snapshot.cols,
        rows: snapshot.rows,
        title: snapshot.title.clone(),
        pwd: snapshot.pwd.clone(),
        cursor: control::UiCursorSnapshot {
            visible: snapshot.cursor.visible,
            blinking: snapshot.cursor.blinking,
            x: snapshot.cursor.x,
            y: snapshot.cursor.y,
            style: snapshot.cursor.style,
        },
        rows_data: snapshot
            .rows_data
            .iter()
            .map(|row| control::UiTerminalRowSnapshot {
                cells: row
                    .iter()
                    .map(|cell| control::UiTerminalCellSnapshot {
                        text: cell.text.clone(),
                        display_width: cell.display_width,
                        fg: [cell.fg.r, cell.fg.g, cell.fg.b],
                        bg: [cell.bg.r, cell.bg.g, cell.bg.b],
                        bold: cell.bold,
                        italic: cell.italic,
                        underline: cell.underline,
                    })
                    .collect(),
            })
            .collect(),
    }
}
