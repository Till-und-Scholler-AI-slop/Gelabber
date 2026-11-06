//! The desktop app's setup screen, as the Tauri app bundled it
//! (desktop/parity/reference/setup, the parity reference): asks for the
//! server address, checks that it answers, then hands the window over to
//! [`GelabberApp`]. "Server wechseln …" in the user menu and Ctrl+Shift+S
//! come back here with the way back to the current server; a stored server
//! that does not answer at start shows it with the reason.

use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;

use egui::text::LayoutJob;
use egui::{
    Color32, CornerRadius, Frame, Id, Key, Margin, Modifiers, Rect, Sense, Stroke, StrokeKind,
    TextBuffer, TextEdit, Ui, UiBuilder, Vec2, pos2,
};
use gelabber_client::Client;
use parking_lot::Mutex;

use crate::app::GelabberApp;
use crate::text::{Block, TextStyle};

/// Called with the origin once a server was chosen, to remember it.
pub type Remember = Box<dyn Fn(&str)>;

/// Called with each new client before its first request (the desktop app
/// puts the remembered sign-in back).
pub type Prepare = Box<dyn Fn(&Client)>;

/// `CONNECT_TIMEOUT` of the Tauri app's reachability check.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// The reachability check of an address, on its own thread.
type Probe = Arc<Mutex<Option<Result<String, String>>>>;

/// The setup page's state (its query and form).
struct Setup {
    address: String,
    /// The server in use (`?server=`): the way back to it.
    current: Option<String>,
    /// Why the page shows, or why the last check failed.
    error: Option<String>,
    /// Shown because the server did not answer: "Erneut verbinden mit".
    retry: bool,
    probe: Option<Probe>,
}

impl Setup {
    fn new(current: Option<String>, error: Option<String>) -> Self {
        Self {
            address: current.clone().unwrap_or_default(),
            retry: error.is_some(),
            current,
            error,
            probe: None,
        }
    }
}

enum Stage {
    Setup(Setup),
    App {
        app: Box<GelabberApp>,
        origin: String,
        /// The start-up check of a stored server.
        probe: Option<Probe>,
    },
}

/// The eframe app: the setup screen, then Gelabber.
pub struct Launcher {
    stage: Stage,
    remember: Remember,
    prepare: Option<Prepare>,
    /// The system was dark when the setup screen last showed.
    setup_dark: std::cell::Cell<bool>,
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
            setup_dark: std::cell::Cell::new(true),
        };
        if let Some(server) = server {
            match server_origin(&server) {
                Ok(origin) => {
                    launcher.open(&cc.egui_ctx, &origin, false);
                    // Like the Tauri app: open at once, and show the setup with
                    // the reason when the server does not answer.
                    if let Stage::App { probe, .. } = &mut launcher.stage {
                        *probe = Some(start_probe(&cc.egui_ctx, origin));
                    }
                }
                Err(message) => {
                    let mut setup = Setup::new(None, Some(message));
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
            },
            remember: Box::new(|_| {}),
            prepare: None,
            setup_dark: std::cell::Cell::new(true),
        }
    }

    /// The setup screen as `?server=` (`current`) and `?error=` open it.
    pub fn setup(
        cc: &eframe::CreationContext<'_>,
        current: Option<String>,
        error: Option<String>,
        remember: Remember,
    ) -> Self {
        crate::fonts::install(&cc.egui_ctx);
        Self {
            stage: Stage::Setup(Setup::new(current, error)),
            remember,
            prepare: None,
            setup_dark: std::cell::Cell::new(true),
        }
    }

    /// The running app, once a server was chosen.
    pub fn app(&mut self) -> Option<&mut GelabberApp> {
        match &mut self.stage {
            Stage::App { app, .. } => Some(app.as_mut()),
            Stage::Setup(_) => None,
        }
    }

    /// True while the app loads or an address is being checked.
    pub fn busy(&self) -> bool {
        let probing = |p: &Option<Probe>| p.as_ref().is_some_and(|p| p.lock().is_none());
        match &self.stage {
            Stage::App { app, probe, .. } => app.busy() || probing(probe),
            Stage::Setup(setup) => probing(&setup.probe),
        }
    }

    /// Whether the setup screen shows.
    pub fn in_setup(&self) -> bool {
        matches!(self.stage, Stage::Setup(_))
    }

    /// Like the Tauri app's `set_server`: only an address that accepts a
    /// connection is stored, so a typo does not stick across restarts.
    fn check(&mut self, ctx: &egui::Context, input: &str) {
        let Stage::Setup(setup) = &mut self.stage else {
            return;
        };
        setup.error = None;
        match server_origin(input) {
            Ok(origin) => setup.probe = Some(start_probe(ctx, origin)),
            Err(message) => setup.error = Some(message),
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
                };
            }
            Err(e) => {
                if let Stage::Setup(setup) = &mut self.stage {
                    setup.error = Some(e.to_string());
                }
            }
        }
    }

    /// "Server wechseln": the setup screen with the current server filled in.
    fn show_setup(&mut self, current: String, error: Option<String>) {
        self.stage = Stage::Setup(Setup::new(Some(current), error));
    }
}

