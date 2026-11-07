//! `1px solid` hairlines (`--lr-border` and the like), the one way the app
//! paints a separator or a panel edge.
//!
//! A hairline fills the pixel row (or column) that starts at its `y` (`x`),
//! as a CSS border fills the box's last or first pixel. It is drawn as a
//! rect snapped to device pixels, as wide as Chromium draws a `1px` border:
//! whole device pixels, rounded down, at least one. So at 125 % or 150 %
//! every line stays one crisp device pixel instead of an antialiased 1.25
//! or 1.5, and lines drawn at different positions keep the same width.

use egui::{Color32, Painter, Rangef, Rect, pos2};

use crate::theme::Theme;

/// A hairline's width in points (CSS `1px`); layouts reserve this much.
pub const HAIRLINE: f32 = 1.0;

/// The width in points that covers whole device pixels.
fn width(ppp: f32) -> f32 {
    (HAIRLINE * ppp).floor().max(1.0) / ppp
}

/// The device-pixel edge nearest to `v`.
fn snap(v: f32, ppp: f32) -> f32 {
    (v * ppp).round() / ppp
}

/// A horizontal `--lr-border` hairline in the row that starts at `y`.
pub fn hairline_h(painter: &Painter, theme: &Theme, x: impl Into<Rangef>, y: f32) {
    rule_h(painter, theme.c.border, x, y);
}

/// A vertical `--lr-border` hairline in the column that starts at `x`.
pub fn hairline_v(painter: &Painter, theme: &Theme, x: f32, y: impl Into<Rangef>) {
    rule_v(painter, theme.c.border, x, y);
}

/// A horizontal hairline in another colour (`--lr-border-strong`, a
/// preview's own border).
pub fn rule_h(painter: &Painter, color: Color32, x: impl Into<Rangef>, y: f32) {
    let ppp = painter.pixels_per_point();
    let x = x.into();
    let top = snap(y, ppp);
    let rect = Rect::from_min_max(
        pos2(snap(x.min, ppp), top),
        pos2(snap(x.max, ppp), top + width(ppp)),
    );
    painter.rect_filled(rect, 0.0, color);
}

/// A vertical hairline in another colour.
pub fn rule_v(painter: &Painter, color: Color32, x: f32, y: impl Into<Rangef>) {
    let ppp = painter.pixels_per_point();
    let y = y.into();
    let left = snap(x, ppp);
    let rect = Rect::from_min_max(
        pos2(left, snap(y.min, ppp)),
        pos2(left + width(ppp), snap(y.max, ppp)),
    );
    painter.rect_filled(rect, 0.0, color);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hairline_is_whole_device_pixels_like_chromium() {
        for (ppp, px) in [(1.0, 1.0), (1.25, 1.0), (1.5, 1.0), (2.0, 2.0), (2.5, 2.0)] {
            assert!(
                (width(ppp) * ppp - px).abs() < 1e-4,
                "{ppp}: {}",
                width(ppp) * ppp
            );
        }
        // The row at 51 pt is the last device row of a 52 pt header.
        assert!((snap(51.0, 1.25) * 1.25 - 64.0).abs() < 1e-4);
        assert!((snap(52.0, 1.25) * 1.25 - 65.0).abs() < 1e-4);
    }
}
