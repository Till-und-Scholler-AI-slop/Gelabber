//! `.ui-badge` tags and the `count` pill (components/ui/Badge.tsx).

use egui::{Color32, Painter, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2, Widget};

use crate::text::{self, Block};
use crate::theme::{Theme, corner, over};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BadgeTone {
    #[default]
    Neutral,
    Accent,
    Success,
    Warning,
    Danger,
}

/// An uppercase caps tag: min-height 20, padding 0 6, radius xs.
#[must_use]
pub struct Badge<'a> {
    label: &'a str,
    tone: BadgeTone,
    /// The surface under the translucent tones.
    on: Option<Color32>,
}

impl<'a> Badge<'a> {
    pub fn new(label: &'a str) -> Self {
        Self {
            label,
            tone: BadgeTone::Neutral,
            on: None,
        }
    }

    pub fn tone(mut self, tone: BadgeTone) -> Self {
        self.tone = tone;
        self
    }

    pub fn on(mut self, surface: Color32) -> Self {
        self.on = Some(surface);
        self
    }
}

impl Widget for Badge<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = Theme::of(ui.ctx());
        let c = &theme.c;
        let under = self.on.unwrap_or(c.bg);
        let (fill, color) = match self.tone {
            BadgeTone::Neutral => (c.surface, c.muted),
            BadgeTone::Accent => (over(under, c.accent, 0.16), c.accent_text),
            BadgeTone::Success => (over(under, c.online, 0.14), c.online_text),
            BadgeTone::Warning => (over(under, c.warning, 0.16), c.text),
            BadgeTone::Danger => (over(under, c.danger_bg, 0.18), c.danger),
        };
        let label = self.label.to_uppercase();
        let block = Block::single(ui, &label, text::CAPS, color);
        let size = Vec2::new(block.width() + 12.0, 20.0);
        let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
        if ui.is_rect_visible(rect) {
            ui.painter()
                .rect_filled(rect, corner(theme.radius.xs), fill);
            block.paint(
                ui.painter(),
                Pos2::new(rect.left() + 6.0, rect.center().y - block.height() / 2.0),
            );
        }
        response
    }
}

/// The unread count pill in the accent, as unread marks are: min 18×18,
/// padding 0 5, 700 11px/1, `99+` above 99, with an optional `0 0 0 2px`
/// ring in the surface colour.
pub fn count_badge(
    painter: &Painter,
    ui: &Ui,
    anchor_bottom_right: Pos2,
    count: u32,
    ring: Option<Color32>,
) -> Rect {
    let theme = Theme::of(ui.ctx());
    let c = &theme.c;
    let label = if count > 99 {
        "99+".to_owned()
    } else {
        count.to_string()
    };
    let style = text::TextStyle::new(11.0, 700.0, 1.0);
    let block = Block::single(ui, &label, style, c.accent_ink);
    let width = (block.width() + 10.0).max(18.0);
    let rect = Rect::from_min_max(
        anchor_bottom_right - Vec2::new(width, 18.0),
        anchor_bottom_right,
    );
    if let Some(ring) = ring {
        painter.rect_stroke(
            rect.expand(1.0),
            corner(10.0),
            Stroke::new(2.0, ring),
            StrokeKind::Middle,
        );
    }
    painter.rect_filled(rect, corner(9.0), c.accent);
    block.paint(
        painter,
        Pos2::new(
            rect.center().x - block.width() / 2.0,
            rect.center().y - block.height() / 2.0,
        ),
    );
    rect
}
