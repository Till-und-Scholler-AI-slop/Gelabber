//! Server settings, v0.10 roles (#516; web pages/ServerRoles.tsx,
//! serverRoles.css, servers/roles.ts): the "Rollen" editor (the list and
//! the chosen role's panel), and the role chips and picker on a member
//! row. Every change saves at once and shows ahead of the server's answer,
//! as the web's optimistic writes do.

use std::collections::{HashMap, HashSet};

use egui::{Color32, Id, Painter, Pos2, Rect, Sense, Stroke, Ui, UiBuilder, pos2, vec2};
use gelabber_client::{
    ApiError, Member, NewRole, OpState, Permission, RoleColor, RolePatch, ServerDetail, ServerRole,
};
use gelabber_i18n::{common, roles as role_texts, server_settings as texts, servers};
use uuid::Uuid;

use super::PERMISSIONS;
use crate::app::AppCtx;
use crate::icons::{self, Icon};
use crate::rules::{Failure, FormErrors, ROLE_NAME_MAX, field_message, validate_role_name};
use crate::screens::settings::controls::{self, Confirm, Status};
use crate::text::{self, Block, TextStyle};
use crate::theme::{Theme, corner, role_color};
use crate::widgets::{Button, ButtonVariant, Checkbox, Popover, Switch, TextField, eased};

/// A row of the roles list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Choice {
    Owner,
    Role(Uuid),
    /// "Alle Mitglieder": `member_permissions`.
    Base,
}

/// A value shown ahead of the server's answer; `settled` once the write
/// is in, so a detail that never comes back the same cannot keep it.
struct Ahead<T> {
    value: T,
    settled: Option<f64>,
}

impl<T> Ahead<T> {
    fn new(value: T) -> Self {
        Self {
            value,
            settled: None,
        }
    }
}

/// How long a settled value may wait for the detail to agree.
const SETTLE_SECS: f64 = 3.0;

/// A write on its way, by what it does. The quick ones carry their own
/// number: each settles from its own request's answer.
#[derive(Clone, Debug, PartialEq)]
enum Write {
    /// "Neue Rolle", with the name it sent.
    Create(String),
    /// Colour or permissions.
    Role(Uuid, u64),
    Rename(Uuid, u64),
    Delete(Uuid),
    /// A member's roles, by user.
    Member(Uuid, u64),
    /// "Alle Mitglieder".
    Base(u64),
}

impl Write {
    /// The client's op key (commands.rs).
    fn key(&self, server: Uuid) -> String {
        match self {
            Self::Create(_) => format!("create_role:{server}"),
            Self::Role(id, seq) | Self::Rename(id, seq) => format!("role:{id}:{seq}"),
            Self::Delete(id) => format!("delete_role:{id}"),
            Self::Member(user, seq) => format!("member_roles:{user}:{seq}"),
            Self::Base(seq) => format!("server_update:{server}:{seq}"),
        }
    }

    /// Whether `other` writes the same value ahead of the server.
    fn same_value(&self, other: &Write) -> bool {
        match (self, other) {
            (Self::Role(a, _), Self::Role(b, _)) | (Self::Member(a, _), Self::Member(b, _)) => {
                a == b
            }
            (Self::Base(_), Self::Base(_)) => true,
            _ => false,
        }
    }
}

/// The name field of the chosen role: its text and the name it last took
/// over from the server (web `RoleNameField`).
struct NameField {
    role: Uuid,
    text: String,
    synced: String,
}

#[derive(Default)]
pub(super) struct Roles {
    chosen: Option<Choice>,
    name: Option<NameField>,
    name_errors: FormErrors,
    /// The panel's `InlineStatus`.
    pub(super) status: Status,
    /// The new role whose name field takes the focus.
    focus_name: Option<Uuid>,
    writes: Vec<Write>,
    permissions: HashMap<Uuid, Ahead<Vec<Permission>>>,
    colors: HashMap<Uuid, Ahead<RoleColor>>,
    deleted: HashSet<Uuid>,
    /// Members' role ids, by user.
    members: HashMap<Uuid, Ahead<Vec<Uuid>>>,
    /// "Alle Mitglieder" ahead of the server's answer.
    base: Option<Ahead<Vec<Permission>>>,
    /// The last write's number.
    seq: u64,
    /// The member whose picker is open.
    pub(super) picker: Option<Uuid>,
}

// ------------------------------------------------------------- helpers

/// Custom roles by position (`rolesInOrder`), without `hidden` ones.
fn in_order(roles: &[ServerRole], hidden: &HashSet<Uuid>) -> Vec<ServerRole> {
    let mut list: Vec<ServerRole> = roles
        .iter()
        .filter(|r| !hidden.contains(&r.id))
        .cloned()
        .collect();
    list.sort_by(|a, b| a.position.cmp(&b.position).then(a.id.cmp(&b.id)));
    list
}

/// How many members hold each custom role (`roleCounts`).
fn counts(detail: &ServerDetail) -> HashMap<Uuid, usize> {
    let mut counts: HashMap<Uuid, usize> = detail.roles.iter().map(|r| (r.id, 0)).collect();
    for member in &detail.members {
        let held: HashSet<&Uuid> = member.role_ids.iter().collect();
        for id in held {
            if let Some(n) = counts.get_mut(id) {
                *n += 1;
            }
        }
    }
    counts
}

/// The colour for a new role: the first one no role uses, else amber.
fn next_color(roles: &[ServerRole]) -> RoleColor {
    RoleColor::ALL
        .into_iter()
        .find(|c| !roles.iter().any(|r| r.color == *c))
        .unwrap_or(RoleColor::Amber)
}

