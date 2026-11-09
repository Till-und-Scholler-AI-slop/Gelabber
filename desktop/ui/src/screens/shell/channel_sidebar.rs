//! The channel sidebar of a server (components/ChannelSidebar.tsx,
//! index.css `.channel-sidebar`): the header with the server menu and the
//! invite button, "Übersicht", then categories, channels and who sits in
//! each voice room.

use egui::{Color32, Id, Rect, Stroke, StrokeKind, Ui, Vec2, pos2};
use gelabber_client::{Category, Channel, ChannelKind, Permission, VoiceOccupant};
use uuid::Uuid;

use super::common::{self, line, paint_mid};
use super::row_menu::{self, Owner};
use super::state;
use crate::app::AppCtx;
use crate::icons::{self, Icon};
use crate::text::{self, Block};
use crate::theme::{Theme, corner, mix};
use crate::widgets::{Popover, count_badge, hairline_h};

/// What the list shows, in the web's order (servers/rows.ts).
enum Row {
    Section,
    Category(Category),
    Empty,
    Channel(Channel),
    Person(Uuid, VoiceOccupant, bool),
}

impl Row {
    fn slot(&self) -> f32 {
        match self {
            Row::Section | Row::Category(_) => 34.0,
            Row::Empty | Row::Person(..) => 28.0,
            Row::Channel(_) => 36.0,
        }
    }
}

