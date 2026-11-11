//! Which Gelabber account a verified provider answer belongs to (#518).
//!
//! Sign-in, in this order:
//! 1. A known identity (`provider`, `sub`) signs its account in, whatever
//!    the address says now: it was attached on purpose.
//! 2. Without an address the provider verified, nothing is created or
//!    joined (`oidc_email_unverified`): a provider must not hand out
//!    someone else's address.
//! 3. An account with that address, confirmed, gets the identity and is
//!    signed in ("joins the existing account").
//! 4. An unconfirmed registration while mail is on (`registration_pending`)
//!    counts as no account: [`create`] takes the row over and drops the
//!    password whoever registered it chose.
//! 5. An account that was never confirmed (registered while mail was off)
//!    is refused (`oidc_account_unverified`): its password may be a
//!    stranger's. Its owner signs in with it and connects the provider.
//! 6. Otherwise a new account, after the operator's admission gate.
//!
//! A disabled account is refused (`account_disabled`) in 1 and 3–5, never
//! taken over.

use sqlx::PgConnection;
use uuid::Uuid;

use crate::auth::user::{self, User};
use crate::error::ApiError;
use crate::i18n::Locale;

/// The verified claims that resolution looks at.
#[derive(Debug, Clone, Copy)]
pub struct Identity<'a> {
    pub provider: &'a str,
    pub subject: &'a str,
    /// Canonical, `None` when the provider sent no usable address.
    pub email: Option<&'a str>,
    pub email_verified: bool,
}

/// The outcome of a sign-in through a provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignIn {
    /// Case 1: the identity's account.
    Known(Uuid),
    /// Case 3: a confirmed account that now carries the identity.
    Joined(Uuid),
    /// Cases 4 and 6: a new account after the name step; `takeover` is the
    /// pending registration it replaces.
    New { takeover: Option<Uuid> },
}

/// Resolves a sign-in on the caller's transaction. A join (case 3) writes
/// the identity there; the row of the address stays locked until commit,
/// so a concurrent registration or takeover cannot slip in between.
pub async fn sign_in(conn: &mut PgConnection, identity: &Identity<'_>) -> Result<SignIn, ApiError> {
    let known: Option<(Uuid, bool)> = sqlx::query_as(
        "SELECT i.user_id, u.disabled_at IS NOT NULL FROM user_identities i \
         JOIN users u ON u.id = i.user_id \
         WHERE i.provider = $1 AND i.subject = $2",
    )
    .bind(identity.provider)
    .bind(identity.subject)
    .fetch_optional(&mut *conn)
    .await?;
    if let Some((user_id, disabled)) = known {
        if disabled {
            return Err(ApiError::AccountDisabled);
        }
        touch(conn, identity).await?;
        return Ok(SignIn::Known(user_id));
    }

    let email = match identity.email {
        Some(email) if identity.email_verified => email,
        _ => return Err(ApiError::OidcEmailUnverified),
    };
    // Either spelling of the domain, as at password sign-in: an older
    // account may store it in Unicode form. The ASCII form wins a tie.
    let unicode = unicode_domain(email);
    let account: Option<(Uuid, bool, bool, bool)> = sqlx::query_as(
        "SELECT id, email_verified_at IS NOT NULL, registration_pending, \
             disabled_at IS NOT NULL \
         FROM users WHERE lower(email) = $1 OR lower(email) = $2 \
         ORDER BY lower(email) = $1 DESC LIMIT 1 FOR UPDATE",
    )
    .bind(email)
    .bind(unicode.as_deref().unwrap_or(email))
    .fetch_optional(&mut *conn)
    .await?;
    match account {
        None => Ok(SignIn::New { takeover: None }),
        Some((_, _, _, true)) => Err(ApiError::AccountDisabled),
        Some((id, true, _, false)) => {
            attach(conn, id, identity).await?;
            Ok(SignIn::Joined(id))
        }
        Some((id, false, true, false)) => Ok(SignIn::New { takeover: Some(id) }),
        Some((_, false, false, false)) => Err(ApiError::OidcAccountUnverified),
    }
}

/// `email` (canonical, ASCII domain) with its domain in Unicode form, if
/// that differs.
fn unicode_domain(email: &str) -> Option<String> {
    let (local, domain) = email.rsplit_once('@')?;
    let unicode = url::quirks::domain_to_unicode(domain);
    (!unicode.is_empty() && unicode != domain).then(|| format!("{local}@{unicode}"))
}

