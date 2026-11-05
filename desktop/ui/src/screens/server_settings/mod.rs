//! Server settings (web pages/ServerSettingsPage.tsx).

use egui::Ui;
use uuid::Uuid;

use crate::app::AppCtx;

/// The server settings navigation in the sidebar column, if the web has one.
pub fn sidebar(_ui: &mut Ui, _app: &mut AppCtx, _server: Uuid) {}

/// The server settings page: header and body, drawn into `ui.max_rect()`.
pub fn page(_ui: &mut Ui, _app: &mut AppCtx, _server: Uuid) {}