/// The header (52), "Übersicht" and the list, into `rect` (the column
/// above the user panel). `channel` is the open one, `overview` whether
/// the overview page is shown.
pub fn show(
    ui: &mut Ui,
    app: &mut AppCtx,
    rect: Rect,
    server: Uuid,
    channel: Option<Uuid>,
    overview: bool,
) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let Some((detail, can_manage, can_settings)) = app.client.with_state(|s| {
        s.detail(server).map(|d| {
            let me = s.server(server).unwrap_or(&d.server);
            (
                d.clone(),
                me.can(Permission::ManageChannels),
                me.can(Permission::ManageServer),
            )
        })
    }) else {
        let failed = app.client.with_state(|s| {
            matches!(
                s.op(&format!("server:{server}")),
                Some(gelabber_client::OpState::Failed(_))
            )
        });
        placeholder(
            ui,
            app,
            rect,
            failed.then_some("Die Kanäle konnten nicht geladen werden."),
            &mut || app_refresh(app, server),
        );
        return;
    };
    let painter = ui.painter().clone();

    // Header: menu button (name + chevron) and the invite button.
    let menu_open = state::with(ui.ctx(), |s| s.server_menu);
    let button = Rect::from_min_max(
        pos2(rect.left() + 8.0, rect.top() + 6.5),
        pos2(rect.right() - 1.0 - 6.0 - 30.0 - 2.0, rect.top() + 44.5),
    );
    let response = common::click(ui, button, Id::unique(("server-menu", server)))
        .on_hover_text(&detail.server.name);
    // The web's `<summary>`: a disclosure named by the server's name.
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::Role::DisclosureTriangle,
            true,
            detail.server.name.clone(),
        )
    });
    if response.hovered() || menu_open {
        common::fill(&painter, button, theme.radius.sm, c.hover);
    }
    common::focus_visible(ui, &response, button, theme.radius.sm);
    let muted = app.client.with_state(|s| {
        let now = chrono::Utc::now();
        s.notification_settings.iter().any(|n| {
            n.scope == gelabber_client::SettingScope::Server
                && n.id == server
                && n.muted
                && n.muted_until.is_none_or(|until| until > now)
        })
    });
    let name_style = text::TITLE_SM.size(15.0).tracking(-0.01);
    let mut name_max = button.width() - 16.0 - 16.0 - 6.0;
    if muted {
        name_max -= 15.0 + 6.0;
    }
    let name = line(ui, &detail.server.name, name_style, c.text, name_max);
    paint_mid(&painter, &name, button.left() + 8.0, button.center().y);
    if muted {
        icons::paint(
            &painter,
            Icon::BellOff,
            pos2(
                button.left() + 8.0 + name.width() + 6.0,
                button.center().y - 7.5,
            ),
            15.0,
            c.muted,
        );
    }
    let chevron = pos2(button.right() - 8.0 - 16.0, button.center().y - 8.0);
    let turn = ui.ctx().animate_bool_with_time(
        Id::unique("server-menu-turn"),
        menu_open,
        crate::theme::motion::FAST,
    );
    if turn > 0.5 {
        icons::paint_rotated(&painter, Icon::Chevron, chevron, 16.0, c.muted);
    } else {
        icons::paint(&painter, Icon::Chevron, chevron, 16.0, c.muted);
    }
    if response.clicked() {
        state::with(ui.ctx(), |s| s.server_menu = !s.server_menu);
    }
    let invite = Rect::from_min_size(
        pos2(rect.right() - 1.0 - 6.0 - 30.0, rect.top() + 8.5),
        Vec2::new(30.0, 34.0),
    );
    if shell_icon_button(
        ui,
        invite,
        Icon::Link,
        16.0,
        "Einladen",
        Id::unique("invite-button"),
    )
    .clicked()
    {
        super::dialogs::open_invite(ui.ctx(), server, &detail.server.name);
    }
    hairline_h(
        &painter,
        &theme,
        rect.left()..=rect.right() - 1.0,
        rect.top() + 51.0,
    );

    if menu_open {
        server_menu(
            ui,
            app,
            button,
            server,
            &detail.server.name,
            muted,
            can_manage,
            can_settings,
        );
    }

    // "Übersicht".
    let link = Rect::from_min_size(
        pos2(rect.left() + 8.0, rect.top() + 62.0),
        Vec2::new(rect.width() - 1.0 - 16.0, 34.0),
    );
    let response = common::click(ui, link, Id::unique(("overview-link", server)));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Link, true, "Übersicht"));
    let (fill, color, icon_color) = if overview {
        (c.selected, c.text, c.accent_text)
    } else if response.hovered() {
        (c.hover, c.text, c.text)
    } else {
        (Color32::TRANSPARENT, c.muted, c.muted)
    };
    common::fill(&painter, link, theme.radius.md, fill);
    common::focus_visible(ui, &response, link, theme.radius.md);
    icons::paint(
        &painter,
        Icon::Chat,
        pos2(link.left() + 10.0, link.center().y - 10.0),
        20.0,
        icon_color,
    );
    let style = if overview {
        text::BODY.weight(600.0)
    } else {
        text::BODY
    };
    let label = Block::single(ui, "Übersicht", style, color);
    paint_mid(&painter, &label, link.left() + 40.0, link.center().y);
    if response.clicked() {
        app.nav.navigate(&format!("/s/{server}"));
    }

    // The list: rows from y 98, padding 0 8 12.
    let rows = build_rows(app, &detail);
    let list = Rect::from_min_max(
        pos2(rect.left(), link.bottom() + 2.0),
        pos2(rect.right() - 1.0, rect.bottom()),
    );
    if rows.is_empty() {
        let block = Block::wrapped(
            ui,
            "Noch keine Kanäle.",
            text::BODY.line_height(1.6),
            c.muted,
            list.width() - 44.0,
        );
        block.paint(&painter, pos2(list.left() + 22.0, list.top() + 24.0));
        return;
    }
    let content_h: f32 = rows.iter().map(Row::slot).sum::<f32>() + 12.0;
    let now = chrono::Utc::now();
    // One Tab stop for the list; the rows that take focus, in order.
    let mut nav = Vec::new();
    let mut nav_of = vec![None; rows.len()];
    for (i, row) in rows.iter().enumerate() {
        let items = match row {
            Row::Channel(ch) => {
                let mut items = vec![Id::unique(("channel-row", ch.id))];
                if can_manage {
                    items.push(Id::unique(("edit-channel", ch.id)));
                }
                if can_manage || ch.kind == ChannelKind::Text {
                    items.push(Id::unique(("channel-more", ch.id)));
                }
                items
            }
            Row::Category(category) if can_manage => (0..2)
                .map(|a| Id::unique(("category-action", category.id, a)))
                .collect(),
            _ => continue,
        };
        nav_of[i] = Some(nav.len());
        nav.push(common::NavRow {
            key: items[0],
            items,
        });
    }
    let preferred = rows.iter().enumerate().find_map(|(i, row)| match row {
        Row::Channel(ch) if channel == Some(ch.id) => nav_of[i],
        _ => None,
    });
    let roved = common::rove(ui, Id::unique(("channel-list", server)), &nav, preferred);
    // Members have a text channel's menu ("…") as its action.
    let any_text = rows
        .iter()
        .any(|row| matches!(row, Row::Channel(ch) if ch.kind == ChannelKind::Text));
    let hint = if can_manage || any_text {
        common::ROW_HINT
    } else {
        common::LIST_HINT
    };
    common::list_hint(ui, list, Id::unique(("channel-list", server)), hint);
    // The open conversation shows its draft in the composer already.
    let drafts = crate::screens::chat::with_drafts(ui.ctx());
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(list));
    child.set_clip_rect(list.intersect(ui.clip_rect()));
    egui::ScrollArea::vertical()
        .id_salt(("sidebar-scroll", server))
        .max_height(list.height())
        .auto_shrink([false, false])
        .show(&mut child, |ui| {
            let (area, _) =
                ui.allocate_exact_size(Vec2::new(list.width(), content_h), egui::Sense::hover());
            let mut y = area.top();
            let left = area.left() + 8.0;
            let right = area.right() - 8.0;
            for (index, row) in rows.iter().enumerate() {
                let slot = Rect::from_min_max(pos2(left, y), pos2(right, y + row.slot()));
                y += row.slot();
                let at = nav_of[index];
                let stop = at.and_then(|r| roved.stop.filter(|(s, _)| *s == r).map(|(_, i)| i));
                let within = at.is_some_and(|r| roved.has_focus_within(r));
                // A channel's actions hide once focus leaves the row: Tab
                // comes back to its link (`useRovingRow` resetOnLeave).
                let stop = match row {
                    Row::Channel(_) if !within => stop.map(|_| 0),
                    _ => stop,
                };
                // The stop and the focused row stay in the frame off screen
                // too: egui drops focus from a widget it does not see.
                if !ui.is_rect_visible(slot) && stop.is_none() && !within {
                    continue;
                }
                if within
                    && ui.input(|i| {
                        i.key_pressed(egui::Key::ArrowDown) || i.key_pressed(egui::Key::ArrowUp)
                    })
                {
                    ui.scroll_to_rect(slot, None);
                }
                let keys = RowKeys { stop, within };
                match row {
                    Row::Section => {
                        caps(ui, slot, "Sprachkanäle", slot.width() - 11.0);
                    }
                    Row::Category(category) => {
                        category_row(ui, app, &theme, slot, server, category, can_manage, keys);
                    }
                    Row::Empty => {
                        let block = Block::single(ui, "Leer", text::META.line_height(1.5), c.muted);
                        paint_mid(ui.painter(), &block, slot.left() + 8.0, slot.center().y);
                    }
                    Row::Channel(ch) => {
                        let row_rect = Rect::from_min_size(slot.min, Vec2::new(slot.width(), 34.0));
                        let draft = drafts.contains(&ch.id)
                            && ch.kind == ChannelKind::Text
                            && channel != Some(ch.id);
                        channel_row(
                            ui,
                            app,
                            &theme,
                            row_rect,
                            server,
                            ch,
                            channel == Some(ch.id),
                            can_manage,
                            now,
                            draft,
                            keys,
                        );
                    }
                    Row::Person(user, occupant, live) => {
                        person_row(ui, app, &theme, slot, &detail, *user, occupant, *live);
                    }
                }
            }
        });
}

