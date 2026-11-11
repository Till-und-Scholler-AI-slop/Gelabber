//! The rows between the steps of a sign-in through a provider (#518):
//! `oidc_flows` from `start` to the provider's callback, `oidc_results`
//! from a verified answer to its last step (the name of a new account, or
//! the desktop app's exchange). Only SHA-256 hashes of `state`, the result
//! code and the browser binding are stored. Each insert first deletes the
//! table's expired rows, as `tokens::issue` does.

use std::time::Duration;

use sqlx::{FromRow, PgConnection, PgPool};
use uuid::Uuid;

use crate::error::ApiError;
use crate::token;

/// How long the provider has to send the person back.
pub const FLOW_TTL: Duration = Duration::from_secs(10 * 60);
/// How long a new account's name step may take.
pub const SIGNUP_TTL: Duration = Duration::from_secs(30 * 60);
/// How long the desktop app has to exchange its result code.
pub const RESULT_TTL: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    /// Sign in, or create an account after the name step.
    SignIn,
    /// Connect the provider to the signed-in account.
    Link,
}

impl Purpose {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SignIn => "sign_in",
            Self::Link => "link",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "sign_in" => Some(Self::SignIn),
            "link" => Some(Self::Link),
            _ => None,
        }
    }
}

/// Where the desktop app waits for the result.
#[derive(Debug, Clone)]
pub struct Desktop {
    pub port: u16,
    /// The app's own `state`, echoed to its loopback receiver.
    pub state: String,
    /// `base64url(sha256(verifier))` of the app's own PKCE pair.
    pub challenge: String,
}

/// What `start` stores. The browser `binding` and `desktop` exclude each
/// other; `binding` is the raw cookie value, stored hashed.
#[derive(Debug)]
pub struct NewFlow<'a> {
    pub provider: &'a str,
    pub purpose: Purpose,
    pub link_user_id: Option<Uuid>,
    pub nonce: &'a str,
    pub pkce_verifier: &'a str,
    pub binding: Option<&'a str>,
    pub desktop: Option<&'a Desktop>,
    pub redirect: Option<&'a str>,
    pub invitation: Option<&'a str>,
}

/// Stores a flow under `sha256(state)`.
pub async fn insert_flow(db: &PgPool, state: &str, flow: &NewFlow<'_>) -> Result<(), ApiError> {
    sqlx::query("DELETE FROM oidc_flows WHERE expires_at <= now()")
        .execute(db)
        .await?;
    sqlx::query(
        "INSERT INTO oidc_flows (state_hash, provider, purpose, link_user_id, nonce, \
             pkce_verifier, binding_hash, desktop_port, desktop_state, desktop_challenge, \
             redirect, invitation, expires_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, \
             now() + make_interval(secs => $13))",
    )
    .bind(token::hash(state))
    .bind(flow.provider)
    .bind(flow.purpose.as_str())
    .bind(flow.link_user_id)
    .bind(flow.nonce)
    .bind(flow.pkce_verifier)
    .bind(flow.binding.map(token::hash))
    .bind(flow.desktop.map(|desktop| i32::from(desktop.port)))
    .bind(flow.desktop.map(|desktop| desktop.state.as_str()))
    .bind(flow.desktop.map(|desktop| desktop.challenge.as_str()))
    .bind(flow.redirect)
    .bind(flow.invitation)
    .bind(FLOW_TTL.as_secs_f64())
    .execute(db)
    .await?;
    Ok(())
}

/// A flow as the callback takes it.
#[derive(Debug, FromRow)]
pub struct Flow {
    pub provider: String,
    pub purpose: String,
    pub link_user_id: Option<Uuid>,
    pub nonce: String,
    pub pkce_verifier: String,
    pub binding_hash: Option<Vec<u8>>,
    pub desktop_port: Option<i32>,
    pub desktop_state: Option<String>,
    pub desktop_challenge: Option<String>,
    pub redirect: Option<String>,
    pub invitation: Option<String>,
}

impl Flow {
    pub fn purpose(&self) -> Purpose {
        Purpose::parse(&self.purpose).unwrap_or(Purpose::SignIn)
    }

    /// The desktop app's loopback port and state, for a desktop flow.
    pub fn desktop(&self) -> Option<(u16, &str)> {
        let port = u16::try_from(self.desktop_port?).ok()?;
        Some((port, self.desktop_state.as_deref()?))
    }
}

/// Takes the live flow behind `state`, once: a second callback with the
/// same `state` finds nothing. An expired row is deleted all the same.
pub async fn take_flow(db: &PgPool, state: &str) -> Result<Option<Flow>, ApiError> {
    let row = sqlx::query_as::<_, FlowRow>(
        "DELETE FROM oidc_flows WHERE state_hash = $1 \
         RETURNING expires_at > now() AS live, provider, purpose, link_user_id, nonce, \
             pkce_verifier, binding_hash, desktop_port, desktop_state, desktop_challenge, \
             redirect, invitation",
    )
    .bind(token::hash(state))
    .fetch_optional(db)
    .await?;
    Ok(row.and_then(|row| row.live.then_some(row.flow)))
}