/// `name`, or `name 2`, `name 3` … when a role has it (ignoring case).
fn next_name(roles: &[ServerRole], name: &str) -> String {
    let taken: HashSet<String> = roles.iter().map(|r| r.name.to_lowercase()).collect();
    if !taken.contains(&name.to_lowercase()) {
        return name.to_owned();
    }
    (2..)
        .map(|n| format!("{name} {n}"))
        .find(|candidate| !taken.contains(&candidate.to_lowercase()))
        .unwrap_or_default()
}

/// One switch flipped, in canonical order (`togglePermission`).
fn toggle_permission(current: &[Permission], flag: Permission) -> Vec<Permission> {
    let on = !current.contains(&flag);
    Permission::ALL
        .into_iter()
        .filter(|p| if *p == flag { on } else { current.contains(p) })
        .collect()
}

/// One checkbox of a member's picker (`toggleMemberRole`).
fn toggle_role(ids: &[Uuid], id: Uuid) -> Vec<Uuid> {
    if ids.contains(&id) {
        ids.iter().copied().filter(|r| *r != id).collect()
    } else {
        ids.iter().copied().chain([id]).collect()
    }
}

/// The panel the editor opens on: the first custom role, else "Alle
/// Mitglieder" (`defaultRoleSelection`).
fn default_choice(roles: &[ServerRole]) -> Choice {
    roles.first().map_or(Choice::Base, |r| Choice::Role(r.id))
}

/// A colour's name, the swatch's label.
fn color_name(color: RoleColor) -> &'static str {
    match color {
        RoleColor::Amber => texts::roles_color_name_amber(),
        RoleColor::Blue => texts::roles_color_name_blue(),
        RoleColor::Green => texts::roles_color_name_green(),
        RoleColor::Pink => texts::roles_color_name_pink(),
        RoleColor::Violet => texts::roles_color_name_violet(),
        RoleColor::Coral => texts::roles_color_name_coral(),
    }
}

/// `RoleDot`: 10px, with a 1px inset ring in `border` so it shows on
/// light themes.
pub(crate) fn dot(painter: &Painter, center: Pos2, fill: Color32, theme: &Theme) {
    painter.circle_filled(center, 5.0, fill);
    painter.circle_stroke(center, 4.5, Stroke::new(1.0, theme.c.border));
}

impl Roles {
    /// The role's colour, ahead of the server.
    fn color(&self, role: &ServerRole) -> RoleColor {
        self.colors.get(&role.id).map_or(role.color, |a| a.value)
    }

    fn flags<'a>(&'a self, role: &'a ServerRole) -> &'a [Permission] {
        self.permissions
            .get(&role.id)
            .map_or(&role.permissions, |a| &a.value)
    }

    /// A member's role ids, ahead of the server.
    fn member_ids<'a>(&'a self, member: &'a Member) -> &'a [Uuid] {
        self.members
            .get(&member.user_id)
            .map_or(&member.role_ids, |a| &a.value)
    }

    /// The number of the next quick write.
    fn next(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    fn track(&mut self, app: &AppCtx, server: Uuid, write: Write) {
        // A finished write of the same key must not read as this one's.
        app.client.clear_op(write.key(server));
        self.writes.push(write);
    }
}

/// A member's custom roles in position order, as their row shows them.
pub(super) fn held(r: &Roles, detail: &ServerDetail, member: &Member) -> Vec<(String, RoleColor)> {
    let ids = r.member_ids(member);
    in_order(&detail.roles, &r.deleted)
        .into_iter()
        .filter(|role| ids.contains(&role.id))
        .map(|role| (role.name.clone(), r.color(&role)))
        .collect()
}

// ------------------------------------------------------------- answers

/// Delete confirmations, finished writes and values the server caught up
/// with. Once per frame, before the page draws.
pub(super) fn poll(ui: &Ui, app: &mut AppCtx, r: &mut Roles, detail: &ServerDetail) {
    let server = detail.server.id;
    let now = ui.input(|i| i.time);
    for role in &detail.roles {
        if let Some(true) = controls::answer(ui.ctx(), &format!("delete-role:{}", role.id)) {
            r.deleted.insert(role.id);
            r.track(app, server, Write::Delete(role.id));
            app.client.delete_role(server, role.id);
        }
    }
    let mut keys: Vec<String> = r.writes.iter().map(|w| w.key(server)).collect();
    keys.dedup();
    for key in keys {
        let outcome: Result<(), ApiError> = match app.client.with_state(|s| s.op(&key).cloned()) {
            Some(OpState::Done) => Ok(()),
            Some(OpState::Failed(e)) => Err(e),
            // Not started yet, or on its way.
            _ => continue,
        };
        app.client.clear_op(key.clone());
        let (settled, rest): (Vec<Write>, Vec<Write>) =
            r.writes.drain(..).partition(|w| w.key(server) == key);
        r.writes = rest;
        for write in settled {
            settle(ui, app, r, server, write, &outcome, now);
        }
    }
    // The server agrees, or had its time: show what it says.
    let role = |id: &Uuid| detail.roles.iter().find(|r| r.id == *id);
    let stale = |settled: Option<f64>| settled.is_some_and(|t| now - t > SETTLE_SECS);
    r.permissions
        .retain(|id, a| role(id).is_some_and(|x| x.permissions != a.value) && !stale(a.settled));
    r.colors
        .retain(|id, a| role(id).is_some_and(|x| x.color != a.value) && !stale(a.settled));
    if r.base
        .as_ref()
        .is_some_and(|a| a.value == detail.server.member_permissions || stale(a.settled))
    {
        r.base = None;
    }
    r.members.retain(|user, a| {
        detail
            .members
            .iter()
            .find(|m| m.user_id == *user)
            .is_some_and(|m| m.role_ids != a.value)
            && !stale(a.settled)
    });
    r.deleted.retain(|id| role(id).is_some());
    let waiting = r
        .permissions
        .values()
        .map(|a| a.settled)
        .chain(r.colors.values().map(|a| a.settled))
        .chain(r.members.values().map(|a| a.settled))
        .chain(r.base.iter().map(|a| a.settled))
        .any(|s| s.is_some());
    if waiting {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(500));
    }
}