fn app_refresh(app: &AppCtx, server: Uuid) {
    app.client.refresh_server(server);
}

/// The rows of servers/rows.ts `sidebarRows`, each voice room followed by
/// the people in it.
fn build_rows(app: &AppCtx, detail: &gelabber_client::ServerDetail) -> Vec<Row> {
    let server = detail.server.id;
    let call = app.call.state();
    let voice_of = |channel: Uuid| -> Vec<Row> {
        app.client.with_state(|s| {
            crate::screens::voice::session::occupants(s, &call, server, channel, true)
                .into_iter()
                .map(|(user, o)| Row::Person(user, o, s.is_live(server, channel, user)))
                .collect()
        })
    };
    let mut rows = Vec::new();
    let push = |rows: &mut Vec<Row>, ch: &Channel| {
        let voice = ch.kind == ChannelKind::Voice;
        rows.push(Row::Channel(ch.clone()));
        if voice {
            rows.extend(voice_of(ch.id));
        }
    };
    for ch in detail
        .channels
        .iter()
        .filter(|c| c.category_id.is_none() && c.kind == ChannelKind::Text)
    {
        push(&mut rows, ch);
    }
    let loose_voice: Vec<&Channel> = detail
        .channels
        .iter()
        .filter(|c| c.category_id.is_none() && c.kind == ChannelKind::Voice)
        .collect();
    if !loose_voice.is_empty() {
        rows.push(Row::Section);
        for ch in loose_voice {
            push(&mut rows, ch);
        }
    }
    for category in &detail.categories {
        rows.push(Row::Category(category.clone()));
        let inside: Vec<&Channel> = detail
            .channels
            .iter()
            .filter(|c| c.category_id == Some(category.id))
            .collect();
        if inside.is_empty() {
            rows.push(Row::Empty);
        }
        for ch in inside {
            push(&mut rows, ch);
        }
    }
    rows
}

