//! The workspace's overlays beside the server dialogs: the quick switcher
//! (Strg+K) and the shortcut sheet (Strg+/), the profile dialogs with
//! moderation, the notification settings, the confirm host and the
//! new-message toasts (web QuickSwitcher.tsx, MemberProfileDialog.tsx,
//! UserProfileDialog.tsx, MemberActions.tsx, NotificationSettingsDialog.tsx,
//! ConfirmHost.tsx, MessageToasts.tsx). One overlay is open at a time; a
//! question of the confirm host stacks on top of it.

pub mod confirm;
pub mod message_toasts;
pub mod notify;
pub mod picker;
pub mod profile;
pub mod switcher;

use std::sync::Arc;

use egui::{Key, Modifiers, Ui};
use parking_lot::Mutex;
use uuid::Uuid;

use crate::app::AppCtx;
use crate::route::Route;
use crate::screens::chat::{self, ChatRequest};

pub use confirm::{ConfirmAction, Question};
pub use notify::Target;

pub enum Overlay {
    Switcher(switcher::Switcher),
    Shortcuts,
    Member(profile::MemberDialog),
    User(profile::UserDialog),
    Notify(notify::NotifyDialog),
}

#[derive(Default)]
struct OverlayState {
    /// The account the open overlays and held toasts belong to.
    account: Option<Uuid>,
    overlay: Option<Overlay>,
    question: Option<Question>,
    toasts: message_toasts::MessageToasts,
    /// The route the open dialogs belong to.
    route: Option<Route>,
    /// The frame the last overlay or dialog opened in.
    opened: u64,
}

/// Notes that a dialog or overlay opens now: one opened in the frame a
/// route changed belongs to the new route (an invite prompt).
pub(super) fn opened(ctx: &egui::Context) {
    shared(ctx).lock().opened = ctx.cumulative_frame_nr();
}

/// After a route change (back, a link) a dialog closes when the part of
/// the page that owns it on the web goes: the member panel's profile off
/// the server, the sidebar's channel and invite dialogs with their server,
/// the DM page's profile and "Neue Nachricht" outside the DMs. The global
/// hosts (switcher, notifications, confirm) stay, as on the web.
pub(super) fn follow_route(ctx: &egui::Context, route: &Route) {
    let state = shared(ctx);
    let mut s = state.lock();
    if s.route.as_ref() == Some(route) {
        return;
    }
    let first = s.route.is_none();
    s.route = Some(route.clone());
    if first || s.opened == ctx.cumulative_frame_nr() {
        return;
    }
    let server = match route {
        Route::Server(s) | Route::Channel(s, _) => Some(*s),
        _ => None,
    };
    let dms = matches!(
        route,
        Route::Dms | Route::Friends | Route::DmRequests | Route::Dm(_)
    );
    s.overlay = match s.overlay.take() {
        Some(Overlay::Member(m)) if Some(m.server()) != server => None,
        Some(Overlay::User(_)) if !matches!(route, Route::Dm(_)) => None,
        other => other,
    };
    drop(s);
    super::state::with(ctx, |s| {
        let keep = match &s.dialog {
            Some(super::state::Dialog::Channel { server: id, .. })
            | Some(super::state::Dialog::Category { server: id, .. }) => server == Some(*id),
            Some(super::state::Dialog::Invite(invite)) => server == Some(invite.server),
            Some(super::state::Dialog::NewMessage { .. }) => dms,
            Some(super::state::Dialog::Add { .. }) | None => true,
        };
        if !keep {
            s.dialog = None;
        }
    });
}

/// Another account signed in: nothing of the one before stays open or
/// held (a question, a profile, the switcher, its toasts).
pub(super) fn switch_account(ctx: &egui::Context, user: Uuid) {
    let state = shared(ctx);
    let mut s = state.lock();
    if s.account != Some(user) {
        *s = OverlayState {
            account: Some(user),
            ..OverlayState::default()
        };
    }
}

fn shared(ctx: &egui::Context) -> Arc<Mutex<OverlayState>> {
    ctx.data_mut(|d| {
        d.get_temp_mut_or_insert_with(egui::Id::unique("gelabber-overlays"), || {
            Arc::new(Mutex::new(OverlayState::default()))
        })
        .clone()
    })
}

/// A left-to-right row of lines exactly `height` tall (`ui.horizontal`
/// would be at least the 36px interact size). What does not fit wraps
/// onto a new line `gap` below, as `flex-wrap: wrap; gap` does on the web,
/// instead of painting past the dialog.
pub fn row<R>(ui: &mut Ui, height: f32, gap: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), height),
        egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(true),
        |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(gap, gap);
            add(ui)
        },
    )
    .inner
}

/// Opens `overlay` in place of the one open now.
pub fn open(ctx: &egui::Context, overlay: Overlay) {
    let state = shared(ctx);
    let mut s = state.lock();
    s.overlay = Some(overlay);
    s.opened = ctx.cumulative_frame_nr();
    ctx.request_repaint();
}

/// `MemberProfileDialog` for `member` of `server`.
pub fn open_member(ctx: &egui::Context, server: Uuid, member: Uuid) {
    open(
        ctx,
        Overlay::Member(profile::MemberDialog::new(server, member)),
    );
}

/// `UserProfileDialog` for a DM peer.
pub fn open_user(ctx: &egui::Context, user: Uuid) {
    open(ctx, Overlay::User(profile::UserDialog::new(user)));
}

/// `openNotificationSettings`.
pub fn open_notifications(ctx: &egui::Context, target: Target) {
    open(ctx, Overlay::Notify(notify::NotifyDialog::new(target)));
}

