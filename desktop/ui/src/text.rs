//! Text as Chromium sets it: the type scale of index.css, Inter with
//! `wght` and `opsz` (font-optical-sizing: auto) as variation coordinates,
//! CSS line boxes with half-leading and letter-spacing in em.
//!
//! Boxes are CSS boxes: [`Block::paint`] takes the top-left corner of the
//! first line box and puts the baseline where Blink does, so text lands on
//! the same pixel rows as in the browser.

use std::sync::Arc;

use egui::epaint::text::VariationCoords;
use egui::text::{LayoutJob, TextFormat};
use egui::{Align, Color32, FontFamily, FontId, Galley, Painter, Pos2, Rect, Stroke, Ui, Vec2};

use crate::fonts;

/// Inter's vertical metrics (unitsPerEm 2048: ascender 1984, descender 494).
const ASCENT_EM: f32 = 1984.0 / 2048.0;
const DESCENT_EM: f32 = 494.0 / 2048.0;

/// One CSS `font` shorthand plus letter-spacing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextStyle {
    /// `font-size` in px.
    pub size: f32,
    /// `font-weight`, 100–900.
    pub weight: f32,
    /// `line-height` as a multiple of the size.
    pub line_height: f32,
    /// `letter-spacing` in em.
    pub tracking: f32,
    pub italic: bool,
}

/// `--lr-type-caps` (use with `.to_uppercase()`; carries its .06em).
pub const CAPS: TextStyle = TextStyle::new(11.0, 600.0, 1.3).tracking(0.06);
/// `--lr-type-meta`.
pub const META: TextStyle = TextStyle::new(12.0, 400.0, 1.4);
/// `--lr-type-ui`.
pub const UI: TextStyle = TextStyle::new(13.0, 500.0, 1.35);
/// `--lr-type-body`.
pub const BODY: TextStyle = TextStyle::new(14.0, 400.0, 1.5);
/// `--lr-type-title-sm`.
pub const TITLE_SM: TextStyle = TextStyle::new(16.0, 600.0, 1.3);
/// `--lr-type-title`.
pub const TITLE: TextStyle = TextStyle::new(20.0, 600.0, 1.25);
/// `--lr-type-page`.
pub const PAGE: TextStyle = TextStyle::new(28.0, 700.0, 1.15);
/// `--lr-type-hero` at desktop widths (clamp(28px, 3vw, 36px) ≥ 1200px).
pub const HERO: TextStyle = TextStyle::new(36.0, 700.0, 1.1);

impl TextStyle {
    pub const fn new(size: f32, weight: f32, line_height: f32) -> Self {
        Self {
            size,
            weight,
            line_height,
            tracking: 0.0,
            italic: false,
        }
    }

    pub const fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    pub const fn weight(mut self, weight: f32) -> Self {
        self.weight = weight;
        self
    }

    pub const fn line_height(mut self, line_height: f32) -> Self {
        self.line_height = line_height;
        self
    }

    /// `letter-spacing` in em.
    pub const fn tracking(mut self, em: f32) -> Self {
        self.tracking = em;
        self
    }

    pub const fn italic(mut self) -> Self {
        self.italic = true;
        self
    }

    /// The line box height in px.
    pub fn line_px(&self) -> f32 {
        self.size * self.line_height
    }

    /// Letter-spacing in px.
    pub fn tracking_px(&self) -> f32 {
        self.tracking * self.size
    }

    pub fn font_id(&self) -> FontId {
        let family = if self.italic {
            fonts::italic_family()
        } else {
            FontFamily::Proportional
        };
        FontId::new(self.size, family)
    }

    /// Variation coordinates as Chromium sets them: the weight, and the
    /// optical size following the font size within Inter's 14–32 range.
    pub fn coords(&self) -> VariationCoords {
        VariationCoords::new([
            (b"wght", self.weight),
            (b"opsz", self.size.clamp(14.0, 32.0)),
        ])
    }