/// `channelOrder`: the channels in the order the sidebar lists them.
pub fn channel_order(detail: &gelabber_client::ServerDetail) -> Vec<Uuid> {
    let loose = |kind: ChannelKind| {
        detail
            .channels
            .iter()
            .filter(move |c| c.category_id.is_none() && c.kind == kind)
            .map(|c| c.id)
    };
    let mut out: Vec<Uuid> = loose(ChannelKind::Text)
        .chain(loose(ChannelKind::Voice))
        .collect();
    for category in &detail.categories {
        out.extend(
            detail
                .channels
                .iter()
                .filter(|c| c.category_id == Some(category.id))
                .map(|c| c.id),
        );
    }
    out
}

/// A caps heading whose text box ends 7px above the slot's bottom.
fn caps(ui: &Ui, slot: Rect, label: &str, max: f32) {
    let theme = Theme::of(ui.ctx());
    let block = line(ui, &label.to_uppercase(), text::CAPS, theme.c.muted, max);
    block.paint(
        ui.painter(),
        pos2(slot.left() + 8.0, slot.bottom() - 7.0 - block.height()),
    );
}

/// A row's part in its list's roving Tab stop.
#[derive(Clone, Copy, Debug, Default)]
struct RowKeys {
    /// The item of this row that Tab reaches, when the row holds the stop.
    stop: Option<usize>,
    /// Focus is inside the row: its actions show as on hover.
    within: bool,
}

#[allow(clippy::too_many_arguments)]
fn category_row(
    ui: &mut Ui,
    app: &mut AppCtx,
    theme: &Theme,
    slot: Rect,
    server: Uuid,
    category: &Category,
    can_manage: bool,
    keys: RowKeys,
) {
    // The 52px of hidden actions always take room (they stay in flow).
    let max = if can_manage {
        slot.width() - 3.0 - 52.0 - 5.0 - 8.0
    } else {
        slot.width() - 11.0
    };
    caps(ui, slot, &category.name, max);
    if !can_manage {
        return;
    }
    // The right-click menu only: no Tab stop of its own.
    let menu = ui.interact(
        slot,
        Id::unique(("category-menu", category.id)),
        egui::Sense::CLICK,
    );
    let owner = || Owner::Category(server, category.clone());
    if menu.secondary_clicked() {
        row_menu::open(ui.ctx(), owner(), slot, None);
    }
    let menu_open = row_menu::is_open_for(ui.ctx(), category.id);
    let hovered = ui.rect_contains_pointer(slot);
    if !hovered && !keys.within && keys.stop.is_none() && !menu_open {
        return;
    }
    let top = slot.top() + 3.0;
    let x0 = slot.right() - 3.0 - 52.0;
    let add = format!("Kanal in „{}“ erstellen", category.name);
    let more = format!("Aktionen für Kategorie „{}“", category.name);
    let actions = [(Icon::Plus, add.as_str()), (Icon::More, more.as_str())];
    // The row's only way in from the keyboard: these stay focusable (only
    // faded) while out of sight. Renaming and deleting are in the menu.
    let shown = hovered || keys.within || menu_open;
    for (i, (icon, label)) in actions.into_iter().enumerate() {
        let rect = Rect::from_min_size(pos2(x0 + 26.0 * i as f32, top), Vec2::splat(26.0));
        let id = Id::unique(("category-action", category.id, i));
        let stop = keys.stop == Some(i) || ui.memory(|m| m.has_focus(id));
        let response = if shown {
            shell_icon_button_stop(ui, rect, icon, 14.0, label, id, stop)
        } else {
            let response = common::click_stop(ui, rect, id, stop);
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
            response
        };
        if row_menu::menu_key(ui, &response) {
            row_menu::open(ui.ctx(), owner(), rect, Some(id));
        } else if response.clicked() && shown {
            match i {
                0 => super::dialogs::open_channel(ui.ctx(), server, None, Some(category.id)),
                _ => row_menu::open(ui.ctx(), owner(), rect, Some(id)),
            }
        }
    }
    let _ = (app, theme);
}

