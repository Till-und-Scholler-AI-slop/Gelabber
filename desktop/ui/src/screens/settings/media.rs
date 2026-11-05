//! "Audio" and "Video" (voice/VoiceSettings.tsx `MediaSettingsForm`,
//! PushToTalkSettings.tsx, settingsLayout.tsx, quality.css and the
//! settings.css overrides). Every value is a device setting in
//! [`super::prefs`]; the device lists show the system default until the
//! native media core reports devices.

use egui::{Color32, Id, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2};

use super::controls;
use super::prefs::{self, Prefs};
use crate::app::AppCtx;
use crate::text::{self, Block, TextStyle};
use crate::theme::{Theme, corner};
use crate::widgets::{Button, Switch, eased};

#[derive(Default)]
pub struct State {
    recording: bool,
    advanced_open: bool,
    revealed: bool,
    /// The custom limit as typed.
    limit_text: Option<String>,
}

const LABEL_W: f32 = 136.0;
const COL_GAP: f32 = 16.0;
/// `.media-hint`.
const HINT: TextStyle = text::META;

/// Device choices: the system default, then what the media core lists.
fn devices(_kind: &str) -> Vec<(String, String)> {
    vec![(String::new(), "Standard".into())]
}

/// `.media-group-head`: the small subtle heading and an optional pill.
fn group_head(ui: &mut Ui, title: &str, badge: Option<&str>) {
    let theme = Theme::of(ui.ctx());
    let c = &theme.c;
    let top = ui.cursor().min;
    let width = ui.available_width();
    let head_h = if badge.is_some() { 26.8 } else { 22.8 };
    let text_top = top.y + if badge.is_some() { 8.0 } else { 6.0 };
    controls::paint(
        ui,
        pos2(top.x, text_top),
        title,
        HINT.weight(600.0),
        c.subtle,
    );
    if let Some(b) = badge {
        let block = Block::single(ui, b, HINT, c.accent_text);
        let rect = Rect::from_min_size(
            pos2(top.x + width - block.width() - 16.0, top.y + 6.0),
            vec2(block.width() + 16.0, 20.8),
        );
        ui.painter()
            .rect_filled(rect, corner(theme.radius.sm), c.selected);
        block.paint(ui.painter(), pos2(rect.left() + 8.0, rect.top() + 2.0));
    }
    ui.allocate_exact_size(vec2(width, head_h), Sense::hover());
}

/// `SettingRow`: the name left (136px), the controls right, a hairline
/// under it unless it is the group's last row.
fn setting_row(ui: &mut Ui, label: &str, last: bool, content: impl FnOnce(&mut Ui)) {
    let theme = Theme::of(ui.ctx());
    let c = &theme.c;
    let top = ui.cursor().min;
    let width = ui.available_width();
    controls::paint(ui, pos2(top.x, top.y + 12.0 + 8.0), label, text::UI, c.text);
    let control = Rect::from_min_size(
        pos2(top.x + LABEL_W + COL_GAP, top.y + 12.0),
        vec2(width - LABEL_W - COL_GAP, f32::INFINITY),
    );
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(control)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    child.spacing_mut().item_spacing = egui::Vec2::ZERO;
    content(&mut child);
    let h = child.min_rect().height().max(25.5);
    let bottom = top.y + 12.0 + h + 12.0;
    if !last {
        ui.painter().hline(
            top.x..=top.x + width,
            bottom + 0.5,
            Stroke::new(1.0, c.border),
        );
    }
    ui.advance_cursor_after_rect(Rect::from_min_max(
        top,
        pos2(top.x + width, bottom + if last { 0.0 } else { 1.0 }),
    ));
}

/// A gap of 8 between the controls of a row (`.media-row-control`).
fn gap(ui: &mut Ui) {
    ui.add_space(8.0);
}

fn hint(ui: &mut Ui, s: &str) {
    let theme = Theme::of(ui.ctx());
    let w = ui.available_width();
    controls::paragraph(ui, s, HINT, theme.c.muted, w);
}