fn settle(
    ui: &Ui,
    app: &mut AppCtx,
    r: &mut Roles,
    server: Uuid,
    write: Write,
    outcome: &Result<(), ApiError>,
    now: f64,
) {
    // A later write of the same value still on its way carries this one's
    // change too: the value stays ahead until the last one answers.
    let last = !r.writes.iter().any(|w| w.same_value(&write));
    match (write, outcome) {
        (Write::Create(name), Ok(())) => {
            // The command put the new role into the detail before it ended.
            let id = app.client.with_state(|s| {
                s.detail(server)
                    .and_then(|d| d.roles.iter().find(|x| x.name == name).map(|x| x.id))
            });
            if let Some(id) = id {
                r.chosen = Some(Choice::Role(id));
                r.focus_name = Some(id);
            }
        }
        (Write::Create(_), Err(e)) => {
            app.toasts
                .failure(servers::error_role_not_created(), None, e)
        }
        (Write::Role(id, _), Ok(())) => {
            if last {
                r.status.success(ui, common::saved());
                if let Some(a) = r.permissions.get_mut(&id) {
                    a.settled = Some(now);
                }
                if let Some(a) = r.colors.get_mut(&id) {
                    a.settled = Some(now);
                }
            }
        }
        (Write::Rename(..), Ok(())) => r.status.success(ui, common::saved()),
        (Write::Role(id, _), Err(e)) => {
            if last {
                r.permissions.remove(&id);
                r.colors.remove(&id);
            }
            r.status.error(ui, common::not_saved());
            app.toasts.failure(servers::error_role_not_saved(), None, e);
        }
        (Write::Rename(..), Err(e)) => {
            r.name_errors.from_failure(&Failure::from_api(e), &["name"]);
            r.name_errors.focus = None;
            r.status.error(ui, common::not_saved());
            app.toasts.failure(servers::error_role_not_saved(), None, e);
        }
        (Write::Delete(_), Ok(())) => {}
        (Write::Delete(id), Err(e)) => {
            r.deleted.remove(&id);
            app.toasts
                .failure(servers::error_role_not_deleted(), None, e);
        }
        (Write::Member(user, _), Ok(())) => {
            if last && let Some(a) = r.members.get_mut(&user) {
                a.settled = Some(now);
            }
        }
        (Write::Member(user, _), Err(e)) => {
            if last {
                r.members.remove(&user);
            }
            app.toasts
                .failure(servers::error_member_roles_not_saved(), None, e);
        }
        // The client toasts a failed server write (`server_update`).
        (Write::Base(_), Ok(())) => {
            if last {
                r.status.success(ui, common::saved());
                if let Some(a) = r.base.as_mut() {
                    a.settled = Some(now);
                }
            }
        }
        (Write::Base(_), Err(_)) => {
            if last {
                r.base = None;
            }
            r.status.error(ui, common::not_saved());
        }
    }
}

// -------------------------------------------------------------- editor

/// `.roles-editor`: the list (232 wide), a 40px gap and the panel; at
/// 640px or less the list stacks over the panel, 24px apart. `width` is
/// the section's, wider than the reading measure.
pub(super) fn editor(
    ui: &mut Ui,
    app: &mut AppCtx,
    r: &mut Roles,
    detail: &ServerDetail,
    width: f32,
) {
    let top = ui.cursor().min;
    let stacked = width <= 640.0;
    let list_w = if stacked { width } else { 232.0 };
    let roles = in_order(&detail.roles, &r.deleted);
    let chosen = r
        .chosen
        .filter(|c| match c {
            Choice::Role(id) => roles.iter().any(|x| x.id == *id),
            _ => true,
        })
        .unwrap_or_else(|| default_choice(&roles));

    let side = Rect::from_min_size(top, vec2(list_w, f32::INFINITY));
    let side_used = controls::place(
        ui,
        UiBuilder::new()
            .max_rect(side)
            .layout(egui::Layout::top_down(egui::Align::Min)),
        |ui| {
            side_list(ui, app, r, detail, &roles, chosen);
            ui.min_rect()
        },
    );
    let panel = if stacked {
        Rect::from_min_size(
            pos2(top.x, side_used.bottom() + 24.0),
            vec2(width, f32::INFINITY),
        )
    } else {
        Rect::from_min_size(
            pos2(top.x + list_w + 40.0, top.y),
            vec2(width - list_w - 40.0, f32::INFINITY),
        )
    };
    let panel_used = controls::place(
        ui,
        UiBuilder::new()
            .max_rect(panel)
            .layout(egui::Layout::top_down(egui::Align::Min)),
        |ui| {
            match chosen {
                Choice::Role(id) => {
                    if let Some(role) = roles.iter().find(|x| x.id == id) {
                        custom_panel(ui, app, r, detail, role);
                    }
                }
                Choice::Owner => owner_panel(ui),
                Choice::Base => base_panel(ui, app, r, detail),
            }
            // The status is always there (empty, no height), after the
            // panel's 16px gap.
            ui.add_space(16.0);
            if let Some((m, t)) = r.status.current(ui) {
                ui.add(crate::widgets::InlineStatus::new(Some(&m), t));
            }
            ui.min_rect()
        },
    );
    let bottom = side_used.bottom().max(panel_used.bottom());
    // Only the height: the sections after it keep the reading measure.
    let measure = ui.available_width().min(width);
    ui.advance_cursor_after_rect(Rect::from_min_max(top, pos2(top.x + measure, bottom)));
}

