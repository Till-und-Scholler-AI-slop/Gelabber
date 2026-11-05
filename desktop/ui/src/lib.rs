//! Gelabber's native desktop interface in egui. It mirrors the web client
//! (web/src) screen by screen; `desktop/parity` compares both.

pub mod fonts;
pub mod route;

use gelabber_client::Client;

pub struct GelabberApp {
    client: Client,
    route: String,
}

impl GelabberApp {
    pub fn new(cc: &eframe::CreationContext<'_>, client: Client, route: Option<String>) -> Self {
        fonts::install(&cc.egui_ctx);
        let ctx = cc.egui_ctx.clone();
        client.set_repaint(std::sync::Arc::new(move || ctx.request_repaint()));
        Self {
            client,
            route: route.unwrap_or_else(|| "/".to_owned()),
        }
    }

    /// True while data or images are still loading.
    pub fn busy(&self) -> bool {
        self.client.busy()
    }
}

impl eframe::App for GelabberApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.label(format!("Gelabber – {}", self.route));
        });
    }
}
