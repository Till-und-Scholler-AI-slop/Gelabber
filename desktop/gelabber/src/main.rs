//! Gelabber, the native desktop app. The server comes from `--server`,
//! `GELABBER_SERVER` or `server` in `desktop.json` of the Tauri app's config
//! directory (first match wins), else the setup screen asks. The desktop
//! around the window (sign-in kept, notifications, tray, one instance,
//! push-to-talk, updates) is `gelabber_ui::platform`. An update accepted
//! in the window installs once it has closed (`update::run_pending`).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod smoke;

use eframe::egui;
use gelabber_ui::Launcher;
use gelabber_ui::platform::{self, Platform, aumid, instance, legacy, settings, state, update};
use tracing_subscriber::EnvFilter;

/// The launcher and the desktop around it.
struct Desktop {
    launcher: Launcher,
    platform: Platform,
    smoke: Option<smoke::Smoke>,
}

impl Desktop {
    /// `GELABBER_SMOKE`: the verdict once the time is up, then the window closes.
    fn smoke(&mut self, ctx: &egui::Context) {
        let Some(test) = &mut self.smoke else { return };
        // Nothing else wakes a window that gets no input (Xvfb).
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
        let now = ctx.input(|i| i.time);
        if test.overdue(now) {
            std::process::exit(smoke::exit_code().unwrap_or(1));
        }
        let screen = match self.launcher.app() {
            None => "setup".to_owned(),
            Some(app) => format!("{:?}", app.nav().route()),
        };
        if test.check(now, &screen).is_some() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

impl eframe::App for Desktop {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.platform.logic(ctx, self.launcher.app());
        self.smoke(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        if let Some(test) = &mut self.smoke {
            test.frame();
        }
        eframe::App::ui(&mut self.launcher, ui, frame);
        self.platform.paint(ui.ctx());
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
    // Before any window: taskbar grouping and toasts go by the shortcuts' id.
    if let Err(e) = aumid::set_for_process() {
        eprintln!("gelabber: {e}");
    }
    // A second start shows the running app and ends before it opens a window.
    let instance = match instance::claim() {
        instance::Claim::First(instance) => instance,
        instance::Claim::Running => return Ok(()),
    };
    // Before anything reads the settings: the Tauri app's, on the first start.
    match legacy::import() {
        Ok(Some(imported)) => eprintln!("gelabber: took over {imported}"),
        Ok(None) => {}
        Err(e) => eprintln!("gelabber: taking over the Tauri app's settings: {e}"),
    }
    let smoke = smoke::Smoke::from_env(std::env::var("GELABBER_SMOKE").ok().as_deref());
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
    let result = eframe::run_native(
        "Gelabber",
        options,
        Box::new(move |cc| {
            let mut platform = platform;
            platform.set_window(cc);
            let prepare = platform.prepare_hook();
            let launcher = Launcher::with_prepare(cc, server, Box::new(remember), Some(prepare));
            Ok(Box::new(Desktop {
                launcher,
                platform,
                smoke,
            }))
        }),
    );
    // The app has ended (instance and files released): the setup replaces it
    // and starts the new version.
    if let Err(e) = update::run_pending() {
        eprintln!("gelabber: update: {e}");
    }
    if let Some(code) = smoke::exit_code() {
        std::process::exit(code);
    }
    result
}

#[cfg(test)]
mod tests {
    use gelabber_ui::platform::update::{pubkey, verify};

    /// The native installer CI built and signed (desktop-native.yml sets the
    /// variable): its signature holds for the key this build carries and for
    /// this version, as the installed app checks it. Nothing without it.
    #[test]
    fn the_installer_built_here_passes_the_updaters_check() {
        let installer = std::env::var_os("GELABBER_NATIVE_UPDATE_TEST_INSTALLER");
        let Some(installer) = installer.filter(|path| !path.is_empty()) else {
            return;
        };
        assert!(!pubkey().trim().is_empty(), "this build has no updater key");
        let mut bytes = std::fs::read(&installer).unwrap();
        let mut path = installer;
        path.push(".sig");
        let signature = std::fs::read_to_string(path).unwrap();
        let version = env!("CARGO_PKG_VERSION");
        assert_eq!(verify(&bytes, &signature, pubkey(), version), Ok(()));
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        assert!(verify(&bytes, &signature, pubkey(), version).is_err());
    }
}