fn side_list(
    ui: &mut Ui,
    app: &mut AppCtx,
    r: &mut Roles,
    detail: &ServerDetail,
    roles: &[ServerRole],
    chosen: Choice,
) {
    let theme = Theme::of(ui.ctx());
    let counts = counts(detail);
    let mut pick = None;
    if list_row(
        ui,
        "owner",
        theme.c.accent,
        role_texts::owner(),
        1,
        true,
        chosen == Choice::Owner,
    ) {
        pick = Some(Choice::Owner);
    }
    for role in roles {
        ui.add_space(2.0);
        let count = counts.get(&role.id).copied().unwrap_or(0);
        if list_row(
            ui,
            role.id,
            role_color(r.color(role)),
            &role.name,
            count,
            false,
            chosen == Choice::Role(role.id),
        ) {
            pick = Some(Choice::Role(role.id));
        }
    }
    ui.add_space(2.0);
    if list_row(
        ui,
        "base",
        theme.c.muted,
        role_texts::base(),
        detail.members.len(),
        false,
        chosen == Choice::Base,
    ) {
        pick = Some(Choice::Base);
    }
    if let Some(choice) = pick {
        r.status.clear();
        r.chosen = Some(choice);
    }
    ui.add_space(16.0);
    let server = detail.server.id;
    let creating = r.writes.iter().any(|w| matches!(w, Write::Create(_)));
    if ui
        .add(
            Button::new(texts::roles_new())
                .variant(ButtonVariant::Outline)
                .icon(Icon::Plus)
                .fill()
                .disabled(creating),
        )
        .clicked()
        && !creating
    {
        r.status.clear();
        let name = next_name(&detail.roles, texts::roles_new());
        let color = next_color(&detail.roles);
        r.track(app, server, Write::Create(name.clone()));
        app.client.create_role(
            server,
            NewRole {
                name,
                color,
                permissions: Some(Vec::new()),
            },
        );
    }
}

/// `.roles-editor__row`: 40 high, dot, name (15px; 600 when chosen), the
/// lock for the owner and the count, right-aligned.
fn list_row(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    color: Color32,
    name: &str,
    count: usize,
    locked: bool,
    selected: bool,
) -> bool {
    let theme = Theme::of(ui.ctx());
    let c = &theme.c;
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, 40.0), Sense::hover());
    let response = ui.interact(rect, Id::unique(("gelabber-role-row", id)), Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, name));
    let hover = eased(
        ui.ctx(),
        response.id,
        response.hovered(),
        Color32::TRANSPARENT,
        c.hover,
    );
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        corner(theme.radius.md),
        if selected { c.selected } else { hover },
    );
    if response.has_focus() && crate::widgets::keyboard_focus(ui.ctx()) {
        crate::widgets::focus_ring(painter, rect, theme.radius.md, &theme);
    }
    let mid = rect.center().y;
    dot(painter, pos2(rect.left() + 17.0, mid), color, &theme);
    // `min-width: 1.5em` of the 12px count, right-aligned.
    let count_text = gelabber_i18n::intl::count(count as u64);
    let cb = Block::single(ui, &count_text, text::META, c.muted);
    let count_w = cb.width().max(18.0);
    cb.paint(
        painter,
        pos2(rect.right() - 12.0 - cb.width(), mid - cb.height() / 2.0),
    );
    let mut end = rect.right() - 12.0 - count_w - 12.0;
    if locked {
        icons::paint(
            painter,
            Icon::Lock,
            pos2(end - 14.0, mid - 7.0),
            14.0,
            c.muted,
        );
        end -= 14.0 + 12.0;
    }
    let style = TextStyle::new(15.0, if selected { 600.0 } else { 400.0 }, 1.35);
    let nb = Block::single(ui, name, style, c.text);
    let left = rect.left() + 34.0;
    let clip = Rect::from_min_max(pos2(left, rect.top()), pos2(end.max(left), rect.bottom()));
    nb.paint(
        &painter.with_clip_rect(clip.intersect(painter.clip_rect())),
        pos2(left, mid - nb.height() / 2.0),
    );
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response.clicked()
}

/// `.roles-editor__header`: the title, and an action at the right.
fn panel_header(ui: &mut Ui, title: &str) -> Rect {
    let theme = Theme::of(ui.ctx());
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, 32.0), Sense::hover());
    let b = Block::wrapped(ui, title, text::TITLE_SM, theme.c.text, width - 140.0);
    b.paint(
        ui.painter(),
        pos2(rect.left(), rect.center().y - b.height() / 2.0),
    );
    rect
}

fn group_label(ui: &mut Ui, label: &str) {
    let theme = Theme::of(ui.ctx());
    let w = ui.available_width();
    controls::paragraph(ui, label, text::UI.weight(600.0), theme.c.text, w);
    ui.add_space(8.0);
}

