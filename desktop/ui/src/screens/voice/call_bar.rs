//! The call bar (components/VoiceSessionControls.tsx, VoiceControls.tsx,
//! room.css `.voice-session-dock`): one 56px strip with the user panel,
//! from the sidebar's edge to the window's. Left the connection and where
//! the call runs, in the middle who is in it, right the microphone,
//! headset, camera, "Live", the settings and "Verlassen".

use egui::{Color32, Id, Key, Modifiers, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};
use gelabber_call::{CallState, CallStatus, LiveSource};
use uuid::Uuid;

use super::look;
use super::{quick, session, volume};
use crate::app::AppCtx;
use crate::icons::{self, Icon};
use crate::screens::settings::prefs;
use crate::screens::shell::common::{self, line, paint_mid};
use crate::text::{self, Block, TextStyle};
use crate::theme::{Theme, corner, mix};
use crate::widgets::{Button, IconButton, Popover, eased, hairline_h};

/// `SHOWN_PEOPLE`.
const SHOWN: usize = 3;

/// Whether the bar shows: while a call runs, or to offer a lost one again.
pub fn visible(app: &AppCtx) -> bool {
    app.call
        .with_state(|s| session::joined(s).is_some() || s.lost_call.is_some())
}

/// `.voice-session-lost`: the warning, "Verbindung verloren" over the
/// channel, "Erneut versuchen" and "Schließen".
fn lost(ui: &mut Ui, app: &mut AppCtx, rect: Rect, server: Uuid, channel: Uuid) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let painter = ui.painter().clone();
    painter.rect_filled(rect, 0.0, c.panel);
    hairline_h(&painter, &theme, rect.left()..=rect.right(), rect.top());
    let cy = rect.top() + 1.0 + 8.0 + (rect.height() - 1.0 - 16.0) / 2.0;
    let left = rect.left() + 16.0;
    let right = rect.right() - 16.0;
    let close = Rect::from_min_size(pos2(right - 36.0, cy - 18.0), Vec2::splat(36.0));
    if ui
        .add(IconButton::new(Icon::Close, "Schließen").at(close))
        .clicked()
    {
        app.call.dismiss_lost_call();
    }
    let label = "Erneut versuchen";
    let width = Block::single(ui, label, text::UI.weight(600.0), c.text).width() + 34.0;
    let retry = Rect::from_min_size(
        pos2(close.left() - 6.0 - width, cy - 18.0),
        vec2(width, 36.0),
    );
    if ui.put(retry, Button::new(label)).clicked() {
        app.call.join(server, channel);
    }
    let x = left + 2.0;
    icons::paint(&painter, Icon::Warning, pos2(x, cy - 8.0), 16.0, c.danger);
    let x = x + 16.0 + 8.0;
    let max = (retry.left() - 6.0 - x).max(10.0);
    let name = app
        .client
        .with_state(|s| s.channel(channel).map(|ch| ch.name.clone()))
        .unwrap_or_default();
    let strong = line(
        ui,
        "Verbindung verloren",
        TextStyle::new(12.0, 600.0, 1.3),
        c.danger,
        max,
    );
    strong.paint(&painter, pos2(x, cy - 15.6));
    let place = line(ui, &name, TextStyle::new(12.0, 400.0, 1.3), c.muted, max);
    place.paint(&painter, pos2(x, cy));
}

/// One person in the call, for the avatars and the "Im Anruf" list.
struct InCall {
    id: Uuid,
    name: String,
    url: Option<String>,
    speaking: bool,
    me: bool,
}

