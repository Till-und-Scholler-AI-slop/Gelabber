//! Server settings (web pages/ServerSettingsPage.tsx, settings.css): name
//! and icon, member permissions, invites, members, bans and the danger
//! zone for managers; members and leaving for everyone else.

mod invite;

use std::collections::HashMap;
use std::sync::Arc;

use egui::{Color32, Id, Rect, Sense, Ui, UiBuilder, pos2, vec2};
use gelabber_client::{
    ApiError, Ban, Invite, Member, OpState, Permission, PresenceStatus, Role, ServerDetail,
    ServerPatch,
};
use parking_lot::Mutex;
use uuid::Uuid;

use crate::app::AppCtx;
use crate::icons::Icon;
use crate::images::ImageState;
use crate::initials::{tile_initials, tile_tone};
use crate::rules::{Failure, FormErrors, error_message, field_message};
use crate::screens::auth::form_error;
use crate::screens::settings::controls::{self, Confirm, Picked, Status};
use crate::screens::settings::{ensure_workspace, me};
use crate::task::{Pending, poll};
use crate::text::{self, Block, TextStyle};
use crate::theme::{Theme, corner};
use crate::widgets::{
    Avatar, Badge, BadgeTone, Button, ButtonVariant, IconButton, Popover, Presence, TextField,
    eased,
};

/// `PERMISSIONS` of servers/permissions.ts.
const PERMISSIONS: [(Permission, &str, &str); 8] = [
    (
        Permission::ManageServer,
        "Server verwalten",
        "Umbenennen, Rechte ändern, Einladungen, Kick und Ban.",
    ),
    (
        Permission::ManageChannels,
        "Kanäle verwalten",
        "Kategorien und Kanäle anlegen, umbenennen, löschen.",
    ),
    (
        Permission::ManageMessages,
        "Nachrichten löschen",
        "Fremde Nachrichten im Chat löschen.",
    ),
    (
        Permission::MentionEveryone,
        "@everyone und @here",
        "Alle Mitglieder auf einmal erwähnen und benachrichtigen.",
    ),
    (
        Permission::SendMessages,
        "Nachrichten schreiben",
        "In Textkanälen schreiben.",
    ),
    (
        Permission::SendFiles,
        "Dateien senden",
        "Anhänge hochladen.",
    ),
    (
        Permission::JoinVoice,
        "Voice beitreten",
        "Sprachkanälen beitreten.",
    ),
    (
        Permission::GoLive,
        "Live gehen",
        "Im Sprachkanal einen Bildschirm oder ein Fenster für alle dort zeigen.",
    ),
];

const SECTIONS: [(&str, &str); 6] = [
    ("allgemein", "Allgemein"),
    ("rechte", "Rechte"),
    ("einladungen", "Einladungen"),
    ("mitglieder", "Mitglieder"),
    ("sperren", "Sperren"),
    ("gefahrenzone", "Gefahrenzone"),
];

const NOT_SAVED: &str = "Nicht gespeichert – erneut versuchen";

type Load<T> = Option<Pending<Result<T, ApiError>>>;