/// Asks for the setup screen ("Server wechseln …"); the [`Launcher`]
/// around the app shows it after this frame.
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

fn start_probe(ctx: &egui::Context, origin: String) -> Probe {
    let probe = Probe::default();
    let slot = probe.clone();
    let repaint = ctx.clone();
    let spawned = std::thread::Builder::new()
        .name("gelabber-setup".into())
        .spawn(move || {
            let result = check_reachable(&origin).map(|()| origin);
            *slot.lock() = Some(result);
            repaint.request_repaint();
        });
    if let Err(e) = spawned {
        *probe.lock() = Some(Err(e.to_string()));
    }
    probe
}

/// `server_origin` of the Tauri app: http(s) with a host, reduced to its
/// origin.
pub fn server_origin(input: &str) -> Result<String, String> {
    let url = url::Url::parse(input.trim()).map_err(|e| format!("Ungültige Serveradresse: {e}"))?;
    if !matches!(url.scheme(), "https" | "http") || url.host().is_none() {
        return Err("Die Serveradresse braucht http:// oder https:// und einen Host.".into());
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
fn check_reachable(origin: &str) -> Result<(), String> {
    let url = url::Url::parse(origin).map_err(|e| e.to_string())?;
    let host = url.host_str().ok_or("Serveradresse ohne Host")?;
    let port = url
        .port_or_known_default()
        .ok_or("Serveradresse ohne Port")?;
    let unreachable = |reason: String| format!("{host}:{port} ist nicht erreichbar ({reason}).");
    let addrs = url
        .socket_addrs(|| None)
        .map_err(|e| unreachable(e.to_string()))?;
    let mut last = "keine Adresse".to_owned();
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT) {
            Ok(_) => return Ok(()),
            Err(e) => last = e.to_string(),
        }
    }
    Err(unreachable(last))
}