    pub fn format(&self, color: Color32) -> TextFormat {
        TextFormat {
            font_id: self.font_id(),
            extra_letter_spacing: self.tracking_px(),
            line_height: Some(self.line_px()),
            color,
            coords: self.coords(),
            valign: Align::Center,
            ..Default::default()
        }
    }

    /// Distance from the top of a line box to the baseline, as Blink
    /// computes it: rounded ascent and descent, the half-leading floored.
    pub fn baseline(&self) -> f32 {
        let ascent = (ASCENT_EM * self.size).round();
        let descent = (DESCENT_EM * self.size).round();
        let half_leading = ((self.line_px() - ascent - descent) / 2.0).floor();
        ascent + half_leading
    }
}

/// A one-style job; `wrap` is the available width, `None` for one line.
pub fn job(text: &str, style: TextStyle, color: Color32, wrap: Option<f32>) -> LayoutJob {
    let mut job = LayoutJob::single_section(text.to_owned(), style.format(color));
    job.wrap.max_width = wrap.unwrap_or(f32::INFINITY);
    job
}

/// Several runs in one paragraph (a sentence with a link, a bold name).
#[derive(Clone, Debug)]
pub struct Rich {
    job: LayoutJob,
    /// The paragraph's own style: it sets the line box.
    base: TextStyle,
}

impl Rich {
    pub fn new(base: TextStyle) -> Self {
        Self {
            job: LayoutJob::default(),
            base,
        }
    }

    /// A run in `style`; the line height stays the paragraph's.
    pub fn push(mut self, text: &str, style: TextStyle, color: Color32) -> Self {
        let mut format = style.format(color);
        format.line_height = Some(self.base.line_px());
        self.job.append(text, 0.0, format);
        self
    }

    /// A run with an underline (`text-decoration: underline`).
    pub fn push_underlined(mut self, text: &str, style: TextStyle, color: Color32) -> Self {
        let mut format = style.format(color);
        format.line_height = Some(self.base.line_px());
        format.underline = Stroke::new(underline_thickness(style.size), color);
        self.job.append(text, 0.0, format);
        self
    }

    pub fn wrap(mut self, width: f32) -> Self {
        self.job.wrap.max_width = width;
        self
    }

    pub fn base(&self) -> TextStyle {
        self.base
    }

    pub fn into_job(self) -> LayoutJob {
        self.job
    }

    pub fn layout(self, ui: &Ui) -> Block {
        let base = self.base;
        Block::new(ui, self.job, base)
    }
}

/// Inter's underline thickness (post table: 2048 upm → ~0.07em), at least
/// one pixel, as Chromium draws it.
fn underline_thickness(size: f32) -> f32 {
    (size / 14.0).round().max(1.0)
}

/// Laid-out text with its CSS line box style.
#[derive(Clone)]
pub struct Block {
    pub galley: Arc<Galley>,
    pub style: TextStyle,
    job: LayoutJob,
}

impl Block {
    pub fn new(ui: &Ui, job: LayoutJob, style: TextStyle) -> Self {
        let galley = ui.fonts_mut(|f| f.layout_job(job.clone()));
        Self { galley, style, job }
    }

    pub fn single(ui: &Ui, text: &str, style: TextStyle, color: Color32) -> Self {
        Self::new(ui, job(text, style, color, None), style)
    }

    pub fn wrapped(ui: &Ui, text: &str, style: TextStyle, color: Color32, width: f32) -> Self {
        Self::new(ui, job(text, style, color, Some(width)), style)
    }

    pub fn rows(&self) -> usize {
        self.galley.rows.len().max(1)
    }

    /// The CSS content width: glyph advances plus letter-spacing after every
    /// character, as Blink counts it.
    pub fn width(&self) -> f32 {
        let width = self
            .galley
            .rows
            .iter()
            .map(|r| r.size.x)
            .fold(0.0, f32::max);
        if self.galley.is_empty() {
            0.0
        } else {
            width + self.style.tracking_px()
        }
    }

