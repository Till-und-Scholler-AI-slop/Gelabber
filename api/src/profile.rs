//! `/api/me`: read and edit the signed-in user's own profile (name, avatar).

use axum::Router;
use axum::extract::State;
use axum::response::Json;
use axum::routing::get;
use serde::Deserialize;

use crate::auth::session::CurrentUser;
use crate::auth::user::{self, ProfilePatch, User};
use crate::auth::validate;
use crate::error::{ApiError, FieldErrors};
use crate::json::Body;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/api/me", get(me).patch(update_me))
}

/// Fields are optional; a missing field is left untouched. `avatar_url: ""`
/// clears the avatar.
#[derive(Debug, Default, Deserialize)]
pub struct UpdateMeBody {
    pub name: Option<String>,
    pub avatar_url: Option<String>,
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
    validate::finish(errors)?;

    if patch == ProfilePatch::default() {
        return Ok(Json(current));
    }

    match user::update_profile(&state.db, current.id, &patch).await? {
        Some(updated) => {
            crate::avatars::discard_replaced(&state, &current, &updated).await;
            Ok(Json(updated))
        }
        // The row disappeared between the extractor and the update (account
        // deleted in another tab); report it as a plain re-login.
        None => Err(ApiError::Unauthenticated),
    }
}
