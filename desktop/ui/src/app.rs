//! The app frame: the session bootstrap (GET /api/auth/session), route
//! guards like web/src/routes.tsx (`requireAnonymous`, `requireUser`),
//! navigation with a back stack, the page background, toasts, and dispatch
//! to the screens.

use std::time::Duration;

use egui::Ui;
use gelabber_client::{ApiError, Client};
use serde_json::Value;

use crate::images::Images;
use crate::route::Route;
use crate::screens::{self, ScreenState};
use crate::task::{Pending, poll};
use crate::theme::{self, Theme};
use crate::toasts::Toasts;

/// Where the app is: a route plus the query and fragment of its href.
#[derive(Clone, Debug, PartialEq)]
pub struct Location {
    pub route: Route,
    pub path: String,
    pub query: Vec<(String, String)>,
    pub fragment: String,
}

impl Location {
    pub fn parse(href: &str) -> Self {
        let (rest, fragment) = href.split_once('#').unwrap_or((href, ""));
        let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
        let path = if path.is_empty() { "/" } else { path };
        let query = query
            .split('&')
            .filter(|p| !p.is_empty())
            .map(|p| {
                let (k, v) = p.split_once('=').unwrap_or((p, ""));
                (decode(k), decode(v))
            })
            .collect();
        Self {
            route: Route::parse(path),
            path: path.to_owned(),
            query,
            fragment: fragment.to_owned(),
        }
    }

    /// Path and query, as the web's `location.href` relative to the origin.
    pub fn href(&self) -> String {
        let mut href = self.path.clone();
        if !self.query.is_empty() {
            href.push('?');
            let parts: Vec<String> = self
                .query
                .iter()
                .map(|(k, v)| format!("{}={}", encode(k), encode(v)))
                .collect();
            href.push_str(&parts.join("&"));
        }
        href
    }

    pub fn query(&self, key: &str) -> Option<&str> {
        self.query
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// `?redirect=`, only when it stays inside the app (`safeRedirect`).
    pub fn redirect(&self) -> Option<String> {
        self.query("redirect").and_then(safe_redirect)
    }
}

/// `safeRedirect`: same-app paths only.
pub fn safe_redirect(value: &str) -> Option<String> {
    (value.starts_with('/') && !value.starts_with("//") && !value.starts_with("/\\"))
        .then(|| value.to_owned())
}

/// `isInviteRedirect`.
pub fn is_invite_redirect(target: Option<&str>) -> bool {
    target.is_some_and(|t| {
        t.strip_prefix("/invite/")
            .and_then(|code| code.chars().next())
            .is_some_and(|c| !matches!(c, '/' | '?' | '#'))
    })
}

/// `path?key=value` with the value encoded.
pub fn with_query(path: &str, query: &[(&str, &str)]) -> String {
    let parts: Vec<String> = query
        .iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, v)| format!("{k}={}", encode(v)))
        .collect();
    if parts.is_empty() {
        path.to_owned()
    } else {
        format!("{path}?{}", parts.join("&"))
    }
}

fn encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            // Hex digits from the bytes: slicing the &str could split a
            // multi-byte character and panic on hrefs like `%aé`.
            b'%' if i + 2 < bytes.len() => {
                match (hex_digit(bytes[i + 1]), hex_digit(bytes[i + 2])) {
                    (Some(hi), Some(lo)) => {
                        out.push(hi << 4 | lo);
                        i += 3;
                        continue;
                    }
                    _ => out.push(b'%'),
                }
            }
            b'+' => out.push(b' '),
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_digit(b: u8) -> Option<u8> {
    char::from(b).to_digit(16).map(|d| d as u8)
}

/// The navigation state: the current location and a back stack.
#[derive(Debug)]
pub struct Nav {
    current: Location,
    back: Vec<Location>,
    /// Entries left by going back, for Alt+→ and the mouse's forward button.
    forward: Vec<Location>,
    changed: bool,
}

impl Nav {
    pub fn new(href: &str) -> Self {
        Self {
            current: Location::parse(href),
            back: Vec::new(),
            forward: Vec::new(),
            changed: true,
        }
    }

    pub fn location(&self) -> &Location {
        &self.current
    }

    pub fn route(&self) -> &Route {
        &self.current.route
    }

