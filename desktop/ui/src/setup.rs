//! The desktop app's setup screen: asks for the instance's address (the
//! Gelabber installation; "server" in the app is a community), checks that
//! Gelabber answers there, then hands the window over to [`GelabberApp`].
//! It sits in the front-door frame of the sign-in pages ([`layout::page`]:
//! artwork, icon and wordmark, one card). "Instanz wechseln …" in the user
//! menu and Ctrl+Shift+S come back here with the way back to the current
//! instance, after asking while a call runs; a stored instance that does
//! not answer at start shows a page that retries by itself (GEL-129).
//! Where the app installs updates itself, it can install one from here too
//! (desktop/parity/reference/setup is the parity reference).

use std::net::TcpStream;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{Color32, Id, Key, Modifiers, Rect, Sense, Ui, Vec2};
use gelabber_client::Client;
use gelabber_i18n::app as texts;
use parking_lot::Mutex;

use crate::app::GelabberApp;
use crate::platform::update::{self, SetupAsk, SetupView};
use crate::screens::auth::{layout, submit};
use crate::text::{self, Block};
use crate::theme::{Theme, corner, space};
use crate::widgets::field::layout_unit;
use crate::widgets::{Button, TextField};

/// Called with the origin once a server was chosen, to remember it.
pub type Remember = Box<dyn Fn(&str)>;

/// Called with each new client before its first request (the desktop app
/// puts the remembered sign-in back).
pub type Prepare = Box<dyn Fn(&Client)>;

/// `CONNECT_TIMEOUT` of the Tauri app's reachability check.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// The whole of the request that asks whether Gelabber answers.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// The waits between attempts while the stored instance does not answer at
/// start; the last one repeats.
const RETRY_WAITS: [Duration; 4] = [
    Duration::from_secs(5),
    Duration::from_secs(10),
    Duration::from_secs(30),
    Duration::from_secs(60),
];

/// Why a server cannot be used: in words for people at the field, and what
/// the system said, for a muted line under it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub message: String,
    pub detail: Option<String>,
    /// The origin another address redirects to: offered as "Use …", never
    /// followed by itself.
    pub redirect: Option<String>,
}

impl Failure {
    pub fn new(message: impl Into<String>, detail: Option<String>) -> Self {
        Self {
            message: message.into(),
            detail,
            redirect: None,
        }
    }

    /// A failed connection, by what the system reported.
    fn connect(error: &std::io::Error) -> Self {
        use std::io::ErrorKind;
        let message = match error.kind() {
            ErrorKind::ConnectionRefused => texts::setup_error_refused(),
            ErrorKind::TimedOut | ErrorKind::WouldBlock => texts::setup_error_timed_out(),
            ErrorKind::NetworkUnreachable | ErrorKind::NetworkDown => texts::setup_error_offline(),
            ErrorKind::HostUnreachable => texts::setup_error_host_unreachable(),
            _ => texts::setup_error_unreachable(),
        };
        Self::new(message, Some(error.to_string()))
    }
}

/// The reachability check of an address, on its own thread: the origin to
/// use, or why not.
type Probe = Arc<Mutex<Option<Result<String, Failure>>>>;

/// The setup page's state (its query and form).
struct Setup {
    address: String,
    /// The server in use (`?server=`): the way back to it.
    current: Option<String>,
    /// Why the page shows, or why the last check failed.
    error: Option<Failure>,
    /// Shown because the server did not answer: "Erneut verbinden mit".
    retry: bool,
    probe: Option<Probe>,
}

impl Setup {
    fn new(current: Option<String>, error: Option<Failure>) -> Self {
        Self {
            address: current.clone().unwrap_or_default(),
            retry: error.is_some(),
            current,
            error,
            probe: None,
        }
    }
}

/// The stored instance did not answer at start: tried again by itself,
/// with growing waits, until it answers or the user picks another one.
struct Retry {
    origin: String,
    /// Why the last attempt failed.
    failure: Failure,
    /// Failed attempts so far; the start-up check is the first.
    failures: usize,
    /// When the next attempt starts.
    next: Instant,
    probe: Option<Probe>,
}

impl Retry {
    fn new(origin: String, failure: Failure, now: Instant) -> Self {
        Self {
            origin,
            failure,
            failures: 1,
            next: now + RETRY_WAITS[0],
            probe: None,
        }
    }

    /// Whether an attempt should start now.
    fn due(&self, now: Instant) -> bool {
        self.probe.is_none() && now >= self.next
    }

    /// Another failed attempt: the next one waits longer.
    fn failed(&mut self, failure: Failure, now: Instant) {
        self.probe = None;
        self.failure = failure;
        self.failures += 1;
        self.next = now + RETRY_WAITS[(self.failures - 1).min(RETRY_WAITS.len() - 1)];
    }

    /// Whole seconds until the next attempt, rounded up.
    fn seconds_left(&self, now: Instant) -> u64 {
        let left = self.next.saturating_duration_since(now);
        left.as_secs() + u64::from(left.subsec_nanos() > 0)
    }
}

enum Stage {
    Setup(Setup),
    Retry(Retry),
    App {
        app: Box<GelabberApp>,
        origin: String,
        /// The start-up check of a stored instance.
        probe: Option<Probe>,
        /// "Instanz wechseln" during a call: asking whether to leave it.
        asking: bool,
    },
}

/// The language the app starts in, before any window: the one this device
/// showed last, else the system's, else English. Signing in switches to
/// the account's (app.rs). The desktop app calls it; tests and the parity
/// tool keep German unless they choose.
pub fn start_locale() {
    let locale = crate::screens::settings::prefs::stored_locale()
        .or_else(gelabber_i18n::detect_system)
        .unwrap_or(gelabber_i18n::Locale::En);
    gelabber_i18n::set(locale);
}

/// The eframe app: the setup screen, then Gelabber.
pub struct Launcher {
    stage: Stage,
    remember: Remember,
    prepare: Option<Prepare>,
    /// The page colour of the theme the setup screen last showed in.
    setup_bg: std::cell::Cell<Color32>,
}

