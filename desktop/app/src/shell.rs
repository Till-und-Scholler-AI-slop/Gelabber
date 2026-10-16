//! The window outside the page: unread state on the taskbar button, the
//! launcher and the tray icon, a flash for a new direct message, the way back
//! to the window from a notification, and the tray with its menu.
//!
//! The web client works out what to show (`web/src/messages/unreadIndicators.ts`)
//! and hands it over with `shell_unread`; nothing here knows about messages.

use std::sync::Mutex;
#[cfg(target_os = "linux")]
use std::sync::OnceLock;

use serde::Serialize;
use tauri::{
    AppHandle, Manager, Runtime, UserAttentionType, WebviewWindow,
    image::Image,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent},
};

use crate::{WINDOW, load_settings, save_settings};

const TRAY: &str = "main";
const MENU_OPEN: &str = "open";
const MENU_QUIT: &str = "quit";

/// Red of the unread dot, the web client's badge red.
const ALERT: [u8; 4] = [0xe5, 0x48, 0x4d, 0xff];
/// Ordinary unread messages: a light dot, like the web client's rail.
const UNREAD: [u8; 4] = [0xee, 0xf0, 0xee, 0xff];

/// What the page last reported.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Unread {
    /// The number for badges; 0 shows none.
    pub count: u32,
    /// Something is unread at all, counted or not.
    pub any: bool,
}

impl Unread {
    fn dot(self) -> Option<[u8; 4]> {
        if self.count > 0 {
            Some(ALERT)
        } else if self.any {
            Some(UNREAD)
        } else {
            None
        }
    }
}

#[derive(Default)]
pub struct Shell {
    tray: Mutex<Option<TrayIcon>>,
    unread: Mutex<Unread>,
}

impl Shell {
    pub fn has_tray(&self) -> bool {
        self.tray.lock().is_ok_and(|tray| tray.is_some())
    }
}

/// The app icon with a dot in its lower right corner: the colour, with a ring
/// of transparency that sets it off from the icon.
pub fn with_dot(rgba: &[u8], width: u32, height: u32, color: [u8; 4]) -> Vec<u8> {
    let mut out = rgba.to_vec();
    let size = width.min(height) as f32;
    let radius = size * 0.24;
    let ring = (size * 0.06).max(1.0);
    let cx = width as f32 - radius - ring;
    let cy = height as f32 - radius - ring;
    for y in 0..height {
        for x in 0..width {
            let dx = x as f32 + 0.5 - cx;
            let dy = y as f32 + 0.5 - cy;
            let distance = (dx * dx + dy * dy).sqrt();
            let at = ((y * width + x) * 4) as usize;
            if distance <= radius {
                out[at..at + 4].copy_from_slice(&color);
            } else if distance <= radius + ring {
                out[at + 3] = 0;
            }
        }
    }
    out
}

fn dotted(base: &Image<'_>, color: Option<[u8; 4]>) -> Image<'static> {
    match color {
        Some(color) => Image::new_owned(
            with_dot(base.rgba(), base.width(), base.height(), color),
            base.width(),
            base.height(),
        ),
        None => base.clone().to_owned(),
    }
}

/// A plain red dot for the taskbar button's overlay (Windows).
#[cfg(windows)]
fn overlay() -> Image<'static> {
    const SIZE: u32 = 32;
    let blank = vec![0; (SIZE * SIZE * 4) as usize];
    let mut dot = with_dot(&blank, SIZE, SIZE, ALERT);
    // Only the dot: no ring needed on a transparent square, and it fills it.
    for pixel in dot.chunks_mut(4) {
        if pixel[3] != 0 {
            pixel.copy_from_slice(&ALERT);
        }
    }
    Image::new_owned(dot, SIZE, SIZE)
}

fn main_window<R: Runtime>(app: &AppHandle<R>) -> Option<WebviewWindow<R>> {
    app.get_webview_window(WINDOW)
}