    /// Goes to `href` (`/s/…`, `/login?redirect=…`), keeping a way back.
    pub fn navigate(&mut self, href: &str) {
        let next = Location::parse(href);
        if next != self.current {
            self.back.push(std::mem::replace(&mut self.current, next));
            self.forward.clear();
            self.changed = true;
        }
    }

    pub fn navigate_route(&mut self, route: &Route) {
        self.navigate(&route.path());
    }

    /// Goes to `href` in place of the current entry (`replace: true`).
    pub fn replace(&mut self, href: &str) {
        let next = Location::parse(href);
        if next != self.current {
            self.current = next;
            self.changed = true;
        }
    }

    /// Back one entry; false when there is none.
    pub fn back(&mut self) -> bool {
        match self.back.pop() {
            Some(previous) => {
                self.forward
                    .push(std::mem::replace(&mut self.current, previous));
                self.changed = true;
                true
            }
            None => false,
        }
    }

    /// Forward one entry after [`Self::back`]; false when there is none.
    pub fn forward(&mut self) -> bool {
        match self.forward.pop() {
            Some(next) => {
                self.back.push(std::mem::replace(&mut self.current, next));
                self.changed = true;
                true
            }
            None => false,
        }
    }

    pub fn can_go_back(&self) -> bool {
        !self.back.is_empty()
    }

    /// True once after every change; screens reset their state on it.
    fn take_changed(&mut self) -> bool {
        std::mem::take(&mut self.changed)
    }
}

/// `useSession` status.
#[derive(Clone, Debug, PartialEq)]
pub enum SessionStatus {
    /// Not asked yet, or the check failed (see [`Session::failed`]).
    Unknown,
    Anonymous,
    /// The signed-in user (`User` of GET /api/auth/session).
    Authenticated(Value),
}

/// `SESSION_RETRY_MS` (session.ts).
const RETRY: [f64; 4] = [2.0, 5.0, 15.0, 30.0];

pub struct Session {
    pub status: SessionStatus,
    check: Option<Pending<Result<Value, ApiError>>>,
    failures: usize,
    retry_at: Option<f64>,
}

impl Session {
    pub(crate) fn new() -> Self {
        Self {
            status: SessionStatus::Unknown,
            check: None,
            failures: 0,
            retry_at: None,
        }
    }

    pub fn user(&self) -> Option<&Value> {
        match &self.status {
            SessionStatus::Authenticated(user) => Some(user),
            _ => None,
        }
    }

    pub fn checking(&self) -> bool {
        self.check.is_some()
    }

    /// The last check failed (network, 5xx): the recovery notice shows.
    pub fn failed(&self) -> bool {
        self.failures > 0 && self.status == SessionStatus::Unknown
    }

    /// Seconds until the next automatic check, when one is planned.
    pub fn retry_in(&self, now: f64) -> Option<f64> {
        self.retry_at.map(|at| (at - now).max(0.0))
    }

    /// Starts GET /api/auth/session unless one is running.
    pub fn check(&mut self, client: &Client) {
        if self.check.is_some() {
            return;
        }
        self.retry_at = None;
        let http = client.http().clone();
        self.check = Some(Pending::spawn(client, async move { http.session().await }));
    }

    /// Adopts a `SessionResponse` (login, register, reset, verify).
    pub fn adopt(&mut self, response: &Value) {
        self.failures = 0;
        self.status = match response.get("user") {
            Some(user) if !user.is_null() => SessionStatus::Authenticated(user.clone()),
            _ => SessionStatus::Anonymous,
        };
    }

    fn poll(&mut self, now: f64) {
        match poll(&mut self.check) {
            Some(Ok(response)) => self.adopt(&response),
            Some(Err(_)) => {
                self.failures += 1;
                let wait = RETRY[self.failures.clamp(1, 4) - 1];
                self.retry_at = Some(now + wait);
            }
            None => {}
        }
    }
}

/// What a screen gets each frame.
pub struct AppCtx<'a> {
    pub client: &'a Client,
    /// The voice call (desktop/call).
    pub call: &'a gelabber_call::Call,
    pub images: &'a Images,
    pub nav: &'a mut Nav,
    pub session: &'a mut Session,
    /// GET /api/auth/mail: whether the server sends account mail; `None`
    /// until known.
    pub account_mail: Option<bool>,
    pub toasts: &'a mut Toasts,
}