impl eframe::App for Launcher {
    fn ui(&mut self, ui: &mut Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // Ctrl+Shift+S: the setup screen, from every page (Tauri's SHORTCUTS).
        let shortcut =
            ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::S));
        match &mut self.stage {
            Stage::App { app, origin, probe } => {
                eframe::App::ui(app.as_mut(), ui, frame);
                let failed = probe
                    .as_ref()
                    .and_then(|p| p.lock().take())
                    .and_then(Result::err);
                if failed.is_some() {
                    *probe = None;
                }
                let origin = origin.clone();
                let switch = take_switch(&ctx) || shortcut;
                if switch || failed.is_some() {
                    // Leaving for the setup page ends the call, as the
                    // Tauri window's navigation does; the server is told.
                    app.call().leave();
                }
                if switch {
                    self.show_setup(origin, None);
                } else if let Some(reason) = failed {
                    self.show_setup(origin, Some(reason));
                }
                if self.in_setup() {
                    ctx.request_repaint();
                }
            }
            Stage::Setup(setup) => {
                let checked = setup.probe.as_ref().and_then(|p| p.lock().take());
                let checking = setup.probe.is_some() && checked.is_none();
                let action = setup_page(ui, setup, checking);
                self.setup_dark.set(crate::theme::system_dark(&ctx));
                let target = match (&checked, action) {
                    (Some(_), _) | (None, None) => None,
                    (None, Some(_)) if checking => None,
                    (None, Some(Action::Connect)) => Some(setup.address.clone()),
                    (None, Some(Action::Back)) => setup.current.clone(),
                };
                match checked {
                    Some(Ok(origin)) => self.open(&ctx, &origin, true),
                    Some(Err(message)) => {
                        setup.probe = None;
                        setup.error = Some(message);
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
            Stage::Setup(_) => egui::Rgba::from(Palette::of(self.setup_dark.get()).bg).to_array(),
        }
    }
}

/// setup.css: its own palette, light or dark with the system.
#[derive(Clone, Copy)]
struct Palette {
    bg: Color32,
    panel: Color32,
    text: Color32,
    muted: Color32,
    border: Color32,
    accent: Color32,
    accent_ink: Color32,
    danger: Color32,
}

impl Palette {
    fn of(dark: bool) -> Self {
        use crate::theme::hex;
        if dark {
            Self {
                bg: hex(0x17191a),
                panel: hex(0x1d2021),
                text: hex(0xeef0ee),
                muted: hex(0xa4abb1),
                border: hex(0x2c3032),
                accent: hex(0xf3b752),
                accent_ink: hex(0x191b1b),
                danger: hex(0xf39b98),
            }
        } else {
            Self {
                bg: hex(0xf8f5f0),
                panel: hex(0xf4f0e9),
                text: hex(0x252829),
                muted: hex(0x646d74),
                border: hex(0xdcd6cc),
                accent: hex(0xe8a638),
                accent_ink: hex(0x251e12),
                danger: hex(0xa12b29),
            }
        }
    }
}

/// Chromium's `::placeholder` colour (darkGray at 0.7 over the field).
const PLACEHOLDER: Color32 = crate::theme::hex(0x757575);
const REM: f32 = 16.0;

/// `font: <size> system-ui` with `line-height: normal`, as Blink rounds it.
fn normal(size: f32, weight: f32) -> TextStyle {
    let m = crate::fonts::system_ui_metrics();
    let line = (m.ascent * size).round() + (m.descent * size).round();
    TextStyle::new(size, weight, line / size).system_ui()
}

enum Action {
    Connect,
    Back,
}

/// The setup card of index.html laid out by setup.css.
fn setup_page(ui: &mut Ui, setup: &mut Setup, checking: bool) -> Option<Action> {
    let ctx = ui.ctx().clone();
    crate::fonts::use_system_ui(&ctx);
    let dark = crate::theme::system_dark(&ctx);
    let p = Palette::of(dark);
    let screen = ctx.content_rect();
    ui.painter().rect_filled(screen, 0.0, p.bg);

    // main: max-width 26rem, margin 18vh auto 0, padding 2rem, 1px border.
    let content_w = (26.0 * REM).min(screen.width() - 4.0 * REM - 2.0).max(0.0);
    let outer_w = content_w + 4.0 * REM + 2.0;
    let left = screen.left() + ((screen.width() - outer_w) / 2.0).max(0.0);
    let top = screen.top() + 0.18 * screen.height();
    let x = left + 1.0 + 2.0 * REM;
    let mut y = top + 1.0 + 2.0 * REM;
    let card = ui.painter().add(egui::Shape::Noop);

    let h1 = normal(1.5 * REM, 700.0).tracking(-0.02);
    let title = Block::single(ui, "Gelabber", h1, p.text);
    title.paint(ui.painter(), pos2(x, y));
    y += title.height() + 0.25 * REM;
    let lead = Block::wrapped(
        ui,
        "Mit welchem Gelabber-Server soll sich die App verbinden?",
        TextStyle::new(REM, 400.0, 1.5).system_ui(),
        p.muted,
        content_w,
    );
    lead.paint(ui.painter(), pos2(x, y));
    y += lead.height() + 1.5 * REM;

    // form: a grid with 0.6rem gaps.
    let gap = 0.6 * REM;
    let label = Block::single(ui, "Serveradresse", normal(0.85 * REM, 600.0), p.text);
    label.paint(ui.painter(), pos2(x, y));
    let input_id = crate::widgets::field::field_id("server");
    let label_rect = Rect::from_min_size(pos2(x, y), label.size());
    if ui
        .interact(label_rect, input_id.with("label"), Sense::CLICK)
        .clicked()
    {
        ui.memory_mut(|m| m.request_focus(input_id));
    }
    y += label.height() + gap;

    // input: min-height 2.6rem plus its border, padding 0 0.85rem.
    let control = 2.6 * REM;
    let input = Rect::from_min_size(pos2(x, y), Vec2::new(content_w, control + 2.0));
    if crate::widgets::field::mounted(&ctx, input_id.with("autofocus")) {
        ui.memory_mut(|m| m.request_focus(input_id));
    }
    let focused = ui.memory(|m| m.has_focus(input_id));
    if focused {
        // outline: 2px solid accent, offset 1px.
        ui.painter().rect_stroke(
            input.expand(2.0),
            CornerRadius::same(10),
            Stroke::new(2.0, p.accent),
            StrokeKind::Middle,
        );
    }
    ui.painter().rect(
        input,
        CornerRadius::same(8),
        p.bg,
        Stroke::new(1.0, p.border),
        StrokeKind::Inside,
    );
    let body = normal(REM, 400.0);
    let text_color = p.text;
    let mut layouter = move |ui: &Ui, buf: &dyn TextBuffer, _wrap: f32| {
        let mut job = LayoutJob::single_section(buf.as_str().to_owned(), body.format(text_color));
        job.keep_trailing_whitespace = true;
        ui.fonts_mut(|f| f.layout_job(job))
    };
    let pad_y = ((control - body.line_px()) / 2.0).round() as i8;
    let inner = input.shrink(1.0);
    let edit = TextEdit::singleline(&mut setup.address)
        .id(input_id)
        .frame(Frame::NONE.inner_margin(Margin {
            left: (0.85 * REM).round() as i8,
            right: (0.85 * REM).round() as i8,
            top: pad_y,
            bottom: pad_y,
        }))
        .desired_width(inner.width())
        .min_size(inner.size())
        .text_color(p.text)
        .layouter(&mut layouter)
        .hint_text(LayoutJob::single_section(
            "https://gelabber.example.org".to_owned(),
            body.format(PLACEHOLDER),
        ));
    let output = ui
        .scope_builder(UiBuilder::new().max_rect(inner), |ui| edit.show(ui))
        .inner;
    let response = output.response.response.accessible_name("Serveradresse");
    let mut action = (response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)))
        .then_some(Action::Connect);
    y = input.bottom() + gap;

    let button = |ui: &mut Ui, y: f32, label: &str, primary: bool, disabled: bool| -> bool {
        let rect = Rect::from_min_size(pos2(x, y), Vec2::new(content_w, control));
        let id = Id::unique(("gelabber-setup-button", label));
        let response = ui.interact(rect, id, Sense::click());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, !disabled, label));
        // button:disabled { opacity: 0.6 }
        let fade = |c: Color32| {
            if disabled {
                crate::theme::mix(p.panel, c, 0.6)
            } else {
                c
            }
        };
        let (fill, ink, stroke, weight) = if primary {
            (
                fade(p.accent),
                fade(p.accent_ink),
                Color32::TRANSPARENT,
                650.0,
            )
        } else {
            (Color32::TRANSPARENT, fade(p.text), fade(p.border), 500.0)
        };
        ui.painter().rect(
            rect,
            CornerRadius::same(8),
            fill,
            Stroke::new(1.0, stroke),
            StrokeKind::Inside,
        );
        let text = Block::single(ui, label, normal(REM, weight), ink);
        let at = pos2(
            rect.center().x - text.width() / 2.0,
            rect.top() + (control - text.height()) / 2.0,
        );
        text.paint(ui.painter(), at);
        if response.hovered() && !disabled {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        response.clicked() && !disabled
    };
    if button(ui, y, "Verbinden", true, checking) {
        action = Some(Action::Connect);
    }
    y += control;
    if let Some(current) = &setup.current {
        y += gap;
        let lead = if setup.retry {
            "Erneut verbinden mit "
        } else {
            "Zurück zu "
        };
        if button(
            ui,
            y,
            &format!("{lead}{}", host_of(current)),
            false,
            checking,
        ) {
            action = Some(Action::Back);
        }
        y += control;
    }
    if let Some(error) = &setup.error {
        y += gap + 0.25 * REM;
        let block = Block::wrapped(
            ui,
            error,
            TextStyle::new(0.9 * REM, 400.0, 1.45).system_ui(),
            p.danger,
            content_w,
        );
        block.paint(ui.painter(), pos2(x, y));
        let rect = Rect::from_min_size(pos2(x, y), block.size());
        let alert = ui.interact(rect, Id::unique("gelabber-setup-error"), Sense::hover());
        crate::widgets::announce(&alert, egui::Role::Alert, error);
        y += block.height();
    }
    if let Some(view) = crate::platform::update::setup_view(&ctx) {
        y = updates_section(ui, &view, &p, x, y, content_w, &button);
    }
    let bottom = y + 2.0 * REM + 1.0;
    ui.painter().set(
        card,
        egui::epaint::RectShape::new(
            Rect::from_min_max(pos2(left, top), pos2(left + outer_w, bottom)),
            CornerRadius::same(14),
            p.panel,
            Stroke::new(1.0, p.border),
            StrokeKind::Inside,
        ),
    );
    action
}

