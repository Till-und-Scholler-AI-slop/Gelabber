//! `/` (pages/WorkspaceIndexPage.tsx): back to the last server or DM, or
//! the first server; someone without servers gets the welcome with
//! "Server erstellen" and "Server beitreten".

use egui::{Rect, Stroke, StrokeKind, Ui, Vec2, pos2};

use super::state::{self, Place};
use crate::app::AppCtx;
use crate::text::{self, Block};
use crate::theme::{Theme, corner};
use crate::widgets::{Button, ButtonSize};

/// `startPlace`: where "/" goes; `None` while the lists still load.
pub fn redirect(ui: &Ui, app: &mut AppCtx) -> bool {
    let (servers_loaded, servers, dms_loaded, dms, user) = app.client.with_state(|s| {
        (
            s.servers_loaded,
            s.servers.iter().map(|v| v.id).collect::<Vec<_>>(),
            s.dms_loaded || matches!(s.op("bootstrap"), Some(gelabber_client::OpState::Failed(_))),
            s.dms.iter().map(|d| d.id).collect::<Vec<_>>(),
            s.me().map(|m| m.id),
        )
    });
    if !servers_loaded {
        // A failed load shows the welcome with its error.
        return !failed(ui.ctx(), app);
    }
    // One being left stays listed until its request ends: not a way back.
    let left = state::with(ui.ctx(), |s| s.left.clone());
    let servers: Vec<_> = servers.into_iter().filter(|id| !left.contains(id)).collect();
    let remembered = state::memory(ui.ctx(), user).place;
    let target = match remembered {
        Some(Place::Server(id)) if servers.contains(&id) => Some(format!("/s/{id}")),
        Some(Place::Dm(id)) if dms.contains(&id) => Some(format!("/d/{id}")),
        Some(Place::Dm(_)) if !dms_loaded => return true,
        _ => servers.first().map(|id| format!("/s/{id}")),
    };
    match target {
        Some(target) => {
            app.nav.replace(&target);
            true
        }
        None => false,
    }
}

/// The first load failed (`isError` of the server list).
fn failed(ctx: &egui::Context, app: &AppCtx) -> bool {
    let loaded = app.client.with_state(|s| s.servers_loaded);
    bootstrap_failed(ctx, app.client, loaded)
}

/// Whether the workspace's first load failed and `loaded` is still false.
/// It stays failed while "Erneut versuchen" runs, as a query keeps
/// `isError` during its refetch: the error shows with a pending button
/// instead of falling back to the skeleton.
pub fn bootstrap_failed(
    ctx: &egui::Context,
    client: &gelabber_client::Client,
    loaded: bool,
) -> bool {
    use gelabber_client::OpState;
    let id = egui::Id::unique("gelabber-bootstrap-failed");
    let op = client.with_state(|s| s.op("bootstrap").cloned());
    let failed = match op {
        _ if loaded => false,
        Some(OpState::Failed(_)) => true,
        Some(OpState::Pending) => ctx.data(|d| d.get_temp::<bool>(id)).unwrap_or(false),
        _ => false,
    };
    ctx.data_mut(|d| d.insert_temp(id, failed));
    failed
}

/// Whether "/" still waits for the lists (`MainSkeleton kind="overview"`).
pub fn loading(ctx: &egui::Context, app: &AppCtx) -> bool {
    app.client.with_state(|s| !s.servers_loaded) && !failed(ctx, app)
}

/// `.welcome__error`: the alert and "Erneut versuchen" in one row under
/// the greeting (margin-top 20, padding 10 12, the 3px danger bar).
fn welcome_error(ui: &mut Ui, app: &mut AppCtx, left: f32, top: f32, width: f32) -> f32 {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let message = "Die Server konnten nicht geladen werden.";
    let block = Block::single(ui, message, text::BODY, c.danger);
    let width = width.min(2.0 * 360.0 + 16.0);
    let rect = Rect::from_min_size(pos2(left, top), Vec2::new(width, 10.0 + 32.0 + 10.0));
    let painter = ui.painter().clone();
    painter.rect_filled(
        rect,
        corner(theme.radius.sm),
        crate::theme::over(c.bg, c.danger_bg, 0.18),
    );
    painter
        .with_clip_rect(Rect::from_min_max(
            rect.min,
            pos2(rect.left() + 3.0, rect.bottom()),
        ))
        .rect_filled(rect, corner(theme.radius.sm), c.danger);
    let text_top = rect.center().y - block.height() / 2.0;
    block.paint(&painter, pos2(rect.left() + 15.0, text_top));
    let alert = ui.interact(
        Rect::from_min_size(pos2(rect.left() + 15.0, text_top), block.size()),
        egui::Id::unique("welcome-error"),
        egui::Sense::hover(),
    );
    crate::widgets::announce(&alert, egui::Role::Alert, message);
    let pending = app
        .client
        .with_state(|s| matches!(s.op("bootstrap"), Some(gelabber_client::OpState::Pending)));
    let label = "Erneut versuchen";
    let bw = Block::single(ui, label, text::UI, c.text).width() + 26.0;
    let button = Rect::from_min_size(
        pos2(rect.left() + 15.0 + block.width() + 12.0, rect.top() + 10.0),
        Vec2::new(bw, 32.0),
    );
    if ui
        .put(
            button,
            Button::new(label)
                .small()
                .pending(pending.then_some("Wird geladen…")),
        )
        .clicked()
        && !pending
    {
        app.client.bootstrap();
    }
    rect.height() + 20.0
}

