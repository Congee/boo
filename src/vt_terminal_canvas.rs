#![cfg(any(target_os = "linux", target_os = "macos"))]

use crate::vt_backend_core;
use iced::advanced::renderer;
use iced::advanced::text::{self, Paragraph as _, Renderer as _};
use iced::advanced::widget::Tree;
use iced::advanced::{Layout, Widget, layout};
use iced::alignment;
use iced::font;
use iced::mouse;
use iced::widget::canvas::{self, Cache, Frame};
use iced::{Color, Font, Length, Pixels, Point, Rectangle, Renderer, Size, Theme};
use std::cell::RefCell;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub(crate) const PADDING_X: f32 = 4.0;
pub(crate) const PADDING_Y: f32 = 2.0;
#[derive(Debug)]
pub struct TerminalCanvas {
    pub snapshot: Arc<vt_backend_core::TerminalSnapshot>,
    pub cell_width: f32,
    pub cell_height: f32,
    pub font_size: f32,
    pub font_families: Arc<[&'static str]>,
    pub snapshot_generation: u64,
    pub appearance_revision: u64,
    pub background_opacity: f32,
    pub background_opacity_cells: bool,
    pub cursor_blink_visible: bool,
    pub selection_rects: Vec<TerminalSelectionRect>,
    pub selection_color: Color,
    pub selection_foreground: Option<Color>,
    pub cursor_text_color: Option<Color>,
    pub url_color: Option<Color>,
    pub preedit_text: Option<String>,
    pub viewport: Option<TerminalViewport>,
    pub paint_base: bool,
    pub paint_text: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct TerminalBackgroundCanvas {
    pub color: Color,
}

impl<Message> canvas::Program<Message> for TerminalBackgroundCanvas {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry<Renderer>> {
        let mut frame = Frame::new(renderer, bounds.size());
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), self.color);
        vec![frame.into_geometry()]
    }
}

