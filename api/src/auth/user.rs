//! `users` table access and the public `User` shape.

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use crate::error::ApiError;

/// What the API returns about an account. Never carries the password hash.
#[derive(Debug, Clone, Serialize, FromRow, PartialEq, Eq)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub name: String,
    pub avatar_url: Option<String>,
    pub created_at: DateTime<Utc>,
    #[sqlx(flatten)]
    #[serde(flatten)]
    pub card: ProfileCard,
}

/// The optional profile card fields (migration 0013). `None` everywhere is
/// "not set"; clients render their defaults then.
#[derive(Debug, Clone, Default, Serialize, FromRow, PartialEq, Eq)]
pub struct ProfileCard {
    pub bio: Option<String>,
    pub pronouns: Option<String>,
    pub status_text: Option<String>,
    /// `#rrggbb`, lower-case.
    pub accent_color: Option<String>,
    /// `#rrggbb`, lower-case; shown when there is no banner image.
    pub banner_color: Option<String>,
    /// An uploaded banner (`crate::avatars`), never a foreign link.
    pub banner_url: Option<String>,
    /// One of `validate::AVATAR_DECORATIONS`.
    pub avatar_decoration: Option<String>,
}

/// What another member may see about an account: the profile card without
/// the e-mail address.
#[derive(Debug, Clone, Serialize, FromRow, PartialEq, Eq)]
pub struct PublicProfile {
    pub id: Uuid,
    pub name: String,
    pub avatar_url: Option<String>,
    pub created_at: DateTime<Utc>,
    #[sqlx(flatten)]
    #[serde(flatten)]
    pub card: ProfileCard,
}

/// `PublicProfile` of `target` if `viewer` may see it: themselves, someone
/// sharing a server, or the peer of a DM. Otherwise `None`, so a stranger's
/// id is indistinguishable from a missing one.
pub async fn public_profile(
    db: &PgPool,
    viewer: Uuid,
    target: Uuid,
) -> Result<Option<PublicProfile>, ApiError> {
    Ok(sqlx::query_as::<_, PublicProfile>(
        "SELECT u.id, u.name, u.avatar_url, u.created_at, u.bio, u.pronouns, u.status_text, \
                u.accent_color, u.banner_color, u.banner_url, u.avatar_decoration \
         FROM users u \
         WHERE u.id = $2 AND ( \
             u.id = $1 \
             OR EXISTS (SELECT 1 FROM server_members a \
                        JOIN server_members b ON b.server_id = a.server_id \
                        WHERE a.user_id = $1 AND b.user_id = u.id) \
             OR EXISTS (SELECT 1 FROM channel_members a \
                        JOIN channel_members b ON b.channel_id = a.channel_id \
                        JOIN channels c ON c.id = a.channel_id AND c.kind = 'dm' \
                        WHERE a.user_id = $1 AND b.user_id = u.id))",
    )
    .bind(viewer)
    .bind(target)
    .fetch_optional(db)
    .await?)
}

/// Internal row used only by login and password changes.
#[derive(Debug, FromRow)]
pub struct Credentials {
    pub id: Uuid,
    pub password_hash: String,
}

/// Inserts a new account. `email` must already be normalised (trimmed,
/// lower-cased). A unique-index hit maps to `ApiError::EmailTaken`.
pub async fn insert(
    db: &PgPool,
    email: &str,
    name: &str,
    password_hash: &str,
) -> Result<User, ApiError> {
    sqlx::query_as::<_, User>(
        "INSERT INTO users (email, name, password_hash) VALUES ($1, $2, $3) \
         RETURNING id, email, name, avatar_url, created_at, bio, pronouns, status_text, \
             accent_color, banner_color, banner_url, avatar_decoration",
    )
    .bind(email)
    .bind(name)
    .bind(password_hash)
    .fetch_one(db)
    .await
    .map_err(|err| match &err {
        // SQLSTATE 23505 = unique_violation.
        sqlx::Error::Database(db_err) if db_err.code().as_deref() == Some("23505") => {
            ApiError::EmailTaken
        }
        _ => err.into(),
    })
}

pub async fn credentials_by_email(
    db: &PgPool,
    email: &str,
) -> Result<Option<Credentials>, ApiError> {
    Ok(
        sqlx::query_as::<_, Credentials>("SELECT id, password_hash FROM users WHERE email = $1")
            .bind(email)
            .fetch_optional(db)
            .await?,
    )
}

pub async fn password_hash_by_id(db: &PgPool, id: Uuid) -> Result<Option<String>, ApiError> {
    Ok(
        sqlx::query_scalar("SELECT password_hash FROM users WHERE id = $1")
            .bind(id)
            .fetch_optional(db)
            .await?,
    )
}

