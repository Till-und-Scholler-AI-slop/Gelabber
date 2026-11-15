//! `/d/friends` and `/d/requests` (#563, web/src/friends/FriendsPage.tsx
//! and RequestsPage.tsx): the friend list (all · pending · add, with the
//! user's own friend code) and the incoming message requests.

use std::time::Duration;

use egui::{Id, Rect, Sense, Ui, UiBuilder, Vec2, pos2, vec2};
use gelabber_client::{
    DirectMessage, FriendAction, FriendList, FriendPerson, FriendTarget, OpState,
};
use gelabber_i18n::friends as t;

use super::common::{self, line, paint_mid};
use super::overlays::{self, ConfirmAction, Question};
use crate::app::AppCtx;
use crate::screens::settings::controls;
use crate::text::{self, Block};
use crate::theme::Theme;
use crate::widgets::{Avatar, AvatarSize, Button, TextField, hairline_h, load_error};
use uuid::Uuid;

/// The page body's width (`.friends-page__body`).
const MEASURE: f32 = 640.0;
const ROW: f32 = 56.0;
/// The list is read again this often while it is on screen: online dots
/// come with the list, not live.
const REFRESH: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Tab {
    #[default]
    All,
    Pending,
    Add,
}

fn tab_id() -> Id {
    Id::unique("friends-tab")
}

/// Reads `op` again once it is older than [`REFRESH`] on this page.
fn refresh(ui: &Ui, key: &'static str, load: impl FnOnce()) {
    let id = Id::unique(("friends-loaded", key));
    let now = ui.input(|i| i.time);
    let last = ui.data(|d| d.get_temp::<f64>(id));
    if last.is_none_or(|at| now - at > REFRESH.as_secs_f64()) {
        ui.data_mut(|d| d.insert_temp(id, now));
        load();
    }
    ui.ctx().request_repaint_after(REFRESH);
}

pub fn friends_page(ui: &mut Ui, app: &mut AppCtx, rect: Rect) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    ui.painter().rect_filled(rect, 0.0, c.bg);
    refresh(ui, "friends", || app.client.load_friends());
    controls::header(ui, rect, t::page_title(), None);
    let (list, op) = app
        .client
        .with_state(|s| (s.friends.clone(), s.op("friends").cloned()));
    let mut tab = ui.data(|d| d.get_temp::<Tab>(tab_id())).unwrap_or_default();

    let body = Rect::from_min_max(pos2(rect.left(), rect.top() + 52.0), rect.max);
    let width = (body.width() - 48.0).min(MEASURE);
    controls::scroll(ui, body, "friends-page", None, |ui| {
        ui.add_space(20.0);
        let indent = 24.0;
        ui.horizontal(|ui| {
            ui.add_space(indent);
            controls::column(ui, width, |ui| {
                let pending = list.as_ref().map_or(0, |l| l.incoming.len());
                tabs(ui, &mut tab, pending);
                ui.add_space(20.0);
                match tab {
                    Tab::Add => {
                        add_friend(ui, app, width);
                        ui.add_space(28.0);
                        code_card(ui, app, width);
                    }
                    _ => match (&list, &op) {
                        (Some(list), _) if tab == Tab::All => friend_rows(ui, app, list, width),
                        (Some(list), _) => pending_rows(ui, app, list, width),
                        (None, Some(OpState::Failed(_))) => {
                            if load_error(ui, t::list_load_failed(), false) {
                                app.client.load_friends();
                            }
                        }
                        (None, _) => {
                            crate::widgets::skeleton::main(
                                ui,
                                Rect::from_min_size(ui.cursor().min, vec2(width, 240.0)),
                                crate::widgets::skeleton::Kind::List,
                                Id::unique("friends-skeleton"),
                            );
                        }
                    },
                }
                ui.add_space(24.0);
            });
        });
    });
    ui.data_mut(|d| d.insert_temp(tab_id(), tab));
}