/// `confirmAction`: a new question replaces an open one.
pub fn ask(ctx: &egui::Context, question: Question) {
    let state = shared(ctx);
    let mut s = state.lock();
    s.question = Some(question);
    s.opened = ctx.cumulative_frame_nr();
    ctx.request_repaint();
}

/// Whether an overlay or a question is open (the shell pauses its
/// shortcuts meanwhile).
pub fn is_open(ctx: &egui::Context) -> bool {
    let s = shared(ctx);
    let s = s.lock();
    s.overlay.is_some() || s.question.is_some()
}

/// Strg/⌘+K and Strg/⌘+/ (`shellShortcut`): taken before the pages see
/// them, so a focused composer does not swallow them.
pub fn keys(ui: &Ui, other_dialog: bool) {
    let ctx = ui.ctx();
    let (mut switch, mut help) = (false, false);
    ctx.input_mut(|i| {
        i.events.retain(|event| {
            let egui::Event::Key {
                key,
                modifiers,
                pressed: true,
                ..
            } = event
            else {
                return true;
            };
            if !modifiers.command || modifiers.alt {
                return true;
            }
            match key {
                Key::K if !modifiers.shift => switch = true,
                // "/" needs Umschalt on German keyboards.
                Key::Slash => help = true,
                _ => return true,
            }
            false
        });
    });
    if other_dialog || (!switch && !help) {
        return;
    }
    let state = shared(ctx);
    let mut s = state.lock();
    if s.question.is_some() {
        return;
    }
    // A keyboard opening shows the focus ring, as :focus-visible does.
    crate::widgets::set_keyboard_focus(ctx, true);
    s.opened = ctx.cumulative_frame_nr();
    s.overlay = Some(if switch {
        Overlay::Switcher(switcher::Switcher::default())
    } else {
        Overlay::Shortcuts
    });
}

/// Draws the open overlay, the question over it and the new-message
/// toasts; takes the chat's requests and the client's notices first.
pub fn show(ui: &mut Ui, app: &mut AppCtx) {
    let ctx = ui.ctx().clone();
    follow_route(&ctx, app.nav.route());
    for request in chat::take_requests(&ctx) {
        match request {
            ChatRequest::Notifications {
                channel,
                server,
                name,
            } => open_notifications(
                &ctx,
                match server {
                    Some(server) => Target::channel(channel, server, name),
                    None => Target::dm(channel, name),
                },
            ),
            ChatRequest::Profile { user } => open_user(&ctx, user),
        }
    }
    let state = shared(&ctx);
    message_toasts::collect(&ctx, app, &mut state.lock().toasts);

    let (mut overlay, mut question) = {
        let mut s = state.lock();
        (s.overlay.take(), s.question.take())
    };
    // The question on top gets Esc and Enter; the overlay under it none.
    let escape =
        question.is_some() && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
    let keep = match &mut overlay {
        Some(Overlay::Switcher(s)) => switcher::show(ui, app, s),
        Some(Overlay::Shortcuts) => switcher::shortcut_sheet(ui),
        Some(Overlay::Member(m)) => profile::member_dialog(ui, app, m),
        Some(Overlay::User(u)) => profile::user_dialog(ui, app, u),
        Some(Overlay::Notify(n)) => notify::show(ui, app, n),
        None => false,
    };
    let answered = match &mut question {
        Some(q) => confirm::show(ui, app, q, escape),
        None => false,
    };
    let mut s = state.lock();
    if keep && s.overlay.is_none() {
        s.overlay = overlay;
    }
    if !answered && s.question.is_none() {
        s.question = question;
    }
    let mut toasts = std::mem::take(&mut s.toasts);
    drop(s);
    message_toasts::show(ui, app, &mut toasts);
    state.lock().toasts = toasts;
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use egui::{Rect, Sense, UiBuilder, vec2};

    /// `flex-wrap: wrap; gap: 12px` (`.member-profile__actions`): what
    /// does not fit wraps onto a new row 12 below, each keeps its height,
    /// and what follows the row starts under its last line.
    #[test]
    fn a_row_wraps_what_does_not_fit() {
        type Laid = (Rect, Vec<Rect>, Rect);
        let laid: Rc<RefCell<Option<Laid>>> = Rc::default();
        let out = laid.clone();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(vec2(400.0, 400.0))
            .build_ui(move |ui| {
                let bounds = Rect::from_min_size(ui.max_rect().min, vec2(300.0, 300.0));
                ui.scope_builder(UiBuilder::new().max_rect(bounds), |ui| {
                    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                    let mut items = Vec::new();
                    // Four 120 wide buttons: two fit in 300 with the gap.
                    super::row(ui, 40.0, 12.0, |ui| {
                        for _ in 0..4 {
                            items.push(ui.allocate_exact_size(vec2(120.0, 40.0), Sense::click()).0);
                        }
                    });
                    let below = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover()).0;
                    *out.borrow_mut() = Some((bounds, items, below));
                });
            });
        harness.run();
        let (bounds, items, below) = laid.borrow().clone().expect("drawn");
        for item in &items {
            assert!(item.right() <= bounds.right(), "{item:?} past {bounds:?}");
            assert_eq!(item.height(), 40.0);
        }
        assert_eq!(items[0].top(), items[1].top());
        assert_eq!(items[1].left(), items[0].right() + 12.0);
        assert_eq!(items[2].left(), bounds.left());
        assert_eq!(items[2].top(), items[1].bottom() + 12.0);
        assert_eq!(items[3].top(), items[2].top());
        assert!(below.top() >= items[3].bottom(), "{below:?}");
    }
}
