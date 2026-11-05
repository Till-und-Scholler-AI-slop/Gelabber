//! The voice dialogs: "Voice & Video" (components/VoiceSettingsDialog.tsx,
//! the `openDialog` of voice/settings.ts) with the tabs Audio, "Video &
//! Stream" and Diagnose, and "Mikrofon testen" (voice/MicrophoneTest.tsx,
//! the desktop branch of voice/native/microphoneTest.ts: meters, no
//! recordings). Opened from the call bar, the room, the overview and the
//! settings, drawn once per frame by the shell.

use egui::{Color32, Id, Rect, Sense, Stroke, Ui, Vec2, pos2, vec2};
use gelabber_call::{MicTest, MicTestPhase};

use crate::app::AppCtx;
use crate::icons::{self, Icon};
use crate::screens::settings::{controls, media, prefs};
use crate::text::{self, Block};
use crate::theme::{Theme, corner};
use crate::widgets::{Button, Modal, ModalWidth};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Audio,
    Video,
    Diagnose,
}

impl Tab {
    const ALL: [Tab; 3] = [Tab::Audio, Tab::Video, Tab::Diagnose];

    fn label(self) -> &'static str {
        match self {
            Tab::Audio => "Audio",
            Tab::Video => "Video & Stream",
            Tab::Diagnose => "Diagnose",
        }
    }
}

#[derive(Clone, Default)]
struct Open {
    settings: Option<Tab>,
    mic_test: bool,
    /// The forms' own state (key recording, disclosures) while open.
    form: media::State,
    /// The message of the microphone test once stopped by the person.
    stopped: bool,
}

fn id() -> Id {
    Id::unique("gelabber-voice-dialogs")
}

fn with<R>(ctx: &egui::Context, f: impl FnOnce(&mut Open) -> R) -> R {
    ctx.data_mut(|d| f(d.get_temp_mut_or_default::<Open>(id())))
}

/// `openDialog(section)`: "Voice & Video" on `tab`.
pub fn open_settings(ctx: &egui::Context, tab: Tab) {
    with(ctx, |o| {
        if o.settings.is_none() {
            o.form = media::State::default();
        }
        o.settings = Some(tab);
    });
}

/// "Mikrofon testen", over whatever is open.
pub fn open_mic_test(ctx: &egui::Context) {
    with(ctx, |o| {
        o.mic_test = true;
        o.stopped = false;
    });
}

/// A voice dialog is open (the room's keys keep off then).
pub fn open(ctx: &egui::Context) -> bool {
    with(ctx, |o| o.settings.is_some() || o.mic_test)
}

/// The open dialogs, the settings under the microphone test.
pub fn show(ui: &mut Ui, app: &mut AppCtx) {
    let ctx = ui.ctx().clone();
    super::window::show(&ctx, app.call);
    let (settings, mic_test) = with(&ctx, |o| (o.settings, o.mic_test));
    if let Some(tab) = settings {
        settings_dialog(ui, app, tab, !mic_test);
    }
    if mic_test {
        mic_test_dialog(ui, app);
    } else if app.call.mic_test().phase != MicTestPhase::Idle {
        // A test runs only while its dialog is open.
        app.call.stop_mic_test();
    }
}

