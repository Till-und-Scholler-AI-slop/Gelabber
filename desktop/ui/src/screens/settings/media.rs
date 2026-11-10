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

#[derive(Clone, Default)]
pub struct State {
    recording: bool,
    advanced_open: bool,
    /// The custom limit as typed.
    limit_text: Option<String>,
}

const LABEL_W: f32 = 136.0;
const COL_GAP: f32 = 16.0;
/// `.media-hint`.
const HINT: TextStyle = text::META;

/// `AUDIO_PROCESSING_DESKTOP`: mode, label and hint as the desktop app
/// words them, since its native core filters, not a browser.
pub const PROCESSING_MODES: [(&str, &str, &str); 3] = [
    (
        "enhanced",
        "Verbesserte Sprache",
        "Lokale Rauschunterdrückung",
    ),
    (
        "browser",
        "WebRTC-Filter",
        "Rauschunterdrückung und Auto-Gain von WebRTC",
    ),
    (
        "original",
        "Original / Musik",
        "Stereo sofern unterstützt, ohne Rauschfilter, Auto-Gain oder Sprechpausen-Erkennung",
    ),
];

fn dialog_id() -> Id {
    Id::unique("gelabber-media-form-in-dialog")
}

/// The forms inside the Audio & Video dialog: fields on the page colour,
/// buttons and chips without the settings page's accent text.
/// The dialog draws over the page's own form: its controls get ids of
/// their own.
pub fn set_in_dialog(ctx: &egui::Context, on: bool) {
    ctx.data_mut(|d| d.insert_temp(dialog_id(), on));
    controls::set_id_scope(ctx, on.then_some("media-dialog"));
}

fn recording_id() -> Id {
    Id::unique("gelabber-ptt-recording")
}

/// A push-to-talk key is being recorded (the recorder drew last frame): the
/// mouse's side buttons are its to take, not the window's back and forward.
pub fn recording_key(ctx: &egui::Context) -> bool {
    let drawn: Option<u64> = ctx.data(|d| d.get_temp(recording_id()));
    drawn.is_some_and(|f| f + 1 >= ctx.cumulative_frame_nr())
}

fn in_dialog(ui: &Ui) -> bool {
    ui.ctx().data(|d| d.get_temp(dialog_id())).unwrap_or(false)
}

/// The fill of a select (`--lr-field` differs in a modal).
fn field_fill(ui: &Ui) -> Color32 {
    let theme = Theme::of(ui.ctx());
    if in_dialog(ui) {
        theme.c.bg
    } else {
        theme.c.surface
    }
}

/// Device choices: the system default, then what the call lists.
fn devices(list: &[gelabber_call::Device]) -> Vec<(String, String)> {
    crate::screens::voice::quick::device_options(list)
}

/// `SourceAudioApp` lists on mount and on focus, and the device lists
/// follow the system: the call lists both again when `form` opens and when
/// the window comes back to the front.
fn refresh_lists(ui: &Ui, app: &AppCtx, form: &str) {
    let ctx = ui.ctx();
    let id = Id::unique(("gelabber-media-lists", form));
    let frame = ctx.cumulative_frame_nr();
    let focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));
    let last: Option<(u64, bool)> = ctx.data(|d| d.get_temp(id));
    let opened = last.is_none_or(|(f, _)| f + 1 < frame);
    let refocused = last.is_some_and(|(_, was)| !was) && focused;
    ctx.data_mut(|d| d.insert_temp(id, (frame, focused)));
    if opened || refocused {
        app.call.refresh_devices();
    }
}

/// A chosen device for the call: `None` is the system default.
fn chosen(id: &str) -> Option<String> {
    (!id.is_empty()).then(|| id.to_owned())
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
        // A quiet chip: the accent is for actions.
        let block = Block::single(ui, b, HINT, c.muted);
        let rect = Rect::from_min_size(
            pos2(top.x + width - block.width() - 16.0, top.y + 6.0),
            vec2(block.width() + 16.0, 20.8),
        );
        ui.painter()
            .rect_filled(rect, corner(theme.radius.sm), c.surface);
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
        crate::widgets::rule_h(ui.painter(), c.border, top.x..=top.x + width, bottom);
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
    // `size="sm"` padding (12px) at the form's 36px min-height.
    let b = Button::new(label)
        .width(button_width(ui, label))
        .disabled(disabled);
    // Outside the settings page a `size="sm"` button keeps its 32px.
    let b = if in_dialog(ui) {
        b.height(32.0)
    } else {
        b.text_color(theme.c.accent_text)
    };
    let b = if ghost { b.ghost() } else { b };
    ui.add(b).clicked()
}

