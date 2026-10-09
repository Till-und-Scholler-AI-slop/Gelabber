//! Gelabber desktop: a Tauri window on the Gelabber server's own origin
//! (cookie login, CSRF and signaling stay as in the browser) plus the native
//! media core behind a narrow set of commands.
//!
//! Server choice, first match wins: `--server <url>`, `GELABBER_SERVER`,
//! `server` in `<config dir>/desktop.json`. Without one the window shows the
//! bundled setup page, which stores the choice and reloads onto the server.
//! There is no menu bar: an unreachable server at start opens that page with
//! the reason, and Ctrl+Shift+S or the web client's user menu ("Server
//! wechseln …") lead back to it at any time, so a wrong server never locks the
//! app. F5 / Ctrl+R reload. Those keys are a script of the page: when WebKit's
//! web process dies, the app loads the page again itself (Linux).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod frames;
mod media;
mod viewer;

use serde::{Deserialize, Serialize};
#[cfg(target_os = "linux")]
use std::time::Instant;
use std::{
    fs,
    net::{SocketAddr, TcpStream},
    path::PathBuf,
    time::Duration,
};
use tauri::{
    AppHandle, Manager, WebviewUrl, WebviewWindowBuilder, ipc::CapabilityBuilder,
    webview::PageLoadEvent,
};
use url::Url;

const WINDOW: &str = "main";
/// Runs in every page of the window, the server's and the setup page. Plain
/// shortcuts instead of a menu bar: reload, and the way back to the setup
/// page. WebKit's own error page runs no scripts; the start-up reachability
/// check covers the usual way into it.
const SHORTCUTS: &str = r#"
addEventListener("keydown", (event) => {
  const key = event.key.toLowerCase();
  const command = event.ctrlKey || event.metaKey;
  if (event.key === "F5" || (command && !event.shiftKey && key === "r")) {
    event.preventDefault();
    location.reload();
  } else if (command && event.shiftKey && key === "s") {
    event.preventDefault();
    window.__TAURI_INTERNALS__?.invoke("open_setup").catch(() => {});
  }
}, true);
"#;
/// WebView2 (Windows) arguments. Setting any replaces wry's defaults, so
/// those are repeated: the three `ms*` features off (no Edge mini menus, no
/// SmartScreen) and autoplay (call sounds without a click). The rest keeps
/// the page's timers running while the window is minimized or covered: the
/// gateway heartbeat and the signaling live in the page, and unlike in a
/// browser nothing there (no RTCPeerConnection, no playing audio: media is
/// in the native core) exempts it from Chromium's background throttling.
const WEBVIEW2_ARGS: &str = concat!(
    "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection,IntensiveWakeUpThrottling",
    " --autoplay-policy=no-user-gesture-required",
    " --disable-background-timer-throttling",
    " --disable-renderer-backgrounding",
    " --disable-backgrounding-occluded-windows",
);
/// Per address; a typo'd host fails at DNS long before that.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

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
    let url = Url::parse(input.trim()).map_err(|e| format!("Ungültige Serveradresse: {e}"))?;
    if !matches!(url.scheme(), "https" | "http") || url.host().is_none() {
        return Err("Die Serveradresse braucht http:// oder https:// und einen Host.".into());
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

/// The server's pages may call the media commands and open the setup page,
/// nothing else.
fn allow_server(app: &AppHandle, origin: &Url) -> tauri::Result<()> {
    let origin = origin.origin().ascii_serialization();
    app.add_capability(
        CapabilityBuilder::new("server-media")
            .remote(origin)
            .local(false)
            .window(WINDOW)
            .permission("media")
            .permission("allow-open-setup"),
    )
}

/// The setup page, with the current server and the reason it is shown
/// filled in when there are.
fn setup_url(server: Option<&str>, error: Option<&str>) -> Url {
    // Where Tauri serves the bundled frontend.
    let base = if cfg!(windows) {
        "http://tauri.localhost/index.html"
    } else {
        "tauri://localhost/index.html"
    };
    let mut url = Url::parse(base).expect("static URL");
    if let Some(server) = server {
        url.query_pairs_mut().append_pair("server", server);
    }
    if let Some(error) = error {
        url.query_pairs_mut().append_pair("error", error);
    }
    url
}

/// Whether `url` is a page of the bundled frontend, the setup page.
#[cfg(target_os = "linux")]
fn is_setup_page(url: &str) -> bool {
    let setup = setup_url(None, None);
    Url::parse(url)
        .is_ok_and(|url| url.scheme() == setup.scheme() && url.host_str() == setup.host_str())
}

fn current_server(app: &AppHandle) -> Option<Url> {
    configured_server(app).and_then(|value| server_origin(&value).ok())
}

/// "Server wechseln": the setup page with the current server filled in.
#[tauri::command]
fn open_setup(app: AppHandle) -> Result<(), String> {
    let server = current_server(&app);
    app.get_webview_window(WINDOW)
        .ok_or("main window missing")?
        .navigate(setup_url(server.as_ref().map(Url::as_str), None))
        .map_err(|e| e.to_string())
}

/// Catches typos and wrong ports before the address is stored: something has
/// to accept a connection there.
fn check_reachable(origin: &Url) -> Result<(), String> {
    let host = origin.host_str().ok_or("Serveradresse ohne Host")?;
    let port = origin
        .port_or_known_default()
        .ok_or("Serveradresse ohne Port")?;
    let unreachable = |reason: String| format!("{host}:{port} ist nicht erreichbar ({reason}).");
    let addrs = server_addrs(origin).map_err(|e| unreachable(e.to_string()))?;
    let mut last = "keine Adresse".to_owned();
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT) {
            Ok(_) => return Ok(()),
            Err(e) => last = e.to_string(),
        }
    }
    Err(unreachable(last))
}

