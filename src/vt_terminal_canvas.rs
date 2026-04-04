#![cfg(any(target_os = "linux", target_os = "macos"))]

use crate::vt_backend_core;
use iced::alignment;
use iced::font;
use iced::mouse;
use iced::widget::canvas::{self, Cache, Frame};
use iced::{Color, Font, Pixels, Point, Rectangle, Renderer, Size, Theme};
use std::cell::RefCell;
use std::hash::{Hash, Hasher};

const PADDING_X: f32 = 4.0;
const PADDING_Y: f32 = 2.0;
#[derive(Debug)]
pub struct TerminalCanvas {
    pub snapshot: vt_backend_core::TerminalSnapshot,
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
        snapshot: vt_backend_core::TerminalSnapshot,
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

    fn draw_terminal(&self, frame: &mut Frame<Renderer>) {
        let default_fg = color_from_rgb(self.snapshot.colors.foreground, 1.0);
        let default_bg = color_from_rgb(self.snapshot.colors.background, self.background_opacity);
        let cursor_bg = if self.snapshot.colors.cursor_has_value {
            color_from_rgb(self.snapshot.colors.cursor, 1.0)
        } else {
            default_fg
        };

        frame.fill_rectangle(Point::ORIGIN, frame.size(), default_bg);

        for rect in &self.selection_rects {
            frame.fill_rectangle(
                Point::new(rect.x + PADDING_X, rect.y + PADDING_Y),
                Size::new(rect.width, rect.height),
                self.selection_color,
            );
        }

        for (row_index, row) in self.snapshot.rows_data.iter().enumerate() {
            let y = PADDING_Y + row_index as f32 * self.cell_height;

            for col_index in 0..self.snapshot.cols as usize {
                let x = PADDING_X + col_index as f32 * self.cell_width;
                let cell = row.get(col_index);
                let is_cursor = self.snapshot.cursor.visible
                    && self.snapshot.cursor.x as usize == col_index
                    && self.snapshot.cursor.y as usize == row_index;

                let bg = if is_cursor {
                    cursor_bg
                } else {
                    cell.map(|cell| {
                        let is_default_bg = cell.bg.r == self.snapshot.colors.background.r
                            && cell.bg.g == self.snapshot.colors.background.g
                            && cell.bg.b == self.snapshot.colors.background.b;
                        color_from_rgb(
                            cell.bg,
                            if self.background_opacity_cells || is_default_bg {
                                self.background_opacity
                            } else {
                                1.0
                            },
                        )
                    })
                    .unwrap_or(default_bg)
                };
                if bg != default_bg {
                    frame.fill_rectangle(
                        Point::new(x, y),
                        Size::new(self.cell_width, self.cell_height),
                        bg,
                    );
                }

                let content = cell
                    .map(|cell| cell.text.as_str())
                    .filter(|text| !text.is_empty() && *text != "\0");

                if let Some(content) = content {
                    let fg = if is_cursor {
                        default_bg
                    } else {
                        cell.map(|cell| color_from_rgb(cell.fg, 1.0)).unwrap_or(default_fg)
                    };
                    let font = cell
                        .map(|cell| font_for_cell(cell, self.font_family))
                        .unwrap_or_else(|| configured_font(self.font_family));

                    let draw_width = cell
                        .map(|cell| self.cell_width * f32::from(cell.display_width.max(1)))
                        .unwrap_or(self.cell_width);
                    frame.fill_text(canvas::Text {
                        content: content.to_string(),
                        position: Point::new(x, y),
                        color: fg,
                        size: Pixels(self.font_size),
                        line_height: iced::widget::text::LineHeight::Absolute(Pixels(self.cell_height)),
                        font,
                        align_x: iced::widget::text::Alignment::Left,
                        align_y: alignment::Vertical::Top,
                        shaping: iced::widget::text::Shaping::Advanced,
                        max_width: draw_width,
                    });

                    if cell.is_some_and(|cell| cell.underline != 0) {
                        let underline_y = y + self.cell_height - 2.0;
                        frame.fill_rectangle(
                            Point::new(x, underline_y),
                            Size::new(draw_width, 1.5),
                            fg,
                        );
                    }
                }
            }
        }

        if self.snapshot.cursor.visible
            && matches!(self.snapshot.cursor.style, 0 | 1 | 3)
            && self.snapshot.cursor.y < self.snapshot.rows
            && self.snapshot.cursor.x < self.snapshot.cols
        {
            let x = PADDING_X + self.snapshot.cursor.x as f32 * self.cell_width;
            let y = PADDING_Y + self.snapshot.cursor.y as f32 * self.cell_height;

            match self.snapshot.cursor.style {
                0 => {
                    frame.fill_rectangle(
                        Point::new(x, y),
                        Size::new(2.0, self.cell_height),
                        cursor_bg,
                    );
                }
                3 => {
                    frame.fill_rectangle(
                        Point::new(x, y + self.cell_height - 2.0),
                        Size::new(self.cell_width, 2.0),
                        cursor_bg,
                    );
                }
                _ => {}
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

#[derive(Debug, Default)]
pub struct TerminalCanvasState {
    cache: Cache,
    fingerprint: RefCell<Option<u64>>,
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
        let fingerprint = self.fingerprint();
        {
            let mut cached = state.fingerprint.borrow_mut();
            if cached.as_ref() != Some(&fingerprint) {
                state.cache.clear();
                *cached = Some(fingerprint);
            }
        }

        vec![state.cache.draw(renderer, bounds.size(), |frame| {
            self.draw_terminal(frame);
        })]
    }
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

impl TerminalCanvas {
    fn fingerprint(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.snapshot.cols.hash(&mut hasher);
        self.snapshot.rows.hash(&mut hasher);
        self.snapshot.cursor.visible.hash(&mut hasher);
        self.snapshot.cursor.x.hash(&mut hasher);
        self.snapshot.cursor.y.hash(&mut hasher);
        self.snapshot.cursor.style.hash(&mut hasher);
        self.snapshot.colors.background.r.hash(&mut hasher);
        self.snapshot.colors.background.g.hash(&mut hasher);
        self.snapshot.colors.background.b.hash(&mut hasher);
        self.snapshot.colors.foreground.r.hash(&mut hasher);
        self.snapshot.colors.foreground.g.hash(&mut hasher);
        self.snapshot.colors.foreground.b.hash(&mut hasher);
        self.font_size.to_bits().hash(&mut hasher);
        self.background_opacity.to_bits().hash(&mut hasher);
        self.background_opacity_cells.hash(&mut hasher);
        self.font_family.hash(&mut hasher);
        self.appearance_revision.hash(&mut hasher);
        self.selection_rects.len().hash(&mut hasher);
        self.preedit_text.hash(&mut hasher);
        for rect in &self.selection_rects {
            rect.x.to_bits().hash(&mut hasher);
            rect.y.to_bits().hash(&mut hasher);
            rect.width.to_bits().hash(&mut hasher);
            rect.height.to_bits().hash(&mut hasher);
        }
        for row in &self.snapshot.rows_data {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_canvas(revision: u64) -> TerminalCanvas {
        TerminalCanvas::new(
            vt_backend_core::TerminalSnapshot::default(),
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
}
