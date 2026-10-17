//! Notification settings of an account (issue #242): per server and per
//! channel (a DM is a channel) a level and a mute. The unread rows carry the
//! effective result (`messages/workflows.rs`), so a client filters badges and
//! notifications without asking again.

pub mod settings;

use axum::Router;

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    settings::router()
}