fn settings_dialog(ui: &mut Ui, app: &mut AppCtx, tab: Tab, on_top: bool) {
    let ctx = ui.ctx().clone();
    let mut form = with(&ctx, |o| o.form.clone());
    // In a call the microphone's names are known: no "Gerätenamen anzeigen".
    if app.call.with_state(|s| s.channel.is_some()) {
        form.revealed = true;
    }
    let mut chosen = tab;
    let mut done = false;
    let mut footer = |ui: &mut Ui| {
        let w = Block::single(ui, "Fertig", text::UI.weight(600.0), Color32::WHITE).width() + 34.0;
        if ui.add(Button::new("Fertig").ghost().width(w)).clicked() {
            done = true;
        }
    };
    // The body scrolls without a gutter of its own, as in the browser.
    let saved = ctx.global_style().spacing.scroll;
    ctx.all_styles_mut(|s| {
        s.spacing.scroll.floating = true;
        s.spacing.scroll.floating_allocated_width = 0.0;
        s.spacing.scroll.dormant_handle_opacity = 0.0;
    });
    let output = Modal::new("voice-settings", "Voice & Video")
        .width(ModalWidth::Large)
        .show(
            &ctx,
            |ui| {
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                // The web's title line is a pixel shorter than the modal's.
                ui.add_space(-1.0);
                let width = ui.available_width();
                if let Some(next) = tabs(ui, chosen, width) {
                    chosen = next;
                }
                ui.add_space(20.0);
                match chosen {
                    Tab::Audio => {
                        media::set_in_dialog(ui.ctx(), true);
                        media::audio(ui, app, &mut form);
                        media::set_in_dialog(ui.ctx(), false);
                    }
                    Tab::Video => {
                        media::set_in_dialog(ui.ctx(), true);
                        media::video(ui, app, &mut form);
                        media::set_in_dialog(ui.ctx(), false);
                    }
                    Tab::Diagnose => super::diagnostics::tab(ui, app, width),
                }
            },
            Some(&mut footer),
        );
    ctx.all_styles_mut(|s| s.spacing.scroll = saved);
    let close = done || (on_top && output.close_requested);
    with(&ctx, |o| {
        o.form = form;
        if close {
            o.settings = None;
        } else if o.settings.is_some() {
            o.settings = Some(chosen);
        }
    });
}

/// The tab list (`.voice-settings-tablist`); returns a newly chosen tab.
fn tabs(ui: &mut Ui, current: Tab, width: f32) -> Option<Tab> {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let top = ui.cursor().min;
    let mut x = top.x;
    let mut chosen = None;
    for tab in Tab::ALL {
        let selected = tab == current;
        let style = if selected {
            text::UI.weight(600.0)
        } else {
            text::UI
        };
        let block = Block::single(ui, tab.label(), style, c.text);
        let rect = Rect::from_min_size(pos2(x, top.y), vec2(block.width() + 24.0, 36.0));
        let response = ui.interact(
            rect,
            Id::unique(("gelabber-voice-tab", tab.label())),
            Sense::click(),
        );
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::Role::Tab, true, selected, tab.label())
        });
        let color = if selected || response.hovered() {
            c.text
        } else {
            c.muted
        };
        block.paint_tinted(
            ui.painter(),
            pos2(rect.left() + 12.0, rect.center().y - block.height() / 2.0),
            Some(color),
        );
        if selected {
            ui.painter().rect_filled(
                Rect::from_min_max(
                    pos2(rect.left(), rect.bottom() - 1.0),
                    pos2(rect.right(), rect.bottom() + 1.0),
                ),
                0.0,
                c.accent,
            );
        }
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if response.clicked() && !selected {
            chosen = Some(tab);
        }
        x = rect.right() + 4.0;
    }
    ui.painter().hline(
        top.x..=top.x + width,
        top.y + 36.0 + 0.5,
        Stroke::new(1.0, c.border),
    );
    ui.allocate_rect(Rect::from_min_size(top, vec2(width, 37.0)), Sense::hover());
    chosen
}

/// `.voice-microphone-test meter`: Chromium's meter, 12px of a 24px box;
/// the level in the accent over its track.
fn meter(ui: &mut Ui, value: f64, width: f32) {
    let theme = Theme::of(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(width, 24.0), Sense::hover());
    // The track's rows, top to bottom, as Chromium paints them.
    const ROWS: [u8; 12] = [225, 232, 237, 225, 215, 204, 204, 207, 210, 213, 217, 219];
    let top = rect.top() + 6.0;
    for (i, g) in ROWS.iter().enumerate() {
        ui.painter().rect_filled(
            Rect::from_min_size(pos2(rect.left(), top + i as f32), vec2(width, 1.0)),
            0.0,
            Color32::from_gray(*g),
        );
    }
    let filled = (value.clamp(0.0, 100.0) / 100.0) as f32 * width;
    if filled > 0.0 {
        ui.painter().rect_filled(
            Rect::from_min_size(pos2(rect.left(), top), vec2(filled, 12.0)),
            0.0,
            theme.c.accent,
        );
    }
}

