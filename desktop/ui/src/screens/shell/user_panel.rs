//! The user panel at the bottom of the sidebar column
//! (components/UserPanel.tsx, index.css `.user-panel`): your avatar with
//! presence, name and status line, and the settings gear. A click on the
//! name opens the account popover (#570): the status card, then the
//! presence modes (#590) and the account's entries as a menu.

use egui::{Id, Rect, Ui, Vec2, pos2};
use gelabber_client::{
    ModeDuration, ModeState, OpState, PresenceMode, PresenceStatus, ProfilePatch,
};
use gelabber_i18n::shell;
use uuid::Uuid;

use super::common::{self, line, paint_mid};
use crate::app::AppCtx;
use crate::icons::{self, Icon};
use crate::text::{self, Block};
use crate::theme::Theme;
use crate::widgets::{Button, InlineStatus, Popover, StatusTone, TextField, hairline_h};

/// `STATUS_TEXT_MAX` (auth/rules.ts).
const STATUS_MAX: usize = 80;

/// The status card's draft while it is open.
#[derive(Clone, Default)]
struct StatusDraft {
    open: bool,
    text: String,
    saved: Option<bool>,
    saving: bool,
    /// Focus the field on the next frame (the popover's initial focus).
    focus: bool,
    /// The mode whose durations show below its row (`asking`).
    asking: Option<PresenceMode>,
}

fn draft_id() -> Id {
    Id::unique("gelabber-user-status")
}

/// Another account signed in: the status card's draft goes.
pub(super) fn switch_account(ctx: &egui::Context) {
    ctx.data_mut(|d| d.remove::<StatusDraft>(draft_id()));
}

/// `rect` is the 56px strip at the bottom of the sidebar column; `context`
/// the server or DM whose presence counts (`ownPresence`). A rect as narrow
/// as the rail is the folded channel list's (GEL-34, `.is-nav-collapsed
/// .user-panel`): the avatar at the rail's foot, the gear above it, the
/// name only for screen readers; the shell paints the rail behind it.
pub fn show(ui: &mut Ui, app: &mut AppCtx, rect: Rect, context: Option<Uuid>) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let painter = ui.painter().clone();
    let compact = rect.width() <= crate::screens::layout::RAIL_WIDTH;
    if !compact {
        // `.user-panel` sits inside `.channel-sidebar`: the column's right
        // border (the shell's) runs on past it down to the window's edge.
        let inner = Rect::from_min_max(rect.min, pos2(rect.right() - 1.0, rect.bottom()));
        painter.rect_filled(inner, 0.0, c.panel);
        hairline_h(&painter, &theme, inner.left()..=inner.right(), inner.top());
    }

    let (name, avatar_url, status_text, presence, mode) = app.client.with_state(|s| {
        let me = s.me();
        let id = me.map(|m| m.id);
        let presence = id.map_or(PresenceStatus::Online, |id| {
            context
                .and_then(|ctx| s.presence.get(&ctx).and_then(|m| m.get(&id)).copied())
                .or_else(|| s.presence.values().find_map(|m| m.get(&id).copied()))
                .unwrap_or(PresenceStatus::Online)
        });
        (
            me.map(|m| m.name.clone()),
            me.and_then(|m| m.avatar_url.clone()),
            me.and_then(|m| m.status_text.clone()),
            presence,
            s.presence_mode,
        )
    });
    // Before the client knows the account, the session's user stands in.
    let session = app.session.user();
    let name = name
        .or_else(|| {
            session
                .and_then(|u| u.get("name"))
                .and_then(|v| v.as_str())
                .map(str::to_owned)
        })
        .unwrap_or_default();
    let avatar_url = avatar_url.or_else(|| {
        session
            .and_then(|u| u.get("avatar_url"))
            .and_then(|v| v.as_str())
            .map(str::to_owned)
    });
    // The mode wins over what the servers report (`ownShown`).
    let mode = ModeState {
        mode: mode.at(chrono::Utc::now()),
        ..mode
    };
    let presence = common::own_presence(mode.mode, presence);
    let label = presence.label();
    let subtitle = match status_text.as_deref().filter(|t| !t.is_empty()) {
        Some(text) => format!("{label} · {text}"),
        None => label.to_owned(),
    };

    let inner_top = rect.top() + 1.0;
    let center_y = inner_top + (rect.bottom() - inner_top) / 2.0;
    let right = rect.right() - 1.0 - 8.0;
    let (gear, profile) = if compact {
        // Column, from the bottom: 10, the 40px avatar button, 6, the gear.
        let mid = rect.left() + (rect.width() - 1.0) / 2.0;
        let profile =
            Rect::from_center_size(pos2(mid, rect.bottom() - 10.0 - 20.0), Vec2::splat(40.0));
        let gear =
            Rect::from_center_size(pos2(mid, profile.top() - 6.0 - 18.0), Vec2::new(30.0, 36.0));
        (gear, profile)
    } else {
        let gear = Rect::from_min_size(pos2(right - 30.0, center_y - 18.0), Vec2::new(30.0, 36.0));
        let profile = Rect::from_min_max(
            pos2(rect.left() + 8.0, center_y - 21.75),
            pos2(gear.left() - 4.0, center_y + 21.75),
        );
        (gear, profile)
    };
    // Feedback (#542): the entry while the instance takes it, and the dot
    // on the avatar and the entry while a reply is unread.
    let feedback = crate::screens::feedback::enabled(ui.ctx(), app) == Some(true);
    let unread = feedback && crate::screens::feedback::unread(ui.ctx(), app) > 0;

    let mut draft: StatusDraft = ui.data(|d| d.get_temp(draft_id())).unwrap_or_default();
    let response = common::click(ui, profile, profile_id());
    // The unread reply is part of the button's name (the web's sr-only).
    let profile_label = if unread {
        format!("{name}, {}", shell::user_panel_feedback_unread())
    } else {
        name.clone()
    };
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &profile_label));
    common::focus_visible(ui, &response, profile, theme.radius.md);
    if response.hovered() || draft.open {
        common::fill(&painter, profile, theme.radius.md, c.hover);
    }
    let avatar = Rect::from_min_size(
        pos2(profile.left() + 2.0, profile.center().y - 18.0),
        Vec2::splat(36.0),
    );
    let under = if response.hovered() || draft.open {
        c.hover
    } else if compact {
        c.rail
    } else {
        c.panel
    };
    common::avatar(
        ui,
        app.images,
        avatar,
        &name,
        avatar_url.as_deref(),
        14.0,
        1.0,
        under,
    );
    common::presence_dot(&painter, &theme, avatar, presence);
    if unread {
        // `.user-panel-avatar > .unread-dot`: top and right -1px, with a
        // 2px ring in the panel's colour.
        let center = pos2(avatar.right() + 1.0 - 4.0, avatar.top() - 1.0 + 4.0);
        painter.circle_filled(center, 6.0, under);
        crate::screens::feedback::dialog::dot(&painter, center, c.accent);
    }
    if !compact {
        let text_left = avatar.right() + 9.0;
        let text_max = profile.right() - 4.0 - text_left;
        let top = center_y - 19.75;
        let strong = line(ui, &name, text::BODY.weight(600.0), c.text, text_max);
        strong.paint(&painter, pos2(text_left, top));
        let small = line(
            ui,
            &subtitle,
            text::META.size(11.0).line_height(1.5),
            c.muted,
            text_max,
        );
        small.paint(&painter, pos2(text_left, top + 21.0 + 2.0));
    }
    if response.clicked() {
        draft.open = !draft.open;
        draft.text = status_text.clone().unwrap_or_default();
        draft.saved = None;
        draft.focus = draft.open;
        draft.asking = None;
    }

    // Always Settings, in a call too: the call's own settings are in the
    // voice room and in Settings → Audio (#570).
    let gear_response = icon_button(ui, gear, Icon::Gear, 21.0, shell::user_panel_settings());
    if gear_response.clicked() {
        app.nav.navigate("/settings/darstellung");
    }

    if draft.open {
        let current = status_text.clone().unwrap_or_default();
        account_popover(
            ui,
            app,
            profile,
            &name,
            avatar_url.as_deref(),
            presence,
            mode,
            &current,
            &mut draft,
            Entries { feedback, unread },
        );
    }
    ui.data_mut(|d| d.insert_temp(draft_id(), draft));
}

