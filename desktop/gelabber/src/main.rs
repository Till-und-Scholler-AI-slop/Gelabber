//! Gelabber, the native desktop app. The server comes from `--server`,
//! `GELABBER_SERVER` or `server` in `desktop.json` of the Tauri app's config
//! directory (first match wins), else the setup screen asks. The desktop
//! around the window (sign-in kept, notifications, tray, one instance,
//! push-to-talk, updates) is `gelabber_ui::platform`.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;
use gelabber_ui::platform::{self, Platform, instance, settings, state};
use gelabber_ui::{GelabberApp, Launcher};
use tracing_subscriber::EnvFilter;

/// The launcher and the desktop around it.
struct Desktop {
    launcher: Launcher,
    platform: Platform,
}

impl eframe::App for Desktop {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.platform.logic(ctx, self.launcher.app());
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        eframe::App::ui(&mut self.launcher, ui, frame);
        let app: Option<&GelabberApp> = self.launcher.app().map(|a| &*a);
        self.platform.paint(ui, app);
    }

    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        eframe::App::clear_color(&self.launcher, visuals)
    }

    fn on_exit(&mut self) {
        self.platform.on_exit();
    }
}

fn remember(server: &str) {
    if let Err(e) = settings::set_server(server) {
        eprintln!("gelabber: saving the server: {e}");
    }
}

fn main() -> eframe::Result {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")),
        )
        .init();
    // A second start shows the running app and ends before it opens a window.
    let instance = match instance::claim() {
        instance::Claim::First(instance) => instance,
        instance::Claim::Running => return Ok(()),
    };
    let server = settings::configured_server(
        std::env::args().skip(1),
        std::env::var("GELABBER_SERVER").ok(),
    );
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Gelabber")
        .with_app_id(platform::APP_ID)
        // The Tauri window's sizes (desktop/app/src/main.rs).
        .with_inner_size(state::DEFAULT_SIZE)
        .with_min_inner_size(state::MIN_SIZE);
    if let Some(window) = state::window() {
        viewport = viewport
            .with_inner_size([window.width, window.height])
            .with_maximized(window.maximized);
        if let (Some(x), Some(y)) = (window.x, window.y) {
            viewport = viewport.with_position([x, y]);
        }
    }
    if let Some(icon) = gelabber_ui::window_icon() {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    let platform = Platform::new(Some(instance));
    eframe::run_native(
        "Gelabber",
        options,
        Box::new(move |cc| {
            let mut platform = platform;
            platform.set_window(cc);
            let prepare = platform.prepare_hook();
            let launcher = Launcher::with_prepare(cc, server, Box::new(remember), Some(prepare));
            Ok(Box::new(Desktop { launcher, platform }))
        }),
    )
}