/// The origin's socket addresses: DNS for names, none for IP literals.
fn server_addrs(origin: &Url) -> std::io::Result<Vec<SocketAddr>> {
    // `host_str()` keeps IPv6 brackets ("[::1]"), which the resolver would
    // send to DNS; `socket_addrs` uses the typed host instead.
    origin.socket_addrs(|| None)
}

/// Setup page only: check and store the server, then open it.
#[tauri::command]
async fn set_server(app: AppHandle, server: String) -> Result<(), String> {
    let origin = server_origin(&server)?;
    let probe = origin.clone();
    tauri::async_runtime::spawn_blocking(move || check_reachable(&probe))
        .await
        .map_err(|e| e.to_string())??;
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

#[cfg(target_os = "linux")]
const WEBKIT_DISABLE_DMABUF: &str = "WEBKIT_DISABLE_DMABUF_RENDERER";
#[cfg(target_os = "linux")]
const WEBKIT_FORCE_SHM: &str = "WEBKIT_DMABUF_RENDERER_FORCE_SHM";

/// The WebKitGTK variable to set on the NVIDIA driver. Its DMA-BUF renderer
/// dies there under Wayland ("Error 71 (Protocol error) dispatching to
/// Wayland display") when it hands buffers over as DMA-BUFs. Handing them
/// over through shared memory works and keeps accelerated compositing, which
/// video on a canvas needs: with the renderer disabled, the web process
/// paints every frame on the CPU, half a core and more for one stream across
/// the window. An explicit setting of either variable by the user wins.
#[cfg(target_os = "linux")]
fn webkit_workaround(nvidia: bool, set_by_user: impl Fn(&str) -> bool) -> Option<&'static str> {
    (nvidia && !set_by_user(WEBKIT_DISABLE_DMABUF) && !set_by_user(WEBKIT_FORCE_SHM))
        .then_some(WEBKIT_FORCE_SHM)
}

/// Whether the machine runs the NVIDIA driver.
#[cfg(target_os = "linux")]
fn nvidia_driver() -> bool {
    std::path::Path::new("/proc/driver/nvidia/version").exists()
        || std::path::Path::new("/sys/module/nvidia_drm").exists()
}

#[cfg(target_os = "linux")]
fn webkit_workarounds() {
    let nvidia = nvidia_driver();
    if let Some(name) = webkit_workaround(nvidia, |name| std::env::var_os(name).is_some()) {
        // SAFETY: called first thing in main, before any other thread exists.
        unsafe { std::env::set_var(name, "1") };
    }
}

/// Ends WebKit's web process the moment the window is closed, on the NVIDIA
/// driver. Left to shut down by itself it crashes once a page has drawn with
/// WebGL, which the video tiles do: when its connection to the app closes,
/// WebKitGTK frees its GL contexts, and `eglDestroyContext` then reads
/// through a null pointer inside libnvidia-eglcore (seen with driver
/// 610.57.04 and WebKitGTK 2.52.6, in shared-memory mode and with the
/// DMA-BUF renderer disabled alike; leaving the page first does not help).
/// Nothing was lost by that crash, but every close of the app left a core
/// dump and, on desktops that announce them, a crash notification. A web
/// process that is killed frees nothing.
///
/// Only a window that is closed gets here. An app that is killed itself
/// still leaves the web process to that shutdown.
#[cfg(target_os = "linux")]
fn end_web_process(window: &tauri::Window) {
    use webkit2gtk::WebViewExt;
    let Some(window) = window.get_webview_window(window.label()) else {
        return;
    };
    // The event comes on the main thread, where this runs at once: before
    // the window and its webview are destroyed.
    if let Err(error) = window.with_webview(|webview| webview.inner().terminate_web_process()) {
        eprintln!("gelabber: web process: {error}");
    }
}

