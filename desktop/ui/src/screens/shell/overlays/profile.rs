//! `MemberProfileDialog` and `UserProfileDialog` (components/*.tsx,
//! overview.css `.member-profile`): the profile card, a member's server
//! details, "Nachricht senden" or "Profil bearbeiten", and for managers the
//! collapsed "Moderation" with kick and ban (MemberActions.tsx,
//! memberActions.css), both asked first.

use egui::{Rect, Sense, Ui, Vec2, pos2};
use gelabber_client::{OpState, Permission, Role};
use uuid::Uuid;

use super::confirm::Question;
use crate::app::AppCtx;
use crate::icons::{self, Icon};
use crate::screens::settings::card::{CardPerson, paint_card};
use crate::screens::shell::common;
use crate::text::{self, Block, TextStyle};
use crate::theme::{Theme, corner};
use crate::widgets::{Button, ButtonSize, ButtonVariant, Modal, Presence, eased, hairline_h};

pub struct MemberDialog {
    server: Uuid,
    member: Uuid,
    moderation: bool,
    /// The person picked "Nachricht senden"; the dialog waits for the DM.
    opening: bool,
    requested: bool,
}

impl MemberDialog {
    pub fn new(server: Uuid, member: Uuid) -> Self {
        Self {
            server,
            member,
            moderation: false,
            opening: false,
            requested: false,
        }
    }
}

pub struct UserDialog {
    user: Uuid,
    requested: bool,
}

impl UserDialog {
    pub fn new(user: Uuid) -> Self {
        Self {
            user,
            requested: false,
        }
    }
}

/// `withCard`: what the list knows, filled in from `GET /api/users/{id}`.
fn card_of(
    s: &gelabber_client::State,
    user: Uuid,
    name: &str,
    avatar: Option<&str>,
    accent: Option<&str>,
    status: Option<&str>,
    decoration: Option<&str>,
) -> CardPerson {
    if let Some(me) = s.me().filter(|m| m.id == user) {
        // Your own card comes from the session, which every save updates.
        return CardPerson {
            name: me.name.clone(),
            avatar_url: me.avatar_url.clone(),
            banner_url: me.banner_url.clone(),
            banner_color: me.banner_color.clone(),
            accent_color: me.accent_color.clone(),
            pronouns: me.pronouns.clone(),
            status_text: me.status_text.clone(),
            bio: me.bio.clone(),
            avatar_decoration: me.avatar_decoration.clone(),
        };
    }
    let p = s.profiles.get(&user);
    CardPerson {
        name: p.map_or_else(|| name.to_owned(), |p| p.name.clone()),
        avatar_url: p.map_or(avatar.map(str::to_owned), |p| p.avatar_url.clone()),
        banner_url: p.and_then(|p| p.banner_url.clone()),
        banner_color: p.and_then(|p| p.banner_color.clone()),
        accent_color: p.map_or(accent.map(str::to_owned), |p| p.accent_color.clone()),
        pronouns: p.and_then(|p| p.pronouns.clone()),
        status_text: p.map_or(status.map(str::to_owned), |p| p.status_text.clone()),
        bio: p.and_then(|p| p.bio.clone()),
        avatar_decoration: p.map_or(decoration.map(str::to_owned), |p| {
            p.avatar_decoration.clone()
        }),
    }
}

/// The card as a block in the dialog body.
fn card(ui: &mut Ui, app: &AppCtx, person: &CardPerson, presence: Presence) {
    let width = ui.available_width();
    let min = ui.cursor().min;
    // Measured on a throwaway pass, then painted where the space is taken.
    let height = paint_card(
        &ui.new_child(egui::UiBuilder::new().invisible()),
        app.images,
        min,
        width,
        person,
        Some(presence),
    );
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    paint_card(ui, app.images, rect.min, width, person, Some(presence));
}

const PRIMARY: TextStyle = TextStyle::new(14.0, 650.0, 1.5);