fn people(app: &AppCtx, call: &CallState, server: Uuid, channel: Uuid) -> Vec<InCall> {
    app.client.with_state(|s| {
        let me = s.me().map(|m| m.id);
        let members = s
            .detail(server)
            .map(|d| d.members.clone())
            .unwrap_or_default();
        let mut out: Vec<InCall> = session::occupants(s, call, server, channel, true)
            .into_iter()
            .map(|(id, o)| {
                let member = members.iter().find(|m| m.user_id == id);
                InCall {
                    id,
                    name: member.map_or("Jemand".into(), |m| m.name.clone()),
                    url: member.and_then(|m| m.avatar_url.clone()),
                    speaking: session::speaking(call, id, o.muted, o.deafened),
                    me: Some(id) == me,
                }
            })
            .collect();
        out.sort_by_cached_key(|p| gelabber_client::store::collation_key(&p.name));
        out
    })
}

/// Strg+Umschalt+M and Strg+Umschalt+D while joined (`callShortcuts.ts`).
fn shortcuts(ui: &Ui, app: &AppCtx, state: &CallState) {
    let combo = Modifiers::COMMAND | Modifiers::SHIFT;
    let (mute, deafen) = ui
        .ctx()
        .input_mut(|i| (i.consume_key(combo, Key::M), i.consume_key(combo, Key::D)));
    if mute {
        app.call.set_muted(!(state.muted || state.deafened));
    }
    if deafen {
        app.call.set_deafened(!state.deafened);
    }
}