pub struct GelabberApp {
    client: Client,
    call: gelabber_call::Call,
    images: Images,
    nav: Nav,
    session: Session,
    screen: ScreenState,
    mail_check: Option<Pending<Result<Value, ApiError>>>,
    account_mail: Option<bool>,
    toasts: Toasts,
    /// The client's workspace (lists, gateway) was started for this sign-in.
    workspace: bool,
    /// The client's own session was signed in last frame.
    client_signed_in: bool,
    /// The video tiles' pipeline is installed in the renderer.
    video_ready: bool,
    /// The theme's page colour, for the window's clear colour.
    background: std::cell::Cell<egui::Color32>,
}

impl GelabberApp {
    pub fn new(cc: &eframe::CreationContext<'_>, client: Client, route: Option<String>) -> Self {
        Self::with_context(&cc.egui_ctx, client, route)
    }

    /// Like [`Self::new`], for a context that already runs (the setup
    /// screen hands over to the app once a server is chosen).
    pub fn with_context(ctx: &egui::Context, client: Client, route: Option<String>) -> Self {
        crate::fonts::install(ctx);
        theme::ensure(ctx);
        // A new server's account: its own theme document.
        screens::settings::forget_account(ctx);
        let repaint_ctx = ctx.clone();
        client.set_repaint(std::sync::Arc::new(move || repaint_ctx.request_repaint()));
        let images = Images::new(client.clone());
        // The artwork of the signed-out pages is large; start early.
        images.get(ctx, screens::auth::layout::ARTWORK);
        let mut session = Session::new();
        session.check(&client);
        let http = client.http().clone();
        let mail_check = Some(Pending::spawn(&client, async move {
            http.get::<Value>("/auth/mail").await
        }));
        Self {
            nav: Nav::new(route.as_deref().unwrap_or("/")),
            call: gelabber_call::Call::new(client.clone()),
            client,
            images,
            session,
            screen: ScreenState::None,
            mail_check,
            account_mail: None,
            toasts: Toasts::default(),
            workspace: false,
            client_signed_in: false,
            video_ready: false,
            background: std::cell::Cell::new(Theme::dark().c.bg),
        }
    }

    /// True while data or images are still loading.
    pub fn busy(&self) -> bool {
        self.client.busy() || self.images.busy() || self.call.busy()
    }

    pub fn nav(&self) -> &Nav {
        &self.nav
    }

    /// For the desktop platform (notifications, sign-in, tray).
    pub fn client(&self) -> &Client {
        &self.client
    }

    pub fn call(&self) -> &gelabber_call::Call {
        &self.call
    }

    pub fn navigate(&mut self, href: &str) {
        self.nav.navigate(href);
    }

    pub fn back(&mut self) -> bool {
        self.nav.back()
    }

    /// The route guards of routes.tsx, once the session is known.
    fn guard(&mut self) {
        let location = self.nav.location().clone();
        let signed_in = matches!(self.session.status, SessionStatus::Authenticated(_));
        match &location.route {
            Route::Login | Route::Register | Route::ForgotPassword if signed_in => {
                self.nav
                    .replace(&location.redirect().unwrap_or_else(|| "/".into()));
            }
            Route::Invite(_) if !signed_in => {
                self.nav
                    .replace(&with_query("/register", &[("redirect", &location.href())]));
            }
            route if is_workspace(route) && !signed_in => {
                let href = location.href();
                let redirect = if href == "/" { String::new() } else { href };
                self.nav
                    .replace(&with_query("/login", &[("redirect", &redirect)]));
            }
            Route::Profile if signed_in => self.nav.replace("/settings/profil"),
            _ => {}
        }
    }

    /// Starts the client's workspace once signed in (`bootstrap`, or the
    /// session check first when the client does not know the account yet),
    /// and follows the client when the server ends the session.
    fn sync_workspace(&mut self) {
        let signed_in = matches!(self.session.status, SessionStatus::Authenticated(_));
        let client_signed_in = self
            .client
            .with_state(|s| matches!(s.session, gelabber_client::Session::Authenticated(_)));
        if signed_in && self.client_signed_in && !client_signed_in {
            self.session.adopt(&Value::Null);
            self.workspace = false;
        } else if signed_in && !self.workspace {
            self.workspace = true;
            if client_signed_in {
                self.client.bootstrap();
            } else {
                self.client.restore_session();
            }
        } else if !signed_in {
            self.workspace = false;
        }
        self.client_signed_in = client_signed_in;
    }