/// `.lr-primary`: padding 12/18, radius 9, 650 14px, an optional 19px icon.
fn lr_primary(ui: &mut Ui, icon: Option<Icon>, label: &str, disabled: bool) -> egui::Response {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let block = Block::single(ui, label, PRIMARY, c.accent_ink);
    let icon_w = if icon.is_some() { 19.0 + 10.0 } else { 0.0 };
    let size = Vec2::new(
        2.0 + 36.0 + icon_w + block.width(),
        2.0 + 24.0 + PRIMARY.line_px(),
    );
    let sense = if disabled {
        Sense::hover()
    } else {
        Sense::click()
    };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, !disabled, label));
    let rect = crate::widgets::snap(ui.ctx(), rect);
    let painter = ui.painter();
    if response.has_focus() && crate::widgets::keyboard_focus(ui.ctx()) {
        crate::widgets::focus_ring(painter, rect, 9.0, &theme);
    }
    let fill = eased(
        ui.ctx(),
        response.id.with("hover"),
        response.hovered() && !disabled,
        c.accent,
        crate::theme::brightness(c.accent, 1.08),
    );
    let alpha = if disabled { 0.55 } else { 1.0 };
    painter.rect_filled(rect, corner(9.0), fill.gamma_multiply(alpha));
    let content = icon_w + block.width();
    let mut x = rect.center().x - content / 2.0;
    let ink = c.accent_ink.gamma_multiply(alpha);
    if let Some(icon) = icon {
        icons::paint(painter, icon, pos2(x, rect.center().y - 9.5), 19.0, ink);
        x += icon_w;
    }
    block.paint_tinted(
        painter,
        pos2(x, rect.center().y - PRIMARY.line_px() / 2.0),
        Some(ink),
    );
    if response.hovered() {
        ui.ctx().set_cursor_icon(if disabled {
            egui::CursorIcon::Wait
        } else {
            egui::CursorIcon::PointingHand
        });
    }
    response
}

/// The actions row: gap 12, as tall as its tallest button.
fn actions(ui: &mut Ui, height: f32, add: impl FnOnce(&mut Ui)) {
    super::row(ui, height, 12.0, add);
}

/// `.lr-primary` height: one 21px line, padding 12, a 1px border.
const PRIMARY_H: f32 = 2.0 + 24.0 + 21.0;

/// The ghost "Schließen", stretched to the row's height (flex
/// `align-items: stretch`).
fn close_button(ui: &mut Ui, height: f32) -> egui::Response {
    ui.add(Button::new("Schließen").ghost().height(height))
}

const DT: TextStyle = TextStyle::new(13.0, 400.0, 1.5);

/// One `dl` row: dt left, dd right, baselines aligned.
fn detail(ui: &mut Ui, dt: &str, dd: &str) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let width = ui.available_width();
    let left = Block::single(ui, dt, DT, c.muted);
    let right = Block::wrapped(ui, dd, text::BODY, c.text, width - left.width() - 20.0);
    let h = right.height().max(left.height());
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, h), Sense::hover());
    right.paint(ui.painter(), pos2(rect.right() - right.width(), rect.top()));
    let top = common::baseline_top(text::BODY, rect.top(), DT);
    left.paint(ui.painter(), pos2(rect.left(), top));
}