#[derive(FromRow)]
struct FlowRow {
    live: bool,
    #[sqlx(flatten)]
    flow: Flow,
}

/// What a verified provider answer leaves for the last step.
#[derive(Debug)]
pub struct NewResult<'a> {
    pub provider: &'a str,
    pub subject: &'a str,
    /// Empty when the provider sent no usable address.
    pub email: &'a str,
    pub email_verified: bool,
    pub name_hint: Option<&'a str>,
    pub purpose: Purpose,
    pub link_user_id: Option<Uuid>,
    pub binding_hash: Option<&'a [u8]>,
    pub desktop_challenge: Option<&'a str>,
    pub redirect: Option<&'a str>,
    pub invitation: Option<&'a str>,
    /// A new account waiting for its name ([`SIGNUP_TTL`]); otherwise a
    /// desktop result waiting for its exchange ([`RESULT_TTL`]).
    pub signup: bool,
}

/// Stores a result and returns its raw code (64 hex), which only the
/// browser's fragment or the desktop app's loopback redirect carries.
pub async fn insert_result(db: &PgPool, result: &NewResult<'_>) -> Result<String, ApiError> {
    sqlx::query("DELETE FROM oidc_results WHERE expires_at <= now()")
        .execute(db)
        .await?;
    let code = token::generate();
    let ttl = if result.signup {
        SIGNUP_TTL
    } else {
        RESULT_TTL
    };
    sqlx::query(
        "INSERT INTO oidc_results (code_hash, provider, subject, email, email_verified, \
             name_hint, purpose, link_user_id, binding_hash, desktop_challenge, redirect, \
             invitation, signup, expires_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, \
             now() + make_interval(secs => $14))",
    )
    .bind(token::hash(&code))
    .bind(result.provider)
    .bind(result.subject)
    .bind(result.email)
    .bind(result.email_verified)
    .bind(result.name_hint)
    .bind(result.purpose.as_str())
    .bind(result.link_user_id)
    .bind(result.binding_hash)
    .bind(result.desktop_challenge)
    .bind(result.redirect)
    .bind(result.invitation)
    .bind(result.signup)
    .bind(ttl.as_secs_f64())
    .execute(db)
    .await?;
    Ok(code)
}

/// A stored result.
#[derive(Debug, Clone, FromRow)]
pub struct Stored {
    pub provider: String,
    pub subject: String,
    pub email: String,
    pub email_verified: bool,
    pub name_hint: Option<String>,
    pub purpose: String,
    pub link_user_id: Option<Uuid>,
    pub binding_hash: Option<Vec<u8>>,
    pub desktop_challenge: Option<String>,
    pub redirect: Option<String>,
    pub invitation: Option<String>,
    pub signup: bool,
}

impl Stored {
    pub fn purpose(&self) -> Purpose {
        Purpose::parse(&self.purpose).unwrap_or(Purpose::SignIn)
    }

    /// The provider's address, `None` when it sent none.
    pub fn email(&self) -> Option<&str> {
        Some(self.email.as_str()).filter(|email| !email.is_empty())
    }
}

/// The columns of [`Stored`]; a macro so the queries stay `&'static str`.
macro_rules! stored_columns {
    () => {
        "provider, subject, email, email_verified, name_hint, purpose, link_user_id, \
         binding_hash, desktop_challenge, redirect, invitation, signup"
    };
}

/// The live result behind `code`, left in place. A code no token can be
/// costs no query.
pub async fn peek_result(db: &PgPool, code: &str) -> Result<Option<Stored>, ApiError> {
    if !token::is_valid(code) {
        return Ok(None);
    }
    Ok(sqlx::query_as::<_, Stored>(concat!(
        "SELECT ",
        stored_columns!(),
        " FROM oidc_results WHERE code_hash = $1 AND expires_at > now()"
    ))
    .bind(token::hash(code))
    .fetch_optional(db)
    .await?)
}

/// Takes the live result behind `code`, once, on the caller's connection
/// (inside the transaction that acts on it, so a rollback keeps it).
pub async fn take_result(conn: &mut PgConnection, code: &str) -> Result<Option<Stored>, ApiError> {
    if !token::is_valid(code) {
        return Ok(None);
    }
    Ok(sqlx::query_as::<_, Stored>(concat!(
        "DELETE FROM oidc_results WHERE code_hash = $1 AND expires_at > now() RETURNING ",
        stored_columns!()
    ))
    .bind(token::hash(code))
    .fetch_optional(&mut *conn)
    .await?)
}

/// Turns a desktop result into a new account's name step: it lives
/// [`SIGNUP_TTL`] from now. `false` when the result is gone.
pub async fn await_name(db: &PgPool, code: &str) -> Result<bool, ApiError> {
    let updated = sqlx::query(
        "UPDATE oidc_results SET signup = true, expires_at = now() + make_interval(secs => $2) \
         WHERE code_hash = $1 AND expires_at > now()",
    )
    .bind(token::hash(code))
    .bind(SIGNUP_TTL.as_secs_f64())
    .execute(db)
    .await?;
    Ok(updated.rows_affected() == 1)
}
