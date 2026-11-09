//! The user panel at the bottom of the sidebar column
//! (components/UserPanel.tsx, index.css `.user-panel`): your avatar with
//! presence, name and status line, the settings gear and the user menu;
//! a click on the name opens the status card.

use egui::{Color32, Id, Rect, Stroke, StrokeKind, Ui, Vec2, pos2};
use gelabber_client::{OpState, PresenceStatus, ProfilePatch};
use uuid::Uuid;

use super::common::{self, line, paint_mid};
use super::state;
use crate::app::AppCtx;
use crate::icons::{self, Icon};
use crate::text::{self, Block};
use crate::theme::{Theme, corner};
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
}

fn draft_id() -> Id {
    Id::unique("gelabber-user-status")
}

/// `rect` is the 56px strip at the bottom of the sidebar column; `context`
/// the server or DM whose presence counts (`ownPresence`).
pub fn show(ui: &mut Ui, app: &mut AppCtx, rect: Rect, context: Option<Uuid>) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let painter = ui.painter().clone();
    // `.user-panel` sits inside `.channel-sidebar`: the column's right
    // border (the shell's) runs on past it down to the window's edge.
    let inner = Rect::from_min_max(rect.min, pos2(rect.right() - 1.0, rect.bottom()));
    painter.rect_filled(inner, 0.0, c.panel);
    hairline_h(&painter, &theme, inner.left()..=inner.right(), inner.top());

    let (name, avatar_url, status_text, presence) = app.client.with_state(|s| {
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
    let presence = common::presence(presence);
    let label = presence.label();
    let subtitle = match status_text.as_deref().filter(|t| !t.is_empty()) {
        Some(text) => format!("{label} · {text}"),
        None => label.to_owned(),
    };

    let inner_top = rect.top() + 1.0;
    let center_y = inner_top + (rect.bottom() - inner_top) / 2.0;
    let right = rect.right() - 1.0 - 8.0;
    let chevron = Rect::from_min_size(pos2(right - 30.0, center_y - 18.0), Vec2::new(30.0, 36.0));
    let gear = Rect::from_min_size(
        pos2(chevron.left() - 4.0 - 30.0, center_y - 18.0),
        Vec2::new(30.0, 36.0),
    );
    let profile = Rect::from_min_max(
        pos2(rect.left() + 8.0, center_y - 21.75),
        pos2(gear.left() - 4.0, center_y + 21.75),
    );

    let mut draft: StatusDraft = ui.data(|d| d.get_temp(draft_id())).unwrap_or_default();
    let response = common::click(ui, profile, Id::unique("user-panel-profile"));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &name));
    if response.hovered() || draft.open {
        common::fill(&painter, profile, theme.radius.md, c.hover);
    }
    let avatar = Rect::from_min_size(
        pos2(profile.left() + 2.0, center_y - 18.0),
        Vec2::splat(36.0),
    );
    let under = if response.hovered() || draft.open {
        c.hover
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
    if response.clicked() {
        draft.open = !draft.open;
        draft.text = status_text.clone().unwrap_or_default();
        draft.saved = None;
        draft.focus = draft.open;
    }

    let gear_response = icon_button(ui, gear, Icon::Gear, 21.0, "Einstellungen", false);
    if gear_response.clicked() {
        app.nav.navigate("/settings/darstellung");
    }
    let menu_open = state::with(ui.ctx(), |s| s.user_menu);
    let chevron_response = icon_button(ui, chevron, Icon::Chevron, 18.0, "Benutzermenü", true);
    if chevron_response.clicked() {
        state::with(ui.ctx(), |s| s.user_menu = !s.user_menu);
    }
    if menu_open {
        user_menu(ui, app, chevron);
    }

    if draft.open {
        let current = status_text.clone().unwrap_or_default();
        status_card(
            ui,
            app,
            profile,
            &name,
            avatar_url.as_deref(),
            presence,
            &current,
            &mut draft,
        );
    }
    ui.data_mut(|d| d.insert_temp(draft_id(), draft));
}

/// `.shell-icon-button` in the panel: 30×36, muted, hover fill.
fn icon_button(
    ui: &mut Ui,
    rect: Rect,
    icon: Icon,
    size: f32,
    label: &str,
    turned: bool,
) -> egui::Response {
    let theme = Theme::of(ui.ctx());
    let response = common::click(ui, rect, Id::unique(("user-panel", label)));
    let response = if turned {
        response
    } else {
        response.on_hover_text(label)
    };
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    let color = if response.hovered() {
        common::fill(ui.painter(), rect, theme.radius.md, theme.c.hover);
        theme.c.text
    } else {
        theme.c.muted
    };
    let min = rect.center() - Vec2::splat(size / 2.0);
    if turned {
        icons::paint_rotated(ui.painter(), icon, min, size, color);
    } else {
        icons::paint(ui.painter(), icon, min, size, color);
    }
    response
}