/// `.shell-icon-button` in the panel: 30×36, muted, hover fill.
fn icon_button(ui: &mut Ui, rect: Rect, icon: Icon, size: f32, label: &str) -> egui::Response {
    let theme = Theme::of(ui.ctx());
    let response = common::click(ui, rect, icon_id(icon)).on_hover_text(label);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    common::focus_visible(ui, &response, rect, theme.radius.md);
    let color = if response.hovered() {
        common::fill(ui.painter(), rect, theme.radius.md, theme.c.hover);
        theme.c.text
    } else {
        theme.c.muted
    };
    let min = rect.center() - Vec2::splat(size / 2.0);
    icons::paint(ui.painter(), icon, min, size, color);
    response
}

/// The panel's icon buttons keep their id while their names change.
fn icon_id(icon: Icon) -> Id {
    Id::unique(("user-panel", icon))
}

/// The avatar and name: the account popover's button, and where the focus
/// goes back to.
fn profile_id() -> Id {
    Id::unique("user-panel-profile")
}

/// What an entry of the account menu does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum MenuItem {
    Profile,
    /// While the instance takes feedback: the "Feedback geben" dialog.
    Feedback,
    /// Operators only: the operator area, in the system browser.
    Operator,
    SwitchServer,
    SignOut,
}

/// The account menu's entries, in the web's order (`AccountMenu`):
/// "Profil bearbeiten", "Feedback geben" while the instance takes feedback,
/// "Betrieb" for operators, the app's own "Server wechseln …", and after a
/// line "Abmelden". No "Einstellungen": the gear is beside the avatar.
fn menu_items(operator: bool, feedback: bool) -> Vec<(MenuItem, &'static str)> {
    let mut items = vec![(MenuItem::Profile, shell::user_panel_edit_profile())];
    if feedback {
        items.push((MenuItem::Feedback, shell::user_panel_feedback()));
    }
    if operator {
        items.push((MenuItem::Operator, shell::user_panel_operator()));
    }
    // "Server wechseln …" is the desktop app's own (isDesktopApp).
    items.push((MenuItem::SwitchServer, shell::user_panel_switch_server()));
    items.push((MenuItem::SignOut, shell::user_panel_sign_out()));
    items
}

fn item_id(item: MenuItem) -> Id {
    Id::unique(("user-account-item", item))
}

/// The entry to focus after `key` on entry `at` of `count` (`accountStep`,
/// components/accountKeys.ts): ↓ and ↑ round the ends, Home and End jump,
/// ↑ on the first entry goes back to the status field (`Some(None)`).
fn account_step(key: egui::Key, at: usize, count: usize) -> Option<Option<usize>> {
    match key {
        egui::Key::ArrowDown => Some(Some((at + 1) % count)),
        egui::Key::ArrowUp => Some(at.checked_sub(1)),
        egui::Key::Home => Some(Some(0)),
        egui::Key::End => Some(Some(count - 1)),
        _ => None,
    }
}

/// What the account popover shows besides the card.
#[derive(Clone, Copy)]
struct Entries {
    feedback: bool,
    unread: bool,
}

/// `hr.shell-menu-separator`: 4px above and below a 1px line, 6px in.
const SEPARATOR_H: f32 = 4.0 + 1.0 + 4.0;
/// `.shell-menu-item`: `--lr-control-md` tall, 2px between the menu's rows.
const ITEM_H: f32 = 36.0;
const MENU_GAP: f32 = 2.0;

/// The account's entries under the status card; returns the one chosen.
fn account_menu(
    ui: &mut Ui,
    left: f32,
    top: f32,
    width: f32,
    items: &[(MenuItem, &'static str)],
    unread: bool,
) -> Option<MenuItem> {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    // The unread entry's name carries the web's sr-only suffix.
    let feedback_label = format!(
        "{}, {}",
        shell::user_panel_feedback(),
        shell::user_panel_feedback_unread()
    );
    let separator = |ui: &Ui, y: f32| {
        hairline_h(
            ui.painter(),
            &theme,
            left + 6.0..=left + width - 6.0,
            y + 4.0,
        );
    };
    let mut picked = None;
    let mut y = top;
    separator(ui, y);
    y += SEPARATOR_H + MENU_GAP;
    let last = items.len() - 1;
    for (i, (action, label)) in items.iter().enumerate() {
        if i == last {
            separator(ui, y);
            y += SEPARATOR_H + MENU_GAP;
        }
        let item = Rect::from_min_size(pos2(left, y), Vec2::new(width, ITEM_H));
        let response = common::click(ui, item, item_id(*action));
        let dotted = unread && *action == MenuItem::Feedback;
        let name = if dotted {
            feedback_label.as_str()
        } else {
            label
        };
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::MenuItem, true, name));
        let response = if *action == MenuItem::SwitchServer {
            response.on_hover_text(format!("{}+{}+S", shell::keys_ctrl(), shell::keys_shift()))
        } else {
            response
        };
        if response.hovered() {
            common::fill(ui.painter(), item, theme.radius.sm, c.hover);
        }
        common::focus_visible(ui, &response, item, theme.radius.sm);
        let block = Block::single(ui, label, text::UI, c.text);
        paint_mid(ui.painter(), &block, item.left() + 10.0, item.center().y);
        if dotted {
            // `.unread-dot` with `margin-left: auto`: at the entry's right
            // padding, centred on the line.
            crate::screens::feedback::dialog::dot(
                ui.painter(),
                pos2(item.right() - 10.0 - 4.0, item.center().y),
                c.accent,
            );
        }
        if response.clicked() {
            picked = Some(*action);
        }
        y += ITEM_H + MENU_GAP;
    }
    picked
}

