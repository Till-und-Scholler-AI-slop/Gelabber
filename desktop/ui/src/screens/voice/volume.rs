//! One participant's playback volume and local mute, for this client
//! only (components/UserVolumeControl.tsx, voice/userVolume.ts), and the
//! playback slider's mapping (voice/settings.ts): 100 % in the middle.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use egui::{Color32, Id, Sense, Ui, UiBuilder, vec2};
use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::screens::settings::{controls, prefs};
use crate::text::{self, Block, TextStyle};
use crate::theme::Theme;
use crate::widgets::Checkbox;

/// `MAX_PLAYBACK_VOLUME`.
pub const MAX_PLAYBACK: f64 = 4.0;
/// `SAVE_DELAY_MS`: a slider drag is written once it rests.
const SAVE_DELAY: Duration = Duration::from_millis(400);
const FILE: &str = "user-volumes.json";

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

/// The levels of the signed-in account (`gelabber.user-volumes:<account>`):
/// only people away from 100 % and not muted are kept.
#[derive(Clone, Default)]
struct Store {
    account: Option<Uuid>,
    levels: HashMap<Uuid, Level>,
    save_at: Option<Instant>,
}

fn store_id() -> Id {
    Id::unique("gelabber-user-volumes")
}

fn with_store<R>(ctx: &egui::Context, f: impl FnOnce(&mut Store) -> R) -> R {
    ctx.data_mut(|d| f(d.get_temp_mut_or_default::<Store>(store_id())))
}

pub fn level(ctx: &egui::Context, user: Uuid) -> Level {
    ctx.data(|d| {
        d.get_temp::<Store>(store_id())
            .and_then(|s| s.levels.get(&user).copied())
    })
    .unwrap_or_default()
}

/// Keeps `level` for the account and hands it to the call.
pub fn set(ctx: &egui::Context, call: &gelabber_call::Call, user: Uuid, level: Level) {
    with_store(ctx, |s| {
        if level == Level::default() {
            s.levels.remove(&user);
        } else {
            s.levels.insert(user, level);
        }
        s.save_at = Some(Instant::now() + SAVE_DELAY);
    });
    ctx.request_repaint_after(SAVE_DELAY);
    call.set_volume(user, level.volume as f32);
    call.set_user_muted(user, level.muted);
}

/// Once per frame: the account's kept levels reach the call when someone
/// signs in, and a change is written once it rests.
pub fn sync(ctx: &egui::Context, call: &gelabber_call::Call, account: Option<Uuid>) {
    let (switched, write) = with_store(ctx, |s| {
        let due = s
            .save_at
            .is_some_and(|at| at <= Instant::now() || s.account != account);
        let write = due.then(|| (s.account, s.levels.clone()));
        if due {
            s.save_at = None;
        }
        let switched = s.account != account;
        if switched {
            s.account = account;
            s.levels = account.map(load).unwrap_or_default();
        }
        (switched.then(|| s.levels.clone()), write)
    });
    if let Some((Some(owner), levels)) = write {
        save(owner, levels);
    }
    if let Some(levels) = switched {
        call.set_volumes(
            levels
                .into_iter()
                .map(|(user, l)| {
                    let v = gelabber_call::UserVolume {
                        volume: l.volume,
                        muted: l.muted,
                    };
                    (user, v)
                })
                .collect(),
        );
    }
}

fn read_file() -> Map<String, Value> {
    prefs::config_file(FILE)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

/// The account's levels, sanitized like `sanitize` of userVolume.ts.
fn load(account: Uuid) -> HashMap<Uuid, Level> {
    parse(
        read_file()
            .get(&account.to_string())
            .unwrap_or(&Value::Null),
    )
}

fn parse(saved: &Value) -> HashMap<Uuid, Level> {
    let Some(entries) = saved.as_object() else {
        return HashMap::new();
    };
    entries
        .iter()
        .filter_map(|(id, v)| {
            let user = Uuid::parse_str(id).ok()?;
            let v = v.as_object()?;
            let level = Level {
                volume: v
                    .get("volume")
                    .and_then(Value::as_f64)
                    .filter(|x| x.is_finite())
                    .unwrap_or(1.0)
                    .clamp(0.0, MAX_PLAYBACK),
                muted: v.get("muted").and_then(Value::as_bool) == Some(true),
            };
            (level != Level::default()).then_some((user, level))
        })
        .collect()
}

fn save(account: Uuid, levels: HashMap<Uuid, Level>) {
    let Some(path) = prefs::config_file(FILE) else {
        return;
    };
    // Off the UI thread; a failed write only loses the levels on restart.
    std::thread::spawn(move || {
        let mut all = read_file();
        let entries: Map<String, Value> = levels
            .iter()
            .map(|(u, l)| {
                (
                    u.to_string(),
                    json!({ "volume": l.volume, "muted": l.muted }),
                )
            })
            .collect();
        if entries.is_empty() {
            all.remove(&account.to_string());
        } else {
            all.insert(account.to_string(), Value::Object(entries));
        }
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = path.with_extension("json.tmp");
        let text = serde_json::to_string_pretty(&Value::Object(all)).unwrap_or_default();
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    });
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
        controls::slider_named(
            ui,
            ("user-volume", user),
            "Lautstärke",
            &mut pos,
            (0.0, 200.0, 1.0),
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
    ui.put(
        check,
        Checkbox::new(&mut lv.muted).label("Lokal stummschalten"),
    );
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
        "Nur für dich hörbar. Live-Ton bleibt unverändert.",
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
    fn saved_levels_are_sanitized() {
        let (jonas, mira, tom) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3));
        let saved = json!({
            jonas.to_string(): { "volume": 9.0 },
            mira.to_string(): { "volume": 1.0, "muted": true },
            tom.to_string(): { "volume": 1.0, "muted": false },
            "nobody": { "volume": 0.5 },
        });
        let levels = parse(&saved);
        assert_eq!(levels.len(), 2);
        assert_eq!(
            levels[&jonas],
            Level {
                volume: 4.0,
                muted: false
            }
        );
        assert_eq!(
            levels[&mira],
            Level {
                volume: 1.0,
                muted: true
            }
        );
    }
}