/// Connects the identity to the signed-in account `user_id`. `true` when
/// it was attached now, `false` when the account had it already. An
/// identity of another account, or a second one of this provider, is
/// `identity_taken`. A verified address equal to the account's confirms it.
pub async fn link(
    conn: &mut PgConnection,
    user_id: Uuid,
    identity: &Identity<'_>,
) -> Result<bool, ApiError> {
    let owner: Option<Uuid> = sqlx::query_scalar(
        "SELECT user_id FROM user_identities WHERE provider = $1 AND subject = $2",
    )
    .bind(identity.provider)
    .bind(identity.subject)
    .fetch_optional(&mut *conn)
    .await?;
    match owner {
        Some(owner) if owner == user_id => {
            touch(conn, identity).await?;
            return Ok(false);
        }
        Some(_) => return Err(ApiError::IdentityTaken),
        None => {}
    }
    attach(conn, user_id, identity).await?;
    if identity.email_verified
        && let Some(email) = identity.email
    {
        sqlx::query(
            "UPDATE users SET email_verified_at = COALESCE(email_verified_at, now()) \
             WHERE id = $1 AND lower(email) = $2",
        )
        .bind(user_id)
        .bind(email)
        .execute(&mut *conn)
        .await?;
    }
    Ok(true)
}

/// Creates the account of a first sign-in (cases 4 and 6) with its name
/// and language and attaches the identity. A pending registration is taken
/// over in place: the provider's account owns the address now, so the
/// registrant's password, confirmation links and any session go. Returns
/// the account and the hashes of the sessions that ended.
pub async fn create(
    conn: &mut PgConnection,
    identity: &Identity<'_>,
    takeover: Option<Uuid>,
    name: &str,
    locale: Locale,
) -> Result<(User, Vec<Vec<u8>>), ApiError> {
    let email = identity.email.ok_or(ApiError::OidcEmailUnverified)?;
    let (account, revoked) = match takeover {
        None => (
            user::insert_oidc(conn, email, name, locale).await?,
            Vec::new(),
        ),
        Some(id) => {
            let account = sqlx::query_as::<_, User>(
                "UPDATE users SET name = $2, password_hash = NULL, registration_pending = false, \
                     email_verified_at = now(), locale = $3, updated_at = now() \
                 WHERE id = $1 AND registration_pending AND email_verified_at IS NULL \
                     AND disabled_at IS NULL \
                 RETURNING id, email, name, avatar_url, created_at, bio, pronouns, status_text, \
                     accent_color, banner_color, banner_url, avatar_decoration, locale",
            )
            .bind(id)
            .bind(name)
            .bind(locale)
            .fetch_optional(&mut *conn)
            .await?
            .ok_or_else(|| ApiError::Internal(format!("pending account {id} changed")))?;
            sqlx::query("DELETE FROM account_tokens WHERE user_id = $1")
                .bind(id)
                .execute(&mut *conn)
                .await?;
            let revoked =
                sqlx::query_scalar("DELETE FROM sessions WHERE user_id = $1 RETURNING token_hash")
                    .bind(id)
                    .fetch_all(&mut *conn)
                    .await?;
            (account, revoked)
        }
    };
    attach(conn, account.id, identity).await?;
    Ok((account, revoked))
}

/// Inserts the identity; a unique-index hit (it or this provider is
/// attached already) is `identity_taken`.
async fn attach(
    conn: &mut PgConnection,
    user_id: Uuid,
    identity: &Identity<'_>,
) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO user_identities (provider, subject, user_id, email) VALUES ($1, $2, $3, $4)",
    )
    .bind(identity.provider)
    .bind(identity.subject)
    .bind(user_id)
    .bind(identity.email.unwrap_or_default())
    .execute(&mut *conn)
    .await
    .map_err(|err| match &err {
        // SQLSTATE 23505 = unique_violation.
        sqlx::Error::Database(db_err) if db_err.code().as_deref() == Some("23505") => {
            ApiError::IdentityTaken
        }
        _ => err.into(),
    })?;
    Ok(())
}

/// A known identity was used: its address as the provider says now.
async fn touch(conn: &mut PgConnection, identity: &Identity<'_>) -> Result<(), ApiError> {
    sqlx::query(
        "UPDATE user_identities SET email = COALESCE($3, email), last_used_at = now() \
         WHERE provider = $1 AND subject = $2",
    )
    .bind(identity.provider)
    .bind(identity.subject)
    .bind(identity.email)
    .execute(&mut *conn)
    .await?;
    Ok(())
}