impl Launcher {
    /// With `server`, starts Gelabber right away; otherwise asks first.
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        server: Option<String>,
        remember: Remember,
    ) -> Self {
        Self::with_prepare(cc, server, remember, None)
    }

    /// Like [`Self::new`], with a hook for each new client.
    pub fn with_prepare(
        cc: &eframe::CreationContext<'_>,
        server: Option<String>,
        remember: Remember,
        prepare: Option<Prepare>,
    ) -> Self {
        crate::fonts::install(&cc.egui_ctx);
        let mut launcher = Self {
            stage: Stage::Setup(Setup::new(None, None)),
            remember,
            prepare,
            setup_bg: std::cell::Cell::new(Theme::dark().c.bg),
        };
        if let Some(server) = server {
            match server_origin(&server) {
                Ok(origin) => {
                    launcher.open(&cc.egui_ctx, &origin, false);
                    // Like the Tauri app: open at once; when the instance does
                    // not answer, the page that tries again.
                    if let Stage::App { probe, .. } = &mut launcher.stage {
                        *probe = Some(start_probe(&cc.egui_ctx, origin, false));
                    }
                }
                Err(failure) => {
                    let mut setup = Setup::new(None, Some(failure));
                    setup.address = server;
                    launcher.stage = Stage::Setup(setup);
                }
            }
        }
        launcher
    }

    /// Gelabber on `client` right away (the parity renders and tests).
    pub fn with_app(
        cc: &eframe::CreationContext<'_>,
        client: Client,
        route: Option<String>,
    ) -> Self {
        crate::fonts::install(&cc.egui_ctx);
        let origin = client.http().base().origin().ascii_serialization();
        Self {
            stage: Stage::App {
                app: Box::new(GelabberApp::with_context(&cc.egui_ctx, client, route)),
                origin,
                probe: None,
                asking: false,
            },
            remember: Box::new(|_| {}),
            prepare: None,
            setup_bg: std::cell::Cell::new(Theme::dark().c.bg),
        }
    }

    /// The setup screen as `?server=` (`current`) and `?error=` open it.
    pub fn setup(
        cc: &eframe::CreationContext<'_>,
        current: Option<String>,
        error: Option<Failure>,
        remember: Remember,
    ) -> Self {
        crate::fonts::install(&cc.egui_ctx);
        Self {
            stage: Stage::Setup(Setup::new(current, error)),
            remember,
            prepare: None,
            setup_bg: std::cell::Cell::new(Theme::dark().c.bg),
        }
    }

    /// The running app, once an instance was chosen.
    pub fn app(&mut self) -> Option<&mut GelabberApp> {
        match &mut self.stage {
            Stage::App { app, .. } => Some(app.as_mut()),
            Stage::Setup(_) | Stage::Retry(_) => None,
        }
    }

    /// True while the app loads or an address is being checked.
    pub fn busy(&self) -> bool {
        let probing = |p: &Option<Probe>| p.as_ref().is_some_and(|p| p.lock().is_none());
        match &self.stage {
            Stage::App { app, probe, .. } => app.busy() || probing(probe),
            Stage::Setup(setup) => probing(&setup.probe),
            Stage::Retry(retry) => probing(&retry.probe),
        }
    }

    /// Whether the setup screen shows.
    pub fn in_setup(&self) -> bool {
        matches!(self.stage, Stage::Setup(_))
    }

    /// Whether the page shows that tries the stored instance again.
    pub fn in_retry(&self) -> bool {
        matches!(self.stage, Stage::Retry(_))
    }

    /// Like the Tauri app's `set_server`: only an address where Gelabber
    /// answers is stored, so a typo does not stick across restarts.
    fn check(&mut self, ctx: &egui::Context, input: &str) {
        let Stage::Setup(setup) = &mut self.stage else {
            return;
        };
        setup.error = None;
        match server_origin(input) {
            Ok(origin) => {
                // The field shows what is tried: with https:// added.
                setup.address = origin.clone();
                setup.probe = Some(start_probe(ctx, origin, true));
            }
            Err(failure) => setup.error = Some(failure),
        }
    }

    fn open(&mut self, ctx: &egui::Context, origin: &str, remember: bool) {
        match Client::new(origin) {
            Ok(client) => {
                if remember {
                    (self.remember)(origin);
                }
                if let Some(prepare) = &self.prepare {
                    prepare(&client);
                }
                self.stage = Stage::App {
                    app: Box::new(GelabberApp::with_context(ctx, client, None)),
                    origin: origin.to_owned(),
                    probe: None,
                    asking: false,
                };
            }
            Err(e) => {
                if let Stage::Setup(setup) = &mut self.stage {
                    setup.error = Some(Failure::new(
                        texts::setup_error_unreachable(),
                        Some(e.to_string()),
                    ));
                }
            }
        }
    }

    /// "Instanz wechseln": the setup screen with the current instance
    /// filled in.
    fn show_setup(&mut self, current: String) {
        self.stage = Stage::Setup(Setup::new(Some(current), None));
    }
}

/// Asks for the setup screen ("Instanz wechseln …"); the [`Launcher`]
/// around the app shows it after this frame, or first asks whether to
/// leave a running call.
pub fn switch_server(ctx: &egui::Context) {
    ctx.data_mut(|d| d.insert_temp(switch_id(), true));
}

fn switch_id() -> Id {
    Id::unique("gelabber-switch-server")
}

fn take_switch(ctx: &egui::Context) -> bool {
    ctx.data_mut(|d| d.remove_temp::<bool>(switch_id()))
        .unwrap_or(false)
}

/// Checks `origin` on a thread of its own. `gelabber`: also asks whether
/// Gelabber answers there (a chosen address); without it a connection is
/// enough (the start-up check of the stored server, which the app then
/// reconnects to by itself).
fn start_probe(ctx: &egui::Context, origin: String, gelabber: bool) -> Probe {
    let probe = Probe::default();
    let slot = probe.clone();
    let repaint = ctx.clone();
    let spawned = std::thread::Builder::new()
        .name("gelabber-setup".into())
        .spawn(move || {
            let result = check_reachable(&origin).and_then(|()| {
                if gelabber {
                    check_gelabber(&origin)
                } else {
                    Ok(origin)
                }
            });
            *slot.lock() = Some(result);
            repaint.request_repaint();
        });
    if let Err(e) = spawned {
        *probe.lock() = Some(Err(Failure::new(
            texts::setup_error_unreachable(),
            Some(e.to_string()),
        )));
    }
    probe
}

