//! User settings (web pages/UserSettingsPage.tsx, settingsAreas.ts).

use egui::Ui;

use crate::app::AppCtx;

/// The settings navigation in the sidebar column (`ui.max_rect()`).
pub fn sidebar(_ui: &mut Ui, _app: &mut AppCtx, _area: Option<&str>) {}

/// The settings page: header and body, drawn into `ui.max_rect()`.
pub fn page(_ui: &mut Ui, _app: &mut AppCtx, _area: Option<&str>) {}