/// Draws the bar into `rect` (56px high).
pub fn show(ui: &mut Ui, app: &mut AppCtx, rect: Rect) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let state = app.call.state();
    let Some((server, channel)) = session::joined(&state) else {
        if let Some((server, channel)) = state.lost_call {
            lost(ui, app, rect, server, channel);
        }
        return;
    };
    shortcuts(ui, app, &state);
    let painter = ui.painter().clone();
    painter.rect_filled(rect, 0.0, c.panel);
    hairline_h(&painter, &theme, rect.left()..=rect.right(), rect.top());
    // The row: 36 high, centred between the 8px paddings under the line.
    let cy = rect.top() + 1.0 + 8.0 + (rect.height() - 1.0 - 16.0) / 2.0;
    let left = rect.left() + 16.0;
    let right = rect.right() - 16.0;

    let (channel_name, server_name, can_live) = app.client.with_state(|s| {
        (
            s.channel(channel).map(|c| c.name.clone()),
            s.server(server).map(|v| v.name.clone()),
            s.server(server)
                .is_none_or(|v| v.can(gelabber_client::Permission::GoLive)),
        )
    });

    // Right side, from the end.
    let leave_label = Block::single(ui, "Verlassen", text::UI, c.on_danger);
    // Every `.voice-control` keeps a 1px (transparent) border.
    let leave_w = 1.0 + 12.0 + 18.0 + 4.0 + leave_label.width() + 12.0 + 1.0;
    let leave = Rect::from_min_size(pos2(right - leave_w, cy - 18.0), vec2(leave_w, 36.0));
    let response = common::click(ui, leave, Id::unique("gelabber-call-leave"));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Verlassen"));
    let response = response.on_hover_text("Verlassen");
    let fill = eased(
        ui.ctx(),
        response.id.with("hover"),
        response.hovered(),
        c.danger_bg,
        mix(c.danger_bg, Color32::BLACK, 0.12),
    );
    painter.rect_filled(leave, corner(theme.radius.md), fill);
    icons::paint(
        &painter,
        Icon::HangUp,
        pos2(leave.left() + 13.0, cy - 9.0),
        18.0,
        c.on_danger,
    );
    paint_mid(&painter, &leave_label, leave.left() + 13.0 + 18.0 + 4.0, cy);
    if response.clicked() {
        app.call.leave();
    }
    let mut x = leave.left() - 4.0 - 6.0;

    // Controls, gap 4.
    let muted = state.muted || state.deafened;
    let gear = control(
        ui,
        &mut x,
        cy,
        Icon::Gear,
        None,
        false,
        Tone::Plain,
        "Voice-Einstellungen",
        "Voice-Einstellungen",
    );
    let gear_rect = gear.rect;
    quick::button(ui, app, gear, gear_rect);
    // Stream sound, while someone's Live plays here.
    let watching = state
        .participants
        .iter()
        .any(|p| !p.me && p.watching && p.live.is_some());
    if watching {
        let stream_muted = prefs::get(ui.ctx()).source_audio_muted;
        let sound = control(
            ui,
            &mut x,
            cy,
            if stream_muted {
                Icon::SpeakerOff
            } else {
                Icon::Speaker
            },
            None,
            false,
            if stream_muted { Tone::Off } else { Tone::Plain },
            "Lautstärke",
            "Lautstärke",
        );
        let rect = sound.rect;
        quick::sound_button(ui, sound, rect, state.deafened);
    }
    let live_title = if state.live {
        "Du bist live"
    } else {
        "Live gehen"
    };
    let live = control(
        ui,
        &mut x,
        cy,
        Icon::Live,
        Some("Live"),
        true,
        if state.live { Tone::Live } else { Tone::Plain },
        "Live gehen",
        live_title,
    );
    live_menu(ui, app, &live, &state, can_live);
    let camera = control(
        ui,
        &mut x,
        cy,
        if state.camera {
            Icon::Camera
        } else {
            Icon::CameraOff
        },
        None,
        false,
        if state.camera { Tone::On } else { Tone::Plain },
        "Kamera",
        if state.camera {
            "Kamera ausschalten"
        } else {
            "Kamera einschalten"
        },
    );
    if camera.clicked() {
        app.call.set_camera(!state.camera);
    }
    let deafen = control(
        ui,
        &mut x,
        cy,
        if state.deafened {
            Icon::HeadsetOff
        } else {
            Icon::Headset
        },
        None,
        false,
        if state.deafened {
            Tone::Off
        } else {
            Tone::Plain
        },
        "Taubschalten",
        if state.deafened {
            "Taubschaltung aufheben (Strg+Umschalt+D)"
        } else {
            "Taubschalten (Strg+Umschalt+D)"
        },
    );
    if deafen.clicked() {
        app.call.set_deafened(!state.deafened);
    }
    let ptt = prefs::get(ui.ctx()).voice_input == "ptt";
    let held = push_to_talk(ui, app, ptt && !muted);
    let mic_title = format!(
        "{}{}",
        if muted {
            "Stummschaltung aufheben (Strg+Umschalt+M)"
        } else {
            "Stummschalten (Strg+Umschalt+M)"
        },
        if ptt && !muted {
            " · Push-to-Talk"
        } else {
            ""
        }
    );
    let mic = control(
        ui,
        &mut x,
        cy,
        if muted { Icon::MicOff } else { Icon::Mic },
        (ptt && !muted).then_some("PTT"),
        false,
        if muted {
            Tone::Off
        } else if ptt && held {
            Tone::Talking
        } else if ptt {
            Tone::Idle
        } else {
            Tone::Plain
        },
        "Stummschalten",
        &mic_title,
    );
    if mic.clicked() {
        app.call.set_muted(!muted);
    }
    let controls_left = x + 4.0;

    // Status head: flex 0 1 340px.
    let head_w = 340.0_f32.min(controls_left - 6.0 - left).max(96.0);
    let head = Rect::from_min_size(pos2(left, cy - 18.0), vec2(head_w, 36.0));
    status(
        ui,
        app,
        head,
        &state,
        server,
        channel,
        channel_name.as_deref(),
        server_name.as_deref(),
    );

    // People in the middle (auto margins between head and controls).
    let list = people(app, &state, server, channel);
    if !list.is_empty() {
        let mut shown: Vec<&InCall> = list
            .iter()
            .filter(|p| p.speaking)
            .chain(list.iter().filter(|p| !p.speaking))
            .take(SHOWN)
            .collect();
        shown.sort_by_cached_key(|p| gelabber_client::store::collation_key(&p.name));
        let more = list.len() - shown.len();
        let more_block = (more > 0)
            .then(|| Block::single(ui, &format!("+{more}"), text::META.weight(600.0), c.muted));
        let avatars_w = shown.len() as f32 * 24.0 - (shown.len() as f32 - 1.0) * 5.0;
        let w = 6.0 + avatars_w + more_block.as_ref().map_or(0.0, |b| 4.0 + b.width()) + 6.0;
        let start = head.right() + 6.0;
        let end = controls_left - 6.0;
        let x0 = start + ((end - start) - w).max(0.0) / 2.0;
        let button = Rect::from_min_size(pos2(x0, cy - 18.0), vec2(w, 36.0));
        let names: Vec<String> = list.iter().map(|p| p.name.clone()).collect();
        let names = names.join(", ");
        let response = common::click(ui, button, Id::unique("gelabber-call-people"));
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::Button, true, format!("Im Anruf: {names}"))
        });
        let response = response.on_hover_text(&names);
        if response.hovered() {
            painter.rect_filled(button, corner(theme.radius.md), c.hover);
        }
        let under = if response.hovered() { c.hover } else { c.panel };
        let mut ax = button.left() + 6.0;
        for p in &shown {
            let r = Rect::from_min_size(pos2(ax, cy - 12.0), Vec2::splat(24.0));
            // The 2px panel ring (room.css wins over the green one), the
            // glow over it.
            painter.circle_filled(r.center(), 14.0, under);
            if p.speaking {
                look::speaking_ring(ui, &painter, r, false);
            }
            common::avatar(
                ui,
                app.images,
                r,
                &p.name,
                p.url.as_deref(),
                10.0,
                1.0,
                under,
            );
            ax += 24.0 - 5.0;
        }
        if let Some(b) = more_block {
            paint_mid(&painter, &b, ax + 5.0 + 4.0, cy);
        }
        people_popover(ui, app, &response, &list);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tone {
    Plain,
    /// Microphone or headset closed: danger on a 16 % danger fill.
    Off,
    /// Camera on: accent text on the selected fill.
    On,
    /// Live: white on danger.
    Live,
    /// Push-to-talk held: the speaking colour.
    Talking,
    /// Push-to-talk waiting: the icon dimmed.
    Idle,
}

