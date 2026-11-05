//! The first screen of the desktop app when no server is configured (the
//! Tauri app's setup page, desktop/app/setup): asks for the server address,
//! then hands the window over to [`GelabberApp`].

use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;

use egui::{Rect, Stroke, StrokeKind, Ui, UiBuilder, Vec2};
use gelabber_client::Client;
use parking_lot::Mutex;

use crate::app::GelabberApp;
use crate::text::{self, Block};
use crate::theme::{self, Theme, corner, space};
use crate::widgets::{Button, FormError, TextField};

/// Called with the origin once a server was chosen, to remember it.
pub type Remember = Box<dyn Fn(&str)>;

/// `CONNECT_TIMEOUT` of the Tauri app's reachability check.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// The reachability check of a typed address, on its own thread.
type Probe = Arc<Mutex<Option<Result<String, String>>>>;

enum Stage {
    Setup {
        address: String,
        error: Option<String>,
        probe: Option<Probe>,
    },
    App(Box<GelabberApp>),
}

/// The eframe app: the setup screen, then Gelabber.
pub struct Launcher {
    stage: Stage,
    remember: Remember,
}

impl Launcher {
    /// With `server`, starts Gelabber right away; otherwise asks first.
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        server: Option<String>,
        remember: Remember,
    ) -> Self {
        crate::fonts::install(&cc.egui_ctx);
        theme::set(&cc.egui_ctx, Theme::dark());
        let mut launcher = Self {
            stage: Stage::Setup {
                address: server.clone().unwrap_or_default(),
                error: None,
                probe: None,
            },
            remember,
        };
        if let Some(server) = server {
            match server_origin(&server) {
                Ok(origin) => launcher.open(&cc.egui_ctx, &origin, false),
                Err(message) => launcher.fail(&server, message),
            }
        }
        launcher
    }

    /// Like the Tauri app's `set_server`: only an address that accepts a
    /// connection is stored, so a typo does not stick across restarts.
    fn check(&mut self, ctx: &egui::Context, input: &str) {
        let origin = match server_origin(input) {
            Ok(origin) => origin,
            Err(message) => return self.fail(input, message),
        };
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
        match spawned {
            Ok(_) => {
                self.stage = Stage::Setup {
                    address: input.to_owned(),
                    error: None,
                    probe: Some(probe),
                }
            }
            Err(e) => self.fail(input, e.to_string()),
        }
    }

    fn open(&mut self, ctx: &egui::Context, origin: &str, remember: bool) {
        match Client::new(origin) {
            Ok(client) => {
                if remember {
                    (self.remember)(origin);
                }
                self.stage = Stage::App(Box::new(GelabberApp::with_context(ctx, client, None)));
            }
            Err(e) => self.fail(origin, e.to_string()),
        }
    }

    fn fail(&mut self, input: &str, message: String) {
        self.stage = Stage::Setup {
            address: input.to_owned(),
            error: Some(message),
            probe: None,
        };
    }
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
        let (submit, checked) = match &mut self.stage {
            Stage::App(app) => {
                eframe::App::ui(app.as_mut(), ui, frame);
                return;
            }
            Stage::Setup {
                address,
                error,
                probe,
            } => {
                let checked = probe.as_ref().and_then(|p| p.lock().take());
                let checking = probe.is_some() && checked.is_none();
                let submit = setup(ui, address, error.as_deref(), checking) && !checking;
                (submit.then(|| address.clone()), checked)
            }
        };
        match checked {
            Some(Ok(origin)) => self.open(&ctx, &origin, true),
            Some(Err(message)) => {
                let address = match &self.stage {
                    Stage::Setup { address, .. } => address.clone(),
                    Stage::App(_) => String::new(),
                };
                self.fail(&address, message);
            }
            None => {
                if let Some(address) = submit {
                    self.check(&ctx, &address);
                }
            }
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        egui::Rgba::from(Theme::dark().c.bg).to_array()
    }
}

/// The setup card; true when "Verbinden" was pressed.
fn setup(ui: &mut Ui, address: &mut String, error: Option<&str>, checking: bool) -> bool {
    let ctx = ui.ctx().clone();
    let theme = Theme::of(&ctx);
    let c = theme.c;
    let screen = ctx.content_rect();
    ui.painter().rect_filled(screen, 0.0, c.bg);
    let width = (screen.width() - 32.0).min(384.0);
    let height_id = egui::Id::unique("gelabber-setup-height");
    let height: f32 = ctx.data(|d| d.get_temp(height_id)).unwrap_or(300.0);
    let card = Rect::from_center_size(screen.center(), Vec2::new(width, height));
    let background = ui.painter().add(egui::Shape::Noop);
    let inner = Rect::from_min_size(
        card.min + Vec2::splat(33.0),
        Vec2::new(width - 66.0, f32::INFINITY),
    );
    let (pressed, used) = ui
        .scope_builder(UiBuilder::new().max_rect(inner), |ui| {
            let title = Block::wrapped(
                ui,
                "Gelabber",
                text::PAGE.tracking(-0.03),
                c.text,
                inner.width(),
            );
            crate::screens::auth::layout::show_block(ui, &title);
            crate::screens::auth::layout::lead(
                ui,
                "Mit welchem Gelabber-Server soll sich die App verbinden?",
            );
            ui.add_space(space::S6);
            let field = TextField::new("server", address)
                .label("Serveradresse")
                .placeholder("https://gelabber.example.org")
                .autofocus(true)
                .show(ui);
            ui.add_space(space::S5);
            if let Some(error) = error {
                ui.add(FormError::new(error).on(c.panel));
                ui.add_space(space::S5);
            }
            let pressed = ui
                .add(
                    Button::new("Verbinden")
                        .primary()
                        .large()
                        .fill()
                        .disabled(checking),
                )
                .clicked()
                || field.submitted;
            (pressed, ui.min_rect().height())
        })
        .inner;
    let measured = used + 66.0;
    ui.painter().set(
        background,
        egui::epaint::RectShape::new(
            Rect::from_min_size(card.min, Vec2::new(width, measured)),
            corner(theme.radius.lg),
            c.panel,
            Stroke::new(1.0, c.border),
            StrokeKind::Inside,
        ),
    );
    if (measured - height).abs() > 0.5 {
        ctx.data_mut(|d| d.insert_temp(height_id, measured));
        ctx.request_discard("setup height");
    }
    pressed
}

#[cfg(test)]
mod tests {
    use super::server_origin;

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
    }
}