/// `AUDIO_PROCESSING`'s labels, as the settings' filter cards name them
/// (the desktop branch says "WebRTC-Filter").
const MODES: [(&str, &str); 3] = [
    ("enhanced", "Verbesserte Sprache"),
    ("browser", "Browserfilter"),
    ("original", "Original / Musik"),
];

fn mic_test_dialog(ui: &mut Ui, app: &mut AppCtx) {
    let ctx = ui.ctx().clone();
    let test: MicTest = app.call.mic_test();
    let stopped = with(&ctx, |o| o.stopped);
    let running = matches!(test.phase, MicTestPhase::Active | MicTestPhase::Pending);
    let mut p = prefs::get(&ctx);
    let before = p.clone();
    let mut action: Option<bool> = None;
    let mut close = false;
    // Opened from the settings, the settings page's form look reaches it.
    let framed = matches!(app.nav.route(), crate::route::Route::Settings(_));
    let output = Modal::new("mic-test", "Mikrofon testen").show(
        &ctx,
        |ui| {
            let theme = Theme::of(ui.ctx());
            let c = theme.c;
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            let width = ui.available_width();
            let (r, _) = ui.allocate_exact_size(vec2(width, 36.0), Sense::hover());
            icons::paint(ui.painter(), Icon::Mic, r.min, 36.0, c.accent_text);
            ui.add_space(14.0);
            // The browser's wording, as the settings page keeps it; the
            // desktop branch would say "ohne Aufnahme oder Übertragung".
            controls::paragraph(
                ui,
                "Der Test läuft lokal, ohne Übertragung. Aufnahmen dauern höchstens acht Sekunden und werden beim Schließen gelöscht.",
                text::BODY,
                c.muted,
                width,
            );
            ui.add_space(14.0);
            for (label, value) in [
                ("Eingangspegel", test.input),
                ("Nach Verarbeitung", test.processed),
            ] {
                controls::paragraph(ui, label, text::BODY, c.text, width);
                ui.add_space(14.0);
                meter(ui, value, width);
                ui.add_space(14.0);
            }
            // The fieldset: its legend, then the modes in a wrapping row. In
            // the settings their fieldset look applies: a framed box with
            // the legend on its edge and 36px rows.
            let legend = Block::single(ui, "Verarbeitung", text::UI.weight(600.0), c.text);
            let frame_top = ui.cursor().min;
            let (pad, row_h, legend_h) = if framed {
                // The frame's top edge runs through the legend's middle.
                (18.0, 36.0, legend.height() / 2.0 + 33.0)
            } else {
                (0.0, 32.0, legend.height() + 6.0)
            };
            let (r, _) = ui.allocate_exact_size(vec2(width, legend_h), Sense::hover());
            if !framed {
                legend.paint(ui.painter(), r.min);
            }
            let top = ui.cursor().min + vec2(pad, 0.0);
            let inner = width - 2.0 * pad;
            let (mut x, mut y) = (top.x, top.y);
            for (mode, label) in MODES {
                let block = Block::single(ui, label, text::UI.weight(400.0), c.text);
                let w = 20.0 + 8.0 + block.width();
                if x > top.x && x + w > top.x + inner {
                    x = top.x;
                    y += row_h + 8.0;
                }
                let rect = Rect::from_min_size(pos2(x, y), vec2(w, row_h));
                let response = ui.interact(
                    rect,
                    Id::unique(("gelabber-mic-test-mode", mode)),
                    Sense::click(),
                );
                let checked = p.processing_mode == mode;
                response.widget_info(|| {
                    egui::WidgetInfo::selected(egui::Role::RadioButton, true, checked, label)
                });
                controls::radio_mark(ui, pos2(x, rect.center().y - 10.0), checked);
                block.paint(
                    ui.painter(),
                    pos2(x + 28.0, rect.center().y - block.height() / 2.0),
                );
                if response.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                if response.clicked() && !checked {
                    p.processing_mode = mode.into();
                    // A running test compares with the new mode straight away.
                    if running {
                        action = Some(true);
                    }
                }
                x += w + 16.0;
            }
            ui.allocate_rect(
                Rect::from_min_max(top, pos2(top.x + inner, y + row_h)),
                Sense::hover(),
            );
            if framed {
                ui.add_space(19.0);
                let frame = Rect::from_min_max(
                    frame_top + vec2(0.0, legend.height() / 2.0),
                    pos2(frame_top.x + width, ui.cursor().min.y),
                );
                let painter = ui.painter();
                painter.rect(
                    frame,
                    corner(theme.radius.md),
                    Color32::TRANSPARENT,
                    Stroke::new(1.0, c.border),
                    egui::StrokeKind::Inside,
                );
                // The legend interrupts the frame's top edge.
                let lx = frame.left() + 25.0;
                painter.rect_filled(
                    Rect::from_min_size(
                        pos2(lx - 4.0, frame.top() - 1.0),
                        vec2(legend.width() + 8.0, 3.0),
                    ),
                    0.0,
                    c.elevated,
                );
                legend.paint(painter, pos2(lx, frame.top() - legend.height() / 2.0));
            }
            ui.add_space(14.0);
            let message = match &test.phase {
                MicTestPhase::Idle if stopped => {
                    "Test beendet. Mikrofon und Aufnahmen wurden freigegeben.".to_owned()
                }
                MicTestPhase::Idle => "Prüfe dein Mikrofon und vergleiche die Verarbeitung.".into(),
                MicTestPhase::Pending => "Mikrofon wird gestartet.".into(),
                MicTestPhase::Active => super::diagnostics::processing_message(
                    &gelabber_call::CallState {
                        channel: Some(uuid::Uuid::nil()),
                        ..Default::default()
                    },
                    &p.processing_mode,
                ),
                MicTestPhase::Error(text) => text.clone(),
            };
            controls::paragraph(ui, &message, text::BODY, c.text, width);
            if test.clipping {
                ui.add_space(14.0);
                controls::paragraph(
                    ui,
                    "Übersteuerung erkannt. Verringere den Mic-Gain oder Gerätepegel.",
                    text::BODY,
                    c.text,
                    width,
                );
            }
            ui.add_space(14.0);
            let top = ui.cursor().min;
            let primary = if running {
                "Test beenden"
            } else {
                "Test starten"
            };
            let pw = Block::single(ui, primary, text::UI.weight(600.0), c.text).width() + 34.0;
            let start = Rect::from_min_size(top, vec2(pw, 36.0));
            let button = if running {
                Button::new(primary)
            } else {
                Button::new(primary).primary()
            };
            if ui.put(start, button).clicked() {
                action = Some(!running);
            }
            let cw = Block::single(ui, "Schließen", text::UI.weight(600.0), c.text).width() + 34.0;
            let close_rect = Rect::from_min_size(pos2(start.right() + 8.0, top.y), vec2(cw, 36.0));
            let ghost = Button::new("Schließen").ghost();
            let ghost = if framed {
                ghost.text_color(c.accent_text)
            } else {
                ghost
            };
            if ui.put(close_rect, ghost).clicked() {
                close = true;
            }
            ui.add_space(14.0);
            controls::paragraph(
                ui,
                "Ein Wechsel der Verarbeitung startet den Test neu und löscht die Aufnahmen.",
                text::BODY,
                c.muted,
                width,
            );
        },
        None,
    );
    if p != before {
        prefs::update(&ctx, |q| *q = p.clone());
        // The call follows the saved settings this frame, the test with it.
        app.call
            .set_settings(super::sync::settings(&prefs::get(&ctx)));
    }
    match action {
        Some(true) => {
            app.call.start_mic_test();
            with(&ctx, |o| o.stopped = false);
        }
        Some(false) => {
            app.call.stop_mic_test();
            with(&ctx, |o| o.stopped = true);
        }
        None => {}
    }
    if close || output.close_requested {
        app.call.stop_mic_test();
        with(&ctx, |o| {
            o.mic_test = false;
            o.stopped = false;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dialogs_open_and_close_by_their_state() {
        let ctx = egui::Context::default();
        assert!(!open(&ctx));
        open_settings(&ctx, Tab::Diagnose);
        assert!(open(&ctx));
        assert_eq!(with(&ctx, |o| o.settings), Some(Tab::Diagnose));
        open_mic_test(&ctx);
        assert!(with(&ctx, |o| o.mic_test));
    }
}