/// `#updates`: a grid with 0.6rem gaps under a top border, 1.5rem below the
/// form; the Tauri page shows it only where the app updates itself.
fn updates_section(
    ui: &mut Ui,
    view: &crate::platform::update::SetupView,
    p: &Palette,
    x: f32,
    mut y: f32,
    width: f32,
    button: &dyn Fn(&mut Ui, f32, &str, bool, bool) -> bool,
) -> f32 {
    use crate::platform::update::{SetupAsk, ask};
    let ctx = ui.ctx().clone();
    let gap = 0.6 * REM;
    y += 1.5 * REM;
    ui.painter().rect_filled(
        Rect::from_min_size(pos2(x, y), Vec2::new(width, 1.0)),
        0.0,
        p.border,
    );
    y += 1.0 + 1.25 * REM;
    let title = Block::single(ui, "Updates", normal(REM, 700.0), p.text);
    title.paint(ui.painter(), pos2(x, y));
    y += title.height() + gap;

    // label.check: a 1rem box, 0.5rem gap, the text centred beside it.
    let label = "Beim Start und alle 6 Stunden nach Updates suchen";
    let text = Block::single(ui, label, normal(0.85 * REM, 500.0), p.text);
    let row_h = text.height().max(REM);
    let row = Rect::from_min_size(pos2(x, y), Vec2::new(REM * 1.5 + text.width(), row_h));
    let response = ui.interact(row, Id::unique("gelabber-setup-auto-check"), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::CheckBox, !view.checking, view.auto_check, label)
    });
    let check = Rect::from_min_size(pos2(x, y + (row_h - REM) / 2.0), Vec2::splat(REM));
    paint_checkbox(ui, check, view.auto_check, view.checking, p);
    text.paint(
        ui.painter(),
        pos2(x + 1.5 * REM, y + (row_h - text.height()) / 2.0),
    );
    if response.clicked() && !view.checking {
        ask(&ctx, SetupAsk::AutoCheck(!view.auto_check));
    }
    y += row_h + gap;

    if button(ui, y, "Jetzt nach Updates suchen", false, view.checking) {
        ask(&ctx, SetupAsk::CheckNow);
    }
    y += 2.6 * REM + gap;

    // #update-status: role status, 0.9rem/1.45 muted.
    let line = view.line();
    let block = Block::wrapped(
        ui,
        &line,
        TextStyle::new(0.9 * REM, 400.0, 1.45).system_ui(),
        p.muted,
        width,
    );
    block.paint(ui.painter(), pos2(x, y));
    let rect = Rect::from_min_size(pos2(x, y), block.size());
    let status = ui.interact(
        rect,
        Id::unique("gelabber-setup-update-status"),
        Sense::hover(),
    );
    crate::widgets::announce(&status, egui::Role::Status, &line);
    y + block.height()
}