#[allow(clippy::too_many_arguments)]
fn channel_row(
    ui: &mut Ui,
    app: &mut AppCtx,
    theme: &Theme,
    row: Rect,
    server: Uuid,
    ch: &Channel,
    active: bool,
    can_manage: bool,
    now: chrono::DateTime<chrono::Utc>,
    draft: bool,
    keys: RowKeys,
) {
    let c = theme.c;
    let link_id = Id::unique(("channel-row", ch.id));
    let stop = keys.stop == Some(0) || ui.memory(|m| m.has_focus(link_id));
    let response = common::click_stop(ui, row, link_id, stop);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Link, true, &ch.name));
    let voice = ch.kind == ChannelKind::Voice;
    // Its menu: a text channel's notifications for everyone, editing and
    // deleting for those who manage channels.
    let has_menu = can_manage || !voice;
    if has_menu && (response.secondary_clicked() || row_menu::menu_key(ui, &response)) {
        row_menu::open(
            ui.ctx(),
            Owner::Channel(server, ch.clone()),
            row,
            Some(link_id),
        );
    }
    let menu_open = row_menu::is_open_for(ui.ctx(), ch.id);
    let own = crate::screens::voice::session::own_seat(app, server).map(|(c, _, _)| c);
    let (badge, muted, in_voice, live) = app.client.with_state(|s| {
        let read = s.read.get(&ch.id);
        (
            s.channel_badge(ch.id, now),
            read.is_some_and(|r| r.is_muted(now)),
            own == Some(ch.id),
            s.live
                .get(&server)
                .and_then(|l| l.get(&ch.id))
                .is_some_and(|users| !users.is_empty()),
        )
    });
    let hovered = response.hovered();
    let fill = if active {
        c.selected
    } else if hovered || menu_open {
        c.hover
    } else {
        Color32::TRANSPARENT
    };
    let painter = ui.painter().clone();
    common::fill(&painter, row, theme.radius.md, fill);
    let mut color = if active || hovered || in_voice {
        c.text
    } else {
        c.muted
    };
    // `.is-in-voice` comes after `.is-active` in index.css.
    let icon_color = if in_voice {
        c.online_text
    } else if active {
        c.accent_text
    } else {
        color
    };
    let unread = !voice && (badge.count > 0 || badge.dot);
    if unread {
        common::edge_bar(&painter, theme, row.left() - 8.0, row.center().y);
        color = c.text;
    }
    icons::paint(
        &painter,
        if voice { Icon::Speaker } else { Icon::Hash },
        pos2(row.left() + 10.0, row.center().y - 8.5),
        17.0,
        icon_color,
    );
    let name_left = row.left() + 10.0 + 17.0 + 10.0;
    let mut name_right = row.right() - 3.0 - 10.0;
    // The count pill or the LIVE chip sit at the end of the link.
    if badge.count > 0 && !voice {
        let r = count_badge(
            &painter,
            ui,
            pos2(name_right, row.center().y + 9.0),
            badge.count.min(u32::MAX as i64) as u32,
            None,
        );
        name_right = r.left() - 10.0;
    } else if live && voice {
        let chip_style = text::TextStyle::new(10.0, 700.0, 1.5).tracking(0.04);
        let block = Block::single(ui, "LIVE", chip_style, c.on_live);
        let chip = Rect::from_min_max(
            pos2(name_right - block.width() - 10.0, row.center().y - 8.5),
            pos2(name_right, row.center().y + 8.5),
        );
        painter.rect_filled(chip, corner(theme.radius.sm), c.live);
        paint_mid(&painter, &block, chip.left() + 5.0, chip.center().y);
        name_right = chip.left() - 10.0;
    }
    let style = if unread {
        text::BODY.weight(600.0)
    } else {
        text::BODY
    };
    let under = if fill == Color32::TRANSPARENT {
        c.panel
    } else {
        fill
    };
    let name_color = if muted && !voice {
        mix(under, color, 0.5)
    } else {
        color
    };
    if draft {
        // `.channel-row-draft`: the pencil titled "Entwurf" before the badge.
        let pencil = Rect::from_min_size(
            pos2(name_right - 13.0, row.center().y - 6.5),
            Vec2::splat(13.0),
        );
        icons::paint(&painter, Icon::Pencil, pencil.min, 13.0, c.muted);
        let mark = ui.interact(pencil, link_id.with("draft"), egui::Sense::hover());
        crate::widgets::announce(&mark, egui::Role::Label, "Entwurf");
        mark.on_hover_text("Entwurf");
        name_right = pencil.left() - 10.0;
    }
    let name = line(ui, &ch.name, style, name_color, name_right - name_left);
    paint_mid(&painter, &name, name_left, row.center().y);
    common::focus_visible(ui, &response, row, theme.radius.md);
    if in_voice {
        response
            .clone()
            .on_hover_text(format!("{} · verbunden", ch.name));
    }

    // Hover (or focus) actions cover the name's end with the row's own fill:
    // edit for those who manage channels, then "…" for the menu, which holds
    // deleting. Out of sight they are not drawn at all, so neither screen
    // readers nor Tab meet them; the link is the row's way in.
    let shown = has_menu && (hovered || ui.rect_contains_pointer(row) || keys.within || menu_open);
    if shown {
        let label = if voice {
            ch.name.clone()
        } else {
            format!("#{}", ch.name)
        };
        let count = if can_manage { 2.0 } else { 1.0 };
        let actions = Rect::from_min_max(
            pos2(row.right() - 3.0 - 6.0 - 26.0 * count, row.top() + 4.0),
            pos2(row.right() - 3.0, row.top() + 30.0),
        );
        painter.rect(
            actions,
            corner(theme.radius.md),
            under,
            Stroke::NONE,
            StrokeKind::Inside,
        );
        let focused = ui.memory(|m| m.focused());
        let stop_at = |i: usize, id: Id| keys.stop == Some(i) || focused == Some(id);
        let mut x = actions.left() + 6.0;
        if can_manage {
            let edit_id = Id::unique(("edit-channel", ch.id));
            let edit = Rect::from_min_size(pos2(x, actions.top()), Vec2::splat(26.0));
            x += 26.0;
            let edit = shell_icon_button_stop(
                ui,
                edit,
                Icon::Pencil,
                14.0,
                &format!("{label} bearbeiten"),
                edit_id,
                stop_at(1, edit_id),
            );
            if row_menu::menu_key(ui, &edit) {
                row_menu::open(
                    ui.ctx(),
                    Owner::Channel(server, ch.clone()),
                    row,
                    Some(edit_id),
                );
            } else if edit.clicked() {
                super::dialogs::open_channel(ui.ctx(), server, Some(ch), ch.category_id);
                return;
            }
        }
        let more_id = Id::unique(("channel-more", ch.id));
        let more = Rect::from_min_size(pos2(x, actions.top()), Vec2::splat(26.0));
        let index = if can_manage { 2 } else { 1 };
        let response = shell_icon_button_stop(
            ui,
            more,
            Icon::More,
            14.0,
            &format!("Aktionen für {label}"),
            more_id,
            stop_at(index, more_id),
        );
        if response.clicked() || row_menu::menu_key(ui, &response) {
            row_menu::open(
                ui.ctx(),
                Owner::Channel(server, ch.clone()),
                more,
                Some(more_id),
            );
            return;
        }
    }
    if response.clicked() {
        app.nav.navigate(&format!("/s/{server}/c/{}", ch.id));
    }
}

