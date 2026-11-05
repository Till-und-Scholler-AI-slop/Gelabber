//! Text channels and DMs: the page header and the message pane with its
//! composer (web components/MessagePane.tsx, pages/ChannelPage.tsx).

use egui::Ui;
use uuid::Uuid;

use crate::app::AppCtx;

/// A server text channel: header and body, drawn into `ui.max_rect()`.
pub fn channel_page(_ui: &mut Ui, _app: &mut AppCtx, _server: Uuid, _channel: Uuid) {}

/// A DM conversation: header and body, drawn into `ui.max_rect()`.
pub fn dm_page(_ui: &mut Ui, _app: &mut AppCtx, _channel: Uuid) {}