/// `Slider`: range + value, optional scale and hint.
#[allow(clippy::too_many_arguments)]
fn slider(
    ui: &mut Ui,
    id: &str,
    value: &mut f64,
    min: f64,
    max: f64,
    step: f64,
    suffix: &str,
    scale: Option<[&str; 3]>,
    hint_text: Option<&str>,
    disabled: bool,
) -> bool {
    let theme = Theme::of(ui.ctx());
    let c = &theme.c;
    let width = ui.available_width();
    let top = ui.cursor().min;
    let out = Block::single(ui, suffix, text::UI, c.muted);
    let out_w = out.width().max(42.0);
    let range_w = width - 12.0 - out_w;
    let range = Rect::from_min_size(top, vec2(range_w, 20.0));
    let mut changed = false;
    controls::place(ui, UiBuilder::new().max_rect(range), |ui| {
        changed = controls::slider(ui, id, value, min, max, step, disabled);
    });
    out.paint(
        ui.painter(),
        pos2(
            top.x + width - out.width(),
            top.y + 10.0 - text::UI.line_px() / 2.0,
        ),
    );
    let mut h = 20.0;
    if let Some(marks) = scale {
        let y = top.y + 24.0;
        let blocks: Vec<Block> = marks
            .iter()
            .map(|m| Block::single(ui, m, HINT, c.subtle))
            .collect();
        blocks[0].paint(ui.painter(), pos2(top.x, y));
        blocks[1].paint(
            ui.painter(),
            pos2(top.x + (range_w - blocks[1].width()) / 2.0 + 0.4, y),
        );
        blocks[2].paint(ui.painter(), pos2(top.x + range_w - blocks[2].width(), y));
        h += 4.0 + 16.8;
    }
    ui.allocate_exact_size(vec2(width, h), Sense::hover());
    if let Some(t) = hint_text {
        ui.add_space(4.0);
        hint(ui, t);
    }
    // `.media-slider` keeps at least a control's height.
    let used = ui.cursor().min.y - top.y;
    if used < 36.0 {
        ui.add_space(36.0 - used);
    }
    changed
}

/// `SwitchLine`: the explanation left, the switch right.
fn switch_line(ui: &mut Ui, id: &str, text_s: &str, on: &mut bool, disabled: bool) -> bool {
    let theme = Theme::of(ui.ctx());
    let c = &theme.c;
    let width = ui.available_width();
    let block = Block::wrapped(ui, text_s, HINT, c.muted, width - 38.0 - 16.0);
    let h = block.height().max(36.0);
    let top = ui.cursor().min;
    block.paint(
        ui.painter(),
        pos2(top.x, top.y + (h - block.height()) / 2.0),
    );
    let rect = Rect::from_min_size(
        pos2(top.x + width - 38.0, top.y + (h - 22.0) / 2.0),
        vec2(38.0, 22.0),
    );
    let mut value = *on;
    let changed = controls::place(ui, UiBuilder::new().max_rect(rect).id_salt(id), |ui| {
        ui.add(Switch::new(&mut value).disabled(disabled)).changed()
    });
    if changed {
        *on = value;
    }
    ui.allocate_exact_size(vec2(width, h), Sense::hover());
    changed
}

/// `Switch` with its label (`.ui-choice-row--between`, 44px in settings).
fn toggle(ui: &mut Ui, id: &str, label: &str, on: &mut bool, disabled: bool) -> bool {
    let theme = Theme::of(ui.ctx());
    let c = &theme.c;
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(
        vec2(width, 44.0),
        if disabled {
            Sense::hover()
        } else {
            Sense::click()
        },
    );
    let color = if disabled {
        c.text.gamma_multiply(0.6)
    } else {
        c.text
    };
    let b = Block::single(ui, label, text::UI, color);
    b.paint(
        ui.painter(),
        pos2(rect.left(), rect.center().y - b.height() / 2.0),
    );
    let sw = Rect::from_min_size(
        pos2(rect.right() - 38.0, rect.center().y - 11.0),
        vec2(38.0, 22.0),
    );
    let mut value = *on;
    let mut changed = controls::place(ui, UiBuilder::new().max_rect(sw).id_salt(id), |ui| {
        ui.add(Switch::new(&mut value).disabled(disabled)).changed()
    });
    if response.clicked() && !disabled && !changed {
        value = !value;
        changed = true;
    }
    if changed {
        *on = value;
    }
    changed
}

/// A settings-area button: 36px, accent text (settings.css override).
fn button(ui: &mut Ui, label: &str, ghost: bool, disabled: bool) -> bool {
    let theme = Theme::of(ui.ctx());
    let b = Button::new(label)
        .text_color(theme.c.accent_text)
        .disabled(disabled);
    let b = if ghost { b.ghost() } else { b };
    ui.add(b).clicked()
}

/// A field with its small label above (`.media-field`).
fn labelled_select(
    ui: &mut Ui,
    id: &str,
    label: &str,
    options: &[(String, String)],
    value: &mut String,
) -> bool {
    let theme = Theme::of(ui.ctx());
    let c = &theme.c;
    controls::paint(ui, ui.cursor().min, label, HINT, c.subtle);
    ui.add_space(16.8 + 4.0);
    controls::select(ui, id.to_owned(), options, value, c.surface)
}

fn pct(v: f64) -> String {
    format!("{} %", (v * 100.0).round() as i64)
}

