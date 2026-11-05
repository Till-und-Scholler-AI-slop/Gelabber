//! `NotificationSettingsDialog` (components/NotificationSettingsDialog.tsx,
//! notificationSettings.css): "Als gelesen markieren", the mute chips or
//! the running mute, and the level of a server or channel. Every choice is
//! saved at once.

use chrono::{DateTime, Datelike, Local, Utc};
use egui::{Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2};
use gelabber_client::{
    NotificationLevel, NotificationSettingBody, OpState, ReadScope, SettingScope, SettingTarget,
};
use uuid::Uuid;

use crate::app::AppCtx;
use crate::icons::{self, Icon};
use crate::screens::settings::controls::radio_mark;
use crate::screens::shell::common::paint_mid;
use crate::text::{self, Block, TextStyle};
use crate::theme::{Theme, corner};
use crate::widgets::{Button, ButtonSize, Modal, eased, focus_ring, keyboard_focus, snap};

/// `NotificationTarget`.
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub scope: SettingScope,
    pub id: Uuid,
    /// "#allgemein", a server's or a person's name.
    pub name: String,
    /// A server channel inherits from this server.
    pub server: Option<Uuid>,
    pub dm: bool,
    /// The rail's DM button: only "mark as read".
    pub all_dms: bool,
}

impl Target {
    pub fn server(id: Uuid, name: &str) -> Self {
        Self {
            scope: SettingScope::Server,
            id,
            name: name.to_owned(),
            server: None,
            dm: false,
            all_dms: false,
        }
    }

    /// `name` is "#channel".
    pub fn channel(id: Uuid, server: Uuid, name: String) -> Self {
        Self {
            scope: SettingScope::Channel,
            id,
            name,
            server: Some(server),
            dm: false,
            all_dms: false,
        }
    }

    pub fn dm(id: Uuid, name: String) -> Self {
        Self {
            scope: SettingScope::Channel,
            id,
            name,
            server: None,
            dm: true,
            all_dms: false,
        }
    }

    pub fn all_dms() -> Self {
        Self {
            scope: SettingScope::Channel,
            id: Uuid::nil(),
            name: "Direktnachrichten".into(),
            server: None,
            dm: false,
            all_dms: true,
        }
    }
}

pub struct NotifyDialog {
    target: Target,
    /// When a save was sent whose answer is outstanding, and whether its
    /// op was seen running (an older answer may still be in the slot).
    saving: Option<(f64, bool)>,
    error: Option<&'static str>,
}

impl NotifyDialog {
    pub fn new(target: Target) -> Self {
        Self {
            target,
            saving: None,
            error: None,
        }
    }
}

const MUTES: [(&str, Option<i64>); 5] = [
    ("15 Minuten", Some(15)),
    ("1 Stunde", Some(60)),
    ("8 Stunden", Some(8 * 60)),
    ("24 Stunden", Some(24 * 60)),
    ("Bis ich es wieder einschalte", None),
];

const LEVELS: [(NotificationLevel, &str, &str); 3] = [
    (
        NotificationLevel::All,
        "Alle Nachrichten",
        "Jede neue Nachricht",
    ),
    (
        NotificationLevel::Mentions,
        "Nur @Erwähnungen",
        "Wenn dich jemand erwähnt oder @everyone/@here schreibt",
    ),
    (
        NotificationLevel::None,
        "Nichts",
        "Keine Benachrichtigungen, kein Zähler",
    ),
];

fn level_name(level: NotificationLevel) -> &'static str {
    LEVELS
        .iter()
        .find(|(l, _, _)| *l == level)
        .map_or("Alle Nachrichten", |(_, n, _)| n)
}

/// `text-sm font-semibold`: 600 14px/20px.
const HEAD: TextStyle = TextStyle::new(14.0, 600.0, 20.0 / 14.0);

#[derive(Clone, Copy, Default)]
struct Setting {
    level: Option<NotificationLevel>,
    muted: bool,
    until: Option<DateTime<Utc>>,
}

impl Setting {
    fn is_muted(&self, now: DateTime<Utc>) -> bool {
        self.muted && self.until.is_none_or(|u| u > now)
    }
}

