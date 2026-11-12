//! `users` table access and the public `User` shape.

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{FromRow, PgConnection, PgPool};
use uuid::Uuid;

use crate::error::ApiError;
use crate::i18n::Locale;

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
    /// The account's language (migration 0030). Private to the account:
    /// [`PublicProfile`] and member lists do not carry it.
    pub locale: Locale,
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

/// Who may see `u`'s profile card (bound as `$1` = viewer): themselves,
/// someone sharing a server, or the peer of a DM. A macro so the queries
/// below stay `&'static str` (`concat!`).
macro_rules! card_visible_to_viewer {
    () => {
        "( \
     u.id = $1 \
     OR EXISTS (SELECT 1 FROM server_members a \
                JOIN server_members b ON b.server_id = a.server_id \
                WHERE a.user_id = $1 AND b.user_id = u.id) \
     OR EXISTS (SELECT 1 FROM channel_members a \
                JOIN channel_members b ON b.channel_id = a.channel_id \
                JOIN channels c ON c.id = a.channel_id AND c.kind = 'dm' \
                WHERE a.user_id = $1 AND b.user_id = u.id))"
    };
}

/// `PublicProfile` of `target` if `viewer` may see it: themselves, someone
/// sharing a server, or the peer of a DM. Otherwise `None`, so a stranger's
/// id is indistinguishable from a missing one.
pub async fn public_profile(
    db: &PgPool,
    viewer: Uuid,
    target: Uuid,
) -> Result<Option<PublicProfile>, ApiError> {
    Ok(sqlx::query_as::<_, PublicProfile>(concat!(
        "SELECT u.id, u.name, u.avatar_url, u.created_at, u.bio, u.pronouns, u.status_text, \
                u.accent_color, u.banner_color, u.banner_url, u.avatar_decoration \
         FROM users u WHERE u.id = $2 AND u.deleted_at IS NULL AND ",
        card_visible_to_viewer!()
    ))
    .bind(viewer)
    .bind(target)
    .fetch_optional(db)
    .await?)
}

/// Whether `viewer` may see `target`'s profile card, by the same rule as
/// [`public_profile`]. Gates banner downloads, which only the card shows.
pub async fn card_visible(db: &PgPool, viewer: Uuid, target: Uuid) -> Result<bool, ApiError> {
    Ok(sqlx::query_scalar::<_, bool>(concat!(
        "SELECT EXISTS (SELECT 1 FROM users u WHERE u.id = $2 AND u.deleted_at IS NULL AND ",
        card_visible_to_viewer!(),
        ")"
    ))
    .bind(viewer)
    .bind(target)
    .fetch_one(db)
    .await?)
}

/// Internal row used only by login and password changes.
#[derive(Debug, FromRow)]
pub struct Credentials {
    pub id: Uuid,
    /// `None`: the account signs in only through a provider (migration
    /// 0033) until its owner sets a password.
    pub password_hash: Option<String>,
    /// The owner opened a confirmation or reset link (migration 0021).
    pub email_verified: bool,
    /// An operator disabled the account (migration 0032).
    pub disabled: bool,
}

/// Inserts a new account. `email` must already be normalised (trimmed,
/// lower-cased). A unique-index hit maps to `ApiError::EmailTaken`.
pub async fn insert(
    db: &PgPool,
    email: &str,
    name: &str,
    password_hash: &str,
    locale: Locale,
) -> Result<User, ApiError> {
    let mut conn = db.acquire().await?;
    insert_in(&mut conn, email, name, password_hash, locale).await
}

/// [`insert`] on the caller's connection, so registration can redeem its
/// invitation in the same transaction (`operator::admission::redeem`).
pub async fn insert_in(
    conn: &mut PgConnection,
    email: &str,
    name: &str,
    password_hash: &str,
    locale: Locale,
) -> Result<User, ApiError> {
    sqlx::query_as::<_, User>(
        "INSERT INTO users (email, name, password_hash, locale) VALUES ($1, $2, $3, $4) \
         RETURNING id, email, name, avatar_url, created_at, bio, pronouns, status_text, \
             accent_color, banner_color, banner_url, avatar_decoration, locale",
    )
    .bind(email)
    .bind(name)
    .bind(password_hash)
    .bind(locale)
    .fetch_one(&mut *conn)
    .await
    .map_err(|err| match &err {
        // SQLSTATE 23505 = unique_violation.
        sqlx::Error::Database(db_err) if db_err.code().as_deref() == Some("23505") => {
            ApiError::EmailTaken
        }
        _ => err.into(),
    })
}

/// Inserts an account created through a provider's first sign-in: no
/// password, the address confirmed (the provider reported it verified).
/// `email` must already be normalised. A unique-index hit maps to
/// `ApiError::EmailTaken`.
pub async fn insert_oidc(
    conn: &mut PgConnection,
    email: &str,
    name: &str,
    locale: Locale,
) -> Result<User, ApiError> {
    sqlx::query_as::<_, User>(
        "INSERT INTO users (email, name, password_hash, email_verified_at, \
             registration_pending, locale) \
         VALUES ($1, $2, NULL, now(), false, $3) \
         RETURNING id, email, name, avatar_url, created_at, bio, pronouns, status_text, \
             accent_color, banner_color, banner_url, avatar_decoration, locale",
    )
    .bind(email)
    .bind(name)
    .bind(locale)
    .fetch_one(&mut *conn)
    .await
    .map_err(|err| match &err {
        // SQLSTATE 23505 = unique_violation.
        sqlx::Error::Database(db_err) if db_err.code().as_deref() == Some("23505") => {
            ApiError::EmailTaken
        }
        _ => err.into(),
    })
}