fn button_width(ui: &Ui, label: &str) -> f32 {
    Block::single(ui, label, text::UI, Color32::WHITE).width() + 2.0 * 12.0 + 2.0
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
    let fill = field_fill(ui);
    controls::select(ui, id.to_owned(), options, value, fill)
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

pub fn audio(ui: &mut Ui, app: &mut AppCtx, s: &mut State) {
    let ctx = ui.ctx().clone();
    let theme = Theme::of(&ctx);
    let c = &theme.c;
    let mut p = prefs::get(&ctx);
    let before = p.clone();
    refresh_lists(ui, app, "audio");
    let listed = app.call.with_state(|s| s.devices.clone());
    controls::column(ui, ui.available_width().min(720.0), |ui| {
        group_head(ui, "Eingabe", None);
        setting_row(ui, "Mikrofon", false, |ui| {
            if controls::select(
                ui,
                "audio-input",
                &devices(&listed.inputs),
                &mut p.audio_input_id,
                field_fill(ui),
            ) {
                app.call.set_input(chosen(&p.audio_input_id));
            }
            // Next to the microphone, in view when the dialog opens.
            gap(ui);
            if button(ui, "Mikrofon testen", false, false) {
                crate::screens::voice::dialogs::open_mic_test(ui.ctx());
            }
        });
        setting_row(ui, "Mikrofonverstärkung", false, |ui| {
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
                Some("100 % fügt keine Verstärkung hinzu."),
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
                // The settings page makes its chips 36px; elsewhere they stay 32.
                if in_dialog(ui) { 32.0 } else { 36.0 },
                (!in_dialog(ui)).then_some(c.accent_text),
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
                if s.recording {
                    let frame = ui.ctx().cumulative_frame_nr();
                    ui.ctx().data_mut(|d| d.insert_temp(recording_id(), frame));
                }
                if s.recording
                    && let Some((code, label)) = record_key(ui.ctx(), s)
                {
                    p.ptt_key = code;
                    p.ptt_key_label = label;
                }
                gap(ui);
                let note = if p.ptt_key.is_empty() {
                    "Ohne Taste bleibt dein Mikrofon im Gespräch stumm.".to_owned()
                } else {
                    let in_call = app.call.with_state(|s| {
                        matches!(
                            s.status,
                            gelabber_call::CallStatus::Connecting
                                | gelabber_call::CallStatus::Connected
                                | gelabber_call::CallStatus::Reconnecting
                        )
                    });
                    global_key_note(
                        crate::platform::ptt_status(ui.ctx()).as_ref(),
                        in_call,
                        cfg!(windows),
                    )
                };
                hint(ui, &note);
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
            for (mode, label, desc) in PROCESSING_MODES {
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
            let processing = crate::screens::voice::diagnostics::processing_message(
                &app.call.state(),
                &p.processing_mode,
                &p.audio_input_id,
            );
            hint(ui, &processing);
        });
        ui.add_space(20.0);

        group_head(ui, "Ausgabe", None);
        setting_row(ui, "Lautsprecher", false, |ui| {
            if controls::select(
                ui,
                "audio-output",
                &devices(&listed.outputs),
                &mut p.audio_output_id,
                field_fill(ui),
            ) {
                app.call.set_output(chosen(&p.audio_output_id));
            }
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
                Some("Über 100 % wird der Ton digital verstärkt."),
                false,
            ) {
                p.output_volume = playback_volume(pos);
            }
        });
        setting_row(ui, "Live-Ton", false, |ui| {
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
                "Live-Ton stummschalten",
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
                Some(
                    "Nutzt die Wiedergabelautstärke. Signaltöne spielen auf dem Standard-Ausgabegerät des Systems.",
                ),
                off,
            );
            gap(ui);
            let silent = off || p.call_sound_volume == 0.0 || p.output_volume == 0.0;
            // `VoiceSettings.tsx`: the test tone is the own join's cue.
            if controls::row(ui, 0.0, 36.0, |ui| {
                button(ui, "Testton abspielen", false, silent)
            }) {
                app.call.play_sound(gelabber_call::Sound::JoinSelf);
            }
        });
        ui.add_space(20.0);
        disclosure(ui, s, &mut p);
    });
    if p != before {
        prefs::update(&ctx, |x| *x = p);
    }
}