/// A row of the presence modes (`PresenceModeMenu`): a mode, or one of the
/// durations below the mode that asks.
fn mode_id(mode: PresenceMode) -> Id {
    Id::unique(("user-presence-mode", mode))
}

fn duration_id(duration: ModeDuration) -> Id {
    Id::unique(("user-presence-duration", duration))
}

/// The modes' rows in order, the durations under the one that asks.
fn mode_ids(asking: Option<PresenceMode>) -> Vec<Id> {
    let mut ids = Vec::new();
    for mode in PresenceMode::ALL {
        ids.push(mode_id(mode));
        if asking == Some(mode) {
            ids.extend(ModeDuration::ALL.into_iter().map(duration_id));
        }
    }
    ids
}

fn mode_name(mode: PresenceMode) -> &'static str {
    match mode {
        PresenceMode::Auto => shell::presence_mode_auto(),
        PresenceMode::Idle => shell::presence_mode_idle(),
        PresenceMode::Dnd => shell::presence_mode_dnd(),
        PresenceMode::Invisible => shell::presence_mode_invisible(),
    }
}

/// What the mode shows on your avatar (`DOT`).
fn mode_dot(mode: PresenceMode) -> crate::widgets::Presence {
    use crate::widgets::Presence;
    match mode {
        PresenceMode::Auto => Presence::Online,
        PresenceMode::Idle => Presence::Away,
        PresenceMode::Dnd => Presence::Dnd,
        PresenceMode::Invisible => Presence::Invisible,
    }
}

/// What the mode does, under its name while it asks.
fn mode_hint(mode: PresenceMode) -> Option<&'static str> {
    match mode {
        PresenceMode::Dnd => Some(shell::presence_mode_hint_dnd()),
        PresenceMode::Invisible => Some(shell::presence_mode_hint_invisible()),
        _ => None,
    }
}

fn duration_name(duration: ModeDuration) -> &'static str {
    match duration {
        ModeDuration::Hour => shell::presence_mode_duration_h1(),
        ModeDuration::EightHours => shell::presence_mode_duration_h8(),
        ModeDuration::Day => shell::presence_mode_duration_h24(),
        ModeDuration::Forever => shell::presence_mode_duration_forever(),
    }
}

/// The label column of a mode's row: after the 10px dot with 4px either
/// side, the item's 10px padding and its 8px gap.
const MODE_TEXT_X: f32 = 10.0 + 18.0 + 8.0;

/// The hint under the asking mode's name, wrapped to the row.
fn hint_block(ui: &Ui, width: f32, mode: PresenceMode) -> Option<Block> {
    let theme = Theme::of(ui.ctx());
    mode_hint(mode).map(|hint| {
        Block::wrapped(
            ui,
            hint,
            text::META,
            theme.c.muted,
            width - MODE_TEXT_X - 10.0,
        )
    })
}

/// A mode's row: one line, or two with the hint while it asks.
fn mode_row_height(ui: &Ui, width: f32, mode: PresenceMode, asking: bool) -> f32 {
    match asking.then(|| hint_block(ui, width, mode)).flatten() {
        Some(hint) => (8.0 + text::UI.line_px() + hint.height() + 8.0).max(ITEM_H),
        None => ITEM_H,
    }
}

/// The height of [`mode_menu`].
fn modes_height(ui: &Ui, width: f32, asking: Option<PresenceMode>) -> f32 {
    PresenceMode::ALL
        .into_iter()
        .map(|mode| {
            let open = asking == Some(mode);
            let durations = if open {
                ModeDuration::ALL.len() as f32 * (ITEM_H + MENU_GAP)
            } else {
                0.0
            };
            mode_row_height(ui, width, mode, open) + MENU_GAP + durations
        })
        .sum()
}

/// A choice in the modes: open or close a mode's durations, or set one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ModeChoice {
    Ask(Option<PresenceMode>),
    Set(PresenceMode, ModeDuration),
}

/// The presence modes at the top of the account menu (#590): Automatic
/// takes at once, the others ask for how long first, right below their row.
fn mode_menu(
    ui: &mut Ui,
    left: f32,
    top: f32,
    width: f32,
    current: PresenceMode,
    asking: Option<PresenceMode>,
) -> Option<ModeChoice> {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let mut chosen = None;
    let mut y = top;
    for mode in PresenceMode::ALL {
        let open = asking == Some(mode);
        let height = mode_row_height(ui, width, mode, open);
        let row = Rect::from_min_size(pos2(left, y), Vec2::new(width, height));
        let response = common::click(ui, row, mode_id(mode));
        let name = mode_name(mode);
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::Role::MenuItemRadio, true, current == mode, name)
        });
        if response.hovered() {
            common::fill(ui.painter(), row, theme.radius.sm, c.hover);
        }
        common::focus_visible(ui, &response, row, theme.radius.sm);
        let hint = open.then(|| hint_block(ui, width, mode)).flatten();
        let block = Block::single(ui, name, text::UI, c.text);
        let line_y = if hint.is_some() {
            row.top() + 8.0 + text::UI.line_px() / 2.0
        } else {
            row.center().y
        };
        // The cut-outs take the menu's surface.
        mode_dot(mode).paint(
            ui.painter(),
            &theme,
            pos2(row.left() + 10.0 + 4.0 + 5.0, line_y),
            10.0,
            c.elevated,
        );
        paint_mid(ui.painter(), &block, row.left() + MODE_TEXT_X, line_y);
        if let Some(hint) = &hint {
            hint.paint(
                ui.painter(),
                pos2(
                    row.left() + MODE_TEXT_X,
                    row.top() + 8.0 + text::UI.line_px(),
                ),
            );
        }
        if response.clicked() {
            chosen = Some(match mode {
                PresenceMode::Auto => ModeChoice::Set(mode, ModeDuration::Forever),
                _ if open => ModeChoice::Ask(None),
                _ => ModeChoice::Ask(Some(mode)),
            });
        }
        y += height + MENU_GAP;
        if !open {
            continue;
        }
        for duration in ModeDuration::ALL {
            let item = Rect::from_min_size(pos2(left, y), Vec2::new(width, ITEM_H));
            let response = common::click(ui, item, duration_id(duration));
            let label = duration_name(duration);
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::MenuItem, true, label));
            if response.hovered() {
                common::fill(ui.painter(), item, theme.radius.sm, c.hover);
            }
            common::focus_visible(ui, &response, item, theme.radius.sm);
            let block = Block::single(ui, label, text::UI, c.text);
            // `.presence-mode__durations`: indented to the modes' names.
            paint_mid(
                ui.painter(),
                &block,
                item.left() + MODE_TEXT_X,
                item.center().y,
            );
            if response.clicked() {
                chosen = Some(ModeChoice::Set(mode, duration));
            }
            y += ITEM_H + MENU_GAP;
        }
    }
    chosen
}