    fn frame(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);
        self.session.poll(now);
        if let Some(result) = poll(&mut self.mail_check) {
            self.account_mail = Some(
                result
                    .ok()
                    .and_then(|v| v.get("enabled").and_then(Value::as_bool))
                    .unwrap_or(false),
            );
        }
        if let Some(wait) = self.session.retry_in(now) {
            if wait <= 0.0 {
                self.session.check(&self.client);
            } else {
                ctx.request_repaint_after(Duration::from_secs_f64(wait.min(1.0)));
            }
        }
        // The mouse's back and forward buttons and Alt+←/→, as in the browser.
        let (back, forward) = ctx.input(|i| {
            (
                i.pointer.button_pressed(egui::PointerButton::Extra1)
                    || (i.modifiers.alt && i.key_pressed(egui::Key::ArrowLeft)),
                i.pointer.button_pressed(egui::PointerButton::Extra2)
                    || (i.modifiers.alt && i.key_pressed(egui::Key::ArrowRight)),
            )
        });
        if back {
            self.nav.back();
        } else if forward {
            self.nav.forward();
        }

        // The account's theme (or the one stored for the first paint), text
        // size and motion, on every page.
        screens::settings::apply_preferences(&ctx, &self.client);
        let theme = Theme::of(&ctx);
        self.background.set(theme.c.bg);
        let screen = ctx.content_rect();
        ui.painter().rect_filled(screen, 0.0, theme.c.bg);
        // The first Tab stop of every page.
        let skip = skip_link(ui);

        if self.session.status == SessionStatus::Unknown {
            let mut app = AppCtx {
                client: &self.client,
                call: &self.call,
                images: &self.images,
                nav: &mut self.nav,
                session: &mut self.session,
                account_mail: self.account_mail,
                toasts: &mut self.toasts,
            };
            if app.session.failed() {
                screens::auth::recovery::show(ui, &mut app);
            } else {
                screens::auth::layout::placeholder(ui);
            }
            return;
        }

        self.sync_workspace();
        let signed_in = matches!(self.session.status, SessionStatus::Authenticated(_));
        screens::voice::sync::sync(&ctx, &self.call, &mut self.toasts, signed_in);
        self.guard();
        if self.nav.take_changed() {
            self.screen = ScreenState::for_location(self.nav.location());
            crate::widgets::field::remount(&ctx);
        }
        let mut app = AppCtx {
            client: &self.client,
            call: &self.call,
            images: &self.images,
            nav: &mut self.nav,
            session: &mut self.session,
            account_mail: self.account_mail,
            toasts: &mut self.toasts,
        };
        if !is_workspace(app.nav.route()) {
            content_anchor(ui, screen.min);
        }
        screens::show(ui, &mut app, &mut self.screen);
        skip.paint(ui);
        if signed_in {
            let status = self.client.with_state(|s| s.gateway);
            screens::shell::connection::show(ui, status);
        } else {
            screens::shell::connection::show(ui, gelabber_client::GatewayStatus::Idle);
        }
        logout_failed(ui, &self.client, signed_in);
        // A file choice that no desktop chooser took: the path field.
        crate::platform::files::show_in_app(&ctx);
        self.toasts.show(ui, !is_workspace(self.nav.route()));
        crate::widgets::opener::restore(&ctx);
    }
}

/// "Zum Inhalt springen" (`.shell-skip-link`, AppShell.tsx): off screen
/// until it has focus, then in the top left corner over the page.
struct SkipLink {
    focused: bool,
    rect: egui::Rect,
}

