//! Gelabber, the native desktop app. The server comes from `--server`,
//! `GELABBER_SERVER` or `server` in `desktop.json` of the Tauri app's config
//! directory (first match wins), else the setup screen asks. The desktop
//! around the window (sign-in kept, notifications, tray, one instance,
//! push-to-talk, updates) is `gelabber_ui::platform`. An update accepted
//! in the window installs once it has closed (`update::run_pending`).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod smoke;

use std::io::Write;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use eframe::egui;
use gelabber_ui::Launcher;
use gelabber_ui::platform::{self, Platform, aumid, instance, legacy, settings, state, update};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::prelude::*;

/// A log that grew past this at start becomes `gelabber.log.1`.
const LOG_MAX: u64 = 1 << 20;

/// The log file, for lines that do not go through `tracing` (the panic
/// hook, the start's own notes).
static LOG: OnceLock<Mutex<std::fs::File>> = OnceLock::new();

/// Opens `path` for appending; one that grew past `max` is first moved to
/// `<path>.1`, replacing the one before.
fn open_log(path: &Path, max: u64) -> std::io::Result<std::fs::File> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    if std::fs::metadata(path).is_ok_and(|m| m.len() > max) {
        let mut old = path.as_os_str().to_owned();
        old.push(".1");
        std::fs::rename(path, old)?;
    }
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
}

/// A line for stderr and the log file: a release build on Windows has no
/// console, so the file is the only trace.
fn note(line: &str) {
    eprintln!("{line}");
    if let Some(file) = LOG.get()
        && let Ok(mut file) = file.lock()
    {
        let _ = writeln!(file, "{line}");
    }
}

/// `tracing` to stderr and to `gelabber.log` in the config directory, and
/// a panic hook that writes the panic with its backtrace there too.
fn init_logging() {
    let file =
        platform::paths::file("gelabber.log").and_then(|path| match open_log(&path, LOG_MAX) {
            Ok(file) => Some(file),
            Err(e) => {
                eprintln!("gelabber: log file {}: {e}", path.display());
                None
            }
        });
    let layer = file.as_ref().and_then(|f| f.try_clone().ok()).map(|f| {
        tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_writer(Mutex::new(f))
    });
    if let Some(file) = file {
        let _ = LOG.set(Mutex::new(file));
    }
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")))
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .with(layer)
        .init();
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        let thread = std::thread::current();
        let at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        if let Some(file) = LOG.get()
            && let Ok(mut file) = file.lock()
        {
            let name = thread.name().unwrap_or("?");
            let _ = writeln!(
                file,
                "panic at unix {at} in thread {name}: {info}\n{backtrace}"
            );
        }
        default(info);
    }));
}

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
        note(&format!("gelabber: saving the server: {e}"));
    }
}

fn main() -> eframe::Result {
    init_logging();
    // Before any window: taskbar grouping and toasts go by the shortcuts' id.
    if let Err(e) = aumid::set_for_process() {
        note(&format!("gelabber: {e}"));
    }
    // A second start shows the running app and ends before it opens a window.
    let instance = match instance::claim() {
        instance::Claim::First(instance) => instance,
        instance::Claim::Running => return Ok(()),
    };
    // Before anything reads the settings: the Tauri app's, on the first start.
    match legacy::import() {
        Ok(Some(imported)) => note(&format!("gelabber: took over {imported}")),
        Ok(None) => {}
        Err(e) => note(&format!(
            "gelabber: taking over the Tauri app's settings: {e}"
        )),
    }
    let smoke = smoke::Smoke::from_env(std::env::var("GELABBER_SMOKE").ok().as_deref());
    let server = settings::configured_server(
        std::env::args().skip(1),
        std::env::var("GELABBER_SERVER").ok(),
    );
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Gelabber")
        .with_app_id(platform::APP_ID)
        // The Tauri window's sizes.
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
    if let Err(e) = &result {
        note(&format!("gelabber: the window did not start: {e}"));
    }
    if let Err(e) = update::run_pending() {
        note(&format!("gelabber: update: {e}"));
    }
    if let Some(code) = smoke::exit_code() {
        std::process::exit(code);
    }
    result
}

#[cfg(test)]
mod tests {
    use gelabber_ui::platform::update::{pubkey, verify};

    /// A log past its size at start moves aside; a small one is appended to.
    #[test]
    fn the_log_rotates_once_it_is_too_large() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("gelabber-log-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("gelabber.log");
        writeln!(super::open_log(&path, 8).unwrap(), "first").unwrap();
        writeln!(super::open_log(&path, 8).unwrap(), "second").unwrap();
        // 13 bytes now: the next start moves them to .1.
        writeln!(super::open_log(&path, 8).unwrap(), "third").unwrap();
        let read = |p: &std::path::Path| std::fs::read_to_string(p).unwrap();
        assert_eq!(read(&path), "third\n");
        assert_eq!(read(&dir.join("gelabber.log.1")), "first\nsecond\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

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
