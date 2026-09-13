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
}

/// Internal row used only by login.
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
         RETURNING id, email, name, avatar_url, created_at",
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

pub async fn by_id(db: &PgPool, id: Uuid) -> Result<Option<User>, ApiError> {
    Ok(sqlx::query_as::<_, User>(
        "SELECT id, email, name, avatar_url, created_at FROM users WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(db)
    .await?)
}

/// Profile fields a user may change about themselves. `None` leaves the
/// column untouched; `Some(None)` for the avatar clears it.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ProfilePatch {
    pub name: Option<String>,
    pub avatar_url: Option<Option<String>>,
}

pub async fn update_profile(
    db: &PgPool,
    id: Uuid,
    patch: &ProfilePatch,
) -> Result<Option<User>, ApiError> {
    Ok(sqlx::query_as::<_, User>(
        "UPDATE users \
         SET name = COALESCE($2, name), \
             avatar_url = CASE WHEN $3 THEN $4 ELSE avatar_url END, \
             updated_at = now() \
         WHERE id = $1 \
         RETURNING id, email, name, avatar_url, created_at",
    )
    .bind(id)
    .bind(patch.name.as_deref())
    .bind(patch.avatar_url.is_some())
    .bind(patch.avatar_url.clone().flatten())
    .fetch_optional(db)
    .await?)
}
