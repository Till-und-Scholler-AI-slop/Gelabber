//! User settings (web pages/UserSettingsPage.tsx, settingsAreas.ts,
//! SettingsAreaContent.tsx, settings.css): the area navigation in the
//! sidebar column and one area per page.

pub mod appearance;
pub mod card;
pub mod controls;
pub mod desktop_app;
pub mod media;
pub mod notifications;
pub mod prefs;
pub mod profile;
#[cfg(test)]
mod tests_drawn;
pub mod themes;

use std::sync::Arc;

use egui::{Color32, Id, Rect, Sense, Ui, UiBuilder, pos2, vec2};
use gelabber_client::{OpState, User};
use gelabber_i18n::{Text, settings as texts};
use parking_lot::Mutex;

use crate::app::AppCtx;
use crate::text::{self, Block, TextStyle};
use crate::theme::{Theme, corner};
use crate::widgets::{eased, focus_ring_inset, keyboard_focus};
use controls::Scope;

pub use themes::apply_theme;

/// `SETTINGS_AREAS`: id, group, label, description, scope tag.
pub struct Area {
    pub id: &'static str,
    pub group: &'static str,
    pub label: Text,
    pub description: Text,
    pub scope: Option<Scope>,
}

pub const AREAS: [Area; 9] = [
    Area {
        id: "profil",
        group: "konto",
        label: texts::area_profil_label,
        description: texts::area_profil_description,
        scope: Some(Scope::Account),
    },
    Area {
        id: "profilkarte",
        group: "konto",
        label: texts::area_profilkarte_label,
        description: texts::area_profilkarte_description,
        scope: Some(Scope::Account),
    },
    Area {
        id: "konto",
        group: "konto",
        label: texts::area_konto_label,
        description: texts::area_konto_description,
        scope: Some(Scope::Account),
    },
    Area {
        id: "darstellung",
        group: "app",
        label: texts::area_darstellung_label,
        description: texts::area_darstellung_description,
        scope: None,
    },
    Area {
        id: "themes",
        group: "app",
        label: texts::area_themes_label,
        description: texts::area_themes_description,
        scope: Some(Scope::Account),
    },
    Area {
        id: "audio",
        group: "app",
        label: texts::area_audio_label,
        description: texts::area_audio_description,
        scope: Some(Scope::Device),
    },
    Area {
        id: "video",
        group: "app",
        label: texts::area_video_label,
        description: texts::area_video_description,
        scope: Some(Scope::Device),
    },
    Area {
        id: "benachrichtigungen",
        group: "app",
        label: texts::area_benachrichtigungen_label,
        description: texts::area_benachrichtigungen_description,
        scope: None,
    },
    Area {
        id: "app",
        group: "app",
        label: texts::area_app_label,
        // The web's "Gelabber als App installieren." is for the browser.
        description: texts::area_app_description_desktop,
        scope: None,
    },
];

const GROUPS: [(&str, Text); 2] = [("konto", texts::group_konto), ("app", texts::group_app)];

/// Where `/settings` and the gear lead.
pub const DEFAULT_AREA: &str = "darstellung";

fn area_of(id: Option<&str>) -> &'static Area {
    let id = id.unwrap_or(DEFAULT_AREA);
    AREAS
        .iter()
        .find(|a| a.id == id)
        .or_else(|| AREAS.iter().find(|a| a.id == DEFAULT_AREA))
        .expect("default area exists")
}

/// The open area's forms; a new area or account starts them afresh, like
/// the web's remount per route.
#[derive(Default)]
pub(crate) struct PageState {
    area: &'static str,
    user: Option<uuid::Uuid>,
    pub profile: profile::ProfileState,
    pub security: profile::SecurityState,
    pub card: card::CardState,
    pub appearance: appearance::State,
    pub themes: themes::PanelState,
    pub media: media::State,
    pub notifications: notifications::State,
    pub desktop_app: desktop_app::State,
    /// A navigation that waits for a "discard?" answer.
    pub leave_to: Option<String>,
    /// A new area moves focus to its heading (UserSettingsPage).
    focus_heading: bool,
}