/// The account menu above the chevron: panel colour, 37px items.
fn user_menu(ui: &mut Ui, app: &mut AppCtx, chevron: Rect) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    // "Server wechseln …" is the desktop app's own (isDesktopApp).
    let items = [
        "Profil bearbeiten",
        "Einstellungen",
        "Server wechseln …",
        "Abmelden",
    ];
    let width = 180.0;
    let height = 2.0 + 16.0 + 37.0 * items.len() as f32 + 3.0 * (items.len() - 1) as f32;
    let frame = Rect::from_min_max(
        pos2(chevron.right() - width, chevron.bottom() - 43.0 - height),
        pos2(chevron.right(), chevron.bottom() - 43.0),
    );
    let mut picked = None;
    let area = egui::Area::new(Id::unique("user-panel-menu"))
        .order(egui::Order::Foreground)
        .fixed_pos(frame.min)
        .show(ui.ctx(), |ui| {
            let painter = ui.painter();
            let shadow = egui::Shadow {
                offset: [0, 8],
                blur: 25,
                spread: 0,
                color: Color32::from_black_alpha(0x33),
            };
            painter.add(shadow.as_shape(frame, corner(theme.radius.md)));
            painter.rect(
                frame,
                corner(theme.radius.md),
                c.panel,
                Stroke::new(1.0, c.border),
                StrokeKind::Inside,
            );
            let mut y = frame.top() + 9.0;
            for (i, label) in items.iter().enumerate() {
                let item =
                    Rect::from_min_size(pos2(frame.left() + 9.0, y), Vec2::new(width - 18.0, 37.0));
                let response = common::click(ui, item, Id::unique(("user-menu-item", i)));
                response
                    .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, *label));
                let response = if i == 2 {
                    response.on_hover_text("Strg+Umschalt+S")
                } else {
                    response
                };
                if response.hovered() {
                    common::fill(ui.painter(), item, theme.radius.sm, c.hover);
                }
                let block = Block::single(ui, label, text::BODY, c.text);
                block.paint(ui.painter(), pos2(item.left() + 10.0, item.top() + 8.0));
                if response.clicked() {
                    picked = Some(i);
                }
                y += 37.0 + 3.0;
            }
            ui.allocate_rect(frame, egui::Sense::hover());
        });
    let outside = ui.input(|i| {
        i.pointer.any_pressed()
            && i.pointer
                .interact_pos()
                .is_some_and(|p| !area.response.rect.contains(p) && !chevron.contains(p))
    });
    // Used up, with or without Shift (consume_key ignores an extra Shift):
    // the page under it (als gelesen markieren) does not also act.
    let escape = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    if picked.is_some() || outside || escape {
        state::with(ui.ctx(), |s| s.user_menu = false);
    }
    match picked {
        Some(0) => app.nav.navigate("/settings/profil"),
        Some(1) => app.nav.navigate("/settings/darstellung"),
        Some(2) => crate::setup::switch_server(ui.ctx()),
        Some(_) => log_out(ui.ctx(), app),
        None => {}
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
                title: "Abmelden und Anruf beenden?".into(),
                message: Some("Du verlässt dabei den Sprachkanal.".into()),
                label: "Abmelden".into(),
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

/// "Dein Status": who you are, the status line with "Speichern", and the
/// link to the profile (`StatusCard`): a 320px popover above the name.
#[allow(clippy::too_many_arguments)]
fn status_card(
    ui: &mut Ui,
    app: &mut AppCtx,
    anchor: Rect,
    name: &str,
    avatar_url: Option<&str>,
    presence: crate::widgets::Presence,
    current: &str,
    draft: &mut StatusDraft,
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
            let height = 4.0 + 44.8 + 12.0 + 17.55 + 6.0 + 37.0 + status_h + 12.0 + 36.0;
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
            let small = Block::single(ui, presence.label(), text::META, c.muted);
            // The label sits on the 16px/1.5 line of its inline parent.
            let strut = text::TextStyle::new(16.0, 400.0, 1.5);
            small.paint(
                ui.painter(),
                pos2(x, common::baseline_top(strut, top + 20.8, text::META)),
            );
            // The form.
            let mut y = top + 44.8 + 12.0;
            let label = Block::single(ui, "Status", text::UI, c.text);
            label.paint(ui.painter(), pos2(left, y));
            y += 17.55 + 6.0;
            let button_label = if saving { "Speichert…" } else { "Speichern" };
            let button_w = Block::single(ui, button_label, text::UI, c.text).width() + 26.0;
            let field_rect =
                Rect::from_min_size(pos2(left, y), Vec2::new(inner_w - button_w - 8.0, 37.0));
            let mut save = false;
            let mut field_ui = ui.new_child(egui::UiBuilder::new().max_rect(field_rect));
            let field = TextField::new("user-status-text", &mut draft.text)
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
                    Button::new("Speichern")
                        .small()
                        .width(button_w)
                        .disabled(!changed && !saving)
                        .pending(saving.then_some("Speichert…")),
                )
                .clicked();
            y += 37.0 + 6.0;
            if let Some(ok) = draft.saved {
                let (msg, tone) = if ok {
                    ("Gespeichert.", StatusTone::Success)
                } else {
                    ("Nicht gespeichert – erneut versuchen", StatusTone::Error)
                };
                let mut status_ui = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(Rect::from_min_size(pos2(left, y), Vec2::new(inner_w, 20.0))),
                );
                status_ui.add(InlineStatus::new(Some(msg), tone));
                y += 20.0;
            }
            y += 12.0;
            // "Profil bearbeiten" bleeds to the card's edges.
            let link = Rect::from_min_max(pos2(card.left(), y), pos2(card.right(), y + 36.0));
            let response = common::click(ui, link, Id::unique("user-status-profile"));
            if response.hovered() {
                let r = theme.radius.sm as u8;
                ui.painter().rect_filled(
                    link,
                    egui::CornerRadius {
                        nw: 0,
                        ne: 0,
                        sw: r,
                        se: r,
                    },
                    c.hover,
                );
            }
            hairline_h(ui.painter(), &theme, link.left()..=link.right(), link.top());
            let block = Block::single(ui, "Profil bearbeiten", text::UI, c.text);
            paint_mid(
                ui.painter(),
                &block,
                link.left() + 8.0,
                link.center().y + 0.5,
            );
            (save, response.clicked())
        });
    match out {
        None => draft.open = false,
        Some((save, profile)) => {
            if save && changed && !saving {
                draft.saving = true;
                draft.saved = None;
                app.client.update_profile(ProfilePatch {
                    status_text: Some(draft.text.trim().to_owned()),
                    ..ProfilePatch::default()
                });
            }
            if profile {
                draft.open = false;
                app.nav.navigate("/settings/profil");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use egui::accesskit::Role;
    use egui_kittest::kittest::{NodeT, Queryable};

    use crate::app::testkit::{FakeApi, Reply, launcher, wait_for, wait_for_label};
    use crate::platform::paths::testing::TempConfig;

    fn open_menu(harness: &mut egui_kittest::Harness<'_, crate::Launcher>) {
        wait_for_label(harness, "Benutzermenü");
        harness
            .get_by_role_and_label(Role::Button, "Benutzermenü")
            .click();
        wait_for_label(harness, "Abmelden");
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
        harness
            .get_by_role_and_label(Role::Button, "Abmelden")
            .click();
        wait_for_label(&mut harness, "Abmelden und Anruf beenden?");
        assert!(
            harness
                .query_by_label("Du verlässt dabei den Sprachkanal.")
                .is_some()
        );
        harness
            .get_by_role_and_label(Role::Button, "Abbrechen")
            .click();
        harness.run_steps(10);
        assert!(!api.saw("POST /auth/logout"));
        assert!(
            harness
                .query_by_label("Abmelden und Anruf beenden?")
                .is_none()
        );
        // Confirmed: signed out, on the login page.
        open_menu(&mut harness);
        harness
            .get_by_role_and_label(Role::Button, "Abmelden")
            .click();
        wait_for_label(&mut harness, "Abmelden und Anruf beenden?");
        let confirm = harness
            .query_all_by_label("Abmelden")
            .rfind(|n| n.accesskit_node().role() == Role::Button)
            .expect("the dialog's button");
        confirm.click();
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
        wait_for_label(&mut harness, "Benutzermenü");
        harness.get_by_role_and_label(Role::Button, "Lena").click();
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
        harness
            .get_by_role_and_label(Role::Button, "Abmelden")
            .click();
        let text =
            "Die Abmeldung auf dem Server konnte nicht bestätigt werden. Bitte versuche es erneut.";
        wait_for_label(&mut harness, text);
        assert!(
            harness
                .query_by_label("Abmelden und Anruf beenden?")
                .is_none()
        );
        fail.store(false, std::sync::atomic::Ordering::SeqCst);
        harness
            .get_by_role_and_label(Role::Button, "Erneut abmelden")
            .click();
        wait_for(&mut harness, "the strip to go", |h| {
            h.query_by_label(text).is_none()
        });
    }

    /// Esc closes the open menu and is used up there: the conversation
    /// under it is not marked read too, nor with Shift the whole server.
    #[test]
    fn esc_closes_the_menu_and_marks_nothing_read() {
        use crate::app::testkit::{ALLGEMEIN, HOME, workspace};
        use std::time::{Duration, Instant};

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
        // Settled: requests go to another thread.
        let settle = |harness: &mut egui_kittest::Harness<'_, crate::Launcher>| {
            for _ in 0..20 {
                harness.step();
                std::thread::sleep(Duration::from_millis(10));
            }
        };
        // With nothing open, Esc marks #allgemein read (once it is shown).
        let start = Instant::now();
        while reads() == 0 {
            assert!(
                start.elapsed() < Duration::from_secs(15),
                "no Esc mark-read"
            );
            harness.key_press(egui::Key::Escape);
            settle(&mut harness);
        }
        settle(&mut harness);
        let before = reads();
        for shift in [false, true] {
            open_menu(&mut harness);
            if shift {
                harness.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::Escape);
            } else {
                harness.key_press(egui::Key::Escape);
            }
            settle(&mut harness);
            assert!(
                harness.query_by_label("Abmelden").is_none(),
                "closed (shift: {shift})"
            );
            assert_eq!(reads(), before, "nothing marked read (shift: {shift})");
        }
    }
}