/// When a mode ends (`modeLabel`): "18:30" today, else "Mo., 09:00".
fn mode_end(until: chrono::DateTime<chrono::Utc>) -> String {
    use chrono::{Datelike, Local, Timelike};
    let local = until.with_timezone(&Local);
    let time = format!("{:02}:{:02}", local.hour(), local.minute());
    if local.date_naive() == Local::now().date_naive() {
        time
    } else {
        let weekday =
            gelabber_i18n::intl::weekday_name(local.weekday().num_days_from_monday(), true);
        format!("{weekday}, {time}")
    }
}

/// The height of [`account_menu`] for `count` entries.
fn menu_height(count: usize) -> f32 {
    2.0 * SEPARATOR_H + count as f32 * ITEM_H + (count + 1) as f32 * MENU_GAP
}

/// The popover's keys (`moveInAccount`): ↓ from the status field to the
/// first entry, the arrows, Home and End within the menu (the modes, their
/// durations while open, then the account's entries).
fn account_keys(ui: &Ui, ids: &[Id], field: Id) {
    let focused = ui.memory(|m| m.focused());
    let at = focused.and_then(|f| ids.iter().position(|id| *id == f));
    if let Some(at) = at {
        // The arrows move between the entries instead of egui's focus.
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                ids[at],
                egui::EventFilter {
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    ..Default::default()
                },
            )
        });
        let keys = [
            egui::Key::ArrowDown,
            egui::Key::ArrowUp,
            egui::Key::Home,
            egui::Key::End,
        ];
        let pressed = ui.input_mut(|i| {
            keys.into_iter()
                .find(|k| i.consume_key(egui::Modifiers::NONE, *k))
        });
        if let Some(step) = pressed.and_then(|k| account_step(k, at, ids.len())) {
            let next = step.map_or(field, |n| ids[n]);
            ui.memory_mut(|m| m.request_focus(next));
            crate::widgets::set_keyboard_focus(ui.ctx(), true);
        }
    } else if focused == Some(field)
        && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown))
    {
        ui.memory_mut(|m| m.request_focus(ids[0]));
        crate::widgets::set_keyboard_focus(ui.ctx(), true);
    }
}

/// What a chosen entry does.
fn pick(ui: &Ui, app: &mut AppCtx, item: MenuItem) {
    match item {
        MenuItem::Profile => app.nav.navigate("/settings/profil"),
        MenuItem::Operator => super::operator::open_in_browser(
            app,
            &format!("/betrieb/{}", super::operator::DEFAULT_PAGE),
        ),
        MenuItem::SwitchServer => crate::setup::switch_server(ui.ctx()),
        MenuItem::SignOut => log_out(ui.ctx(), app),
        MenuItem::Feedback => {
            // The popover closes, and the avatar takes the focus back from
            // the dialog (UserPanel.tsx).
            crate::widgets::opener::opened_from(ui.ctx(), profile_id());
            crate::screens::feedback::open_dialog(ui.ctx(), app);
        }
    }
}

/// `logOut` (components/logout.ts): asks first when it would end a call.
pub fn log_out(ctx: &egui::Context, app: &mut AppCtx) {
    use gelabber_call::CallStatus;
    if matches!(
        app.call.state().status,
        CallStatus::Connected | CallStatus::Reconnecting
    ) {
        super::overlays::ask(
            ctx,
            super::overlays::Question {
                title: shell::logout_end_call_title().into(),
                message: Some(shell::logout_end_call_message().into()),
                label: shell::logout_end_call_confirm().into(),
                neutral: false,
                action: super::overlays::ConfirmAction::Logout,
            },
        );
    } else {
        sign_out(app);
    }
}

/// Signed out at once like the web, then the server is told.
pub fn sign_out(app: &mut AppCtx) {
    app.call.leave();
    app.client.logout();
    app.session.adopt(&serde_json::Value::Null);
    app.nav.navigate("/login");
}