/// Push-to-talk inside the window: the chosen key (or Maustaste 4/5)
/// opens the microphone while held. Returns whether it is held.
fn push_to_talk(ui: &Ui, app: &AppCtx, active: bool) -> bool {
    let id = Id::unique("gelabber-ptt-held");
    let was: bool = ui.ctx().data(|d| d.get_temp(id)).unwrap_or(false);
    if !active {
        if was {
            app.call.push_to_talk(false);
            ui.ctx().data_mut(|d| d.insert_temp(id, false));
        }
        return false;
    }
    let code = prefs::get(ui.ctx()).ptt_key;
    let held = ui.input(|i| match code.as_str() {
        "Mouse4" => i.pointer.button_down(egui::PointerButton::Extra1),
        "Mouse5" => i.pointer.button_down(egui::PointerButton::Extra2),
        "" => false,
        code => {
            let name = code
                .strip_prefix("Key")
                .or_else(|| code.strip_prefix("Digit"))
                .unwrap_or(code);
            egui::Key::from_name(name).is_some_and(|k| i.key_down(k))
                && !ui.ctx().egui_wants_keyboard_input()
        }
    });
    if held != was {
        app.call.push_to_talk(held);
        ui.ctx().data_mut(|d| d.insert_temp(id, held));
    }
    held
}