/// Brings the window back: from the tray, the taskbar or a notification.
pub fn show<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = main_window(app) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// Whether a tray host can be reached at all. Without the AppIndicator
/// library tray-icon panics when the icon is built (Linux); plain GNOME has
/// none installed.
#[cfg(target_os = "linux")]
fn tray_supported() -> bool {
    static SUPPORTED: OnceLock<bool> = OnceLock::new();
    *SUPPORTED.get_or_init(|| {
        [
            "libayatana-appindicator3.so.1",
            "libappindicator3.so.1",
            "libayatana-appindicator3.so",
            "libappindicator3.so",
        ]
        .iter()
        // SAFETY: the libraries tray-icon would load itself; their
        // initialisers run either way.
        .any(|name| unsafe { libloading::Library::new(name) }.is_ok())
    })
}

#[cfg(not(target_os = "linux"))]
fn tray_supported() -> bool {
    true
}

/// The tray icon with "Gelabber öffnen" and "Beenden". Nothing when the
/// desktop has no tray: the app runs as before.
pub fn build_tray(app: &AppHandle) {
    if !tray_supported() {
        eprintln!("gelabber: no AppIndicator library, running without a tray icon");
        return;
    }
    let Some(icon) = app.default_window_icon().cloned() else {
        return;
    };
    let built = (|| {
        let open = MenuItem::with_id(app, MENU_OPEN, "Gelabber öffnen", true, None::<&str>)?;
        let end = MenuItem::with_id(app, MENU_QUIT, "Beenden", true, None::<&str>)?;
        let menu = Menu::with_items(app, &[&open, &end])?;
        TrayIconBuilder::with_id(TRAY)
            .icon(icon.to_owned())
            .tooltip("Gelabber")
            .menu(&menu)
            .show_menu_on_left_click(false)
            .on_menu_event(|app, event| match event.id().as_ref() {
                MENU_OPEN => show(app),
                MENU_QUIT => quit(app),
                _ => {}
            })
            .on_tray_icon_event(|tray, event| {
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } = event
                {
                    show(tray.app_handle());
                }
            })
            .build(app)
    })();
    match built {
        Ok(tray) => {
            if let Ok(mut slot) = app.state::<Shell>().tray.lock() {
                *slot = Some(tray);
            }
        }
        Err(error) => eprintln!("gelabber: tray icon: {error}"),
    }
}

/// "Beenden" in the tray: the app ends, a running call with it.
fn quit(app: &AppHandle) {
    #[cfg(target_os = "linux")]
    if crate::nvidia_driver()
        && let Some(window) = main_window(app)
    {
        crate::end_web_process(&window.as_ref().window());
    }
    app.exit(0);
}

/// Whether closing the window hides it in the tray instead of ending the app.
pub fn closes_to_tray(app: &AppHandle) -> bool {
    load_settings(app).close_to_tray && app.state::<Shell>().has_tray()
}

#[derive(Serialize)]
pub struct ShellInfo {
    tray: bool,
    close_to_tray: bool,
}

/// What the settings page offers: the tray option only with a tray.
#[tauri::command]
pub fn shell_info(app: AppHandle) -> ShellInfo {
    ShellInfo {
        tray: app.state::<Shell>().has_tray(),
        close_to_tray: load_settings(&app).close_to_tray,
    }
}

#[tauri::command]
pub fn shell_close_to_tray(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut settings = load_settings(&app);
    settings.close_to_tray = enabled;
    save_settings(&app, &settings)
}

/// The window in front, from a click on a notification the page showed.
#[tauri::command]
pub fn shell_focus(app: AppHandle) {
    show(&app);
}