/// "Alle · Ausstehend (n) · Hinzufügen" (`ChipGroup`).
fn tabs(ui: &mut Ui, tab: &mut Tab, pending: usize) {
    let pending_label = if pending > 0 {
        format!("{} ({pending})", t::tab_pending())
    } else {
        t::tab_pending().to_owned()
    };
    controls::row(ui, 8.0, 32.0, |ui| {
        for (value, label) in [
            (Tab::All, t::tab_all()),
            (Tab::Pending, pending_label.as_str()),
            (Tab::Add, t::tab_add()),
        ] {
            let mut button = Button::new(label).small();
            button = if *tab == value {
                button.primary()
            } else {
                button.ghost()
            };
            if ui.add(button).clicked() {
                *tab = value;
            }
        }
    });
}

/// A muted line where a list is empty (`.friends-empty`).
fn empty(ui: &mut Ui, message: &str, width: f32) {
    let c = Theme::of(ui.ctx()).c;
    controls::paragraph(ui, message, text::BODY.line_height(1.6), c.muted, width);
}

/// One person's row: avatar, name with an optional detail, then the
/// buttons from the right. Returns the index of the button clicked.
pub(crate) fn person_row(
    ui: &mut Ui,
    app: &AppCtx,
    person: &FriendPerson,
    detail: Option<&str>,
    presence: Option<crate::widgets::Presence>,
    buttons: Vec<Button<'_>>,
    width: f32,
) -> Option<usize> {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let (rect, _) = ui.allocate_exact_size(vec2(width, ROW), Sense::hover());
    let avatar = Rect::from_min_size(pos2(rect.left(), rect.center().y - 18.0), Vec2::splat(36.0));
    Avatar::new(&person.name)
        .image(person.avatar_url.as_deref(), app.images)
        .size(AvatarSize::Md)
        .presence(presence)
        .ring(c.bg)
        .paint(ui, avatar);
    let mut right = rect.right();
    let mut clicked = None;
    for (i, button) in buttons.into_iter().enumerate().rev() {
        let w = button.natural_width(ui);
        let at = Rect::from_min_size(pos2(right - w, rect.center().y - 16.0), vec2(w, 32.0));
        if ui.put(at, button).clicked() {
            clicked = Some(i);
        }
        right = at.left() - 8.0;
    }
    let left = avatar.right() + 12.0;
    let name = line(
        ui,
        &person.name,
        text::BODY.weight(600.0),
        c.text,
        right - left,
    );
    match detail {
        Some(detail) => {
            let detail = line(ui, detail, text::META, c.muted, right - left);
            paint_mid(ui.painter(), &name, left, rect.center().y - 9.0);
            paint_mid(ui.painter(), &detail, left, rect.center().y + 10.0);
        }
        None => paint_mid(ui.painter(), &name, left, rect.center().y),
    }
    hairline_h(ui.painter(), &theme, rect.x_range(), rect.bottom() - 0.5);
    clicked
}

fn pending_op(app: &AppCtx, key: &str) -> bool {
    app.client
        .with_state(|s| matches!(s.op(key), Some(OpState::Pending)))
}

fn friend_rows(ui: &mut Ui, app: &mut AppCtx, list: &FriendList, width: f32) {
    if list.friends.is_empty() {
        empty(ui, t::list_empty(), width);
        return;
    }
    let opening = pending_op(app, "dm");
    for friend in &list.friends {
        let user = friend.user.id;
        let detail = friend.status.map(|s| match s {
            gelabber_client::PresenceStatus::Idle => t::status_idle(),
            _ => t::status_online(),
        });
        let presence = Some(common::presence(
            friend
                .status
                .unwrap_or(gelabber_client::PresenceStatus::Offline),
        ));
        let removing = pending_op(app, &FriendAction::Remove.op(user));
        let buttons = vec![
            Button::new(t::action_message()).small().disabled(opening),
            Button::new(t::action_remove())
                .small()
                .ghost()
                .disabled(removing),
        ];
        match person_row(ui, app, &friend.user, detail, presence, buttons, width) {
            Some(0) => message(app, user),
            Some(1) => overlays::ask(
                ui.ctx(),
                Question::danger(
                    t::action_remove_confirm(&friend.user.name),
                    Some(t::action_remove_hint()),
                    t::action_remove(),
                    ConfirmAction::RemoveFriend { user },
                ),
            ),
            _ => {}
        }
    }
}