/// A `.voice-control-compact` button ending at `*x`; moves `*x` left.
#[allow(clippy::too_many_arguments)]
fn control(
    ui: &mut Ui,
    x: &mut f32,
    cy: f32,
    icon: Icon,
    label: Option<&str>,
    caret: bool,
    tone: Tone,
    name: &str,
    title: &str,
) -> egui::Response {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    // "PTT" is a caps badge; "Live" a word.
    let style = if label == Some("PTT") {
        text::CAPS.tracking(0.04)
    } else {
        text::UI
    };
    let block = label.map(|l| Block::single(ui, l, style, c.text));
    let mut w = 1.0 + 8.0 + 17.0 + 8.0 + 1.0;
    if let Some(b) = &block {
        w += 4.0 + b.width();
    }
    if caret {
        w += 4.0 + 12.0;
    }
    let w = w.max(36.0);
    let rect = Rect::from_min_size(pos2(*x - w, cy - 18.0), vec2(w, 36.0));
    *x -= w + 4.0;
    let response = common::click(ui, rect, Id::unique(("gelabber-call-control", name)));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, name));
    let response = response.on_hover_text(title);
    let hovered = response.hovered();
    let (fill, fg) = match tone {
        Tone::Off => (c.danger_bg.gamma_multiply(0.16), c.danger),
        Tone::On => (c.selected, c.accent_text),
        Tone::Live => (c.danger_bg, c.on_danger),
        Tone::Talking => (
            if hovered {
                c.hover
            } else {
                Color32::TRANSPARENT
            },
            c.speaking,
        ),
        Tone::Idle if hovered => (c.hover, c.text),
        Tone::Idle => (Color32::TRANSPARENT, c.muted),
        Tone::Plain if hovered => (c.hover, c.text),
        Tone::Plain => (Color32::TRANSPARENT, c.muted),
    };
    let painter = ui.painter();
    painter.rect(
        rect,
        corner(theme.radius.md),
        fill,
        Stroke::NONE,
        StrokeKind::Inside,
    );
    let mut cx = rect.left() + 9.0;
    if block.is_none() && !caret {
        cx = rect.center().x - 8.5;
    }
    let icon_fg = if tone == Tone::Idle {
        fg.gamma_multiply(0.55)
    } else {
        fg
    };
    icons::paint(painter, icon, pos2(cx, cy - 8.5), 17.0, icon_fg);
    cx += 17.0 + 4.0;
    if let Some(b) = &block {
        b.paint_tinted(painter, pos2(cx, cy - b.height() / 2.0), Some(fg));
        cx += b.width() + 4.0;
    }
    if caret {
        icons::paint_rotated(
            painter,
            Icon::Chevron,
            pos2(cx, cy - 6.0),
            12.0,
            fg.gamma_multiply(0.75),
        );
    }
    response
}