    /// The CSS height: one line box per row.
    pub fn height(&self) -> f32 {
        self.rows() as f32 * self.style.line_px()
    }

    pub fn size(&self) -> Vec2 {
        Vec2::new(self.width(), self.height())
    }

    /// Paints with the first line box's top-left corner at `top_left`.
    pub fn paint(&self, painter: &Painter, top_left: Pos2) {
        self.paint_tinted(painter, top_left, None);
    }

    /// Paints in one colour, ignoring the runs' own colours.
    pub fn paint_tinted(&self, painter: &Painter, top_left: Pos2, color: Option<Color32>) {
        let ppp = painter.ctx().pixels_per_point();
        // A fractional x goes into the layout as leading space, so glyphs
        // keep their sub-pixel positions instead of being resampled.
        let x_px = top_left.x * ppp;
        let frac = (x_px - x_px.floor()) / ppp;
        let galley = if frac.abs() > 0.01 && !self.job.sections.is_empty() {
            let mut job = self.job.clone();
            job.sections[0].leading_space += frac;
            painter.ctx().fonts_mut(|f| f.layout_job(job))
        } else {
            self.galley.clone()
        };
        let first_baseline = galley
            .rows
            .first()
            .and_then(|r| r.glyphs.first().map(|g| r.pos.y + g.pos.y))
            .unwrap_or(0.0);
        let baseline = ((top_left.y + self.style.baseline()) * ppp).round() / ppp;
        let pos = Pos2::new(x_px.floor() / ppp, baseline - first_baseline);
        match color {
            Some(color) => painter.galley_with_override_text_color(pos, galley, color),
            None => painter.galley(pos, galley, Color32::PLACEHOLDER),
        }
    }

    /// The rect of section `index` in the first row it appears in, relative
    /// to `top_left` as given to [`Self::paint`] (for links inside text).
    pub fn section_rect(&self, top_left: Pos2, index: usize) -> Option<Rect> {
        let section = self.job.sections.get(index)?;
        let text = &self.job.text;
        let range = section.byte_range.start.0..section.byte_range.end.0;
        let start = text.get(..range.start)?.chars().count();
        let len = text.get(range)?.chars().count();
        let a = self.galley.pos_from_cursor(egui::text::CCursor::new(start));
        let b = self
            .galley
            .pos_from_cursor(egui::text::CCursor::new(start + len));
        let line = self.style.line_px();
        let row = (a.center().y / self.galley.rows[0].size.y.max(1.0)).floor();
        let top = top_left.y + row * line;
        let right = if (b.min.y - a.min.y).abs() < 1.0 {
            b.min.x
        } else {
            self.galley.rect.right()
        };
        Some(Rect::from_min_max(
            Pos2::new(top_left.x + a.min.x, top),
            Pos2::new(top_left.x + right, top + line),
        ))
    }
}

/// Allocates a CSS text box in `ui` and paints `text` in it.
pub fn label(ui: &mut Ui, text: &str, style: TextStyle, color: Color32) -> egui::Response {
    let block = Block::wrapped(ui, text, style, color, ui.available_width());
    show(ui, &block)
}

/// Allocates `block` at the cursor (full available width) and paints it.
pub fn show(ui: &mut Ui, block: &Block) -> egui::Response {
    let size = Vec2::new(ui.available_width().max(block.width()), block.height());
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        block.paint(ui.painter(), rect.min);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baselines_match_blink() {
        // Measured in Chromium on the login page (text rect top + ascent).
        assert_eq!(BODY.baseline(), 16.0);
        assert_eq!(UI.baseline(), 13.0);
        assert_eq!(PAGE.baseline(), 26.0);
        assert_eq!(META.size(13.0).baseline(), 14.0);
    }
}
