//! `NotificationSettingsDialog` (components/NotificationSettingsDialog.tsx,
//! notificationSettings.css): the row of mute chips or the running mute,
//! the level of a server or channel, and "Als gelesen markieren" beside
//! "Fertig". Every choice is saved at once. The row menus (row_menu.rs)
//! offer the same choices and share what is here.

use chrono::{DateTime, Datelike, Local, Utc};
use egui::{Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2};
use gelabber_client::{
    NotificationLevel, NotificationSettingBody, OpState, ReadScope, SettingScope, SettingTarget,
};
use gelabber_i18n::{Locale, Text, intl, notifications as texts, shell};
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
    /// Every conversation (the rail's home tile, whose menu only marks
    /// them read).
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
            name: shell::rail_dms().into(),
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
    /// The setting when that save was sent: a changed one is its answer,
    /// even when the op ran between two frames.
    before: Setting,
    error: Option<&'static str>,
    /// `notifications` failures when that save was sent: a refusal can
    /// fail the op between two frames, before it is ever seen running.
    failures: u64,
}

impl NotifyDialog {
    pub fn new(target: Target) -> Self {
        Self {
            target,
            saving: None,
            before: Setting::default(),
            error: None,
            failures: 0,
        }
    }
}

/// Why a save did not go through, under the dialog's choices.
fn failure_text(e: &gelabber_client::ApiError) -> &'static str {
    if matches!(e, gelabber_client::ApiError::Api { code, .. } if code == "unknown_level") {
        // The client refused it: the stored level is newer than this app.
        shell::notify_unknown_level()
    } else if e.status() == Some(404) {
        texts::dialog_gone()
    } else {
        texts::dialog_not_saved()
    }
}

/// `MUTES` of notificationActions.ts: the length, its short form for the
/// dialog's row of chips, and the minutes (none: until turned back on).
pub(crate) const MUTES: [(Text, Text, Option<i64>); 5] = [
    (
        texts::mute_quarter_hour,
        texts::mute_quarter_hour_short,
        Some(15),
    ),
    (texts::mute_hour, texts::mute_hour_short, Some(60)),
    (
        texts::mute_eight_hours,
        texts::mute_eight_hours_short,
        Some(8 * 60),
    ),
    (texts::mute_day, texts::mute_day_short, Some(24 * 60)),
    (texts::mute_forever, texts::mute_forever_short, None),
];

pub(crate) const LEVELS: [(NotificationLevel, Text, Text); 3] = [
    (
        NotificationLevel::All,
        texts::level_all,
        texts::level_all_hint,
    ),
    (
        NotificationLevel::Mentions,
        texts::level_mentions,
        texts::level_mentions_hint,
    ),
    (
        NotificationLevel::None,
        texts::level_none,
        texts::level_none_hint,
    ),
];

pub(crate) fn level_name(level: NotificationLevel) -> &'static str {
    LEVELS
        .iter()
        .find(|(l, _, _)| *l == level)
        .map_or_else(texts::level_all, |(_, n, _)| n())
}

/// `text-sm font-semibold`: 600 14px/20px.
const HEAD: TextStyle = TextStyle::new(14.0, 600.0, 20.0 / 14.0);

/// The own setting of a server or conversation; inheriting when absent.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Setting {
    pub level: Option<NotificationLevel>,
    pub muted: bool,
    pub until: Option<DateTime<Utc>>,
}

impl Setting {
    pub fn is_muted(&self, now: DateTime<Utc>) -> bool {
        self.muted && self.until.is_none_or(|u| u > now)
    }

    /// `muteState` of a save: muted for this long, or until turned back on.
    pub fn mute(&self, minutes: Option<i64>) -> NotificationSettingBody {
        NotificationSettingBody {
            level: self.level,
            muted: true,
            mute_minutes: minutes,
        }
    }

    pub fn unmute(&self) -> NotificationSettingBody {
        NotificationSettingBody {
            level: self.level,
            muted: false,
            mute_minutes: None,
        }
    }

    /// A new level (`None`: as the server) keeps a running mute and its
    /// end. A server at "all" needs no row of its own.
    pub fn with_level(
        &self,
        level: Option<NotificationLevel>,
        server: bool,
        now: DateTime<Utc>,
    ) -> NotificationSettingBody {
        NotificationSettingBody {
            level: if server && level == Some(NotificationLevel::All) {
                None
            } else {
                level
            },
            muted: self.muted,
            mute_minutes: self
                .muted
                .then_some(self.until)
                .flatten()
                .map(|u| ((u - now).num_seconds() as f64 / 60.0).ceil().max(1.0) as i64),
        }
    }
}

/// "Stumm bis 14:30 Uhr" or "Stumm, bis du es aufhebst".
pub(crate) fn muted_line(until: Option<DateTime<Utc>>) -> String {
    match until {
        Some(until) => texts::muted_until(&format_until(until)),
        None => texts::muted_until_unmuted().to_owned(),
    }
}