fn page_state(ctx: &egui::Context) -> Arc<Mutex<PageState>> {
    let id = Id::unique("gelabber-settings-page");
    ctx.data_mut(|d| {
        d.get_temp_mut_or_insert_with(id, || Arc::new(Mutex::new(PageState::default())))
            .clone()
    })
}

/// The signed-in account: the client's copy, else the app session's.
pub(crate) fn me(app: &AppCtx) -> Option<User> {
    app.client.with_state(|s| s.me().cloned()).or_else(|| {
        app.session
            .user()
            .and_then(|v| serde_json::from_value::<User>(v.clone()).ok())
    })
}

/// Keeps the app session in step after the client replaced the user.
pub(crate) fn adopt_user(app: &mut AppCtx, user: &User) {
    if let Ok(value) = serde_json::to_value(user) {
        app.session.adopt(&serde_json::json!({ "user": value }));
    }
}

/// The workspace data the pages read (themes, privacy, servers); loaded
/// once if the shell has not done it yet.
pub(crate) fn ensure_workspace(app: &AppCtx, ctx: &egui::Context) {
    let id = Id::unique("gelabber-settings-bootstrap");
    let asked = ctx.data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
    let needed = app
        .client
        .with_state(|s| !s.bootstrapped && s.op("bootstrap").is_none());
    if needed && !asked {
        ctx.data_mut(|d| d.insert_temp(id, true));
        app.client.bootstrap();
    }
}

/// The result of the client operation `key` once it settled, then forgets it.
pub(crate) fn settled(app: &AppCtx, key: &str) -> Option<Result<(), gelabber_client::ApiError>> {
    let state = app.client.with_state(|s| s.op(key).cloned());
    match state {
        Some(OpState::Done) => {
            app.client.clear_op(key);
            Some(Ok(()))
        }
        Some(OpState::Failed(e)) => {
            app.client.clear_op(key);
            Some(Err(e))
        }
        _ => None,
    }
}

pub(crate) fn op_pending(app: &AppCtx, key: &str) -> bool {
    app.client
        .with_state(|s| matches!(s.op(key), Some(OpState::Pending)))
}

/// What the settings change for the whole window: the account's theme
/// (or a gallery preview), text size and reduced motion. Cheap; the shell
/// calls it every frame so every page follows.
/// Drops what the window knows about the previous account's themes (a
/// new app after "Server wechseln").
pub fn forget_account(ctx: &egui::Context) {
    themes::forget(ctx);
}

pub fn apply_preferences(ctx: &egui::Context, client: &gelabber_client::Client) {
    themes::apply_theme(ctx, client);
    prefs::apply(ctx, &prefs::get(ctx));
}

// -------------------------------------------------------------- sidebar

/// `.sidebar-brand`: 700 16px, letter-spacing −0.3px.
const BRAND: TextStyle = TextStyle::new(16.0, 700.0, 1.5).tracking(-0.3 / 16.0);