/// Registration while account mail is on: inserts an unconfirmed account,
/// or takes over a pending one (`registration_pending`, migration 0021)
/// with this registrant's name, password and language, dropping its earlier
/// confirmation links (a reset link the owner asked for stays).
/// Nobody has proven they own a pending address, so the person who finally
/// confirms it gets the credentials they typed, never a stranger's.
/// Returns `None` when the address has a confirmed account or one from a
/// time without mail; that row does not change.
pub async fn insert_unverified(
    db: &PgPool,
    email: &str,
    name: &str,
    password_hash: &str,
    locale: Locale,
) -> Result<Option<Uuid>, ApiError> {
    let mut tx = db.begin().await?;
    let id = insert_unverified_in(&mut tx, email, name, password_hash, locale).await?;
    tx.commit().await?;
    Ok(id)
}

/// [`insert_unverified`] inside the caller's transaction.
pub async fn insert_unverified_in(
    conn: &mut PgConnection,
    email: &str,
    name: &str,
    password_hash: &str,
    locale: Locale,
) -> Result<Option<Uuid>, ApiError> {
    let id: Option<Uuid> = sqlx::query_scalar(
        "INSERT INTO users (email, name, password_hash, registration_pending, locale) \
         VALUES ($1, $2, $3, true, $4) \
         ON CONFLICT ((lower(email))) DO UPDATE \
         SET name = EXCLUDED.name, password_hash = EXCLUDED.password_hash, \
             locale = EXCLUDED.locale, updated_at = now() \
         WHERE users.registration_pending AND users.email_verified_at IS NULL \
             AND users.disabled_at IS NULL AND users.deleted_at IS NULL \
         RETURNING id",
    )
    .bind(email)
    .bind(name)
    .bind(password_hash)
    .bind(locale)
    .fetch_optional(&mut *conn)
    .await?;
    if let Some(id) = id {
        sqlx::query("DELETE FROM account_tokens WHERE user_id = $1 AND purpose = 'verify_email'")
            .bind(id)
            .execute(&mut *conn)
            .await?;
    }
    Ok(id)
}

/// The account behind `email` and whether its address is confirmed. An
/// account under deletion (migration 0034) has none.
pub async fn verified_by_email(db: &PgPool, email: &str) -> Result<Option<(Uuid, bool)>, ApiError> {
    Ok(sqlx::query_as(
        "SELECT id, email_verified_at IS NOT NULL FROM users \
         WHERE lower(email) = $1 AND deleted_at IS NULL",
    )
    .bind(email)
    .fetch_optional(db)
    .await?)
}

/// An account under deletion has none, so it signs in like an unknown
/// address.
pub async fn credentials_by_email(
    db: &PgPool,
    email: &str,
) -> Result<Option<Credentials>, ApiError> {
    Ok(sqlx::query_as::<_, Credentials>(
        "SELECT id, password_hash, email_verified_at IS NOT NULL AS email_verified, \
             disabled_at IS NOT NULL AS disabled \
             FROM users WHERE lower(email) = $1 AND deleted_at IS NULL",
    )
    .bind(email)
    .fetch_optional(db)
    .await?)
}

/// Whether an operator disabled the account; `false` for an unknown id.
pub async fn is_disabled(db: &PgPool, id: Uuid) -> Result<bool, ApiError> {
    Ok(
        sqlx::query_scalar::<_, bool>("SELECT disabled_at IS NOT NULL FROM users WHERE id = $1")
            .bind(id)
            .fetch_optional(db)
            .await?
            .unwrap_or(false),
    )
}

/// The account's password hash; `None` for an unknown id and for an
/// account without a password (migration 0033).
pub async fn password_hash_by_id(db: &PgPool, id: Uuid) -> Result<Option<String>, ApiError> {
    Ok(
        sqlx::query_scalar::<_, Option<String>>("SELECT password_hash FROM users WHERE id = $1")
            .bind(id)
            .fetch_optional(db)
            .await?
            .flatten(),
    )
}

/// Replaces the hash and drops every session of the account in the same
/// transaction, so no cookie issued under the old password outlives it, nor
/// a mailed reset link. Returns the removed session hashes for the
/// live-connection cleanup.
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
    sqlx::query("DELETE FROM account_tokens WHERE user_id = $1 AND purpose = 'reset_password'")
        .bind(id)
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

/// `None` for an unknown id and for an account under deletion.
pub async fn by_id(db: &PgPool, id: Uuid) -> Result<Option<User>, ApiError> {
    Ok(sqlx::query_as::<_, User>(
        "SELECT id, email, name, avatar_url, created_at, bio, pronouns, status_text, \
             accent_color, banner_color, banner_url, avatar_decoration, locale \
         FROM users WHERE id = $1 AND deleted_at IS NULL",
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
             u.locale, prev.avatar_url AS previous_avatar_url, prev.banner_url AS previous_banner_url",
    )
    .bind(id)
    .bind(patch.name.as_deref());
    for field in optional {
        query = query.bind(field.is_some()).bind(field.clone().flatten());
    }
    Ok(query.fetch_optional(db).await?)
}

/// Sets the account's language. `None` when the row is gone.
pub async fn set_locale(db: &PgPool, id: Uuid, locale: Locale) -> Result<Option<Locale>, ApiError> {
    Ok(
        sqlx::query_scalar("UPDATE users SET locale = $2 WHERE id = $1 RETURNING locale")
            .bind(id)
            .bind(locale)
            .fetch_optional(db)
            .await?,
    )
}