/// `server_origin` of the Tauri app: http(s) with a host, reduced to its
/// origin. Without a scheme it is https; http only when typed.
pub fn server_origin(input: &str) -> Result<String, Failure> {
    let input = input.trim();
    if input.is_empty() {
        return Err(Failure::new(texts::setup_error_empty(), None));
    }
    let with_scheme;
    let input = if input.contains("://") {
        input
    } else {
        with_scheme = format!("https://{input}");
        &with_scheme
    };
    let url =
        url::Url::parse(input).map_err(|_| Failure::new(texts::setup_error_no_address(), None))?;
    if !matches!(url.scheme(), "https" | "http") || url.host().is_none() {
        return Err(Failure::new(texts::setup_error_no_address(), None));
    }
    Ok(url.origin().ascii_serialization())
}

/// `new URL(server).host` of setup.js: the host with a port that is not
/// the default.
fn host_of(origin: &str) -> String {
    url::Url::parse(origin)
        .ok()
        .and_then(|u| {
            let host = u.host_str()?.to_owned();
            Some(match u.port() {
                Some(port) => format!("{host}:{port}"),
                None => host,
            })
        })
        .unwrap_or_else(|| origin.to_owned())
}

/// `check_reachable` of the Tauri app: something has to accept a TCP
/// connection at the origin.
fn check_reachable(origin: &str) -> Result<(), Failure> {
    let url =
        url::Url::parse(origin).map_err(|_| Failure::new(texts::setup_error_no_address(), None))?;
    let host = url
        .host_str()
        .ok_or(Failure::new(texts::setup_error_no_address(), None))?;
    let not_found =
        |detail: Option<String>| Failure::new(texts::setup_error_not_found(host), detail);
    let addrs = url
        .socket_addrs(|| None)
        .map_err(|e| not_found(Some(e.to_string())))?;
    let mut last = None;
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT) {
            Ok(_) => return Ok(()),
            Err(e) => last = Some(e),
        }
    }
    Err(match last {
        Some(e) => Failure::connect(&e),
        None => not_found(None),
    })
}

/// A redirect the check follows: within the typed origin, or the same host
/// from http to https. Anything else could be anyone on the path.
fn redirect_allowed(first: &reqwest::Url, to: &reqwest::Url) -> bool {
    to.origin() == first.origin()
        || (first.scheme() == "http" && to.scheme() == "https" && to.host() == first.host())
}

/// Whether Gelabber answers at `origin`: `GET /api/auth/mail`, which every
/// server answers signed out. A redirect from http to https on the same host
/// is followed, and its origin is the one to use; another one is named.
fn check_gelabber(origin: &str) -> Result<String, Failure> {
    let failed = |e: String| Failure::new(texts::setup_error_unreachable(), Some(e));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| failed(e.to_string()))?;
    runtime.block_on(async {
        let policy = reqwest::redirect::Policy::custom(|attempt| {
            let allowed = attempt.previous().len() < 10
                && attempt
                    .previous()
                    .first()
                    .is_some_and(|first| redirect_allowed(first, attempt.url()));
            if allowed {
                attempt.follow()
            } else {
                attempt.stop()
            }
        });
        let http = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .redirect(policy)
            .build()
            .map_err(|e| failed(e.to_string()))?;
        let response = http
            .get(format!("{origin}/api/auth/mail"))
            .send()
            .await
            .map_err(|e| request_failure(&e))?;
        let status = response.status();
        let landed = response.url().origin().ascii_serialization();
        let detail = Some(format!("HTTP {status}")); // i18n-ok
        if status.is_redirection() {
            let to = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|l| l.to_str().ok())
                .and_then(|l| response.url().join(l).ok());
            return Err(match to {
                Some(to) => {
                    let to = to.origin().ascii_serialization();
                    Failure {
                        redirect: Some(to.clone()),
                        ..Failure::new(texts::setup_error_redirect(&to), detail)
                    }
                }
                None => Failure::new(texts::setup_error_not_gelabber(), detail),
            });
        }
        if status.is_server_error() {
            return Err(Failure::new(texts::setup_error_server_trouble(), detail));
        }
        let body = response.bytes().await.map_err(|e| request_failure(&e))?;
        let json = serde_json::from_slice::<serde_json::Value>(&body).unwrap_or_default();
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            // Gelabber's rate limit answers in its own error shape; another
            // 429 (a proxy in front of it) is trouble for now, not proof.
            return if json["error"] == "rate_limited" {
                Ok(landed)
            } else {
                Err(Failure::new(texts::setup_error_server_trouble(), detail))
            };
        }
        if status.is_success() && json["enabled"].is_boolean() {
            Ok(landed)
        } else {
            Err(Failure::new(texts::setup_error_not_gelabber(), detail))
        }
    })
}

/// A request that got no answer: a certificate or TLS failure, no answer
/// in time, a refused connection, or something that does not speak HTTP.
fn request_failure(error: &reqwest::Error) -> Failure {
    // Its causes only: reqwest's own line names the URL, whose host may
    // well contain "ssl" or "tls".
    let mut causes = Vec::new();
    let mut source = std::error::Error::source(error);
    while let Some(e) = source {
        causes.push(e.to_string());
        source = e.source();
    }
    classify(
        &causes,
        error.to_string(),
        error.is_timeout(),
        error.is_connect(),
    )
}

/// [`request_failure`] by the error's causes (outermost first) and kind;
/// `fallback` is the detail when there are no causes.
fn classify(causes: &[String], fallback: String, timeout: bool, connect: bool) -> Failure {
    let lower = causes.join(" ").to_lowercase();
    let message = if lower.contains("certificate") {
        texts::setup_error_tls_certificate()
    } else if lower.contains("tls") || lower.contains("handshake") || lower.contains("ssl") {
        texts::setup_error_tls()
    } else if timeout {
        texts::setup_error_timed_out()
    } else if connect {
        texts::setup_error_refused()
    } else {
        texts::setup_error_not_gelabber()
    };
    Failure::new(message, Some(causes.last().cloned().unwrap_or(fallback)))
}