/// Marks the target read: a conversation, a server or every DM. A failure
/// is a toast (op keys `read:<id>`, `read_all`).
pub(crate) fn mark_read(app: &AppCtx, target: &Target) {
    if target.all_dms {
        app.client.mark_all_read(ReadScope::Dms);
    } else if target.scope == SettingScope::Server {
        app.client.mark_server_read(target.id);
    } else {
        app.client.mark_conversation_read(target.id);
    }
}

/// Whether the target has anything unread to mark read, muted or not.
pub(crate) fn has_unread(s: &gelabber_client::State, target: &Target) -> bool {
    s.read.values().any(|r| {
        r.unread_count > 0
            && if target.all_dms {
                r.server_id.is_none()
            } else if target.scope == SettingScope::Server {
                r.server_id == Some(target.id)
            } else {
                r.channel_id == target.id
            }
    })
}

pub(crate) fn setting_target(target: &Target) -> SettingTarget {
    match target.scope {
        SettingScope::Server => SettingTarget::Server(target.id),
        SettingScope::Channel => SettingTarget::Channel(target.id),
    }
}

pub(crate) fn setting_of(s: &gelabber_client::State, scope: SettingScope, id: Uuid) -> Setting {
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
    let (own, server, op, failures) = app.client.with_state(|s| {
        (
            setting_of(s, target.scope, target.id),
            target
                .server
                .map(|id| setting_of(s, SettingScope::Server, id)),
            s.op("notifications").cloned(),
            s.failures("notifications"),
        )
    });
    let time = ctx.input(|i| i.time);
    if let Some((since, seen)) = d.saving.as_mut() {
        let stale = time - *since > 1.5;
        match op {
            Some(OpState::Failed(e)) if *seen || failures != d.failures => {
                d.saving = None;
                d.error = Some(failure_text(&e));
            }
            Some(OpState::Pending) => *seen = true,
            _ if *seen || stale || own != d.before => d.saving = None,
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
    let title = texts::dialog_title_for(&target.name);
    let output = Modal::new("notification-settings", &title).show(
        &ctx,
        |ui| {
            ui.spacing_mut().item_spacing.y = 20.0;
            ui.scope(|ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                text::label(ui, texts::dialog_mute(), HEAD, c.text);
                if muted {
                    if muted_box(ui, &muted_line(own.until), saving) {
                        save = Some(own.unmute());
                    }
                } else {
                    // `.notify-mute-row`: one row of chips that share
                    // its width (`flex: 1 0 auto`).
                    let gap = 8.0;
                    let natural: Vec<f32> = MUTES
                        .iter()
                        .map(|(_, short, _)| {
                            Block::single(ui, short(), text::UI, c.text).width() + 26.0
                        })
                        .collect();
                    let total = natural.iter().sum::<f32>() + gap * (natural.len() - 1) as f32;
                    let extra = ((ui.available_width() - total) / natural.len() as f32).max(0.0);
                    super::row(ui, 32.0, gap, |ui| {
                        for ((label, short, minutes), w) in MUTES.iter().zip(&natural) {
                            if chip(ui, short(), label(), w + extra, saving).clicked() {
                                save = Some(own.mute(*minutes));
                            }
                        }
                    });
                }
                let mut hint = if target.dm {
                    texts::dialog_mute_hint_dm().to_owned()
                } else {
                    texts::dialog_mute_hint_channel().to_owned()
                };
                if server_muted {
                    hint.push(' ');
                    hint.push_str(texts::dialog_server_muted());
                }
                let w = ui.available_width();
                let block = Block::wrapped(ui, &hint, text::META, c.muted, w);
                text::show(ui, &block);
            });
            if !target.dm {
                ui.scope(|ui| {
                    ui.spacing_mut().item_spacing.y = 8.0;
                    // The legend's own 4px margin before the first card.
                    let legend = Block::single(ui, texts::dialog_level_legend(), HEAD, c.text);
                    let (r, _) = ui.allocate_exact_size(
                        Vec2::new(ui.available_width(), legend.height() + 4.0 - 8.0),
                        Sense::hover(),
                    );
                    legend.paint(ui.painter(), r.min);
                    let server_scope = target.server.is_none();
                    if target.server.is_some() {
                        let label = texts::level_inherit(level_name(inherited));
                        if radio_card(ui, &label, None, own.level.is_none(), saving) {
                            save = Some(own.with_level(None, false, now));
                        }
                    }
                    for (level, label, hint) in LEVELS {
                        let checked = if target.server.is_some() {
                            own.level == Some(level)
                        } else {
                            own.level.unwrap_or_default() == level
                        };
                        if radio_card(ui, label(), Some(hint()), checked, saving) {
                            save = Some(own.with_level(Some(level), server_scope, now));
                        }
                    }
                });
            }
            if let Some(error) = d.error {
                let w = ui.available_width();
                let block = Block::wrapped(ui, error, text::META, c.danger, w);
                let shown = text::show(ui, &block);
                crate::widgets::announce(&shown, egui::Role::Alert, error);
            }
            // `.notify-footer`: marking read is an action, beside "Fertig".
            super::row(ui, 36.0, 8.0, |ui| {
                if ui
                    .add(Button::new(texts::dialog_mark_read()).size(ButtonSize::Sm))
                    .clicked()
                {
                    mark = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let fertig = ui.add(Button::new(gelabber_i18n::common::done()).ghost());
                    if first {
                        fertig.request_focus();
                    }
                    if fertig.clicked() {
                        done = true;
                    }
                });
            });
        },
        None,
    );
    if mark {
        mark_read(app, &target);
        return false;
    }
    if let Some(body) = save {
        app.client.set_notifications(setting_target(&target), body);
        d.saving = Some((time, false));
        d.before = own;
        d.error = None;
        d.failures = failures;
    }
    !(output.close_requested || done)
}

