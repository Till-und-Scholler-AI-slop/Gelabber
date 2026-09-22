//! Invite links. The code is the last path segment of the link the web app
//! renders (`/invite/{code}`); joining needs a session, so an anonymous
//! visitor is sent through login/register first and lands back here.
//!
//! Any member may create an invite (that is how communities grow); listing
//! and revoking the server's invites needs `manage_server`, except that the
//! creator may always revoke their own.

use axum::Router;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post};
use chrono::{DateTime, Duration, Utc};
use gelabber_shared::ticket::ALPHABET;
use rand::RngExt;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};
use tracing::info;
use uuid::Uuid;

use crate::auth::session::CurrentUser;
use crate::error::{ApiError, FieldErrors};
use crate::json::Body;
use crate::path::Id;
use crate::state::AppState;

use super::ServerView;
use super::membership::{self, Membership};
use super::moderation;
use super::permissions::Permission;
use super::validate;

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/servers/{id}/invites",
            get(list_invites).post(create_invite),
        )
        .route(
            "/api/invites/{code}",
            get(preview_invite).delete(revoke_invite),
        )
        .route("/api/invites/{code}/join", post(join))
}

/// Lower-case only and without 0/o, 1/l/i: links get read aloud, typed and
/// retyped from screenshots, so no character may be confusable with another.
/// 30 symbols × 12 places ≈ 5·10¹⁷ codes. Same alphabet as media tickets.
pub const CODE_LEN: usize = 12;

pub fn generate_code() -> String {
    let mut rng = rand::rng();
    (0..CODE_LEN)
        .map(|_| ALPHABET[rng.random_range(0..ALPHABET.len())] as char)
        .collect()
}

/// Shape check before the database sees a code from the URL. Upper-case
/// input is accepted (someone retyped it) and folded to the stored form.
pub fn normalise_code(raw: &str) -> Option<String> {
    let code = raw.trim().to_ascii_lowercase();
    (code.len() == CODE_LEN && code.bytes().all(|b| ALPHABET.contains(&b))).then_some(code)
}

#[derive(Debug, Clone, Serialize, FromRow, PartialEq, Eq)]
pub struct Invite {
    pub code: String,
    pub server_id: Uuid,
    pub created_by: Uuid,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub max_uses: Option<i32>,
    pub uses: i32,
}

