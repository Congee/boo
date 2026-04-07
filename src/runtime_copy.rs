use super::*;

impl BooApp {
    pub(crate) fn dispatch_copy_mode_action(&mut self, action: bindings::CopyModeAction) {
        use bindings::CopyModeAction::*;

        if let Some(_kind) = self.copy_mode.as_ref().and_then(|cm| cm.pending_jump) {
            if let JumpForward | JumpBackward | JumpToForward | JumpToBackward = action {
            } else if let Some(ref mut cm) = self.copy_mode {
                cm.pending_jump = None;
            }
        }

        match action {
            Move(dir) => self.copy_mode_move(dir),
            WordNext => self.copy_mode_word_move(WordMoveKind::NextWord),
            WordBack => self.copy_mode_word_move(WordMoveKind::PrevWord),
            WordEnd => self.copy_mode_word_move(WordMoveKind::EndWord),
            BigWordNext => self.copy_mode_word_move(WordMoveKind::NextBigWord),
            BigWordBack => self.copy_mode_word_move(WordMoveKind::PrevBigWord),
            BigWordEnd => self.copy_mode_word_move(WordMoveKind::EndBigWord),
            LineStart => {
                if let Some(ref mut cm) = self.copy_mode {
                    cm.cursor_col = 0;
                }
                self.update_copy_mode_highlight();
            }
            LineEnd => {
                if let Some(ref mut cm) = self.copy_mode {
                    cm.cursor_col = cm.viewport_cols.saturating_sub(1);
                }
                self.update_copy_mode_highlight();
            }
            FirstNonBlank => self.copy_mode_first_non_blank(),
            ScreenTop => {
                if let Some(ref mut cm) = self.copy_mode {
                    cm.cursor_row = self.scrollbar.offset as i64;
                    cm.cursor_col = 0;
                }
                self.update_copy_mode_highlight();
            }
            ScreenMiddle => {
                if let Some(ref mut cm) = self.copy_mode {
                    cm.cursor_row = self.scrollbar.offset as i64 + cm.viewport_rows as i64 / 2;
                    cm.cursor_col = 0;
                }
                self.update_copy_mode_highlight();
            }
            ScreenBottom => {
                if let Some(ref mut cm) = self.copy_mode {
                    cm.cursor_row = self.scrollbar.offset as i64 + cm.viewport_rows as i64 - 1;
                    cm.cursor_col = 0;
                }
                self.update_copy_mode_highlight();
            }
            HistoryTop => {
                if let Some(ref mut cm) = self.copy_mode {
                    cm.cursor_row = 0;
                    cm.cursor_col = 0;
                }
                self.ghostty_binding_action("scroll_to_top");
                self.update_copy_mode_highlight();
            }
            HistoryBottom => {
                if let Some(ref mut cm) = self.copy_mode {
                    cm.cursor_row = self.scrollbar.total as i64;
                    cm.cursor_col = 0;
                }
                self.ghostty_binding_action("scroll_to_bottom");
                self.update_copy_mode_highlight();
            }
            PageUp => {
                if let Some(ref mut cm) = self.copy_mode {
                    cm.cursor_row = cm.cursor_row.saturating_sub(cm.viewport_rows as i64);
                }
                self.ghostty_binding_action("scroll_page_up");
                self.update_copy_mode_highlight();
            }
            PageDown => {
                if let Some(ref mut cm) = self.copy_mode {
                    cm.cursor_row += cm.viewport_rows as i64;
                }
                self.ghostty_binding_action("scroll_page_down");
                self.update_copy_mode_highlight();
            }
            HalfPageUp => {
                if let Some(ref mut cm) = self.copy_mode {
                    let half = (cm.viewport_rows / 2) as i64;
                    cm.cursor_row = cm.cursor_row.saturating_sub(half);
                }
                self.copy_mode_ensure_visible();
                self.update_copy_mode_highlight();
            }
            HalfPageDown => {
                if let Some(ref mut cm) = self.copy_mode {
                    let half = (cm.viewport_rows / 2) as i64;
                    cm.cursor_row += half;
                }
                self.copy_mode_ensure_visible();
                self.update_copy_mode_highlight();
            }
            ScrollUp => {
                self.ghostty_binding_action("scroll_page_lines:-1");
                self.update_copy_mode_highlight();
            }
            ScrollDown => {
                self.ghostty_binding_action("scroll_page_lines:1");
                self.update_copy_mode_highlight();
            }
            ScrollMiddle => {
                if let Some(ref cm) = self.copy_mode {
                    let target_offset = cm.cursor_row - cm.viewport_rows as i64 / 2;
                    let target_offset = target_offset.max(0) as usize;
                    let current = self.scrollbar.offset;
                    let diff = target_offset as i64 - current as i64;
                    if diff != 0 {
                        let cmd = format!("scroll_page_lines:{diff}");
                        self.ghostty_binding_action(&cmd);
                    }
                }
                self.update_copy_mode_highlight();
            }
            StartCharSelect => {
                if let Some(ref mut cm) = self.copy_mode {
                    if cm.selection == SelectionMode::Char {
                        cm.selection = SelectionMode::None;
                        cm.sel_anchor = None;
                    } else {
                        cm.selection = SelectionMode::Char;
                        cm.sel_anchor = Some((cm.cursor_row, cm.cursor_col));
                    }
                }
                self.update_copy_mode_highlight();
            }
            StartLineSelect => {
                if let Some(ref mut cm) = self.copy_mode {
                    if cm.selection == SelectionMode::Line {
                        cm.selection = SelectionMode::None;
                        cm.sel_anchor = None;
                    } else {
                        cm.selection = SelectionMode::Line;
                        cm.sel_anchor = Some((cm.cursor_row, cm.cursor_col));
                    }
                }
                self.update_copy_mode_highlight();
            }
            StartRectSelect => {
                if let Some(ref mut cm) = self.copy_mode {
                    if cm.selection == SelectionMode::Rectangle {
                        cm.selection = SelectionMode::None;
                        cm.sel_anchor = None;
                    } else {
                        cm.selection = SelectionMode::Rectangle;
                        if cm.sel_anchor.is_none() {
                            cm.sel_anchor = Some((cm.cursor_row, cm.cursor_col));
                        }
                    }
                }
                self.update_copy_mode_highlight();
            }
            ClearSelection => {
                if let Some(ref mut cm) = self.copy_mode {
                    if cm.selection != SelectionMode::None {
                        cm.selection = SelectionMode::None;
                        cm.sel_anchor = None;
                        self.update_copy_mode_highlight();
                    } else {
                        self.exit_copy_mode();
                    }
                }
            }
            SwapAnchor => {
                if let Some(ref mut cm) = self.copy_mode {
                    if let Some((ar, ac)) = cm.sel_anchor {
                        let (cr, cc) = (cm.cursor_row, cm.cursor_col);
                        cm.cursor_row = ar;
                        cm.cursor_col = ac;
                        cm.sel_anchor = Some((cr, cc));
                    }
                }
                self.copy_mode_ensure_visible();
                self.update_copy_mode_highlight();
            }
            JumpForward => {
                if let Some(ref mut cm) = self.copy_mode {
                    cm.pending_jump = Some(JumpKind::Forward);
                }
            }
            JumpBackward => {
                if let Some(ref mut cm) = self.copy_mode {
                    cm.pending_jump = Some(JumpKind::Backward);
                }
            }
            JumpToForward => {
                if let Some(ref mut cm) = self.copy_mode {
                    cm.pending_jump = Some(JumpKind::ToForward);
                }
            }
            JumpToBackward => {
                if let Some(ref mut cm) = self.copy_mode {
                    cm.pending_jump = Some(JumpKind::ToBackward);
                }
            }
            JumpAgain => self.copy_mode_jump_repeat(false),
            JumpReverse => self.copy_mode_jump_repeat(true),
            NextParagraph => self.copy_mode_paragraph(true),
            PreviousParagraph => self.copy_mode_paragraph(false),
            MatchingBracket => self.copy_mode_matching_bracket(),
            SetMark => {
                if let Some(ref mut cm) = self.copy_mode {
                    cm.mark = Some((cm.cursor_row, cm.cursor_col));
                }
            }
            JumpToMark => {
                if let Some(ref mut cm) = self.copy_mode {
                    if let Some((r, c)) = cm.mark {
                        cm.cursor_row = r;
                        cm.cursor_col = c;
                    }
                }
                self.copy_mode_ensure_visible();
                self.update_copy_mode_highlight();
            }
            SearchForward => {
                if let Some(ref mut cm) = self.copy_mode {
                    cm.last_search_forward = true;
                }
                self.search_active = true;
                self.search_query.clear();
                self.search_total = 0;
                self.search_selected = 0;
                self.relayout();
            }
            SearchBackward => {
                if let Some(ref mut cm) = self.copy_mode {
                    cm.last_search_forward = false;
                }
                self.search_active = true;
                self.search_query.clear();
                self.search_total = 0;
                self.search_selected = 0;
                self.relayout();
            }
            SearchAgain => {
                let forward = self
                    .copy_mode
                    .as_ref()
                    .map_or(true, |cm| cm.last_search_forward);
                if forward {
                    self.ghostty_binding_action("navigate_search:next");
                } else {
                    self.ghostty_binding_action("navigate_search:previous");
                }
            }
            SearchReverse => {
                let forward = self
                    .copy_mode
                    .as_ref()
                    .map_or(true, |cm| cm.last_search_forward);
                if forward {
                    self.ghostty_binding_action("navigate_search:previous");
                } else {
                    self.ghostty_binding_action("navigate_search:next");
                }
            }
            SearchWordForward => {
                if let Some(word) = self.copy_mode_word_under_cursor() {
                    if let Some(ref mut cm) = self.copy_mode {
                        cm.last_search_forward = true;
                    }
                    self.search_active = true;
                    self.search_query = word;
                    self.relayout();
                    self.ghostty_binding_action("navigate_search:next");
                }
            }
            SearchWordBackward => {
                if let Some(word) = self.copy_mode_word_under_cursor() {
                    if let Some(ref mut cm) = self.copy_mode {
                        cm.last_search_forward = false;
                    }
                    self.search_active = true;
                    self.search_query = word;
                    self.relayout();
                    self.ghostty_binding_action("navigate_search:previous");
                }
            }
            CopyAndExit => {
                self.copy_mode_copy();
                self.exit_copy_mode();
            }
            CopyToEndOfLine => {
                if let Some(ref mut cm) = self.copy_mode {
                    cm.selection = SelectionMode::Char;
                    cm.sel_anchor = Some((cm.cursor_row, cm.cursor_col));
                    cm.cursor_col = cm.viewport_cols.saturating_sub(1);
                }
                self.copy_mode_copy();
                self.exit_copy_mode();
            }
            AppendAndCancel => {
                self.copy_mode_append_copy();
                self.exit_copy_mode();
            }
            OpenPrompt => {
                self.command_prompt.active = true;
                self.command_prompt.input.clear();
                self.command_prompt.selected_suggestion = 0;
                self.command_prompt.history_idx = None;
                self.command_prompt.update_suggestions();
            }
            RefreshFromPane => {
                self.update_copy_mode_highlight();
            }
            TogglePosition => {
                if let Some(ref mut cm) = self.copy_mode {
                    cm.show_position = !cm.show_position;
                }
            }
            Exit => self.exit_copy_mode(),
        }
    }

