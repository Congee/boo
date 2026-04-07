#![cfg(any(target_os = "linux", target_os = "macos"))]

use crate::vt_backend_core;
use iced::alignment;
use iced::font;
use iced::mouse;
use iced::widget::canvas::{self, Cache, Frame};
use iced::{Color, Font, Pixels, Point, Rectangle, Renderer, Size, Theme};
use std::cell::RefCell;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::{Duration, Instant};

const PADDING_X: f32 = 4.0;
const PADDING_Y: f32 = 2.0;
const ROW_CACHE_CHUNK_SIZE: usize = 8;

#[derive(Debug)]
pub struct TerminalCanvas {
    pub snapshot: Arc<vt_backend_core::TerminalSnapshot>,
    pub cell_width: f32,
    pub cell_height: f32,
    pub font_size: f32,
    pub font_family: Option<&'static str>,
    pub appearance_revision: u64,
    pub background_opacity: f32,
    pub background_opacity_cells: bool,
    pub selection_rects: Vec<TerminalSelectionRect>,
    pub selection_color: Color,
    pub preedit_text: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct TerminalSelectionRect {
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
        font_family: Option<&'static str>,
        appearance_revision: u64,
        background_opacity: f32,
        background_opacity_cells: bool,
        selection_rects: Vec<TerminalSelectionRect>,
        selection_color: Color,
        preedit_text: Option<String>,
    ) -> Self {
        Self {
            snapshot,
            cell_width,
            cell_height,
            font_size,
            font_family,
            appearance_revision,
            background_opacity,
            background_opacity_cells,
            selection_rects,
            selection_color,
            preedit_text,
        }
    }

    fn draw_base(&self, frame: &mut Frame<Renderer>) {
        let default_bg = color_from_rgb(self.snapshot.colors.background, self.background_opacity);
        frame.fill_rectangle(Point::ORIGIN, frame.size(), default_bg);
    }