pub fn member_dialog(ui: &mut Ui, app: &mut AppCtx, d: &mut MemberDialog) -> bool {
    let ctx = ui.ctx().clone();
    let theme = Theme::of(&ctx);
    let c = theme.c;
    struct Data {
        server_name: String,
        role: Role,
        joined: String,
        person: CardPerson,
        presence: Presence,
        name: String,
        me: bool,
        room: Option<(Uuid, String)>,
        moderate: bool,
        busy: bool,
        dm: Option<Uuid>,
    }
    let data = app.client.with_state(|s| {
        let detail = s.detail(d.server)?;
        let m = detail.members.iter().find(|m| m.user_id == d.member)?;
        let me = s.me().map(|u| u.id);
        let own = me == Some(m.user_id);
        let occupant = if own {
            (s.voice_self.server == Some(d.server))
                .then_some(s.voice_self.channel)
                .flatten()
        } else {
            s.voice
                .get(&d.server)
                .and_then(|r| r.get(&m.user_id))
                .map(|o| o.channel_id)
        };
        let room = occupant.and_then(|ch| {
            detail
                .channels
                .iter()
                .find(|c| c.id == ch)
                .map(|c| (c.id, c.name.clone()))
        });
        let local = m.joined_at.with_timezone(&chrono::Local);
        Some(Data {
            server_name: detail.server.name.clone(),
            role: m.role,
            joined: local.format("%d.%m.%Y").to_string(),
            person: card_of(
                s,
                m.user_id,
                &m.name,
                m.avatar_url.as_deref(),
                m.accent_color.as_deref(),
                m.status_text.as_deref(),
                m.avatar_decoration.as_deref(),
            ),
            presence: common::presence(s.presence_of(d.server, m.user_id)),
            name: m.name.clone(),
            me: own,
            room,
            moderate: detail.server.can(Permission::ManageServer) && m.role != Role::Owner && !own,
            busy: [format!("kick:{}", m.user_id), format!("ban:{}", m.user_id)]
                .iter()
                .any(|k| matches!(s.op(k), Some(OpState::Pending))),
            dm: s.dms.iter().find(|x| x.peer.id == m.user_id).map(|x| x.id),
        })
    });
    // Gone from the server (kicked, left): the dialog closes.
    let Some(data) = data else {
        return false;
    };
    if !d.requested {
        d.requested = true;
        if !data.me {
            app.client.load_profile(d.member);
        }
    }
    if d.opening {
        if let Some(dm) = data.dm {
            app.nav.navigate(&format!("/d/{dm}"));
            return false;
        }
        if matches!(
            app.client.with_state(|s| s.op("dm").cloned()),
            Some(OpState::Failed(_))
        ) {
            d.opening = false;
        }
    }
    let title = if data.me {
        "Dein Profil".to_owned()
    } else {
        format!("Profil von {}", data.name)
    };
    let mut done = false;
    let mut go: Option<String> = None;
    let mut ask: Option<Question> = None;
    let first = crate::widgets::field::mounted(&ctx, egui::Id::unique("gelabber-member-open"));
    let output = Modal::new("member-profile", &title).show(
        &ctx,
        |ui| {
            ui.spacing_mut().item_spacing.y = 22.0;
            card(ui, app, &data.person, data.presence);
            ui.scope(|ui| {
                ui.spacing_mut().item_spacing.y = 12.0;
                detail(ui, "Server", &data.server_name);
                detail(
                    ui,
                    "Mitgliedschaft",
                    if data.role == Role::Owner {
                        "Inhaber"
                    } else {
                        "Mitglied"
                    },
                );
                detail(ui, "Dabei seit", &data.joined);
            });
            if let Some((channel, name)) = &data.room {
                let label = format!("In {name}");
                let block = Block::single(ui, &label, text::BODY, c.accent_text);
                let (rect, response) = ui.allocate_exact_size(
                    Vec2::new(20.0 + 10.0 + block.width(), text::BODY.line_px()),
                    Sense::click(),
                );
                response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Link, true, &label));
                icons::paint(
                    ui.painter(),
                    Icon::Speaker,
                    pos2(rect.left(), rect.center().y - 10.0),
                    20.0,
                    c.accent_text,
                );
                block.paint(ui.painter(), pos2(rect.left() + 30.0, rect.top()));
                if response.hovered() {
                    ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                if response.has_focus() && crate::widgets::keyboard_focus(&ctx) {
                    crate::widgets::focus_ring(ui.painter(), rect, 3.0, &theme);
                }
                // The first focusable element takes the focus as it opens.
                if first {
                    response.request_focus();
                }
                if response.clicked() {
                    go = Some(format!("/s/{}/c/{channel}", d.server));
                }
            }
            actions(ui, PRIMARY_H, |ui| {
                let primary = if data.me {
                    lr_primary(ui, None, "Profil bearbeiten", false)
                } else if d.opening {
                    lr_primary(ui, Some(Icon::Chat), "Unterhaltung wird geöffnet…", true)
                } else {
                    lr_primary(ui, Some(Icon::Chat), "Nachricht senden", false)
                };
                if first && data.room.is_none() {
                    primary.request_focus();
                }
                if primary.clicked() {
                    if data.me {
                        go = Some("/settings/profil".into());
                    } else if let Some(dm) = data.dm {
                        go = Some(format!("/d/{dm}"));
                    } else {
                        app.client.start_dm(d.member);
                        d.opening = true;
                    }
                }
                if close_button(ui, primary.rect.height()).clicked() {
                    done = true;
                }
            });
            if data.moderate {
                moderation(ui, d, &data.name, data.busy, &mut ask);
            }
        },
        None,
    );
    if let Some(q) = ask {
        super::ask(&ctx, q);
    }
    if let Some(href) = go {
        app.nav.navigate(&href);
        return false;
    }
    !(output.close_requested || done)
}