impl eframe::App for Launcher {
    fn ui(&mut self, ui: &mut Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // Ctrl+Shift+S: the setup screen, from every page (Tauri's SHORTCUTS).
        let shortcut =
            ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::S));
        let now = Instant::now();
        match &mut self.stage {
            Stage::App {
                app,
                origin,
                probe,
                asking,
            } => {
                // The question on top takes Esc from the page under it.
                let escape =
                    *asking && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
                eframe::App::ui(app.as_mut(), ui, frame);
                let failed = probe
                    .as_ref()
                    .and_then(|p| p.lock().take())
                    .and_then(Result::err);
                if failed.is_some() {
                    *probe = None;
                }
                let origin = origin.clone();
                let mut switch = take_switch(&ctx) || shortcut;
                if switch && in_call(app) {
                    // Nothing happens before the answer.
                    *asking = true;
                    switch = false;
                }
                if *asking {
                    match crate::screens::shell::overlays::confirm::dialog(
                        &ctx,
                        texts::setup_switch_in_call_title(),
                        Some(texts::setup_switch_in_call_message()),
                        texts::setup_switch_in_call_confirm(),
                        false,
                        escape,
                    ) {
                        Some(yes) => {
                            *asking = false;
                            switch = yes;
                        }
                        None => ctx.request_repaint(),
                    }
                }
                if switch || failed.is_some() {
                    // Leaving for the setup page ends the call, as the
                    // Tauri window's navigation does; the server is told.
                    app.call().leave();
                    // Its video windows close and let the old call go.
                    crate::screens::voice::window::forget(&ctx);
                }
                if switch {
                    self.show_setup(origin);
                } else if let Some(reason) = failed {
                    self.stage = Stage::Retry(Retry::new(origin, reason, now));
                }
                if !matches!(self.stage, Stage::App { .. }) {
                    ctx.request_repaint();
                }
            }
            Stage::Retry(retry) => {
                match retry.probe.as_ref().and_then(|p| p.lock().take()) {
                    Some(Ok(origin)) => {
                        self.open(&ctx, &origin, false);
                        ctx.request_repaint();
                        return;
                    }
                    Some(Err(failure)) => retry.failed(failure, now),
                    None => {}
                }
                let action = retry_page(ui, retry, now);
                self.setup_bg.set(Theme::of(&ctx).c.bg);
                match action {
                    Some(RetryAction::Now) => retry.next = now,
                    Some(RetryAction::Another) => {
                        let mut setup = Setup::new(Some(retry.origin.clone()), None);
                        // "Erneut verbinden mit" stays the way back.
                        setup.retry = true;
                        setup.address.clear();
                        self.stage = Stage::Setup(setup);
                        ctx.request_repaint();
                        return;
                    }
                    None => {}
                }
                if retry.due(now) {
                    retry.probe = Some(start_probe(&ctx, retry.origin.clone(), false));
                }
                if retry.probe.is_none() {
                    // The countdown, once a second.
                    let left = retry.next.saturating_duration_since(now);
                    let tick = Duration::from_nanos(u64::from(left.subsec_nanos()));
                    ctx.request_repaint_after(if tick.is_zero() {
                        Duration::from_secs(1)
                    } else {
                        tick
                    });
                }
            }
            Stage::Setup(setup) => {
                let checked = setup.probe.as_ref().and_then(|p| p.lock().take());
                let checking = setup.probe.is_some() && checked.is_none();
                let action = setup_page(ui, setup, checking);
                self.setup_bg.set(Theme::of(&ctx).c.bg);
                let target = match (&checked, action) {
                    (Some(_), _) | (None, None) => None,
                    (None, Some(_)) if checking => None,
                    (None, Some(Action::Connect)) => Some(setup.address.clone()),
                    (None, Some(Action::Back)) => setup.current.clone(),
                    (None, Some(Action::Use(to))) => Some(to),
                };
                match checked {
                    Some(Ok(origin)) => self.open(&ctx, &origin, true),
                    Some(Err(failure)) => {
                        setup.probe = None;
                        setup.error = Some(failure);
                    }
                    None => {
                        if let Some(target) = target {
                            self.check(&ctx, &target);
                        }
                    }
                }
            }
        }
    }

    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        match &self.stage {
            Stage::App { app, .. } => eframe::App::clear_color(app.as_ref(), visuals),
            Stage::Setup(_) | Stage::Retry(_) => egui::Rgba::from(self.setup_bg.get()).to_array(),
        }
    }
}

/// Whether leaving for the setup would end a call (`logOut`'s test).
fn in_call(app: &GelabberApp) -> bool {
    use gelabber_call::CallStatus;
    matches!(
        app.call().state().status,
        CallStatus::Connecting | CallStatus::Connected | CallStatus::Reconnecting
    )
}

enum Action {
    Connect,
    Back,
    /// The address a redirect named.
    Use(String),
}

enum RetryAction {
    Now,
    Another,
}

/// "Can't reach {host}": why, when the next attempt starts, and the ways
/// out, in the setup's frame.
fn retry_page(ui: &mut Ui, retry: &Retry, now: Instant) -> Option<RetryAction> {
    let ctx = ui.ctx().clone();
    ui.painter()
        .rect_filled(ctx.content_rect(), 0.0, Theme::of(&ctx).c.bg);
    let artwork = layout::wide(&ctx)
        .then(|| layout::bundled_artwork(&ctx))
        .flatten();
    let mut action = None;
    layout::page(ui, "setup-retry", artwork.as_ref(), false, &[], |ui| {
        let theme = Theme::of(ui.ctx());
        let width = ui.available_width();
        let title = texts::setup_cannot_reach(&host_of(&retry.origin));
        let heading = Block::wrapped(ui, &title, layout::HEADING, theme.c.text, width);
        let size = Vec2::new(width, layout_unit(heading.height()));
        let (rect, shown) = ui.allocate_exact_size(size, Sense::hover());
        heading.paint(ui.painter(), rect.min);
        crate::widgets::announce(&shown, egui::Role::Heading, &title);
        let reason = Block::wrapped(ui, &retry.failure.message, text::BODY, theme.c.text, width);
        let shown = text::show(ui, &reason);
        crate::widgets::announce(&shown, egui::Role::Alert, &retry.failure.message);
        if let Some(detail) = retry.failure.detail.as_deref() {
            ui.add_space(space::S1);
            let line = texts::setup_details(detail);
            let block = Block::wrapped(ui, &line, text::META, theme.c.muted, width);
            let shown = text::show(ui, &block);
            crate::widgets::announce(&shown, egui::Role::Label, &line);
        }
        ui.add_space(space::S4);
        let probing = retry.probe.is_some();
        let status = if probing {
            texts::setup_connecting().to_owned()
        } else {
            texts::recovery_retry_in(retry.seconds_left(now))
        };
        let block = Block::wrapped(ui, &status, text::META, theme.c.muted, width);
        let shown = text::show(ui, &block);
        crate::widgets::announce(&shown, egui::Role::Status, &status);
        ui.add_space(space::S5);
        if submit(
            ui,
            texts::setup_retry_now(),
            texts::setup_connecting(),
            probing,
        ) {
            action = Some(RetryAction::Now);
        }
        ui.add_space(space::S2);
        if ui
            .add(Button::new(texts::setup_use_another()).ghost().fill())
            .clicked()
        {
            action = Some(RetryAction::Another);
        }
        if let Some(view) = update::setup_view(ui.ctx()).filter(|v| v.installs) {
            updates_section(ui, &view);
        }
    });
    action
}