/// `.welcome`: padding 40 32 32, the greeting and two 360px cards.
pub fn welcome(ui: &mut Ui, app: &mut AppCtx, rect: Rect) {
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    ui.painter().rect_filled(rect, 0.0, c.bg);
    let name = app.client.with_state(|s| s.me().map(|m| m.name.clone()));
    let title = match name {
        Some(name) => format!("Willkommen, {name}"),
        None => "Willkommen".to_owned(),
    };
    let left = rect.left() + 32.0;
    let width = rect.width() - 64.0;
    let h1 = Block::wrapped(ui, &title, text::HERO.tracking(-0.035), c.text, width);
    h1.paint(ui.painter(), pos2(left, rect.top() + 40.0));
    let mut top = rect.top() + 40.0 + h1.height() + 32.0;
    if failed(ui.ctx(), app) {
        top += welcome_error(ui, app, left, rect.top() + 40.0 + h1.height() + 20.0, width) - 20.0
            + 12.0;
    }
    let card_w = ((width - 16.0) / 2.0).min(360.0);
    let card_h = 20.0 + 20.8 + 8.0 + 21.0 + 12.0 + 44.0 + 20.0;
    for i in 0..2 {
        let card = Rect::from_min_size(
            pos2(left + i as f32 * (card_w + 16.0), top),
            Vec2::new(card_w, card_h),
        );
        ui.painter().rect(
            card,
            corner(theme.radius.lg),
            c.panel,
            Stroke::new(1.0, c.border),
            StrokeKind::Inside,
        );
        let inner = card.shrink(21.0);
        let (h2, p) = if i == 0 {
            (
                "Eigenen Server erstellen",
                Some("Mit #allgemein und einem Sprachkanal."),
            )
        } else {
            ("Server beitreten", None)
        };
        let block = Block::single(ui, h2, text::TITLE_SM, c.text);
        block.paint(ui.painter(), inner.min);
        let mut y = inner.top() + block.height() + 8.0;
        if let Some(p) = p {
            let block = Block::wrapped(ui, p, text::BODY, c.muted, inner.width());
            block.paint(ui.painter(), pos2(inner.left(), y));
            y += block.height() + 12.0;
        }
        let action = Rect::from_min_max(pos2(inner.left(), inner.bottom() - 44.0), inner.max);
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(action));
        if i == 0 {
            if child
                .add(
                    Button::new("Server erstellen")
                        .primary()
                        .size(ButtonSize::Lg)
                        .fill(),
                )
                .clicked()
            {
                super::dialogs::open_add(ui.ctx(), state::AddStep::Create);
            }
        } else if child
            .add(Button::new("Server beitreten").size(ButtonSize::Lg).fill())
            .clicked()
        {
            super::dialogs::open_add(ui.ctx(), state::AddStep::Join);
        }
        let _ = y;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use egui::accesskit::Role;
    use egui_kittest::kittest::Queryable;

    use crate::app::testkit::{FakeApi, Reply, launcher, wait_for, wait_for_label};
    use crate::platform::paths::testing::TempConfig;

    /// The lists fail to load (500) until `fail` is cleared.
    fn failing(fail: Arc<AtomicBool>) -> FakeApi {
        FakeApi::start(true, move |method, path, _| {
            (method == "GET" && matches!(path, "/servers" | "/dms") && fail.load(Ordering::SeqCst))
                .then(|| Reply::error(500, "internal"))
        })
    }

    /// WorkspaceIndexPage: the welcome with its error and a retry.
    #[test]
    fn a_failed_server_list_says_so_on_the_welcome() {
        let _config = TempConfig::new("home-failed");
        let fail = Arc::new(AtomicBool::new(true));
        let api = failing(fail.clone());
        let mut harness = launcher(&api, "/");
        let message = "Die Server konnten nicht geladen werden.";
        wait_for_label(&mut harness, message);
        fail.store(false, Ordering::SeqCst);
        harness
            .get_by_role_and_label(Role::Button, "Erneut versuchen")
            .click();
        wait_for(&mut harness, "the list", |h| {
            h.query_by_label(message).is_none()
        });
    }

    /// DmIndexPage and the DM sidebar's placeholder: LoadError and its text.
    #[test]
    fn a_failed_dm_list_says_so_twice() {
        let _config = TempConfig::new("dms-failed");
        let fail = Arc::new(AtomicBool::new(true));
        let api = failing(fail.clone());
        let mut harness = launcher(&api, "/d");
        let page = "Die Direktnachrichten konnten nicht geladen werden.";
        wait_for_label(&mut harness, page);
        assert!(
            harness
                .query_by_label("Die Unterhaltungen konnten nicht geladen werden.")
                .is_some()
        );
        assert!(harness.query_by_label("Navigation").is_some());
        fail.store(false, Ordering::SeqCst);
        let retry = harness
            .query_all_by_label("Erneut versuchen")
            .next()
            .expect("a retry button");
        retry.click();
        wait_for(&mut harness, "the list", |h| {
            h.query_by_label(page).is_none()
        });
    }

    /// "Erneut versuchen" keeps the error up while it runs, with the pending
    /// button (a query's refetch keeps `isError`), instead of the skeleton.
    #[test]
    fn a_retry_keeps_the_error_while_it_runs() {
        let _config = TempConfig::new("dms-retry");
        // 0: the lists fail, 1: they hang, 2: they load.
        let phase = Arc::new(std::sync::atomic::AtomicU8::new(0));
        let serve = phase.clone();
        let api = FakeApi::start(true, move |method, path, _| {
            if method != "GET" || !matches!(path, "/servers" | "/dms") {
                return None;
            }
            match serve.load(Ordering::SeqCst) {
                0 => Some(Reply::error(500, "internal")),
                _ => {
                    while serve.load(Ordering::SeqCst) == 1 {
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                    None
                }
            }
        });
        let mut harness = launcher(&api, "/d");
        let page = "Die Direktnachrichten konnten nicht geladen werden.";
        wait_for_label(&mut harness, page);
        phase.store(1, Ordering::SeqCst);
        harness
            .query_all_by_label("Erneut versuchen")
            .next()
            .expect("a retry button")
            .click();
        wait_for_label(&mut harness, "Wird geladen…");
        harness.run_steps(5);
        assert!(harness.query_by_label(page).is_some());
        phase.store(2, Ordering::SeqCst);
        wait_for(&mut harness, "the list", |h| {
            h.query_by_label(page).is_none()
        });
    }

    /// Leaving a server sends the user to "/" while the request runs: "/"
    /// does not send them back into it, and its end shows no "not found".
    #[test]
    fn leaving_a_server_does_not_come_back_to_it() {
        use crate::app::testkit::{HOME, workspace};
        let _config = TempConfig::new("leave-server");
        let release = Arc::new(AtomicBool::new(false));
        let left = Arc::new(AtomicBool::new(false));
        let (hold, gone) = (release.clone(), left.clone());
        let api = FakeApi::start(true, move |method, path, _| {
            if method == "POST" && path == format!("/servers/{HOME}/leave") {
                while !hold.load(Ordering::SeqCst) {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                gone.store(true, Ordering::SeqCst);
                return Some(Reply::json(serde_json::Value::Null));
            }
            if gone.load(Ordering::SeqCst) && path.starts_with(&format!("/servers/{HOME}")) {
                return Some(Reply::error(403, "forbidden"));
            }
            workspace(method, path)
        });
        let mut harness = launcher(&api, &format!("/s/{HOME}"));
        wait_for_label(&mut harness, "allgemein");
        // What server settings does on "Server verlassen".
        let ctx = harness.ctx.clone();
        let home: uuid::Uuid = HOME.parse().expect("id");
        {
            let app = harness.state_mut().app().expect("app");
            let client = app.client().clone();
            crate::screens::shell::leave_server(&ctx, &client, home);
            client.leave_server(home);
            app.navigate("/");
        }
        let route = |h: &mut egui_kittest::Harness<'_, crate::Launcher>| {
            h.state_mut().app().expect("app").nav().route().clone()
        };
        for _ in 0..30 {
            harness.step();
            std::thread::sleep(std::time::Duration::from_millis(10));
            assert_eq!(route(&mut harness), crate::route::Route::Home);
        }
        release.store(true, Ordering::SeqCst);
        wait_for(&mut harness, "the server to go", |h| {
            h.query_by_label("allgemein").is_none() && h.query_by_label("Wohnzimmer").is_none()
        });
        for _ in 0..30 {
            harness.step();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(route(&mut harness), crate::route::Route::Home);
        assert!(
            harness
                .query_by_label_contains("Server nicht gefunden")
                .is_none()
        );
    }

    /// While the workspace loads: the sidebar's skeleton and the page's.
    #[test]
    fn loading_shows_skeletons() {
        let _config = TempConfig::new("home-loading");
        /// Lets the held request go also when the test fails.
        struct Release(Arc<AtomicBool>);
        impl Drop for Release {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let release = Release(Arc::new(AtomicBool::new(false)));
        let wait = release.0.clone();
        let api = FakeApi::start(true, move |method, path, _| {
            if method == "GET" && path == "/servers" {
                while !wait.load(Ordering::SeqCst) {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
            }
            None
        });
        let mut harness = launcher(&api, "/d");
        wait_for_label(&mut harness, "Navigation wird geladen");
        wait_for_label(&mut harness, "Wird geladen…");
        release.0.store(true, Ordering::SeqCst);
        wait_for(&mut harness, "the lists", |h| {
            h.query_by_label("Navigation wird geladen").is_none()
        });
    }
}