/// "Nachricht": the known DM, else open one and go there.
fn message(app: &mut AppCtx, user: Uuid) {
    let known = app
        .client
        .with_state(|s| s.dms.iter().find(|d| d.peer.id == user).map(|d| d.id));
    match known {
        Some(dm) => app.nav.navigate(&format!("/d/{dm}")),
        None => app.client.start_dm(user),
    }
}

fn section_title(ui: &mut Ui, title: &str) {
    let c = Theme::of(ui.ctx()).c;
    let block = Block::single(ui, title, text::META.weight(650.0), c.muted);
    let (rect, _) =
        ui.allocate_exact_size(vec2(block.width(), block.height() + 6.0), Sense::hover());
    block.paint(ui.painter(), rect.min);
}

fn pending_rows(ui: &mut Ui, app: &mut AppCtx, list: &FriendList, width: f32) {
    if list.incoming.is_empty() && list.outgoing.is_empty() {
        empty(ui, t::pending_empty(), width);
        return;
    }
    if !list.incoming.is_empty() {
        section_title(ui, t::pending_incoming());
        for request in &list.incoming {
            let user = request.user.id;
            let buttons = vec![
                Button::new(t::action_accept())
                    .small()
                    .primary()
                    .disabled(pending_op(app, &FriendAction::Accept.op(user))),
                Button::new(t::action_decline())
                    .small()
                    .ghost()
                    .disabled(pending_op(app, &FriendAction::Decline.op(user))),
            ];
            match person_row(ui, app, &request.user, None, None, buttons, width) {
                Some(0) => app.client.friend_action(FriendAction::Accept, user),
                Some(1) => app.client.friend_action(FriendAction::Decline, user),
                _ => {}
            }
        }
        ui.add_space(20.0);
    }
    if !list.outgoing.is_empty() {
        section_title(ui, t::pending_outgoing());
        for request in &list.outgoing {
            let user = request.user.id;
            let buttons = vec![
                Button::new(t::action_withdraw())
                    .small()
                    .ghost()
                    .disabled(pending_op(app, &FriendAction::Withdraw.op(user))),
            ];
            if person_row(ui, app, &request.user, None, None, buttons, width).is_some() {
                app.client.friend_action(FriendAction::Withdraw, user);
            }
        }
    }
}

#[derive(Clone, Default)]
struct AddForm {
    code: String,
    /// The field's error, from the last answer.
    error: Option<String>,
    /// `friend_send` failures seen when the form last sent.
    failures: u64,
    sending: bool,
}

fn add_id() -> Id {
    Id::unique("friends-add-form")
}

/// "Freund hinzufügen" by code (`AddFriend`).
fn add_friend(ui: &mut Ui, app: &mut AppCtx, width: f32) {
    let theme = Theme::of(ui.ctx());
    let mut form = ui
        .data(|d| d.get_temp::<AddForm>(add_id()))
        .unwrap_or_default();
    let (op, failures) = app.client.with_state(|s| {
        (
            s.op("friend_send").cloned(),
            s.failures.get("friend_send").copied().unwrap_or(0),
        )
    });
    let pending = matches!(op, Some(OpState::Pending));
    if form.sending && !pending {
        form.sending = false;
        match &op {
            Some(OpState::Done) => form.code.clear(),
            Some(OpState::Failed(error)) if failures != form.failures => {
                form.error = code_error(error);
            }
            _ => {}
        }
    }
    let block = Block::single(ui, t::add_title(), text::TITLE_SM, theme.c.text);
    let (rect, _) = ui.allocate_exact_size(vec2(width, block.height() + 12.0), Sense::hover());
    block.paint(ui.painter(), rect.min);
    let before = form.code.clone();
    let out = TextField::new("friend-code", &mut form.code)
        .label(t::add_label())
        .hint(Some(t::add_hint()))
        .error(form.error.as_deref())
        .placeholder("K7QX-M2PD") // i18n-ok: the shape of a code
        .char_limit(16)
        .show(ui);
    if form.code != before {
        form.error = None;
    }
    ui.add_space(12.0);
    let blank = form.code.trim().is_empty();
    let send = ui
        .add(
            Button::new(t::add_submit())
                .primary()
                .disabled(pending || blank),
        )
        .clicked();
    if (send || out.submitted) && !pending && !blank {
        form.error = None;
        form.sending = true;
        form.failures = failures;
        app.client
            .send_friend_request(FriendTarget::Code(form.code.trim().to_owned()));
    }
    ui.data_mut(|d| d.insert_temp(add_id(), form));
}

