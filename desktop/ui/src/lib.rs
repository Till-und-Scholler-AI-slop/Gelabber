//! Gelabber's native desktop interface in egui. It mirrors the web client
//! (web/src) screen by screen; `desktop/parity` compares both.

pub mod app;
pub mod fonts;
pub mod icons;
mod icons_generated;
pub mod images;
pub mod initials;
pub mod platform;
pub mod route;
pub mod rules;
pub mod screens;
pub mod setup;
pub mod style;
pub mod task;
pub mod text;
pub mod theme;
pub mod toasts;
pub mod widgets;

pub use app::{AppCtx, GelabberApp, Location, Nav};
pub use setup::Launcher;
pub use theme::Theme;

/// The app icon (assets/images/icon-192.png) for the window and taskbar.
pub fn window_icon() -> Option<egui::IconData> {
    let image = image::load_from_memory(include_bytes!("../assets/images/icon-192.png"))
        .ok()?
        .to_rgba8();
    Some(egui::IconData {
        width: image.width(),
        height: image.height(),
        rgba: image.into_raw(),
    })
}