#[allow(clippy::too_many_arguments)]
fn person_row(
    ui: &mut Ui,
    app: &mut AppCtx,
    theme: &Theme,
    row: Rect,
    detail: &gelabber_client::ServerDetail,
    user: Uuid,
    occupant: &VoiceOccupant,
    live: bool,
) {
    let c = theme.c;
    let me = app.client.with_state(|s| s.me().map(|m| m.id));
    let member = detail.members.iter().find(|m| m.user_id == user);
    let name = member.map_or("Jemand", |m| m.name.as_str());
    let title = if me == Some(user) {
        "Dein Profil"
    } else {
        "Profil und Lautstärke"
    };
    let response = common::click(
        ui,
        row,
        Id::unique(("voice-person", occupant.channel_id, user)),
    )
    .on_hover_text(title);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, name));
    // A click or a right click: the profile with its volume controls.
    if response.clicked() || response.secondary_clicked() {
        super::overlays::open_member(ui.ctx(), detail.server.id, user);
    }
    let painter = ui.painter().clone();
    if response.hovered() {
        common::fill(&painter, row, theme.radius.md, c.hover);
    }
    let under = if response.hovered() { c.hover } else { c.panel };
    let avatar = Rect::from_min_size(
        pos2(row.left() + 36.0, row.center().y - 10.0),
        Vec2::splat(20.0),
    );
    let speaking = app.call.with_state(|call| {
        crate::screens::voice::session::speaking(call, user, occupant.muted, occupant.deafened)
    });
    if speaking {
        crate::screens::voice::look::speaking_ring(ui, &painter, avatar, true);
    }
    common::avatar(
        ui,
        app.images,
        avatar,
        name,
        member.and_then(|m| m.avatar_url.as_deref()),
        9.0,
        1.0,
        under,
    );
    let mut right = row.right() - 8.0;
    let mut marks: Vec<(Icon, f32, Color32)> = Vec::new();
    if live {
        marks.push((Icon::Live, 13.0, c.live_text));
    }
    if occupant.deafened {
        marks.push((Icon::HeadsetOff, 13.0, c.danger));
    } else if occupant.muted {
        marks.push((Icon::MicOff, 13.0, c.danger));
    }
    for (icon, size, color) in marks.iter().rev() {
        right -= size;
        icons::paint(
            &painter,
            *icon,
            pos2(right, row.center().y - size / 2.0),
            *size,
            *color,
        );
        right -= 8.0;
    }
    let left = avatar.right() + 8.0;
    let block = line(
        ui,
        name,
        text::UI.weight(400.0).line_height(1.5),
        if speaking { c.text } else { c.muted },
        right - left,
    );
    paint_mid(&painter, &block, left, row.center().y);
}

