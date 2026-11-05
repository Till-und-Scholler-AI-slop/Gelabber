//! The first screen of the desktop app when no server is configured (the
//! Tauri app's setup page, desktop/app/setup): asks for the server address,
//! then hands the window over to [`GelabberApp`].

use egui::{Rect, Stroke, StrokeKind, Ui, UiBuilder, Vec2};
use gelabber_client::Client;

use crate::app::GelabberApp;
use crate::text::{self, Block};
use crate::theme::{self, Theme, corner, space};
use crate::widgets::{Button, FormError, TextField};

/// Called with the origin once a server was chosen, to remember it.
pub type Remember = Box<dyn Fn(&str)>;

enum Stage {
    Setup {
        address: String,
        error: Option<String>,
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
            },
            remember,
        };
        if let Some(server) = server {
            launcher.connect(&cc.egui_ctx, &server);
        }
        launcher
    }

    fn connect(&mut self, ctx: &egui::Context, input: &str) {
        let result = server_origin(input).and_then(|origin| {
            Client::new(&origin)
                .map(|client| (origin, client))
                .map_err(|e| e.to_string())
        });
        match result {
            Ok((origin, client)) => {
                (self.remember)(&origin);
                self.stage = Stage::App(Box::new(GelabberApp::with_context(ctx, client, None)));
            }
            Err(message) => {
                self.stage = Stage::Setup {
                    address: input.to_owned(),
                    error: Some(message),
                };
            }
        }
    }
}

/// `server_origin` of the Tauri app: http(s) with a host, reduced to its
/// origin.
pub fn server_origin(input: &str) -> Result<String, String> {
    let input = input.trim();
    let (scheme, rest) = input.split_once("://").ok_or_else(|| {
        "Die Serveradresse braucht http:// oder https:// und einen Host.".to_owned()
    })?;
    let scheme = scheme.to_ascii_lowercase();
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    if !matches!(scheme.as_str(), "http" | "https") || host.is_empty() || host.contains('@') {
        return Err("Die Serveradresse braucht http:// oder https:// und einen Host.".into());
    }
    Ok(format!("{scheme}://{}", host.to_ascii_lowercase()))
}

impl eframe::App for Launcher {
    fn ui(&mut self, ui: &mut Ui, frame: &mut eframe::Frame) {
        let mut connect = None;
        match &mut self.stage {
            Stage::App(app) => {
                eframe::App::ui(app.as_mut(), ui, frame);
                return;
            }
            Stage::Setup { address, error } => {
                if setup(ui, address, error.as_deref()) {
                    connect = Some(address.clone());
                }
            }
        }
        if let Some(address) = connect {
            let ctx = ui.ctx().clone();
            self.connect(&ctx, &address);
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        egui::Rgba::from(Theme::dark().c.bg).to_array()
    }
}

/// The setup card; true when "Verbinden" was pressed.
fn setup(ui: &mut Ui, address: &mut String, error: Option<&str>) -> bool {
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
                .add(Button::new("Verbinden").primary().large().fill())
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
        assert!(server_origin("file:///etc/passwd").is_err());
        assert!(server_origin("chat.example.org").is_err());
    }
}