/// Chromium's checkbox with `accent-color`: a 2px-rounded box, filled with
/// the accent and a check when on, a grey frame when off.
fn paint_checkbox(ui: &Ui, rect: Rect, on: bool, disabled: bool, p: &Palette) {
    let dark = crate::theme::system_dark(ui.ctx());
    let fade = |c: Color32| {
        if disabled {
            crate::theme::mix(p.panel, c, 0.6)
        } else {
            c
        }
    };
    let painter = ui.painter();
    if on {
        painter.rect_filled(rect, CornerRadius::same(2), fade(p.accent));
        let s = rect.width() / 16.0;
        let at = |px: f32, py: f32| pos2(rect.left() + px * s, rect.top() + py * s);
        painter.add(egui::Shape::line(
            vec![at(3.5, 8.0), at(6.5, 11.0), at(12.5, 5.0)],
            Stroke::new(2.0 * s, fade(CHECK_MARK)),
        ));
    } else {
        let (fill, frame) = if dark {
            (crate::theme::hex(0x3b3b3b), crate::theme::hex(0x858585))
        } else {
            (Color32::WHITE, crate::theme::hex(0x767676))
        };
        painter.rect(
            rect,
            CornerRadius::same(2),
            fade(fill),
            Stroke::new(1.0, fade(frame)),
            StrokeKind::Inside,
        );
    }
}