/// The field's line for a refused code (`fieldMessages(err.fields).code`).
fn code_error(error: &gelabber_client::ApiError) -> Option<String> {
    let gelabber_client::ApiError::Api { fields, .. } = error else {
        return None;
    };
    let code = fields.get("code")?.as_str()?;
    Some(crate::rules::field_message("code", code))
}

/// "Dein Freundescode" with copy and a new code (`FriendCodeCard`).
pub fn code_card(ui: &mut Ui, app: &mut AppCtx, width: f32) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    refresh(ui, "friend_code", || app.client.load_friend_code());
    let (code, regenerating) = app.client.with_state(|s| {
        (
            s.friend_code.clone(),
            matches!(s.op("friend_code_new"), Some(OpState::Pending)),
        )
    });
    let block = Block::single(ui, t::code_title(), text::TITLE_SM, c.text);
    let (rect, _) = ui.allocate_exact_size(vec2(width, block.height() + 6.0), Sense::hover());
    block.paint(ui.painter(), rect.min);
    controls::paragraph(ui, t::code_hint(), controls::HINT, c.muted, width);
    ui.add_space(12.0);
    let shown = code.as_deref().unwrap_or(t::code_loading());
    let style = if code.is_some() {
        text::TITLE.weight(650.0)
    } else {
        text::BODY
    };
    let value = Block::single(
        ui,
        shown,
        style,
        if code.is_some() { c.text } else { c.muted },
    );
    let (at, response) =
        ui.allocate_exact_size(vec2(value.width(), value.height()), Sense::hover());
    value.paint(ui.painter(), at.min);
    crate::widgets::announce(&response, egui::Role::Label, shown);
    ui.add_space(12.0);
    controls::row(ui, 8.0, 32.0, |ui| {
        if ui
            .add(Button::new(t::code_copy()).small().disabled(code.is_none()))
            .clicked()
            && let Some(code) = &code
        {
            if crate::screens::chat::clipboard::copy(ui.ctx(), code) {
                app.toasts.notify(t::code_copied());
            } else {
                app.toasts.error(gelabber_i18n::friends::action_failed());
            }
        }
        if ui
            .add(
                Button::new(t::code_regenerate())
                    .small()
                    .ghost()
                    .disabled(regenerating),
            )
            .clicked()
        {
            overlays::ask(
                ui.ctx(),
                Question {
                    title: t::code_regenerate_confirm().to_owned(),
                    message: Some(t::code_regenerate_hint().to_owned()),
                    label: t::code_regenerate().to_owned(),
                    neutral: true,
                    action: ConfirmAction::RegenerateFriendCode,
                },
            );
        }
    });
}

// ------------------------------------------------------------- requests

pub fn requests_page(ui: &mut Ui, app: &mut AppCtx, rect: Rect) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    ui.painter().rect_filled(rect, 0.0, c.bg);
    refresh(ui, "dm_requests", || app.client.load_dm_requests());
    controls::header(ui, rect, t::requests_title(), None);
    let (list, op) = app
        .client
        .with_state(|s| (s.dm_requests.clone(), s.op("dm_requests").cloned()));
    let body = Rect::from_min_max(pos2(rect.left(), rect.top() + 52.0), rect.max);
    let width = (body.width() - 48.0).min(MEASURE);
    controls::scroll(ui, body, "requests-page", None, |ui| {
        ui.add_space(20.0);
        ui.horizontal(|ui| {
            ui.add_space(24.0);
            controls::column(ui, width, |ui| {
                controls::paragraph(ui, t::requests_hint(), controls::HINT, c.muted, width);
                ui.add_space(16.0);
                match (&list, &op) {
                    (Some(list), _) if list.is_empty() => empty(ui, t::requests_empty(), width),
                    (Some(list), _) => {
                        for dm in list {
                            request_row(ui, app, dm, width);
                        }
                    }
                    (None, Some(OpState::Failed(_))) => {
                        if load_error(ui, t::requests_load_failed(), false) {
                            app.client.load_dm_requests();
                        }
                    }
                    (None, _) => crate::widgets::skeleton::main(
                        ui,
                        Rect::from_min_size(ui.cursor().min, vec2(width, 240.0)),
                        crate::widgets::skeleton::Kind::List,
                        Id::unique("requests-skeleton"),
                    ),
                }
                ui.add_space(24.0);
            });
        });
    });
}