const MAX_PLAYBACK: f64 = 4.0;

/// `playbackSliderPosition`: 100 % sits in the middle.
fn playback_position(volume: f64) -> f64 {
    let v = volume.clamp(0.0, MAX_PLAYBACK);
    if v <= 1.0 {
        (v * 100.0).round()
    } else {
        (100.0 + (v - 1.0) / (MAX_PLAYBACK - 1.0) * 100.0).round()
    }
}

/// `playbackSliderVolume`.
fn playback_volume(position: f64) -> f64 {
    let p = position.clamp(0.0, 200.0);
    let v = if p <= 100.0 {
        p / 100.0
    } else {
        1.0 + (p - 100.0) / 100.0 * (MAX_PLAYBACK - 1.0)
    };
    (v * 100.0).round() / 100.0
}

pub fn audio(ui: &mut Ui, _app: &mut AppCtx, s: &mut State) {
    let ctx = ui.ctx().clone();
    let theme = Theme::of(&ctx);
    let c = &theme.c;
    let mut p = prefs::get(&ctx);
    let before = p.clone();
    controls::column(ui, ui.available_width().min(720.0), |ui| {
        group_head(ui, "Eingabe", None);
        setting_row(ui, "Mikrofon", false, |ui| {
            controls::select(
                ui,
                "audio-input",
                &devices("audioinput"),
                &mut p.audio_input_id,
                c.surface,
            );
            if !s.revealed {
                gap(ui);
                reveal_box(ui, s);
            }
        });
        setting_row(ui, "Mic-Gain", false, |ui| {
            let mut v = (p.input_gain * 100.0).round();
            if slider(
                ui,
                "input-gain",
                &mut v,
                0.0,
                200.0,
                1.0,
                &pct(p.input_gain),
                Some(["0 %", "100 %", "200 %"]),
                Some("100 % fügt keine Verstärkung hinzu. Andere Werte laufen über Web Audio."),
                false,
            ) {
                p.input_gain = v / 100.0;
            }
        });
        let modes = ["Sprachaktivierung", "Push-to-Talk"];
        let ptt = p.voice_input == "ptt";
        setting_row(ui, "Senden", false, |ui| {
            if let Some(i) = controls::chips(
                ui,
                "voice-input",
                &modes,
                usize::from(ptt),
                36.0,
                Some(c.accent_text),
            ) {
                p.voice_input = if i == 1 { "ptt" } else { "activity" }.into();
            }
            gap(ui);
            hint(
                ui,
                if p.voice_input == "ptt" {
                    "Das Mikrofon sendet nur, solange du die Taste hältst."
                } else {
                    "Das Mikrofon sendet, sobald du sprichst."
                },
            );
        });
        if p.voice_input == "ptt" {
            setting_row(ui, "Taste", false, |ui| {
                let label = if s.recording {
                    "Taste oder Maustaste drücken … (Esc bricht ab)".to_owned()
                } else if p.ptt_key.is_empty() {
                    "Taste festlegen".to_owned()
                } else if !p.ptt_key_label.is_empty() {
                    p.ptt_key_label.clone()
                } else {
                    key_label(&p.ptt_key)
                };
                if button(ui, &label, false, false) {
                    s.recording = !s.recording;
                }
                if s.recording
                    && let Some((code, label)) = record_key(ui.ctx(), s)
                {
                    p.ptt_key = code;
                    p.ptt_key_label = label;
                }
                gap(ui);
                hint(
                    ui,
                    if p.ptt_key.is_empty() {
                        "Ohne Taste bleibt dein Mikrofon im Gespräch stumm."
                    } else {
                        "Im Gespräch hört die App die Taste auch, wenn ein anderes Fenster vorne ist."
                    },
                );
            });
            setting_row(ui, "Nachlaufzeit", false, |ui| {
                let suffix = format!("{} ms", p.ptt_release_delay.round() as i64);
                slider(
                    ui,
                    "ptt-release-delay",
                    &mut p.ptt_release_delay,
                    0.0,
                    1000.0,
                    50.0,
                    &suffix,
                    None,
                    Some("So lange sendet das Mikrofon nach dem Loslassen weiter."),
                    false,
                );
            });
        }
        setting_row(ui, "Filter", true, |ui| {
            for (mode, label, desc) in [
                (
                    "enhanced",
                    "Verbesserte Sprache",
                    "Lokale Rauschunterdrückung und Auto-Gain; Browserfilter als Ersatz",
                ),
                (
                    "browser",
                    "Browserfilter",
                    "Rauschunterdrückung und Auto-Gain deines Browsers",
                ),
                (
                    "original",
                    "Original / Musik",
                    "Stereo sofern unterstützt, ohne Rauschfilter, Auto-Gain oder Sprechpausen-Erkennung",
                ),
            ] {
                if controls::radio_card(
                    ui,
                    ("processing", mode),
                    label,
                    desc,
                    p.processing_mode == mode,
                ) {
                    p.processing_mode = mode.into();
                }
                ui.add_space(6.0);
            }
            ui.add_space(2.0);
            let top = ui.cursor().min;
            let w = ui.available_width();
            controls::paint(
                ui,
                pos2(top.x, top.y + 9.6),
                "Mikrofon inaktiv",
                HINT,
                c.muted,
            );
            let bw = Block::single(ui, "Mikrofon testen und vergleichen", text::UI, c.text).width()
                + 34.0;
            let rect = Rect::from_min_size(pos2(top.x + w - bw, top.y), vec2(bw, 36.0));
            controls::place(ui, UiBuilder::new().max_rect(rect), |ui| {
                button(ui, "Mikrofon testen und vergleichen", false, false);
            });
            ui.allocate_exact_size(vec2(w, 36.0), Sense::hover());
        });
        ui.add_space(20.0);

        group_head(ui, "Ausgabe", None);
        setting_row(ui, "Lautsprecher", false, |ui| {
            controls::select(
                ui,
                "audio-output",
                &devices("audiooutput"),
                &mut p.audio_output_id,
                c.surface,
            );
        });
        let scale = ["0 %", "100 %", "400 %"];
        setting_row(ui, "Gespräche", false, |ui| {
            let mut pos = playback_position(p.output_volume);
            if slider(
                ui,
                "output-volume",
                &mut pos,
                0.0,
                200.0,
                1.0,
                &pct(p.output_volume),
                Some(scale),
                Some(
                    "Über 100 % wird der Ton digital verstärkt. Nutze dafür Kopfhörer: Über Lautsprecher können die anderen sonst ihr eigenes Echo hören.",
                ),
                false,
            ) {
                p.output_volume = playback_volume(pos);
            }
        });
        setting_row(ui, "Stream-Ton", false, |ui| {
            let mut pos = playback_position(p.source_audio_volume);
            if slider(
                ui,
                "source-audio-volume",
                &mut pos,
                0.0,
                200.0,
                1.0,
                &pct(p.source_audio_volume),
                Some(scale),
                Some(
                    "Der Ton von Lives hat eine eigene Lautstärke. Taub schaltet auch diesen Ton aus.",
                ),
                p.source_audio_muted,
            ) {
                p.source_audio_volume = playback_volume(pos);
            }
            gap(ui);
            toggle(
                ui,
                "source-audio-muted",
                "Stream-Ton stummschalten",
                &mut p.source_audio_muted,
                false,
            );
        });
        setting_row(ui, "Signaltöne", true, |ui| {
            switch_line(
                ui,
                "call-sounds",
                "Töne für Beitreten, Verlassen, Stumm, Taub, Kamera und Live.",
                &mut p.call_sounds,
                false,
            );
            gap(ui);
            let off = !p.call_sounds;
            toggle(
                ui,
                "other-mute-sounds",
                "Töne, wenn andere sich stumm schalten",
                &mut p.other_mute_sounds,
                off,
            );
            gap(ui);
            controls::paint(
                ui,
                ui.cursor().min,
                "Lautstärke der Signaltöne",
                HINT,
                c.subtle,
            );
            ui.add_space(16.8 + 4.0);
            let suffix = pct(p.call_sound_volume);
            slider(
                ui,
                "call-sound-volume",
                &mut p.call_sound_volume,
                0.0,
                1.0,
                0.05,
                &suffix,
                None,
                Some("Nutzt deinen gewählten Lautsprecher und die Wiedergabelautstärke."),
                off,
            );
            gap(ui);
            let silent = off || p.call_sound_volume == 0.0 || p.output_volume == 0.0;
            controls::row(ui, 0.0, 36.0, |ui| {
                button(ui, "Testton abspielen", false, silent)
            });
        });
        ui.add_space(20.0);
        disclosure(ui, s, &mut p);
    });
    if p != before {
        prefs::update(&ctx, |x| *x = p);
    }
}