    pub(crate) fn copy_mode_move(&mut self, dir: bindings::Direction) {
        let Some(ref mut cm) = self.copy_mode else {
            return;
        };
        match dir {
            bindings::Direction::Up => cm.cursor_row -= 1,
            bindings::Direction::Down => cm.cursor_row += 1,
            bindings::Direction::Left => {
                if cm.cursor_col > 0 {
                    cm.cursor_col -= 1;
                }
            }
            bindings::Direction::Right => {
                cm.cursor_col = (cm.cursor_col + 1).min(cm.viewport_cols.saturating_sub(1));
            }
        }
        self.copy_mode_ensure_visible();
        self.update_copy_mode_highlight();
    }

    pub(crate) fn copy_mode_ensure_visible(&mut self) {
        let Some(ref cm) = self.copy_mode else { return };
        let viewport_row = cm.cursor_row - self.scrollbar.offset as i64;
        if viewport_row < 0 {
            let lines = -viewport_row;
            let cmd = format!("scroll_page_lines:-{lines}");
            self.ghostty_binding_action(&cmd);
        } else if viewport_row >= cm.viewport_rows as i64 {
            let lines = viewport_row - cm.viewport_rows as i64 + 1;
            let cmd = format!("scroll_page_lines:{lines}");
            self.ghostty_binding_action(&cmd);
        }
    }

