//! Channels limited to roles (v0.10 roles, #516; web servers/roles.ts,
//! servers/channelRequest.ts, components/ChannelAccessField.tsx,
//! channelAccess.css): the "Zugriff" field of the channel dialog, the lock
//! on a limited channel, and who may see one.

use egui::{Id, Painter, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder, Vec2, pos2, vec2};
use gelabber_client::{Channel, Member, Role, ServerDetail, ServerRole};
use gelabber_i18n::{roles as role_texts, servers};
use uuid::Uuid;

use super::common::{line, paint_mid};
use crate::icons::{self, Icon};
use crate::screens::server_settings::roles::dot;
use crate::screens::settings::controls;
use crate::text::{self, Block};
use crate::theme::{Theme, corner, role_color};
use crate::widgets::{Checkbox, Popover};

/// A channel's access as the "Zugriff" field edits it (`AccessSelection`):
/// `None` is "Alle Mitglieder", a list the roles it is limited to. A
/// limited channel whose roles were all deleted is `Some([])` ("Nur
/// Inhaber").
pub type Selection = Option<Vec<Uuid>>;

/// `accessOf`.
pub fn of(channel: &Channel) -> Selection {
    channel.limited.then(|| channel.role_ids.clone())
}

/// One click in the checklist (`toggleAccess`): `None` is the "Alle
/// Mitglieder" row. Choosing it clears the roles; choosing a role clears
/// it; unchecking the last role chooses it again.
pub fn toggle(selection: &Selection, id: Option<Uuid>) -> Selection {
    let id = id?;
    match selection {
        None => Some(vec![id]),
        Some(held) if held.contains(&id) => {
            let rest: Vec<Uuid> = held.iter().copied().filter(|r| *r != id).collect();
            (!rest.is_empty()).then_some(rest)
        }
        Some(held) => Some(held.iter().copied().chain([id]).collect()),
    }
}

/// Whether two selections allow the same people (`sameAccess`).
pub fn same(a: &Selection, b: &Selection) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => a.len() == b.len() && b.iter().all(|id| a.contains(id)),
        _ => false,
    }
}

/// The custom roles by position.
fn in_order(roles: &[ServerRole]) -> Vec<&ServerRole> {
    let mut list: Vec<&ServerRole> = roles.iter().collect();
    list.sort_by(|a, b| a.position.cmp(&b.position).then(a.id.cmp(&b.id)));
    list
}

/// What the field reads (`accessSummary`): "Alle Mitglieder", the role
/// names by position, or "Nur Inhaber".
pub fn summary(selection: &Selection, roles: &[ServerRole]) -> String {
    let Some(held) = selection else {
        return role_texts::base().to_owned();
    };
    let names: Vec<&str> = in_order(roles)
        .into_iter()
        .filter(|r| held.contains(&r.id))
        .map(|r| r.name.as_str())
        .collect();
    if names.is_empty() {
        role_texts::owner_only().to_owned()
    } else {
        names.join(", ")
    }
}

/// The `role_ids` the dialog sends (`channelRequest`): `None` while the
/// access did not change (an owner-only channel must not open by an
/// unasked `[]`), `[]` for "Alle Mitglieder", else the roles. Roles deleted
/// since the dialog opened drop out; when all chosen roles are gone the
/// ids go along, so the server refuses rather than the channel opening.
pub fn role_ids(
    initial: &Selection,
    selection: &Selection,
    roles: &[ServerRole],
) -> Option<Vec<Uuid>> {
    let live = |s: &Selection| {
        s.as_ref().map(|ids| {
            ids.iter()
                .copied()
                .filter(|id| roles.iter().any(|r| r.id == *id))
                .collect::<Vec<_>>()
        })
    };
    let (initial_live, access) = (live(initial), live(selection));
    let gone = access.as_ref().is_some_and(Vec::is_empty)
        && selection.as_ref().is_some_and(|s| !s.is_empty())
        && !same(&initial_live, &access);
    if gone {
        return selection.clone();
    }
    if same(&initial_live, &access) {
        return None;
    }
    Some(access.unwrap_or_default())
}