/// The check Chromium draws on a light accent.
const CHECK_MARK: Color32 = Color32::BLACK;

#[cfg(test)]
mod tests {
    use super::{Launcher, host_of, server_origin};
    use crate::app::testkit::{FakeApi, launcher, wait_for, wait_for_label};
    use crate::platform::paths::testing::TempConfig;
    use egui::accesskit::Role;
    use egui_kittest::kittest::{NodeT, Queryable};

    fn host(api: &FakeApi) -> String {
        api.origin.trim_start_matches("http://").to_owned()
    }

    /// "Server wechseln …" in the user menu: the setup with the way back.
    #[test]
    fn the_user_menu_switches_servers() {
        let _config = TempConfig::new("setup-menu");
        let api = FakeApi::start(true, |_, _, _| None);
        let mut harness = launcher(&api, "/");
        wait_for_label(&mut harness, "Benutzermenü");
        harness
            .get_by_role_and_label(Role::Button, "Benutzermenü")
            .click();
        wait_for_label(&mut harness, "Server wechseln …");
        harness
            .get_by_role_and_label(Role::Button, "Server wechseln …")
            .click();
        wait_for(&mut harness, "the setup", |h| h.state().in_setup());
        let back = format!("Zurück zu {}", host(&api));
        wait_for_label(&mut harness, &back);
        let field = harness.get_by_label("Serveradresse");
        assert_eq!(field.value().as_deref(), Some(api.origin.as_str()));
        // Back: checked, then the app on the same server again.
        harness.get_by_role_and_label(Role::Button, &back).click();
        wait_for(&mut harness, "the app", |h| !h.state().in_setup());
        wait_for_label(&mut harness, "Benutzermenü");
    }