/// A web process that ends again within this time of the app's last answer
/// to one ending did not get better by that answer.
#[cfg(target_os = "linux")]
const ENDED_AGAIN: Duration = Duration::from_secs(60);

/// What the setup page says when it is shown for that reason.
#[cfg(target_os = "linux")]
const PAGE_KEEPS_ENDING: &str =
    "Die Seite ist wiederholt abgestürzt. Mikrofon, Kamera und Bildschirmfreigabe wurden beendet.";

/// The app's answer to a web process that ended by itself.
#[cfg(target_os = "linux")]
#[derive(Debug, PartialEq)]
enum Recovery {
    /// Loads the page again, which starts a new web process.
    Reload,
    /// Shows the setup page with the reason: loading the server's page once
    /// more may end the same way, for as long as nobody looks.
    Setup,
    /// Leaves the dead view: not even the setup page stays up.
    GiveUp,
}

/// `shown` is the page the web process ended with, `again` whether that was
/// within [`ENDED_AGAIN`] of the last time.
#[cfg(target_os = "linux")]
fn recovery(shown: Option<&str>, again: bool) -> Recovery {
    match (shown.map(is_setup_page), again) {
        (Some(_), false) => Recovery::Reload,
        (Some(true), true) => Recovery::GiveUp,
        // The server's page ended twice, or nothing was loaded to load again.
        (Some(false), true) | (None, _) => Recovery::Setup,
    }
}

/// Answers the end of WebKit's web process: a crash, the kernel's
/// out-of-memory killer, a GPU driver (the page draws video with WebGL). No
/// page load follows by itself, so nothing ended what the page had running:
/// microphone, camera and screen capture went on behind a dead view, and the
/// keys that reload or lead to the setup page are a script of the page that
/// is gone. Windows is left to WebView2, which by its documentation starts a
/// new renderer and loads an error page, a page load like any other; that
/// was not tried.
#[cfg(target_os = "linux")]
fn watch_web_process(app: &AppHandle, window: &tauri::WebviewWindow) {
    use webkit2gtk::{WebProcessTerminationReason, WebViewExt};
    let app = app.clone();
    let watching = window.with_webview(move |webview| {
        let last = std::cell::Cell::new(None::<Instant>);
        let ended = move |view: &webkit2gtk::WebView, reason| {
            // The app's own doing, when its window closes (`end_web_process`).
            if reason == WebProcessTerminationReason::TerminatedByApi {
                return;
            }
            app.state::<media::Media>().reset();
            let now = Instant::now();
            let again = last
                .replace(Some(now))
                .is_some_and(|last| now.duration_since(last) < ENDED_AGAIN);
            let action = recovery(view.uri().as_deref(), again);
            eprintln!("gelabber: web process ended ({reason:?}): {action:?}");
            // On the view itself: this runs inside WebKit's signal, on the
            // main thread.
            match action {
                Recovery::Reload => view.reload(),
                Recovery::Setup => {
                    let server = current_server(&app);
                    let server = server.as_ref().map(Url::as_str);
                    view.load_uri(setup_url(server, Some(PAGE_KEEPS_ENDING)).as_str());
                }
                Recovery::GiveUp => {}
            }
        };
        webview.inner().connect_web_process_terminated(ended);
    });
    if let Err(error) = watching {
        eprintln!("gelabber: web process: {error}");
    }
}

