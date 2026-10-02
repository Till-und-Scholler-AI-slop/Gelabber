//! Private, revision-checked account themes. Theme data never contains executable CSS.
use crate::{AppState, auth::session::CurrentUser, error::ApiError, json::Body};
use axum::{
    Router,
    extract::{DefaultBodyLimit, State},
    response::Json,
    routing::get,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;

pub const BUILTINS: &[&str] = &[
    "dark",
    "light",
    "system",
    "catppuccin",
    "catppuccin-latte",
    "nord",
    "gruvbox",
    "tokyo-night",
    "everforest",
    "kanagawa",
    "rose-pine",
    "flexoki-light",
    "osaka-jade",
    "matte-black",
    "ristretto",
];

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Theme {
    version: u8,
    id: String,
    name: String,
    mode: String,
    style: String,
    colors: Colors,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Colors {
    background: String,
    panel: String,
    surface: String,
    rail: String,
    text: String,
    muted: String,
    accent: String,
    border: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ThemeDocument {
    version: u8,
    revision: i64,
    active: String,
    custom_themes: Vec<Theme>,
}
impl Default for ThemeDocument {
    fn default() -> Self {
        Self {
            version: 1,
            revision: 0,
            active: "dark".into(),
            custom_themes: vec![],
        }
    }
}
fn invalid() -> ApiError {
    ApiError::BadRequest("Invalid theme document".into())
}
impl ThemeDocument {
    fn validate(&self) -> Result<(), ApiError> {
        if self.version != 1
            || self.revision < 0
            || self.revision >= 9_007_199_254_740_991
            || self.custom_themes.len() > 50
        {
            return Err(invalid());
        }
        let mut ids = HashSet::new();
        for t in &self.custom_themes {
            let id =
                t.id.strip_prefix("custom-")
                    .and_then(|s| uuid::Uuid::parse_str(s).ok());
            if t.version != 1
                || id.is_none()
                || t.id.len() != 43
                || !ids.insert(t.id.as_str())
                || t.name.trim().is_empty()
                || t.name.chars().count() > 60
                || t.name.chars().any(char::is_control)
                || !["light", "dark"].contains(&t.mode.as_str())
                || !["clear", "soft", "terminal"].contains(&t.style.as_str())
            {
                return Err(invalid());
            }
            let c = &t.colors;
            for value in [
                &c.background,
                &c.panel,
                &c.surface,
                &c.rail,
                &c.text,
                &c.muted,
                &c.accent,
                &c.border,
            ] {
                if value.len() != 7
                    || !value.starts_with('#')
                    || !value.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
                {
                    return Err(invalid());
                }
            }
        }
        if !BUILTINS.contains(&self.active.as_str()) && !ids.contains(self.active.as_str()) {
            return Err(invalid());
        }
        Ok(())
    }
}
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/me/themes", get(read).put(write))
        .layer(DefaultBodyLimit::max(64 * 1024))
}
async fn read(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<ThemeDocument>, ApiError> {
    let row: Option<(i64, Value)> =
        sqlx::query_as("SELECT revision, document FROM account_themes WHERE user_id = $1")
            .bind(user.id)
            .fetch_optional(&state.db)
            .await?;
    let mut doc = match row {
        Some((revision, value)) => {
            let mut d: ThemeDocument =
                serde_json::from_value(value).map_err(|e| ApiError::Internal(e.to_string()))?;
            d.revision = revision;
            d
        }
        None => ThemeDocument::default(),
    };
    // Keep the database revision authoritative.
    doc.version = 1;
    Ok(Json(doc))
}
async fn write(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Body(mut doc): Body<ThemeDocument>,
) -> Result<Json<ThemeDocument>, ApiError> {
    doc.validate()?;
    let previous = doc.revision;
    doc.revision += 1;
    let value = serde_json::to_value(&doc).map_err(|e| ApiError::Internal(e.to_string()))?;
    // Creation and updates each atomically compare the caller's revision.
    let revision: Option<i64> = if previous == 0 {
        sqlx::query_scalar("INSERT INTO account_themes (user_id, revision, document) VALUES ($1, 1, $2) ON CONFLICT (user_id) DO NOTHING RETURNING revision")
            .bind(user.id).bind(value).fetch_optional(&state.db).await?
    } else {
        sqlx::query_scalar("UPDATE account_themes SET revision = revision + 1, document = $2, updated_at = now() WHERE user_id = $1 AND revision = $3 RETURNING revision")
            .bind(user.id).bind(value).bind(previous).fetch_optional(&state.db).await?
    };
    doc.revision = revision.ok_or(ApiError::ThemeConflict)?;
    Ok(Json(doc))
}
