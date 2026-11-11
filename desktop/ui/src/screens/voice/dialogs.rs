//! The voice dialogs: "Audio & Video" (components/VoiceSettingsDialog.tsx,
//! the `openDialog` of voice/settings.ts) with the tabs Audio, "Video &
//! Stream" and Diagnose, and "Mikrofon testen" (voice/MicrophoneTest.tsx,
//! the desktop branch of voice/native/microphoneTest.ts: meters, no
//! recordings). Opened from the call bar, the room, the overview and the
//! settings, drawn once per frame by the shell.

use egui::{Color32, Id, Rect, Sense, Stroke, Ui, Vec2, pos2, vec2};
use gelabber_call::{MicTest, MicTestPhase};
use gelabber_i18n::{common, voice_settings as texts};

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
            Tab::Audio => texts::tabs_audio(),
            Tab::Video => texts::tabs_video(),
            Tab::Diagnose => texts::tabs_diagnostics(),
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
    /// The dialog opens with the focus on its tab (`autoFocus`).
    focus_tab: bool,
}

fn id() -> Id {
    Id::unique("gelabber-voice-dialogs")
}

fn with<R>(ctx: &egui::Context, f: impl FnOnce(&mut Open) -> R) -> R {
    ctx.data_mut(|d| f(d.get_temp_mut_or_default::<Open>(id())))
}

/// `openDialog(section)`: "Audio & Video" on `tab`.
pub fn open_settings(ctx: &egui::Context, tab: Tab) {
    with(ctx, |o| {
        if o.settings.is_none() {
            o.form = media::State::default();
        }
        o.settings = Some(tab);
        o.focus_tab = true;
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
    let mut chosen = tab;
    let mut done = false;
    let focus_tab = with(&ctx, |o| std::mem::take(&mut o.focus_tab));
    let mut footer = |ui: &mut Ui| {
        let w = Block::single(ui, common::done(), text::UI.weight(600.0), Color32::WHITE).width()
            + 34.0;
        if ui
            .add(Button::new(common::done()).ghost().width(w))
            .clicked()
        {
            done = true;
        }
    };
    // The body scrolls without a gutter of its own, as in the browser.
    let saved = ctx.global_style().spacing.scroll;
    ctx.all_styles_mut(|s| s.spacing.scroll.floating_allocated_width = 0.0);
    let output = Modal::new("voice-settings", texts::dialog_title())
        .width(ModalWidth::Large)
        .show(
            &ctx,
            |ui| {
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                // The web's title line is a pixel shorter than the modal's.
                ui.add_space(-1.0);
                let width = ui.available_width();
                if let Some(next) = tabs(ui, chosen, width, focus_tab) {
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
/// Only the chosen tab is in the Tab order; the arrows, Home and End move
/// between them (`nextChipIndex`).
fn tabs(ui: &mut Ui, current: Tab, width: f32, focus: bool) -> Option<Tab> {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let top = ui.cursor().min;
    let mut x = top.x;
    let mut chosen = None;
    let tab_id = |tab: Tab| Id::unique(("gelabber-voice-tab", tab.label()));
    // The keys first, so the tab they choose is the focusable one this frame.
    let mut current = current;
    let mut focus = focus;
    if ui.memory(|m| m.has_focus(tab_id(current))) {
        // The arrows choose a tab instead of moving egui's focus.
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                tab_id(current),
                egui::EventFilter {
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    ..Default::default()
                },
            )
        });
        let (prev, next, home, end) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::ArrowLeft) || i.key_pressed(egui::Key::ArrowUp),
                i.key_pressed(egui::Key::ArrowRight) || i.key_pressed(egui::Key::ArrowDown),
                i.key_pressed(egui::Key::Home),
                i.key_pressed(egui::Key::End),
            )
        });
        let at = Tab::ALL.iter().position(|t| *t == current).unwrap_or(0);
        let n = Tab::ALL.len();
        let to = if prev {
            Some((at + n - 1) % n)
        } else if next {
            Some((at + 1) % n)
        } else if home {
            Some(0)
        } else if end {
            Some(n - 1)
        } else {
            None
        };
        if let Some(to) = to.filter(|t| *t != at) {
            current = Tab::ALL[to];
            chosen = Some(current);
            focus = true;
        }
    }
    for tab in Tab::ALL {
        let selected = tab == current;
        let style = if selected {
            text::UI.weight(600.0)
        } else {
            text::UI
        };
        let block = Block::single(ui, tab.label(), style, c.text);
        let rect = Rect::from_min_size(pos2(x, top.y), vec2(block.width() + 24.0, 36.0));
        // Only the chosen tab is in the Tab order (`tabIndex`).
        let response = ui.interact(
            rect,
            tab_id(tab),
            if selected {
                Sense::click()
            } else {
                Sense::CLICK
            },
        );
        if selected && focus {
            response.request_focus();
        }
        if response.has_focus() && crate::widgets::keyboard_focus(ui.ctx()) {
            crate::widgets::focus_ring(ui.painter(), rect, theme.radius.sm, &theme);
        }
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
            response.request_focus();
        }
        x = rect.right() + 4.0;
    }
    crate::widgets::rule_h(ui.painter(), c.border, top.x..=top.x + width, top.y + 36.0);
    ui.allocate_rect(Rect::from_min_size(top, vec2(width, 37.0)), Sense::hover());
    chosen
}