/// `.shell-icon-button`: transparent, muted, hover fill and text colour.
pub fn shell_icon_button(
    ui: &mut Ui,
    rect: Rect,
    icon: Icon,
    size: f32,
    label: &str,
    id: Id,
) -> egui::Response {
    shell_icon_button_stop(ui, rect, icon, size, label, id, true)
}

/// Like [`shell_icon_button`]; `stop` whether Tab reaches it.
pub fn shell_icon_button_stop(
    ui: &mut Ui,
    rect: Rect,
    icon: Icon,
    size: f32,
    label: &str,
    id: Id,
    stop: bool,
) -> egui::Response {
    let theme = Theme::of(ui.ctx());
    let response = common::click_stop(ui, rect, id, stop).on_hover_text(label);
    common::focus_visible(ui, &response, rect, theme.radius.md);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    let color = if response.hovered() {
        common::fill(ui.painter(), rect, theme.radius.md, theme.c.hover);
        theme.c.text
    } else {
        theme.c.muted
    };
    icons::paint_centered(ui.painter(), icon, rect, size, color);
    response
}

/// The server menu under the name (`.shell-menu`): 208 wide at least,
/// 36px items with muted icons.
#[allow(clippy::too_many_arguments)]
fn server_menu(
    ui: &mut Ui,
    app: &mut AppCtx,
    anchor: Rect,
    server: Uuid,
    name: &str,
    muted: bool,
    can_manage: bool,
    can_settings: bool,
) {
    let mut items: Vec<(Icon, &str, u8)> = vec![
        (Icon::Link, "Einladen", 0),
        (
            if muted { Icon::BellOff } else { Icon::Bell },
            "Benachrichtigungen…",
            1,
        ),
    ];
    if can_manage {
        items.push((Icon::Plus, "Kanal erstellen", 2));
        items.push((Icon::Plus, "Kategorie erstellen", 3));
    }
    if can_settings {
        items.push((Icon::Gear, "Servereinstellungen", 4));
    } else {
        items.push((Icon::Users, "Mitglieder", 4));
    }
    let width = anchor.width().max(208.0);
    let picked = Popover::new(("server-menu", server), anchor)
        .min_width(width)
        .show(ui.ctx(), |ui| {
            let mut picked = None;
            ui.spacing_mut().item_spacing.y = 2.0;
            for (icon, label, action) in &items {
                if menu_row(ui, *icon, label, width - 18.0).clicked() {
                    picked = Some(*action);
                }
            }
            picked
        });
    match picked {
        None => {
            // Esc or a press outside; a press on the button toggles itself.
            let on_button =
                ui.input(|i| i.pointer.interact_pos().is_some_and(|p| anchor.contains(p)));
            if !on_button {
                state::with(ui.ctx(), |s| s.server_menu = false);
            }
        }
        Some(None) => {}
        Some(Some(action)) => {
            state::with(ui.ctx(), |s| s.server_menu = false);
            match action {
                0 => super::dialogs::open_invite(ui.ctx(), server, name),
                1 => super::overlays::open_notifications(
                    ui.ctx(),
                    super::overlays::Target::server(server, name),
                ),
                2 => super::dialogs::open_channel(ui.ctx(), server, None, None),
                3 => super::dialogs::open_category(ui.ctx(), server, None),
                _ => app.nav.navigate(&format!("/s/{server}/settings")),
            }
        }
    }
}