fn owner_panel(ui: &mut Ui) {
    let theme = Theme::of(ui.ctx());
    panel_header(ui, role_texts::owner());
    ui.add_space(16.0);
    let w = ui.available_width();
    controls::paragraph(ui, texts::roles_owner_text(), text::BODY, theme.c.muted, w);
    ui.add_space(16.0);
    switches(ui, "owner", |_| true, |_| false, true);
}

fn base_panel(ui: &mut Ui, app: &mut AppCtx, r: &mut Roles, detail: &ServerDetail) {
    panel_header(ui, role_texts::base());
    ui.add_space(16.0);
    let current = r.base.as_ref().map_or_else(
        || detail.server.member_permissions.clone(),
        |a| a.value.clone(),
    );
    if let Some(flag) = switches(ui, "base", |p| current.contains(&p), |_| false, false) {
        let next = toggle_permission(&current, flag);
        r.base = Some(Ahead::new(next.clone()));
        r.status.clear();
        let server = detail.server.id;
        let seq = r.next();
        r.track(app, server, Write::Base(seq));
        app.client.update_member_permissions(server, next, seq);
    }
}

fn custom_panel(
    ui: &mut Ui,
    app: &mut AppCtx,
    r: &mut Roles,
    detail: &ServerDetail,
    role: &ServerRole,
) {
    let server = detail.server.id;
    let header = panel_header(ui, &role.name);
    let label = texts::roles_delete();
    let bw = Block::single(ui, label, text::UI, Color32::WHITE).width() + 26.0;
    let br = Rect::from_min_size(pos2(header.right() - bw, header.top()), vec2(bw, 32.0));
    let delete = controls::place(ui, UiBuilder::new().max_rect(br), |ui| {
        ui.add(
            Button::new(label)
                .small()
                .variant(ButtonVariant::DangerOutline),
        )
        .clicked()
    });
    if delete {
        controls::ask(
            ui.ctx(),
            Confirm::danger(
                &format!("delete-role:{}", role.id),
                texts::roles_delete_title(&role.name),
                Some(texts::roles_delete_message()),
                common::delete(),
            ),
        );
    }
    ui.add_space(16.0);

    // The name: saved on Enter or when the field is left, if changed.
    if r.name.as_ref().is_none_or(|f| f.role != role.id) {
        r.name = Some(NameField {
            role: role.id,
            text: role.name.clone(),
            synced: role.name.clone(),
        });
        r.name_errors = FormErrors::default();
    }
    let refused = r.name_errors.field("name").is_some();
    let field = r.name.as_mut().expect("set above");
    // A rename elsewhere (or our own coming back) reaches an untouched
    // field; a refused name stays for its error (`syncRoleName`).
    if field.synced != role.name && !refused {
        if field.text == field.synced || field.text.trim() == role.name {
            field.text = role.name.clone();
        }
        field.synced = role.name.clone();
    }
    let mut text = field.text.clone();
    let focus = r.focus_name == Some(role.id);
    let id = ("role-name", role.id);
    let out = controls::column(ui, ui.available_width().min(320.0), |ui| {
        TextField::new(id, &mut text)
            .label(texts::roles_name_label())
            .char_limit(ROLE_NAME_MAX)
            .error(r.name_errors.field("name"))
            .autofocus(focus)
            .show(ui)
    });
    if out.response.changed() {
        r.name_errors.clear_field("name");
    }
    if let Some(f) = r.name.as_mut() {
        f.text = text.clone();
    }
    if focus && out.response.has_focus() {
        // The new role's name is selected, ready to be typed over.
        let fid = crate::widgets::field::field_id(id);
        if let Some(mut state) = egui::text_edit::TextEditState::load(ui.ctx(), fid) {
            let end = egui::text::CCursor::new(text.chars().count());
            state
                .cursor
                .set_char_range(Some(egui::text::CCursorRange::two(
                    egui::text::CCursor::new(0),
                    end,
                )));
            state.store(ui.ctx(), fid);
        }
        r.focus_name = None;
    }
    if out.submitted || out.response.lost_focus() {
        let name = text.trim();
        if name != role.name {
            let others: Vec<&str> = detail
                .roles
                .iter()
                .filter(|x| x.id != role.id)
                .map(|x| x.name.as_str())
                .collect();
            match validate_role_name(name, &others) {
                Some(code) => r
                    .name_errors
                    .set_fields(vec![("name", field_message("name", code))]),
                None => {
                    r.status.clear();
                    let seq = r.next();
                    r.track(app, server, Write::Rename(role.id, seq));
                    app.client.update_role(
                        server,
                        role.id,
                        RolePatch {
                            name: Some(name.to_owned()),
                            ..RolePatch::default()
                        },
                        seq,
                    );
                }
            }
        }
    }
    r.name_errors.focus = None;
    ui.add_space(16.0);

    // "Farbe": six swatches as a radio group.
    group_label(ui, texts::roles_color_label());
    let current = r.color(role);
    if let Some(color) = swatches(ui, role.id, current)
        && color != current
    {
        r.status.clear();
        r.colors.insert(role.id, Ahead::new(color));
        let seq = r.next();
        r.track(app, server, Write::Role(role.id, seq));
        app.client.update_role(
            server,
            role.id,
            RolePatch {
                color: Some(color),
                ..RolePatch::default()
            },
            seq,
        );
    }
    ui.add_space(16.0);

    let flags = r.flags(role).to_vec();
    let base = &detail.server.member_permissions;
    if let Some(flag) = switches(
        ui,
        role.id,
        |p| flags.contains(&p),
        |p| base.contains(&p),
        false,
    ) {
        let next = toggle_permission(&flags, flag);
        r.status.clear();
        r.permissions.insert(role.id, Ahead::new(next.clone()));
        let seq = r.next();
        r.track(app, server, Write::Role(role.id, seq));
        app.client.update_role(
            server,
            role.id,
            RolePatch {
                permissions: Some(next),
                ..RolePatch::default()
            },
            seq,
        );
    }
}