fn setting_of(s: &gelabber_client::State, scope: SettingScope, id: Uuid) -> Setting {
    s.notification_settings
        .iter()
        .find(|r| r.scope == scope && r.id == id)
        .map(|r| Setting {
            level: r.level,
            muted: r.muted,
            until: r.muted_until,
        })
        .unwrap_or_default()
}

pub fn show(ui: &mut Ui, app: &mut AppCtx, d: &mut NotifyDialog) -> bool {
    let ctx = ui.ctx().clone();
    let theme = Theme::of(&ctx);
    let c = theme.c;
    let now = app.client.http().server_now();
    let target = d.target.clone();
    let (own, server, op) = app.client.with_state(|s| {
        (
            setting_of(s, target.scope, target.id),
            target
                .server
                .map(|id| setting_of(s, SettingScope::Server, id)),
            s.op("notifications").cloned(),
        )
    });
    let time = ctx.input(|i| i.time);
    if let Some((since, seen)) = d.saving.as_mut() {
        let stale = time - *since > 1.5;
        match op {
            Some(OpState::Pending) => *seen = true,
            Some(OpState::Failed(e)) if *seen => {
                d.saving = None;
                d.error = Some(if e.status() == Some(404) {
                    "Nicht mehr verfügbar."
                } else {
                    "Nicht gespeichert. Versuch es noch einmal."
                });
            }
            _ if *seen || stale => d.saving = None,
            _ => ctx.request_repaint(),
        }
    }
    let saving = d.saving.is_some();
    let muted = own.is_muted(now);
    let server_muted = server.is_some_and(|s| s.is_muted(now));
    let inherited = server.and_then(|s| s.level).unwrap_or_default();
    let mut save: Option<NotificationSettingBody> = None;
    let mut done = false;
    let mut mark = false;
    let first = crate::widgets::field::mounted(&ctx, egui::Id::unique("gelabber-notify-open"));
    let title = format!("Benachrichtigungen: {}", target.name);
    let output = Modal::new("notification-settings", &title).show(
        &ctx,
        |ui| {
            ui.spacing_mut().item_spacing.y = 20.0;
            let label = if target.all_dms {
                "Alle Direktnachrichten als gelesen markieren"
            } else if target.scope == SettingScope::Server {
                "Server als gelesen markieren"
            } else {
                "Als gelesen markieren"
            };
            super::row(ui, 32.0, 8.0, |ui| {
                if ui.add(Button::new(label).size(ButtonSize::Sm)).clicked() {
                    mark = true;
                }
            });
            if !target.all_dms {
                ui.scope(|ui| {
                    ui.spacing_mut().item_spacing.y = 8.0;
                    text::label(ui, "Stummschalten", HEAD, c.text);
                    if muted {
                        let text = match own.until {
                            Some(until) => format!("Stumm bis {}", format_until(until)),
                            None => "Stumm, bis du es aufhebst".to_owned(),
                        };
                        if muted_box(ui, &text, saving) {
                            save = Some(NotificationSettingBody {
                                level: own.level,
                                muted: false,
                                mute_minutes: None,
                            });
                        }
                    } else {
                        let width = ui.available_width();
                        let half = (width - 8.0) / 2.0;
                        for pair in MUTES.chunks(2) {
                            super::row(ui, 36.0, 8.0, |ui| {
                                let w = if pair.len() == 1 { width } else { half };
                                for (label, minutes) in pair {
                                    if chip(ui, label, w, saving).clicked() {
                                        save = Some(NotificationSettingBody {
                                            level: own.level,
                                            muted: true,
                                            mute_minutes: *minutes,
                                        });
                                    }
                                }
                            });
                        }
                    }
                    let mut hint = if target.dm {
                        "Stumm: kein Zähler, keine Benachrichtigung und kein Ton.".to_owned()
                    } else {
                        "Stumm: kein Punkt, keine Benachrichtigung und kein Ton. @Erwähnungen zählen weiter."
                            .to_owned()
                    };
                    if server_muted {
                        hint.push_str(" Der ganze Server ist gerade stumm.");
                    }
                    let w = ui.available_width();
                    let block = Block::wrapped(ui, &hint, text::META, c.muted, w);
                    text::show(ui, &block);
                });
                if !target.dm {
                    ui.scope(|ui| {
                        ui.spacing_mut().item_spacing.y = 8.0;
                        // The legend's own 4px margin before the first card.
                        let legend = Block::single(ui, "Benachrichtigen bei", HEAD, c.text);
                        let (r, _) = ui.allocate_exact_size(
                            Vec2::new(ui.available_width(), legend.height() + 4.0 - 8.0),
                            Sense::hover(),
                        );
                        legend.paint(ui.painter(), r.min);
                        let keep = |level: Option<NotificationLevel>| NotificationSettingBody {
                            level,
                            muted: own.muted,
                            mute_minutes: (own.muted)
                                .then_some(own.until)
                                .flatten()
                                .map(|u| ((u - now).num_seconds() as f64 / 60.0).ceil().max(1.0) as i64),
                        };
                        if target.server.is_some() {
                            let label = format!("Wie der Server ({})", level_name(inherited));
                            if radio_card(ui, &label, None, own.level.is_none(), saving) {
                                save = Some(NotificationSettingBody {
                                    level: None,
                                    muted: own.muted,
                                    mute_minutes: None,
                                });
                            }
                        }
                        for (level, label, hint) in LEVELS {
                            let checked = if target.server.is_some() {
                                own.level == Some(level)
                            } else {
                                own.level.unwrap_or_default() == level
                            };
                            if radio_card(ui, label, Some(hint), checked, saving) {
                                // A server at "all" needs no row of its own.
                                let sent = if target.server.is_none()
                                    && level == NotificationLevel::All
                                {
                                    None
                                } else {
                                    Some(level)
                                };
                                save = Some(keep(sent));
                            }
                        }
                    });
                }
                if let Some(error) = d.error {
                    let w = ui.available_width();
                    let block = Block::wrapped(ui, error, text::META, c.danger, w);
                    text::show(ui, &block);
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                let fertig = ui.add(Button::new("Fertig").ghost());
                if first {
                    fertig.request_focus();
                }
                if fertig.clicked() {
                    done = true;
                }
            });
        },
        None,
    );
    if mark {
        if target.all_dms {
            app.client.mark_all_read(ReadScope::Dms);
        } else if target.scope == SettingScope::Server {
            app.client.mark_server_read(target.id);
        } else {
            app.client.mark_read(target.id);
        }
        return false;
    }
    if let Some(body) = save {
        let to = match target.scope {
            SettingScope::Server => SettingTarget::Server(target.id),
            SettingScope::Channel => SettingTarget::Channel(target.id),
        };
        app.client.set_notifications(to, body);
        d.saving = Some((time, false));
        d.error = None;
    }
    !(output.close_requested || done)
}