/// `media-disclosure`: "Echo und WebRTC-Filter", closed in settings.
fn disclosure(ui: &mut Ui, s: &mut State, p: &mut Prefs) {
    let theme = Theme::of(ui.ctx());
    let c = &theme.c;
    let width = ui.available_width();
    let top = ui.cursor().min;
    let summary = Rect::from_min_size(top + vec2(1.0, 1.0), vec2(width - 2.0, 44.0));
    let id = controls::scoped(ui.ctx(), Id::unique("gelabber-media-disclosure"));
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
        "Echo und WebRTC-Filter",
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
                    "Rauschunterdrückung von WebRTC.",
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
                    "Auto-Gain von WebRTC. Gleicht leise und laute Mikrofone auf einen hörbaren Pegel an.",
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

/// `HYPRLAND_BIND`, with the app id the portal registers (the Tauri app's
/// was `gelabber-desktop`, which the web client still names).
fn hyprland_bind() -> String {
    format!(
        "bind = , <Taste>, global, {}:push-to-talk",
        crate::platform::APP_ID
    )
}

/// `globalKeyNote`: what the global key does, from the platform binding
/// (`None` while it is not wanted or has not answered yet).
pub fn global_key_note(
    status: Option<&crate::platform::PttStatus>,
    wanted: bool,
    windows: bool,
) -> String {
    use crate::platform::PttStatus;
    let linux = format!(
        "Unter Linux bestimmt der Desktop die Taste: KDE und GNOME fragen beim ersten Mal nach, unter Hyprland gehört „{}“ in die hyprland.conf.",
        hyprland_bind()
    );
    match (wanted, status) {
        (false, _) if windows => {
            "Im Gespräch hört die App die Taste auch, wenn ein anderes Fenster vorne ist.".into()
        }
        (false, _) => format!(
            "Im Gespräch hört die App die Taste auch, wenn ein anderes Fenster vorne ist. {linux}"
        ),
        (true, None) => "Die globale Taste wird eingerichtet …".into(),
        (true, Some(PttStatus::Bound { trigger: Some(t) })) => format!("Global aktiv: {t}."),
        (true, Some(PttStatus::Bound { trigger: None })) if windows => {
            "Global aktiv, auch wenn ein anderes Fenster vorne ist.".into()
        }
        (true, Some(PttStatus::Bound { trigger: None })) => format!("Global aktiv. {linux}"),
        (true, Some(PttStatus::Failed { reason })) => {
            format!("Die Taste wirkt nur, solange Gelabber im Vordergrund ist: {reason}")
        }
    }
}

/// `usePushToTalkKeyName`: the key push-to-talk holds, as the settings
/// name it; `None` while push-to-talk is off or has no key.
pub fn push_to_talk_key_name(ctx: &egui::Context) -> Option<String> {
    let p = super::prefs::get(ctx);
    (p.voice_input == "ptt" && !p.ptt_key.is_empty()).then(|| {
        if p.ptt_key_label.is_empty() {
            key_label(&p.ptt_key)
        } else {
            p.ptt_key_label.clone()
        }
    })
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
            let c = c.replacen("Numpad", "Num ", 1);
            if let Some(side) = c.strip_suffix("Left") {
                format!("{side} links")
            } else if let Some(side) = c.strip_suffix("Right") {
                format!("{side} rechts")
            } else {
                c
            }
        }
    }
}

/// The name of a key that typed nothing as it was pressed. The dead keys
/// of a German keyboard show their mark, as the keycap does.
fn silent_key_label(code: &str) -> String {
    match code {
        "Backquote" => "^".into(),
        "Equal" => "´".into(),
        _ => key_label(code),
    }
}

/// What the recorder makes of a frame's events.
#[derive(Debug, PartialEq)]
enum Recorded {
    Nothing,
    Cancel,
    Key(String, String),
}