    fn draw_row(&self, frame: &mut Frame<Renderer>, row_index: usize, state: &TerminalCanvasState) {
        let Some(row) = self.snapshot.rows_data.get(row_index) else {
            return;
        };
        let y = PADDING_Y + row_index as f32 * self.cell_height;
        let row_fingerprint = self.row_fingerprint(row_index);
        {
            let mut fingerprints = state.row_artifact_fingerprints.borrow_mut();
            let mut artifacts = state.row_artifacts.borrow_mut();
            if fingerprints[row_index] != row_fingerprint {
                artifacts[row_index] = RowArtifacts {
                    background_spans: build_background_spans(
                        row,
                        self.snapshot.cols as usize,
                        self.snapshot.colors.background,
                        self.background_opacity,
                        self.background_opacity_cells,
                    ),
                    text_runs: build_text_runs(row, self.snapshot.cols as usize, self.font_family),
                };
                fingerprints[row_index] = row_fingerprint;
            }
        }

        let artifacts = state.row_artifacts.borrow();
        let artifacts = &artifacts[row_index];
        for span in &artifacts.background_spans {
            frame.fill_rectangle(
                Point::new(PADDING_X + span.start_col as f32 * self.cell_width, y),
                Size::new(span.width_cols as f32 * self.cell_width, self.cell_height),
                span.color,
            );
        }

        for run in &artifacts.text_runs {
            let x = PADDING_X + run.start_col as f32 * self.cell_width;
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
                shaping: iced::widget::text::Shaping::Advanced,
                max_width: draw_width,
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
        for row_index in start_row..end_row {
            self.draw_row(frame, row_index, state);
        }
    }

    fn draw_overlay(&self, frame: &mut Frame<Renderer>) {
        let default_fg = color_from_rgb(self.snapshot.colors.foreground, 1.0);
        let cursor_bg = if self.snapshot.colors.cursor_has_value {
            color_from_rgb(self.snapshot.colors.cursor, 0.95)
        } else {
            default_fg
        };

        for rect in &self.selection_rects {
            frame.fill_rectangle(
                Point::new(rect.x + PADDING_X, rect.y + PADDING_Y),
                Size::new(rect.width, rect.height),
                self.selection_color,
            );
        }

        if self.snapshot.cursor.visible
            && self.snapshot.cursor.y < self.snapshot.rows
            && self.snapshot.cursor.x < self.snapshot.cols
        {
            let x = PADDING_X + self.snapshot.cursor.x as f32 * self.cell_width;
            let y = PADDING_Y + self.snapshot.cursor.y as f32 * self.cell_height;
            match self.snapshot.cursor.style {
                0 => frame.fill_rectangle(
                    Point::new(x, y),
                    Size::new(2.0, self.cell_height),
                    cursor_bg,
                ),
                3 => frame.fill_rectangle(
                    Point::new(x, y + self.cell_height - 2.0),
                    Size::new(self.cell_width, 2.0),
                    cursor_bg,
                ),
                _ => {
                    frame.fill_rectangle(
                        Point::new(x, y),
                        Size::new(self.cell_width, self.cell_height),
                        Color {
                            a: 0.18,
                            ..cursor_bg
                        },
                    );
                    let thickness = 1.5;
                    frame.fill_rectangle(Point::new(x, y), Size::new(self.cell_width, thickness), cursor_bg);
                    frame.fill_rectangle(
                        Point::new(x, y + self.cell_height - thickness),
                        Size::new(self.cell_width, thickness),
                        cursor_bg,
                    );
                    frame.fill_rectangle(Point::new(x, y), Size::new(thickness, self.cell_height), cursor_bg);
                    frame.fill_rectangle(
                        Point::new(x + self.cell_width - thickness, y),
                        Size::new(thickness, self.cell_height),
                        cursor_bg,
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
            let x = PADDING_X + self.snapshot.cursor.x as f32 * self.cell_width;
            let y = PADDING_Y + self.snapshot.cursor.y as f32 * self.cell_height;
            let width = (preedit.chars().count().max(1) as f32) * self.cell_width;
            let overlay = Color::from_rgba(0.92, 0.82, 0.32, 0.18);
            let underline = Color::from_rgba(0.98, 0.86, 0.35, 0.9);
            frame.fill_rectangle(
                Point::new(x, y),
                Size::new(width, self.cell_height),
                overlay,
            );
            frame.fill_text(canvas::Text {
                content: preedit.to_string(),
                position: Point::new(x, y),
                color: default_fg,
                size: Pixels(self.font_size),
                line_height: iced::widget::text::LineHeight::Absolute(Pixels(self.cell_height)),
                font: configured_font(self.font_family),
                align_x: iced::widget::text::Alignment::Left,
                align_y: alignment::Vertical::Top,
                shaping: iced::widget::text::Shaping::Advanced,
                max_width: width,
            });
            frame.fill_rectangle(
                Point::new(x, y + self.cell_height - 2.0),
                Size::new(width, 1.5),
                underline,
            );
        }
    }
}

pub struct TerminalCanvasState {
    base_cache: Cache,
    base_fingerprint: RefCell<Option<u64>>,
    row_chunk_caches: RefCell<Vec<Cache>>,
    row_chunk_fingerprints: RefCell<Vec<u64>>,
    row_artifact_fingerprints: RefCell<Vec<u64>>,
    row_artifacts: RefCell<Vec<RowArtifacts>>,
    overlay_cache: Cache,
    overlay_fingerprint: RefCell<Option<u64>>,
}

impl Default for TerminalCanvasState {
    fn default() -> Self {
        Self {
            base_cache: Cache::new(),
            base_fingerprint: RefCell::new(None),
            row_chunk_caches: RefCell::new(Vec::new()),
            row_chunk_fingerprints: RefCell::new(Vec::new()),
            row_artifact_fingerprints: RefCell::new(Vec::new()),
            row_artifacts: RefCell::new(Vec::new()),
            overlay_cache: Cache::new(),
            overlay_fingerprint: RefCell::new(None),
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
            let chunk_count = row_count.div_ceil(ROW_CACHE_CHUNK_SIZE);
            let mut row_chunk_caches = state.row_chunk_caches.borrow_mut();
            let mut row_chunk_fingerprints = state.row_chunk_fingerprints.borrow_mut();
            if row_chunk_caches.len() < chunk_count {
                row_chunk_caches.resize_with(chunk_count, Cache::new);
            }
            row_chunk_caches.truncate(chunk_count);
            row_chunk_fingerprints.resize(chunk_count, 0);
            row_chunk_fingerprints.truncate(chunk_count);
            let mut row_artifact_fingerprints = state.row_artifact_fingerprints.borrow_mut();
            let mut row_artifacts = state.row_artifacts.borrow_mut();
            row_artifact_fingerprints.resize(row_count, 0);
            row_artifact_fingerprints.truncate(row_count);
            row_artifacts.resize_with(row_count, RowArtifacts::default);
            row_artifacts.truncate(row_count);
            drop(row_artifact_fingerprints);
            drop(row_artifacts);

            for chunk_index in 0..chunk_count {
                let chunk_fingerprint = self.row_chunk_fingerprint(chunk_index);
                if row_chunk_fingerprints[chunk_index] != chunk_fingerprint {
                    row_chunk_caches[chunk_index].clear();
                    row_chunk_fingerprints[chunk_index] = chunk_fingerprint;
                }
                let start_row = chunk_index * ROW_CACHE_CHUNK_SIZE;
                let end_row = (start_row + ROW_CACHE_CHUNK_SIZE).min(row_count);
                geometries.push(
                    row_chunk_caches[chunk_index].draw(renderer, bounds.size(), |frame| {
                        self.draw_row_chunk(frame, start_row, end_row, state);
                    }),
                );
            }
        }

        let overlay_fingerprint = self.overlay_fingerprint();
        {
            let mut cached = state.overlay_fingerprint.borrow_mut();
            if cached.as_ref() != Some(&overlay_fingerprint) {
                state.overlay_cache.clear();
                *cached = Some(overlay_fingerprint);
            }
        }
        geometries.push(state.overlay_cache.draw(renderer, bounds.size(), |frame| {
            self.draw_overlay(frame);
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
}

#[derive(Debug, Clone, Default)]
struct RowArtifacts {
    background_spans: Vec<BackgroundSpan>,
    text_runs: Vec<TextRun>,
}

fn build_background_spans(
    row: &[vt_backend_core::CellSnapshot],
    cols: usize,
    default_background: crate::vt::GhosttyColorRgb,
    background_opacity: f32,
    background_opacity_cells: bool,
) -> Vec<BackgroundSpan> {
    let default_bg = color_from_rgb(default_background, background_opacity);
    let mut spans = Vec::new();
    let mut current: Option<BackgroundSpan> = None;

    for col_index in 0..cols {
        let bg = row
            .get(col_index)
            .map(|cell| {
                let is_default_bg = cell.bg.r == default_background.r
                    && cell.bg.g == default_background.g
                    && cell.bg.b == default_background.b;
                color_from_rgb(
                    cell.bg,
                    if background_opacity_cells || is_default_bg {
                        background_opacity
                    } else {
                        1.0
                    },
                )
            })
            .unwrap_or(default_bg);

        if bg == default_bg {
            if let Some(span) = current.take() {
                spans.push(span);
            }
            continue;
        }

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
    font_family: Option<&'static str>,
) -> Vec<TextRun> {
    let mut runs = Vec::new();
    let mut current: Option<TextRun> = None;

    for col_index in 0..cols {
        let Some(cell) = row.get(col_index) else {
            if let Some(run) = current.take() {
                runs.push(run);
            }
            continue;
        };

        let content = cell.text.as_str();
        if content.is_empty() || content == "\0" {
            if let Some(run) = current.take() {
                runs.push(run);
            }
            continue;
        }

        let fg = color_from_rgb(cell.fg, 1.0);
        let font = font_for_cell(cell, font_family);
        let underline = cell.underline != 0;
        let width_cols = usize::from(cell.display_width.max(1));

        match current.as_mut() {
            Some(run)
                if run.fg == fg
                    && run.font == font
                    && run.underline == underline
                    && run.start_col + run.width_cols == col_index =>
            {
                run.text.push_str(content);
                run.width_cols += width_cols;
            }
            _ => {
                if let Some(run) = current.take() {
                    runs.push(run);
                }
                current = Some(TextRun {
                    start_col: col_index,
                    width_cols,
                    text: content.to_string(),
                    fg,
                    font,
                    underline,
                });
            }
        }
    }

    if let Some(run) = current {
        runs.push(run);
    }

    runs
}

fn font_for_cell(cell: &vt_backend_core::CellSnapshot, family: Option<&'static str>) -> Font {
    let base = configured_font(family);
    Font {
        family: base.family,
        weight: if cell.bold {
            font::Weight::Bold
        } else {
            font::Weight::Normal
        },
        style: if cell.italic {
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

fn color_from_rgb(color: crate::vt::GhosttyColorRgb, alpha: f32) -> Color {
    Color::from_rgba8(color.r, color.g, color.b, alpha.clamp(0.0, 1.0))
}

fn render_debug_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("BOO_RENDER_DEBUG").is_some())
}

impl TerminalCanvas {
    fn base_fingerprint(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.snapshot.cols.hash(&mut hasher);
        self.snapshot.rows.hash(&mut hasher);
        self.appearance_revision.hash(&mut hasher);
        self.snapshot.colors.background.r.hash(&mut hasher);
        self.snapshot.colors.background.g.hash(&mut hasher);
        self.snapshot.colors.background.b.hash(&mut hasher);
        self.background_opacity.to_bits().hash(&mut hasher);
        hasher.finish()
    }

    fn row_fingerprint(&self, row_index: usize) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        row_index.hash(&mut hasher);
        self.snapshot.cols.hash(&mut hasher);
        self.font_size.to_bits().hash(&mut hasher);
        self.background_opacity.to_bits().hash(&mut hasher);
        self.background_opacity_cells.hash(&mut hasher);
        self.font_family.hash(&mut hasher);
        self.appearance_revision.hash(&mut hasher);
        if let Some(revision) = self.snapshot.row_revisions.get(row_index).copied() {
            revision.hash(&mut hasher);
        } else if let Some(row) = self.snapshot.rows_data.get(row_index) {
            for cell in row {
                cell.text.hash(&mut hasher);
                cell.display_width.hash(&mut hasher);
                cell.fg.r.hash(&mut hasher);
                cell.fg.g.hash(&mut hasher);
                cell.fg.b.hash(&mut hasher);
                cell.bg.r.hash(&mut hasher);
                cell.bg.g.hash(&mut hasher);
                cell.bg.b.hash(&mut hasher);
                cell.bold.hash(&mut hasher);
                cell.italic.hash(&mut hasher);
                cell.underline.hash(&mut hasher);
            }
        }
        hasher.finish()
    }

    fn row_chunk_fingerprint(&self, chunk_index: usize) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        chunk_index.hash(&mut hasher);
        let start_row = chunk_index * ROW_CACHE_CHUNK_SIZE;
        let end_row = (start_row + ROW_CACHE_CHUNK_SIZE).min(self.snapshot.rows_data.len());
        for row_index in start_row..end_row {
            self.row_fingerprint(row_index).hash(&mut hasher);
        }
        hasher.finish()
    }

    fn overlay_fingerprint(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.snapshot.cursor.visible.hash(&mut hasher);
        self.snapshot.cursor.x.hash(&mut hasher);
        self.snapshot.cursor.y.hash(&mut hasher);
        self.snapshot.cursor.style.hash(&mut hasher);
        self.snapshot.colors.cursor_has_value.hash(&mut hasher);
        self.snapshot.colors.cursor.r.hash(&mut hasher);
        self.snapshot.colors.cursor.g.hash(&mut hasher);
        self.snapshot.colors.cursor.b.hash(&mut hasher);
        self.selection_rects.len().hash(&mut hasher);
        self.selection_color.r.to_bits().hash(&mut hasher);
        self.selection_color.g.to_bits().hash(&mut hasher);
        self.selection_color.b.to_bits().hash(&mut hasher);
        self.selection_color.a.to_bits().hash(&mut hasher);
        self.preedit_text.hash(&mut hasher);
        for rect in &self.selection_rects {
            rect.x.to_bits().hash(&mut hasher);
            rect.y.to_bits().hash(&mut hasher);
            rect.width.to_bits().hash(&mut hasher);
            rect.height.to_bits().hash(&mut hasher);
        }
        hasher.finish()
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
            Some("CodeNewRoman Nerd Font Mono"),
            revision,
            0.8,
            true,
            Vec::new(),
            Color::from_rgba(0.65, 0.72, 0.95, 0.35),
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
            before.font_family,
            before.appearance_revision,
            before.background_opacity,
            before.background_opacity_cells,
            before.selection_rects.clone(),
            before.selection_color,
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
        let runs = build_text_runs(&row, row.len(), None);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].text, "ab");
        assert_eq!(runs[0].start_col, 0);
        assert_eq!(runs[0].width_cols, 2);
        assert_eq!(runs[1].text, "c");
        assert_eq!(runs[1].start_col, 2);
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
                ..Default::default()
            },
            vt_backend_core::CellSnapshot {
                bg: highlight,
                ..Default::default()
            },
            vt_backend_core::CellSnapshot::default(),
        ];
        let spans = build_background_spans(
            &row,
            row.len(),
            crate::vt::GhosttyColorRgb::default(),
            0.8,
            false,
        );
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].start_col, 0);
        assert_eq!(spans[0].width_cols, 2);
    }
}