/// The setup card in the sign-in pages' frame, in the active theme (the
/// default one on the first start, the account's when switching servers).
fn setup_page(ui: &mut Ui, setup: &mut Setup, checking: bool) -> Option<Action> {
    let ctx = ui.ctx().clone();
    // The page under the artwork's 18 %, as the app paints it: a renderer
    // that does not clear to `clear_color` would show black through.
    ui.painter()
        .rect_filled(ctx.content_rect(), 0.0, Theme::of(&ctx).c.bg);
    let artwork = layout::wide(&ctx)
        .then(|| layout::bundled_artwork(&ctx))
        .flatten();
    let mut action = None;
    layout::page(ui, "setup", artwork.as_ref(), false, &[], |ui| {
        let theme = Theme::of(ui.ctx());
        let switching = setup.current.is_some() && !setup.retry;
        layout::heading(
            ui,
            if switching {
                texts::setup_title_switch()
            } else {
                texts::setup_title_connect()
            },
        );
        layout::lead(ui, texts::setup_lead());
        ui.add_space(space::S6);

        let error = setup.error.clone();
        let field = TextField::new("server", &mut setup.address)
            .label(texts::setup_address_label())
            .placeholder("https://gelabber.example.org")
            .hint(Some(texts::setup_guidance()))
            .error(error.as_ref().map(|e| e.message.as_str()))
            .alert(true)
            .autofocus(true)
            .show(ui);
        if let Some(detail) = error.as_ref().and_then(|e| e.detail.as_deref()) {
            // What the system said, for whoever runs the instance.
            ui.add_space(space::S1);
            let line = texts::setup_details(detail);
            let width = ui.available_width();
            let block = Block::wrapped(ui, &line, text::META, theme.c.muted, width);
            let size = Vec2::new(width, layout_unit(block.height()));
            let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
            block.paint(ui.painter(), rect.min);
            crate::widgets::announce(&response, egui::Role::Label, &line);
        }
        if let Some(to) = error.as_ref().and_then(|e| e.redirect.clone()) {
            // Never followed by itself: the user takes it over.
            ui.add_space(space::S3);
            if ui
                .add(
                    Button::new(&texts::setup_use_address(&to))
                        .fill()
                        .disabled(checking),
                )
                .clicked()
            {
                action = Some(Action::Use(to));
            }
        }
        if field.response.changed() {
            setup.error = None;
        }
        if field.submitted {
            action = Some(Action::Connect);
        }

        ui.add_space(space::S5);
        if submit(
            ui,
            texts::setup_connect(),
            texts::setup_connecting(),
            checking,
        ) {
            action = Some(Action::Connect);
        }
        if let Some(current) = &setup.current {
            // The way back is quieter than "Verbinden".
            ui.add_space(space::S2);
            let label = if setup.retry {
                texts::setup_reconnect_to(&host_of(current))
            } else {
                texts::setup_back_to(&host_of(current))
            };
            if ui
                .add(Button::new(&label).ghost().fill().disabled(checking))
                .clicked()
            {
                action = Some(Action::Back);
            }
        }
        if let Some(view) = update::setup_view(ui.ctx()).filter(|v| v.installs) {
            updates_section(ui, &view);
        }
    });
    action
}