#[derive(Default)]
struct State {
    server: Option<Uuid>,
    name: Option<String>,
    name_errors: FormErrors,
    name_status: Status,
    saving_name: bool,
    icon_status: Status,
    icon: Load<gelabber_client::Server>,
    icon_removing: bool,
    picking: Option<Picked>,
    permissions_status: Status,
    /// The mask the next switch builds on, ahead of the server's answer.
    permissions: Option<Vec<Permission>>,
    saving_permissions: bool,
    invites: Option<Result<Vec<Invite>, ApiError>>,
    invites_load: Load<Vec<Invite>>,
    invites_status: Status,
    revoke: Load<()>,
    copied: HashMap<String, f64>,
    bans: Option<Result<Vec<Ban>, ApiError>>,
    bans_load: Load<Vec<Ban>>,
    unban: Load<()>,
    menu: Option<Uuid>,
    /// Who a kick/ban question is about.
    target: Option<Uuid>,
    inviting: Option<invite::Dialog>,
    /// Section heading positions in the scroll content, for the nav.
    anchors: HashMap<&'static str, f32>,
    scroll_to: Option<&'static str>,
    /// The section last jumped to, and the offset the jump left.
    jumped: Option<(&'static str, f32)>,
    content_height: f32,
    opened_at: f64,
}

/// How far below the anchor row a heading may sit and still count as
/// reached (web pages/sectionSpy.ts `SECTION_LINE`).
const SECTION_LINE: f32 = 32.0;

/// The last section whose heading (its top from the anchor row's bottom)
/// has reached the line; at the very end the last one (`currentSection`).
fn current_section(headings: &[(&'static str, f32)], at_end: bool) -> Option<&'static str> {
    if at_end {
        return headings.last().map(|(id, _)| *id);
    }
    let mut current = headings.first().map(|(id, _)| *id);
    for (id, top) in headings {
        if *top > SECTION_LINE {
            break;
        }
        current = Some(*id);
    }
    current
}

fn state(ctx: &egui::Context) -> Arc<Mutex<State>> {
    let id = Id::unique("gelabber-server-settings");
    ctx.data_mut(|d| {
        d.get_temp_mut_or_insert_with(id, || Arc::new(Mutex::new(State::default())))
            .clone()
    })
}

/// The server settings navigation in the sidebar column. The web shows the
/// server's channel sidebar there, which the shell draws; nothing here.
pub fn sidebar(_ui: &mut Ui, _app: &mut AppCtx, _server: Uuid) {}

/// The server settings page: header and body, drawn into `ui.max_rect()`.
pub fn page(ui: &mut Ui, app: &mut AppCtx, server: Uuid) {
    let ctx = ui.ctx().clone();
    ensure_workspace(app, &ctx);
    crate::screens::settings::apply_preferences(&ctx, app.client);
    app.client.open_server(server);
    let rect = ui.max_rect();
    let theme = Theme::of(&ctx);
    let c = &theme.c;
    let detail = app.client.with_state(|s| s.detail(server).cloned());
    let Some(detail) = detail else {
        controls::header(ui, rect, "Servereinstellungen", None);
        let failed = app
            .client
            .with_state(|s| matches!(s.op(&format!("server:{server}")), Some(OpState::Failed(_))));
        if failed {
            load_error(ui, app, rect, server);
        }
        return;
    };
    controls::header(ui, rect, "Servereinstellungen", Some(&detail.server.name));
    let shared = state(&ctx);
    let mut s = shared.lock();
    if s.server != Some(server) {
        *s = State {
            server: Some(server),
            opened_at: chrono::Utc::now().timestamp_millis() as f64,
            ..State::default()
        };
    }
    let manage = detail.server.can(Permission::ManageServer);
    poll_loads(ui, app, &mut s, &detail, manage);

    // The anchor row: fixed under the header (sticky at the scroll top).
    // Its links take input first, for the Tab order; they are painted after
    // the body, with the section that is in view now.
    let nav = Rect::from_min_size(
        pos2(rect.left(), rect.top() + 52.0),
        vec2(rect.width(), 49.0),
    );
    let mut x = nav.left() + 32.0;
    let ids: Vec<(&'static str, &'static str)> = SECTIONS
        .iter()
        .copied()
        .filter(|(id, _)| manage || matches!(*id, "mitglieder" | "gefahrenzone"))
        .collect();
    let mut links = Vec::with_capacity(ids.len());
    for (id, label) in &ids {
        let b = Block::single(ui, label, text::UI, c.muted);
        let r = Rect::from_min_size(pos2(x, nav.top() + 8.0), vec2(b.width() + 24.0, 32.0));
        let wid = Id::unique(("gelabber-server-nav", *id));
        let response = ui.interact(r, wid, Sense::click());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Link, true, *label));
        if response.clicked() {
            s.scroll_to = Some(id);
        }
        links.push((*id, b, response));
        x += links.last().map_or(0.0, |(_, _, r)| r.rect.width()) + 4.0;
    }

    let body = Rect::from_min_max(pos2(rect.left(), nav.bottom()), rect.max);
    let jump = s.scroll_to.take();
    let offset = jump
        .and_then(|id| s.anchors.get(id).copied())
        .map(|y| (y - 64.0 + 49.0).max(0.0));
    let st = &mut *s;
    let (_, scrolled) = controls::scroll(ui, body, ("server", server), offset, |ui| {
        let width = ui.available_width().min(1040.0);
        let left = ui.cursor().min.x;
        let content_top = ui.cursor().min.y;
        ui.add_space(32.0);
        let inner = Rect::from_min_size(
            pos2(left + 40.0, ui.cursor().min.y),
            vec2((width - 80.0).min(controls::MEASURE), f32::INFINITY),
        );
        let mut col = ui.new_child(
            UiBuilder::new()
                .max_rect(inner)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        col.spacing_mut().item_spacing = egui::Vec2::ZERO;
        let mut first = true;
        let mut section =
            |col: &mut Ui, id: &'static str, title: &str, hint: Option<&str>, st: &mut State| {
                if !first {
                    col.add_space(40.0);
                }
                first = false;
                st.anchors.insert(id, col.cursor().min.y - content_top);
                let w = col.available_width();
                controls::paragraph(col, title, text::TITLE, c.text, w);
                if let Some(h) = hint {
                    col.add_space(4.0);
                    controls::paragraph(col, h, text::BODY, c.muted, w);
                }
                col.add_space(14.0);
            };
        if manage {
            section(
                &mut col,
                "allgemein",
                "Allgemein",
                Some("Ein neues Bild gilt sofort, den Namen übernimmst du mit Speichern."),
                st,
            );
            general(&mut col, app, st, &detail);
            section(
                &mut col,
                "rechte",
                "Rechte für Mitglieder",
                Some("Gilt für alle Mitglieder außer dem Inhaber. Änderungen wirken sofort."),
                st,
            );
            permissions(&mut col, app, st, &detail);
            section(
                &mut col,
                "einladungen",
                "Einladungen",
                Some("Alle aktiven Links. Jedes Mitglied kann Links erstellen."),
                st,
            );
            invites(&mut col, app, st, &detail);
        }
        let title = format!("Mitglieder ({})", detail.members.len());
        section(&mut col, "mitglieder", &title, None, st);
        members(&mut col, app, st, &detail, manage);
        if manage {
            section(
                &mut col,
                "sperren",
                "Gesperrte Konten",
                Some("Gesperrte User können auch mit einem Einladungslink nicht wieder beitreten."),
                st,
            );
            bans(&mut col, app, st);
        }
        let owner = detail.server.role == Role::Owner;
        let hint = if owner {
            "Löscht Kanäle, Kategorien, Nachrichten, Einladungen und Mitgliedschaften endgültig."
        } else {
            "Wieder beitreten kannst du nur mit einem neuen Einladungslink."
        };
        section(&mut col, "gefahrenzone", "Gefahrenzone", Some(hint), st);
        danger(&mut col, app, &detail);
        let used = col.min_rect();
        st.content_height = used.bottom() + 48.0 - content_top;
        ui.advance_cursor_after_rect(Rect::from_min_max(
            pos2(left, inner.top()),
            pos2(left + width, used.bottom() + 48.0),
        ));
    });

    // The section in view (web `useCurrentSection`): a jump marks its
    // section until the reader scrolls on.
    if let Some(id) = jump {
        st.jumped = Some((id, scrolled));
    }
    if st
        .jumped
        .is_some_and(|(_, at)| (scrolled - at).abs() >= 2.0)
    {
        st.jumped = None;
    }
    let current = st.jumped.map(|(id, _)| id).or_else(|| {
        let at_end = scrolled > 0.0 && scrolled + body.height() >= st.content_height - 2.0;
        let tops: Vec<(&'static str, f32)> = ids
            .iter()
            .filter_map(|(id, _)| st.anchors.get(id).map(|y| (*id, y - scrolled)))
            .collect();
        current_section(&tops, at_end)
    });
    ui.painter().rect_filled(nav, 0.0, c.bg);
    crate::widgets::rule_h(ui.painter(), c.border, nav.x_range(), nav.bottom() - 1.0);
    for (id, b, response) in &links {
        let r = response.rect;
        let on = current == Some(*id);
        if on {
            ctx.accesskit_node_builder(response.id, |node| {
                node.set_aria_current(egui::accesskit::AriaCurrent::Location);
            });
        }
        let hover = eased(
            &ctx,
            response.id,
            response.hovered(),
            Color32::TRANSPARENT,
            c.hover,
        );
        ui.painter()
            .rect_filled(r, corner(16.0), if on { c.selected } else { hover });
        if response.has_focus() && crate::widgets::keyboard_focus(&ctx) {
            crate::widgets::focus_ring(ui.painter(), r, 16.0, &theme);
        }
        b.paint_tinted(
            ui.painter(),
            pos2(r.left() + 12.0, r.center().y - b.height() / 2.0),
            Some(if on || response.hovered() {
                c.text
            } else {
                c.muted
            }),
        );
        if response.hovered() {
            ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if response.clicked() {
            ctx.request_repaint();
        }
    }

    if let Some(dialog) = &mut st.inviting
        && !dialog.show(&ctx, app, &detail.server)
    {
        st.inviting = None;
        reload_invites(app, st, server);
    }
    drop(s);
    controls::show_confirm(&ctx);
}

/// Whether a copy can land anywhere: without a display server (Linux) there
/// is no clipboard, and egui would drop the text without a word.
fn clipboard_reachable() -> bool {
    if cfg!(target_os = "linux") {
        std::env::var_os("WAYLAND_DISPLAY").is_some() || std::env::var_os("DISPLAY").is_some()
    } else {
        true
    }
}

fn load_error(ui: &mut Ui, app: &AppCtx, rect: Rect, server: Uuid) {
    let theme = Theme::of(ui.ctx());
    let c = &theme.c;
    let area = Rect::from_min_size(
        pos2(rect.left() + 24.0, rect.top() + 52.0 + 24.0),
        vec2(rect.width() - 48.0, 200.0),
    );
    controls::place(ui, UiBuilder::new().max_rect(area), |ui| {
        let msg = "Die Servereinstellungen konnten nicht geladen werden.";
        let block = Block::wrapped(ui, msg, text::BODY, c.text, area.width() - 30.0);
        let h = 20.0 + block.height() + 12.0 + 32.0;
        let r = Rect::from_min_size(area.min, vec2(area.width(), h));
        ui.painter().rect_filled(
            r,
            corner(theme.radius.sm),
            crate::theme::mix(c.panel, c.danger_bg, 0.18),
        );
        let bar = Rect::from_min_max(r.min, pos2(r.left() + 3.0, r.bottom()));
        ui.painter()
            .with_clip_rect(bar)
            .rect_filled(r, corner(theme.radius.sm), c.danger);
        block.paint(ui.painter(), pos2(r.left() + 15.0, r.top() + 10.0));
        let br = Rect::from_min_size(
            pos2(r.left() + 15.0, r.top() + 10.0 + block.height() + 12.0),
            vec2(200.0, 32.0),
        );
        controls::place(ui, UiBuilder::new().max_rect(br), |ui| {
            if ui.add(Button::new("Erneut versuchen").small()).clicked() {
                app.client.refresh_server(server);
            }
        });
    });
}

fn reload_invites(app: &AppCtx, s: &mut State, server: Uuid) {
    let http = app.client.http().clone();
    s.invites_load = Some(Pending::spawn(app.client, async move {
        http.invites(server).await
    }));
}

fn reload_bans(app: &AppCtx, s: &mut State, server: Uuid) {
    let http = app.client.http().clone();
    s.bans_load = Some(Pending::spawn(app.client, async move {
        http.bans(server).await
    }));
}

fn poll_loads(ui: &Ui, app: &mut AppCtx, s: &mut State, detail: &ServerDetail, manage: bool) {
    let id = detail.server.id;
    if manage && s.invites.is_none() && s.invites_load.is_none() {
        reload_invites(app, s, id);
    }
    if manage && s.bans.is_none() && s.bans_load.is_none() {
        reload_bans(app, s, id);
    }
    if let Some(r) = poll(&mut s.invites_load) {
        s.invites = Some(r);
    }
    if let Some(r) = poll(&mut s.bans_load) {
        s.bans = Some(r);
    }
    if let Some(r) = poll(&mut s.revoke) {
        match r {
            Ok(()) => s.invites_status.success(ui, "Link widerrufen."),
            // useRevokeInvite: a toast, not the section's status.
            Err(e) => app.toasts.failure("Einladung nicht widerrufen.", &e),
        }
        reload_invites(app, s, id);
    }
    if let Some(r) = poll(&mut s.unban) {
        if let Err(e) = r {
            app.toasts.failure("Mitglied nicht entsperrt.", &e);
        }
        reload_bans(app, s, id);
    }
    if let Some(r) = poll(&mut s.icon) {
        match r {
            Ok(_) => {
                s.icon_status.success(ui, "Gespeichert.");
                app.client.refresh_server(id);
                app.client.refresh_servers();
            }
            Err(e) => s
                .icon_status
                .error(ui, error_message(Failure::from_api(&e).code.as_str())),
        }
    }
    let key = format!("server_update:{id}");
    let st = app.client.with_state(|st| st.op(&key).cloned());
    match st {
        Some(OpState::Done) => {
            app.client.clear_op(key);
            if s.saving_name {
                s.saving_name = false;
                s.name = None;
                s.name_status.success(ui, "Gespeichert.");
            }
            if s.saving_permissions {
                s.saving_permissions = false;
                s.permissions_status.success(ui, "Gespeichert.");
            }
        }
        Some(OpState::Failed(e)) => {
            app.client.clear_op(key);
            if s.saving_name {
                s.saving_name = false;
                s.name_errors
                    .from_failure(&Failure::from_api(&e), &["name"]);
            }
            if s.saving_permissions {
                s.saving_permissions = false;
                s.permissions = None;
                s.permissions_status.error(ui, NOT_SAVED);
            }
        }
        _ => {}
    }
    // Kick and ban answers, then the server detail reloads by itself.
    if let Some(user) = s.target {
        if let Some(true) = controls::answer(ui.ctx(), &format!("kick:{user}")) {
            app.client.kick(id, user);
            s.target = None;
        } else if let Some(true) = controls::answer(ui.ctx(), &format!("ban:{user}")) {
            app.client.ban(id, user);
            s.target = None;
            reload_bans(app, s, id);
        }
    }
    let unbans: Vec<Uuid> = s
        .bans
        .as_ref()
        .and_then(|b| b.as_ref().ok())
        .map(|b| b.iter().map(|x| x.user_id).collect())
        .unwrap_or_default();
    for user in unbans {
        if let Some(true) = controls::answer(ui.ctx(), &format!("unban:{user}")) {
            let http = app.client.http().clone();
            s.unban = Some(Pending::spawn(app.client, async move {
                http.unban(id, user).await
            }));
        }
    }
    let codes: Vec<String> = s
        .invites
        .as_ref()
        .and_then(|b| b.as_ref().ok())
        .map(|b| b.iter().map(|x| x.code.clone()).collect())
        .unwrap_or_default();
    for code in codes {
        if let Some(true) = controls::answer(ui.ctx(), &format!("revoke:{code}")) {
            let http = app.client.http().clone();
            s.revoke = Some(Pending::spawn(app.client, async move {
                http.revoke_invite(&code).await
            }));
        }
    }
    if let Some(true) = controls::answer(ui.ctx(), &format!("drop-server:{id}")) {
        if detail.server.role == Role::Owner {
            app.client.delete_server(id);
        } else {
            app.client.leave_server(id);
        }
        app.nav.replace("/");
    }
}

/// `.server-tile.server-settings-icon`: 64px, radius 22, tint or image.
fn icon_tile(ui: &mut Ui, app: &AppCtx, detail: &ServerDetail) {
    let theme = Theme::of(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(vec2(64.0, 64.0), Sense::hover());
    let id = Id::unique(("gelabber-settings-tile", detail.server.id));
    let t = ui
        .ctx()
        .animate_bool_with_time(id, response.hovered(), crate::theme::motion::FAST);
    let radius = 22.0 + (10.0 - 22.0) * t;
    let painter = ui.painter();
    // The rail's hover pill, 10px left of the tile.
    if t > 0.0 {
        let pill = Rect::from_center_size(
            pos2(rect.left() - 10.0 + 2.0, rect.center().y),
            vec2(4.0, 16.0 * t),
        );
        painter.rect_filled(
            pill,
            egui::CornerRadius {
                nw: 0,
                sw: 0,
                ne: 4,
                se: 4,
            },
            theme.c.muted,
        );
    }
    let texture =
        detail
            .server
            .icon_url
            .as_deref()
            .and_then(|u| match app.images.get(ui.ctx(), u) {
                ImageState::Ready(t) => Some(t),
                _ => None,
            });
    match texture {
        Some(tex) => {
            let uv = crate::images::cover_uv(tex.size_vec2(), rect.size());
            painter.add(
                egui::epaint::RectShape::filled(rect, corner(radius), Color32::WHITE)
                    .with_texture(tex.id(), uv),
            );
        }
        None => {
            painter.rect_filled(
                rect,
                corner(radius),
                theme.tile(tile_tone(&detail.server.id.to_string())),
            );
            let style = TextStyle::new(20.0, 600.0, 1.0).tracking(0.01);
            let b = Block::single(ui, &tile_initials(&detail.server.name), style, theme.c.text);
            b.paint(
                painter,
                pos2(
                    rect.center().x - b.width() / 2.0,
                    rect.center().y - b.height() / 2.0,
                ),
            );
        }
    }
}

fn general(ui: &mut Ui, app: &mut AppCtx, s: &mut State, detail: &ServerDetail) {
    let id = detail.server.id;
    if let Some(slot) = &s.picking {
        let done = slot.lock().take();
        if let Some(file) = done {
            s.picking = None;
            if let Some((bytes, _)) = file {
                match controls::prepare_image(&bytes, 256, 256) {
                    Ok((data, kind)) => {
                        s.icon_removing = false;
                        s.icon_status.clear();
                        let http = app.client.http().clone();
                        let kind = kind.to_owned();
                        s.icon = Some(Pending::spawn(app.client, async move {
                            http.upload_server_icon(id, &kind, data).await
                        }));
                    }
                    Err(m) => s.icon_status.error(ui, m),
                }
            }
        }
    }
    let busy = s.icon.is_some();
    controls::row(ui, 16.0, 64.0, |ui| {
        icon_tile(ui, app, detail);
        ui.spacing_mut().item_spacing.x = 8.0;
        if ui
            .add(
                Button::new("Bild hochladen")
                    .small()
                    .disabled(busy && s.icon_removing)
                    .pending((busy && !s.icon_removing).then_some("Lädt hoch…")),
            )
            .clicked()
            && !busy
            && s.picking.is_none()
        {
            s.picking = Some(controls::pick_file(
                ui.ctx(),
                "Serverbild auswählen",
                &controls::IMAGE_FILTER,
            ));
        }
        if detail.server.icon_url.is_some()
            && ui
                .add(
                    Button::new("Entfernen")
                        .small()
                        .ghost()
                        .disabled(busy && !s.icon_removing)
                        .pending((busy && s.icon_removing).then_some("Entfernt…")),
                )
                .clicked()
            && !busy
        {
            s.icon_removing = true;
            s.icon_status.clear();
            let http = app.client.http().clone();
            s.icon = Some(Pending::spawn(app.client, async move {
                http.remove_server_icon(id).await
            }));
        }
        if let Some((m, t)) = s.icon_status.current(ui) {
            ui.add(crate::widgets::InlineStatus::new(Some(&m), t));
        }
    });
    ui.add_space(14.0);
    controls::column(ui, 560.0, |ui| {
        let mut name = s.name.clone().unwrap_or_else(|| detail.server.name.clone());
        let field = TextField::new("server-name", &mut name)
            .label("Servername")
            .char_limit(100)
            .error(s.name_errors.field("name"))
            .show(ui);
        if field.response.changed() {
            s.name_errors.clear_field("name");
            s.name = Some(name.clone());
        }
        let dirty = name.trim() != detail.server.name;
        ui.add_space(20.0);
        form_error(ui, &s.name_errors);
        let mut submit = field.submitted;
        controls::row(ui, 12.0, 36.0, |ui| {
            submit |= ui
                .add(
                    Button::new("Speichern")
                        .primary()
                        .disabled(!dirty)
                        .pending(s.saving_name.then_some("Speichert…")),
                )
                .clicked();
            let status = s.name_status.current(ui);
            // As on Profil: why Speichern waits, next to it.
            if !dirty && !s.saving_name && status.is_none() {
                controls::paint(
                    ui,
                    pos2(ui.cursor().min.x, ui.max_rect().center().y - 9.0),
                    "Keine Änderungen.",
                    controls::HINT,
                    Theme::of(ui.ctx()).c.muted,
                );
            }
            if let Some((m, t)) = status {
                ui.add(crate::widgets::InlineStatus::new(Some(&m), t));
            }
        });
        if submit && dirty && !s.saving_name {
            let v = name.trim();
            let code = if v.is_empty() {
                Some("required")
            } else if v.chars().count() > 100 {
                Some("too_long")
            } else {
                None
            };
            match code {
                Some(code) => s
                    .name_errors
                    .set_fields(vec![("name", field_message("name", code))]),
                None => {
                    s.name_status.clear();
                    s.saving_name = true;
                    app.client.update_server(
                        id,
                        ServerPatch {
                            name: Some(name.clone()),
                            member_permissions: None,
                        },
                    );
                }
            }
        }
        // The web form has no form ref, so focus stays where it was.
        s.name_errors.focus = None;
    });
}

fn list_top(ui: &mut Ui) {
    let theme = Theme::of(ui.ctx());
    let top = ui.cursor().min;
    crate::widgets::rule_h(
        ui.painter(),
        theme.c.border,
        top.x..=top.x + ui.available_width(),
        top.y,
    );
    ui.add_space(1.0);
}

fn permissions(ui: &mut Ui, app: &mut AppCtx, s: &mut State, detail: &ServerDetail) {
    let current = s
        .permissions
        .clone()
        .unwrap_or_else(|| detail.server.member_permissions.clone());
    list_top(ui);
    for (perm, label, desc) in PERMISSIONS {
        let mut on = current.contains(&perm);
        if controls::switch_row(ui, ("perm", label), label, Some(desc), &mut on, false) {
            let next: Vec<Permission> = Permission::ALL
                .iter()
                .copied()
                .filter(|p| if *p == perm { on } else { current.contains(p) })
                .collect();
            s.permissions = Some(next.clone());
            s.saving_permissions = true;
            s.permissions_status.clear();
            app.client.update_server(
                detail.server.id,
                ServerPatch {
                    name: None,
                    member_permissions: Some(next),
                },
            );
        }
    }
    // A finished save hands over to the server's mask again.
    if !s.saving_permissions && s.permissions.as_ref() == Some(&detail.server.member_permissions) {
        s.permissions = None;
    }
    ui.add_space(14.0);
    let w = ui.available_width();
    s.permissions_status.ui(ui, w);
}

fn row_frame(ui: &mut Ui) -> Rect {
    let theme = Theme::of(ui.ctx());
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, 56.0), Sense::hover());
    crate::widgets::rule_h(
        ui.painter(),
        theme.c.border,
        rect.x_range(),
        rect.bottom() - 1.0,
    );
    rect
}

fn hint(ui: &mut Ui, s: &str) {
    controls::hint(ui, s);
}

/// `"läuft in 6 Tagen ab"` (servers/inviteText.ts).
fn expiry(at: Option<&chrono::DateTime<chrono::Utc>>, now_ms: f64) -> String {
    let Some(at) = at else {
        return "läuft nie ab".into();
    };
    let left = at.timestamp_millis() as f64 - now_ms;
    if left <= 0.0 {
        return "abgelaufen".into();
    }
    let minutes = (left / 60_000.0).ceil() as i64;
    if minutes < 60 {
        return format!(
            "läuft in {minutes} {} ab",
            if minutes == 1 { "Minute" } else { "Minuten" }
        );
    }
    let hours = (minutes as f64 / 60.0).round() as i64;
    if hours < 48 {
        return format!(
            "läuft in {hours} {} ab",
            if hours == 1 { "Stunde" } else { "Stunden" }
        );
    }
    let days = (hours as f64 / 24.0).round() as i64;
    format!(
        "läuft in {days} {} ab",
        if days == 1 { "Tag" } else { "Tagen" }
    )
}

fn invites(ui: &mut Ui, app: &mut AppCtx, s: &mut State, detail: &ServerDetail) {
    let theme = Theme::of(ui.ctx());
    let c = &theme.c;
    controls::row(ui, 12.0, 36.0, |ui| {
        if ui.add(Button::new("Neuen Link erstellen")).clicked() {
            s.inviting = Some(invite::Dialog::new(app, detail.server.id));
        }
        if let Some((m, t)) = s.invites_status.current(ui) {
            ui.add(crate::widgets::InlineStatus::new(Some(&m), t));
        }
    });
    ui.add_space(14.0);
    match &s.invites {
        None => hint(ui, "Lade Einladungen…"),
        Some(Err(_)) => {
            hint(ui, "Die Einladungen konnten nicht geladen werden.");
            ui.add_space(8.0);
            if ui.add(Button::new("Erneut versuchen").small()).clicked() {
                reload_invites(app, s, detail.server.id);
            }
        }
        Some(Ok(list)) if list.is_empty() => hint(ui, "Keine aktiven Einladungen."),
        Some(Ok(list)) => {
            let list = list.clone();
            let now = ui.input(|i| i.time);
            list_top(ui);
            for inv in &list {
                let rect = row_frame(ui);
                let code_style = TextStyle::new(13.0, 400.0, 1.5);
                let mono = Block::new(
                    ui,
                    {
                        let mut f = code_style.format(c.text);
                        f.font_id = egui::FontId::monospace(13.0);
                        egui::text::LayoutJob::single_section(inv.code.clone(), f)
                    },
                    code_style,
                );
                let uses = format!(
                    "{}/{} benutzt · {}",
                    inv.uses,
                    inv.max_uses
                        .map(|m| m.to_string())
                        .unwrap_or_else(|| "∞".into()),
                    expiry(inv.expires_at.as_ref(), s.opened_at)
                );
                let hb = Block::single(ui, &uses, controls::HINT, c.muted);
                let total = mono.height() + 2.0 + hb.height();
                let y = rect.top() + (55.0 - total) / 2.0;
                mono.paint(ui.painter(), pos2(rect.left(), y));
                hb.paint(ui.painter(), pos2(rect.left(), y + mono.height() + 2.0));
                // Actions right: Kopieren, Widerrufen.
                let revoke_w = Block::single(ui, "Widerrufen", text::UI, c.text).width() + 26.0;
                let copied = s.copied.get(&inv.code).is_some_and(|t| now - t < 1.5);
                let copy_label = if copied { "Kopiert" } else { "Kopieren" };
                let copy_w = Block::single(ui, "Kopieren", text::UI, c.text).width() + 26.0 + 24.0;
                let actions = Rect::from_min_size(
                    pos2(
                        rect.right() - revoke_w - 8.0 - copy_w,
                        rect.center().y - 16.5,
                    ),
                    vec2(revoke_w + 8.0 + copy_w, 32.0),
                );
                controls::place(
                    ui,
                    UiBuilder::new()
                        .max_rect(actions)
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                    |ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        if ui
                            .add(
                                Button::new(copy_label)
                                    .small()
                                    .icon(if copied { Icon::Check } else { Icon::Copy })
                                    .width(copy_w),
                            )
                            .clicked()
                        {
                            if clipboard_reachable() {
                                ui.ctx().copy_text(app.client.http().invite_link(&inv.code));
                                s.copied.insert(inv.code.clone(), now);
                                s.invites_status.clear();
                            } else {
                                s.invites_status.error(
                                    ui,
                                    format!("Kopieren ging nicht. Der Code ist {}.", inv.code),
                                );
                            }
                            ui.ctx()
                                .request_repaint_after(std::time::Duration::from_millis(1500));
                        }
                        if ui
                            .add(
                                Button::new("Widerrufen")
                                    .small()
                                    .variant(ButtonVariant::DangerOutline),
                            )
                            .clicked()
                        {
                            controls::ask(
                                ui.ctx(),
                                Confirm::danger(
                                    &format!("revoke:{}", inv.code),
                                    "Link widerrufen?",
                                    Some(
                                        "Der Link funktioniert danach nicht mehr. Bestehende Mitglieder bleiben.",
                                    ),
                                    "Widerrufen",
                                ),
                            );
                        }
                    },
                );
            }
        }
    }
}

fn presence(p: PresenceStatus) -> Presence {
    match p {
        PresenceStatus::Online => Presence::Online,
        PresenceStatus::Idle => Presence::Away,
        PresenceStatus::Offline => Presence::Offline,
    }
}

fn members(ui: &mut Ui, app: &mut AppCtx, s: &mut State, detail: &ServerDetail, manage: bool) {
    let theme = Theme::of(ui.ctx());
    let c = &theme.c;
    let me_id = me(app).map(|u| u.id);
    list_top(ui);
    let members: Vec<Member> = detail.members.clone();
    for m in &members {
        let rect = row_frame(ui);
        let status = app
            .client
            .with_state(|st| st.presence_of(detail.server.id, m.user_id));
        let av = Rect::from_min_size(pos2(rect.left(), rect.top() + 11.5), vec2(32.0, 32.0));
        Avatar::new(&m.name)
            .image(m.avatar_url.as_deref(), app.images)
            .presence(Some(presence(status)))
            .paint(ui, av);
        let allowed = manage && m.role != Role::Owner && Some(m.user_id) != me_id;
        let menu_w = if allowed { 32.0 + 12.0 } else { 0.0 };
        let owner = m.role == Role::Owner;
        let label = if owner { "Inhaber" } else { "Mitglied" };
        let badge_w = Block::single(ui, &label.to_uppercase(), text::CAPS, c.text).width() + 12.0;
        let name_w = rect.width() - 32.0 - 12.0 - 12.0 - badge_w - menu_w;
        let nb = Block::single(ui, &m.name, text::UI, c.text);
        let clip = Rect::from_min_size(
            pos2(av.right() + 12.0, rect.top()),
            vec2(name_w, rect.height()),
        );
        nb.paint(
            &ui.painter().with_clip_rect(clip),
            pos2(clip.left(), rect.top() + (55.0 - nb.height()) / 2.0),
        );
        let badge = Rect::from_min_size(
            pos2(rect.right() - menu_w - badge_w, rect.top() + 17.5),
            vec2(badge_w, 20.0),
        );
        controls::place(ui, UiBuilder::new().max_rect(badge), |ui| {
            ui.add(Badge::new(label).tone(if owner {
                BadgeTone::Accent
            } else {
                BadgeTone::Neutral
            }))
        });
        if allowed {
            let br = Rect::from_min_size(
                pos2(rect.right() - 32.0, rect.top() + 11.5),
                vec2(32.0, 32.0),
            );
            let resp = ui.add(
                IconButton::new(Icon::More, &format!("Aktionen für {}", m.name))
                    .size(32.0)
                    .icon_size(18.0)
                    .pressed(false)
                    .at(br),
            );
            if resp.clicked() {
                s.menu = if s.menu == Some(m.user_id) {
                    None
                } else {
                    Some(m.user_id)
                };
            }
            if s.menu == Some(m.user_id) {
                let shown = Popover::new(("member-menu", m.user_id), br)
                    .align_end()
                    // `.member-menu`: min-width 200 inside the 8px padding.
                    .min_width(218.0)
                    .show(ui.ctx(), |ui| {
                        use crate::screens::shell::overlays::profile::member_menu_item;
                        ui.spacing_mut().item_spacing.y = 0.0;
                        let kick = member_menu_item(ui, "Vom Server entfernen", false).clicked();
                        let ban = member_menu_item(ui, "Sperren", true).clicked();
                        (kick, ban)
                    });
                match shown {
                    Some((true, _)) => {
                        s.menu = None;
                        s.target = Some(m.user_id);
                        controls::ask(
                            ui.ctx(),
                            Confirm::danger(
                                &format!("kick:{}", m.user_id),
                                format!("{} vom Server entfernen?", m.name),
                                Some(
                                    "Mit einem neuen Einladungslink ist ein erneuter Beitritt möglich.",
                                ),
                                "Entfernen",
                            ),
                        );
                    }
                    Some((_, true)) => {
                        s.menu = None;
                        s.target = Some(m.user_id);
                        controls::ask(
                            ui.ctx(),
                            Confirm::danger(
                                &format!("ban:{}", m.user_id),
                                format!("{} sperren?", m.name),
                                Some(
                                    "Sie können nicht wieder beitreten, auch nicht mit einem Einladungslink.",
                                ),
                                "Sperren",
                            ),
                        );
                    }
                    Some(_) => {}
                    None => s.menu = None,
                }
            }
        }
    }
}

fn bans(ui: &mut Ui, app: &mut AppCtx, s: &mut State) {
    let theme = Theme::of(ui.ctx());
    let c = &theme.c;
    match &s.bans {
        None => hint(ui, "Lade Sperren…"),
        Some(Err(_)) => {
            hint(ui, "Die Sperren konnten nicht geladen werden.");
            ui.add_space(8.0);
            if ui.add(Button::new("Erneut versuchen").small()).clicked()
                && let Some(id) = s.server
            {
                reload_bans(app, s, id);
            }
        }
        Some(Ok(list)) if list.is_empty() => hint(ui, "Niemand ist gesperrt."),
        Some(Ok(list)) => {
            let list = list.clone();
            list_top(ui);
            for b in &list {
                let rect = row_frame(ui);
                let av =
                    Rect::from_min_size(pos2(rect.left(), rect.top() + 11.5), vec2(32.0, 32.0));
                Avatar::new(&b.name)
                    .image(b.avatar_url.as_deref(), app.images)
                    .presence(Some(Presence::Offline))
                    .paint(ui, av);
                let nb = Block::single(ui, &b.name, text::UI, c.text);
                nb.paint(
                    ui.painter(),
                    pos2(av.right() + 12.0, rect.top() + (55.0 - nb.height()) / 2.0),
                );
                let bw = Block::single(ui, "Entsperren", text::UI, c.text).width() + 26.0;
                let br = Rect::from_min_size(
                    pos2(rect.right() - bw, rect.center().y - 16.5),
                    vec2(bw, 32.0),
                );
                controls::place(ui, UiBuilder::new().max_rect(br), |ui| {
                    if ui.add(Button::new("Entsperren").small()).clicked() {
                        controls::ask(
                            ui.ctx(),
                            Confirm {
                                key: format!("unban:{}", b.user_id),
                                title: format!("{} wieder zulassen?", b.name),
                                message: Some(
                                    "Ein Beitritt per Einladungslink ist dann wieder möglich."
                                        .into(),
                                ),
                                label: "Entsperren".into(),
                                neutral: true,
                            },
                        );
                    }
                });
            }
        }
    }
}

fn danger(ui: &mut Ui, _app: &mut AppCtx, detail: &ServerDetail) {
    let owner = detail.server.role == Role::Owner;
    let label = if owner {
        "Server löschen"
    } else {
        "Server verlassen"
    };
    if ui
        .add(Button::new(label).variant(ButtonVariant::DangerOutline))
        .clicked()
    {
        let name = &detail.server.name;
        let confirm = if owner {
            Confirm::danger(
                &format!("drop-server:{}", detail.server.id),
                format!("Server „{name}“ endgültig löschen?"),
                Some("Kanäle, Nachrichten, Einladungen und Mitgliedschaften werden gelöscht."),
                "Server löschen",
            )
        } else {
            Confirm::danger(
                &format!("drop-server:{}", detail.server.id),
                format!("Server „{name}“ verlassen?"),
                None,
                "Verlassen",
            )
        };
        controls::ask(ui.ctx(), confirm);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invite_expiry_words() {
        let now = 1_000_000_000_000.0;
        let at = |ms: f64| chrono::DateTime::from_timestamp_millis((now + ms) as i64).unwrap();
        assert_eq!(expiry(None, now), "läuft nie ab");
        assert_eq!(expiry(Some(&at(-1.0)), now), "abgelaufen");
        assert_eq!(
            expiry(Some(&at(6.0 * 86_400_000.0)), now),
            "läuft in 6 Tagen ab"
        );
        assert_eq!(
            expiry(Some(&at(30.0 * 60_000.0)), now),
            "läuft in 30 Minuten ab"
        );
        assert_eq!(expiry(Some(&at(3_600_000.0)), now), "läuft in 1 Stunde ab");
    }

    /// The cases of pages/sectionSpy.test.ts.
    #[test]
    fn the_section_in_view() {
        let at = |tops: &[f32]| -> Vec<(&'static str, f32)> {
            ["s0", "s1", "s2"]
                .into_iter()
                .zip(tops.iter().copied())
                .collect()
        };
        assert_eq!(
            current_section(&at(&[32.0, 400.0, 900.0]), false),
            Some("s0")
        );
        assert_eq!(
            current_section(&at(&[120.0, 400.0, 900.0]), false),
            Some("s0")
        );
        assert_eq!(
            current_section(&at(&[-500.0, 15.0, 600.0]), false),
            Some("s1")
        );
        assert_eq!(
            current_section(&at(&[-500.0, 33.0, 600.0]), false),
            Some("s0")
        );
        assert_eq!(
            current_section(&at(&[-900.0, -400.0, -2.0]), false),
            Some("s2")
        );
        assert_eq!(
            current_section(&at(&[-900.0, -100.0, 300.0]), true),
            Some("s2")
        );
        assert_eq!(current_section(&[], false), None);
        assert_eq!(current_section(&[], true), None);
    }
}
