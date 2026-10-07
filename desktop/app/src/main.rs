//! Gelabber desktop: a Tauri window on the Gelabber server's own origin
//! (cookie login, CSRF and signaling stay as in the browser) plus the native
//! media core behind a narrow set of commands.
//!
//! Server choice, first match wins: `--server <url>`, `GELABBER_SERVER`,
//! `server` in `<config dir>/desktop.json`. Without one the window shows the
//! bundled setup page, which stores the choice and reloads onto the server.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod media;
mod viewer;

use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};
use tauri::{
    AppHandle, Manager, WebviewUrl, WebviewWindowBuilder, ipc::CapabilityBuilder,
    webview::PageLoadEvent,
};
use url::Url;

const WINDOW: &str = "main";

#[derive(Default, Serialize, Deserialize)]
struct Settings {
    server: Option<String>,
}

fn settings_path(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|dir| dir.join("desktop.json"))
}

fn load_settings(app: &AppHandle) -> Settings {
    settings_path(app)
        .and_then(|path| fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// Only http(s) origins; the window loads the server root.
fn server_origin(input: &str) -> Result<Url, String> {
    let url = Url::parse(input.trim()).map_err(|e| format!("invalid server address: {e}"))?;
    if !matches!(url.scheme(), "https" | "http") || url.host().is_none() {
        return Err("the server address needs http:// or https:// and a host".into());
    }
    Url::parse(&url.origin().ascii_serialization()).map_err(|e| e.to_string())
}

fn configured_server(app: &AppHandle) -> Option<String> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--server" {
            return args.next();
        }
        if let Some(value) = arg.strip_prefix("--server=") {
            return Some(value.to_owned());
        }
    }
    std::env::var("GELABBER_SERVER")
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| load_settings(app).server)
}

/// The server's pages may call the media commands, nothing else.
fn allow_server(app: &AppHandle, origin: &Url) -> tauri::Result<()> {
    let origin = origin.origin().ascii_serialization();
    app.add_capability(
        CapabilityBuilder::new("server-media")
            .remote(origin)
            .local(false)
            .window(WINDOW)
            .permission("media"),
    )
}

/// Setup page only: store the server and open it.
#[tauri::command]
fn set_server(app: AppHandle, server: String) -> Result<(), String> {
    let origin = server_origin(&server)?;
    if let Some(path) = settings_path(&app) {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let settings = Settings {
            server: Some(origin.to_string()),
        };
        let json = serde_json::to_vec_pretty(&settings).map_err(|e| e.to_string())?;
        fs::write(path, json).map_err(|e| e.to_string())?;
    }
    allow_server(&app, &origin).map_err(|e| e.to_string())?;
    let window = app
        .get_webview_window(WINDOW)
        .ok_or("main window missing")?;
    window.navigate(origin).map_err(|e| e.to_string())
}

fn main() {
    // Native media logging (libwebrtc, libmediasoupclient) to stderr.
    if std::env::var_os("GELABBER_MEDIA_LOG").is_some() {
        gelabber_media_core::set_log_level(gelabber_media_core::LogLevel::Info);
    }
    tauri::Builder::default()
        .manage(media::Media::default())
        .invoke_handler(tauri::generate_handler![
            set_server,
            media::media_info,
            media::media_audio_devices,
            media::media_audio_configure,
            media::media_audio_levels,
            media::media_audio_monitor,
            media::media_device_load,
            media::media_device_close,
            media::media_transport_create,
            media::media_transport_respond,
            media::media_transport_restart_ice,
            media::media_transport_stats,
            media::media_transport_close,
            media::media_source_microphone,
            media::media_source_screen,
            media::media_video_devices,
            media::media_audio_apps,
            media::media_source_app_audio,
            media::media_source_camera,
            media::media_source_state,
            media::media_source_set_enabled,
            media::media_source_close,
            media::media_produce,
            media::media_producer_pause,
            media::media_producer_replace_source,
            media::media_producer_parameters,
            media::media_producer_set_parameters,
            media::media_producer_stats,
            media::media_producer_close,
            media::media_consume,
            media::media_consumer_pause,
            media::media_consumer_set_volume,
            media::media_consumer_stats,
            media::media_consumer_close,
            media::media_viewer_open,
            media::media_viewer_close,
        ])
        .on_page_load(|webview, payload| {
            // A reload or navigation leaves the old page's calls and
            // transports orphaned; close them before the new page starts.
            if payload.event() == PageLoadEvent::Started {
                webview.state::<media::Media>().reset();
            }
        })
        .setup(|app| {
            let handle = app.handle().clone();
            let server = configured_server(&handle).map(|value| server_origin(&value));
            let url = match server {
                Some(Ok(origin)) => {
                    allow_server(&handle, &origin)?;
                    WebviewUrl::External(origin)
                }
                Some(Err(error)) => {
                    eprintln!("gelabber: {error}; showing the setup page");
                    WebviewUrl::App("index.html".into())
                }
                None => WebviewUrl::App("index.html".into()),
            };
            WebviewWindowBuilder::new(app, WINDOW, url)
                .title("Gelabber")
                .inner_size(1280.0, 800.0)
                .min_inner_size(480.0, 360.0)
                .build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Gelabber");
}

#[cfg(test)]
mod tests {
    use super::server_origin;

    #[test]
    fn server_origin_keeps_only_the_origin() {
        assert_eq!(
            server_origin(" https://chat.example.org/app?x=1 ")
                .unwrap()
                .as_str(),
            "https://chat.example.org/"
        );
        assert_eq!(
            server_origin("http://localhost:5173").unwrap().as_str(),
            "http://localhost:5173/"
        );
        assert!(server_origin("file:///etc/passwd").is_err());
        assert!(server_origin("javascript:alert(1)").is_err());
        assert!(server_origin("chat.example.org").is_err());
    }
}
