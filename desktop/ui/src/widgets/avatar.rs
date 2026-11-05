//! `Avatar` and `PresenceAvatar` (components/Avatar.tsx, index.css
//! `.gel-avatar`, `.gel-presence-dot`): a circle with the picture, or the
//! initials on the person's tile tint.

use egui::{Color32, Pos2, Rect, Response, Sense, Ui, Vec2, Widget};

use crate::images::{ImageState, Images};
use crate::initials::{initials, person_tone};
use crate::text::{self, Block};
use crate::theme::Theme;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AvatarSize {
    /// 32px, initials 12px.
    #[default]
    Sm,
    /// 36px, 14px.
    Md,
    /// 64px, 20px.
    Lg,
    /// 112px, 32px (profile hero, voice room tiles).
    Hero,
    /// Any other size; initials at 3/8 of it.
    Px(u16),
}

impl AvatarSize {
    pub fn px(self) -> f32 {
        match self {
            Self::Sm => 32.0,
            Self::Md => 36.0,
            Self::Lg => 64.0,
            Self::Hero => 112.0,
            Self::Px(px) => f32::from(px),
        }
    }

    fn font(self) -> f32 {
        match self {
            Self::Sm => 12.0,
            Self::Md => 14.0,
            Self::Lg => 20.0,
            Self::Hero => 32.0,
            Self::Px(px) => (f32::from(px) * 0.375).round(),
        }
    }
}

/// Presence of `PresenceAvatar`: `o` online, `i` away, `x` offline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Presence {
    Online,
    Away,
    Offline,
}

impl Presence {
    /// The wire letter (`o`, `i`, `x`).
    pub fn from_wire(s: &str) -> Self {
        match s {
            "o" => Self::Online,
            "i" => Self::Away,
            _ => Self::Offline,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Online => "Online",
            Self::Away => "Abwesend",
            Self::Offline => "Offline",
        }
    }
}

#[must_use]
pub struct Avatar<'a> {
    name: &'a str,
    url: Option<&'a str>,
    images: Option<&'a Images>,
    size: AvatarSize,
    presence: Option<Presence>,
    /// The colour behind the presence dot's 2px ring (`--lr-bg` by default).
    ring: Option<Color32>,
}

impl<'a> Avatar<'a> {
    pub fn new(name: &'a str) -> Self {
        Self {
            name,
            url: None,
            images: None,
            size: AvatarSize::Sm,
            presence: None,
            ring: None,
        }
    }

    /// The picture; initials while it loads or when it fails.
    pub fn image(mut self, url: Option<&'a str>, images: &'a Images) -> Self {
        self.url = url.filter(|u| !u.is_empty());
        self.images = Some(images);
        self
    }

    pub fn size(mut self, size: AvatarSize) -> Self {
        self.size = size;
        self
    }

    pub fn presence(mut self, presence: Option<Presence>) -> Self {
        self.presence = presence;
        self
    }

    pub fn ring(mut self, color: Color32) -> Self {
        self.ring = Some(color);
        self
    }

    /// Paints into `rect` (a square) without allocating.
    pub fn paint(&self, ui: &Ui, rect: Rect) {
        let theme = Theme::of(ui.ctx());
        let painter = ui.painter();
        let texture = match (self.url, self.images) {
            (Some(url), Some(images)) => match images.get(ui.ctx(), url) {
                ImageState::Ready(texture) => Some(texture),
                _ => None,
            },
            _ => None,
        };
        let radius = rect.width() / 2.0;
        if let Some(texture) = texture {
            let size = texture.size_vec2();
            let uv = crate::images::cover_uv(size, rect.size());
            let shape = egui::epaint::RectShape::filled(
                rect,
                egui::CornerRadius::same(radius.round() as u8),
                Color32::WHITE,
            )
            .with_texture(texture.id(), uv);
            painter.add(shape);
        } else {
            let tint = theme.tile(person_tone(self.name));
            painter.circle_filled(rect.center(), radius, tint);
            let style = text::TextStyle::new(self.size.font(), 600.0, 1.5);
            let block = Block::single(ui, &initials(self.name), style, theme.c.text);
            block.paint(
                painter,
                Pos2::new(
                    rect.center().x - block.width() / 2.0,
                    rect.center().y - block.height() / 2.0,
                ),
            );
        }
        if let Some(presence) = self.presence {
            let dot = (rect.width() * 0.23).max(10.0);
            let dot_rect = Rect::from_min_max(rect.max - Vec2::splat(dot), rect.max);
            let fill = match presence {
                Presence::Online => theme.c.online,
                Presence::Away => theme.c.warning,
                Presence::Offline => theme.c.muted,
            };
            painter.circle_filled(
                dot_rect.center(),
                dot / 2.0,
                self.ring.unwrap_or(theme.c.bg),
            );
            painter.circle_filled(dot_rect.center(), dot / 2.0 - 2.0, fill);
        }
    }
}

impl Widget for Avatar<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let (rect, response) = ui.allocate_exact_size(Vec2::splat(self.size.px()), Sense::hover());
        if ui.is_rect_visible(rect) {
            self.paint(ui, rect);
        }
        match self.presence {
            Some(presence) => response.on_hover_text(presence.label()),
            None => response,
        }
    }
}