/// The settings navigation in the sidebar column (`ui.max_rect()`): the
/// brand line and the Konto/App groups, above the shell's user panel.
pub fn sidebar(ui: &mut Ui, app: &mut AppCtx, area: Option<&str>) {
    let theme = Theme::of(ui.ctx());
    let c = &theme.c;
    let column = ui.max_rect();
    let current = area_of(area).id;
    // The column's panel fill (the shell's border at its right edge stays).
    let fill = Rect::from_min_max(
        column.min,
        pos2(column.right() - 1.0, column.bottom() - 56.0),
    );
    ui.painter().rect_filled(fill, 0.0, c.panel);
    let painter = ui.painter();
    crate::widgets::rule_h(
        painter,
        c.border,
        column.left()..=column.right() - 1.0,
        column.top() + 51.0,
    );
    controls::paint(
        ui,
        pos2(column.left() + 16.0, column.top() + 13.5),
        texts::title(),
        BRAND,
        c.text,
    );
    let mut y = column.top() + 52.0 + 4.0;
    let x = column.left() + 8.0;
    let width = column.width() - 1.0 - 16.0;
    for (gi, (group, label)) in GROUPS.iter().enumerate() {
        if gi > 0 {
            y += 8.0;
        }
        let caps = label().to_uppercase();
        controls::paint(
            ui,
            pos2(x + 10.0, y + 34.0 - 7.0 - text::CAPS.line_px()),
            &caps,
            text::CAPS,
            c.muted,
        );
        y += 34.0;
        // "App" holds the desktop app's updates and window: listed whenever
        // it has them (`visibleSettingsAreas` hid it in the Tauri app).
        let app_area = current == "app" || desktop_app::has_content(ui.ctx());
        let shown = |a: &&Area| a.group == *group && (a.id != "app" || app_area);
        for a in AREAS.iter().filter(shown) {
            let rect = Rect::from_min_size(pos2(x, y), vec2(width, 34.0));
            let id = Id::unique(("gelabber-settings-nav", a.id));
            let response = ui.interact(rect, id, Sense::click());
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Link, true, (a.label)()));
            let on = a.id == current;
            let hover = eased(
                ui.ctx(),
                id.with("hover"),
                response.hovered(),
                Color32::TRANSPARENT,
                c.hover,
            );
            let fill = if on { c.selected } else { hover };
            let painter = ui.painter();
            painter.rect_filled(rect, corner(theme.radius.md), fill);
            if on {
                let bar = Rect::from_min_max(
                    pos2(rect.left(), rect.top() + 7.0),
                    pos2(rect.left() + 3.0, rect.bottom() - 7.0),
                );
                painter.rect_filled(
                    bar,
                    egui::CornerRadius {
                        nw: 0,
                        sw: 0,
                        ne: 3,
                        se: 3,
                    },
                    c.accent,
                );
            }
            if response.has_focus() && keyboard_focus(ui.ctx()) {
                focus_ring_inset(painter, rect, theme.radius.md, &theme);
            }
            let style = if on {
                text::BODY.weight(600.0)
            } else {
                text::BODY
            };
            let color = if on || response.hovered() {
                c.text
            } else {
                c.muted
            };
            Block::single(ui, (a.label)(), style, color)
                .paint(painter, pos2(rect.left() + 10.0, rect.top() + 6.5));
            if response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if response.clicked() && !on {
                go(ui.ctx(), app, &format!("/settings/{}", a.id));
            }
            y += 34.0;
        }
    }
}

/// Navigates, asking first while the Profilkarte has unsaved changes
/// (`useBlocker` in ProfileCardForm.tsx).
pub(crate) fn go(ctx: &egui::Context, app: &mut AppCtx, href: &str) {
    let shared = page_state(ctx);
    let mut s = shared.lock();
    leave(ctx, app, &mut s, href);
}

/// [`go`] for an area drawn under the page's lock, which it already holds.
fn leave(ctx: &egui::Context, app: &mut AppCtx, s: &mut PageState, href: &str) {
    if s.area == "profilkarte" && s.card.dirty {
        s.leave_to = Some(href.to_owned());
        controls::ask(
            ctx,
            controls::Confirm::danger(
                "card-leave",
                texts::card_discard_title(),
                Some(texts::card_discard_message()),
                texts::card_discard(),
            ),
        );
    } else if s.area == "themes" && s.themes.draft_dirty() {
        s.leave_to = Some(href.to_owned());
        controls::ask(
            ctx,
            controls::Confirm::danger(
                "theme-leave",
                gelabber_i18n::themes::draft_discard_title(),
                Some(gelabber_i18n::themes::draft_discard_message()),
                gelabber_i18n::themes::draft_discard(),
            ),
        );
    } else {
        app.nav.navigate(href);
    }
}