fn reveal_box(ui: &mut Ui, s: &mut State) {
    let theme = Theme::of(ui.ctx());
    let c = &theme.c;
    let width = ui.available_width();
    let top = ui.cursor().min;
    let label = "Gerätenamen anzeigen";
    let bw = Block::single(ui, label, text::UI, c.text).width() + 34.0;
    let text_w = width - 24.0 - 12.0 - bw;
    let msg = Block::wrapped(
        ui,
        "Die Gerätenamen zeigt der Browser erst, wenn du den Mikrofonzugriff erlaubst.",
        TextStyle::new(12.0, 400.0, 16.0 / 12.0),
        c.muted,
        text_w,
    );
    let h = 20.0 + msg.height().max(36.0);
    let rect = Rect::from_min_size(top, vec2(width, h));
    ui.painter()
        .rect_filled(rect, corner(theme.radius.md), c.surface);
    msg.paint(
        ui.painter(),
        pos2(rect.left() + 12.0, rect.center().y - msg.height() / 2.0),
    );
    let brect = Rect::from_min_size(
        pos2(rect.right() - 12.0 - bw, rect.center().y - 18.0),
        vec2(bw, 36.0),
    );
    controls::place(ui, UiBuilder::new().max_rect(brect), |ui| {
        if button(ui, label, false, false) {
            s.revealed = true;
        }
    });
    ui.allocate_exact_size(vec2(width, h), Sense::hover());
}