/// Whether the member may see the channel (`seesChannel`, the server's
/// `Membership::sees`): the owner always, everyone an open channel, a
/// limited one needs one of its roles.
pub fn sees(detail: &ServerDetail, member: &Member, channel: &Channel) -> bool {
    member.role == Role::Owner
        || member.user_id == detail.server.owner_id
        || !channel.limited
        || channel
            .role_ids
            .iter()
            .any(|id| member.role_ids.contains(id))
}

/// Who the `@…` suggestions offer in the channel (`membersWhoSee`).
pub fn members_who_see<'a>(detail: &'a ServerDetail, channel: &Channel) -> Vec<&'a Member> {
    detail
        .members
        .iter()
        .filter(|m| sees(detail, m, channel))
        .collect()
}

/// Whether the channel is limited, from the loaded server.
pub fn is_limited(app: &crate::app::AppCtx, server: Uuid, channel: Uuid) -> bool {
    app.client.with_state(|s| {
        s.detail(server)
            .and_then(|d| d.channels.iter().find(|c| c.id == channel))
            .is_some_and(|c| c.limited)
    })
}

/// The muted 14px lock with its left edge at `x`, centred on `mid`
/// (`.channel-row-lock`, `.lr-channel-title-lock`), named "Nur für
/// bestimmte Rollen" as the web's `role="img"`. Returns its rect.
pub fn lock(ui: &Ui, painter: &Painter, id: Id, x: f32, mid: f32) -> Rect {
    let theme = Theme::of(ui.ctx());
    let rect = Rect::from_min_size(pos2(x, mid - 7.0), Vec2::splat(14.0));
    icons::paint(painter, Icon::Lock, rect.min, 14.0, theme.c.muted);
    let mark = ui.interact(rect, id.with("lock"), Sense::hover());
    crate::widgets::announce(&mark, egui::Role::Image, role_texts::limited_label());
    rect
}

/// "Zugriff": a `FormField` with a trigger drawn like the "Kategorie"
/// select, which opens a checklist of "Alle Mitglieder" and the roles, and
/// the hint under it.
pub fn field(ui: &mut Ui, id: Id, roles: &[ServerRole], value: &mut Selection) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        let label = servers::channel_access_label();
        text::label(ui, label, text::UI, c.text);
        let mut open: bool = ui.data(|d| d.get_temp(id)).unwrap_or(false);
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 37.0), Sense::click());
        let rect = crate::widgets::snap(ui.ctx(), rect);
        let shown = summary(value, roles);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
        // `aria-describedby`: the summary, then the hint.
        let described = format!("{shown} {}", servers::channel_access_hint());
        ui.ctx()
            .accesskit_node_builder(response.id, |node| node.set_description(described));
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if response.clicked() {
            open = !open;
        }
        let painter = ui.painter();
        // The open list holds the focus, so the trigger shows no ring then.
        let focused = response.has_focus() && crate::widgets::keyboard_focus(ui.ctx());
        if focused {
            crate::widgets::focus_ring(painter, rect, theme.radius.sm, &theme);
        }
        painter.rect(
            rect,
            corner(theme.radius.sm),
            c.bg,
            Stroke::new(1.0, if focused { c.focus } else { c.border_strong }),
            StrokeKind::Inside,
        );
        // `.channel-access-trigger`: 12 in, 34 kept for the chevron.
        let block = line(ui, &shown, text::BODY, c.text, rect.width() - 12.0 - 34.0);
        paint_mid(painter, &block, rect.left() + 12.0, rect.center().y);
        icons::paint(
            painter,
            Icon::Chevron,
            pos2(rect.right() - 11.0 - 14.0, rect.center().y - 7.0),
            14.0,
            c.muted,
        );
        let hint = Block::wrapped(
            ui,
            servers::channel_access_hint(),
            text::META,
            c.muted,
            ui.available_width(),
        );
        text::show(ui, &hint);
        if open {
            let picked = Popover::new(id, rect)
                .match_anchor_width()
                .show(ui.ctx(), |ui| checklist(ui, roles, value));
            match picked {
                None => open = false,
                Some(Some(row)) => *value = toggle(value, row),
                Some(None) => {}
            }
        }
        ui.data_mut(|d| d.insert_temp(id, open));
    });
}