/// The connection's state in words and where the call runs; the bars
/// once connected.
#[allow(clippy::too_many_arguments)]
fn status(
    ui: &mut Ui,
    app: &mut AppCtx,
    head: Rect,
    state: &CallState,
    server: Uuid,
    channel: Uuid,
    channel_name: Option<&str>,
    server_name: Option<&str>,
) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let painter = ui.painter().clone();
    let cy = head.center().y;
    let mut x = head.left();
    let (word, color) = match &state.status {
        CallStatus::Connected => ("Verbunden", c.online_text),
        CallStatus::Reconnecting => (
            "Verbindung wird wiederhergestellt …",
            mix(c.warning, c.text, 0.3),
        ),
        CallStatus::Failed(_) => ("Verbindung verloren", c.danger),
        _ => ("Verbinde …", c.muted),
    };
    match &state.status {
        CallStatus::Connected => {
            let bars = Rect::from_min_size(pos2(x, cy - 18.0), Vec2::splat(36.0));
            let level = state.quality.min(4);
            let label = "Verbindung: wird gemessen …";
            let response = common::click(ui, bars, Id::unique("gelabber-call-quality"));
            response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::Role::Button,
                    true,
                    format!("{label} – Details anzeigen"),
                )
            });
            let response = response.on_hover_text(label);
            if response.hovered() {
                painter.rect_filled(bars, corner(theme.radius.md), c.hover);
            }
            let tone = match level {
                4 | 3 => c.online,
                2 => c.warning,
                1 => c.danger,
                _ => c.muted,
            };
            let bottom = bars.top() + 11.0 + 14.0;
            let mut bx = bars.center().x - 9.0;
            for (i, h) in [4.0, 7.0, 10.0, 13.0].iter().enumerate() {
                let lit = (i as u8) < level;
                let r = Rect::from_min_max(pos2(bx, bottom - h), pos2(bx + 3.0, bottom));
                painter.rect_filled(
                    r,
                    corner(1.0),
                    if lit { tone } else { tone.gamma_multiply(0.28) },
                );
                bx += 5.0;
            }
            x = bars.right() + 8.0;
        }
        CallStatus::Failed(_) => {
            icons::paint(
                &painter,
                Icon::Warning,
                pos2(x + 2.0, cy - 8.0),
                16.0,
                c.danger,
            );
            x += 2.0 + 16.0 + 8.0;
        }
        other => {
            let tone = if *other == CallStatus::Reconnecting {
                c.warning
            } else {
                c.muted
            };
            let alpha = if *other == CallStatus::Connecting {
                let t = ui.input(|i| i.time) as f32;
                ui.ctx().request_repaint();
                let phase = (t / 1.2).fract();
                let k = if phase < 0.5 {
                    phase * 2.0
                } else {
                    2.0 - phase * 2.0
                };
                1.0 - 0.65 * k
            } else {
                1.0
            };
            icons::paint(
                &painter,
                Icon::Signal,
                pos2(x + 2.0, cy - 8.0),
                16.0,
                tone.gamma_multiply(alpha),
            );
            x += 2.0 + 16.0 + 8.0;
        }
    }
    let channel_name = channel_name.unwrap_or("Sprachkanal");
    let where_ = match server_name {
        Some(s) if !s.is_empty() => format!("{channel_name} · {s}"),
        _ => channel_name.to_owned(),
    };
    let mut link_right = head.right();
    if matches!(state.status, CallStatus::Failed(_)) {
        let label = "Erneut versuchen";
        let width = Block::single(ui, label, text::UI.weight(600.0), c.text).width() + 34.0;
        let r = Rect::from_min_size(pos2(head.right() - width, cy - 18.0), vec2(width, 36.0));
        if ui.put(r, Button::new(label)).clicked() {
            app.call.join(server, channel);
        }
        link_right = r.left() - 8.0;
    }
    let style = TextStyle::new(12.0, 600.0, 1.3);
    let small = TextStyle::new(12.0, 400.0, 1.3);
    let max = (link_right - x).max(10.0);
    let link = Rect::from_min_size(pos2(x, cy - 15.6), vec2(max, 31.2));
    let response = common::click(ui, link, Id::unique("gelabber-call-link"));
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::Role::Link, true, format!("{word}: {where_}"))
    });
    let strong = line(ui, word, style, color, max);
    strong.paint(&painter, pos2(x, cy - 15.6));
    let hover = response.hovered();
    let place = line(
        ui,
        &where_,
        small,
        if hover { c.text } else { c.muted },
        max,
    );
    place.paint(&painter, pos2(x, cy));
    if hover {
        let y = cy + small.baseline() + 1.5;
        painter.hline(x..=x + place.width(), y, Stroke::new(1.0, c.text));
    }
    if response.clicked() {
        app.nav.navigate(&format!("/s/{server}/c/{channel}"));
    }
}