/// Unread state from the page. `attention` asks once for the user's eye: a
/// direct message arrived while the window is not in front. Windows flashes
/// the taskbar button until the window gets the focus, Linux marks it urgent.
#[tauri::command]
pub fn shell_unread(app: AppHandle, count: u32, any: bool, attention: bool) -> Result<(), String> {
    let next = Unread { count, any };
    let shell = app.state::<Shell>();
    let previous = {
        let mut unread = shell.unread.lock().map_err(|e| e.to_string())?;
        std::mem::replace(&mut *unread, next)
    };
    let window = main_window(&app).ok_or("main window missing")?;
    if attention && !window.is_focused().unwrap_or(true) {
        window
            .request_user_attention(Some(UserAttentionType::Informational))
            .map_err(|e| e.to_string())?;
    }
    if previous == next {
        return Ok(());
    }
    if previous.count != next.count {
        badge(&window, next.count)?;
    }
    if previous.dot() != next.dot()
        && let Ok(tray) = shell.tray.lock()
        && let Some(tray) = tray.as_ref()
        && let Some(icon) = app.default_window_icon()
    {
        tray.set_icon(Some(dotted(icon, next.dot())))
            .map_err(|e| e.to_string())?;
    }
    if let Ok(tray) = shell.tray.lock()
        && let Some(tray) = tray.as_ref()
    {
        let tooltip = match next.count {
            0 if next.any => "Gelabber: ungelesene Nachrichten".to_owned(),
            0 => "Gelabber".to_owned(),
            1 => "Gelabber: 1 neue Nachricht".to_owned(),
            n => format!("Gelabber: {n} neue Nachrichten"),
        };
        // Not shown on Linux; a failure there is no reason to fail the rest.
        let _ = tray.set_tooltip(Some(tooltip));
    }
    Ok(())
}

/// The number on the launcher (Linux, where the desktop shows one) or a red
/// dot on the taskbar button (Windows, which has no number there).
fn badge(window: &WebviewWindow, count: u32) -> Result<(), String> {
    #[cfg(windows)]
    let result = window.set_overlay_icon((count > 0).then(overlay));
    #[cfg(not(windows))]
    let result = window.set_badge_count((count > 0).then_some(i64::from(count)));
    result.map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::{ALERT, UNREAD, Unread, with_dot};
    // commands.rs, which media.rs includes for its tests.
    use crate::media::SHELL_COMMANDS;

    /// The server origin's `shell` set grants exactly these commands.
    #[test]
    fn shell_permission_set_matches_the_commands() {
        let set = include_str!("../permissions/shell.toml");
        let granted: Vec<&str> = set
            .lines()
            .filter_map(|line| line.trim().strip_prefix("\"allow-"))
            .filter_map(|rest| rest.strip_suffix("\","))
            .collect();
        let expected: Vec<String> = SHELL_COMMANDS.iter().map(|c| c.replace('_', "-")).collect();
        assert_eq!(granted, expected);
        let main = include_str!("main.rs");
        for command in SHELL_COMMANDS {
            assert!(
                main.contains(&format!("shell::{command},")),
                "{command} missing from generate_handler!"
            );
        }
    }

    #[test]
    fn the_dot_sits_in_the_lower_right_corner_with_a_clear_ring() {
        let (w, h) = (20, 20);
        let base = vec![10; (w * h * 4) as usize];
        let out = with_dot(&base, w, h, ALERT);
        let at = |x: u32, y: u32| &out[((y * w + x) * 4) as usize..((y * w + x) * 4 + 4) as usize];
        // Centre of the dot.
        assert_eq!(at(14, 14), ALERT);
        // Untouched icon in the opposite corner.
        assert_eq!(at(1, 1), [10, 10, 10, 10]);
        // The ring between dot and icon is transparent.
        assert!((0..w).any(|x| at(x, 14)[3] == 0));
        assert_eq!(out.len(), base.len());
    }

    #[test]
    fn counted_messages_are_red_others_light() {
        assert_eq!(
            Unread {
                count: 2,
                any: true
            }
            .dot(),
            Some(ALERT)
        );
        assert_eq!(
            Unread {
                count: 0,
                any: true
            }
            .dot(),
            Some(UNREAD)
        );
        assert_eq!(Unread::default().dot(), None);
    }
}