    pub(crate) fn copy_mode_first_non_blank(&mut self) {
        if let Some(line) = self.read_viewport_line_for_cursor() {
            let col = line.chars().position(|c| !c.is_whitespace()).unwrap_or(0);
            if let Some(ref mut cm) = self.copy_mode {
                cm.cursor_col = col as u32;
            }
        }
        self.update_copy_mode_highlight();
    }

    pub(crate) fn read_viewport_line_for_cursor(&self) -> Option<String> {
        let cm = self.copy_mode.as_ref()?;
        let viewport_row = (cm.cursor_row - self.scrollbar.offset as i64).max(0) as u32;
        self.read_viewport_line(viewport_row)
    }

    pub(crate) fn read_viewport_line(&self, viewport_row: u32) -> Option<String> {
        let cm = self.copy_mode.as_ref()?;
        let sel = ffi::ghostty_selection_s {
            top_left: ffi::ghostty_point_s {
                tag: ffi::GHOSTTY_POINT_VIEWPORT,
                coord: ffi::GHOSTTY_POINT_COORD_EXACT,
                x: 0,
                y: viewport_row,
            },
            bottom_right: ffi::ghostty_point_s {
                tag: ffi::GHOSTTY_POINT_VIEWPORT,
                coord: ffi::GHOSTTY_POINT_COORD_EXACT,
                x: cm.viewport_cols.saturating_sub(1),
                y: viewport_row,
            },
            rectangle: false,
        };
        self.read_surface_selection_text(sel)
    }