/// The six swatches (`.role-swatch`): 26px, 10 apart; the chosen one gets
/// a ring. Returns the one clicked.
fn swatches(ui: &mut Ui, role: Uuid, current: RoleColor) -> Option<RoleColor> {
    let theme = Theme::of(ui.ctx());
    let c = &theme.c;
    let width = ui.available_width();
    let (row, _) = ui.allocate_exact_size(vec2(width, 26.0), Sense::hover());
    let mut picked = None;
    for (i, color) in RoleColor::ALL.into_iter().enumerate() {
        let rect = Rect::from_min_size(
            pos2(row.left() + i as f32 * 36.0, row.top()),
            vec2(26.0, 26.0),
        );
        let name = color_name(color);
        let checked = color == current;
        let response = ui
            .interact(
                rect,
                Id::unique(("gelabber-role-swatch", role, i)),
                Sense::click(),
            )
            .on_hover_text(name);
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::Role::RadioButton, true, checked, name)
        });
        let painter = ui.painter();
        let center = rect.center();
        painter.circle_filled(center, 13.0, role_color(color));
        painter.circle_stroke(center, 12.5, Stroke::new(1.0, c.border));
        if checked {
            // `0 0 0 2px bg, 0 0 0 4px text`.
            painter.circle_stroke(center, 14.0, Stroke::new(2.0, c.bg));
            painter.circle_stroke(center, 16.0, Stroke::new(2.0, c.text));
        }
        if response.has_focus() && crate::widgets::keyboard_focus(ui.ctx()) {
            painter.circle_stroke(center, 19.0, Stroke::new(2.0, c.focus));
        }
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if response.clicked() {
            picked = Some(color);
        }
    }
    picked
}

/// "Rechte": the nine switches in canonical order. A flag `inherited`
/// shows on and fixed with "über „Alle Mitglieder“". Returns the flag
/// flipped.
fn switches(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug + Copy,
    checked: impl Fn(Permission) -> bool,
    inherited: impl Fn(Permission) -> bool,
    disabled: bool,
) -> Option<Permission> {
    let theme = Theme::of(ui.ctx());
    group_label(ui, texts::roles_permissions_label());
    let top = ui.cursor().min;
    crate::widgets::rule_h(
        ui.painter(),
        theme.c.border,
        top.x..=top.x + ui.available_width(),
        top.y,
    );
    ui.add_space(1.0);
    let note = texts::roles_inherited(role_texts::base());
    let mut flipped = None;
    for (i, (flag, label, hint)) in PERMISSIONS.into_iter().enumerate() {
        let fixed = inherited(flag);
        let on = fixed || checked(flag);
        if permission_row(
            ui,
            (id, i),
            label(),
            hint(),
            on,
            disabled || fixed,
            fixed.then_some(note.as_str()),
        ) {
            flipped = Some(flag);
        }
    }
    flipped
}

/// `.roles-permission`: label and hint, the note, the switch; 42 high at
/// least, 4px above and below, a hairline under it. The row toggles.
fn permission_row(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    label: &str,
    hint: &str,
    on: bool,
    disabled: bool,
    note: Option<&str>,
) -> bool {
    let theme = Theme::of(ui.ctx());
    let c = &theme.c;
    let width = ui.available_width();
    let note_block = note.map(|n| Block::single(ui, n, text::META, c.muted));
    let note_w = note_block.as_ref().map_or(0.0, |b| b.width() + 12.0);
    let text_w = (width - 38.0 - 12.0 - note_w).max(40.0);
    let lb = Block::wrapped(ui, label, text::UI, c.text, text_w);
    let hb = Block::wrapped(ui, hint, text::META, c.muted, text_w);
    let content_h = lb.height() + 2.0 + hb.height();
    let height = (content_h + 8.0).max(42.0) + 1.0;
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let row_id = Id::unique(("gelabber-role-permission", id));
    let response = ui.interact(
        rect,
        row_id,
        if disabled {
            Sense::hover()
        } else {
            Sense::click()
        },
    );
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Switch, !disabled, on, label));
    let painter = ui.painter();
    let inner_mid = rect.top() + (height - 1.0) / 2.0;
    let text_top = inner_mid - content_h / 2.0;
    lb.paint(painter, pos2(rect.left(), text_top));
    hb.paint(painter, pos2(rect.left(), text_top + lb.height() + 2.0));
    if let Some(b) = &note_block {
        b.paint(
            painter,
            pos2(
                rect.right() - 38.0 - 12.0 - b.width(),
                inner_mid - b.height() / 2.0,
            ),
        );
    }
    crate::widgets::rule_h(painter, c.border, rect.x_range(), rect.bottom() - 1.0);
    let sw = Rect::from_min_size(
        pos2(rect.right() - 38.0, inner_mid - 11.0),
        vec2(38.0, 22.0),
    );
    let mut value = on;
    let switch = controls::place(ui, UiBuilder::new().max_rect(sw).id_salt(row_id), |ui| {
        ui.add(Switch::new(&mut value).disabled(disabled))
    });
    switch.widget_info(|| egui::WidgetInfo::selected(egui::Role::Switch, !disabled, on, label));
    if response.hovered() && !disabled {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    !disabled && (response.clicked() || switch.changed())
}