/// `.voice-microphone-test meter`, styled as `.voice-quick-level`: a 6px
/// round track in the border colour, the level in green over it.
fn meter(ui: &mut Ui, value: f64, width: f32) {
    let theme = Theme::of(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(width, 6.0), Sense::hover());
    let round = corner(theme.radius.full);
    ui.painter().rect_filled(rect, round, theme.c.border);
    let filled = (value.clamp(0.0, 100.0) / 100.0) as f32 * width;
    if filled > 0.0 {
        ui.painter().rect_filled(
            Rect::from_min_size(rect.min, vec2(filled, rect.height())),
            round,
            theme.c.online_text,
        );
    }
}

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
    let output = Modal::new("mic-test", texts::mic_test_title()).show(
        &ctx,
        |ui| {
            let theme = Theme::of(ui.ctx());
            let c = theme.c;
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            let width = ui.available_width();
            let (r, _) = ui.allocate_exact_size(vec2(width, 36.0), Sense::hover());
            icons::paint(ui.painter(), Icon::Mic, r.min, 36.0, c.accent_text);
            ui.add_space(14.0);
            // The native test only meters: nothing is recorded (no A/B clips).
            controls::paragraph(ui, texts::mic_test_local_only(), text::BODY, c.muted, width);
            ui.add_space(14.0);
            for (label, value) in [
                (texts::mic_test_input_level(), test.input),
                (texts::mic_test_processed(), test.processed),
            ] {
                // Each meter sits close under its label (6px), apart from the next.
                controls::paragraph(ui, label, text::BODY, c.text, width);
                ui.add_space(6.0);
                meter(ui, value, width);
                ui.add_space(14.0);
            }
            // The fieldset: its legend, then the modes in a wrapping row. In
            // the settings their fieldset look applies: a framed box with
            // the legend on its edge and 36px rows.
            let legend = Block::single(
                ui,
                texts::mic_test_processing(),
                text::UI.weight(600.0),
                c.text,
            );
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
            for (mode, label, _) in media::PROCESSING_MODES {
                let label = label();
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
                MicTestPhase::Idle if stopped => texts::mic_test_stopped().to_owned(),
                MicTestPhase::Idle => texts::mic_test_intro().into(),
                MicTestPhase::Pending => texts::mic_test_starting().into(),
                MicTestPhase::Active => super::diagnostics::processing_text(
                    &p.processing_mode,
                    test.device.as_deref(),
                    &p.audio_input_id,
                ),
                MicTestPhase::Error(text) => text.clone(),
            };
            controls::paragraph(ui, &message, text::BODY, c.text, width);
            if test.clipping {
                ui.add_space(14.0);
                controls::paragraph(ui, texts::mic_test_clipping(), text::BODY, c.text, width);
            }
            ui.add_space(14.0);
            let top = ui.cursor().min;
            let primary = if running {
                texts::mic_test_stop()
            } else {
                texts::mic_test_start()
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
            let cw =
                Block::single(ui, common::close(), text::UI.weight(600.0), c.text).width() + 34.0;
            let close_rect = Rect::from_min_size(pos2(start.right() + 8.0, top.y), vec2(cw, 36.0));
            let ghost = Button::new(common::close()).ghost();
            let ghost = if framed {
                ghost.text_color(c.accent_text)
            } else {
                ghost
            };
            if ui.put(close_rect, ghost).clicked() {
                close = true;
            }
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