    pub(crate) fn copy_mode_word_move(&mut self, kind: WordMoveKind) {
        let Some(line) = self.read_viewport_line_for_cursor() else {
            return;
        };
        let Some(ref mut cm) = self.copy_mode else {
            return;
        };
        let chars: Vec<char> = line.chars().collect();
        let col = cm.cursor_col as usize;
        let len = chars.len();

        let is_word = |c: char, big: bool| -> bool {
            if big {
                !c.is_whitespace()
            } else {
                c.is_alphanumeric() || c == '_'
            }
        };
        let is_sep = |c: char| -> bool { !c.is_alphanumeric() && c != '_' && !c.is_whitespace() };

        let new_col = match kind {
            WordMoveKind::NextWord | WordMoveKind::NextBigWord => {
                let big = matches!(kind, WordMoveKind::NextBigWord);
                let mut i = col;
                if i < len && is_word(chars[i], big) {
                    while i < len && is_word(chars[i], big) {
                        i += 1;
                    }
                } else if !big && i < len && is_sep(chars[i]) {
                    while i < len && is_sep(chars[i]) {
                        i += 1;
                    }
                } else {
                    i += 1;
                }
                while i < len && chars[i].is_whitespace() {
                    i += 1;
                }
                if i >= len { col } else { i }
            }
            WordMoveKind::PrevWord | WordMoveKind::PrevBigWord => {
                let big = matches!(kind, WordMoveKind::PrevBigWord);
                if col == 0 {
                    0
                } else {
                    let mut i = col - 1;
                    while i > 0 && chars[i].is_whitespace() {
                        i -= 1;
                    }
                    if is_word(chars[i], big) {
                        while i > 0 && is_word(chars[i - 1], big) {
                            i -= 1;
                        }
                    } else if !big && is_sep(chars[i]) {
                        while i > 0 && is_sep(chars[i - 1]) {
                            i -= 1;
                        }
                    }
                    i
                }
            }
            WordMoveKind::EndWord | WordMoveKind::EndBigWord => {
                let big = matches!(kind, WordMoveKind::EndBigWord);
                if col + 1 >= len {
                    col
                } else {
                    let mut i = col + 1;
                    while i < len && chars[i].is_whitespace() {
                        i += 1;
                    }
                    if i < len && is_word(chars[i], big) {
                        while i + 1 < len && is_word(chars[i + 1], big) {
                            i += 1;
                        }
                    } else if !big && i < len && is_sep(chars[i]) {
                        while i + 1 < len && is_sep(chars[i + 1]) {
                            i += 1;
                        }
                    }
                    i
                }
            }
        };

        cm.cursor_col = new_col as u32;
        self.copy_mode_ensure_visible();
        self.update_copy_mode_highlight();
    }