fn skip_link(ui: &mut Ui) -> SkipLink {
    let id = egui::Id::unique("gelabber-skip-link");
    // A jump no page took up (a screen without content) does not linger.
    ui.ctx().data_mut(|d| d.remove_temp::<bool>(skip_target()));
    let focused = ui.memory(|m| m.has_focus(id));
    let label = "Zum Inhalt springen";
    let style = crate::text::TextStyle::new(16.0, 400.0, 1.5);
    let block = crate::text::Block::single(ui, label, style, egui::Color32::PLACEHOLDER);
    // padding 12px 18px at left 12px, top 12px.
    let rect = egui::Rect::from_min_size(
        ui.ctx().content_rect().min + egui::vec2(12.0, 12.0),
        egui::vec2(18.0 + block.width() + 18.0, 12.0 + block.height() + 12.0),
    );
    // Only clickable while shown; otherwise it takes no pointer.
    let sense = if focused {
        egui::Sense::click()
    } else {
        egui::Sense::focusable_noninteractive()
    };
    let response = ui.interact(rect, id, sense);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Link, true, label));
    if response.clicked() {
        // To the content: [`content_anchor`] takes the focus this frame.
        ui.ctx().data_mut(|d| d.insert_temp(skip_target(), true));
    }
    SkipLink { focused, rect }
}

fn skip_target() -> egui::Id {
    egui::Id::unique("gelabber-skip-target")
}

/// `#workspace-content` / `#app-content` (tabIndex -1): the page's start,
/// called right before the page draws. "Zum Inhalt springen" focuses it,
/// so the next Tab reaches the page's first control.
pub fn content_anchor(ui: &Ui, at: egui::Pos2) {
    let jump = ui
        .ctx()
        .data_mut(|d| d.remove_temp::<bool>(skip_target()))
        .unwrap_or(false);
    focus_anchor(ui, egui::Id::unique("gelabber-content-anchor"), at, jump);
}

/// An element with `tabIndex={-1}` at `at` that takes the focus when
/// `take` (a skip target, a heading focused on mount). It is a Tab stop
/// only while it holds the focus, so the next Tab reaches what follows it.
pub fn focus_anchor(ui: &Ui, id: egui::Id, at: egui::Pos2, take: bool) {
    if !take && !ui.memory(|m| m.has_focus(id)) {
        return;
    }
    let rect = egui::Rect::from_min_size(at, egui::vec2(1.0, 1.0));
    ui.interact(rect, id, egui::Sense::focusable_noninteractive());
    if take {
        ui.memory_mut(|m| m.request_focus(id));
    }
}

impl SkipLink {
    /// Over everything else of the page.
    fn paint(&self, ui: &Ui) {
        if !self.focused {
            return;
        }
        let theme = Theme::of(ui.ctx());
        let painter = ui.ctx().layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::unique("gelabber-skip-layer"),
        ));
        painter.rect_filled(self.rect, theme::corner(theme.radius.md), theme.c.accent);
        crate::widgets::focus_ring(&painter, self.rect, theme.radius.md, &theme);
        let block = crate::text::Block::single(
            ui,
            "Zum Inhalt springen",
            crate::text::TextStyle::new(16.0, 400.0, 1.5),
            theme.c.accent_ink,
        );
        block.paint(&painter, self.rect.min + egui::vec2(18.0, 12.0));
    }
}