#[derive(Debug, Clone, Copy)]
pub struct TerminalSelectionRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SelectionColSpan {
    start_col: usize,
    end_col: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TerminalViewport {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl TerminalCanvas {
    pub fn new(
        snapshot: Arc<vt_backend_core::TerminalSnapshot>,
        cell_width: f32,
        cell_height: f32,
        font_size: f32,
        font_families: Arc<[&'static str]>,
        snapshot_generation: u64,
        appearance_revision: u64,
        background_opacity: f32,
        background_opacity_cells: bool,
        cursor_blink_visible: bool,
        selection_rects: Vec<TerminalSelectionRect>,
        selection_color: Color,
        selection_foreground: Option<Color>,
        cursor_text_color: Option<Color>,
        url_color: Option<Color>,
        preedit_text: Option<String>,
    ) -> Self {
        Self {
            snapshot,
            cell_width,
            cell_height,
            font_size,
            font_families,
            snapshot_generation,
            appearance_revision,
            background_opacity,
            background_opacity_cells,
            cursor_blink_visible,
            selection_rects,
            selection_color,
            selection_foreground,
            cursor_text_color,
            url_color,
            preedit_text,
            viewport: None,
            paint_base: true,
            paint_text: true,
        }
    }

    pub fn new_with_viewport(mut self, viewport: TerminalViewport) -> Self {
        self.viewport = Some(viewport);
        self
    }

    pub fn without_base_fill(mut self) -> Self {
        self.paint_base = false;
        self
    }

    pub fn without_text_fill(mut self) -> Self {
        self.paint_text = false;
        self
    }

    fn draw_base(&self, frame: &mut Frame<Renderer>) {
        if !self.paint_base {
            return;
        }
        let (origin, size) = self.viewport_origin_and_size(frame.size());
        frame.fill_rectangle(
            origin,
            size,
            color_from_rgb(self.snapshot.colors.background, self.background_opacity),
        );
    }

    fn draw_row(&self, frame: &mut Frame<Renderer>, row_index: usize, state: &TerminalCanvasState) {
        let origin = self.viewport_origin();
        let y = origin.y + PADDING_Y + row_index as f32 * self.cell_height;
        let artifacts = state.row_artifacts.borrow();
        let artifacts = &artifacts[row_index];
        for span in &artifacts.background_spans {
            frame.fill_rectangle(
                Point::new(
                    origin.x + PADDING_X + span.start_col as f32 * self.cell_width,
                    y,
                ),
                Size::new(span.width_cols as f32 * self.cell_width, self.cell_height),
                span.color,
            );
        }

        for run in &artifacts.text_runs {
            if !self.paint_text {
                if run.underline {
                    let x = origin.x + PADDING_X + run.start_col as f32 * self.cell_width;
                    let draw_width = run.width_cols as f32 * self.cell_width;
                    let underline_y = y + self.cell_height - 2.0;
                    frame.fill_rectangle(
                        Point::new(x, underline_y),
                        Size::new(draw_width, 1.5),
                        run.fg,
                    );
                }
                continue;
            }
            let x = origin.x + PADDING_X + run.start_col as f32 * self.cell_width;
            let draw_width = run.width_cols as f32 * self.cell_width;
            let max_width = text_run_max_width(run, draw_width);
            debug_non_ascii_draw_run(run, row_index, x, y, draw_width, max_width);
            frame.fill_text(canvas::Text {
                content: run.text.clone(),
                position: Point::new(x, y),
                color: run.fg,
                size: Pixels(self.font_size),
                line_height: iced::widget::text::LineHeight::Absolute(Pixels(self.cell_height)),
                font: run.font,
                align_x: iced::widget::text::Alignment::Left,
                align_y: alignment::Vertical::Top,
                shaping: run.shaping,
                max_width,
            });

            if run.underline {
                let underline_y = y + self.cell_height - 2.0;
                frame.fill_rectangle(
                    Point::new(x, underline_y),
                    Size::new(draw_width, 1.5),
                    run.fg,
                );
            }
        }
    }

    fn draw_row_chunk(
        &self,
        frame: &mut Frame<Renderer>,
        start_row: usize,
        end_row: usize,
        state: &TerminalCanvasState,
    ) {
        let _ = frame;
        for row_index in start_row..end_row {
            self.draw_row(frame, row_index, state);
        }
    }

    fn draw_selection_overlay(&self, frame: &mut Frame<Renderer>, state: &TerminalCanvasState) {
        let origin = self.viewport_origin();
        for rect in &self.selection_rects {
            frame.fill_rectangle(
                Point::new(origin.x + rect.x + PADDING_X, origin.y + rect.y + PADDING_Y),
                Size::new(rect.width, rect.height),
                self.selection_color,
            );
        }
        self.draw_selection_foreground(frame, state);
    }

    fn draw_cursor_overlay(&self, frame: &mut Frame<Renderer>) {
        let origin = self.viewport_origin();
        let default_fg = color_from_rgb(self.snapshot.colors.foreground, 1.0);
        let cursor_bg = if self.snapshot.colors.cursor_has_value {
            color_from_rgb(self.snapshot.colors.cursor, 0.95)
        } else {
            default_fg
        };

        if self.snapshot.cursor.visible
            && self.cursor_blink_visible
            && self.snapshot.cursor.y < self.snapshot.rows
            && self.snapshot.cursor.x < self.snapshot.cols
        {
            let x = origin.x + PADDING_X + self.snapshot.cursor.x as f32 * self.cell_width;
            let y = origin.y + PADDING_Y + self.snapshot.cursor.y as f32 * self.cell_height;
            match self.snapshot.cursor.style {
                crate::vt::GHOSTTY_RENDER_STATE_CURSOR_VISUAL_STYLE_BAR => frame.fill_rectangle(
                    Point::new(x, y),
                    Size::new(2.0, self.cell_height),
                    cursor_bg,
                ),
                crate::vt::GHOSTTY_RENDER_STATE_CURSOR_VISUAL_STYLE_UNDERLINE => frame
                    .fill_rectangle(
                        Point::new(x, y + self.cell_height - 2.0),
                        Size::new(self.cell_width, 2.0),
                        cursor_bg,
                    ),
                crate::vt::GHOSTTY_RENDER_STATE_CURSOR_VISUAL_STYLE_BLOCK_HOLLOW => {
                    let path = canvas::Path::rectangle(
                        Point::new(x + 0.5, y + 0.5),
                        Size::new(
                            (self.cell_width - 1.0).max(1.0),
                            (self.cell_height - 1.0).max(1.0),
                        ),
                    );
                    frame.stroke(
                        &path,
                        canvas::Stroke::default()
                            .with_color(cursor_bg)
                            .with_width(1.0),
                    );
                }
                _ => {
                    frame.fill_rectangle(
                        Point::new(x, y),
                        Size::new(self.cell_width, self.cell_height),
                        cursor_bg,
                    );
                    self.draw_cursor_text(
                        frame,
                        self.snapshot.cursor.y as usize,
                        self.snapshot.cursor.x as usize,
                    );
                }
            }
        }

        if let Some(preedit) = self
            .preedit_text
            .as_deref()
            .filter(|text| !text.is_empty())
            .filter(|_| self.snapshot.cursor.y < self.snapshot.rows)
        {
            let x = origin.x + PADDING_X + self.snapshot.cursor.x as f32 * self.cell_width;
            let y = origin.y + PADDING_Y + self.snapshot.cursor.y as f32 * self.cell_height;
            let width = (preedit.chars().count().max(1) as f32) * self.cell_width;
            let overlay = Color::from_rgba(0.92, 0.82, 0.32, 0.18);
            let underline = Color::from_rgba(0.98, 0.86, 0.35, 0.9);
            frame.fill_rectangle(
                Point::new(x, y),
                Size::new(width, self.cell_height),
                overlay,
            );
            if self.paint_text {
                frame.fill_text(canvas::Text {
                    content: preedit.to_string(),
                    position: Point::new(x, y),
                    color: default_fg,
                    size: Pixels(self.font_size),
                    line_height: iced::widget::text::LineHeight::Absolute(Pixels(self.cell_height)),
                font: font_for_terminal_text(preedit, false, false, &self.font_families),
                    align_x: iced::widget::text::Alignment::Left,
                    align_y: alignment::Vertical::Top,
                    shaping: iced::widget::text::Shaping::Advanced,
                    max_width: non_ascii_text_max_width(preedit, width),
                });
            }
            frame.fill_rectangle(
                Point::new(x, y + self.cell_height - 2.0),
                Size::new(width, 1.5),
                underline,
            );
        }
    }

    fn draw_selection_foreground(&self, frame: &mut Frame<Renderer>, state: &TerminalCanvasState) {
        let Some(selection_foreground) = self.selection_foreground else {
            return;
        };

        let selection_row_spans = state.selection_row_spans.borrow();
        for (row_index, row) in self.snapshot.rows_data.iter().enumerate() {
            let spans = selection_row_spans.get(row_index).map(Vec::as_slice).unwrap_or(&[]);
            if spans.is_empty() {
                continue;
            }
            for run in build_selection_text_runs(
                row,
                self.snapshot.cols as usize,
                &self.font_families,
                selection_foreground,
                |col_index, display_width| cell_is_selected_in_spans(&spans, col_index, display_width),
            ) {
                self.draw_text_run(frame, row_index, &run);
            }
        }
    }

    fn draw_cursor_text(&self, frame: &mut Frame<Renderer>, row_index: usize, col_index: usize) {
        let Some(color) = self.cursor_text_color else {
            return;
        };
        let Some(row) = self.snapshot.rows_data.get(row_index) else {
            return;
        };
        let Some(cell) = row.get(col_index) else {
            return;
        };
        let run = TextRun {
            start_col: col_index,
            width_cols: usize::from(cell.display_width.max(1)),
            text: cell.text.clone(),
            fg: color,
            font: font_for_cell(cell, &self.font_families),
            underline: cell.underline != 0,
            shaping: terminal_text_shaping(),
        };
        self.draw_text_run(frame, row_index, &run);
    }

    fn draw_text_run(&self, frame: &mut Frame<Renderer>, row_index: usize, run: &TextRun) {
        if !self.paint_text {
            return;
        }
        let origin = self.viewport_origin();
        let x = origin.x + PADDING_X + run.start_col as f32 * self.cell_width;
        let y = origin.y + PADDING_Y + row_index as f32 * self.cell_height;
        let draw_width = run.width_cols as f32 * self.cell_width;
        frame.fill_text(canvas::Text {
            content: run.text.clone(),
            position: Point::new(x, y),
            color: run.fg,
            size: Pixels(self.font_size),
            line_height: iced::widget::text::LineHeight::Absolute(Pixels(self.cell_height)),
            font: run.font,
            align_x: iced::widget::text::Alignment::Left,
            align_y: alignment::Vertical::Top,
            shaping: run.shaping,
            max_width: non_ascii_text_max_width(&run.text, draw_width),
        });
    }

    fn hyperlink_at_position(&self, position: Point) -> Option<(usize, usize)> {
        let origin = self.viewport_origin();
        let local_x = position.x - origin.x - PADDING_X;
        let local_y = position.y - origin.y - PADDING_Y;
        if local_x < 0.0 || local_y < 0.0 {
            return None;
        }
        let col = (local_x / self.cell_width).floor() as usize;
        let row = (local_y / self.cell_height).floor() as usize;
        let row_cells = self.snapshot.rows_data.get(row)?;
        let cell = row_cells.get(col)?;
        cell.hyperlink.then_some((row, col))
    }
}

#[derive(Debug)]
pub struct TerminalTextLayer {
    pub snapshot: Arc<vt_backend_core::TerminalSnapshot>,
    pub cell_width: f32,
    pub cell_height: f32,
    pub font_size: f32,
    pub font_families: Arc<[&'static str]>,
    pub cursor_blink_visible: bool,
    pub selection_rects: Vec<TerminalSelectionRect>,
    pub selection_foreground: Option<Color>,
    pub cursor_text_color: Option<Color>,
    pub url_color: Option<Color>,
    pub preedit_text: Option<String>,
    pub viewport: Option<TerminalViewport>,
}

struct TerminalTextLayerState {
    base_row_entries: Vec<Vec<TextLayerEntry>>,
    base_row_fingerprints: Vec<u64>,
    overlay_row_entries: Vec<Vec<TextLayerEntry>>,
    overlay_row_fingerprints: Vec<u64>,
    selection_row_spans: Vec<Vec<SelectionColSpan>>,
    selection_spans_fingerprint: Option<u64>,
    selection_row_range: Option<(usize, usize)>,
    style_fingerprint: Option<u64>,
}

impl Default for TerminalTextLayerState {
    fn default() -> Self {
        Self {
            base_row_entries: Vec::new(),
            base_row_fingerprints: Vec::new(),
            overlay_row_entries: Vec::new(),
            overlay_row_fingerprints: Vec::new(),
            selection_row_spans: Vec::new(),
            selection_spans_fingerprint: None,
            selection_row_range: None,
            style_fingerprint: None,
        }
    }
}

struct TextLayerEntry {
    paragraph: <iced::Renderer as text::Renderer>::Paragraph,
    position: Point,
    color: Color,
}

impl TerminalTextLayer {
    pub fn new(
        snapshot: Arc<vt_backend_core::TerminalSnapshot>,
        cell_width: f32,
        cell_height: f32,
        font_size: f32,
        font_families: Arc<[&'static str]>,
        cursor_blink_visible: bool,
        selection_rects: Vec<TerminalSelectionRect>,
        selection_foreground: Option<Color>,
        cursor_text_color: Option<Color>,
        url_color: Option<Color>,
        preedit_text: Option<String>,
    ) -> Self {
        Self {
            snapshot,
            cell_width,
            cell_height,
            font_size,
            font_families,
            cursor_blink_visible,
            selection_rects,
            selection_foreground,
            cursor_text_color,
            url_color,
            preedit_text,
            viewport: None,
        }
    }

    pub fn new_with_viewport(mut self, viewport: TerminalViewport) -> Self {
        self.viewport = Some(viewport);
        self
    }

    fn viewport_rect(&self, bounds: Rectangle) -> Rectangle {
        self.viewport
            .map(|viewport| Rectangle {
                x: viewport.x,
                y: viewport.y,
                width: viewport.width,
                height: viewport.height,
            })
            .unwrap_or(bounds)
    }

    fn viewport_origin(&self, bounds: Rectangle) -> Point {
        let viewport = self.viewport_rect(bounds);
        Point::new(viewport.x, viewport.y)
    }

    fn build_base_row_entries(&self, viewport: Rectangle, row_index: usize) -> Vec<TextLayerEntry> {
        let Some(row) = self.snapshot.rows_data.get(row_index) else {
            return Vec::new();
        };
        let mut entries = Vec::new();
        for (col_index, cell) in row.iter().enumerate() {
            let content = cell.text.as_str();
            if content.is_empty() || content == "\0" {
                continue;
            }
            let underline = cell.underline != 0;
            if content == " " && !underline {
                continue;
            }
            self.push_text_entry(
                &mut entries,
                viewport,
                row_index,
                col_index,
                content,
                usize::from(cell.display_width.max(1)),
                font_for_cell(cell, &self.font_families),
                terminal_text_shaping(),
                if cell.hyperlink {
                    self.url_color
                        .unwrap_or_else(|| color_from_rgb(cell.fg, 1.0))
                } else {
                    color_from_rgb(cell.fg, 1.0)
                },
            );
        }
        entries
    }

    fn build_overlay_row_entries(
        &self,
        viewport: Rectangle,
        row_index: usize,
        selection_spans: &[SelectionColSpan],
    ) -> Vec<TextLayerEntry> {
        let mut entries = Vec::new();
        let Some(row) = self.snapshot.rows_data.get(row_index) else {
            return entries;
        };

        if let Some(selection_foreground) = self.selection_foreground.filter(|_| !selection_spans.is_empty()) {
            for (col_index, cell) in row.iter().enumerate() {
                let content = cell.text.as_str();
                if content.is_empty() || content == "\0" {
                    continue;
                }
                let underline = cell.underline != 0;
                if content == " " && !underline {
                    continue;
                }
                if !cell_is_selected_in_spans(selection_spans, col_index, cell.display_width) {
                    continue;
                }
                self.push_text_entry(
                    &mut entries,
                    viewport,
                    row_index,
                    col_index,
                    content,
                    usize::from(cell.display_width.max(1)),
                    font_for_cell(cell, &self.font_families),
                    terminal_text_shaping(),
                    selection_foreground,
                );
            }
        }

        if let Some(cursor_text_color) = self
            .cursor_text_color
            .filter(|_| self.snapshot.cursor.y as usize == row_index)
        {
            for (col_index, cell) in row.iter().enumerate() {
                if !cell.text.is_empty() && self.should_draw_cursor_text(row_index, col_index) {
                    self.push_text_entry(
                        &mut entries,
                        viewport,
                        row_index,
                        col_index,
                        cell.text.as_str(),
                        usize::from(cell.display_width.max(1)),
                        font_for_cell(cell, &self.font_families),
                        terminal_text_shaping(),
                        cursor_text_color,
                    );
                }
            }
        }

        if let Some(preedit) = self
            .preedit_text
            .as_deref()
            .filter(|text| !text.is_empty())
            .filter(|_| self.snapshot.cursor.y < self.snapshot.rows)
            .filter(|_| self.snapshot.cursor.y as usize == row_index)
        {
            self.push_text_entry(
                &mut entries,
                viewport,
                self.snapshot.cursor.y as usize,
                self.snapshot.cursor.x as usize,
                preedit,
                preedit.chars().count().max(1),
                font_for_terminal_text(
                    preedit,
                    false,
                    false,
                    &self.font_families,
                ),
                terminal_text_shaping(),
                color_from_rgb(self.snapshot.colors.foreground, 1.0),
            );
        }

        entries
    }

    fn push_text_entry(
        &self,
        entries: &mut Vec<TextLayerEntry>,
        viewport: Rectangle,
        row_index: usize,
        col_index: usize,
        content: &str,
        width_cols: usize,
        font: Font,
        shaping: iced::widget::text::Shaping,
        color: Color,
    ) {
        if content.is_empty() || content == "\0" {
            return;
        }

        let origin = self.viewport_origin(viewport);
        let x = origin.x + PADDING_X + col_index as f32 * self.cell_width;
        let y = origin.y + PADDING_Y + row_index as f32 * self.cell_height;
        let draw_width = width_cols as f32 * self.cell_width;
        let bounds = Size::new(
            available_text_width(viewport, x),
            self.cell_height.max(draw_width.min(self.cell_height)),
        );
        debug_text_layer_draw_run(
            content, row_index, col_index, width_cols, x, y, bounds.width, font, shaping, color,
        );
        let paragraph = <iced::Renderer as text::Renderer>::Paragraph::with_text(
            iced::advanced::text::Text {
                content,
                bounds,
                size: Pixels(self.font_size),
                line_height: iced::advanced::text::LineHeight::Absolute(Pixels(
                    self.cell_height,
                )),
                font,
                align_x: iced::advanced::text::Alignment::Left,
                align_y: alignment::Vertical::Top,
                shaping,
                wrapping: iced::advanced::text::Wrapping::None,
            },
        );
        entries.push(TextLayerEntry {
            paragraph,
            position: Point::new(x, y),
            color,
        });
    }

    fn should_draw_cursor_text(&self, row_index: usize, col_index: usize) -> bool {
        self.snapshot.cursor.visible
            && self.cursor_blink_visible
            && self.snapshot.cursor.y as usize == row_index
            && self.snapshot.cursor.x as usize == col_index
            && self.snapshot.cursor.style != crate::vt::GHOSTTY_RENDER_STATE_CURSOR_VISUAL_STYLE_BAR
            && self.snapshot.cursor.style
                != crate::vt::GHOSTTY_RENDER_STATE_CURSOR_VISUAL_STYLE_UNDERLINE
            && self.snapshot.cursor.style
                != crate::vt::GHOSTTY_RENDER_STATE_CURSOR_VISUAL_STYLE_BLOCK_HOLLOW
    }

    fn style_fingerprint(&self, viewport: Rectangle) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.font_size.to_bits().hash(&mut hasher);
        self.cell_width.to_bits().hash(&mut hasher);
        self.cell_height.to_bits().hash(&mut hasher);
        self.font_families.hash(&mut hasher);
        self.url_color.map(|c| c.r.to_bits()).hash(&mut hasher);
        self.url_color.map(|c| c.g.to_bits()).hash(&mut hasher);
        self.url_color.map(|c| c.b.to_bits()).hash(&mut hasher);
        self.url_color.map(|c| c.a.to_bits()).hash(&mut hasher);
        viewport.x.to_bits().hash(&mut hasher);
        viewport.y.to_bits().hash(&mut hasher);
        viewport.width.to_bits().hash(&mut hasher);
        viewport.height.to_bits().hash(&mut hasher);
        hasher.finish()
    }

    fn selection_spans_fingerprint(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.snapshot.rows.hash(&mut hasher);
        self.snapshot.cols.hash(&mut hasher);
        self.cell_width.to_bits().hash(&mut hasher);
        self.cell_height.to_bits().hash(&mut hasher);
        self.selection_rects.len().hash(&mut hasher);
        for rect in &self.selection_rects {
            rect.x.to_bits().hash(&mut hasher);
            rect.y.to_bits().hash(&mut hasher);
            rect.width.to_bits().hash(&mut hasher);
            rect.height.to_bits().hash(&mut hasher);
        }
        hasher.finish()
    }

    fn base_row_fingerprint(&self, row_index: usize, style_fingerprint: u64) -> u64 {
        style_fingerprint
            ^ self
                .snapshot
                .row_revisions
                .get(row_index)
                .copied()
                .unwrap_or_default()
                .rotate_left(17)
            ^ (row_index as u64).rotate_left(33)
    }

    fn overlay_row_fingerprint(
        &self,
        row_index: usize,
        viewport: Rectangle,
        style_fingerprint: u64,
        selection_spans: &[SelectionColSpan],
    ) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        style_fingerprint.hash(&mut hasher);
        row_index.hash(&mut hasher);
        self.snapshot
            .row_revisions
            .get(row_index)
            .copied()
            .unwrap_or_default()
            .hash(&mut hasher);

        if !selection_spans.is_empty() {
            self.selection_foreground
                .map(|c| (c.r.to_bits(), c.g.to_bits(), c.b.to_bits(), c.a.to_bits()))
                .hash(&mut hasher);
            for span in selection_spans {
                span.start_col.hash(&mut hasher);
                span.end_col.hash(&mut hasher);
            }
        }

        if self.snapshot.cursor.y as usize == row_index {
            self.cursor_blink_visible.hash(&mut hasher);
            self.snapshot.cursor.visible.hash(&mut hasher);
            self.snapshot.cursor.x.hash(&mut hasher);
            self.snapshot.cursor.y.hash(&mut hasher);
            self.snapshot.cursor.style.hash(&mut hasher);
            self.cursor_text_color
                .map(|c| (c.r.to_bits(), c.g.to_bits(), c.b.to_bits(), c.a.to_bits()))
                .hash(&mut hasher);
            self.preedit_text.hash(&mut hasher);
        }
        viewport.x.to_bits().hash(&mut hasher);
        viewport.y.to_bits().hash(&mut hasher);
        viewport.width.to_bits().hash(&mut hasher);
        viewport.height.to_bits().hash(&mut hasher);
        hasher.finish()
    }
}

impl<Message> Widget<Message, Theme, iced::Renderer> for TerminalTextLayer {
    fn tag(&self) -> iced::advanced::widget::tree::Tag {
        iced::advanced::widget::tree::Tag::of::<TerminalTextLayerState>()
    }

    fn state(&self) -> iced::advanced::widget::tree::State {
        iced::advanced::widget::tree::State::new(TerminalTextLayerState::default())
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }

    fn layout(
        &mut self,
        _tree: &mut Tree,
        _renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        layout::atomic(limits, Length::Fill, Length::Fill)
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        _theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        let viewport = self.viewport_rect(layout.bounds());
        let state = tree.state.downcast_ref::<TerminalTextLayerState>();
        for row_entries in &state.base_row_entries {
            for entry in row_entries {
                renderer.fill_paragraph(
                    &entry.paragraph,
                    entry.position,
                    entry.color,
                    viewport,
                );
            }
        }
        for row_entries in &state.overlay_row_entries {
            for entry in row_entries {
                renderer.fill_paragraph(
                    &entry.paragraph,
                    entry.position,
                    entry.color,
                    viewport,
                );
            }
        }
    }

    fn diff(&self, tree: &mut Tree) {
        let state = tree.state.downcast_mut::<TerminalTextLayerState>();
        let bounds = self
            .viewport
            .map(|viewport| Rectangle {
                x: viewport.x,
                y: viewport.y,
                width: viewport.width,
                height: viewport.height,
            })
            .unwrap_or(Rectangle {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            });
        let viewport = self.viewport_rect(bounds);
        let style_fingerprint = self.style_fingerprint(viewport);
        let style_changed = state.style_fingerprint != Some(style_fingerprint);
        let row_count = self.snapshot.rows_data.len();
        state.base_row_entries.resize_with(row_count, Vec::new);
        state.base_row_entries.truncate(row_count);
        state.base_row_fingerprints.resize(row_count, 0);
        state.base_row_fingerprints.truncate(row_count);
        state.overlay_row_entries.resize_with(row_count, Vec::new);
        state.overlay_row_entries.truncate(row_count);
        state.overlay_row_fingerprints.resize(row_count, 0);
        state.overlay_row_fingerprints.truncate(row_count);
        state.selection_row_spans.resize_with(row_count, Vec::new);
        state.selection_row_spans.truncate(row_count);
        let selection_spans_fingerprint = self.selection_spans_fingerprint();
        let new_selection_row_range =
            selection_row_range(&self.selection_rects, self.cell_height, row_count);
        if style_changed || state.selection_spans_fingerprint != Some(selection_spans_fingerprint) {
            let previous_range = state.selection_row_range;
            let affected_range = match (previous_range, new_selection_row_range) {
                (Some((old_start, old_end)), Some((new_start, new_end))) => {
                    Some((old_start.min(new_start), old_end.max(new_end)))
                }
                (Some(range), None) | (None, Some(range)) => Some(range),
                (None, None) => None,
            };
            if let Some((affected_start, affected_end)) = affected_range {
                for row_index in affected_start..affected_end.min(row_count) {
                    state.selection_row_spans[row_index] = if new_selection_row_range
                        .is_some_and(|(new_start, new_end)| {
                            row_index >= new_start && row_index < new_end
                        }) {
                        selection_col_spans_for_row(
                            &self.selection_rects,
                            row_index,
                            self.cell_width,
                            self.cell_height,
                            self.snapshot.cols as usize,
                        )
                    } else {
                        Vec::new()
                    };
                }
            }
            state.selection_spans_fingerprint = Some(selection_spans_fingerprint);
            state.selection_row_range = new_selection_row_range;
        }
        for row_index in 0..row_count {
            let fingerprint = self.base_row_fingerprint(row_index, style_fingerprint);
            if style_changed || state.base_row_fingerprints[row_index] != fingerprint {
                state.base_row_entries[row_index] = self.build_base_row_entries(viewport, row_index);
                state.base_row_fingerprints[row_index] = fingerprint;
            }
            let overlay_fingerprint =
                self.overlay_row_fingerprint(
                    row_index,
                    viewport,
                    style_fingerprint,
                    &state.selection_row_spans[row_index],
                );
            if style_changed || state.overlay_row_fingerprints[row_index] != overlay_fingerprint {
                state.overlay_row_entries[row_index] =
                    self.build_overlay_row_entries(
                        viewport,
                        row_index,
                        &state.selection_row_spans[row_index],
                    );
                state.overlay_row_fingerprints[row_index] = overlay_fingerprint;
            }
        }
        state.style_fingerprint = Some(style_fingerprint);
        tree.children.clear();
    }
}

pub struct TerminalCanvasState {
    base_cache: Cache,
    base_fingerprint: RefCell<Option<u64>>,
    row_band_caches: RefCell<Vec<Cache>>,
    row_style_fingerprint: RefCell<Option<u64>>,
    row_seen_revisions: RefCell<Vec<u64>>,
    row_artifact_fingerprints: RefCell<Vec<u64>>,
    row_artifacts: RefCell<Vec<RowArtifacts>>,
    selection_overlay_cache: Cache,
    selection_overlay_fingerprint: RefCell<Option<u64>>,
    cursor_overlay_cache: Cache,
    cursor_overlay_fingerprint: RefCell<Option<u64>>,
    selection_row_spans: RefCell<Vec<Vec<SelectionColSpan>>>,
    selection_spans_fingerprint: RefCell<Option<u64>>,
    selection_row_range: RefCell<Option<(usize, usize)>>,
}

impl Default for TerminalCanvasState {
    fn default() -> Self {
        Self {
            base_cache: Cache::new(),
            base_fingerprint: RefCell::new(None),
            row_band_caches: RefCell::new(Vec::new()),
            row_style_fingerprint: RefCell::new(None),
            row_seen_revisions: RefCell::new(Vec::new()),
            row_artifact_fingerprints: RefCell::new(Vec::new()),
            row_artifacts: RefCell::new(Vec::new()),
            selection_overlay_cache: Cache::new(),
            selection_overlay_fingerprint: RefCell::new(None),
            cursor_overlay_cache: Cache::new(),
            cursor_overlay_fingerprint: RefCell::new(None),
            selection_row_spans: RefCell::new(Vec::new()),
            selection_spans_fingerprint: RefCell::new(None),
            selection_row_range: RefCell::new(None),
        }
    }
}

impl<Message> canvas::Program<Message> for TerminalCanvas {
    type State = TerminalCanvasState;

    fn draw(
        &self,
        state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry<Renderer>> {
        let _profile_scope =
            crate::profiling::scope("client.canvas.draw", crate::profiling::Kind::Cpu);
        let started_at = render_debug_enabled().then(Instant::now);
        let base_fingerprint = self.base_fingerprint();
        {
            let mut cached = state.base_fingerprint.borrow_mut();
            if cached.as_ref() != Some(&base_fingerprint) {
                state.base_cache.clear();
                *cached = Some(base_fingerprint);
            }
        }

        let mut geometries = vec![state.base_cache.draw(renderer, bounds.size(), |frame| {
            self.draw_base(frame);
        })];

        {
            let row_count = self.snapshot.rows_data.len();
            let chunk_size = self.row_cache_chunk_size();
            let chunk_count = row_count.div_ceil(chunk_size);
            let row_style_fingerprint = self.row_style_fingerprint();
            let style_changed = {
                let mut cached_style = state.row_style_fingerprint.borrow_mut();
                let changed = cached_style.as_ref() != Some(&row_style_fingerprint);
                if changed {
                    for cache in state.row_band_caches.borrow_mut().iter_mut() {
                        cache.clear();
                    }
                }
                *cached_style = Some(row_style_fingerprint);
                changed
            };
            let mut row_seen_revisions = state.row_seen_revisions.borrow_mut();
            row_seen_revisions.resize(row_count, 0);
            row_seen_revisions.truncate(row_count);
            let mut row_artifact_fingerprints = state.row_artifact_fingerprints.borrow_mut();
            let mut row_artifacts = state.row_artifacts.borrow_mut();
            row_artifact_fingerprints.resize(row_count, 0);
            row_artifact_fingerprints.truncate(row_count);
            row_artifacts.resize_with(row_count, RowArtifacts::default);
            row_artifacts.truncate(row_count);
            let mut dirty_rows = 0u64;
            let mut dirty_chunks = 0u64;
            let mut chunk_has_artifacts = vec![false; chunk_count];
            let mut chunk_dirty_flags = vec![style_changed; chunk_count];

            for chunk_index in 0..chunk_count {
                let start_row = chunk_index * chunk_size;
                let end_row = (start_row + chunk_size).min(row_count);
                let mut chunk_dirty = style_changed;
                for row_index in start_row..end_row {
                    let row_fingerprint = self.row_fingerprint(row_index);
                    if row_artifact_fingerprints[row_index] != row_fingerprint {
                        let row = &self.snapshot.rows_data[row_index];
                        row_artifacts[row_index] = RowArtifacts {
                            background_spans: build_background_spans(
                                row,
                                self.snapshot.cols as usize,
                                self.background_opacity,
                                self.background_opacity_cells,
                            ),
                            text_runs: build_text_runs(
                                row,
                                self.snapshot.cols as usize,
                                &self.font_families,
                                self.url_color,
                            ),
                        };
                        row_artifact_fingerprints[row_index] = row_fingerprint;
                    }
                    let revision = self.row_revision_value(row_index);
                    if row_seen_revisions[row_index] != revision {
                        row_seen_revisions[row_index] = revision;
                        chunk_dirty = true;
                        dirty_rows += 1;
                    }
                }
                chunk_has_artifacts[chunk_index] = (start_row..end_row).any(|row_index| {
                    let artifacts = &row_artifacts[row_index];
                    !artifacts.background_spans.is_empty() || !artifacts.text_runs.is_empty()
                });
                if chunk_dirty {
                    dirty_chunks += 1;
                }
                chunk_dirty_flags[chunk_index] = chunk_dirty;
            }
            drop(row_artifact_fingerprints);
            drop(row_artifacts);

            let mut bands = Vec::new();
            let mut chunk_index = 0usize;
            while chunk_index < chunk_count {
                if !chunk_has_artifacts[chunk_index] {
                    chunk_index += 1;
                    continue;
                }
                let band_start = chunk_index;
                let mut band_end = chunk_index + 1;
                let mut band_dirty = chunk_dirty_flags[chunk_index];
                while band_end < chunk_count && chunk_has_artifacts[band_end] {
                    band_dirty |= chunk_dirty_flags[band_end];
                    band_end += 1;
                }
                bands.push((band_start, band_end, band_dirty));
                chunk_index = band_end;
            }

            let mut row_band_caches = state.row_band_caches.borrow_mut();
            if row_band_caches.len() < bands.len() {
                row_band_caches.resize_with(bands.len(), Cache::new);
            }
            row_band_caches.truncate(bands.len());

            for (band_index, (band_start, band_end, band_dirty)) in bands.into_iter().enumerate() {
                if band_dirty {
                    row_band_caches[band_index].clear();
                }
                let start_row = band_start * chunk_size;
                let end_row = (band_end * chunk_size).min(row_count);
                geometries.push(row_band_caches[band_index].draw(
                    renderer,
                    bounds.size(),
                    |frame| {
                        self.draw_row_chunk(frame, start_row, end_row, state);
                    },
                ));
            }
            crate::profiling::record_units(
                "client.canvas.changed_rows",
                crate::profiling::Kind::Cpu,
                dirty_rows,
            );
            crate::profiling::record_units(
                "client.canvas.changed_chunks",
                crate::profiling::Kind::Cpu,
                dirty_chunks,
            );
        }

        let selection_spans_fingerprint = self.selection_spans_fingerprint();
        {
            let mut cached = state.selection_spans_fingerprint.borrow_mut();
            if cached.as_ref() != Some(&selection_spans_fingerprint) {
                let row_count = self.snapshot.rows_data.len();
                let new_selection_row_range =
                    selection_row_range(&self.selection_rects, self.cell_height, row_count);
                let previous_range = *state.selection_row_range.borrow();
                let mut selection_row_spans = state.selection_row_spans.borrow_mut();
                selection_row_spans.resize_with(row_count, Vec::new);
                selection_row_spans.truncate(row_count);
                let affected_range = match (previous_range, new_selection_row_range) {
                    (Some((old_start, old_end)), Some((new_start, new_end))) => {
                        Some((old_start.min(new_start), old_end.max(new_end)))
                    }
                    (Some(range), None) | (None, Some(range)) => Some(range),
                    (None, None) => None,
                };
                if let Some((affected_start, affected_end)) = affected_range {
                    for row_index in affected_start..affected_end.min(row_count) {
                        selection_row_spans[row_index] = if new_selection_row_range
                            .is_some_and(|(new_start, new_end)| {
                                row_index >= new_start && row_index < new_end
                            }) {
                            selection_col_spans_for_row(
                                &self.selection_rects,
                                row_index,
                                self.cell_width,
                                self.cell_height,
                                self.snapshot.cols as usize,
                            )
                        } else {
                            Vec::new()
                        };
                    }
                }
                *state.selection_row_range.borrow_mut() = new_selection_row_range;
                *cached = Some(selection_spans_fingerprint);
            }
        }
        {
            let selection_overlay_fingerprint = self.selection_overlay_fingerprint();
            let mut cached = state.selection_overlay_fingerprint.borrow_mut();
            if cached.as_ref() != Some(&selection_overlay_fingerprint) {
                state.selection_overlay_cache.clear();
                *cached = Some(selection_overlay_fingerprint);
            }
        }
        {
            let cursor_overlay_fingerprint = self.cursor_overlay_fingerprint();
            let mut cached = state.cursor_overlay_fingerprint.borrow_mut();
            if cached.as_ref() != Some(&cursor_overlay_fingerprint) {
                state.cursor_overlay_cache.clear();
                *cached = Some(cursor_overlay_fingerprint);
            }
        }
        geometries.push(state.selection_overlay_cache.draw(renderer, bounds.size(), |frame| {
            self.draw_selection_overlay(frame, state);
        }));
        geometries.push(state.cursor_overlay_cache.draw(renderer, bounds.size(), |frame| {
            self.draw_cursor_overlay(frame);
        }));

        if let Some(started_at) = started_at {
            let elapsed = started_at.elapsed();
            if elapsed >= Duration::from_millis(4) {
                eprintln!(
                    "boo_render draw_ms={} rows={} cols={}",
                    elapsed.as_secs_f64() * 1000.0,
                    self.snapshot.rows,
                    self.snapshot.cols
                );
            }
        }

        geometries
    }

    fn mouse_interaction(
        &self,
        _state: &Self::State,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        let Some(position) = cursor.position_in(bounds) else {
            return mouse::Interaction::default();
        };
        if self.hyperlink_at_position(position).is_some() {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::default()
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct BackgroundSpan {
    start_col: usize,
    width_cols: usize,
    color: Color,
}

#[derive(Debug, Clone)]
struct TextRun {
    start_col: usize,
    width_cols: usize,
    text: String,
    fg: Color,
    font: Font,
    underline: bool,
    shaping: iced::widget::text::Shaping,
}

#[derive(Debug, Clone, Default)]
struct RowArtifacts {
    background_spans: Vec<BackgroundSpan>,
    text_runs: Vec<TextRun>,
}

fn build_background_spans(
    row: &[vt_backend_core::CellSnapshot],
    cols: usize,
    background_opacity: f32,
    background_opacity_cells: bool,
) -> Vec<BackgroundSpan> {
    let mut spans = Vec::new();
    let mut current: Option<BackgroundSpan> = None;

    for col_index in 0..cols {
        let Some(cell) = row.get(col_index).filter(|cell| !cell.bg_is_default) else {
            if let Some(span) = current.take() {
                spans.push(span);
            }
            continue;
        };
        let bg = color_from_rgb(
            cell.bg,
            if background_opacity_cells {
                background_opacity
            } else {
                1.0
            },
        );

        match current.as_mut() {
            Some(span) if span.color == bg && span.start_col + span.width_cols == col_index => {
                span.width_cols += 1;
            }
            _ => {
                if let Some(span) = current.take() {
                    spans.push(span);
                }
                current = Some(BackgroundSpan {
                    start_col: col_index,
                    width_cols: 1,
                    color: bg,
                });
            }
        }
    }

    if let Some(span) = current {
        spans.push(span);
    }

    spans
}

fn build_text_runs(
    row: &[vt_backend_core::CellSnapshot],
    cols: usize,
    font_families: &[&'static str],
    url_color: Option<Color>,
) -> Vec<TextRun> {
    let mut runs: Vec<TextRun> = Vec::new();

    for col_index in 0..cols {
        let Some(cell) = row.get(col_index) else {
            continue;
        };

        let content = cell.text.as_str();
        if content.is_empty() || content == "\0" {
            continue;
        }

        let underline = cell.underline != 0;
        if content == " " && !underline {
            continue;
        }

        let fg = if cell.hyperlink {
            url_color.unwrap_or_else(|| color_from_rgb(cell.fg, 1.0))
        } else {
            color_from_rgb(cell.fg, 1.0)
        };
        let font = font_for_cell(cell, font_families);
        let width_cols = usize::from(cell.display_width.max(1));
        let shaping = terminal_text_shaping();

        if let Some(previous) = runs.last_mut().filter(|previous| {
            previous.start_col + previous.width_cols == col_index
                && previous.fg == fg
                && previous.font == font
                && previous.underline == underline
                && previous.shaping == shaping
        }) {
            previous.width_cols += width_cols;
            previous.text.push_str(content);
            continue;
        }

        runs.push(TextRun {
            start_col: col_index,
            width_cols,
            text: content.to_string(),
            fg,
            font,
            underline,
            shaping,
        });
    }

    runs
}

fn build_selection_text_runs<F>(
    row: &[vt_backend_core::CellSnapshot],
    cols: usize,
    font_families: &[&'static str],
    selection_foreground: Color,
    is_selected: F,
) -> Vec<TextRun>
where
    F: Fn(usize, u8) -> bool,
{
    let mut runs: Vec<TextRun> = Vec::new();

    for col_index in 0..cols {
        let Some(cell) = row.get(col_index) else {
            continue;
        };

        let content = cell.text.as_str();
        if content.is_empty() || content == "\0" {
            continue;
        }

        let underline = cell.underline != 0;
        if content == " " && !underline {
            continue;
        }

        if !is_selected(col_index, cell.display_width) {
            continue;
        }

        let font = font_for_cell(cell, font_families);
        let width_cols = usize::from(cell.display_width.max(1));
        let shaping = terminal_text_shaping();

        if let Some(previous) = runs.last_mut().filter(|previous| {
            previous.start_col + previous.width_cols == col_index
                && previous.fg == selection_foreground
                && previous.font == font
                && previous.underline == underline
                && previous.shaping == shaping
        }) {
            previous.width_cols += width_cols;
            previous.text.push_str(content);
            continue;
        }

        runs.push(TextRun {
            start_col: col_index,
            width_cols,
            text: content.to_string(),
            fg: selection_foreground,
            font,
            underline,
            shaping,
        });
    }

    runs
}

fn font_for_cell(
    cell: &vt_backend_core::CellSnapshot,
    font_families: &[&'static str],
) -> Font {
    font_for_terminal_text(&cell.text, cell.bold, cell.italic, font_families)
}

fn font_for_terminal_text(
    _text: &str,
    bold: bool,
    italic: bool,
    font_families: &[&'static str],
) -> Font {
    let base = configured_font(primary_terminal_font(font_families));
    Font {
        family: base.family,
        weight: if bold {
            font::Weight::Bold
        } else {
            font::Weight::Normal
        },
        style: if italic {
            font::Style::Italic
        } else {
            font::Style::Normal
        },
        ..base
    }
}

fn configured_font(family: Option<&'static str>) -> Font {
    family.map(Font::with_name).unwrap_or(Font::MONOSPACE)
}

fn primary_terminal_font(font_families: &[&'static str]) -> Option<&'static str> {
    font_families.first().copied()
}

fn color_from_rgb(color: crate::vt::GhosttyColorRgb, alpha: f32) -> Color {
    Color::from_rgba8(color.r, color.g, color.b, alpha.clamp(0.0, 1.0))
}

fn rects_intersect(cell: Rectangle, selection: &TerminalSelectionRect) -> bool {
    let selection_rect = Rectangle {
        x: selection.x,
        y: selection.y,
        width: selection.width,
        height: selection.height,
    };
    cell.x < selection_rect.x + selection_rect.width
        && selection_rect.x < cell.x + cell.width
        && cell.y < selection_rect.y + selection_rect.height
        && selection_rect.y < cell.y + cell.height
}

fn selection_col_spans_for_row(
    selection_rects: &[TerminalSelectionRect],
    row_index: usize,
    cell_width: f32,
    cell_height: f32,
    cols: usize,
) -> Vec<SelectionColSpan> {
    let row_y = row_index as f32 * cell_height;
    let row_rect = Rectangle {
        x: 0.0,
        y: row_y,
        width: cols as f32 * cell_width,
        height: cell_height,
    };
    let mut spans = Vec::new();
    for rect in selection_rects {
        if !rects_intersect(row_rect, rect) {
            continue;
        }
        let start_col = (rect.x / cell_width).floor().max(0.0) as usize;
        let end_col = ((rect.x + rect.width) / cell_width).ceil().max(0.0) as usize;
        let start_col = start_col.min(cols);
        let end_col = end_col.min(cols);
        if start_col >= end_col {
            continue;
        }
        spans.push(SelectionColSpan { start_col, end_col });
    }
    spans.sort_unstable_by_key(|span| span.start_col);
    let mut merged: Vec<SelectionColSpan> = Vec::new();
    for span in spans {
        if let Some(previous) = merged
            .last_mut()
            .filter(|previous| span.start_col <= previous.end_col)
        {
            previous.end_col = previous.end_col.max(span.end_col);
        } else {
            merged.push(span);
        }
    }
    merged
}

fn selection_row_range(
    selection_rects: &[TerminalSelectionRect],
    cell_height: f32,
    rows: usize,
) -> Option<(usize, usize)> {
    let mut start = rows;
    let mut end = 0usize;
    for rect in selection_rects {
        let rect_start = (rect.y / cell_height).floor().max(0.0) as usize;
        let rect_end = ((rect.y + rect.height) / cell_height).ceil().max(0.0) as usize;
        let rect_start = rect_start.min(rows);
        let rect_end = rect_end.min(rows);
        if rect_start >= rect_end {
            continue;
        }
        start = start.min(rect_start);
        end = end.max(rect_end);
    }
    (start < end).then_some((start, end))
}

fn cell_is_selected_in_spans(
    spans: &[SelectionColSpan],
    col_index: usize,
    display_width: u8,
) -> bool {
    let cell_end = col_index + usize::from(display_width.max(1));
    spans
        .iter()
        .any(|span| col_index < span.end_col && span.start_col < cell_end)
}

fn hash_optional_color(
    color: Option<Color>,
    hasher: &mut std::collections::hash_map::DefaultHasher,
) {
    color.is_some().hash(hasher);
    if let Some(color) = color {
        color.r.to_bits().hash(hasher);
        color.g.to_bits().hash(hasher);
        color.b.to_bits().hash(hasher);
        color.a.to_bits().hash(hasher);
    }
}

fn render_debug_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("BOO_RENDER_DEBUG").is_some())
}

fn text_run_max_width(run: &TextRun, draw_width: f32) -> f32 {
    non_ascii_text_max_width(&run.text, draw_width)
}

fn non_ascii_text_max_width(text: &str, draw_width: f32) -> f32 {
    if text.is_ascii() {
        draw_width
    } else {
        f32::INFINITY
    }
}

fn available_text_width(viewport: Rectangle, x: f32) -> f32 {
    (viewport.x + viewport.width - x).max(1.0)
}

fn debug_non_ascii_draw_run(
    run: &TextRun,
    row_index: usize,
    x: f32,
    y: f32,
    draw_width: f32,
    max_width: f32,
) {
    if !render_debug_enabled() || run.text.is_ascii() {
        return;
    }
    eprintln!(
        "boo_render non_ascii_run row={} col={} text={:?} chars={} width_cols={} draw_width={} max_width={} font={:?} shaping={:?} fg=rgba({:.3},{:.3},{:.3},{:.3}) pos=({}, {})",
        row_index,
        run.start_col,
        run.text,
        run.text.chars().count(),
        run.width_cols,
        draw_width,
        max_width,
        run.font,
        run.shaping,
        run.fg.r,
        run.fg.g,
        run.fg.b,
        run.fg.a,
        x,
        y
    );
}

fn debug_text_layer_draw_run(
    text: &str,
    row_index: usize,
    col_index: usize,
    width_cols: usize,
    x: f32,
    y: f32,
    bounds_width: f32,
    font: Font,
    shaping: iced::widget::text::Shaping,
    color: Color,
) {
    if !render_debug_enabled() || text.is_ascii() {
        return;
    }
    eprintln!(
        "boo_render text_layer_run row={} col={} text={:?} chars={} width_cols={} bounds_width={} font={:?} shaping={:?} fg=rgba({:.3},{:.3},{:.3},{:.3}) pos=({}, {})",
        row_index,
        col_index,
        text,
        text.chars().count(),
        width_cols,
        bounds_width,
        font,
        shaping,
        color.r,
        color.g,
        color.b,
        color.a,
        x,
        y
    );
}

fn terminal_text_shaping() -> iced::widget::text::Shaping {
    iced::widget::text::Shaping::Advanced
}

impl TerminalCanvas {
    fn row_cache_chunk_size(&self) -> usize {
        match self.snapshot.rows as usize {
            0..=48 => 4,
            49..=96 => 8,
            _ => 12,
        }
    }

    fn base_fingerprint(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.snapshot.cols.hash(&mut hasher);
        self.snapshot.rows.hash(&mut hasher);
        self.snapshot_generation.hash(&mut hasher);
        self.appearance_revision.hash(&mut hasher);
        self.snapshot.colors.background.r.hash(&mut hasher);
        self.snapshot.colors.background.g.hash(&mut hasher);
        self.snapshot.colors.background.b.hash(&mut hasher);
        self.background_opacity.to_bits().hash(&mut hasher);
        self.paint_base.hash(&mut hasher);
        self.hash_viewport(&mut hasher);
        hasher.finish()
    }

    fn row_fingerprint(&self, row_index: usize) -> u64 {
        let row_style_fingerprint = self.row_style_fingerprint();
        if let Some(revision) = self.snapshot.row_revisions.get(row_index).copied() {
            return row_style_fingerprint
                ^ revision.rotate_left(17)
                ^ (row_index as u64).rotate_left(33);
        }

        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        row_index.hash(&mut hasher);
        row_style_fingerprint.hash(&mut hasher);
        if let Some(row) = self.snapshot.rows_data.get(row_index) {
            for cell in row {
                cell.text.hash(&mut hasher);
                cell.display_width.hash(&mut hasher);
                cell.fg.r.hash(&mut hasher);
                cell.fg.g.hash(&mut hasher);
                cell.fg.b.hash(&mut hasher);
                cell.bg.r.hash(&mut hasher);
                cell.bg.g.hash(&mut hasher);
                cell.bg.b.hash(&mut hasher);
                cell.bg_is_default.hash(&mut hasher);
                cell.bold.hash(&mut hasher);
                cell.italic.hash(&mut hasher);
                cell.underline.hash(&mut hasher);
                cell.hyperlink.hash(&mut hasher);
            }
        }
        hasher.finish()
    }

    fn row_style_fingerprint(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.snapshot.cols.hash(&mut hasher);
        self.snapshot_generation.hash(&mut hasher);
        self.font_size.to_bits().hash(&mut hasher);
        self.background_opacity.to_bits().hash(&mut hasher);
        self.background_opacity_cells.hash(&mut hasher);
        self.font_families.hash(&mut hasher);
        self.appearance_revision.hash(&mut hasher);
        hash_optional_color(self.url_color, &mut hasher);
        self.snapshot.colors.background.r.hash(&mut hasher);
        self.snapshot.colors.background.g.hash(&mut hasher);
        self.snapshot.colors.background.b.hash(&mut hasher);
        self.hash_viewport(&mut hasher);
        hasher.finish()
    }

    fn row_revision_value(&self, row_index: usize) -> u64 {
        self.snapshot
            .row_revisions
            .get(row_index)
            .copied()
            .unwrap_or_else(|| self.row_fingerprint(row_index))
    }

    #[cfg(test)]
    fn overlay_fingerprint(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.selection_overlay_fingerprint().hash(&mut hasher);
        self.cursor_overlay_fingerprint().hash(&mut hasher);
        hasher.finish()
    }

    fn selection_overlay_fingerprint(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.snapshot_generation.hash(&mut hasher);
        self.selection_rects.len().hash(&mut hasher);
        self.selection_color.r.to_bits().hash(&mut hasher);
        self.selection_color.g.to_bits().hash(&mut hasher);
        self.selection_color.b.to_bits().hash(&mut hasher);
        self.selection_color.a.to_bits().hash(&mut hasher);
        hash_optional_color(self.selection_foreground, &mut hasher);
        self.selection_spans_fingerprint().hash(&mut hasher);
        self.hash_viewport(&mut hasher);
        hasher.finish()
    }

    fn cursor_overlay_fingerprint(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.snapshot_generation.hash(&mut hasher);
        self.snapshot.cursor.visible.hash(&mut hasher);
        self.snapshot.cursor.x.hash(&mut hasher);
        self.snapshot.cursor.y.hash(&mut hasher);
        self.snapshot.cursor.style.hash(&mut hasher);
        self.cursor_blink_visible.hash(&mut hasher);
        self.snapshot.colors.cursor_has_value.hash(&mut hasher);
        self.snapshot.colors.cursor.r.hash(&mut hasher);
        self.snapshot.colors.cursor.g.hash(&mut hasher);
        self.snapshot.colors.cursor.b.hash(&mut hasher);
        hash_optional_color(self.cursor_text_color, &mut hasher);
        self.preedit_text.hash(&mut hasher);
        self.hash_viewport(&mut hasher);
        hasher.finish()
    }

    fn selection_spans_fingerprint(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.snapshot.rows.hash(&mut hasher);
        self.snapshot.cols.hash(&mut hasher);
        self.cell_width.to_bits().hash(&mut hasher);
        self.cell_height.to_bits().hash(&mut hasher);
        self.selection_rects.len().hash(&mut hasher);
        for rect in &self.selection_rects {
            rect.x.to_bits().hash(&mut hasher);
            rect.y.to_bits().hash(&mut hasher);
            rect.width.to_bits().hash(&mut hasher);
            rect.height.to_bits().hash(&mut hasher);
        }
        hasher.finish()
    }

    fn viewport_origin(&self) -> Point {
        self.viewport
            .map(|viewport| Point::new(viewport.x, viewport.y))
            .unwrap_or(Point::new(0.0, 0.0))
    }

    fn viewport_origin_and_size(&self, fallback: Size) -> (Point, Size) {
        self.viewport
            .map(|viewport| {
                (
                    Point::new(viewport.x, viewport.y),
                    Size::new(viewport.width, viewport.height),
                )
            })
            .unwrap_or((Point::new(0.0, 0.0), fallback))
    }

    fn hash_viewport(&self, hasher: &mut std::collections::hash_map::DefaultHasher) {
        if let Some(viewport) = self.viewport {
            viewport.x.to_bits().hash(hasher);
            viewport.y.to_bits().hash(hasher);
            viewport.width.to_bits().hash(hasher);
            viewport.height.to_bits().hash(hasher);
        }
    }

    #[cfg(test)]
    fn fingerprint(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.base_fingerprint().hash(&mut hasher);
        self.overlay_fingerprint().hash(&mut hasher);
        for row_index in 0..self.snapshot.rows_data.len() {
            self.row_fingerprint(row_index).hash(&mut hasher);
        }
        hasher.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_canvas(revision: u64) -> TerminalCanvas {
        TerminalCanvas::new(
            Arc::new(vt_backend_core::TerminalSnapshot::default()),
            8.0,
            16.0,
            14.0,
            Arc::from(["CodeNewRoman Nerd Font Mono", "Fallback One", "Fallback Two"]),
            1,
            revision,
            0.8,
            true,
            true,
            Vec::new(),
            Color::from_rgba(0.65, 0.72, 0.95, 0.35),
            Some(Color::WHITE),
            Some(Color::BLACK),
            Some(Color::from_rgb(0.2, 0.6, 1.0)),
            None,
        )
    }

    #[test]
    fn fingerprint_changes_when_appearance_revision_changes() {
        let before = sample_canvas(1).fingerprint();
        let after = sample_canvas(2).fingerprint();
        assert_ne!(before, after);
    }

    #[test]
    fn fingerprint_changes_when_preedit_changes() {
        let before = sample_canvas(1).fingerprint();
        let mut after = sample_canvas(1);
        after.preedit_text = Some("k".to_string());
        assert_ne!(before, after.fingerprint());
    }

    #[test]
    fn fingerprint_changes_when_snapshot_generation_changes() {
        let before = sample_canvas(1).fingerprint();
        let mut after = sample_canvas(1);
        after.snapshot_generation = 2;
        assert_ne!(before, after.fingerprint());
    }

    #[test]
    fn overlay_fingerprint_changes_when_cursor_style_changes() {
        let before = sample_canvas(1).overlay_fingerprint();
        let mut after = sample_canvas(1);
        let mut snapshot = (*after.snapshot).clone();
        snapshot.cursor.style = 3;
        after.snapshot = Arc::new(snapshot);
        assert_ne!(before, after.overlay_fingerprint());
    }

    #[test]
    fn overlay_fingerprint_changes_when_cursor_blink_visibility_changes() {
        let before = sample_canvas(1).overlay_fingerprint();
        let mut after = sample_canvas(1);
        after.cursor_blink_visible = false;
        assert_ne!(before, after.overlay_fingerprint());
    }

    #[test]
    fn overlay_fingerprint_changes_when_selection_foreground_changes() {
        let before = sample_canvas(1).overlay_fingerprint();
        let mut after = sample_canvas(1);
        after.selection_foreground = Some(Color::BLACK);
        assert_ne!(before, after.overlay_fingerprint());
    }

    #[test]
    fn overlay_fingerprint_changes_when_cursor_text_color_changes() {
        let before = sample_canvas(1).overlay_fingerprint();
        let mut after = sample_canvas(1);
        after.cursor_text_color = Some(Color::WHITE);
        assert_ne!(before, after.overlay_fingerprint());
    }

    #[test]
    fn row_fingerprint_ignores_global_content_revision() {
        let mut before = sample_canvas(1);
        before.snapshot = Arc::new(vt_backend_core::TerminalSnapshot {
            cols: 2,
            rows: 1,
            rows_data: vec![vec![
                vt_backend_core::CellSnapshot {
                    text: "a".to_string(),
                    ..Default::default()
                },
                vt_backend_core::CellSnapshot {
                    text: "b".to_string(),
                    ..Default::default()
                },
            ]],
            ..Default::default()
        });
        let mut after = TerminalCanvas::new(
            Arc::clone(&before.snapshot),
            before.cell_width,
            before.cell_height,
            before.font_size,
            before.font_families.clone(),
            before.snapshot_generation,
            before.appearance_revision,
            before.background_opacity,
            before.background_opacity_cells,
            before.cursor_blink_visible,
            before.selection_rects.clone(),
            before.selection_color,
            before.selection_foreground,
            before.cursor_text_color,
            before.url_color,
            before.preedit_text.clone(),
        );
        assert_eq!(before.row_fingerprint(0), after.row_fingerprint(0));
        after.snapshot = Arc::new(vt_backend_core::TerminalSnapshot {
            rows_data: vec![vec![
                vt_backend_core::CellSnapshot {
                    text: "a".to_string(),
                    ..Default::default()
                },
                vt_backend_core::CellSnapshot {
                    text: "c".to_string(),
                    ..Default::default()
                },
            ]],
            ..(*before.snapshot).clone()
        });
        assert_ne!(before.row_fingerprint(0), after.row_fingerprint(0));
    }

    #[test]
    fn text_runs_coalesce_adjacent_cells_with_same_style() {
        let row = vec![
            vt_backend_core::CellSnapshot {
                text: "a".to_string(),
                ..Default::default()
            },
            vt_backend_core::CellSnapshot {
                text: "b".to_string(),
                ..Default::default()
            },
            vt_backend_core::CellSnapshot {
                text: "c".to_string(),
                bold: true,
                ..Default::default()
            },
        ];
        let runs = build_text_runs(&row, row.len(), &[], None);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].text, "ab");
        assert_eq!(runs[0].start_col, 0);
        assert_eq!(runs[0].width_cols, 2);
        assert_eq!(runs[1].text, "c");
        assert_eq!(runs[1].start_col, 2);
    }

    #[test]
    fn text_runs_skip_plain_spaces_but_keep_underlined_spaces() {
        let row = vec![
            vt_backend_core::CellSnapshot {
                text: "a".to_string(),
                ..Default::default()
            },
            vt_backend_core::CellSnapshot {
                text: " ".to_string(),
                ..Default::default()
            },
            vt_backend_core::CellSnapshot {
                text: " ".to_string(),
                underline: 1,
                ..Default::default()
            },
        ];
        let runs = build_text_runs(&row, row.len(), &[], None);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].text, "a");
        assert_eq!(runs[0].start_col, 0);
        assert_eq!(runs[1].text, " ");
        assert_eq!(runs[1].start_col, 2);
        assert!(runs[1].underline);
    }

    #[test]
    fn selection_text_runs_coalesce_adjacent_selected_cells() {
        let row = vec![
            vt_backend_core::CellSnapshot {
                text: "a".to_string(),
                ..Default::default()
            },
            vt_backend_core::CellSnapshot {
                text: "b".to_string(),
                ..Default::default()
            },
            vt_backend_core::CellSnapshot {
                text: "c".to_string(),
                bold: true,
                ..Default::default()
            },
        ];
        let runs = build_selection_text_runs(
            &row,
            row.len(),
            &[],
            Color::WHITE,
            |col_index, _display_width| col_index < 2,
        );
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].text, "ab");
        assert_eq!(runs[0].start_col, 0);
        assert_eq!(runs[0].width_cols, 2);
    }

    #[test]
    fn text_runs_use_advanced_shaping_for_all_terminal_text() {
        let row = vec![
            vt_backend_core::CellSnapshot {
                text: "a".to_string(),
                ..Default::default()
            },
            vt_backend_core::CellSnapshot {
                text: "🙂".to_string(),
                display_width: 2,
                ..Default::default()
            },
        ];
        let runs = build_text_runs(&row, row.len(), &[], None);
        assert!(!runs.is_empty());
        assert!(runs
            .iter()
            .all(|run| run.shaping == iced::widget::text::Shaping::Advanced));
    }

    #[test]
    fn selection_col_spans_merge_overlapping_rects() {
        let spans = selection_col_spans_for_row(
            &[
                TerminalSelectionRect {
                    x: 8.0,
                    y: 0.0,
                    width: 16.0,
                    height: 16.0,
                },
                TerminalSelectionRect {
                    x: 24.0,
                    y: 0.0,
                    width: 16.0,
                    height: 16.0,
                },
            ],
            0,
            8.0,
            16.0,
            10,
        );
        assert_eq!(
            spans,
            vec![SelectionColSpan {
                start_col: 1,
                end_col: 5
            }]
        );
    }

    #[test]
    fn cell_selection_uses_row_spans() {
        let spans = vec![SelectionColSpan {
            start_col: 2,
            end_col: 4,
        }];
        assert!(cell_is_selected_in_spans(&spans, 2, 1));
        assert!(cell_is_selected_in_spans(&spans, 3, 1));
        assert!(!cell_is_selected_in_spans(&spans, 4, 1));
        assert!(cell_is_selected_in_spans(&spans, 1, 2));
    }

    #[test]
    fn selection_row_range_tracks_covered_rows() {
        let range = selection_row_range(
            &[TerminalSelectionRect {
                x: 0.0,
                y: 8.0,
                width: 16.0,
                height: 24.0,
            }],
            16.0,
            10,
        );
        assert_eq!(range, Some((0, 2)));
    }

    #[test]
    fn primary_terminal_font_is_first_configured_family() {
        assert_eq!(
            primary_terminal_font(&["Primary", "Fallback One", "Fallback Two"]),
            Some("Primary")
        );
    }

    #[test]
    fn private_use_text_stays_on_primary_font() {
        let font = font_for_terminal_text(
            "\u{f313}",
            false,
            false,
            &["CodeNewRoman Nerd Font Mono", "Fallback CJK", "Fallback Emoji"],
        );
        assert_eq!(font.family, font::Family::Name("CodeNewRoman Nerd Font Mono"));
    }

    #[test]
    fn non_cjk_non_ascii_text_stays_on_primary_font() {
        let font = font_for_terminal_text(
            "é",
            false,
            false,
            &["CodeNewRoman Nerd Font Mono", "Fallback One", "Fallback Two"],
        );
        assert_eq!(font.family, font::Family::Name("CodeNewRoman Nerd Font Mono"));
    }

    #[test]
    fn non_ascii_text_is_not_clipped_to_cell_width() {
        assert_eq!(non_ascii_text_max_width("a", 10.0), 10.0);
        assert!(non_ascii_text_max_width("仮", 10.0).is_infinite());
    }

    #[test]
    fn background_spans_coalesce_adjacent_non_default_cells() {
        let highlight = crate::vt::GhosttyColorRgb {
            r: 10,
            g: 20,
            b: 30,
        };
        let row = vec![
            vt_backend_core::CellSnapshot {
                bg: highlight,
                bg_is_default: false,
                ..Default::default()
            },
            vt_backend_core::CellSnapshot {
                bg: highlight,
                bg_is_default: false,
                ..Default::default()
            },
            vt_backend_core::CellSnapshot::default(),
        ];
        let spans = build_background_spans(&row, row.len(), 0.8, false);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].start_col, 0);
        assert_eq!(spans[0].width_cols, 2);
    }
}
