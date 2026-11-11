//! The gear's quick settings (voice/QuickCallSettings.tsx, room.css
//! `.voice-quick`): microphone, speaker and camera, the two volumes, voice
//! activation or push-to-talk, and the way to all settings. Width 312.

use egui::{Id, Rect, Response, Sense, Ui, UiBuilder, vec2};
use gelabber_call::Device;
use gelabber_i18n::voice_settings as texts;

use crate::app::AppCtx;
use crate::screens::settings::{controls, prefs};
use crate::text::{self, Block};
use crate::theme::Theme;
use crate::widgets::Popover;

/// The device choices: the system default, then what the call lists. The
/// desktop app names the default "Systemstandard" (`deviceChoices`).
pub fn device_options(list: &[Device]) -> Vec<(String, String)> {
    std::iter::once((String::new(), texts::devices_system_default().to_owned()))
        .chain(list.iter().map(|d| (d.id.clone(), d.label.clone())))
        .collect()
}

fn open_id() -> Id {
    Id::unique("gelabber-voice-quick-open")
}

/// Opens and closes the popover from the gear `response` at `anchor`.
pub fn button(ui: &mut Ui, app: &mut AppCtx, response: Response, anchor: Rect) {
    let ctx = ui.ctx().clone();
    let open: Option<Rect> = ctx.data(|d| d.get_temp(open_id())).flatten();
    let same = open.is_some_and(|r| (r.center() - anchor.center()).length() < 1.0);
    if response.clicked() {
        let next = if same { None } else { Some(anchor) };
        ctx.data_mut(|d| d.insert_temp(open_id(), next));
        return;
    }
    if !same {
        return;
    }
    let shown = Popover::new("voice-quick", anchor)
        .align_end()
        .min_width(312.0)
        .show(&ctx, |ui| content(ui, app));
    match shown {
        Some(false) => {}
        Some(true) | None => {
            ctx.data_mut(|d| d.insert_temp(open_id(), None::<Rect>));
        }
    }
}

/// The stream sound button's popover (`CallSound`, width 264): the call's
/// volume and the stream's, with the stream's mute after its percentage.
pub fn sound_button(ui: &mut Ui, response: Response, anchor: Rect, deafened: bool) {
    let ctx = ui.ctx().clone();
    let id = Id::unique("gelabber-voice-sound-open");
    let mut open: bool = ctx.data(|d| d.get_temp(id)).unwrap_or(false);
    if response.clicked() {
        open = !open;
        ctx.data_mut(|d| d.insert_temp(id, open));
        return;
    }
    if !open {
        return;
    }
    let shown = Popover::new("voice-sound", anchor)
        .align_end()
        .min_width(264.0)
        .show(&ctx, |ui| {
            let theme = Theme::of(ui.ctx());
            let c = theme.c;
            let width = 264.0 - 18.0 - 12.0;
            ui.set_width(264.0 - 18.0);
            let mut p = prefs::get(ui.ctx());
            let before = p.clone();
            let rect = Rect::from_min_size(ui.cursor().min + vec2(6.0, 6.0), vec2(width, 400.0));
            let used = controls::place(ui, UiBuilder::new().max_rect(rect), |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let voices = &mut p.output_volume;
                volume(
                    ui,
                    "sound-voices",
                    texts::output_calls(),
                    voices,
                    deafened,
                    width,
                    0.0,
                );
                ui.add_space(14.0);
                let stream_muted = p.source_audio_muted;
                let row_top = ui.cursor().min.y;
                let off = deafened || stream_muted;
                let stream = &mut p.source_audio_volume;
                volume(
                    ui,
                    "sound-stream",
                    texts::output_live_sound(),
                    stream,
                    off,
                    width,
                    38.0,
                );
                // The 30px mute after the percentage, reaching into the gaps.
                let toggle = Rect::from_min_size(
                    egui::pos2(rect.right() - 30.0, row_top + 16.8 / 2.0 - 15.0),
                    vec2(30.0, 30.0),
                );
                let label = if stream_muted {
                    texts::quick_unmute_live_sound()
                } else {
                    texts::output_mute_live_sound()
                };
                let hit = ui.interact(toggle, Id::unique("sound-stream-mute"), Sense::click());
                hit.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::Role::Button,
                        true,
                        texts::output_mute_live_sound(),
                    )
                });
                let hit = hit.on_hover_text(label);
                let (fill, fg) = if stream_muted {
                    (c.danger_bg.gamma_multiply(0.16), c.danger)
                } else if hit.hovered() {
                    (c.hover, c.text)
                } else {
                    (egui::Color32::TRANSPARENT, c.muted)
                };
                ui.painter()
                    .rect_filled(toggle, crate::theme::corner(theme.radius.md), fill);
                crate::icons::paint_centered(
                    ui.painter(),
                    if stream_muted {
                        crate::icons::Icon::SpeakerOff
                    } else {
                        crate::icons::Icon::Speaker
                    },
                    toggle,
                    16.0,
                    fg,
                );
                if hit.clicked() {
                    p.source_audio_muted = !stream_muted;
                }
                if deafened {
                    ui.add_space(14.0 - 6.0);
                    let b = Block::single(ui, texts::quick_deafened(), text::META, c.muted);
                    let (r, _) = ui.allocate_exact_size(vec2(width, b.height()), Sense::hover());
                    b.paint(ui.painter(), r.min);
                }
                ui.min_rect()
            });
            ui.allocate_exact_size(vec2(264.0 - 18.0, used.height() + 8.0), Sense::hover());
            if p != before {
                prefs::update(ui.ctx(), |q| *q = p);
            }
        });
    if shown.is_none() {
        ctx.data_mut(|d| d.insert_temp(id, false));
    }
}