/// `.notify-mute-grid .ui-chip`: 36 high, radius 5, centred label.
fn chip(ui: &mut Ui, label: &str, width: f32, disabled: bool) -> egui::Response {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let color = if disabled { c.subtle } else { c.text };
    let block = Block::single(ui, label, text::UI, color);
    let sense = if disabled {
        Sense::hover()
    } else {
        Sense::click()
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 36.0), sense);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, !disabled, label));
    let rect = snap(ui.ctx(), rect);
    let fill = eased(
        ui.ctx(),
        response.id.with("hover"),
        response.hovered() && !disabled,
        egui::Color32::TRANSPARENT,
        c.hover,
    );
    if response.has_focus() && keyboard_focus(ui.ctx()) {
        focus_ring(ui.painter(), rect, theme.radius.sm, &theme);
    }
    ui.painter().rect(
        rect,
        corner(theme.radius.sm),
        fill,
        Stroke::new(1.0, c.border_strong),
        StrokeKind::Inside,
    );
    paint_mid(
        ui.painter(),
        &block,
        rect.center().x - block.width() / 2.0,
        rect.center().y,
    );
    if response.hovered() && !disabled {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response
}

/// `.notify-muted`: the bell-off line and "Aufheben" on the surface fill.
fn muted_box(ui: &mut Ui, text: &str, disabled: bool) -> bool {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let width = ui.available_width();
    let block = Block::single(ui, text, text::UI, c.text);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 16.0 + 32.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, corner(theme.radius.md), c.surface);
    icons::paint(
        ui.painter(),
        Icon::BellOff,
        pos2(rect.left() + 12.0, rect.center().y - 8.0),
        16.0,
        c.text,
    );
    paint_mid(
        ui.painter(),
        &block,
        rect.left() + 12.0 + 24.0,
        rect.center().y,
    );
    let label = Block::single(ui, "Aufheben", text::UI, c.text);
    let bw = label.width() + 26.0;
    let button = Rect::from_min_size(
        pos2(rect.right() - 12.0 - bw, rect.center().y - 16.0),
        Vec2::new(bw, 32.0),
    );
    ui.put(
        button,
        Button::new("Aufheben")
            .size(ButtonSize::Sm)
            .disabled(disabled),
    )
    .clicked()
}

