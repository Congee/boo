use std::ffi::c_void;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SelectionMode {
    None,
    Char,
    Line,
    Rectangle,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum JumpKind {
    Forward,
    Backward,
    ToForward,
    ToBackward,
}

#[derive(Debug, Clone, Copy)]
#[allow(clippy::enum_variant_names)]
pub(crate) enum WordMoveKind {
    NextWord,
    PrevWord,
    EndWord,
    NextBigWord,
    PrevBigWord,
    EndBigWord,
}

pub(crate) struct CopyModeState {
    pub(crate) cursor_row: i64,
    pub(crate) cursor_col: u32,
    pub(crate) selection: SelectionMode,
    pub(crate) sel_anchor: Option<(i64, u32)>,
    pub(crate) highlight_layers: Vec<*mut c_void>,
    pub(crate) cursor_layer: *mut c_void,
    pub(crate) cell_width: f64,
    pub(crate) cell_height: f64,
    pub(crate) viewport_rows: u32,
    pub(crate) viewport_cols: u32,
    pub(crate) mark: Option<(i64, u32)>,
    pub(crate) last_jump: Option<(char, JumpKind)>,
    pub(crate) last_search_forward: bool,
    pub(crate) pending_jump: Option<JumpKind>,
    pub(crate) show_position: bool,
}

pub(crate) fn selection_mode_name(selection: SelectionMode) -> &'static str {
    match selection {
        SelectionMode::None => "none",
        SelectionMode::Char => "character",
        SelectionMode::Line => "line",
        SelectionMode::Rectangle => "rectangle",
    }
}