/// `.member-moderation`: a hairline, the caps summary with its chevron,
/// then the two small buttons when open.
fn moderation(
    ui: &mut Ui,
    d: &mut MemberDialog,
    name: &str,
    busy: bool,
    ask: &mut Option<Question>,
) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 1.0 + 12.0 + 32.0), Sense::hover());
    hairline_h(ui.painter(), &theme, rect.left()..=rect.right(), rect.top());
    let summary = Rect::from_min_max(pos2(rect.left(), rect.top() + 13.0), rect.max);
    let response = ui.interact(summary, ui.scope_id().with("moderation"), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Moderation"));
    let color = if response.hovered() { c.text } else { c.muted };
    let block = Block::single(ui, "MODERATION", text::CAPS, color);
    block.paint(
        ui.painter(),
        pos2(summary.left(), summary.center().y - block.height() / 2.0),
    );
    let chevron = pos2(
        summary.left() + block.width() + 4.0,
        summary.center().y - 7.0,
    );
    if d.moderation {
        icons::paint_rotated(ui.painter(), Icon::Chevron, chevron, 14.0, color);
    } else {
        icons::paint(ui.painter(), Icon::Chevron, chevron, 14.0, color);
    }
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if response.has_focus() && crate::widgets::keyboard_focus(ui.ctx()) {
        crate::widgets::focus_ring(ui.painter(), summary, theme.radius.sm, &theme);
    }
    if response.clicked() {
        d.moderation = !d.moderation;
    }
    if !d.moderation {
        return;
    }
    ui.add_space(8.0 - 22.0);
    super::row(ui, 32.0, 8.0, |ui| {
        let kick = ui.add(
            Button::new("Vom Server entfernen")
                .variant(ButtonVariant::DangerOutline)
                .size(ButtonSize::Sm)
                .disabled(busy),
        );
        let ban = ui.add(
            Button::new("Sperren")
                .danger()
                .size(ButtonSize::Sm)
                .disabled(busy),
        );
        if kick.clicked() {
            *ask = Some(Question::kick(d.server, d.member, name));
        }
        if ban.clicked() {
            *ask = Some(Question::ban(d.server, d.member, name));
        }
    });
}

/// `.member-menu__item` of `MemberActionsMenu`: 36 high, padding 0 12,
/// hover fill, danger text for "Sperren".
pub fn member_menu_item(ui: &mut Ui, label: &str, danger: bool) -> egui::Response {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let width = ui.available_width().max(200.0);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 36.0), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    let fill = eased(
        ui.ctx(),
        response.id.with("hover"),
        response.hovered(),
        egui::Color32::TRANSPARENT,
        c.hover,
    );
    common::fill(ui.painter(), rect, theme.radius.sm, fill);
    if response.has_focus() && crate::widgets::keyboard_focus(ui.ctx()) {
        crate::widgets::focus_ring(ui.painter(), rect, theme.radius.sm, &theme);
    }
    let block = Block::single(ui, label, text::UI, if danger { c.danger } else { c.text });
    common::paint_mid(ui.painter(), &block, rect.left() + 12.0, rect.center().y);
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response
}

/// The DM peer's card (`UserProfileDialog`) with only "Schließen".
pub fn user_dialog(ui: &mut Ui, app: &mut AppCtx, d: &mut UserDialog) -> bool {
    let ctx = ui.ctx().clone();
    if !d.requested {
        d.requested = true;
        app.client.load_profile(d.user);
    }
    let data = app.client.with_state(|s| {
        let dm = s.dms.iter().find(|x| x.peer.id == d.user)?;
        let person = card_of(
            s,
            d.user,
            &dm.peer.name,
            dm.peer.avatar_url.as_deref(),
            None,
            None,
            None,
        );
        Some((person, common::presence(s.presence_of(dm.id, d.user))))
    });
    let Some((person, presence)) = data else {
        return false;
    };
    let title = format!("Profil von {}", person.name);
    let mut done = false;
    let first = crate::widgets::field::mounted(&ctx, egui::Id::unique("gelabber-user-open"));
    let output = Modal::new("user-profile", &title).show(
        &ctx,
        |ui| {
            ui.spacing_mut().item_spacing.y = 22.0;
            card(ui, app, &person, presence);
            actions(ui, 36.0, |ui| {
                let close = close_button(ui, 36.0);
                if first {
                    close.request_focus();
                }
                if close.clicked() {
                    done = true;
                }
            });
        },
        None,
    );
    !(output.close_requested || done)
}