/// Updates, where the app installs them itself: the status line and the
/// one thing to do, "Jetzt installieren" once an update is there. The
/// automatic check is switched in Settings › App.
fn updates_section(ui: &mut Ui, view: &SetupView) {
    let ctx = ui.ctx().clone();
    let theme = Theme::of(&ctx);
    let c = &theme.c;
    let width = ui.available_width();
    ui.add_space(space::S6);
    let (rule, _) = ui.allocate_exact_size(Vec2::new(width, 1.0), Sense::hover());
    crate::widgets::rule_h(ui.painter(), c.border, rule.x_range(), rule.top());
    ui.add_space(space::S5);

    let title = Block::single(
        ui,
        texts::setup_updates_title(),
        text::UI.weight(600.0),
        c.text,
    );
    layout::show_block(ui, &title);
    ui.add_space(space::S1);
    let line = view.status();
    let block = Block::wrapped(ui, &line, text::META, c.muted, width);
    let size = Vec2::new(width, layout_unit(block.height()));
    let (rect, status) = ui.allocate_exact_size(size, Sense::hover());
    block.paint(ui.painter(), rect.min);
    crate::widgets::announce(&status, egui::Role::Status, &line);
    if let Some(done) = view.notice.as_ref().and_then(|n| n.progress()) {
        ui.add_space(space::S1_5);
        let (track, _) = ui.allocate_exact_size(Vec2::new(width, 3.0), Sense::hover());
        ui.painter().rect_filled(track, corner(2.0), c.border);
        let bar = Rect::from_min_size(track.min, Vec2::new(track.width() * done, 3.0));
        ui.painter().rect_filled(bar, corner(2.0), c.accent);
    }
    ui.add_space(space::S3);

    if view.can_install() || view.installing() {
        let button = Button::new(texts::setup_install_now())
            .fill()
            .pending(view.installing().then_some(texts::setup_installing()));
        if ui.add(button).clicked() {
            update::ask(&ctx, SetupAsk::Install);
        }
    } else {
        let button = Button::new(texts::setup_check_updates())
            .ghost()
            .fill()
            .pending(view.checking.then_some(texts::setup_checking_updates()));
        if ui.add(button).clicked() {
            update::ask(&ctx, SetupAsk::CheckNow);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Failure, Launcher, RETRY_WAITS, Retry, check_gelabber, classify, host_of, redirect_allowed,
        request_failure, server_origin,
    };
    use crate::app::testkit::{FakeApi, Reply, click_role, launcher, wait_for, wait_for_label};
    use crate::platform::paths::testing::TempConfig;
    use egui::accesskit::Role;
    use egui_kittest::kittest::{NodeT, Queryable};
    use gelabber_i18n::app as texts;
    use std::time::{Duration, Instant};

    fn host(api: &FakeApi) -> String {
        api.origin.trim_start_matches("http://").to_owned()
    }

    /// "Instanz wechseln …" in the account menu: the setup with the way back.
    #[test]
    fn the_user_menu_switches_servers() {
        let _config = TempConfig::new("setup-menu");
        let api = FakeApi::start(true, |_, _, _| None);
        let mut harness = launcher(&api, "/");
        crate::app::testkit::open_account(&mut harness, "Lena");
        click_role(&mut harness, Role::MenuItem, "Instanz wechseln …");
        wait_for(&mut harness, "the setup", |h| h.state().in_setup());
        let back = format!("Zurück zu {}", host(&api));
        wait_for_label(&mut harness, &back);
        let field = harness.get_by_label("Adresse der Instanz");
        assert_eq!(field.value().as_deref(), Some(api.origin.as_str()));
        // Back: checked, then the app on the same server again.
        let asked = api.seen.lock().len();
        click_role(&mut harness, Role::Button, &back);
        wait_for(&mut harness, "the app", |h| !h.state().in_setup());
        wait_for_label(&mut harness, "Einstellungen");
        assert!(
            api.seen.lock()[asked..]
                .iter()
                .any(|r| r == "GET /auth/mail"),
            "asked whether Gelabber answers"
        );
    }

    /// Ctrl+Shift+S (Strg+Umschalt+S), from any page.
    #[test]
    fn the_shortcut_switches_servers() {
        let _config = TempConfig::new("setup-shortcut");
        let api = FakeApi::start(true, |_, _, _| None);
        let mut harness = launcher(&api, "/settings/darstellung");
        wait_for_label(&mut harness, "Einstellungen");
        harness.key_press_modifiers(
            egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
            egui::Key::S,
        );
        wait_for(&mut harness, "the setup", |h| h.state().in_setup());
        wait_for_label(&mut harness, &format!("Zurück zu {}", host(&api)));
        assert!(harness.query_by_label("Verbinden").is_some());
    }

    /// A port nothing listens on: bound once, then given up.
    fn free_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    fn start_with(server: String) -> egui_kittest::Harness<'static, Launcher> {
        egui_kittest::Harness::builder()
            .with_size(egui::vec2(1280.0, 800.0))
            .with_max_steps(10_000)
            .build_eframe(move |cc| {
                Launcher::with_prepare(cc, Some(server.clone()), Box::new(|_| {}), None)
            })
    }

    /// A stored instance that does not answer at start: the page that tries
    /// again, with the reason in words, what the system said, when the next
    /// attempt starts, and the ways out.
    #[test]
    fn an_unreachable_instance_is_retried_at_start() {
        let _config = TempConfig::new("setup-unreachable");
        let port = free_port();
        let mut harness = start_with(format!("http://127.0.0.1:{port}"));
        wait_for(&mut harness, "the retry page", |h| h.state().in_retry());
        wait_for_label(
            &mut harness,
            &format!("127.0.0.1:{port} ist nicht erreichbar"),
        );
        let error = harness.get_by_label(texts::setup_error_refused());
        assert_eq!(error.accesskit_node().role(), Role::Alert);
        assert!(harness.query_by_label_contains("Details: ").is_some());
        let next = harness.get_by_label_contains("Neuer Versuch in ");
        assert_eq!(next.accesskit_node().role(), Role::Status);
        assert!(harness.query_by_label("Jetzt erneut versuchen").is_some());
        // Another instance: the setup, with the way back to this one.
        click_role(&mut harness, Role::Button, "Andere Instanz verwenden");
        wait_for(&mut harness, "the setup", |h| h.state().in_setup());
        wait_for_label(
            &mut harness,
            &format!("Erneut verbinden mit 127.0.0.1:{port}"),
        );
        assert!(harness.query_by_label_contains("Zurück zu").is_none());
    }

    /// "Jetzt erneut versuchen" once the instance answers: the app.
    #[test]
    fn retrying_opens_the_app_once_the_instance_answers() {
        let _config = TempConfig::new("setup-retry-now");
        let port = free_port();
        let mut harness = start_with(format!("http://127.0.0.1:{port}"));
        wait_for(&mut harness, "the retry page", |h| h.state().in_retry());
        let _listener = std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
        click_role(&mut harness, Role::Button, "Jetzt erneut versuchen");
        wait_for(&mut harness, "the app", |h| {
            !h.state().in_retry() && !h.state().in_setup()
        });
    }

    /// The waits grow, 5, 10, 30, then 60 s for good; "Jetzt" is a wait of
    /// none.
    #[test]
    fn the_retries_wait_longer_each_time() {
        let start = Instant::now();
        let failure = || Failure::new("x", None);
        let mut retry = Retry::new("http://127.0.0.1:1".into(), failure(), start);
        assert!(!retry.due(start));
        assert_eq!(retry.seconds_left(start), 5);
        assert_eq!(retry.seconds_left(start + Duration::from_millis(4_200)), 1);
        assert!(retry.due(start + RETRY_WAITS[0]));
        let mut now = start;
        let mut waits = Vec::new();
        for _ in 0..5 {
            now += Duration::from_secs(100);
            retry.failed(failure(), now);
            waits.push(retry.next - now);
        }
        let secs = |s| Duration::from_secs(s);
        assert_eq!(waits, [secs(10), secs(30), secs(60), secs(60), secs(60)]);
        // While an attempt runs, none starts.
        retry.probe = Some(Default::default());
        assert!(!retry.due(now + secs(3600)));
    }

    /// "Instanz wechseln …" during a call asks first; "Abbrechen" keeps the
    /// call and the app, "Instanz wechseln" leaves both. Ctrl+Shift+S asks
    /// the same.
    #[test]
    fn switching_instance_in_a_call_asks_first() {
        let _config = TempConfig::new("setup-switch-call");
        let api = FakeApi::start(true, |_, _, _| None);
        let mut harness = launcher(&api, "/");
        let call = harness.state_mut().app().unwrap().call().clone();
        call.set_state(gelabber_call::CallState {
            status: gelabber_call::CallStatus::Connected,
            server: Some(uuid::Uuid::from_u128(1)),
            channel: Some(uuid::Uuid::from_u128(2)),
            ..Default::default()
        });
        let question = "Anruf verlassen und Instanz wechseln?";
        crate::app::testkit::open_account(&mut harness, "Lena");
        click_role(&mut harness, Role::MenuItem, "Instanz wechseln …");
        wait_for_label(&mut harness, question);
        assert!(
            !harness.state().in_setup(),
            "nothing happens before the answer"
        );
        assert_eq!(call.state().status, gelabber_call::CallStatus::Connected);
        click_role(&mut harness, Role::Button, "Abbrechen");
        wait_for(&mut harness, "the question to go", |h| {
            h.query_by_label(question).is_none()
        });
        assert!(!harness.state().in_setup());
        assert_eq!(call.state().status, gelabber_call::CallStatus::Connected);

        harness.key_press_modifiers(
            egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
            egui::Key::S,
        );
        wait_for_label(&mut harness, question);
        assert!(!harness.state().in_setup());
        crate::app::testkit::click_last(&mut harness, Role::Button, "Instanz wechseln");
        wait_for(&mut harness, "the setup", |h| h.state().in_setup());
    }

    /// A redirect to another address offers it as a button, which fills the
    /// field and connects there.
    #[test]
    fn a_redirect_is_taken_over_by_a_click() {
        let _config = TempConfig::new("setup-redirect");
        let good = FakeApi::start(true, |_, _, _| None);
        let target = format!("{}/api/auth/mail", good.origin);
        let moved = FakeApi::start(true, move |_, path, _| {
            (path == "/auth/mail").then(|| {
                Reply::status(302, serde_json::json!({})).header("location", target.clone())
            })
        });
        let origin = moved.origin.clone();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1280.0, 800.0))
            .with_max_steps(10_000)
            .build_eframe(move |cc| {
                Launcher::setup(cc, Some(origin.clone()), None, Box::new(|_| {}))
            });
        harness.run();
        let use_label = format!("{} verwenden", good.origin);
        assert!(harness.query_by_label(&use_label).is_none());
        harness
            .get_by_role_and_label(Role::Button, "Verbinden")
            .click();
        wait_for_label(&mut harness, &use_label);
        assert!(!good.saw("GET /auth/mail"), "not followed by itself");
        click_role(&mut harness, Role::Button, &use_label);
        wait_for(&mut harness, "the app", |h| !h.state().in_setup());
        assert!(good.saw("GET /auth/mail"));
    }

    /// An address where something answers, but not Gelabber, is not taken.
    #[test]
    fn an_address_without_gelabber_is_refused() {
        let _config = TempConfig::new("setup-not-gelabber");
        let api = FakeApi::start(true, |method, path, _| {
            (method == "GET" && path == "/auth/mail").then(|| Reply::error(404, "not_found"))
        });
        let origin = api.origin.clone();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1280.0, 800.0))
            .with_max_steps(10_000)
            .build_eframe(move |cc| {
                Launcher::setup(cc, Some(origin.clone()), None, Box::new(|_| {}))
            });
        harness.run();
        harness
            .get_by_role_and_label(Role::Button, "Verbinden")
            .click();
        wait_for_label(&mut harness, texts::setup_error_not_gelabber());
        assert!(harness.state().in_setup());
        assert!(
            harness
                .query_by_label_contains("Details: HTTP 404")
                .is_some()
        );
    }

    /// Where the app installs updates itself: the status line and one
    /// action, "Jetzt installieren" once an update is there.
    #[test]
    fn the_update_section_installs_where_the_app_updates_itself() {
        use crate::platform::update::{
            Notice, Offer, Phase, SetupAsk, SetupView, show_setup, take_ask,
        };
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1280.0, 800.0))
            .build_eframe(|cc| Launcher::setup(cc, None, None, Box::new(|_| {})));
        harness.run();
        assert!(harness.query_by_label("Updates").is_none());
        let latest = SetupView {
            current: "0.9.0".into(),
            auto_check: true,
            checked: true,
            ..SetupView::default()
        };
        // Not where a package manager or the user updates the app.
        show_setup(&harness.ctx, Some(latest.clone()));
        harness.run();
        assert!(harness.query_by_label("Updates").is_none());
        show_setup(
            &harness.ctx,
            Some(SetupView {
                installs: true,
                ..latest.clone()
            }),
        );
        harness.run();
        let status = harness.get_by_label("Installiert: 0.9.0. Das ist die neueste Version.");
        assert_eq!(status.accesskit_node().role(), Role::Status);
        assert!(
            harness
                .query_by_label("Beim Start und alle 6 Stunden nach Updates suchen")
                .is_none(),
            "the switch is in Settings › App"
        );
        harness
            .get_by_role_and_label(Role::Button, "Nach Updates suchen")
            .click();
        harness.run();
        assert_eq!(take_ask(&harness.ctx), Some(SetupAsk::CheckNow));

        show_setup(
            &harness.ctx,
            Some(SetupView {
                installs: true,
                available: Some("0.9.1".into()),
                notice: Some(Notice::new("0.9.1", Offer::Restart)),
                ..latest.clone()
            }),
        );
        harness.run();
        harness.get_by_label("Installiert: 0.9.0. Verfügbar: 0.9.1.");
        harness
            .get_by_role_and_label(Role::Button, "Jetzt installieren")
            .click();
        harness.run();
        assert_eq!(take_ask(&harness.ctx), Some(SetupAsk::Install));

        let mut notice = Notice::new("0.9.1", Offer::Restart);
        notice.phase = Phase::Downloading {
            downloaded: 19,
            total: Some(47),
        };
        show_setup(
            &harness.ctx,
            Some(SetupView {
                installs: true,
                available: Some("0.9.1".into()),
                notice: Some(notice),
                ..latest
            }),
        );
        harness.run();
        harness.get_by_label("Update wird geladen … 40 %");
        let busy = harness.get_by_role_and_label(Role::Button, "Wird installiert…");
        assert!(busy.accesskit_node().is_disabled());
    }

    /// What the server answers decides: Gelabber's mail status or its rate
    /// limit pass; a server error or a foreign 429 is trouble for now, with
    /// the status in the details only.
    #[test]
    fn the_answer_tells_gelabber_from_trouble() {
        let api = FakeApi::start(true, |method, path, _| {
            if method != "GET" {
                return None;
            }
            match path {
                "/auth/mail" => None,
                "/limited/api/auth/mail" => Some(Reply::error(429, "rate_limited")),
                "/proxy/api/auth/mail" => Some(Reply::status(429, serde_json::json!("slow"))),
                "/broken/api/auth/mail" => Some(Reply::error(502, "upstream")),
                _ => Some(Reply::error(404, "not_found")),
            }
        });
        let origin = api.origin.clone();
        assert_eq!(check_gelabber(&origin), Ok(origin.clone()));
        // Under a path, so one fake server answers each way.
        let at = |prefix: &str| check_gelabber(&format!("{origin}/{prefix}"));
        assert_eq!(at("limited"), Ok(origin.clone()));
        let trouble = |status: &str| {
            Err(Failure::new(
                texts::setup_error_server_trouble(),
                Some(status.into()),
            ))
        };
        assert_eq!(at("proxy"), trouble("HTTP 429 Too Many Requests"));
        assert_eq!(at("broken"), trouble("HTTP 502 Bad Gateway"));
    }

    /// Only the same host going from http to https is followed; another
    /// address is named, not taken (a redirect anyone on the path can send).
    #[test]
    fn a_redirect_to_another_address_is_named_not_taken() {
        let good = FakeApi::start(true, |_, _, _| None);
        let target = format!("{}/api/auth/mail", good.origin);
        let evil = FakeApi::start(true, move |_, path, _| match path {
            "/auth/mail" => {
                Some(Reply::status(302, serde_json::json!({})).header("location", target.clone()))
            }
            "/moved/api/auth/mail" => Some(
                Reply::status(301, serde_json::json!({})).header("location", "/ok/api/auth/mail"),
            ),
            "/ok/api/auth/mail" => Some(Reply::json(serde_json::json!({"enabled": true}))),
            _ => None,
        });
        let failure = check_gelabber(&evil.origin).unwrap_err();
        assert!(
            failure.message.contains(&good.origin),
            "{}",
            failure.message
        );
        assert_eq!(failure.redirect.as_deref(), Some(good.origin.as_str()));
        assert!(!good.saw("GET /auth/mail"), "not followed");
        // Within the same origin a redirect is fine.
        let moved = check_gelabber(&format!("{}/moved", evil.origin));
        assert_eq!(moved, Ok(evil.origin.clone()));

        let url = |u: &str| reqwest::Url::parse(u).unwrap();
        let from = url("http://chat.example/api/auth/mail");
        assert!(redirect_allowed(
            &from,
            &url("https://chat.example/api/auth/mail")
        ));
        assert!(redirect_allowed(&from, &url("http://chat.example/x")));
        assert!(!redirect_allowed(
            &from,
            &url("https://evil.example/api/auth/mail")
        ));
        assert!(!redirect_allowed(
            &from,
            &url("http://chat.example:8080/api/auth/mail")
        ));
        let secure = url("https://chat.example/api/auth/mail");
        assert!(!redirect_allowed(
            &secure,
            &url("http://chat.example/api/auth/mail")
        ));
    }

    /// TLS words count in the causes only, not in the URL reqwest names.
    #[test]
    fn request_errors_read_as_words() {
        let causes = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let tls = classify(
            &causes(&[
                "client error (Connect)",
                "invalid peer certificate: UnknownIssuer",
            ]),
            String::new(),
            false,
            true,
        );
        assert_eq!(tls.message, texts::setup_error_tls_certificate());
        assert_eq!(
            tls.detail.as_deref(),
            Some("invalid peer certificate: UnknownIssuer")
        );
        let handshake = classify(
            &causes(&[
                "client error (Connect)",
                "received fatal alert: HandshakeFailure",
            ]),
            String::new(),
            false,
            true,
        );
        assert_eq!(handshake.message, texts::setup_error_tls());
        let slow = classify(
            &causes(&["operation timed out"]),
            String::new(),
            true,
            false,
        );
        assert_eq!(slow.message, texts::setup_error_timed_out());
        let refused = classify(
            &causes(&["tcp connect error", "Connection refused (os error 111)"]),
            String::new(),
            false,
            true,
        );
        assert_eq!(refused.message, texts::setup_error_refused());
        assert_eq!(
            classify(&[], "error decoding response".into(), false, false),
            Failure::new(
                texts::setup_error_not_gelabber(),
                Some("error decoding response".into())
            )
        );

        // A real refused request at a URL full of TLS words.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let error = runtime
            .block_on(reqwest::get(format!(
                "http://127.0.0.1:{port}/ssl/tls/handshake"
            )))
            .unwrap_err();
        assert!(error.to_string().contains("ssl/tls"), "{error}");
        assert_eq!(
            request_failure(&error).message,
            texts::setup_error_refused()
        );
    }

    #[test]
    fn origins() {
        assert_eq!(
            server_origin(" https://chat.example.org/app?x=1 ").as_deref(),
            Ok("https://chat.example.org")
        );
        assert_eq!(
            server_origin("http://localhost:5173").as_deref(),
            Ok("http://localhost:5173")
        );
        // Default ports drop out of the origin, as in the Tauri app.
        assert_eq!(
            server_origin("HTTPS://Chat.Example.org:443/").as_deref(),
            Ok("https://chat.example.org")
        );
        assert_eq!(
            server_origin("file:///etc/passwd").unwrap_err().message,
            texts::setup_error_no_address()
        );
        // Without a scheme: https; http only when typed.
        assert_eq!(
            server_origin("chat.example.org").as_deref(),
            Ok("https://chat.example.org")
        );
        assert_eq!(
            server_origin(" localhost:5173/app ").as_deref(),
            Ok("https://localhost:5173")
        );
        assert_eq!(
            server_origin("http://chat.example.org").as_deref(),
            Ok("http://chat.example.org")
        );
        assert_eq!(
            server_origin("chat example").unwrap_err().message,
            texts::setup_error_no_address()
        );
        assert!(server_origin("  ").is_err());
        assert_eq!(host_of("http://127.0.0.1:5180"), "127.0.0.1:5180");
        assert_eq!(host_of("https://chat.example.org"), "chat.example.org");
    }
}