/// Replaces the hash and drops every session of the account in the same
/// transaction, so no cookie issued under the old password outlives it.
/// Returns the removed session hashes for the live-connection cleanup.
pub async fn set_password_hash(
    db: &PgPool,
    id: Uuid,
    password_hash: &str,
) -> Result<Vec<Vec<u8>>, ApiError> {
    let mut tx = db.begin().await?;
    sqlx::query("UPDATE users SET password_hash = $2, updated_at = now() WHERE id = $1")
        .bind(id)
        .bind(password_hash)
        .execute(&mut *tx)
        .await?;
    let revoked =
        sqlx::query_scalar("DELETE FROM sessions WHERE user_id = $1 RETURNING token_hash")
            .bind(id)
            .fetch_all(&mut *tx)
            .await?;
    tx.commit().await?;
    Ok(revoked)
}

pub async fn by_id(db: &PgPool, id: Uuid) -> Result<Option<User>, ApiError> {
    Ok(sqlx::query_as::<_, User>(
        "SELECT id, email, name, avatar_url, created_at, bio, pronouns, status_text, \
             accent_color, banner_color, banner_url, avatar_decoration \
         FROM users WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(db)
    .await?)
}

/// Profile fields a user may change about themselves. `None` leaves the
/// column untouched; `Some(None)` clears an optional one.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ProfilePatch {
    pub name: Option<String>,
    pub avatar_url: Option<Option<String>>,
    pub bio: Option<Option<String>>,
    pub pronouns: Option<Option<String>>,
    pub status_text: Option<Option<String>>,
    pub accent_color: Option<Option<String>>,
    pub banner_color: Option<Option<String>>,
    pub banner_url: Option<Option<String>>,
    pub avatar_decoration: Option<Option<String>>,
}

/// The updated row and the uploads this update really replaced.
#[derive(Debug, FromRow)]
pub struct ProfileUpdate {
    #[sqlx(flatten)]
    pub user: User,
    pub previous_avatar_url: Option<String>,
    pub previous_banner_url: Option<String>,
}

/// Applies `patch` and returns the avatar and banner values this very
/// statement overwrote, read under the row lock, not one read earlier in the
/// request: two overlapping writes each see their own predecessor. `FOR NO
/// KEY UPDATE` is the lock the `UPDATE` takes anyway; it does not wait for
/// the key-share locks that foreign-key checks (e.g. message inserts) hold.
pub async fn update_profile(
    db: impl sqlx::PgExecutor<'_>,
    id: Uuid,
    patch: &ProfilePatch,
) -> Result<Option<ProfileUpdate>, ApiError> {
    let optional = [
        &patch.avatar_url,
        &patch.bio,
        &patch.pronouns,
        &patch.status_text,
        &patch.accent_color,
        &patch.banner_color,
        &patch.banner_url,
        &patch.avatar_decoration,
    ];
    let mut query = sqlx::query_as::<_, ProfileUpdate>(
        "UPDATE users u \
         SET name = COALESCE($2, u.name), \
             avatar_url = CASE WHEN $3 THEN $4 ELSE u.avatar_url END, \
             bio = CASE WHEN $5 THEN $6 ELSE u.bio END, \
             pronouns = CASE WHEN $7 THEN $8 ELSE u.pronouns END, \
             status_text = CASE WHEN $9 THEN $10 ELSE u.status_text END, \
             accent_color = CASE WHEN $11 THEN $12 ELSE u.accent_color END, \
             banner_color = CASE WHEN $13 THEN $14 ELSE u.banner_color END, \
             banner_url = CASE WHEN $15 THEN $16 ELSE u.banner_url END, \
             avatar_decoration = CASE WHEN $17 THEN $18 ELSE u.avatar_decoration END, \
             updated_at = now() \
         FROM (SELECT id, avatar_url, banner_url FROM users WHERE id = $1 FOR NO KEY UPDATE) prev \
         WHERE u.id = prev.id \
         RETURNING u.id, u.email, u.name, u.avatar_url, u.created_at, u.bio, u.pronouns, \
             u.status_text, u.accent_color, u.banner_color, u.banner_url, u.avatar_decoration, \
             prev.avatar_url AS previous_avatar_url, prev.banner_url AS previous_banner_url",
    )
    .bind(id)
    .bind(patch.name.as_deref());
    for field in optional {
        query = query.bind(field.is_some()).bind(field.clone().flatten());
    }
    Ok(query.fetch_optional(db).await?)
}