/// The fields; returns true once "Alle Einstellungen …" was pressed.
fn content(ui: &mut Ui, app: &mut AppCtx) -> bool {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let ctx = ui.ctx().clone();
    let width = 312.0 - 18.0 - 12.0;
    ui.set_width(312.0 - 18.0);
    let mut p = prefs::get(&ctx);
    let before = p.clone();
    // The device lists as the system has them now.
    if crate::widgets::popover::opening(&ctx) {
        app.call.refresh_devices();
    }
    let devices = app.call.with_state(|s| s.devices.clone());
    let rect = Rect::from_min_size(ui.cursor().min + vec2(6.0, 6.0), vec2(width, 800.0));
    let mut more = false;
    let used = controls::place(ui, UiBuilder::new().max_rect(rect), |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let label = |ui: &mut Ui, s: &str| {
            let b = Block::single(ui, s, text::META.weight(600.0), c.muted);
            let (r, _) = ui.allocate_exact_size(vec2(width, b.height()), Sense::hover());
            b.paint(ui.painter(), r.min);
            ui.add_space(6.0);
        };
        label(ui, texts::input_microphone());
        if controls::select(
            ui,
            "quick-mic",
            &device_options(&devices.inputs),
            &mut p.audio_input_id,
            c.surface,
        ) {
            app.call
                .set_input(Some(p.audio_input_id.clone()).filter(|s| !s.is_empty()));
        }
        ui.add_space(14.0);
        label(ui, texts::output_speaker());
        if controls::select(
            ui,
            "quick-speaker",
            &device_options(&devices.outputs),
            &mut p.audio_output_id,
            c.surface,
        ) {
            app.call
                .set_output(Some(p.audio_output_id.clone()).filter(|s| !s.is_empty()));
        }
        ui.add_space(14.0);
        label(ui, texts::camera_title());
        if controls::select(
            ui,
            "quick-camera",
            &device_options(&devices.cameras),
            &mut p.video_input_id,
            c.surface,
        ) {
            app.call
                .set_camera_device(Some(p.video_input_id.clone()).filter(|s| !s.is_empty()));
        }
        ui.add_space(14.0);
        volume(
            ui,
            "quick-voices",
            texts::output_calls(),
            &mut p.output_volume,
            false,
            width,
            0.0,
        );
        ui.add_space(14.0);
        let muted = p.source_audio_muted;
        volume(
            ui,
            "quick-stream",
            texts::output_live_sound(),
            &mut p.source_audio_volume,
            muted,
            width,
            0.0,
        );
        ui.add_space(14.0);
        let selected = usize::from(p.voice_input == "ptt");
        if let Some(i) = controls::chips(
            ui,
            "quick-mode",
            &[texts::ptt_activity(), texts::ptt_ptt()],
            selected,
            32.0,
            None,
        ) {
            p.voice_input = if i == 1 { "ptt" } else { "activity" }.into();
        }
        if p.voice_input == "ptt" && p.ptt_key.is_empty() {
            ui.add_space(14.0 - 6.0);
            let b = Block::wrapped(ui, texts::quick_no_key(), text::META, c.muted, width);
            let (r, _) = ui.allocate_exact_size(vec2(width, b.height()), Sense::hover());
            b.paint(ui.painter(), r.min);
        }
        ui.add_space(14.0);
        let top = ui.cursor().min;
        crate::widgets::rule_h(ui.painter(), c.border, top.x..=top.x + width, top.y);
        // Full width under a rule, square, the label at the start.
        if super::chips::ghost_row(ui, texts::quick_all_settings(), top.x, width).clicked() {
            more = true;
        }
        ui.min_rect()
    });
    ui.allocate_exact_size(
        vec2(312.0 - 18.0, used.height() + 6.0 + 2.0),
        Sense::hover(),
    );
    if p != before {
        prefs::update(&ctx, |q| *q = p);
    }
    if more {
        super::dialogs::open_settings(&ctx, super::dialogs::Tab::Audio);
    }
    more
}

/// A volume row: label and `{pct} %` (`end` px from the right), the
/// playback slider under them.
fn volume(
    ui: &mut Ui,
    id: &str,
    label: &str,
    value: &mut f64,
    disabled: bool,
    width: f32,
    end: f32,
) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let (row, _) = ui.allocate_exact_size(vec2(width, 16.8), Sense::hover());
    Block::single(ui, label, text::META.weight(600.0), c.muted).paint(ui.painter(), row.min);
    let pct = Block::single(
        ui,
        &texts::percent(&((*value * 100.0).round() as i64).to_string()),
        text::META,
        c.text,
    );
    pct.paint(
        ui.painter(),
        egui::pos2(row.right() - end - pct.width(), row.top()),
    );
    ui.add_space(6.0);
    let mut pos = super::volume::position(*value);
    let r = Rect::from_min_size(ui.cursor().min, vec2(width, 20.0));
    let changed = controls::place(ui, UiBuilder::new().max_rect(r), |ui| {
        controls::slider(ui, id.to_owned(), &mut pos, 0.0, 200.0, 1.0, disabled)
    });
    ui.allocate_exact_size(vec2(width, 20.0), Sense::hover());
    if changed {
        *value = super::volume::volume(pos);
    }
}