impl Invite {
    fn is_usable(&self, now: DateTime<Utc>) -> bool {
        let not_expired = self.expires_at.is_none_or(|at| at > now);
        let has_uses = self.max_uses.is_none_or(|max| self.uses < max);
        not_expired && has_uses
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct CreateInviteBody {
    /// Omit for unlimited.
    pub max_uses: Option<i64>,
    /// Omit for a link that never expires.
    pub expires_in_hours: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct InvitePreview {
    pub code: String,
    pub server: InviteServer,
    pub expires_at: Option<DateTime<Utc>>,
    /// Already a member: the client can skip straight to the server.
    pub member: bool,
}

#[derive(Debug, Serialize, FromRow)]
pub struct InviteServer {
    pub id: Uuid,
    pub name: String,
    pub member_count: i64,
}

async fn list_invites(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(server_id): Id,
) -> Result<Json<Vec<Invite>>, ApiError> {
    let member = membership::load(&state.db, server_id, user.id).await?;
    member.require(Permission::ManageServer)?;

    // Same liveness rule as `Invite::is_usable`: a link nobody can redeem
    // any more (expired or used up) is not "active" and must not be copied
    // out of the settings page.
    let invites = sqlx::query_as::<_, Invite>(
        "SELECT code, server_id, created_by, created_at, expires_at, max_uses, uses FROM invites \
         WHERE server_id = $1 \
           AND (expires_at IS NULL OR expires_at > now()) \
           AND (max_uses IS NULL OR uses < max_uses) \
         ORDER BY created_at DESC, code",
    )
    .bind(server_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(invites))
}

async fn create_invite(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(server_id): Id,
    Body(body): Body<CreateInviteBody>,
) -> Result<Response, ApiError> {
    membership::load(&state.db, server_id, user.id).await?;

    let mut errors = FieldErrors::new();
    let max_uses = validate::max_uses(body.max_uses, &mut errors);
    let expires_in = validate::expires_in_hours(body.expires_in_hours, &mut errors);
    validate::finish(errors)?;
    let max_uses = max_uses.expect("validated");
    let expires_at = expires_in
        .expect("validated")
        .map(|hours| Utc::now() + Duration::hours(hours));

    // A primary-key collision on a 12-char code is astronomically unlikely;
    // retrying a couple of times keeps it from ever being a user-facing error.
    for _ in 0..3 {
        let code = generate_code();
        let inserted = sqlx::query_as::<_, Invite>(
            "INSERT INTO invites (code, server_id, created_by, expires_at, max_uses) \
             VALUES ($1, $2, $3, $4, $5) \
             ON CONFLICT (code) DO NOTHING \
             RETURNING code, server_id, created_by, created_at, expires_at, max_uses, uses",
        )
        .bind(&code)
        .bind(server_id)
        .bind(user.id)
        .bind(expires_at)
        .bind(max_uses)
        .fetch_optional(&state.db)
        .await?;
        if let Some(invite) = inserted {
            info!(server_id = %server_id, user_id = %user.id, "invite created");
            return Ok((StatusCode::CREATED, Json(invite)).into_response());
        }
    }
    Err(ApiError::Internal(
        "could not allocate a unique invite code".to_owned(),
    ))
}

async fn revoke_invite(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(code): Path<String>,
) -> Result<StatusCode, ApiError> {
    let invite = by_code(&state.db, &code).await?;
    let member = membership::load(&state.db, invite.server_id, user.id).await?;
    if invite.created_by != user.id {
        member.require(Permission::ManageServer)?;
    }

    sqlx::query("DELETE FROM invites WHERE code = $1")
        .bind(&invite.code)
        .execute(&state.db)
        .await?;
    info!(server_id = %invite.server_id, "invite revoked");
    Ok(StatusCode::NO_CONTENT)
}

/// What the join page shows before the button is pressed. Unlike every
/// other server read this does *not* require membership — that is the point
/// of a link — but it also exposes nothing beyond name and size.
async fn preview_invite(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(code): Path<String>,
) -> Result<Json<InvitePreview>, ApiError> {
    let invite = by_code(&state.db, &code).await?;
    if moderation::is_banned(&state.db, invite.server_id, user.id).await? {
        return Err(ApiError::Banned);
    }
    let member = membership::load(&state.db, invite.server_id, user.id)
        .await
        .map(|_| true)
        .or_else(|err| match err {
            ApiError::NotFound => Ok(false),
            other => Err(other),
        })?;
    // A member may still see their own server through a dead link.
    if !member && !invite.is_usable(Utc::now()) {
        return Err(ApiError::InviteInvalid);
    }
    let server = server_summary(&state.db, invite.server_id).await?;
    Ok(Json(InvitePreview {
        code: invite.code,
        server,
        expires_at: invite.expires_at,
        member,
    }))
}

/// Adds the caller to the server. Idempotent: an existing member gets the
/// server back without consuming a use.
async fn join(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(code): Path<String>,
) -> Result<Json<ServerView>, ApiError> {
    let code = normalise_code(&code).ok_or(ApiError::NotFound)?;

    let mut tx = state.db.begin().await?;
    // Lock the row so two concurrent joins cannot both pass a `max_uses`
    // check with one slot left.
    let invite = sqlx::query_as::<_, Invite>("SELECT code, server_id, created_by, created_at, expires_at, max_uses, uses FROM invites WHERE code = $1 FOR UPDATE")
    .bind(&code)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(ApiError::NotFound)?;

    let already: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM server_members WHERE server_id = $1 AND user_id = $2)",
    )
    .bind(invite.server_id)
    .bind(user.id)
    .fetch_one(&mut *tx)
    .await?;

    if !already {
        let banned: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM server_bans WHERE server_id = $1 AND user_id = $2)",
        )
        .bind(invite.server_id)
        .bind(user.id)
        .fetch_one(&mut *tx)
        .await?;
        if banned {
            return Err(ApiError::Banned);
        }
        if !invite.is_usable(Utc::now()) {
            return Err(ApiError::InviteInvalid);
        }
        sqlx::query("INSERT INTO server_members (server_id, user_id) VALUES ($1, $2)")
            .bind(invite.server_id)
            .bind(user.id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE invites SET uses = uses + 1 WHERE code = $1")
            .bind(&invite.code)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    if !already {
        info!(server_id = %invite.server_id, user_id = %user.id, "member joined via invite");
    }

    let member: Membership = membership::load(&state.db, invite.server_id, user.id).await?;
    Ok(Json(member.into()))
}

async fn by_code(db: &PgPool, code: &str) -> Result<Invite, ApiError> {
    let code = normalise_code(code).ok_or(ApiError::NotFound)?;
    sqlx::query_as::<_, Invite>("SELECT code, server_id, created_by, created_at, expires_at, max_uses, uses FROM invites WHERE code = $1")
        .bind(code)
        .fetch_optional(db)
        .await?
        .ok_or(ApiError::NotFound)
}

async fn server_summary(db: &PgPool, server_id: Uuid) -> Result<InviteServer, ApiError> {
    sqlx::query_as::<_, InviteServer>(
        "SELECT s.id, s.name, \
                (SELECT count(*) FROM server_members m WHERE m.server_id = s.id) AS member_count \
         FROM servers s WHERE s.id = $1",
    )
    .bind(server_id)
    .fetch_optional(db)
    .await?
    .ok_or(ApiError::NotFound)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_have_the_expected_shape() {
        let a = generate_code();
        let b = generate_code();
        assert_eq!(a.len(), CODE_LEN);
        assert_eq!(normalise_code(&a).as_deref(), Some(a.as_str()), "{a}");
        assert_ne!(a, b);
        assert_eq!(
            normalise_code(&a.to_ascii_uppercase()).as_deref(),
            Some(a.as_str()),
            "retyped in caps still resolves"
        );
        assert_eq!(normalise_code(""), None);
        assert_eq!(normalise_code("abc"), None);
        assert_eq!(
            normalise_code(&"0".repeat(CODE_LEN)),
            None,
            "0 is not in the alphabet"
        );
        assert_eq!(normalise_code(&"a".repeat(CODE_LEN + 1)), None);
    }

    #[test]
    fn usability_checks_expiry_and_uses() {
        let now = Utc::now();
        let base = Invite {
            code: "x".repeat(CODE_LEN),
            server_id: Uuid::from_u128(1),
            created_by: Uuid::from_u128(2),
            created_at: now,
            expires_at: None,
            max_uses: None,
            uses: 0,
        };
        assert!(base.is_usable(now));
        assert!(
            Invite {
                expires_at: Some(now + Duration::minutes(1)),
                ..base.clone()
            }
            .is_usable(now)
        );
        assert!(
            !Invite {
                expires_at: Some(now - Duration::minutes(1)),
                ..base.clone()
            }
            .is_usable(now)
        );
        assert!(
            Invite {
                max_uses: Some(2),
                uses: 1,
                ..base.clone()
            }
            .is_usable(now)
        );
        assert!(
            !Invite {
                max_uses: Some(2),
                uses: 2,
                ..base
            }
            .is_usable(now)
        );
    }
}