// --------------------------------------------------------- member rows

/// `.role-chip`'s width: 8, the dot, 6, the name, 10.
pub(super) fn chip_width(ui: &Ui, name: &str) -> f32 {
    8.0 + 10.0 + 6.0 + Block::single(ui, name, text::META, Color32::WHITE).width() + 10.0
}

/// `.role-chip`: 24 high, round, the surface fill; dot and name.
pub(super) fn chip(ui: &Ui, rect: Rect, name: &str, color: RoleColor) {
    let theme = Theme::of(ui.ctx());
    let painter = ui.painter();
    painter.rect_filled(rect, corner(12.0), theme.c.surface);
    dot(
        painter,
        pos2(rect.left() + 13.0, rect.center().y),
        role_color(color),
        &theme,
    );
    let b = Block::single(ui, name, text::META, theme.c.text);
    b.paint(
        painter,
        pos2(rect.left() + 24.0, rect.center().y - b.height() / 2.0),
    );
}

/// What the picker asks the page to do.
pub(super) enum Picked {
    Nothing,
    /// "Rollen verwalten": jump to the Rollen section.
    Manage,
}

/// The dashed "+" of a member row at `rect` (24px) and, while open, its
/// checklist of the custom roles. A checkbox saves at once and the list
/// stays open.
pub(super) fn picker(
    ui: &mut Ui,
    app: &mut AppCtx,
    r: &mut Roles,
    detail: &ServerDetail,
    member: &Member,
    rect: Rect,
) -> Picked {
    let theme = Theme::of(ui.ctx());
    let c = &theme.c;
    let label = texts::members_roles_for(&member.name);
    let open = r.picker == Some(member.user_id);
    let response = ui
        .interact(
            rect,
            Id::unique(("gelabber-role-picker", member.user_id)),
            Sense::click(),
        )
        .on_hover_text(&label);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
    let hovered = response.hovered();
    let painter = ui.painter();
    let center = rect.center();
    if hovered {
        painter.circle_filled(center, 12.0, c.hover);
    }
    if open {
        painter.circle_stroke(center, 11.5, Stroke::new(1.0, c.accent));
    } else {
        // `1px dashed var(--lr-border-strong)`.
        let points: Vec<Pos2> = (0..=48)
            .map(|i| {
                let a = i as f32 / 48.0 * std::f32::consts::TAU;
                center + vec2(a.cos(), a.sin()) * 11.5
            })
            .collect();
        painter.extend(egui::Shape::dashed_line(
            &points,
            Stroke::new(1.0, c.border_strong),
            3.0,
            3.0,
        ));
    }
    icons::paint(
        painter,
        Icon::Plus,
        center - vec2(7.0, 7.0),
        14.0,
        if open || hovered { c.text } else { c.muted },
    );
    if response.has_focus() && crate::widgets::keyboard_focus(ui.ctx()) {
        crate::widgets::focus_ring(painter, rect, 12.0, &theme);
    }
    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if response.clicked() {
        r.picker = if open { None } else { Some(member.user_id) };
    }
    if r.picker != Some(member.user_id) {
        return Picked::Nothing;
    }
    let roles = in_order(&detail.roles, &r.deleted);
    let held: Vec<Uuid> = r.member_ids(member).to_vec();
    let colors: Vec<RoleColor> = roles.iter().map(|x| r.color(x)).collect();
    let shown = Popover::new(("role-picker", member.user_id), rect)
        .align_end()
        .min_width(232.0)
        .padding(12.0)
        .show(ui.ctx(), |ui| {
            picker_list(ui, &label, &roles, &colors, &held)
        });
    let server = detail.server.id;
    match shown {
        None => {
            r.picker = None;
            Picked::Nothing
        }
        Some(Choose::Role(id)) => {
            let next = toggle_role(&held, id);
            r.members.insert(member.user_id, Ahead::new(next.clone()));
            let seq = r.next();
            r.track(app, server, Write::Member(member.user_id, seq));
            app.client
                .set_member_roles(server, member.user_id, next, seq);
            Picked::Nothing
        }
        Some(Choose::Manage) => {
            r.picker = None;
            Picked::Manage
        }
        Some(Choose::Nothing) => Picked::Nothing,
    }
}

enum Choose {
    Nothing,
    Role(Uuid),
    Manage,
}