    pub(crate) fn copy_mode_execute_jump(&mut self, target: char, kind: JumpKind) {
        let Some(line) = self.read_viewport_line_for_cursor() else {
            return;
        };
        let Some(ref mut cm) = self.copy_mode else {
            return;
        };
        let col = cm.cursor_col as usize;
        let chars: Vec<char> = line.chars().collect();

        let new_col = match kind {
            JumpKind::Forward => chars
                .iter()
                .enumerate()
                .skip(col + 1)
                .find(|(_, c)| **c == target)
                .map(|(i, _)| i),
            JumpKind::Backward => chars
                .iter()
                .enumerate()
                .take(col)
                .rev()
                .find(|(_, c)| **c == target)
                .map(|(i, _)| i),
            JumpKind::ToForward => chars
                .iter()
                .enumerate()
                .skip(col + 1)
                .find(|(_, c)| **c == target)
                .map(|(i, _)| i.saturating_sub(1).max(col + 1)),
            JumpKind::ToBackward => chars
                .iter()
                .enumerate()
                .take(col)
                .rev()
                .find(|(_, c)| **c == target)
                .map(|(i, _)| (i + 1).min(col.saturating_sub(1))),
        };

        if let Some(nc) = new_col {
            cm.cursor_col = nc as u32;
        }
        self.update_copy_mode_highlight();
    }

    pub(crate) fn copy_mode_jump_repeat(&mut self, reverse: bool) {
        let Some(ref cm) = self.copy_mode else { return };
        let Some((target, kind)) = cm.last_jump else {
            return;
        };
        let kind = if reverse {
            match kind {
                JumpKind::Forward => JumpKind::Backward,
                JumpKind::Backward => JumpKind::Forward,
                JumpKind::ToForward => JumpKind::ToBackward,
                JumpKind::ToBackward => JumpKind::ToForward,
            }
        } else {
            kind
        };
        self.copy_mode_execute_jump(target, kind);
    }

    pub(crate) fn copy_mode_paragraph(&mut self, forward: bool) {
        let Some(ref mut cm) = self.copy_mode else {
            return;
        };
        let offset = self.scrollbar.offset as i64;
        let max_row = self.scrollbar.total as i64;

        if forward {
            let mut r = cm.cursor_row + 1;
            while r <= max_row {
                let vp = (r - offset).max(0) as u32;
                if let Some(line) = self.read_viewport_line(vp) {
                    if line.trim().is_empty() {
                        if let Some(ref mut cm) = self.copy_mode {
                            cm.cursor_row = r;
                            cm.cursor_col = 0;
                        }
                        break;
                    }
                } else {
                    break;
                }
                r += 1;
            }
        } else {
            let mut r = cm.cursor_row - 1;
            while r >= 0 {
                let vp = (r - offset).max(0) as u32;
                if let Some(line) = self.read_viewport_line(vp) {
                    if line.trim().is_empty() {
                        if let Some(ref mut cm) = self.copy_mode {
                            cm.cursor_row = r;
                            cm.cursor_col = 0;
                        }
                        break;
                    }
                } else {
                    break;
                }
                r -= 1;
            }
        }
        self.copy_mode_ensure_visible();
        self.update_copy_mode_highlight();
    }