/// The next key or side mouse button: the key's place as the global
/// binding knows it (`KeyboardEvent.code`), named by the character it
/// types, as `PushToTalkSettings` does. Esc cancels.
fn recorded(events: &[egui::Event]) -> Recorded {
    for (at, e) in events.iter().enumerate() {
        match e {
            egui::Event::Key {
                key,
                physical_key,
                pressed: true,
                ..
            } => {
                if *key == egui::Key::Escape || *physical_key == Some(egui::Key::Escape) {
                    return Recorded::Cancel;
                }
                let Some(code) = physical_key.and_then(crate::platform::ptt::code_of) else {
                    continue;
                };
                if code == "Enter" || code == "NumpadEnter" || code == "Backspace" {
                    continue;
                }
                // The text the press typed, if it is one visible character.
                let typed = events[at + 1..].iter().find_map(|e| match e {
                    egui::Event::Text(t) => Some(t.as_str()),
                    _ => None,
                });
                let label = match typed {
                    Some(t) if t.chars().count() == 1 && t != " " => t.to_uppercase(),
                    _ => silent_key_label(code),
                };
                return Recorded::Key(code.to_owned(), label);
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
                return Recorded::Key(code.to_owned(), key_label(code));
            }
            _ => {}
        }
    }
    Recorded::Nothing
}