    /// Ctrl+Shift+S (Strg+Umschalt+S), from any page.
    #[test]
    fn the_shortcut_switches_servers() {
        let _config = TempConfig::new("setup-shortcut");
        let api = FakeApi::start(true, |_, _, _| None);
        let mut harness = launcher(&api, "/settings/darstellung");
        wait_for_label(&mut harness, "Benutzermenü");
        harness.key_press_modifiers(
            egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
            egui::Key::S,
        );
        wait_for(&mut harness, "the setup", |h| h.state().in_setup());
        wait_for_label(&mut harness, &format!("Zurück zu {}", host(&api)));
        assert!(harness.query_by_label("Verbinden").is_some());
    }

    /// A stored server that does not answer: the setup with the reason and
    /// "Erneut verbinden mit".
    #[test]
    fn an_unreachable_server_starts_in_setup() {
        let _config = TempConfig::new("setup-unreachable");
        // A port nothing listens on: bound once, then given up.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let server = format!("http://127.0.0.1:{port}");
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1280.0, 800.0))
            .with_max_steps(10_000)
            .build_eframe(move |cc| {
                Launcher::with_prepare(cc, Some(server.clone()), Box::new(|_| {}), None)
            });
        wait_for(&mut harness, "the setup", |h| h.state().in_setup());
        let retry = format!("Erneut verbinden mit 127.0.0.1:{port}");
        wait_for_label(&mut harness, &retry);
        let error = harness.get_by_label_contains("ist nicht erreichbar");
        assert_eq!(error.accesskit_node().role(), Role::Alert);
        let text = error.accesskit_node().label().unwrap_or_default();
        assert!(text.starts_with(&format!("127.0.0.1:{port} ist nicht erreichbar (")));
        assert!(harness.query_by_label_contains("Zurück zu").is_none());
    }

    /// The Windows setup's `#updates`: shown only when the Updater says so;
    /// the switch and the button ask it, the status line reads as setup.js's.
    #[test]
    fn the_update_section_shows_where_the_app_updates_itself() {
        use crate::platform::update::{SetupAsk, SetupView, show_setup, take_ask};
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1280.0, 800.0))
            .build_eframe(|cc| Launcher::setup(cc, None, None, Box::new(|_| {})));
        harness.run();
        assert!(harness.query_by_label("Updates").is_none());
        show_setup(
            &harness.ctx,
            Some(SetupView {
                current: "0.9.0".into(),
                auto_check: true,
                checked: true,
                ..SetupView::default()
            }),
        );
        harness.run();
        let label = "Beim Start und alle 6 Stunden nach Updates suchen";
        let check = harness.get_by_role_and_label(Role::CheckBox, label);
        assert_eq!(
            check.accesskit_node().toggled(),
            Some(egui::accesskit::Toggled::True)
        );
        check.click();
        harness.run();
        assert_eq!(take_ask(&harness.ctx), Some(SetupAsk::AutoCheck(false)));
        harness
            .get_by_role_and_label(Role::Button, "Jetzt nach Updates suchen")
            .click();
        harness.run();
        assert_eq!(take_ask(&harness.ctx), Some(SetupAsk::CheckNow));
        let status = harness.get_by_label("Installiert: 0.9.0. Das ist die neueste Version.");
        assert_eq!(status.accesskit_node().role(), Role::Status);
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
        assert!(server_origin("file:///etc/passwd").is_err());
        assert!(server_origin("chat.example.org").is_err());
        assert_eq!(host_of("http://127.0.0.1:5180"), "127.0.0.1:5180");
        assert_eq!(host_of("https://chat.example.org"), "chat.example.org");
    }
}
