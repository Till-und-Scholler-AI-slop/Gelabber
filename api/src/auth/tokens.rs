//! Single-use tokens for mailed links (`account_tokens`, migration 0021).
//!
//! Same shape as session cookies: 256 random bits as hex in the link, only
//! the SHA-256 at rest. Issuing a token replaces the account's earlier ones
//! of the same purpose, so only the newest mail works; using one deletes it.

use std::time::Duration;

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::ApiError;
use crate::token;

/// A confirmation link may wait in an inbox overnight.
pub const VERIFY_EMAIL_TTL: Duration = Duration::from_secs(24 * 3600);
/// A reset link opens the account to whoever holds it: one hour.
pub const RESET_PASSWORD_TTL: Duration = Duration::from_secs(3600);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    VerifyEmail,
    ResetPassword,
}

impl Purpose {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::VerifyEmail => "verify_email",
            Self::ResetPassword => "reset_password",
        }
    }

    fn ttl(self) -> Duration {
        match self {
            Self::VerifyEmail => VERIFY_EMAIL_TTL,
            Self::ResetPassword => RESET_PASSWORD_TTL,
        }
    }
}

/// Stores a fresh token for `user_id` and returns the raw value for the
/// mail. Earlier tokens of the purpose go, and with them every expired row.
pub async fn issue(db: &PgPool, user_id: Uuid, purpose: Purpose) -> Result<String, ApiError> {
    let raw = token::generate();
    let expires_at = Utc::now()
        + chrono::Duration::from_std(purpose.ttl())
            .map_err(|err| ApiError::Internal(format!("token ttl out of range: {err}")))?;
    let mut tx = db.begin().await?;
    sqlx::query(
        "DELETE FROM account_tokens \
         WHERE expires_at <= now() OR (user_id = $1 AND purpose = $2)",
    )
    .bind(user_id)
    .bind(purpose.as_str())
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO account_tokens (token_hash, user_id, purpose, expires_at) \
         VALUES ($1, $2, $3, $4)",
    )
    .bind(token::hash(&raw))
    .bind(user_id)
    .bind(purpose.as_str())
    .bind(expires_at)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(raw)
}

/// The account a live token belongs to, without using it up. Lets a reset
/// refuse a dead link before it pays for hashing the new password.
pub async fn peek(db: &PgPool, raw: &str, purpose: Purpose) -> Result<Option<Uuid>, ApiError> {
    if !token::is_valid(raw) {
        return Ok(None);
    }
    Ok(sqlx::query_scalar(
        "SELECT user_id FROM account_tokens \
         WHERE token_hash = $1 AND purpose = $2 AND expires_at > now()",
    )
    .bind(token::hash(raw))
    .bind(purpose.as_str())
    .fetch_optional(db)
    .await?)
}

/// Confirms the address behind a live confirmation token and uses it up.
/// Returns the account, or `None` for an unknown, expired or used link.
pub async fn verify_email(db: &PgPool, raw: &str) -> Result<Option<Uuid>, ApiError> {
    if !token::is_valid(raw) {
        return Ok(None);
    }
    Ok(sqlx::query_scalar(
        "WITH used AS ( \
             DELETE FROM account_tokens \
             WHERE token_hash = $1 AND purpose = 'verify_email' AND expires_at > now() \
             RETURNING user_id) \
         UPDATE users u \
         SET email_verified_at = COALESCE(u.email_verified_at, now()), updated_at = now() \
         FROM used WHERE u.id = used.user_id \
         RETURNING u.id",
    )
    .bind(token::hash(raw))
    .fetch_optional(db)
    .await?)
}

/// Uses up a live reset token, sets the new hash, confirms the address (the
/// link reached its inbox) and ends every session, all in one transaction.
/// Returns the account and the removed session hashes, or `None` when the
/// link is no longer valid.
pub async fn reset_password(
    db: &PgPool,
    raw: &str,
    password_hash: &str,
) -> Result<Option<(Uuid, Vec<Vec<u8>>)>, ApiError> {
    if !token::is_valid(raw) {
        return Ok(None);
    }
    let mut tx = db.begin().await?;
    let user_id: Option<Uuid> = sqlx::query_scalar(
        "DELETE FROM account_tokens \
         WHERE token_hash = $1 AND purpose = 'reset_password' AND expires_at > now() \
         RETURNING user_id",
    )
    .bind(token::hash(raw))
    .fetch_optional(&mut *tx)
    .await?;
    let Some(user_id) = user_id else {
        return Ok(None);
    };
    sqlx::query(
        "UPDATE users SET password_hash = $2, \
             email_verified_at = COALESCE(email_verified_at, now()), updated_at = now() \
         WHERE id = $1",
    )
    .bind(user_id)
    .bind(password_hash)
    .execute(&mut *tx)
    .await?;
    // Other links of the account die with the old password.
    sqlx::query("DELETE FROM account_tokens WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    let revoked =
        sqlx::query_scalar("DELETE FROM sessions WHERE user_id = $1 RETURNING token_hash")
            .bind(user_id)
            .fetch_all(&mut *tx)
            .await?;
    tx.commit().await?;
    Ok(Some((user_id, revoked)))
}
