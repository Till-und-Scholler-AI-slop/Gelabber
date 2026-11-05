//! `/api/me`: read and edit the signed-in user's own profile (name, avatar,
//! profile card). `/api/users/{id}`: someone else's profile card.

use axum::Router;
use axum::extract::State;
use axum::response::Json;
use axum::routing::get;
use serde::Deserialize;

use crate::auth::session::CurrentUser;
use crate::auth::user::{self, ProfilePatch, PublicProfile, User};
use crate::auth::validate;
use crate::error::{ApiError, FieldErrors};
use crate::json::Body;
use crate::path::Id;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/me", get(me).patch(update_me))
        .route("/api/users/{id}", get(profile_of))
}

/// Fields are optional; a missing field is left untouched. An empty string
/// clears an optional field (`avatar_url: ""` clears the avatar).
#[derive(Debug, Default, Deserialize)]
pub struct UpdateMeBody {
    pub name: Option<String>,
    pub avatar_url: Option<String>,
    pub bio: Option<String>,
    pub pronouns: Option<String>,
    pub status_text: Option<String>,
    pub accent_color: Option<String>,
    pub banner_color: Option<String>,
    /// Only `""` (remove) or the current value: banners are set by
    /// `PUT /api/me/banner`.
    pub banner_url: Option<String>,
    pub avatar_decoration: Option<String>,
}

async fn me(CurrentUser(user): CurrentUser) -> Json<User> {
    Json(user)
}

async fn update_me(
    State(state): State<AppState>,
    CurrentUser(current): CurrentUser,
    Body(body): Body<UpdateMeBody>,
) -> Result<Json<User>, ApiError> {
    let mut errors = FieldErrors::new();
    let mut patch = ProfilePatch {
        name: body.name.and_then(|raw| validate::name(&raw, &mut errors)),
        avatar_url: body
            .avatar_url
            .and_then(|raw| validate::avatar_url(&raw, &mut errors)),
        bio: body.bio.and_then(|raw| validate::bio(&raw, &mut errors)),
        pronouns: body
            .pronouns
            .and_then(|raw| validate::pronouns(&raw, &mut errors)),
        status_text: body
            .status_text
            .and_then(|raw| validate::status_text(&raw, &mut errors)),
        accent_color: body
            .accent_color
            .and_then(|raw| validate::color("accent_color", &raw, &mut errors)),
        banner_color: body
            .banner_color
            .and_then(|raw| validate::color("banner_color", &raw, &mut errors)),
        banner_url: None,
        avatar_decoration: body
            .avatar_decoration
            .and_then(|raw| validate::avatar_decoration(&raw, &mut errors)),
    };
    // Uploads are set by `PUT /api/me/avatar` only. A form may echo the
    // current one back, which changes nothing; any other upload path (a
    // stale tab's earlier one, already deleted, or someone else's) would
    // point the profile at a missing object and delete the current one.
    if let Some(Some(url)) = &patch.avatar_url
        && crate::avatars::parse_url(url).is_some()
    {
        if current.avatar_url.as_deref() == Some(url.as_str()) {
            patch.avatar_url = None;
        } else {
            errors.insert("avatar_url", "stale");
        }
    }
    // Same rule for banners, which have no link form at all.
    if let Some(raw) = body.banner_url {
        let value = raw.trim();
        if value.is_empty() {
            patch.banner_url = Some(None);
        } else if current.card.banner_url.as_deref() != Some(value) {
            let code = if crate::avatars::parse_banner_url(value).is_some() {
                "stale"
            } else {
                "invalid"
            };
            errors.insert("banner_url", code);
        }
    }
    validate::finish(errors)?;

    if patch == ProfilePatch::default() {
        return Ok(Json(current));
    }

    match crate::avatars::save_profile(&state, current.id, &patch, None).await? {
        Some(updated) => Ok(Json(updated)),
        // The row disappeared between the extractor and the update (account
        // deleted in another tab); report it as a plain re-login.
        None => Err(ApiError::Unauthenticated),
    }
}

/// Visible to the person themselves, members of a shared server and DM
/// peers; anyone else gets the same `404` as for an unknown id.
async fn profile_of(
    State(state): State<AppState>,
    CurrentUser(viewer): CurrentUser,
    Id(target): Id,
) -> Result<Json<PublicProfile>, ApiError> {
    user::public_profile(&state.db, viewer.id, target)
        .await?
        .map(Json)
        .ok_or(ApiError::NotFound)
}
