//! One participant's playback volume and local mute, for this client
//! only (components/UserVolumeControl.tsx, voice/userVolume.ts), and the
//! playback slider's mapping (voice/settings.ts): 100 % in the middle.

use std::collections::{HashMap, HashSet};

use egui::{Color32, Id, Sense, Ui, UiBuilder, vec2};
use uuid::Uuid;

use crate::screens::settings::controls;
use crate::text::{self, Block, TextStyle};
use crate::theme::Theme;
use crate::widgets::Checkbox;

/// `MAX_PLAYBACK_VOLUME`.
pub const MAX_PLAYBACK: f64 = 4.0;

/// `playbackSliderPosition` (0..200).
pub fn position(volume: f64) -> f64 {
    let v = volume.clamp(0.0, MAX_PLAYBACK);
    if v <= 1.0 {
        (v * 100.0).round()
    } else {
        (100.0 + (v - 1.0) / (MAX_PLAYBACK - 1.0) * 100.0).round()
    }
}

/// `playbackSliderVolume`.
pub fn volume(position: f64) -> f64 {
    let p = position.clamp(0.0, 200.0);
    let v = if p <= 100.0 {
        p / 100.0
    } else {
        1.0 + (p - 100.0) / 100.0 * (MAX_PLAYBACK - 1.0)
    };
    (v * 100.0).round() / 100.0
}

/// A person's level on this client.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Level {
    pub volume: f64,
    pub muted: bool,
}

impl Default for Level {
    fn default() -> Self {
        Self {
            volume: 1.0,
            muted: false,
        }
    }
}

fn store_id() -> Id {
    Id::unique("gelabber-user-volumes")
}

pub fn level(ctx: &egui::Context, user: Uuid) -> Level {
    ctx.data(|d| {
        d.get_temp::<HashMap<Uuid, Level>>(store_id())
            .and_then(|m| m.get(&user).copied())
    })
    .unwrap_or_default()
}

/// Keeps `level` and hands the call what it plays at.
pub fn set(ctx: &egui::Context, call: &gelabber_call::Call, user: Uuid, level: Level) {
    ctx.data_mut(|d| {
        d.get_temp_mut_or_default::<HashMap<Uuid, Level>>(store_id())
            .insert(user, level);
    });
    let played = if level.muted { 0.0 } else { level.volume };
    call.set_volume(user, played as f32);
}

/// Hands the call the kept level of everyone who (re)appears in it: the
/// levels last for the session, a new call or consumer starts at 100 %.
pub fn sync(ctx: &egui::Context, call: &gelabber_call::Call) {
    let seen_id = Id::unique("gelabber-user-volumes-seen");
    let present: HashSet<Uuid> = call.with_state(|s| {
        if super::session::joined(s).is_none() {
            return HashSet::new();
        }
        s.participants
            .iter()
            .filter(|p| !p.me)
            .map(|p| p.user)
            .collect()
    });
    let before: HashSet<Uuid> = ctx.data(|d| d.get_temp(seen_id)).unwrap_or_default();
    if present == before {
        return;
    }
    let kept: HashMap<Uuid, Level> = ctx.data(|d| d.get_temp(store_id())).unwrap_or_default();
    for (user, played) in to_apply(&before, &present, &kept) {
        call.set_volume(user, played);
    }
    ctx.data_mut(|d| d.insert_temp(seen_id, present));
}

/// The kept levels of the people new in `present`, as the call plays them.
fn to_apply(
    before: &HashSet<Uuid>,
    present: &HashSet<Uuid>,
    kept: &HashMap<Uuid, Level>,
) -> Vec<(Uuid, f32)> {
    present
        .difference(before)
        .filter_map(|user| {
            let level = kept.get(user).filter(|l| **l != Level::default())?;
            let played = if level.muted { 0.0 } else { level.volume };
            Some((*user, played as f32))
        })
        .collect()
}