/// `media-disclosure`: "Echo und Browserfilter", closed in settings.
fn disclosure(ui: &mut Ui, s: &mut State, p: &mut Prefs) {
    let theme = Theme::of(ui.ctx());
    let c = &theme.c;
    let width = ui.available_width();
    let top = ui.cursor().min;
    let summary = Rect::from_min_size(top + vec2(1.0, 1.0), vec2(width - 2.0, 44.0));
    let id = Id::unique("gelabber-media-disclosure");
    let response = ui.interact(summary, id, Sense::click());
    if response.clicked() {
        s.advanced_open = !s.advanced_open;
    }
    let hover = eased(
        ui.ctx(),
        id,
        response.hovered(),
        Color32::TRANSPARENT,
        c.hover,
    );
    let body_top = summary.bottom();
    let painter = ui.painter().clone();
    let frame_idx = painter.add(egui::Shape::Noop);
    painter.rect_filled(summary, corner(theme.radius.md), hover);
    // The chevron: a 6px box with two borders, rotated.
    let t = ui.ctx().animate_bool_with_time(
        id.with("open"),
        s.advanced_open,
        crate::theme::motion::STANDARD,
    );
    let center = pos2(summary.left() + 14.0 + 4.0, summary.center().y);
    let ang = (-45.0_f32 + 90.0 * t).to_radians();
    let rot = |x: f32, y: f32| {
        center + vec2(x * ang.cos() - y * ang.sin(), x * ang.sin() + y * ang.cos())
    };
    let stroke = Stroke::new(2.0, c.text);
    painter.line_segment([rot(3.0, -3.0), rot(3.0, 3.0)], stroke);
    painter.line_segment([rot(-3.0, 3.0), rot(3.0, 3.0)], stroke);
    controls::paint(
        ui,
        pos2(
            summary.left() + 14.0 + 6.0 + 12.0,
            summary.center().y - text::UI.line_px() / 2.0,
        ),
        "Echo und Browserfilter",
        text::UI.weight(600.0),
        c.text,
    );
    let small = Block::single(ui, "Für Musik oder Lautsprecher", HINT, c.subtle);
    small.paint(
        &painter,
        pos2(
            summary.right() - 14.0 - small.width(),
            summary.center().y - small.height() / 2.0,
        ),
    );
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    ui.allocate_exact_size(vec2(width, 46.0), Sense::hover());
    if s.advanced_open {
        let body = Rect::from_min_size(
            pos2(top.x + 15.0, body_top),
            vec2(width - 30.0, f32::INFINITY),
        );
        let mut child = ui.new_child(
            UiBuilder::new()
                .max_rect(body)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        child.spacing_mut().item_spacing = egui::Vec2::ZERO;
        let browser = p.processing_mode == "browser";
        let original = p.processing_mode == "original";
        setting_row(&mut child, "Echo", !browser && original, |ui| {
            switch_line(
                ui,
                "aec",
                "Echo-Unterdrückung (AEC). Bei Lautsprechern empfohlen. Für Musik mit Kopfhörern kannst du sie ausschalten.",
                &mut p.echo_cancellation,
                false,
            );
        });
        if browser {
            setting_row(&mut child, "Rauschfilter", false, |ui| {
                switch_line(
                    ui,
                    "ns",
                    "Rauschunterdrückung des Browsers.",
                    &mut p.noise_suppression,
                    false,
                );
            });
        }
        if !original {
            setting_row(&mut child, "Auto-Gain", true, |ui| {
                switch_line(
                    ui,
                    "agc",
                    "Auto-Gain des Browsers. Gleicht leise und laute Mikrofone auf einen hörbaren Pegel an.",
                    &mut p.auto_gain_control,
                    false,
                );
            });
        }
        let h = child.min_rect().height();
        ui.allocate_exact_size(vec2(width, h), Sense::hover());
    }
    let frame = Rect::from_min_max(top, pos2(top.x + width, ui.cursor().min.y));
    painter.set(
        frame_idx,
        egui::epaint::RectShape::new(
            frame,
            corner(theme.radius.md),
            Color32::TRANSPARENT,
            Stroke::new(1.0, c.border),
            StrokeKind::Inside,
        ),
    );
}

/// Web `KeyboardEvent.code` names for egui keys, as the PTT core expects.
fn key_code(key: egui::Key) -> Option<String> {
    use egui::Key;
    let name = key.name();
    match key {
        Key::Escape | Key::Enter | Key::Backspace => None,
        Key::Space => Some("Space".into()),
        _ if name.len() == 1 && name.chars().all(|c| c.is_ascii_alphabetic()) => {
            Some(format!("Key{}", name.to_ascii_uppercase()))
        }
        _ if name.len() == 1 && name.chars().all(|c| c.is_ascii_digit()) => {
            Some(format!("Digit{name}"))
        }
        _ => Some(name.replace(' ', "")),
    }
}

/// `pushToTalkKeyLabel`.
fn key_label(code: &str) -> String {
    match code {
        "Mouse4" => "Maustaste 4".into(),
        "Mouse5" => "Maustaste 5".into(),
        "Space" => "Leertaste".into(),
        _ => {
            let c = code
                .strip_prefix("Key")
                .filter(|r| r.len() == 1)
                .or_else(|| code.strip_prefix("Digit").filter(|r| r.len() == 1))
                .unwrap_or(code);
            c.replacen("Numpad", "Num ", 1)
        }
    }
}

/// The next key or side mouse button while recording; Esc cancels.
fn record_key(ctx: &egui::Context, s: &mut State) -> Option<(String, String)> {
    let mut out = None;
    ctx.input(|i| {
        for e in &i.events {
            match e {
                egui::Event::Key {
                    key: egui::Key::Escape,
                    pressed: true,
                    ..
                } => {
                    s.recording = false;
                    return;
                }
                egui::Event::Key {
                    key, pressed: true, ..
                } => {
                    if let Some(code) = key_code(*key) {
                        let label = key_label(&code);
                        out = Some((code, label));
                        return;
                    }
                }
                egui::Event::PointerButton {
                    button,
                    pressed: true,
                    ..
                } => {
                    let code = match button {
                        egui::PointerButton::Extra1 => "Mouse4",
                        egui::PointerButton::Extra2 => "Mouse5",
                        _ => continue,
                    };
                    out = Some((code.to_owned(), key_label(code)));
                    return;
                }
                _ => {}
            }
        }
    });
    if out.is_some() {
        s.recording = false;
    }
    out
}

// ----------------------------------------------------------------- video

const RESOLUTIONS: [u32; 5] = [480, 720, 1080, 1440, 2160];
const FRAME_RATES: [u32; 5] = [15, 24, 30, 45, 60];

/// The height and frame rate of a profile id (`balanced` = 720p30).
fn profile(id: &str) -> (bool, u32, u32) {
    if let Some((h, f)) = id.split_once('p')
        && let (Ok(h), Ok(f)) = (h.parse(), f.parse())
    {
        return (false, h, f);
    }
    match id {
        "detail" => (false, 1080, 30),
        "smooth" => (false, 720, 60),
        _ => (true, 720, 30),
    }
}

/// `profileSummary`.
fn summary(id: &str) -> String {
    let (auto, h, f) = profile(id);
    if auto {
        return "Automatisch".into();
    }
    let res = if h == 2160 {
        "4K".to_owned()
    } else {
        format!("{h}p")
    };
    format!("{res} · {f} FPS")
}

/// `StreamProfileFields`: resolution and frame rate side by side.
fn profile_fields(ui: &mut Ui, name: &str, value: &mut String, screen: bool) -> bool {
    let (auto, h, f) = profile(value);
    let height = if auto && screen { 1080 } else { h };
    let width = ui.available_width();
    let half = (width - 12.0) / 2.0;
    let top = ui.cursor().min;
    let mut res = if auto {
        "auto".to_owned()
    } else {
        height.to_string()
    };
    let mut fps = if auto {
        "auto".to_owned()
    } else {
        f.to_string()
    };
    let res_opts: Vec<(String, String)> =
        std::iter::once(("auto".to_owned(), "Auto (bis 1080p)".to_owned()))
            .chain(RESOLUTIONS.iter().map(|r| {
                (
                    r.to_string(),
                    if *r == 2160 {
                        "4K · 2160p".to_owned()
                    } else {
                        format!("{r}p")
                    },
                )
            }))
            .collect();
    let fps_opts: Vec<(String, String)> = auto
        .then(|| ("auto".to_owned(), "Auto (bis 30 FPS)".to_owned()))
        .into_iter()
        .chain(
            FRAME_RATES
                .iter()
                .map(|r| (r.to_string(), format!("{r} FPS"))),
        )
        .collect();
    let mut changed = false;
    for (i, (label, opts)) in [("Auflösung", &res_opts), ("Bildrate", &fps_opts)]
        .into_iter()
        .enumerate()
    {
        let rect = Rect::from_min_size(
            pos2(top.x + i as f32 * (half + 12.0), top.y),
            vec2(half, 57.8),
        );
        let mut child = ui.new_child(
            UiBuilder::new()
                .max_rect(rect)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        child.spacing_mut().item_spacing = egui::Vec2::ZERO;
        let slot = if i == 0 { &mut res } else { &mut fps };
        if labelled_select(&mut child, &format!("{name}-{i}"), label, opts, slot) {
            changed = true;
        }
    }
    ui.allocate_exact_size(vec2(width, 57.8), Sense::hover());
    if changed {
        let next = if res == "auto" {
            "balanced".to_owned()
        } else {
            let h: u32 = res.parse().unwrap_or(height);
            let f: u32 = if fps == "auto" {
                f
            } else {
                fps.parse().unwrap_or(f)
            };
            format!("{h}p{f}")
        };
        let next = if fps != "auto" && res == "auto" && auto {
            // A frame rate chosen on automatic keeps the 720p default.
            format!("{height}p{fps}")
        } else {
            next
        };
        if *value != next {
            *value = next;
            return true;
        }
    }
    false
}

fn mbit(bits: f64) -> String {
    let rounded = (bits / 1_000_000.0 * 10.0).round() / 10.0;
    if rounded.fract() == 0.0 {
        format!("{} Mbit/s", rounded as i64)
    } else {
        format!("{rounded:.1} Mbit/s").replace('.', ",")
    }
}

pub fn video(ui: &mut Ui, _app: &mut AppCtx, s: &mut State) {
    let ctx = ui.ctx().clone();
    let theme = Theme::of(&ctx);
    let c = &theme.c;
    let mut p = prefs::get(&ctx);
    let before = p.clone();
    controls::column(ui, ui.available_width().min(720.0), |ui| {
        group_head(ui, "Kamera", Some(&summary(&p.camera_profile)));
        setting_row(ui, "Gerät", false, |ui| {
            controls::select(
                ui,
                "video-input",
                &devices("videoinput"),
                &mut p.video_input_id,
                c.surface,
            );
            if !s.revealed {
                gap(ui);
                controls::row(ui, 0.0, 36.0, |ui| {
                    if button(ui, "Kameranamen anzeigen", true, false) {
                        s.revealed = true;
                    }
                });
            }
        });
        setting_row(ui, "Qualität", true, |ui| {
            profile_fields(ui, "camera-stream-profile", &mut p.camera_profile, false);
        });
        ui.add_space(20.0);

        group_head(ui, "Live", Some(&summary(&p.screen_profile)));
        setting_row(ui, "Qualität", false, |ui| {
            profile_fields(ui, "screen-stream-profile", &mut p.screen_profile, true);
            gap(ui);
            hint(
                ui,
                "Höhere Werte brauchen mehr Rechenleistung und Upload. Die erreichbare Qualität hängt von Quelle, Gerät und Verbindung ab.",
            );
        });
        setting_row(ui, "Ton mitsenden", true, |ui| {
            let mut on = p.source_audio_share == "on";
            if switch_line(
                ui,
                "share-source-audio",
                "Gilt ab deinem nächsten Live; Ausschalten wirkt sofort. Wähle den Ton im Browserdialog aus; je nach Browser und Quelle ist nur Video verfügbar. Der Stream-Ton wird als Stereo-Musik übertragen, ohne Mikrofonfilter.",
                &mut on,
                false,
            ) {
                p.source_audio_share = if on { "on" } else { "auto" }.into();
            }
        });
        ui.add_space(20.0);

        let limit = if p.video_upload_limit > 0.0 {
            Some(p.video_upload_limit)
        } else if p.economy_mode {
            Some(2_500_000.0)
        } else {
            None
        };
        let badge = limit.map(mbit).unwrap_or_else(|| "ohne Limit".into());
        group_head(ui, "Upload", Some(&badge));
        setting_row(ui, "Sparmodus", false, |ui| {
            let text_s = if p.economy_mode {
                format!(
                    "Ausdrückliche Obergrenzen: Sprache 64 kbit/s, Stream-Ton 128 kbit/s, Video gemeinsam {}.",
                    mbit(limit.unwrap_or(2_500_000.0))
                )
            } else {
                "Für wenig Upload: Sprache 64 kbit/s, Stream-Ton 128 kbit/s, Video gemeinsam 2,5 Mbit/s.".to_owned()
            };
            switch_line(ui, "economy-mode", &text_s, &mut p.economy_mode, false);
        });
        setting_row(ui, "Video-Limit", true, |ui| {
            let first = if p.economy_mode {
                "Sparmodus (2,5 Mbit/s)"
            } else {
                "Ohne Limit"
            };
            let top = ui.cursor().min;
            let mut x = top.x;
            for (i, label) in [first, "Eigenes Limit"].iter().enumerate() {
                let checked = if i == 0 {
                    p.video_upload_limit == 0.0
                } else {
                    p.video_upload_limit > 0.0
                };
                let b = Block::single(ui, label, text::UI, c.text);
                let w = 1.0 + 12.0 + 20.0 + 8.0 + b.width() + 12.0 + 1.0 + 4.0;
                let rect = Rect::from_min_size(pos2(x, top.y), vec2(w, 36.0));
                let id = Id::unique(("gelabber-upload-mode", i));
                let response = ui.interact(rect, id, Sense::click());
                let painter = ui.painter();
                painter.rect(
                    rect,
                    corner(theme.radius.md),
                    if checked {
                        c.selected
                    } else {
                        Color32::TRANSPARENT
                    },
                    Stroke::new(1.0, if checked { c.accent } else { c.border }),
                    StrokeKind::Inside,
                );
                controls::radio_mark(ui, pos2(rect.left() + 13.0, rect.top() + 8.0), checked);
                b.paint(
                    ui.painter(),
                    pos2(
                        rect.left() + 13.0 + 20.0 + 12.0,
                        rect.center().y - b.height() / 2.0,
                    ),
                );
                if response.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                if response.clicked() && !checked {
                    p.video_upload_limit = if i == 0 { 0.0 } else { 10_000_000.0 };
                    s.limit_text = None;
                }
                x += w + 10.0;
            }
            ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::hover());
            if p.video_upload_limit > 0.0 {
                gap(ui);
                controls::row(ui, 8.0, 37.0, |ui| {
                    let mut t = s
                        .limit_text
                        .clone()
                        .unwrap_or_else(|| format!("{}", p.video_upload_limit / 1_000_000.0));
                    let rect = Rect::from_min_size(ui.cursor().min, vec2(128.0, 37.0));
                    let mut child = ui.new_child(UiBuilder::new().max_rect(rect));
                    let out = crate::widgets::TextField::new("video-upload-limit", &mut t)
                        .show(&mut child);
                    ui.advance_cursor_after_rect(rect);
                    if out.response.changed() {
                        if let Ok(v) = t.replace(',', ".").parse::<f64>()
                            && v > 0.0
                        {
                            p.video_upload_limit = (v.min(4294.967) * 1_000_000.0).round();
                        }
                        s.limit_text = Some(t);
                    }
                    ui.add_space(8.0);
                    controls::paint(
                        ui,
                        pos2(ui.cursor().min.x, rect.center().y - 8.4),
                        "Mbit/s für alle Videos zusammen",
                        HINT,
                        c.muted,
                    );
                });
            }
            gap(ui);
            hint(
                ui,
                "Der Browser passt die tatsächliche Bitrate an die Verbindung an. Das Limit ist eine Obergrenze, keine feste Datenrate.",
            );
        });
        ui.add_space(20.0);
        let w = ui.available_width();
        controls::paragraph(
            ui,
            "Änderungen gelten beim nächsten Start und werden, soweit unterstützt, auch auf laufende Streams angewendet.",
            HINT,
            c.subtle,
            w,
        );
    });
    if p != before {
        prefs::update(&ctx, |x| *x = p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playback_scale_keeps_100_in_the_middle() {
        assert_eq!(playback_position(1.0), 100.0);
        assert_eq!(playback_volume(100.0), 1.0);
        assert_eq!(playback_volume(200.0), 4.0);
        assert_eq!(playback_position(0.5), 50.0);
    }

    #[test]
    fn profiles_and_labels() {
        assert_eq!(summary("balanced"), "Automatisch");
        assert_eq!(summary("2160p60"), "4K · 60 FPS");
        assert_eq!(mbit(2_500_000.0), "2,5 Mbit/s");
        assert_eq!(mbit(10_000_000.0), "10 Mbit/s");
        assert_eq!(key_label("KeyV"), "V");
        assert_eq!(key_code(egui::Key::V).as_deref(), Some("KeyV"));
    }
}