/// The "Live gehen" menu (`LiveMenuItems`), above the button's end.
fn live_menu(
    ui: &mut Ui,
    app: &mut AppCtx,
    button: &egui::Response,
    state: &CallState,
    can_live: bool,
) {
    let ctx = ui.ctx().clone();
    let id = Id::unique("gelabber-live-menu-open");
    let mut open = ctx.data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
    if button.clicked() {
        open = !open;
        ctx.data_mut(|d| d.insert_temp(id, open));
        return;
    }
    if !open {
        return;
    }
    let mut p = prefs::get(&ctx);
    let sound = sound_on(&p.source_audio_share);
    let reason = (!can_live).then_some("In diesem Server darfst du nicht live gehen.");
    let shown = Popover::new("live-menu", button.rect)
        .align_end()
        .min_width(320.0)
        .show(&ctx, |ui| {
            let theme = Theme::of(ui.ctx());
            let c = theme.c;
            ui.set_width(320.0 - 18.0);
            ui.spacing_mut().item_spacing.y = 0.0;
            let mut done = false;
            if state.live {
                if menu_entry(ui, Icon::Live, "Live beenden", None, false, true) {
                    app.call.stop_live();
                    done = true;
                }
            } else if menu_entry(
                ui,
                Icon::Live,
                "Live gehen",
                Some(reason.unwrap_or("Sehen alle im Sprachkanal")),
                reason.is_some(),
                false,
            ) {
                app.call
                    .start_live(LiveSource::Screen { with_audio: sound });
                done = true;
            }
            ui.add_space(4.0);
            let top = ui.cursor().min;
            ui.painter().hline(
                top.x..=top.x + 320.0 - 18.0,
                top.y + 0.5,
                Stroke::new(1.0, c.border),
            );
            ui.add_space(1.0 + 4.0);
            // "Ton mitsenden" with its switch track.
            let hint = if p.source_audio_app.is_empty() {
                "Ton aller Anwendungen außer Gelabber".to_owned()
            } else {
                format!("Ton von „{}“", p.source_audio_app)
            };
            let width = 320.0 - 18.0;
            let title = Block::single(ui, "Ton mitsenden", text::UI, c.text);
            let small = Block::wrapped(
                ui,
                &hint,
                text::META,
                c.muted,
                width - 20.0 - 27.0 - 10.0 - 32.0,
            );
            let h = (16.0 + title.height() + 2.0 + small.height()).max(36.0);
            let (rect, response) = ui.allocate_exact_size(vec2(width, h), Sense::click());
            response.widget_info(|| {
                egui::WidgetInfo::selected(egui::Role::CheckBox, true, sound, "Ton mitsenden")
            });
            if response.hovered() {
                ui.painter()
                    .rect_filled(rect, corner(theme.radius.sm), c.hover);
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            let tx = rect.left() + 10.0 + 27.0;
            let ty = rect.center().y - (title.height() + 2.0 + small.height()) / 2.0;
            title.paint(ui.painter(), pos2(tx, ty));
            small.paint(ui.painter(), pos2(tx, ty + title.height() + 2.0));
            let track = Rect::from_min_size(
                pos2(rect.right() - 10.0 - 32.0, rect.center().y - 9.0),
                vec2(32.0, 18.0),
            );
            let (fill, border, knob) = if sound {
                (c.accent, c.accent, c.accent_ink)
            } else {
                (c.surface, c.border_strong, c.muted)
            };
            ui.painter().rect(
                track,
                corner(9.0),
                fill,
                Stroke::new(1.0, border),
                StrokeKind::Inside,
            );
            let kx = track.left() + 1.0 + 2.0 + if sound { 14.0 } else { 0.0 };
            ui.painter()
                .circle_filled(pos2(kx + 6.0, track.top() + 1.0 + 2.0 + 6.0), 6.0, knob);
            if response.clicked() {
                p.source_audio_share = if sound { "off" } else { "on" }.into();
            }
            if menu_entry(
                ui,
                Icon::Gear,
                "Anwendung für den Ton wählen …",
                None,
                false,
                false,
            ) {
                app.nav.navigate("/settings/video");
                done = true;
            }
            done
        });
    if prefs::get(&ctx) != p {
        prefs::update(&ctx, |q| *q = p.clone());
    }
    match shown {
        Some(false) => {}
        _ => {
            ctx.data_mut(|d| d.insert_temp(id, false));
        }
    }
}

/// `sharesSourceAudio` in the desktop app, whose capture leaves itself out.
fn sound_on(share: &str) -> bool {
    share != "off"
}

/// A `.voice-menu-item`: icon, title and hint; true when chosen.
fn menu_entry(
    ui: &mut Ui,
    icon: Icon,
    title: &str,
    hint: Option<&str>,
    disabled: bool,
    danger: bool,
) -> bool {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let width = ui.available_width().max(320.0 - 18.0);
    let tone = if disabled {
        c.subtle
    } else if danger {
        c.danger
    } else {
        c.text
    };
    let icon_tone = if disabled {
        c.subtle
    } else if danger {
        c.danger
    } else {
        c.muted
    };
    let t = Block::single(ui, title, text::UI, tone);
    let h_block = hint.map(|h| Block::wrapped(ui, h, text::META, c.muted, width - 20.0 - 27.0));
    let content = t.height() + h_block.as_ref().map_or(0.0, |b| 2.0 + b.height());
    let (rect, response) =
        ui.allocate_exact_size(vec2(width, (content + 16.0).max(36.0)), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, !disabled, title));
    if response.hovered() && !disabled {
        ui.painter()
            .rect_filled(rect, corner(theme.radius.sm), c.hover);
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    icons::paint(
        ui.painter(),
        icon,
        pos2(rect.left() + 10.0, rect.top() + 8.0 + 1.0),
        17.0,
        icon_tone,
    );
    t.paint(
        ui.painter(),
        pos2(rect.left() + 10.0 + 27.0, rect.top() + 8.0),
    );
    if let Some(b) = h_block {
        b.paint(
            ui.painter(),
            pos2(
                rect.left() + 10.0 + 27.0,
                rect.top() + 8.0 + t.height() + 2.0,
            ),
        );
    }
    response.clicked() && !disabled
}

/// "Im Anruf": everyone with their volume, above the avatars.
fn people_popover(ui: &mut Ui, app: &mut AppCtx, button: &egui::Response, list: &[InCall]) {
    let ctx = ui.ctx().clone();
    let id = Id::unique("gelabber-call-people-open");
    let mut open = ctx.data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
    if button.clicked() {
        open = !open;
        ctx.data_mut(|d| d.insert_temp(id, open));
        return;
    }
    if !open {
        return;
    }
    let call = app.call.clone();
    let shown = Popover::new("call-people", button.rect)
        .min_width(260.0 + 8.0 + 18.0)
        .show(&ctx, |ui| {
            let theme = Theme::of(ui.ctx());
            let c = theme.c;
            let width = 260.0;
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.add_space(4.0);
            for (i, p) in list.iter().enumerate() {
                if i > 0 {
                    ui.add_space(14.0);
                }
                let (row, _) = ui.allocate_exact_size(vec2(width + 8.0, 32.0), Sense::hover());
                let avatar =
                    Rect::from_min_size(pos2(row.left() + 4.0, row.top()), Vec2::splat(32.0));
                look::avatar(
                    ui,
                    app.images,
                    avatar,
                    &p.name,
                    p.url.as_deref(),
                    12.0,
                    c.elevated,
                    p.speaking,
                );
                let name = if p.me {
                    format!("{} (du)", p.name)
                } else {
                    p.name.clone()
                };
                let n = line(
                    ui,
                    &name,
                    text::UI.weight(600.0),
                    c.text,
                    width - 32.0 - 60.0,
                );
                paint_mid(ui.painter(), &n, avatar.right() + 8.0, row.center().y);
                if p.speaking {
                    let s = Block::single(ui, "spricht", text::META, c.online_text);
                    paint_mid(
                        ui.painter(),
                        &s,
                        row.right() - 4.0 - s.width(),
                        row.center().y,
                    );
                }
                if list.len() > 1 {
                    ui.add_space(6.0);
                }
                if !p.me {
                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(
                        Rect::from_min_size(ui.cursor().min + vec2(4.0, 0.0), vec2(width, 200.0)),
                    ));
                    volume::control(&mut child, &call, p.id);
                    let used = child.min_rect().height();
                    ui.allocate_exact_size(vec2(width, used), Sense::hover());
                }
            }
            ui.add_space(4.0);
        });
    if shown.is_none() {
        ctx.data_mut(|d| d.insert_temp(id, false));
    }
}
