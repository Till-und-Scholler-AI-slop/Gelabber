//! Drawing pieces the voice parts share: the speaking ring and its glow
//! (index.css `.gel-avatar.is-speaking`), avatars with it, the mute marks
//! (components/VoiceStateIcons.tsx), dashed seats and the overlay pills
//! on pictures.

use egui::epaint::{CircleShape, Shadow};
use egui::{Color32, Painter, Pos2, Rect, Stroke, StrokeKind, Ui, Vec2, pos2};

use crate::icons::{self, Icon};
use crate::images::Images;
use crate::screens::shell::common;
use crate::theme::{Theme, corner};

/// Video background and the pills over pictures (theme-independent).
pub const VIDEO_BG: Color32 = Color32::from_rgb(0x0b, 0x0d, 0x0e);
/// `rgba(10,12,13,.62)` and its hover `.82`.
pub fn pill(alpha: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(10, 12, 13, (alpha * 255.0).round() as u8)
}
/// The mute marks drawn on pictures.
pub const MARK_ON_PICTURE: Color32 = Color32::from_rgb(0xf3, 0x9b, 0x98);

/// `lr-speaking-glow`: opacity .45 → 1 → .45 over 1.2s, ease-in-out.
pub fn glow_opacity(ui: &Ui) -> f32 {
    let reduced = crate::screens::settings::prefs::get(ui.ctx()).reduced_motion;
    if reduced {
        return 0.45;
    }
    let t = ui.input(|i| i.time) as f32;
    let phase = (t / 1.2).fract();
    // Half a cycle up, half down, each eased.
    let x = if phase < 0.5 {
        phase * 2.0
    } else {
        2.0 - phase * 2.0
    };
    let eased = x * x * (3.0 - 2.0 * x);
    ui.ctx().request_repaint();
    0.45 + 0.55 * eased
}

/// The ring and glow around a speaking avatar circle in `rect`: a 2px
/// ring, then a 4px ring at 30 % and an 8px blur spread 4 at 45 %, both
/// breathing. Drawn before the avatar, which covers their inner edges.
pub fn speaking_ring(ui: &Ui, painter: &Painter, rect: Rect, own_ring: bool) {
    let theme = Theme::of(ui.ctx());
    let speaking = theme.c.speaking;
    let r = rect.width() / 2.0;
    let opacity = glow_opacity(ui);
    // CSS blurs with a Gaussian of σ = blur / 2; egui's blur is a linear
    // ramp, which spreads as far at about 14px for the web's 8px.
    let glow = Shadow {
        offset: [0, 0],
        blur: 14,
        spread: 4,
        color: speaking.gamma_multiply(0.45 * opacity),
    };
    painter.add(glow.as_shape(rect, corner(r)));
    // A circle's stroke lies outside its radius. Both rings start a pixel
    // under the avatar: from its very edge they would leave a dark seam.
    painter.add(CircleShape::stroke(
        rect.center(),
        r - 1.0,
        Stroke::new(5.0, speaking.gamma_multiply(0.30 * opacity)),
    ));
    if own_ring {
        painter.circle_stroke(rect.center(), r - 1.0, Stroke::new(3.0, speaking));
    }
}

/// The edge of a speaking picture (`.voice-video-tile.is-speaking`): an
/// inset 22px glow spread 6 at 50 %, breathing, under a 3px inset ring.
/// No avatar shows on a picture, so this edge is its speaking cue.
pub fn picture_speaking_edge(ui: &Ui, painter: &Painter, rect: Rect, radius: f32) {
    let speaking = Theme::of(ui.ctx()).c.speaking;
    let t = glow_opacity(ui);
    for i in 0..6 {
        let a = 0.5 * t * (1.0 - i as f32 / 6.0) * 0.35;
        painter.rect_stroke(
            rect.shrink(3.0 + i as f32 * 3.0),
            corner((radius - 3.0 - i as f32 * 3.0).max(0.0)),
            Stroke::new(3.0, speaking.gamma_multiply(a)),
            StrokeKind::Inside,
        );
    }
    painter.rect_stroke(
        rect,
        corner(radius),
        Stroke::new(3.0, speaking),
        StrokeKind::Inside,
    );
}

/// An avatar circle: picture or initials (`font` px) on the person's
/// tint, with the speaking ring when `speaking`.
#[allow(clippy::too_many_arguments)]
pub fn avatar(
    ui: &Ui,
    images: &Images,
    rect: Rect,
    name: &str,
    url: Option<&str>,
    font: f32,
    under: Color32,
    speaking: bool,
) {
    let painter = ui.painter().clone();
    if speaking {
        speaking_ring(ui, &painter, rect, true);
    }
    common::avatar(ui, images, rect, name, url, font, 1.0, under);
}

/// `VoiceStateIcons` in a row from `min`: slot one the mic-off mark, or
/// the green speaker when `speaker`; slot two the headset-off mark. 14px
/// slots, gap 2. Returns the right edge.
pub fn state_icons(
    painter: &Painter,
    theme: &Theme,
    min: Pos2,
    muted: bool,
    deafened: bool,
    speaker: bool,
    color: Color32,
) -> f32 {
    let off = muted || deafened;
    if off {
        icons::paint(painter, Icon::MicOff, min, 14.0, color);
    } else if speaker {
        icons::paint(painter, Icon::Speaker, min, 14.0, theme.c.online);
    }
    let second = pos2(min.x + 16.0, min.y);
    if deafened {
        icons::paint(painter, Icon::HeadsetOff, second, 14.0, color);
    }
    second.x + 14.0
}

/// A dashed circle (`border: 1.5px dashed`), as Chromium dashes a round
/// border: dashes about twice the width long, evenly spread.
pub fn dashed_circle(painter: &Painter, center: Pos2, radius: f32, width: f32, color: Color32) {
    let r = radius - width / 2.0;
    let length = std::f32::consts::TAU * r;
    let dash = width * 2.0;
    let count = ((length / (dash * 2.0)).round() as usize).max(4);
    let step = std::f32::consts::TAU / count as f32;
    let on = step * 0.5;
    for i in 0..count {
        let start = i as f32 * step - std::f32::consts::FRAC_PI_2;
        let points: Vec<Pos2> = (0..=6)
            .map(|k| {
                let a = start + on * k as f32 / 6.0;
                center + Vec2::new(a.cos(), a.sin()) * r
            })
            .collect();
        painter.add(egui::Shape::line(points, Stroke::new(width, color)));
    }
}