fn request_row(ui: &mut Ui, app: &mut AppCtx, dm: &DirectMessage, width: f32) {
    let person = FriendPerson {
        id: dm.peer.id,
        name: dm.peer.name.clone(),
        avatar_url: dm.peer.avatar_url.clone(),
    };
    let buttons = vec![
        Button::new(t::requests_open()).small(),
        Button::new(t::action_accept())
            .small()
            .primary()
            .disabled(pending_op(app, &format!("dm_accept:{}", dm.id))),
        Button::new(t::action_decline())
            .small()
            .ghost()
            .disabled(pending_op(app, &format!("dm_decline:{}", dm.id))),
    ];
    match person_row(ui, app, &person, None, None, buttons, width) {
        Some(0) => app.nav.navigate(&format!("/d/{}", dm.id)),
        Some(1) => app.client.accept_dm_request(dm.id),
        Some(2) => app.client.decline_dm_request(dm.id),
        _ => {}
    }
}

/// The bar instead of the composer while the peer's request waits
/// (`RequestBar`): what accepting means, with accept and decline.
pub fn incoming_bar(ui: &mut Ui, app: &mut AppCtx, rect: Rect, dm: &DirectMessage) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    ui.painter().rect_filled(rect, 0.0, c.bg);
    hairline_h(ui.painter(), &theme, rect.x_range(), rect.top());
    let inner = rect.shrink2(vec2(16.0, 12.0));
    let accept = Button::new(t::action_accept())
        .primary()
        .disabled(pending_op(app, &format!("dm_accept:{}", dm.id)));
    let decline = Button::new(t::action_decline())
        .ghost()
        .disabled(pending_op(app, &format!("dm_decline:{}", dm.id)));
    // Blocking from here too (#565), asked first as in the profile.
    let blocking = app.client.with_state(|s| {
        matches!(
            s.op(&format!("block:{}", dm.peer.id)),
            Some(OpState::Pending)
        )
    });
    let block = Button::new(gelabber_i18n::blocks::action_block())
        .ghost()
        .disabled(blocking);
    let (aw, dw, bw) = (
        accept.natural_width(ui),
        decline.natural_width(ui),
        block.natural_width(ui),
    );
    let buttons_w = aw + 8.0 + dw + 8.0 + bw;
    let note = Block::wrapped(
        ui,
        &t::bar_incoming(&dm.peer.name),
        text::BODY.line_height(1.5),
        c.text,
        (inner.width() - buttons_w - 16.0).max(80.0),
    );
    note.paint(
        ui.painter(),
        pos2(inner.left(), inner.center().y - note.height() / 2.0),
    );
    let a = Rect::from_min_size(
        pos2(inner.right() - buttons_w, inner.center().y - 18.0),
        vec2(aw, 36.0),
    );
    let d = Rect::from_min_size(pos2(a.right() + 8.0, a.top()), vec2(dw, 36.0));
    let b = Rect::from_min_size(pos2(d.right() + 8.0, a.top()), vec2(bw, 36.0));
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect));
    if child.put(a, accept).clicked() {
        app.client.accept_dm_request(dm.id);
    }
    if child.put(d, decline).clicked() {
        app.client.decline_dm_request(dm.id);
        app.nav.navigate("/d/requests");
    }
    if child.put(b, block).clicked() {
        overlays::ask(ui.ctx(), Question::block(dm.peer.id, &dm.peer.name));
    }
}

/// The note above the composer while the user's own request waits.
pub fn outgoing_note(dm: &DirectMessage) -> Option<String> {
    let request = dm.request.filter(|r| !r.incoming)?;
    Some(if request.remaining > 0 {
        t::bar_outgoing(&dm.peer.name, request.remaining)
    } else {
        t::bar_outgoing_full(&dm.peer.name)
    })
}

/// The incoming-request bar's height.
pub const BAR_HEIGHT: f32 = 76.0;