// ----------------------------------------------------------------- page

/// The settings page: header and the area, drawn into `ui.max_rect()`.
pub fn page(ui: &mut Ui, app: &mut AppCtx, area: Option<&str>) {
    let ctx = ui.ctx().clone();
    ensure_workspace(app, &ctx);
    apply_preferences(&ctx, app.client);
    let info = area_of(area);
    // `/settings` goes on to the default area (`<Navigate replace>`).
    if area.is_none_or(|a| a != info.id) {
        app.nav.replace(&format!("/settings/{}", info.id));
    }
    let rect = ui.max_rect();
    // The sidebar's title already names the page (UserSettingsPage).
    controls::header(ui, rect, "", None);
    let Some(user) = me(app) else { return };

    let shared = page_state(&ctx);
    let mut s = shared.lock();
    if s.area != info.id || s.user != Some(user.id) {
        let fresh = PageState {
            area: info.id,
            user: Some(user.id),
            focus_heading: true,
            ..PageState::default()
        };
        *s = fresh;
    }
    for key in ["card-leave", "theme-leave"] {
        if let Some(yes) = controls::answer(&ctx, key)
            && let Some(href) = s.leave_to.take().filter(|_| yes)
        {
            s.card.dirty = false;
            s.themes.drop_draft();
            app.nav.navigate(&href);
            return;
        }
    }

    let body = Rect::from_min_max(pos2(rect.left(), rect.top() + 52.0), rect.max);
    let st = &mut *s;
    controls::scroll(ui, body, info.id, None, |ui| {
        let viewport = ui.clip_rect();
        let width = ui.available_width().min(1040.0);
        let left = ui.cursor().min.x;
        ui.add_space(32.0);
        // Rows and forms keep one measure; Profilkarte and Themes give the
        // rest to their previews (`.settings-area > *`).
        let column = match info.id {
            "profilkarte" | "themes" => width - 80.0,
            _ => (width - 80.0).min(controls::MEASURE),
        };
        let inner = Rect::from_min_size(
            pos2(left + 40.0, ui.cursor().min.y),
            vec2(column, f32::INFINITY),
        );
        let mut child = ui.new_child(
            UiBuilder::new()
                .max_rect(inner)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        child.spacing_mut().item_spacing = egui::Vec2::ZERO;
        // The heading takes the focus, so screen readers say where they are
        // and the next Tab is the area's first control.
        let take = std::mem::take(&mut st.focus_heading);
        let heading = Id::unique("gelabber-settings-heading");
        crate::app::focus_anchor(&child, heading, child.cursor().min, take);
        let description = match info.id {
            "app" => desktop_app::description(child.ctx()),
            _ => (info.description)(),
        };
        controls::area_heading(&mut child, (info.label)(), info.scope, description);
        match info.id {
            "profil" => profile::profile(&mut child, app, &user, &mut st.profile),
            "profilkarte" => card::area(&mut child, app, &user, &mut st.card, viewport),
            "konto" => profile::security(&mut child, app, &user, &mut st.security),
            "darstellung" => appearance::area(&mut child, app, &mut st.appearance),
            "themes" => themes::area(&mut child, app, &mut st.themes, viewport),
            "audio" => media::audio(&mut child, app, &mut st.media),
            "video" => media::video(&mut child, app, &mut st.media),
            "benachrichtigungen" => notifications::area(&mut child, app, &mut st.notifications),
            "app" => desktop_app::area(&mut child, &mut st.desktop_app),
            _ => {}
        }
        let used = child.min_rect();
        ui.advance_cursor_after_rect(Rect::from_min_max(
            pos2(left, inner.top()),
            pos2(left + width, used.bottom() + 48.0),
        ));
    });
    // An area's link: the page's state is locked here, so not through `go`.
    if let Some(href) = s.appearance.navigate.take() {
        leave(&ctx, app, &mut s, href);
    }
    drop(s);
    controls::show_confirm(&ctx);
}