/// [`recorded`] for this frame; recording ends with a key or Esc.
fn record_key(ctx: &egui::Context, s: &mut State) -> Option<(String, String)> {
    match ctx.input(|i| recorded(&i.events)) {
        Recorded::Nothing => None,
        Recorded::Cancel => {
            s.recording = false;
            None
        }
        Recorded::Key(code, label) => {
            s.recording = false;
            Some((code, label))
        }
    }
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

pub fn video(ui: &mut Ui, app: &mut AppCtx, s: &mut State) {
    let ctx = ui.ctx().clone();
    let theme = Theme::of(&ctx);
    let c = &theme.c;
    let mut p = prefs::get(&ctx);
    let before = p.clone();
    refresh_lists(ui, app, "video");
    let listed = app.call.with_state(|s| s.devices.clone());
    controls::column(ui, ui.available_width().min(720.0), |ui| {
        group_head(ui, "Kamera", Some(&summary(&p.camera_profile)));
        setting_row(ui, "Gerät", false, |ui| {
            if controls::select(
                ui,
                "video-input",
                &devices(&listed.cameras),
                &mut p.video_input_id,
                field_fill(ui),
            ) {
                app.call.set_camera_device(chosen(&p.video_input_id));
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
        setting_row(ui, "Ton mitsenden", false, |ui| {
            // `sharesSourceAudio` where "every application" leaves Gelabber
            // out: unchosen means on, so off is a choice to keep.
            let mut on = p.source_audio_share != "off";
            if switch_line(
                ui,
                "share-source-audio",
                "Gilt auch während du live bist. Mitgesendet wird der Ton anderer Anwendungen, nie dein Mikrofon oder der Ton des Anrufs. Der Live-Ton wird als Stereo-Musik übertragen, ohne Mikrofonfilter.",
                &mut on,
                false,
            ) {
                p.source_audio_share = if on { "on" } else { "off" }.into();
            }
        });
        // `SourceAudioApp`: whose sound a Live carries; a saved application
        // that is quiet now stays selectable.
        setting_row(ui, "Ton von", true, |ui| {
            let apps = &listed.apps;
            let saved = &p.source_audio_app;
            let listed_id = |id: &str| apps.iter().any(|a| a.id == id);
            // Apps before v0.6 saved the name as the id; the list shows the app.
            let mut chosen = if listed_id(saved) {
                saved.clone()
            } else {
                apps.iter()
                    .find(|a| &a.label == saved)
                    .map_or_else(|| saved.clone(), |a| a.id.clone())
            };
            let quiet = (!chosen.is_empty() && !listed_id(&chosen))
                .then(|| (chosen.clone(), format!("{chosen} (gerade still)")));
            let options: Vec<(String, String)> =
                std::iter::once((String::new(), "Alle Anwendungen außer Gelabber".to_owned()))
                    .chain(quiet)
                    .chain(apps.iter().map(|a| (a.id.clone(), a.label.clone())))
                    .collect();
            let was_open = controls::select_open(ui.ctx(), "source-audio-app");
            if controls::select(
                ui,
                "source-audio-app",
                &options,
                &mut chosen,
                field_fill(ui),
            ) {
                p.source_audio_app = chosen;
            }
            // The list asks again as it opens (`onFocus={load}`).
            if !was_open && controls::select_open(ui.ctx(), "source-audio-app") {
                app.call.refresh_devices();
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
                    "Ausdrückliche Obergrenzen: Sprache 64 kbit/s, Live-Ton 128 kbit/s, Video gemeinsam {}.",
                    mbit(limit.unwrap_or(2_500_000.0))
                )
            } else {
                "Für wenig Upload: Sprache 64 kbit/s, Live-Ton 128 kbit/s, Video gemeinsam 2,5 Mbit/s.".to_owned()
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
                let w = 1.0 + 12.0 + 20.0 + 8.0 + b.width() + 12.0 + 1.0;
                let rect = Rect::from_min_size(pos2(x, top.y), vec2(w, 36.0));
                let id = controls::scoped(ui.ctx(), Id::unique(("gelabber-upload-mode", i)));
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
                        rect.left() + 13.0 + 20.0 + 8.0,
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
                    let field = ("video-upload-limit", controls::id_scope(ui.ctx()));
                    let out = crate::widgets::TextField::new(field, &mut t).show(&mut child);
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
                "Die Desktop-App passt die tatsächliche Bitrate an die Verbindung an. Das Limit ist eine Obergrenze, keine feste Datenrate.",
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
    fn the_global_key_note_follows_the_binding() {
        use crate::platform::PttStatus;
        assert_eq!(
            global_key_note(None, true, false),
            "Die globale Taste wird eingerichtet …"
        );
        let bound = PttStatus::Bound {
            trigger: Some("F13".into()),
        };
        assert_eq!(
            global_key_note(Some(&bound), true, false),
            "Global aktiv: F13."
        );
        let desktop = PttStatus::Bound { trigger: None };
        let note = global_key_note(Some(&desktop), true, false);
        assert!(note.starts_with("Global aktiv. Unter Linux bestimmt der Desktop die Taste:"));
        assert!(note.ends_with("in die hyprland.conf."));
        assert!(note.contains("„bind = , <Taste>, global, gelabber:push-to-talk“"));
        assert_eq!(
            global_key_note(Some(&desktop), true, true),
            "Global aktiv, auch wenn ein anderes Fenster vorne ist."
        );
        let failed = PttStatus::Failed {
            reason: "Kein GlobalShortcuts-Portal".into(),
        };
        assert_eq!(
            global_key_note(Some(&failed), true, false),
            "Die Taste wirkt nur, solange Gelabber im Vordergrund ist: Kein GlobalShortcuts-Portal"
        );
        // Outside a call nothing is bound yet: what the key will do.
        assert!(
            global_key_note(None, false, false)
                .starts_with("Im Gespräch hört die App die Taste auch")
        );
    }

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
        assert_eq!(key_label("ControlLeft"), "Control links");
        assert_eq!(key_label("Numpad0"), "Num 0");
    }

    fn press(logical: egui::Key, physical: Option<egui::Key>) -> egui::Event {
        egui::Event::Key {
            key: logical,
            physical_key: physical,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    /// The key's place is recorded, as the global binding listens by place.
    #[test]
    fn the_recorder_keeps_the_place_and_names_the_character() {
        use egui::Key;
        let key = |code: &str, label: &str| Recorded::Key(code.into(), label.into());
        // QWERTZ: the key labelled Z sits where US keyboards have Y.
        assert_eq!(
            recorded(&[press(Key::Z, Some(Key::Y)), egui::Event::Text("z".into())]),
            key("KeyY", "Z")
        );
        assert_eq!(
            recorded(&[press(Key::ArrowUp, Some(Key::ArrowUp))]),
            key("ArrowUp", "ArrowUp")
        );
        assert_eq!(
            recorded(&[press(Key::Backtick, Some(Key::Backtick))]),
            key("Backquote", "^")
        );
        // On a US keyboard the same place types a character, which names it.
        assert_eq!(
            recorded(&[
                press(Key::Backtick, Some(Key::Backtick)),
                egui::Event::Text("`".into())
            ]),
            key("Backquote", "`")
        );
        assert_eq!(
            recorded(&[press(Key::ShiftLeft, Some(Key::ShiftLeft))]),
            key("ShiftLeft", "Shift links")
        );
        assert_eq!(
            recorded(&[
                press(Key::Space, Some(Key::Space)),
                egui::Event::Text(" ".into())
            ]),
            key("Space", "Leertaste")
        );
        assert_eq!(
            recorded(&[press(Key::Escape, Some(Key::Escape))]),
            Recorded::Cancel
        );
        assert_eq!(
            recorded(&[press(Key::Enter, Some(Key::Enter))]),
            Recorded::Nothing
        );
    }
}