/// `LogoutFailed` (SessionRecoveryNotice.tsx): the server did not confirm
/// the sign-out; a strip above the page offers "Erneut abmelden".
fn logout_failed(ui: &mut Ui, client: &Client, signed_in: bool) {
    use gelabber_client::OpState;
    let state = client.with_state(|s| s.op("logout").cloned());
    // A new sign-in ends the old problem, as applySession does.
    if signed_in {
        if matches!(state, Some(OpState::Failed(_))) {
            client.clear_op("logout");
        }
        return;
    }
    let pending = matches!(state, Some(OpState::Pending));
    let shown = ui
        .ctx()
        .data(|d| d.get_temp::<bool>(egui::Id::unique("gelabber-logout-failed")))
        .unwrap_or(false);
    let failed = matches!(state, Some(OpState::Failed(_)));
    if !failed && !(pending && shown) {
        ui.ctx()
            .data_mut(|d| d.remove::<bool>(egui::Id::unique("gelabber-logout-failed")));
        return;
    }
    ui.ctx()
        .data_mut(|d| d.insert_temp(egui::Id::unique("gelabber-logout-failed"), true));
    let theme = Theme::of(ui.ctx());
    let c = theme.c;
    let message =
        "Die Abmeldung auf dem Server konnte nicht bestätigt werden. Bitte versuche es erneut.";
    let screen = ui.ctx().content_rect();
    // margin 12px 16px, padding 12px 16px, the 3px warning bar on the left.
    let outer = egui::Rect::from_min_max(
        egui::pos2(screen.left() + 16.0, screen.top() + 12.0),
        egui::pos2(screen.right() - 16.0, screen.top() + 12.0 + 56.0),
    );
    egui::Area::new(egui::Id::unique("gelabber-logout-failed-strip"))
        .order(egui::Order::Foreground)
        .fixed_pos(outer.min)
        .show(ui.ctx(), |ui| {
            let painter = ui.painter();
            painter.rect(
                outer,
                theme::corner(theme.radius.md),
                c.elevated,
                egui::Stroke::new(1.0, c.border),
                egui::StrokeKind::Inside,
            );
            painter
                .with_clip_rect(egui::Rect::from_min_max(
                    outer.min,
                    egui::pos2(outer.left() + 3.0, outer.bottom()),
                ))
                .rect_filled(outer, theme::corner(theme.radius.md), c.warning);
            let block = crate::text::Block::single(ui, message, crate::text::UI, c.text);
            let at = egui::pos2(outer.left() + 19.0, outer.center().y - block.height() / 2.0);
            block.paint(painter, at);
            let alert = ui.interact(
                egui::Rect::from_min_size(at, block.size()),
                egui::Id::unique("gelabber-logout-failed-text"),
                egui::Sense::hover(),
            );
            crate::widgets::announce(&alert, egui::Role::Alert, message);
            let button = egui::Rect::from_min_size(
                egui::pos2(outer.right() - 16.0 - 140.0, outer.center().y - 16.0),
                egui::vec2(140.0, 32.0),
            );
            let retry = ui.put(
                button,
                crate::widgets::Button::new("Erneut abmelden")
                    .small()
                    .pending(pending.then_some("Wird abgemeldet…")),
            );
            if retry.clicked() && !pending {
                client.logout();
            }
        });
}

/// Routes inside the signed-in workspace (`workspaceLayout`).
pub fn is_workspace(route: &Route) -> bool {
    matches!(
        route,
        Route::Home
            | Route::Server(_)
            | Route::Channel(_, _)
            | Route::ServerSettings(_)
            | Route::Settings(_)
            | Route::Dms
            | Route::Dm(_)
            | Route::Profile
    )
}

impl eframe::App for GelabberApp {
    fn ui(&mut self, ui: &mut Ui, frame: &mut eframe::Frame) {
        // Video tiles draw through the renderer's wgpu device.
        if !self.video_ready
            && let Some(render_state) = frame.wgpu_render_state()
        {
            screens::voice::video::install(render_state);
            self.video_ready = true;
        }
        self.frame(ui);
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        egui::Rgba::from(self.background.get()).to_array()
    }
}

#[cfg(test)]
pub(crate) mod testkit;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locations() {
        let l = Location::parse("/login?redirect=%2Finvite%2Fabc&reason=expired#token=x");
        assert_eq!(l.route, Route::Login);
        assert_eq!(l.redirect().as_deref(), Some("/invite/abc"));
        assert_eq!(l.query("reason"), Some("expired"));
        assert_eq!(l.fragment, "token=x");
        // Malformed escapes stay as typed, also next to multi-byte text.
        assert_eq!(
            Location::parse("/x?q=%aé%zz%e2%82%ac").query("q"),
            Some("%aé%zz€")
        );
        assert!(is_invite_redirect(Some("/invite/abc")));
        assert!(!is_invite_redirect(Some("/invite/")));
        assert_eq!(safe_redirect("//evil"), None);
        assert_eq!(
            with_query("/login", &[("redirect", "/s/1?x=1")]),
            "/login?redirect=/s/1%3Fx%3D1"
        );
        assert_eq!(
            Location::parse(&with_query("/login", &[("redirect", "/s/1?x=1")]))
                .redirect()
                .as_deref(),
            Some("/s/1?x=1")
        );
    }

    #[test]
    fn navigation() {
        let mut nav = Nav::new("/login");
        nav.navigate("/register");
        assert_eq!(nav.route(), &Route::Register);
        assert!(nav.back());
        assert_eq!(nav.route(), &Route::Login);
        assert!(!nav.back());
        // Forward returns until a new navigation drops the way forward.
        assert!(nav.forward());
        assert_eq!(nav.route(), &Route::Register);
        assert!(!nav.forward());
        assert!(nav.back());
        nav.navigate("/forgot-password");
        assert!(!nav.forward());
    }
}