/// `UserVolumeControl`: "Lautstärke" and the percentage, the slider,
/// "Lokal stummschalten" with "Zurücksetzen", and the hint. Grid gap 10.
pub fn control(ui: &mut Ui, call: &gelabber_call::Call, user: Uuid) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let mut lv = level(ui.ctx(), user);
    let before = lv;
    let width = ui.available_width();
    let body = TextStyle::new(14.0, 400.0, 1.5);
    // Row: label and value.
    let (row, _) = ui.allocate_exact_size(vec2(width, 21.0), Sense::hover());
    Block::single(ui, "Lautstärke", body, c.text).paint(ui.painter(), row.min);
    let pct = format!("{} %", (lv.volume * 100.0).round() as i64);
    let value = Block::single(ui, &pct, body, c.muted);
    value.paint(
        ui.painter(),
        egui::pos2(row.right() - value.width(), row.top()),
    );
    ui.add_space(10.0);
    let mut pos = position(lv.volume);
    let slider = egui::Rect::from_min_size(ui.cursor().min, vec2(width, 20.0));
    let changed = controls::place(ui, UiBuilder::new().max_rect(slider), |ui| {
        controls::slider(
            ui,
            ("user-volume", user),
            &mut pos,
            0.0,
            200.0,
            1.0,
            lv.muted,
        )
    });
    ui.allocate_exact_size(vec2(width, 20.0), Sense::hover());
    if changed {
        lv.volume = volume(pos);
    }
    ui.add_space(10.0);
    // Row: local mute and reset.
    let (row, _) = ui.allocate_exact_size(vec2(width, 21.0), Sense::hover());
    let check = egui::Rect::from_min_size(
        egui::pos2(row.left(), row.center().y - 10.0),
        vec2(20.0, 20.0),
    );
    ui.put(check, Checkbox::new(&mut lv.muted));
    let label = Block::single(ui, "Lokal stummschalten", body, c.text);
    label.paint(ui.painter(), egui::pos2(row.left() + 28.0, row.top()));
    let mute_hit = ui.interact(
        egui::Rect::from_min_size(row.min + vec2(28.0, 0.0), vec2(label.width(), 21.0)),
        Id::unique(("user-volume-mute", user)),
        Sense::click(),
    );
    if mute_hit.clicked() {
        lv.muted = !lv.muted;
    }
    let changed_level = lv.volume != 1.0 || lv.muted;
    let reset_color = if changed_level {
        c.accent_text
    } else {
        c.muted
    };
    let reset = Block::single(ui, "Zurücksetzen", text::UI.weight(400.0), reset_color);
    let reset_rect = egui::Rect::from_min_size(
        egui::pos2(row.right() - reset.width(), row.top()),
        vec2(reset.width(), 21.0),
    );
    reset.paint(
        ui.painter(),
        egui::pos2(reset_rect.left(), row.center().y - reset.height() / 2.0),
    );
    let reset_hit = ui.interact(
        reset_rect,
        Id::unique(("user-volume-reset", user)),
        if changed_level {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    reset_hit.widget_info(|| {
        egui::WidgetInfo::labeled(egui::Role::Button, changed_level, "Zurücksetzen")
    });
    if reset_hit.clicked() {
        lv = Level::default();
    }
    ui.add_space(10.0);
    let hint = Block::wrapped(
        ui,
        "Nur für dich hörbar. Stream-Ton bleibt unverändert.",
        text::META,
        c.muted,
        width,
    );
    let (r, _) = ui.allocate_exact_size(vec2(width, hint.height()), Sense::hover());
    hint.paint(ui.painter(), r.min);
    if lv != before {
        set(ui.ctx(), call, user, lv);
    }
}

/// The volume chip on a card (`VolumeChip`): `{pct} %` while not 100 %
/// and not muted; returns its width, painted with its left at `x`.
pub fn chip(ui: &Ui, painter: &egui::Painter, x: f32, center_y: f32, lv: Level) -> Option<f32> {
    if lv.muted || (lv.volume - 1.0).abs() < f64::EPSILON {
        return None;
    }
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let label = Block::single(
        ui,
        &format!("{} %", (lv.volume * 100.0).round() as i64),
        text::META.weight(600.0),
        c.accent_text,
    );
    let rect = egui::Rect::from_min_size(
        egui::pos2(x, center_y - (label.height() + 2.0) / 2.0),
        vec2(label.width() + 12.0, label.height() + 2.0),
    );
    painter.rect_filled(rect, crate::theme::corner(theme.radius.full), c.selected);
    label.paint(painter, egui::pos2(x + 6.0, rect.top() + 1.0));
    Some(rect.width())
}

/// The locally muted mark (`SpeakerOffIcon 16`).
pub fn muted_mark(painter: &egui::Painter, x: f32, center_y: f32, color: Color32) -> f32 {
    crate::icons::paint(
        painter,
        crate::icons::Icon::SpeakerOff,
        egui::pos2(x, center_y - 8.0),
        16.0,
        color,
    );
    16.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playback_scale_keeps_100_in_the_middle() {
        assert_eq!(position(1.0), 100.0);
        assert_eq!(position(4.0), 200.0);
        assert_eq!(position(0.5), 50.0);
        assert_eq!(volume(100.0), 1.0);
        assert_eq!(volume(150.0), 2.5);
        assert_eq!(volume(200.0), 4.0);
    }

    #[test]
    fn kept_levels_reach_whoever_comes_back() {
        let (jonas, mira) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let kept: HashMap<Uuid, Level> = [
            (
                jonas,
                Level {
                    volume: 0.5,
                    muted: false,
                },
            ),
            (
                mira,
                Level {
                    volume: 1.0,
                    muted: true,
                },
            ),
        ]
        .into();
        let none = HashSet::new();
        let both: HashSet<Uuid> = [jonas, mira].into();
        let mut applied = to_apply(&none, &both, &kept);
        applied.sort_by_key(|(u, _)| *u);
        assert_eq!(applied, [(jonas, 0.5), (mira, 0.0)]);
        // Nobody new: nothing is sent again.
        assert!(to_apply(&both, &both, &kept).is_empty());
        // Untouched levels stay the call's own.
        let other = Uuid::from_u128(3);
        assert!(to_apply(&none, &[other].into(), &kept).is_empty());
    }
}