    pub(crate) fn copy_mode_matching_bracket(&mut self) {
        let Some(line) = self.read_viewport_line_for_cursor() else {
            return;
        };
        let Some(ref mut cm) = self.copy_mode else {
            return;
        };
        let chars: Vec<char> = line.chars().collect();
        let col = cm.cursor_col as usize;

        let brackets = [('(', ')'), ('[', ']'), ('{', '}')];
        let mut found = None;
        for i in col..chars.len() {
            for &(open, close) in &brackets {
                if chars[i] == open {
                    found = Some((i, open, close, true));
                    break;
                } else if chars[i] == close {
                    found = Some((i, open, close, false));
                    break;
                }
            }
            if found.is_some() {
                break;
            }
        }

        let Some((pos, open, close, is_open)) = found else {
            return;
        };
        let mut depth = 0i32;
        if is_open {
            for (i, ch) in chars.iter().enumerate().skip(pos) {
                if *ch == open {
                    depth += 1;
                }
                if *ch == close {
                    depth -= 1;
                }
                if depth == 0 {
                    cm.cursor_col = i as u32;
                    break;
                }
            }
        } else {
            for i in (0..=pos).rev() {
                if chars[i] == close {
                    depth += 1;
                }
                if chars[i] == open {
                    depth -= 1;
                }
                if depth == 0 {
                    cm.cursor_col = i as u32;
                    break;
                }
            }
        }
        self.update_copy_mode_highlight();
    }

    pub(crate) fn copy_mode_word_under_cursor(&self) -> Option<String> {
        let line = self.read_viewport_line_for_cursor()?;
        let cm = self.copy_mode.as_ref()?;
        let chars: Vec<char> = line.chars().collect();
        let col = cm.cursor_col as usize;
        if col >= chars.len() {
            return None;
        }

        let is_word = |c: char| c.is_alphanumeric() || c == '_';
        if !is_word(chars[col]) {
            return None;
        }

        let mut start = col;
        while start > 0 && is_word(chars[start - 1]) {
            start -= 1;
        }
        let mut end = col;
        while end + 1 < chars.len() && is_word(chars[end + 1]) {
            end += 1;
        }

        Some(chars[start..=end].iter().collect())
    }

    pub(crate) fn copy_mode_append_copy(&mut self) {
        let existing = if self.last_clipboard_text.is_empty() {
            platform::clipboard_read().unwrap_or_default()
        } else {
            self.last_clipboard_text.clone()
        };

        let Some(ref cm) = self.copy_mode else { return };
        let Some((anchor_row, anchor_col)) = cm.sel_anchor else {
            return;
        };

        let (r1, c1, r2, c2) = if anchor_row < cm.cursor_row
            || (anchor_row == cm.cursor_row && anchor_col <= cm.cursor_col)
        {
            (anchor_row, anchor_col, cm.cursor_row, cm.cursor_col)
        } else {
            (cm.cursor_row, cm.cursor_col, anchor_row, anchor_col)
        };
        let (c1, c2) = if cm.selection == SelectionMode::Line {
            (0u32, cm.viewport_cols.saturating_sub(1))
        } else {
            (c1, c2)
        };

        let sel = ffi::ghostty_selection_s {
            top_left: ffi::ghostty_point_s {
                tag: ffi::GHOSTTY_POINT_VIEWPORT,
                coord: ffi::GHOSTTY_POINT_COORD_EXACT,
                x: c1,
                y: (r1 - self.scrollbar.offset as i64).max(0) as u32,
            },
            bottom_right: ffi::ghostty_point_s {
                tag: ffi::GHOSTTY_POINT_VIEWPORT,
                coord: ffi::GHOSTTY_POINT_COORD_EXACT,
                x: c2,
                y: (r2 - self.scrollbar.offset as i64).max(0) as u32,
            },
            rectangle: cm.selection == SelectionMode::Rectangle,
        };
        if let Some(new_text) = self.read_surface_selection_text(sel) {
            let combined = format!("{existing}{new_text}");
            platform::clipboard_write(&combined);
            self.last_clipboard_text = combined;
            log::info!("copy mode: appended {} bytes to clipboard", new_text.len());
        }
    }