/// `.role-picker`: the title, a checkbox row per role (or "Noch keine
/// Rollen."), a divider and "Rollen verwalten".
fn picker_list(
    ui: &mut Ui,
    title: &str,
    roles: &[ServerRole],
    colors: &[RoleColor],
    held: &[Uuid],
) -> Choose {
    let theme = Theme::of(ui.ctx());
    let c = &theme.c;
    ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
    let width = ui.available_width();
    let mut chosen = Choose::Nothing;
    // `.role-picker__title`: padding 4 4 6, meta 600.
    let tb = Block::single(ui, title, text::META.weight(600.0), c.text);
    let (tr, _) = ui.allocate_exact_size(vec2(width, 4.0 + tb.height() + 6.0), Sense::hover());
    tb.paint(ui.painter(), pos2(tr.left() + 4.0, tr.top() + 4.0));
    if roles.is_empty() {
        let b = Block::wrapped(
            ui,
            texts::members_no_roles(),
            text::META,
            c.muted,
            width - 8.0,
        );
        let (er, _) = ui.allocate_exact_size(vec2(width, b.height() + 8.0), Sense::hover());
        b.paint(ui.painter(), pos2(er.left() + 4.0, er.top() + 4.0));
    }
    for (role, color) in roles.iter().zip(colors) {
        let (row, _) = ui.allocate_exact_size(vec2(width, 36.0), Sense::hover());
        let response = ui.interact(
            row,
            Id::unique(("gelabber-role-picker-row", role.id)),
            Sense::click(),
        );
        let mut on = held.contains(&role.id);
        response
            .widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, on, &role.name));
        let mid = row.center().y;
        let check = Rect::from_min_size(pos2(row.left() + 4.0, mid - 8.0), vec2(16.0, 16.0));
        controls::place(ui, UiBuilder::new().max_rect(check), |ui| {
            ui.add(Checkbox::new(&mut on).size(16.0).decorative())
        });
        let painter = ui.painter();
        dot(
            painter,
            pos2(check.right() + 12.0 + 5.0, mid),
            role_color(*color),
            &theme,
        );
        let nb = Block::single(ui, &role.name, text::BODY, c.text);
        let x = check.right() + 12.0 + 10.0 + 10.0;
        let clip = Rect::from_min_max(pos2(x, row.top()), pos2(row.right() - 4.0, row.bottom()));
        nb.paint(
            &painter.with_clip_rect(clip.intersect(painter.clip_rect())),
            pos2(x, mid - nb.height() / 2.0),
        );
        if response.has_focus() && crate::widgets::keyboard_focus(ui.ctx()) {
            crate::widgets::focus_ring(painter, row, theme.radius.sm, &theme);
        }
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if response.clicked() {
            chosen = Choose::Role(role.id);
        }
    }
    // `.role-picker__divider`: margin 8 0 4.
    ui.add_space(8.0);
    let y = ui.cursor().min.y;
    crate::widgets::rule_h(ui.painter(), c.border, row_x(ui, width), y);
    ui.add_space(1.0 + 4.0);
    let (mr, _) = ui.allocate_exact_size(vec2(width, 32.0), Sense::hover());
    let manage = ui.interact(
        mr,
        Id::unique("gelabber-role-picker-manage"),
        Sense::click(),
    );
    let label = texts::members_manage_roles();
    manage.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    let mb = Block::single(
        ui,
        label,
        text::META,
        if manage.hovered() { c.text } else { c.muted },
    );
    mb.paint(
        ui.painter(),
        pos2(mr.left() + 4.0, mr.center().y - mb.height() / 2.0),
    );
    if manage.has_focus() && crate::widgets::keyboard_focus(ui.ctx()) {
        crate::widgets::focus_ring(ui.painter(), mr, theme.radius.sm, &theme);
    }
    if manage.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if manage.clicked() {
        chosen = Choose::Manage;
    }
    chosen
}

fn row_x(ui: &Ui, width: f32) -> std::ops::RangeInclusive<f32> {
    let x = ui.cursor().min.x;
    x..=x + width
}

#[cfg(test)]
mod tests {
    use super::*;

    fn role(id: u128, name: &str, color: RoleColor, position: i32) -> ServerRole {
        ServerRole {
            id: Uuid::from_u128(id),
            server_id: Uuid::nil(),
            name: name.into(),
            color,
            permissions: Vec::new(),
            position,
            created_at: chrono::DateTime::from_timestamp(0, 0).unwrap(),
        }
    }

    /// The cases of servers/roles.test.ts for the editor's helpers.
    #[test]
    fn role_helpers_follow_the_web() {
        let roles = vec![
            role(2, "Stammgäste", RoleColor::Green, 1),
            role(1, "Moderation", RoleColor::Blue, 0),
        ];
        let ordered: Vec<String> = in_order(&roles, &HashSet::new())
            .into_iter()
            .map(|r| r.name)
            .collect();
        assert_eq!(ordered, ["Moderation", "Stammgäste"]);
        assert_eq!(
            in_order(&roles, &HashSet::from([Uuid::from_u128(1)])).len(),
            1
        );
        assert_eq!(next_color(&roles), RoleColor::Amber);
        assert_eq!(next_color(&[]), RoleColor::Amber);
        let all: Vec<ServerRole> = RoleColor::ALL
            .into_iter()
            .enumerate()
            .map(|(i, c)| role(i as u128 + 10, c.name(), c, i as i32))
            .collect();
        assert_eq!(next_color(&all), RoleColor::Amber);
        assert_eq!(next_color(&all[..2]), RoleColor::Green);
        assert_eq!(next_name(&roles, "Neue Rolle"), "Neue Rolle");
        let taken = vec![
            role(3, "neue rolle", RoleColor::Amber, 2),
            role(4, "Neue Rolle 2", RoleColor::Amber, 3),
        ];
        assert_eq!(next_name(&taken, "Neue Rolle"), "Neue Rolle 3");
        assert_eq!(
            default_choice(&in_order(&roles, &HashSet::new())),
            Choice::Role(Uuid::from_u128(1))
        );
        assert_eq!(default_choice(&[]), Choice::Base);
        assert_eq!(
            toggle_permission(&[Permission::GoLive], Permission::ManageServer),
            [Permission::ManageServer, Permission::GoLive]
        );
        assert_eq!(
            toggle_permission(
                &[Permission::ManageServer, Permission::GoLive],
                Permission::GoLive
            ),
            [Permission::ManageServer]
        );
        let (a, b) = (Uuid::from_u128(1), Uuid::from_u128(2));
        assert_eq!(toggle_role(&[a], b), [a, b]);
        assert_eq!(toggle_role(&[a, b], a), [b]);
    }
}