/// `.channel-access__list`: a 36px checkbox row for "Alle Mitglieder"
/// (muted dot) and one per role with its dot; with no roles, the hint
/// where to make them. Returns the row pressed: `Some(None)` is "Alle
/// Mitglieder".
fn checklist(ui: &mut Ui, roles: &[ServerRole], value: &Selection) -> Option<Option<Uuid>> {
    let theme = Theme::of(ui.ctx());
    let c = &theme.c;
    ui.spacing_mut().item_spacing = Vec2::ZERO;
    let width = ui.available_width();
    let mut pressed = None;
    let base = role_texts::base();
    let rows = std::iter::once((None, base, c.muted, value.is_none())).chain(
        in_order(roles).into_iter().map(|r| {
            (
                Some(r.id),
                r.name.as_str(),
                role_color(r.color),
                value.as_ref().is_some_and(|v| v.contains(&r.id)),
            )
        }),
    );
    for (row_id, name, fill, on) in rows {
        let (row, _) = ui.allocate_exact_size(vec2(width, 36.0), Sense::hover());
        let response = ui.interact(
            row,
            Id::unique(("gelabber-channel-access-row", row_id)),
            Sense::click(),
        );
        response.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, on, name));
        let mid = row.center().y;
        // `padding-inline: 4px`, the 20px box, then a 12 gap.
        let check = Rect::from_min_size(pos2(row.left() + 4.0, mid - 10.0), vec2(20.0, 20.0));
        let mut shown = on;
        controls::place(ui, UiBuilder::new().max_rect(check), |ui| {
            ui.add(Checkbox::new(&mut shown).decorative())
        });
        let painter = ui.painter();
        dot(painter, pos2(check.right() + 12.0 + 5.0, mid), fill, &theme);
        let x = check.right() + 12.0 + 10.0 + 10.0;
        let block = line(ui, name, text::UI, c.text, row.right() - 4.0 - x);
        paint_mid(painter, &block, x, mid);
        if response.has_focus() && crate::widgets::keyboard_focus(ui.ctx()) {
            crate::widgets::focus_ring(painter, row, theme.radius.sm, &theme);
        }
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if response.clicked() {
            pressed = Some(row_id);
        }
    }
    if roles.is_empty() {
        // `.channel-access__empty`: 4 above, padding 4, meta muted.
        let b = Block::wrapped(
            ui,
            servers::channel_access_no_roles(),
            text::META,
            c.muted,
            width - 8.0,
        );
        let (er, _) = ui.allocate_exact_size(vec2(width, 4.0 + b.height() + 8.0), Sense::hover());
        b.paint(ui.painter(), pos2(er.left() + 4.0, er.top() + 4.0 + 4.0));
    }
    pressed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn role(id: u128, name: &str, position: i32) -> ServerRole {
        ServerRole {
            id: Uuid::from_u128(id),
            server_id: Uuid::nil(),
            name: name.into(),
            color: gelabber_client::RoleColor::Blue,
            permissions: Vec::new(),
            position,
            created_at: chrono::DateTime::from_timestamp(0, 0).unwrap(),
        }
    }

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    /// The cases of servers/roles.test.ts for `toggleAccess`,
    /// `accessSummary` and `sameAccess`.
    #[test]
    fn access_follows_the_web() {
        assert_eq!(toggle(&None, None), None);
        assert_eq!(toggle(&None, Some(id(1))), Some(vec![id(1)]));
        assert_eq!(
            toggle(&Some(vec![id(1)]), Some(id(2))),
            Some(vec![id(1), id(2)])
        );
        assert_eq!(
            toggle(&Some(vec![id(1), id(2)]), Some(id(1))),
            Some(vec![id(2)])
        );
        assert_eq!(toggle(&Some(vec![id(1)]), Some(id(1))), None);
        assert_eq!(toggle(&Some(vec![id(1)]), None), None);

        assert!(same(&None, &None));
        assert!(same(&Some(vec![id(1), id(2)]), &Some(vec![id(2), id(1)])));
        assert!(!same(&None, &Some(vec![])));
        assert!(!same(&Some(vec![id(1)]), &Some(vec![id(1), id(2)])));

        let roles = [role(2, "Stammgäste", 1), role(1, "Moderation", 0)];
        assert_eq!(summary(&None, &roles), "Alle Mitglieder");
        assert_eq!(
            summary(&Some(vec![id(2), id(1)]), &roles),
            "Moderation, Stammgäste"
        );
        assert_eq!(summary(&Some(vec![]), &roles), "Nur Inhaber");
        assert_eq!(summary(&Some(vec![id(9)]), &roles), "Nur Inhaber");
    }

    /// servers/channelRequest.ts: what the dialog sends.
    #[test]
    fn the_dialog_sends_access_only_when_it_changed() {
        let roles = [role(1, "Moderation", 0), role(2, "Stammgäste", 1)];
        // Unchanged: nothing, also for an owner-only channel.
        assert_eq!(role_ids(&None, &None, &roles), None);
        assert_eq!(role_ids(&Some(vec![]), &Some(vec![]), &roles), None);
        assert_eq!(
            role_ids(&Some(vec![id(1)]), &Some(vec![id(1)]), &roles),
            None
        );
        // Opened to everyone, limited, changed.
        assert_eq!(role_ids(&Some(vec![id(1)]), &None, &roles), Some(vec![]));
        assert_eq!(
            role_ids(&None, &Some(vec![id(1), id(2)]), &roles),
            Some(vec![id(1), id(2)])
        );
        assert_eq!(
            role_ids(&Some(vec![id(1)]), &Some(vec![id(2)]), &roles),
            Some(vec![id(2)])
        );
        // A role deleted meanwhile drops out; all gone: the ids go along.
        assert_eq!(
            role_ids(&None, &Some(vec![id(1), id(9)]), &roles),
            Some(vec![id(1)])
        );
        assert_eq!(
            role_ids(&None, &Some(vec![id(9)]), &roles),
            Some(vec![id(9)])
        );
    }

    fn detail(channel_roles: &[Uuid], limited: bool) -> (ServerDetail, Channel) {
        let owner = Uuid::from_u128(100);
        let detail: ServerDetail = serde_json::from_value(serde_json::json!({
            "id": Uuid::nil(), "name": "Wohnzimmer", "owner_id": owner,
            "icon_url": null, "created_at": "2026-01-01T00:00:00Z", "role": "owner",
            "member_permissions": [], "permissions": [],
            "categories": [], "channels": [],
            "members": [
                {"user_id": owner, "name": "Lena", "avatar_url": null,
                 "joined_at": "2026-01-01T00:00:00Z", "status_text": null,
                 "accent_color": null, "avatar_decoration": null, "role": "owner"},
                {"user_id": Uuid::from_u128(101), "name": "Jonas", "avatar_url": null,
                 "joined_at": "2026-01-01T00:00:00Z", "status_text": null,
                 "accent_color": null, "avatar_decoration": null, "role": "member",
                 "role_ids": [id(1)]},
                {"user_id": Uuid::from_u128(102), "name": "Ada", "avatar_url": null,
                 "joined_at": "2026-01-01T00:00:00Z", "status_text": null,
                 "accent_color": null, "avatar_decoration": null, "role": "member"},
            ],
        }))
        .expect("detail");
        let channel: Channel = serde_json::from_value(serde_json::json!({
            "id": Uuid::from_u128(50), "server_id": Uuid::nil(), "category_id": null,
            "name": "spiele", "kind": "text", "created_at": "2026-01-01T00:00:00Z",
            "limited": limited, "role_ids": channel_roles,
        }))
        .expect("channel");
        (detail, channel)
    }

    /// `seesChannel` and `membersWhoSee`: the `@…` suggestions of a
    /// limited channel offer only who can read it.
    #[test]
    fn only_who_sees_a_limited_channel_is_offered() {
        let names = |d: &ServerDetail, c: &Channel| {
            members_who_see(d, c)
                .into_iter()
                .map(|m| m.name.clone())
                .collect::<Vec<_>>()
        };
        let (d, c) = detail(&[], false);
        assert_eq!(names(&d, &c), ["Lena", "Jonas", "Ada"]);
        let (d, c) = detail(&[id(1)], true);
        assert_eq!(names(&d, &c), ["Lena", "Jonas"]);
        // The last role deleted: the owner's alone.
        let (d, c) = detail(&[], true);
        assert_eq!(names(&d, &c), ["Lena"]);
    }
}
