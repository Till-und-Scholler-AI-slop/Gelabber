//! The server directory "Entdecken" (v0.10, #520). Off unless the operator
//! sets `API_DIRECTORY`; while it is off, `GET /api/directory` says
//! `enabled: false` and the listing routes answer `404`. Signed-in only: there is no
//! anonymous route.
//!
//! An owner lists a server ([`settings`]). A listing is visible to a caller
//! while it is `listed`, the operator neither delisted
//! (`servers.delisted_at`) nor blocked (`servers.blocked_at`) the server, and
//! the caller is not banned from it (`visible_listing_where!`).

pub mod languages;
pub mod settings;
pub mod validate;

use std::collections::HashMap;

use axum::Router;
use axum::extract::State;
use axum::response::Json;
use axum::routing::get;
use serde::Serialize;

use crate::auth::session::CurrentUser;
use crate::error::ApiError;
use crate::state::AppState;

use self::languages::LANGUAGES;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/directory", get(status))
        .merge(settings::router())
}

/// The visibility rule over `server_listings l` joined with `servers s`,
/// for the caller `$1`; a macro so the queries stay `&'static str`.
macro_rules! visible_listing_where {
    () => {
        "l.listed AND s.delisted_at IS NULL AND s.blocked_at IS NULL \
         AND NOT EXISTS (SELECT 1 FROM server_bans b \
                          WHERE b.server_id = s.id AND b.user_id = $1)"
    };
}

/// `404` while the directory is off.
pub(crate) fn require_enabled(state: &AppState) -> Result<(), ApiError> {
    if state.directory {
        Ok(())
    } else {
        Err(ApiError::NotFound)
    }
}

#[derive(Debug, Serialize)]
pub struct Language {
    pub code: &'static str,
    pub name: &'static str,
    /// Listings of this language the caller can see.
    pub listings: i64,
}

#[derive(Debug, Serialize)]
pub struct Status {
    pub enabled: bool,
    /// Every language a listing can declare; empty while off.
    pub languages: Vec<Language>,
}

/// `GET /api/directory`: whether the directory is on, and the languages
/// with the number of listings the caller can see in each.
async fn status(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<Status>, ApiError> {
    if !state.directory {
        return Ok(Json(Status {
            enabled: false,
            languages: Vec::new(),
        }));
    }
    let counts: HashMap<String, i64> = sqlx::query_as::<_, (String, i64)>(concat!(
        "SELECT l.language, COUNT(*) FROM server_listings l \
         JOIN servers s ON s.id = l.server_id WHERE ",
        visible_listing_where!(),
        " GROUP BY l.language"
    ))
    .bind(user.id)
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .collect();
    let languages = LANGUAGES
        .iter()
        .map(|(code, name)| Language {
            code,
            name,
            listings: counts.get(*code).copied().unwrap_or(0),
        })
        .collect();
    Ok(Json(Status {
        enabled: true,
        languages,
    }))
}