    pub(crate) fn enter_copy_mode(&mut self) {
        let scale = self.scale_factor();
        let cell_w_pts = self.cell_width / scale;
        let Some((col, row, cell_h_pts)) = self.focused_cursor_cell_position() else {
            return;
        };

        let viewport_rows = if cell_h_pts > 0.0 {
            ((self.last_size.height as f64 - STATUS_BAR_HEIGHT) / cell_h_pts) as u32
        } else {
            24
        };

        let frame = self.terminal_frame();
        let viewport_cols = if cell_w_pts > 0.0 {
            (frame.size.width / cell_w_pts) as u32
        } else {
            80
        };

        let cursor_layer = platform::create_highlight_layer();
        self.copy_mode = Some(CopyModeState {
            cursor_row: self.scrollbar.offset as i64 + row,
            cursor_col: col,
            selection: SelectionMode::None,
            sel_anchor: None,
            highlight_layers: Vec::new(),
            cursor_layer,
            cell_width: cell_w_pts,
            cell_height: cell_h_pts,
            viewport_rows,
            viewport_cols,
            mark: None,
            last_jump: None,
            last_search_forward: true,
            pending_jump: None,
            show_position: false,
        });
        self.bindings.enter_copy_mode();
        self.update_copy_mode_highlight();
    }

    pub(crate) fn exit_copy_mode(&mut self) {
        if let Some(cm) = self.copy_mode.take() {
            platform::update_highlight_layer(cm.cursor_layer, 0.0, 0.0, 0.0, 0.0, false, false);
            for layer in &cm.highlight_layers {
                platform::update_highlight_layer(*layer, 0.0, 0.0, 0.0, 0.0, false, false);
            }
        }
        self.bindings.exit_copy_mode();
        self.ghostty_binding_action("scroll_to_bottom");
        self.ghostty_binding_action("end_search");
        self.search_active = false;
    }

    pub(crate) fn update_copy_mode_highlight(&mut self) {
        let Some(ref cm) = self.copy_mode else { return };

        let frame = self.terminal_frame();
        let term_y = frame.origin.y;
        let offset = self.scrollbar.offset as i64;
        let viewport_row = cm.cursor_row - offset;
        let px = cm.cursor_col as f64 * cm.cell_width;
        let py = term_y + viewport_row as f64 * cm.cell_height;

        platform::update_highlight_layer(cm.cursor_layer, px, py, 2.0, cm.cell_height, true, false);

        let rects = if cm.selection != SelectionMode::None {
            if let Some((anchor_row, anchor_col)) = cm.sel_anchor {
                Self::compute_selection_rects_static(
                    cm.selection,
                    cm.cursor_row,
                    cm.cursor_col,
                    anchor_row,
                    anchor_col,
                    offset,
                    cm.viewport_cols,
                    cm.cell_width,
                    cm.cell_height,
                    term_y,
                )
            } else {
                vec![]
            }
        } else {
            vec![]
        };

        let cm = self.copy_mode.as_mut().unwrap();
        while cm.highlight_layers.len() < rects.len() {
            cm.highlight_layers.push(platform::create_highlight_layer());
        }
        for (i, &(x, y, w, h)) in rects.iter().enumerate() {
            platform::update_highlight_layer(cm.highlight_layers[i], x, y, w, h, true, true);
        }
        for i in rects.len()..cm.highlight_layers.len() {
            platform::update_highlight_layer(
                cm.highlight_layers[i],
                0.0,
                0.0,
                0.0,
                0.0,
                false,
                true,
            );
        }
    }

