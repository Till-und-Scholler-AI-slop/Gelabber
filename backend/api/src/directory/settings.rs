//! `/api/servers/{id}/listing`: the owner's listing form. Every member may
//! read it; only the owner saves it. A row exists once the owner saved the
//! form; until then the defaults come back with `language: null`.
//!
//! While the operator has the server delisted, a save still goes through
//! and the listing stays hidden (`delisted: true`). The operator's reason is
//! not shown here.

use axum::extract::State;
use axum::response::Json;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgConnection;
use tracing::info;
use uuid::Uuid;

use super::require_enabled;
use super::validate::{self, JoinMode};
use crate::auth::session::CurrentUser;
use crate::auth::validate::finish;
use crate::error::{ApiError, FieldErrors};
use crate::json::Body;
use crate::path::Id;
use crate::servers::membership;
use crate::state::AppState;

pub fn router() -> axum::Router<AppState> {
    use axum::routing::get;
    axum::Router::new().route("/api/servers/{id}/listing", get(read).put(save))
}

/// The form as the server's members see it.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ListingSettings {
    pub listed: bool,
    pub description: String,
    /// `null` until the owner first saved the form.
    pub language: Option<String>,
    pub join_mode: String,
    /// The operator took the server out of the directory.
    pub delisted: bool,
    /// The first time the owner listed it.
    pub listed_at: Option<DateTime<Utc>>,
}

/// The form of an existing server, with the defaults when it has no row.
async fn load(conn: &mut PgConnection, server_id: Uuid) -> Result<ListingSettings, ApiError> {
    sqlx::query_as(
        "SELECT COALESCE(l.listed, false) AS listed, \
                COALESCE(l.description, '') AS description, \
                l.language, \
                COALESCE(l.join_mode, $2) AS join_mode, \
                s.delisted_at IS NOT NULL AS delisted, \
                l.listed_at \
         FROM servers s LEFT JOIN server_listings l ON l.server_id = s.id \
         WHERE s.id = $1",
    )
    .bind(server_id)
    .bind(JoinMode::Request.as_str())
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(ApiError::NotFound)
}

/// `GET /api/servers/{id}/listing` for any member.
async fn read(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(server_id): Id,
) -> Result<Json<ListingSettings>, ApiError> {
    require_enabled(&state)?;
    membership::load(&state.db, server_id, user.id).await?;
    let mut conn = state.db.acquire().await?;
    Ok(Json(load(&mut conn, server_id).await?))
}

/// All four fields are required. Options, so that a missing one is a field
/// error instead of a `400`.
#[derive(Debug, Deserialize)]
pub struct SaveBody {
    #[serde(default)]
    listed: Option<bool>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    join_mode: Option<String>,
}

/// `PUT /api/servers/{id}/listing` for the owner: saves the form and answers
/// it as [`read`] does. Unlisting keeps the row and the pending requests,
/// and so does switching the join mode.
async fn save(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(server_id): Id,
    Body(body): Body<SaveBody>,
) -> Result<Json<ListingSettings>, ApiError> {
    require_enabled(&state)?;
    membership::load(&state.db, server_id, user.id)
        .await?
        .require_owner()?;

    let mut errors = FieldErrors::new();
    if body.listed.is_none() {
        errors.insert("listed", "required");
    }
    let listed = body.listed.unwrap_or(false);
    let description = validate::description(body.description.as_deref(), listed, &mut errors);
    let language = validate::language(body.language.as_deref(), &mut errors);
    let join_mode = validate::join_mode(body.join_mode.as_deref(), &mut errors);
    finish(errors)?;
    let (description, language, join_mode) = (
        description.expect("validated"),
        language.expect("validated"),
        join_mode.expect("validated"),
    );

    let mut tx = state.db.begin().await?;
    sqlx::query(
        "INSERT INTO server_listings \
             (server_id, listed, description, language, join_mode, listed_at) \
         VALUES ($1, $2, $3, $4, $5, CASE WHEN $2 THEN now() END) \
         ON CONFLICT (server_id) DO UPDATE SET \
             listed = EXCLUDED.listed, \
             description = EXCLUDED.description, \
             language = EXCLUDED.language, \
             join_mode = EXCLUDED.join_mode, \
             listed_at = COALESCE(server_listings.listed_at, EXCLUDED.listed_at), \
             updated_at = now()",
    )
    .bind(server_id)
    .bind(listed)
    .bind(&description)
    .bind(language)
    .bind(join_mode.as_str())
    .execute(&mut *tx)
    .await
    .map_err(|err| match &err {
        // The server was deleted after the membership check.
        sqlx::Error::Database(db) if db.code().as_deref() == Some("23503") => ApiError::NotFound,
        _ => err.into(),
    })?;
    let settings = load(&mut tx, server_id).await?;
    tx.commit().await?;
    info!(
        server_id = %server_id,
        listed,
        language,
        join_mode = join_mode.as_str(),
        "listing saved"
    );
    Ok(Json(settings))
}