/// A `RadioCard` whose description is optional.
fn radio_card(ui: &mut Ui, label: &str, desc: Option<&str>, checked: bool, disabled: bool) -> bool {
    let theme = Theme::of(ui.ctx());
    let c = &theme.c;
    let width = ui.available_width();
    let text_w = width - 2.0 - 24.0 - 20.0 - 12.0;
    let l = Block::wrapped(ui, label, text::UI, c.text, text_w);
    let d = desc.map(|d| Block::wrapped(ui, d, text::META, c.muted, text_w));
    let text_h = l.height() + d.as_ref().map_or(0.0, |d| 2.0 + d.height());
    // The radio sits 1px down; the row is as tall as the taller column.
    let h = 2.0 + 24.0 + text_h.max(21.0);
    let sense = if disabled {
        Sense::hover()
    } else {
        Sense::click()
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, h.max(44.0)), sense);
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::RadioButton, !disabled, checked, label)
    });
    let hover = eased(
        ui.ctx(),
        response.id.with("hover"),
        response.hovered() && !disabled,
        c.panel,
        c.hover,
    );
    let (fill, border) = if checked {
        (c.selected, c.accent)
    } else {
        (hover, c.border_strong)
    };
    let alpha = if disabled { 0.6 } else { 1.0 };
    let painter = ui.painter().clone();
    let shown = snap(ui.ctx(), rect);
    if response.has_focus() && keyboard_focus(ui.ctx()) {
        focus_ring(&painter, shown, theme.radius.md, &theme);
    }
    painter.rect(
        shown,
        corner(theme.radius.md),
        fill.gamma_multiply(alpha),
        Stroke::new(1.0, border.gamma_multiply(alpha)),
        StrokeKind::Inside,
    );
    radio_mark(ui, pos2(rect.left() + 13.0, rect.top() + 14.0), checked);
    l.paint(&painter, pos2(rect.left() + 45.0, rect.top() + 13.0));
    if let Some(d) = d {
        d.paint(
            &painter,
            pos2(rect.left() + 45.0, rect.top() + 13.0 + l.height() + 2.0),
        );
    }
    if response.hovered() && !disabled {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response.clicked() && !checked
}

const WEEKDAYS: [&str; 7] = ["Mo.", "Di.", "Mi.", "Do.", "Fr.", "Sa.", "So."];
const MONTHS: [&str; 12] = [
    "Jan.", "Feb.", "März", "Apr.", "Mai", "Juni", "Juli", "Aug.", "Sept.", "Okt.", "Nov.", "Dez.",
];

/// `formatUntil`: "14:30 Uhr" today, else "Mi., 5. Nov., 14:30 Uhr".
fn format_until(until: DateTime<Utc>) -> String {
    let local = until.with_timezone(&Local);
    let today = Local::now().date_naive();
    let time = local.format("%H:%M");
    if local.date_naive() == today {
        format!("{time} Uhr")
    } else {
        format!(
            "{}, {}. {}, {time} Uhr",
            WEEKDAYS[local.weekday().num_days_from_monday() as usize],
            local.day(),
            MONTHS[local.month0() as usize]
        )
    }
}