/// `.shell-menu-item` of the server menu: min-height 36, padding 0 10,
/// gap 10, radius 5, 16px icons.
fn menu_row(ui: &mut Ui, icon: Icon, label: &str, width: f32) -> egui::Response {
    let theme = Theme::of(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 36.0), egui::Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    if response.hovered() {
        common::fill(ui.painter(), rect, theme.radius.sm, theme.c.hover);
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    icons::paint(
        ui.painter(),
        icon,
        pos2(rect.left() + 10.0, rect.center().y - 8.0),
        16.0,
        theme.c.muted,
    );
    let block = Block::single(ui, label, text::UI, theme.c.text);
    paint_mid(ui.painter(), &block, rect.left() + 36.0, rect.center().y);
    response
}

/// `SidebarPlaceholder`: the brand line, four pulsing bars while loading,
/// or the failure with "Erneut versuchen".
pub fn placeholder(
    ui: &mut Ui,
    app: &AppCtx,
    rect: Rect,
    failure: Option<&str>,
    retry: &mut dyn FnMut(),
) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    brand(ui, rect);
    let painter = ui.painter().clone();
    // `aria-label` of the <aside>, and its role="status" text.
    let region = ui.interact(
        rect,
        Id::unique("sidebar-placeholder"),
        egui::Sense::hover(),
    );
    crate::widgets::announce(
        &region,
        egui::Role::Navigation,
        if failure.is_some() {
            "Navigation"
        } else {
            "Navigation wird geladen"
        },
    );
    match failure {
        None => {
            let t = ui.input(|i| i.time) as f32;
            // Tailwind animate-pulse: 1 → .5 → 1 over 2s.
            let k = 0.75 + 0.25 * (t * std::f32::consts::PI).cos();
            for i in 0..4 {
                let bar = Rect::from_min_size(
                    pos2(
                        rect.left() + 18.0,
                        rect.top() + 52.0 + 24.0 + 36.0 * i as f32,
                    ),
                    Vec2::new(rect.width() - 1.0 - 36.0, 22.0),
                );
                painter.rect_filled(bar, corner(theme.radius.md), c.surface.gamma_multiply(k));
            }
            ui.ctx().request_repaint();
        }
        Some(message) => {
            let block = Block::wrapped(
                ui,
                message,
                text::BODY.line_height(1.6),
                c.muted,
                rect.width() - 45.0,
            );
            let top = rect.top() + 52.0 + 24.0;
            block.paint(&painter, pos2(rect.left() + 22.0, top));
            let status = ui.interact(
                Rect::from_min_size(pos2(rect.left() + 22.0, top), block.size()),
                Id::unique("sidebar-failure"),
                egui::Sense::hover(),
            );
            crate::widgets::announce(&status, egui::Role::Status, message);
            let busy = app.client.busy();
            let label = if busy {
                "Wird geladen…"
            } else {
                "Erneut versuchen"
            };
            let lb = Block::single(ui, label, text::BODY.weight(600.0), c.text);
            let button = Rect::from_min_size(
                pos2(rect.left() + 22.0, top + block.height() + 12.0),
                Vec2::new(lb.width() + 26.0, 36.0),
            );
            let response = common::click(ui, button, Id::unique("sidebar-retry"));
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, !busy, label));
            painter.rect(
                button,
                corner(theme.radius.sm),
                if response.hovered() {
                    c.hover
                } else {
                    Color32::TRANSPARENT
                },
                Stroke::new(1.0, c.border),
                StrokeKind::Inside,
            );
            paint_mid(&painter, &lb, button.left() + 13.0, button.center().y);
            if response.clicked() && !busy {
                retry();
            }
        }
    }
}

/// `.sidebar-brand`: "Gelabber" on the 52px header line.
pub fn brand(ui: &Ui, rect: Rect) {
    let theme = Theme::of(ui.ctx());
    let style = text::TextStyle::new(16.0, 700.0, 1.5).tracking(-0.3 / 16.0);
    let block = Block::single(ui, "Gelabber", style, theme.c.text);
    paint_mid(ui.painter(), &block, rect.left() + 16.0, rect.top() + 25.5);
    hairline_h(
        ui.painter(),
        &theme,
        rect.left()..=rect.right() - 1.0,
        rect.top() + 51.0,
    );
}