    pub(crate) fn compute_selection_rects_static(
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
        let (r1, c1, r2, c2) =
            if anchor_row < cursor_row || (anchor_row == cursor_row && anchor_col <= cursor_col) {
                (anchor_row, anchor_col, cursor_row, cursor_col)
            } else {
                (cursor_row, cursor_col, anchor_row, anchor_col)
            };
        let full_w = viewport_cols as f64 * cell_width;

        match selection {
            SelectionMode::Char => {
                if r1 == r2 {
                    let x = c1 as f64 * cell_width;
                    let y = term_y + (r1 - offset) as f64 * cell_height;
                    let w = (c2 as f64 - c1 as f64 + 1.0) * cell_width;
                    vec![(x, y, w, cell_height)]
                } else {
                    let mut rects = Vec::new();
                    let y1 = term_y + (r1 - offset) as f64 * cell_height;
                    rects.push((
                        c1 as f64 * cell_width,
                        y1,
                        full_w - c1 as f64 * cell_width,
                        cell_height,
                    ));
                    for r in (r1 + 1)..r2 {
                        let y = term_y + (r - offset) as f64 * cell_height;
                        rects.push((0.0, y, full_w, cell_height));
                    }
                    let y2 = term_y + (r2 - offset) as f64 * cell_height;
                    rects.push((0.0, y2, (c2 as f64 + 1.0) * cell_width, cell_height));
                    rects
                }
            }
            SelectionMode::Line => (r1..=r2)
                .map(|r| {
                    let y = term_y + (r - offset) as f64 * cell_height;
                    (0.0, y, full_w, cell_height)
                })
                .collect(),
            SelectionMode::Rectangle => {
                let min_c = c1.min(c2);
                let max_c = c1.max(c2);
                let x = min_c as f64 * cell_width;
                let w = (max_c as f64 - min_c as f64 + 1.0) * cell_width;
                (r1..=r2)
                    .map(|r| {
                        let y = term_y + (r - offset) as f64 * cell_height;
                        (x, y, w, cell_height)
                    })
                    .collect()
            }
            SelectionMode::None => vec![],
        }
    }

    pub(crate) fn copy_mode_copy(&mut self) {
        let Some(ref cm) = self.copy_mode else { return };
        let Some((anchor_row, anchor_col)) = cm.sel_anchor else {
            return;
        };

        let (r1, c1, r2, c2) = if anchor_row < cm.cursor_row
            || (anchor_row == cm.cursor_row && anchor_col <= cm.cursor_col)
        {
            (anchor_row, anchor_col, cm.cursor_row, cm.cursor_col)
        } else {
            (cm.cursor_row, cm.cursor_col, anchor_row, anchor_col)
        };

        let (c1, c2) = if cm.selection == SelectionMode::Line {
            (0u32, cm.viewport_cols.saturating_sub(1))
        } else {
            (c1, c2)
        };

        let sel = ffi::ghostty_selection_s {
            top_left: ffi::ghostty_point_s {
                tag: ffi::GHOSTTY_POINT_VIEWPORT,
                coord: ffi::GHOSTTY_POINT_COORD_EXACT,
                x: c1,
                y: (r1 - self.scrollbar.offset as i64).max(0) as u32,
            },
            bottom_right: ffi::ghostty_point_s {
                tag: ffi::GHOSTTY_POINT_VIEWPORT,
                coord: ffi::GHOSTTY_POINT_COORD_EXACT,
                x: c2,
                y: (r2 - self.scrollbar.offset as i64).max(0) as u32,
            },
            rectangle: cm.selection == SelectionMode::Rectangle,
        };
        if let Some(text) = self.read_surface_selection_text(sel) {
            platform::clipboard_write(&text);
            self.last_clipboard_text = text.clone();
            log::info!("copy mode: copied {} bytes", text.len());
        }
    }
}
