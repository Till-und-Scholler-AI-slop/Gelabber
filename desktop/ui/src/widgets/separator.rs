//! 1px `--lr-border` hairlines, drawn on whole pixels like CSS borders.

use egui::{Painter, Stroke};

use crate::theme::Theme;

/// A horizontal hairline filling the pixel row that starts at `y`.
pub fn hairline_h(painter: &Painter, theme: &Theme, x: std::ops::RangeInclusive<f32>, y: f32) {
    let y = y.round() + 0.5;
    painter.hline(x, y, Stroke::new(1.0, theme.c.border));
}

/// A vertical hairline filling the pixel column that starts at `x`.
pub fn hairline_v(painter: &Painter, theme: &Theme, x: f32, y: std::ops::RangeInclusive<f32>) {
    let x = x.round() + 0.5;
    painter.vline(x, y, Stroke::new(1.0, theme.c.border));
}
