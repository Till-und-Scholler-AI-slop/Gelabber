//! Gelabber, the native desktop app. The server comes from `--server`,
//! `GELABBER_SERVER` or the last one chosen on the setup screen, like the
//! Tauri app (desktop/app/src/main.rs `configured_server`).

use std::path::PathBuf;

use eframe::egui;
use tracing_subscriber::EnvFilter;

fn server_arg() -> Option<String> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--server" {
            return args.next();
        }
        if let Some(value) = arg.strip_prefix("--server=") {
            return Some(value.to_owned());
        }
    }
    None
}

/// Where the chosen server is remembered, per platform.
fn settings_file() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    }?;
    Some(base.join("gelabber").join("server"))
}

fn saved_server() -> Option<String> {
    let text = std::fs::read_to_string(settings_file()?).ok()?;
    let server = text.trim();
    (!server.is_empty()).then(|| server.to_owned())
}

fn remember(server: &str) {
    let Some(path) = settings_file() else { return };
    if let Some(dir) = path.parent()
        && let Err(e) = std::fs::create_dir_all(dir)
    {
        eprintln!("gelabber: settings directory {}: {e}", dir.display());
        return;
    }
    if let Err(e) = std::fs::write(&path, format!("{server}\n")) {
        eprintln!("gelabber: saving the server in {}: {e}", path.display());
    }
}

fn main() -> eframe::Result {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")),
        )
        .init();
    let server = server_arg()
        .or_else(|| {
            std::env::var("GELABBER_SERVER")
                .ok()
                .filter(|v| !v.is_empty())
        })
        .or_else(saved_server);
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Gelabber")
        .with_app_id("gelabber")
        // The Tauri window's sizes (desktop/app/src/main.rs).
        .with_inner_size([1280.0, 800.0])
        .with_min_inner_size([480.0, 360.0]);
    if let Some(icon) = gelabber_ui::window_icon() {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(
        "Gelabber",
        options,
        Box::new(move |cc| {
            Ok(Box::new(gelabber_ui::Launcher::new(
                cc,
                server,
                Box::new(remember),
            )))
        }),
    )
}