fn main() {
    #[cfg(target_os = "linux")]
    webkit_workarounds();
    // Native media logging (libwebrtc, libmediasoupclient) to stderr.
    if std::env::var_os("GELABBER_MEDIA_LOG").is_some() {
        gelabber_media_core::set_log_level(gelabber_media_core::LogLevel::Info);
    }
    let builder = tauri::Builder::default();
    #[cfg(target_os = "linux")]
    let builder = builder.on_window_event(|window, event| {
        if matches!(event, tauri::WindowEvent::CloseRequested { .. }) && nvidia_driver() {
            end_web_process(window);
        }
    });
    builder
        .manage(media::Media::default())
        .invoke_handler(tauri::generate_handler![
            set_server,
            open_setup,
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
            media::media_view_open,
            media::media_view_configure,
            media::media_view_frame,
            media::media_view_close,
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
            let window = WebviewWindowBuilder::new(app, WINDOW, url.clone())
                .title("Gelabber")
                .inner_size(1280.0, 800.0)
                .min_inner_size(480.0, 360.0)
                .initialization_script(SHORTCUTS)
                // Only WebView2 takes them; ignored on the other platforms.
                .additional_browser_args(WEBVIEW2_ARGS)
                .build()?;
            #[cfg(target_os = "linux")]
            watch_web_process(&handle, &window);
            #[cfg(not(target_os = "linux"))]
            let _ = window;
            // Opening a stored server that does not answer would end on
            // WebKit's error page; show the setup page with the reason instead.
            if let WebviewUrl::External(origin) = url {
                tauri::async_runtime::spawn(async move {
                    let probe = origin.clone();
                    let reachable =
                        tauri::async_runtime::spawn_blocking(move || check_reachable(&probe)).await;
                    if let Ok(Err(reason)) = reachable
                        && let Some(window) = handle.get_webview_window(WINDOW)
                    {
                        let setup = setup_url(Some(origin.as_str()), Some(reason.as_str()));
                        if let Err(error) = window.navigate(setup) {
                            eprintln!("gelabber: setup page: {error}");
                        }
                    }
                });
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Gelabber");
}

#[cfg(test)]
mod tests {
    use super::{check_reachable, server_addrs, server_origin, setup_url};
    use std::net::TcpListener;

    #[cfg(target_os = "linux")]
    #[test]
    fn nvidia_gets_the_shared_memory_renderer_unless_the_user_chose() {
        use super::{WEBKIT_DISABLE_DMABUF, WEBKIT_FORCE_SHM, webkit_workaround};
        let chose = |chosen: &'static [&'static str]| move |name: &str| chosen.contains(&name);
        assert_eq!(webkit_workaround(true, chose(&[])), Some(WEBKIT_FORCE_SHM));
        assert_eq!(webkit_workaround(false, chose(&[])), None);
        assert_eq!(
            webkit_workaround(true, chose(&[WEBKIT_DISABLE_DMABUF])),
            None
        );
        assert_eq!(webkit_workaround(true, chose(&[WEBKIT_FORCE_SHM])), None);
    }

    /// One reload; the setup page when that did not help; nothing when not
    /// even the setup page stays up, or the app would start web processes
    /// for as long as they end.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_web_process_that_ended_is_answered_once_by_a_reload() {
        use super::{Recovery, is_setup_page, recovery};
        let server = "https://chat.example.org/channels/7";
        let setup = setup_url(Some("https://chat.example.org/"), Some("abgestürzt"));
        assert!(is_setup_page(setup.as_str()));
        assert!(is_setup_page("tauri://localhost/"));
        assert!(!is_setup_page(server));
        assert!(!is_setup_page("https://localhost/index.html"));
        assert!(!is_setup_page("about:blank"));

        assert_eq!(recovery(Some(server), false), Recovery::Reload);
        assert_eq!(recovery(Some(server), true), Recovery::Setup);
        assert_eq!(recovery(Some(setup.as_str()), false), Recovery::Reload);
        assert_eq!(recovery(Some(setup.as_str()), true), Recovery::GiveUp);
        assert_eq!(recovery(None, false), Recovery::Setup);
        assert_eq!(recovery(None, true), Recovery::Setup);
    }

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

    #[test]
    fn setup_url_carries_the_current_server() {
        let url = setup_url(Some("https://chat.example.org/"), None);
        assert_eq!(url.path(), "/index.html");
        assert_eq!(
            url.query_pairs().collect::<Vec<_>>(),
            [("server".into(), "https://chat.example.org/".into())]
        );
        assert_eq!(setup_url(None, None).query(), None);
        let failed = setup_url(Some("https://chat.example.org/"), Some("nicht erreichbar"));
        assert_eq!(
            failed.query_pairs().collect::<Vec<_>>(),
            [
                ("server".into(), "https://chat.example.org/".into()),
                ("error".into(), "nicht erreichbar".into())
            ]
        );
    }

    #[test]
    fn check_reachable_needs_a_listener() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = server_origin(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
        assert!(check_reachable(&origin).is_ok());
        drop(listener);
        assert!(check_reachable(&origin).is_err());
        assert!(check_reachable(&server_origin("https://gelabber.invalid").unwrap()).is_err());
    }

    #[test]
    fn server_addrs_take_ip_literals_without_dns() {
        let v6 = server_origin("http://[::1]:8080").unwrap();
        assert_eq!(v6.host_str(), Some("[::1]"));
        assert_eq!(server_addrs(&v6).unwrap(), ["[::1]:8080".parse().unwrap()]);
        let v4 = server_origin("https://127.0.0.1").unwrap();
        assert_eq!(
            server_addrs(&v4).unwrap(),
            ["127.0.0.1:443".parse().unwrap()]
        );
    }

    #[test]
    fn check_reachable_takes_ipv6_literals() {
        // Hosts without IPv6 loopback cannot run this case.
        let Ok(listener) = TcpListener::bind("[::1]:0") else {
            return;
        };
        let origin = server_origin(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
        assert_eq!(origin.host_str(), Some("[::1]"));
        assert_eq!(check_reachable(&origin), Ok(()));
    }
}