/// `.notify-mute-row .ui-chip`: a 32px pill, centred label; `title` the
/// whole length, as its hover text and its name (`aria-label`).
fn chip(ui: &mut Ui, label: &str, title: &str, width: f32, disabled: bool) -> egui::Response {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let color = if disabled { c.subtle } else { c.text };
    let block = Block::single(ui, label, text::UI, color);
    let sense = if disabled {
        Sense::hover()
    } else {
        Sense::click()
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 32.0), sense);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, !disabled, title));
    let response = response.on_hover_text(title);
    let rect = snap(ui.ctx(), rect);
    let fill = eased(
        ui.ctx(),
        response.id.with("hover"),
        response.hovered() && !disabled,
        egui::Color32::TRANSPARENT,
        c.hover,
    );
    if response.has_focus() && keyboard_focus(ui.ctx()) {
        focus_ring(ui.painter(), rect, theme.radius.full, &theme);
    }
    ui.painter().rect(
        rect,
        corner(theme.radius.full),
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
    let label = Block::single(ui, texts::dialog_unmute(), text::UI, c.text);
    let bw = label.width() + 26.0;
    let button = Rect::from_min_size(
        pos2(rect.right() - 12.0 - bw, rect.center().y - 16.0),
        Vec2::new(bw, 32.0),
    );
    // In a child: `put` would move the cursor to under the button, not
    // under the box.
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(button));
    child
        .add(
            Button::new(texts::dialog_unmute())
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

/// `formatUntil`: "14:30 Uhr" today, else "Mi., 5. Nov., 14:30 Uhr"
/// ("14:30", "Wed 5 Nov, 14:30" in English).
fn format_until(until: DateTime<Utc>) -> String {
    let local = until.with_timezone(&Local);
    let today = Local::now().date_naive();
    let time = local.format("%H:%M").to_string();
    if local.date_naive() == today {
        texts::until_today(&time)
    } else {
        let weekday = intl::weekday_name(local.weekday().num_days_from_monday(), true);
        let month = intl::month_name(local.month(), true);
        let day = local.day();
        // `weekday: short, day: numeric, month: short` in de-DE / en-GB.
        let date = match gelabber_i18n::get() {
            Locale::De => format!("{weekday}, {day}. {month}"),
            Locale::En => format!("{weekday} {day} {month}"),
        };
        texts::until_later(&date, &time)
    }
}

#[cfg(test)]
mod tests {
    use crate::app::testkit::{
        ALLGEMEIN, FakeApi, HOME, Reply, launcher, wait_for_label, wait_for_text, workspace,
    };
    use crate::platform::paths::testing::TempConfig;
    use egui_kittest::kittest::Queryable;

    /// A level a newer server stored is never saved back: the client
    /// refuses at once, and the dialog says why instead of going quiet.
    #[test]
    fn a_refused_save_says_why() {
        let _config = TempConfig::new("notify-unknown-level");
        let api = FakeApi::start(true, |method, path, _| match (method, path) {
            ("GET", "/notification-settings") => Some(Reply::json(serde_json::json!([{
                "scope": "channel", "id": ALLGEMEIN, "level": "digest",
                "muted": false, "muted_until": null
            }]))),
            _ => workspace(method, path),
        });
        let mut harness = launcher(&api, &format!("/s/{HOME}/c/{ALLGEMEIN}"));
        wait_for_label(&mut harness, "Benachrichtigungen für #allgemein");
        harness
            .get_by_label("Benachrichtigungen für #allgemein")
            .click();
        wait_for_label(&mut harness, "15 Minuten");
        harness.get_by_label("15 Minuten").click();
        wait_for_text(
            &mut harness,
            "Diese Stufe kennt diese Version von Gelabber nicht.",
        );
        assert!(api.seen.lock().iter().all(|r| !r.starts_with("PUT")));
    }
}