/// The account popover, 320px above the name: who you are and the status
/// line with "Speichern" (`StatusCard`), then the account's entries
/// (`AccountMenu`).
#[allow(clippy::too_many_arguments)]
fn account_popover(
    ui: &mut Ui,
    app: &mut AppCtx,
    anchor: Rect,
    name: &str,
    avatar_url: Option<&str>,
    presence: crate::widgets::Presence,
    mode: ModeState,
    current: &str,
    draft: &mut StatusDraft,
    entries: Entries,
) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let saving = app
        .client
        .with_state(|s| matches!(s.op("profile"), Some(OpState::Pending)));
    if draft.saving && !saving {
        draft.saving = false;
        draft.saved = Some(
            app.client
                .with_state(|s| matches!(s.op("profile"), Some(OpState::Done))),
        );
        if draft.saved == Some(true) {
            draft.text = current.to_owned();
        }
    }
    let field_id = crate::widgets::field::field_id("user-status-text");
    // Asked again until the field holds it: the click that opened the
    // card settles focus in the same frame.
    if draft.focus {
        if ui.memory(|m| m.has_focus(field_id)) {
            draft.focus = false;
        } else {
            ui.memory_mut(|m| m.request_focus(field_id));
        }
    }
    let changed = draft.text.trim() != current;
    let items = menu_items(app.session.operator(), entries.feedback);
    let mut ids = mode_ids(draft.asking);
    ids.extend(items.iter().map(|(item, _)| item_id(*item)));
    account_keys(ui, &ids, field_id);
    let out = Popover::new("user-status", anchor)
        .min_width(320.0)
        .show(ui.ctx(), |ui| {
            // The card: padding 4, a column with 12px gaps.
            let width = 320.0 - 18.0;
            // The InlineStatus keeps its 6px gap even while empty.
            let status_h = if draft.saved.is_some() {
                6.0 + 20.0
            } else {
                6.0
            };
            let card_h = 4.0 + 44.8 + 12.0 + 17.55 + 6.0 + 37.0 + status_h + 4.0;
            // `.user-account`: 4px between the card and the menu.
            let modes_h = modes_height(ui, width, draft.asking);
            let height = card_h + 4.0 + modes_h + menu_height(items.len());
            let (card, _) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::hover());
            let left = card.left() + 4.0;
            let inner_w = width - 8.0;
            let top = card.top() + 4.0;
            // Who: avatar 36 and the name over the presence label.
            let avatar = Rect::from_min_size(pos2(left, top + 22.4 - 18.0), Vec2::splat(36.0));
            common::avatar(
                ui, app.images, avatar, name, avatar_url, 14.0, 1.0, c.elevated,
            );
            common::presence_dot(ui.painter(), &theme, avatar, presence);
            let x = avatar.right() + 10.0;
            let strong = line(ui, name, text::TITLE_SM, c.text, left + inner_w - x);
            strong.paint(ui.painter(), pos2(x, top));
            // "Nicht stören bis 18:30" while the mode has an end.
            let presence_label = match mode.until {
                Some(until) if mode.mode != PresenceMode::Auto => {
                    shell::presence_mode_until(presence.label(), &mode_end(until))
                }
                _ => presence.label().to_owned(),
            };
            let small = Block::single(ui, &presence_label, text::META, c.muted);
            // The label sits on the 16px/1.5 line of its inline parent.
            let strut = text::TextStyle::new(16.0, 400.0, 1.5);
            small.paint(
                ui.painter(),
                pos2(x, common::baseline_top(strut, top + 20.8, text::META)),
            );
            // The form.
            let mut y = top + 44.8 + 12.0;
            let label = Block::single(ui, shell::user_panel_status(), text::UI, c.text);
            label.paint(ui.painter(), pos2(left, y));
            y += 17.55 + 6.0;
            let button_label = if saving {
                shell::user_panel_saving()
            } else {
                gelabber_i18n::common::save()
            };
            let button_w = Block::single(ui, button_label, text::UI, c.text).width() + 26.0;
            let field_rect =
                Rect::from_min_size(pos2(left, y), Vec2::new(inner_w - button_w - 8.0, 37.0));
            let mut save = false;
            let mut field_ui = ui.new_child(egui::UiBuilder::new().max_rect(field_rect));
            let field = TextField::new("user-status-text", &mut draft.text)
                .name(shell::user_panel_status())
                .char_limit(STATUS_MAX)
                .show(&mut field_ui);
            save |= field.submitted;
            if field.response.changed() {
                draft.saved = None;
            }
            let button_rect = Rect::from_min_size(
                pos2(field_rect.right() + 8.0, y + 2.5),
                Vec2::new(button_w, 32.0),
            );
            let mut button_ui = ui.new_child(egui::UiBuilder::new().max_rect(button_rect));
            save |= button_ui
                .add(
                    Button::new(gelabber_i18n::common::save())
                        .small()
                        .width(button_w)
                        .disabled(!changed && !saving)
                        .pending(saving.then_some(shell::user_panel_saving())),
                )
                .clicked();
            y += 37.0 + 6.0;
            if let Some(ok) = draft.saved {
                let (msg, tone) = if ok {
                    (gelabber_i18n::common::saved(), StatusTone::Success)
                } else {
                    (gelabber_i18n::common::not_saved(), StatusTone::Error)
                };
                let mut status_ui = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(Rect::from_min_size(pos2(left, y), Vec2::new(inner_w, 20.0))),
                );
                status_ui.add(InlineStatus::new(Some(msg), tone));
            }
            let menu_top = card.top() + card_h + 4.0;
            let chosen = mode_menu(ui, card.left(), menu_top, width, mode.mode, draft.asking);
            let picked = account_menu(
                ui,
                card.left(),
                menu_top + modes_h,
                width,
                &items,
                entries.unread,
            );
            (save, chosen, picked)
        });
    match out {
        None => {
            draft.open = false;
            // Esc hands the focus back to the avatar (Popover.tsx).
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                ui.memory_mut(|m| m.request_focus(profile_id()));
            }
        }
        Some((save, chosen, picked)) => {
            match chosen {
                Some(ModeChoice::Ask(asking)) => {
                    draft.asking = asking;
                    // The focus stays on the mode's row.
                }
                Some(ModeChoice::Set(mode, duration)) => {
                    draft.asking = None;
                    draft.open = false;
                    ui.memory_mut(|m| m.request_focus(profile_id()));
                    app.client.set_presence_mode(mode, duration);
                }
                None => {}
            }
            if save && changed && !saving {
                draft.saving = true;
                draft.saved = None;
                app.client.update_profile(ProfilePatch {
                    status_text: Some(draft.text.trim().to_owned()),
                    ..ProfilePatch::default()
                });
            }
            if let Some(item) = picked {
                draft.open = false;
                pick(ui, app, item);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::app::testkit::{click_last, click_node, click_role, id_of};
    use egui::accesskit::Role;
    use egui_kittest::kittest::{NodeT, Queryable};

    use crate::app::testkit::{
        FakeApi, Reply, account_button, launcher, open_account, wait_for, wait_for_label,
        wait_for_path, wait_for_text,
    };
    use crate::platform::paths::testing::TempConfig;

    type App<'a> = egui_kittest::Harness<'a, crate::Launcher>;

    fn open_menu(harness: &mut App<'_>) {
        open_account(harness, "Lena");
    }

    /// The account menu's entries, in order.
    fn entries(harness: &App<'_>) -> Vec<String> {
        harness
            .query_all_by_role(Role::MenuItem)
            .filter_map(|n| n.accesskit_node().label())
            .collect()
    }

    /// logOut: in a call it asks first; "Abbrechen" stays signed in.
    #[test]
    fn signing_out_in_a_call_asks_first() {
        let _config = TempConfig::new("logout-call");
        let api = FakeApi::start(true, |_, _, _| None);
        let mut harness = launcher(&api, "/");
        open_menu(&mut harness);
        let call = harness.state_mut().app().unwrap().call().clone();
        call.set_state(gelabber_call::CallState {
            status: gelabber_call::CallStatus::Connected,
            server: Some(uuid::Uuid::from_u128(1)),
            channel: Some(uuid::Uuid::from_u128(2)),
            ..Default::default()
        });
        click_role(&mut harness, Role::MenuItem, "Abmelden");
        wait_for_label(&mut harness, "Abmelden und Anruf beenden?");
        assert!(
            harness
                .query_by_label("Du verlässt dabei den Sprachkanal.")
                .is_some()
        );
        click_role(&mut harness, Role::Button, "Abbrechen");
        harness.run_steps(10);
        assert!(!api.saw("POST /auth/logout"));
        assert!(
            harness
                .query_by_label("Abmelden und Anruf beenden?")
                .is_none()
        );
        // Confirmed: signed out, on the login page.
        open_menu(&mut harness);
        click_role(&mut harness, Role::MenuItem, "Abmelden");
        wait_for_label(&mut harness, "Abmelden und Anruf beenden?");
        click_last(&mut harness, Role::Button, "Abmelden");
        wait_for(&mut harness, "the logout", |_| api.saw("POST /auth/logout"));
        wait_for_label(&mut harness, "Anmelden");
    }

    /// `focusInside`: the status field is the card's first control, so it
    /// holds the focus from the frame the card opens, with no button first.
    #[test]
    fn the_status_card_opens_with_its_field_focused() {
        let _config = TempConfig::new("status-focus");
        let api = FakeApi::start(true, |_, _, _| None);
        let mut harness = launcher(&api, "/");
        click_node(&mut harness, "the avatar", |h| {
            account_button(h, "Lena").map(id_of)
        });
        let field = crate::widgets::field::field_id("user-status-text");
        let mut seen = Vec::new();
        for _ in 0..6 {
            harness.step();
            seen.push(harness.ctx.memory(|m| m.focused()));
        }
        assert!(harness.query_by_label("Speichern").is_some());
        // The click's own frame may still settle; after it, only the field.
        assert!(seen.iter().all(|f| *f == Some(field)), "{field:?} {seen:?}");
    }

    /// Without a call it signs out at once; a failed server logout shows
    /// the strip with "Erneut abmelden" (SessionRecoveryNotice LogoutFailed).
    #[test]
    fn a_failed_logout_offers_to_try_again() {
        let _config = TempConfig::new("logout-failed");
        let fail = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let failing = fail.clone();
        let api = FakeApi::start(true, move |method, path, _| {
            (method == "POST"
                && path == "/auth/logout"
                && failing.load(std::sync::atomic::Ordering::SeqCst))
            .then(|| Reply::error(500, "internal"))
        });
        let mut harness = launcher(&api, "/");
        open_menu(&mut harness);
        click_role(&mut harness, Role::MenuItem, "Abmelden");
        let text =
            "Die Abmeldung auf dem Server konnte nicht bestätigt werden. Bitte versuche es erneut.";
        wait_for_label(&mut harness, text);
        assert!(
            harness
                .query_by_label("Abmelden und Anruf beenden?")
                .is_none()
        );
        fail.store(false, std::sync::atomic::Ordering::SeqCst);
        click_role(&mut harness, Role::Button, "Erneut abmelden");
        wait_for(&mut harness, "the strip to go", |h| {
            h.query_by_label(text).is_none()
        });
    }

    /// "Auf allen Geräten abmelden" leaves for /login before the server
    /// answers: a failure says the other devices are not signed out yet,
    /// and "Erneut abmelden" repeats `logout-all`.
    #[test]
    fn a_failed_logout_everywhere_says_so_and_is_retried_as_one() {
        let _config = TempConfig::new("logout-all-failed");
        let fail = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let failing = fail.clone();
        let api = FakeApi::start(true, move |method, path, _| match (method, path) {
            ("POST", "/auth/logout-all") if failing.load(std::sync::atomic::Ordering::SeqCst) => {
                Some(Reply::error(500, "internal"))
            }
            ("POST", "/auth/logout-all") => {
                Some(Reply::json(serde_json::json!({"csrf_token": "csrf"})))
            }
            _ => None,
        });
        // A real session: the held sign-out carries its cookie.
        let client = gelabber_client::Client::new(&api.origin).expect("client");
        client.http().restore_session_cookie("tok");
        let mut harness = crate::app::testkit::launcher_with(&api, client, "/settings/konto");
        wait_for_label(&mut harness, "Auf allen Geräten abmelden");
        click_role(&mut harness, Role::Button, "Auf allen Geräten abmelden");
        // The question's button; the row of Sicherheit has that title too.
        wait_for(&mut harness, "the question", |h| {
            h.query_by_role_and_label(Role::Button, "Überall abmelden")
                .is_some()
        });
        click_role(&mut harness, Role::Button, "Überall abmelden");
        let text = "Die Abmeldung auf allen Geräten konnte nicht bestätigt werden. Bitte versuche es erneut.";
        wait_for_label(&mut harness, text);
        fail.store(false, std::sync::atomic::Ordering::SeqCst);
        click_role(&mut harness, Role::Button, "Erneut abmelden");
        wait_for(&mut harness, "the strip to go", |h| {
            h.query_by_label(text).is_none()
        });
        let posts: Vec<String> = api
            .seen
            .lock()
            .iter()
            .filter(|r| r.starts_with("POST /auth/logout"))
            .cloned()
            .collect();
        assert_eq!(posts.len(), 2, "{posts:?}");
        assert!(
            posts.iter().all(|p| p.starts_with("POST /auth/logout-all")),
            "{posts:?}"
        );
    }

    /// Esc closes the open menu and is used up there: the conversation
    /// under it is not marked read too, nor with Shift the whole server.
    #[test]
    fn esc_closes_the_menu_and_marks_nothing_read() {
        use crate::app::testkit::{ALLGEMEIN, HOME, wait_until_idle, workspace};

        let _config = TempConfig::new("menu-esc");
        let api = FakeApi::start(true, |method, path, _| workspace(method, path));
        let mut harness = launcher(&api, &format!("/s/{HOME}/c/{ALLGEMEIN}"));
        let reads = || {
            api.seen
                .lock()
                .iter()
                .filter(|r| r.starts_with("PUT ") && r.ends_with("/read"))
                .count()
        };
        // With nothing open, Esc marks #allgemein read once it is shown.
        wait_until_idle(&mut harness);
        harness.key_press(egui::Key::Escape);
        wait_until_idle(&mut harness);
        assert_eq!(reads(), 1, "the Esc mark-read");
        let before = reads();
        for shift in [false, true] {
            open_menu(&mut harness);
            if shift {
                harness.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::Escape);
            } else {
                harness.key_press(egui::Key::Escape);
            }
            wait_until_idle(&mut harness);
            assert!(
                harness.query_by_label("Abmelden").is_none(),
                "closed (shift: {shift})"
            );
            assert_eq!(reads(), before, "nothing marked read (shift: {shift})");
            // The focus is back on the avatar (Popover.tsx).
            assert_eq!(
                harness.ctx.memory(|m| m.focused()),
                Some(super::profile_id()),
                "focus (shift: {shift})"
            );
        }
    }

    /// The stand-in API with Lena's workspace, Lena an operator or not.
    fn operator_api(operator: bool) -> FakeApi {
        feedback_api(operator, false)
    }

    /// As [`operator_api`], on an instance that takes feedback or not.
    fn feedback_api(operator: bool, feedback: bool) -> FakeApi {
        FakeApi::start(true, move |method, path, _| match (method, path) {
            ("GET", "/auth/session") => Some(Reply::json(serde_json::json!({
                "user": crate::app::testkit::lena(), "csrf_token": "csrf", "operator": operator
            }))),
            ("GET", "/feedback/status") if feedback => Some(Reply::json(
                serde_json::json!({"enabled": true, "unread": 0}),
            )),
            _ => crate::app::testkit::workspace(method, path),
        })
    }

    /// The entries in the web's order (`AccountMenu`): "Profil bearbeiten",
    /// "Feedback geben" while the instance takes feedback, "Betrieb" for
    /// operators, the app's own "Server wechseln …", then "Abmelden". No
    /// "Einstellungen": the gear is beside the avatar.
    #[test]
    fn the_menu_keeps_the_web_order() {
        use super::MenuItem::*;
        let order = |operator, feedback| -> Vec<super::MenuItem> {
            super::menu_items(operator, feedback)
                .into_iter()
                .map(|(item, _)| item)
                .collect()
        };
        assert_eq!(order(false, false), [Profile, SwitchServer, SignOut]);
        assert_eq!(
            order(false, true),
            [Profile, Feedback, SwitchServer, SignOut]
        );
        assert_eq!(
            order(true, false),
            [Profile, Operator, SwitchServer, SignOut]
        );
        assert_eq!(
            order(true, true),
            [Profile, Feedback, Operator, SwitchServer, SignOut]
        );
    }

    /// `accountStep`: round the ends, ↑ on the first back to the field.
    #[test]
    fn the_arrows_step_round_the_menu() {
        use egui::Key;
        let step = super::account_step;
        assert_eq!(step(Key::ArrowDown, 0, 3), Some(Some(1)));
        assert_eq!(step(Key::ArrowDown, 2, 3), Some(Some(0)));
        assert_eq!(step(Key::ArrowUp, 2, 3), Some(Some(1)));
        assert_eq!(step(Key::ArrowUp, 0, 3), Some(None));
        assert_eq!(step(Key::Home, 2, 3), Some(Some(0)));
        assert_eq!(step(Key::End, 0, 3), Some(Some(2)));
        assert_eq!(step(Key::Enter, 0, 3), None);
    }

    /// Drawn: the status card on top, then the entries as menu items, with
    /// "Feedback geben" after the profile; no "Einstellungen" among them.
    #[test]
    fn feedback_follows_the_profile() {
        use crate::app::testkit::HOME;

        let _config = TempConfig::new("menu-feedback");
        let api = feedback_api(true, true);
        let mut harness = launcher(&api, &format!("/s/{HOME}"));
        wait_for(&mut harness, "the status", |_| {
            api.saw("GET /feedback/status")
        });
        harness.run_steps(5);
        open_menu(&mut harness);
        wait_for_label(&mut harness, "Feedback geben");
        assert_eq!(
            entries(&harness),
            [
                "Profil bearbeiten",
                "Feedback geben",
                "Betrieb",
                "Server wechseln …",
                "Abmelden"
            ]
        );
        assert!(harness.query_by_label("Speichern").is_some());
    }

    /// The keys (#570): ↓ from the status field to the first entry, the
    /// arrows round the ends, Home and End, ↑ back into the field; Enter
    /// takes the entry.
    #[test]
    fn the_arrows_move_through_the_menu() {
        use super::MenuItem::*;
        use crate::app::testkit::HOME;

        let _config = TempConfig::new("menu-keys");
        let api = operator_api(false);
        let mut harness = launcher(&api, &format!("/s/{HOME}"));
        open_menu(&mut harness);
        harness.run_steps(4);
        let field = crate::widgets::field::field_id("user-status-text");
        let focused = |h: &App<'_>| h.ctx.memory(|m| m.focused());
        assert_eq!(focused(&harness), Some(field));
        let auto = super::mode_id(gelabber_client::PresenceMode::Auto);
        let invisible = super::mode_id(gelabber_client::PresenceMode::Invisible);
        for (key, want) in [
            // The modes come first (#590).
            (egui::Key::ArrowDown, auto),
            (egui::Key::End, super::item_id(SignOut)),
            (egui::Key::ArrowDown, auto),
            (egui::Key::ArrowUp, field),
            (egui::Key::ArrowDown, auto),
            (
                egui::Key::ArrowDown,
                super::mode_id(gelabber_client::PresenceMode::Idle),
            ),
            (
                egui::Key::ArrowDown,
                super::mode_id(gelabber_client::PresenceMode::Dnd),
            ),
            (egui::Key::ArrowDown, invisible),
            (egui::Key::ArrowDown, super::item_id(Profile)),
            (egui::Key::ArrowDown, super::item_id(SwitchServer)),
        ] {
            harness.key_press(key);
            harness.run_steps(3);
            assert_eq!(focused(&harness), Some(want), "{key:?}");
        }
        harness.key_press(egui::Key::ArrowUp);
        harness.run_steps(3);
        assert_eq!(focused(&harness), Some(super::item_id(Profile)));
        harness.key_press(egui::Key::Enter);
        wait_for_path(&mut harness, "/settings/profil");
    }

    /// The modes (#590): Automatic is checked; another mode asks for how
    /// long first, right below its row, and the choice goes to the server
    /// and onto your avatar.
    #[test]
    fn a_mode_asks_for_how_long_and_is_saved() {
        let _config = TempConfig::new("presence-mode");
        let until = (chrono::Utc::now() + chrono::Duration::hours(8)).to_rfc3339();
        let api = FakeApi::start(true, move |method, path, _| match (method, path) {
            ("GET", "/auth/session") => Some(Reply::json(serde_json::json!({
                "user": crate::app::testkit::lena(), "csrf_token": "csrf", "operator": false
            }))),
            ("PUT", "/me/presence") => Some(Reply::json(
                serde_json::json!({"mode": "dnd", "until": until}),
            )),
            _ => crate::app::testkit::workspace(method, path),
        });
        let mut harness = launcher(&api, &format!("/s/{}", crate::app::testkit::HOME));
        open_menu(&mut harness);
        harness.run_steps(3);
        let radios: Vec<(String, bool)> = harness
            .query_all_by_role(Role::MenuItemRadio)
            .map(|n| {
                let node = n.accesskit_node();
                (
                    node.label().unwrap_or_default(),
                    node.toggled() == Some(egui::accesskit::Toggled::True),
                )
            })
            .collect();
        assert_eq!(
            radios,
            [
                ("Automatisch".to_owned(), true),
                ("Abwesend".to_owned(), false),
                ("Nicht stören".to_owned(), false),
                ("Unsichtbar".to_owned(), false),
            ]
        );
        assert!(!entries(&harness).iter().any(|e| e == "8 Stunden"));
        click_role(&mut harness, Role::MenuItemRadio, "Nicht stören");
        harness.run_steps(3);
        assert_eq!(
            entries(&harness)[..4],
            ["1 Stunde", "8 Stunden", "24 Stunden", "Bis ich es ändere"]
        );
        click_role(&mut harness, Role::MenuItem, "8 Stunden");
        wait_for(&mut harness, "the mode saved", |_| {
            api.saw("PUT /me/presence")
        });
        harness.run_steps(3);
        assert!(
            harness.query_by_label("Abmelden").is_none(),
            "the menu closed"
        );
        let client = harness.state_mut().app().unwrap().client().clone();
        wait_for(&mut harness, "the mode shown", |_| {
            client.with_state(|s| s.presence_mode.mode == gelabber_client::PresenceMode::Dnd)
        });
        open_menu(&mut harness);
        harness.run_steps(3);
        assert!(client.with_state(|s| s.presence_mode.until.is_some()));
        let checked = harness
            .get_by_role_and_label(Role::MenuItemRadio, "Nicht stören")
            .accesskit_node()
            .toggled();
        assert_eq!(checked, Some(egui::accesskit::Toggled::True));
    }

    /// `modeLabel`'s end: the time today, the weekday before it otherwise.
    #[test]
    fn a_mode_end_names_its_day_when_not_today() {
        use chrono::{Local, TimeZone};
        let today = Local::now()
            .date_naive()
            .and_hms_opt(18, 30, 0)
            .and_then(|t| Local.from_local_datetime(&t).single())
            .expect("today 18:30");
        assert_eq!(super::mode_end(today.to_utc()), "18:30");
        let later = super::mode_end((today + chrono::Duration::days(1)).to_utc());
        assert!(later.ends_with(", 18:30"), "{later}");
    }

    /// A mode the server turns down comes back as it was, with a toast.
    #[test]
    fn a_refused_mode_goes_back() {
        let _config = TempConfig::new("presence-mode-refused");
        let api = FakeApi::start(true, move |method, path, _| match (method, path) {
            ("GET", "/auth/session") => Some(Reply::json(serde_json::json!({
                "user": crate::app::testkit::lena(), "csrf_token": "csrf", "operator": false
            }))),
            ("PUT", "/me/presence") => Some(Reply::error(500, "internal")),
            _ => crate::app::testkit::workspace(method, path),
        });
        let mut harness = launcher(&api, &format!("/s/{}", crate::app::testkit::HOME));
        open_menu(&mut harness);
        harness.run_steps(3);
        click_role(&mut harness, Role::MenuItemRadio, "Abwesend");
        harness.run_steps(3);
        click_role(&mut harness, Role::MenuItem, "Bis ich es ändere");
        let client = harness.state_mut().app().unwrap().client().clone();
        wait_for(&mut harness, "the refusal", |_| {
            client.with_state(|s| {
                matches!(
                    s.op("presence_mode"),
                    Some(gelabber_client::OpState::Failed(_))
                )
            })
        });
        wait_for_text(&mut harness, "Status nicht geändert.");
        let mode = harness
            .state_mut()
            .app()
            .unwrap()
            .client()
            .with_state(|s| s.presence_mode.mode);
        assert_eq!(mode, gelabber_client::PresenceMode::Auto);
    }

    /// The gear is Settings, in a call too (#570), and the call goes on.
    #[test]
    fn the_gear_opens_the_settings_in_a_call() {
        use crate::app::testkit::HOME;

        let _config = TempConfig::new("gear-call");
        let api = operator_api(false);
        let mut harness = launcher(&api, &format!("/s/{HOME}"));
        wait_for_label(&mut harness, "Einstellungen");
        let call = harness.state_mut().app().unwrap().call().clone();
        call.set_state(gelabber_call::CallState {
            status: gelabber_call::CallStatus::Connected,
            server: Some(uuid::Uuid::from_u128(1)),
            channel: Some(uuid::Uuid::from_u128(2)),
            ..Default::default()
        });
        harness.run_steps(4);
        // The panel's gear keeps its name (the overview's room card has
        // the call's own gear).
        click_role(&mut harness, Role::Button, "Einstellungen");
        wait_for_path(&mut harness, "/settings/darstellung");
        assert_eq!(call.state().status, gelabber_call::CallStatus::Connected);
    }

    /// "Betrieb" is in the menu for operators only, after the profile, and
    /// opens /betrieb/server in the browser; the app stays where it was.
    #[test]
    fn betrieb_shows_for_operators_and_opens_the_browser() {
        use crate::app::testkit::HOME;

        let _config = TempConfig::new("menu-operator");
        let api = operator_api(false);
        let mut harness = launcher(&api, &format!("/s/{HOME}"));
        open_menu(&mut harness);
        assert!(harness.query_by_label("Betrieb").is_none());

        let api = operator_api(true);
        let mut harness = launcher(&api, &format!("/s/{HOME}"));
        let opened = crate::platform::open_testing::Recording::start();
        open_menu(&mut harness);
        assert_eq!(
            entries(&harness),
            [
                "Profil bearbeiten",
                "Betrieb",
                "Server wechseln …",
                "Abmelden"
            ]
        );
        click_role(&mut harness, Role::MenuItem, "Betrieb");
        wait_for(&mut harness, "the browser", |_| !opened.opened().is_empty());
        assert_eq!(opened.opened(), [format!("{}/betrieb/server", api.origin)]);
        harness.run_steps(3);
        assert!(harness.query_by_label("Abmelden").is_none());
        let route = harness.state_mut().app().unwrap().nav().route().clone();
        assert_eq!(route, crate::route::Route::Server(HOME.parse().unwrap()));
    }

    /// `/betrieb/{page}`: operators get the hand-off card, which opens the
    /// same page in the browser; anyone else meets "not found".
    #[test]
    fn betrieb_in_the_app_hands_off_to_the_browser() {
        let _config = TempConfig::new("route-operator");
        let api = operator_api(true);
        let mut harness = launcher(&api, "/betrieb/meldungen");
        let opened = crate::platform::open_testing::Recording::start();
        wait_for_label(&mut harness, "Im Browser öffnen");
        click_role(&mut harness, Role::Button, "Im Browser öffnen");
        wait_for(&mut harness, "the browser", |_| !opened.opened().is_empty());
        assert_eq!(
            opened.opened(),
            [format!("{}/betrieb/meldungen", api.origin)]
        );

        let api = operator_api(false);
        let mut harness = launcher(&api, "/betrieb");
        wait_for_label(&mut harness, "Zur Startseite");
        assert!(harness.query_by_label("Im Browser öffnen").is_none());
    }
}
