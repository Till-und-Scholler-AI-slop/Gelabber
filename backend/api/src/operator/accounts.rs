//! `/api/operator/accounts`: the page "Konten". Operators list accounts and
//! disable or enable them. A disabled account cannot sign in
//! (`auth::signed_in`), its cookies resolve to nobody (`session::SESSION_ROW`)
//! and disabling ends its sessions, sockets and media tickets at once. The
//! servers it owns stay as they are.
//!
//! An account whose address was never confirmed (GEL-40) gets a new
//! confirmation mail here, or its address marked confirmed once the operator
//! checked with its owner. Marking it confirmed vouches for whoever chose
//! the password: that is why a provider sign-in refuses such an account
//! (`auth::oidc::link`, case 5).

use axum::extract::{Query, State};
use axum::response::Json;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgConnection;
use tracing::{info, warn};
use uuid::Uuid;

use super::access::{self, Operator};
use super::audit::{self, Action, Targets};
use crate::auth::{self, session, validate};
use crate::error::{ApiError, FieldErrors};
use crate::json::Body;
use crate::limits::{self, MailBudget};
use crate::path::Id;
use crate::state::AppState;

/// Rows per page of the list.
pub const PAGE: i64 = 50;
/// Longest search text.
pub const QUERY_MAX: usize = 100;
/// Longest reason for disabling (the column's check).
pub const REASON_MAX: usize = 500;

/// One account as the operator sees it.
#[derive(Debug, Serialize)]
pub struct Account {
    pub id: Uuid,
    pub name: String,
    pub email: String,
    pub avatar_url: Option<String>,
    pub created_at: DateTime<Utc>,
    /// `active`, `pending` (registered while mail is on, address not
    /// confirmed yet), `unconfirmed` (registered while mail was off, never
    /// confirmed, and mail is on now: it cannot sign in until confirmed) or
    /// `disabled`.
    pub status: &'static str,
    /// Whether the address is confirmed, also while mail is off.
    pub email_verified: bool,
    pub disabled_at: Option<DateTime<Utc>>,
    pub disabled_reason: Option<String>,
    /// Named in `API_OPERATOR_EMAILS` (and confirmed while mail is on).
    pub operator: bool,
    pub owned_servers: i64,
    /// Every server the account is a member of, its own included.
    pub member_servers: i64,
    pub live_sessions: i64,
}

#[derive(Debug, sqlx::FromRow)]
struct AccountRow {
    id: Uuid,
    name: String,
    email: String,
    avatar_url: Option<String>,
    created_at: DateTime<Utc>,
    disabled_at: Option<DateTime<Utc>>,
    disabled_reason: Option<String>,
    pending: bool,
    verified: bool,
    owned_servers: i64,
    member_servers: i64,
    live_sessions: i64,
}

impl AccountRow {
    fn into_account(self, state: &AppState) -> Account {
        let status = if self.disabled_at.is_some() {
            "disabled"
        } else if self.pending {
            "pending"
        } else if !self.verified && state.mail.is_some() {
            "unconfirmed"
        } else {
            "active"
        };
        Account {
            operator: access::is_operator_address(state, &self.email, self.verified),
            email_verified: self.verified,
            id: self.id,
            name: self.name,
            email: self.email,
            avatar_url: self.avatar_url,
            created_at: self.created_at,
            status,
            disabled_at: self.disabled_at,
            disabled_reason: self.disabled_reason,
            owned_servers: self.owned_servers,
            member_servers: self.member_servers,
            live_sessions: self.live_sessions,
        }
    }
}

/// The account columns plus the counts, over `users u`; a macro so the
/// queries stay `&'static str` (`concat!`).
macro_rules! account_select {
    () => {
        "SELECT u.id, u.name, u.email, u.avatar_url, u.created_at, u.disabled_at, \
             u.disabled_reason, \
             (u.registration_pending AND u.email_verified_at IS NULL) AS pending, \
             u.email_verified_at IS NOT NULL AS verified, \
             (SELECT COUNT(*) FROM servers s WHERE s.owner_id = u.id) AS owned_servers, \
             (SELECT COUNT(*) FROM server_members m WHERE m.user_id = u.id) AS member_servers, \
             (SELECT COUNT(*) FROM sessions x \
                 WHERE x.user_id = u.id AND x.expires_at > now()) AS live_sessions \
         FROM users u "
    };
}

/// The list's search (`$1`, an `ILIKE` pattern or NULL) and filter (`$2`).
/// An account under deletion (`deleted_at`) is left out.
macro_rules! account_filter {
    () => {
        "WHERE u.deleted_at IS NULL \
           AND ($1::text IS NULL \
             OR u.name ILIKE $1 ESCAPE '\\' OR u.email ILIKE $1 ESCAPE '\\') \
           AND CASE $2 \
             WHEN 'disabled' THEN u.disabled_at IS NOT NULL \
             WHEN 'pending' THEN u.disabled_at IS NULL \
                 AND u.registration_pending AND u.email_verified_at IS NULL \
             WHEN 'unconfirmed' THEN u.disabled_at IS NULL AND u.email_verified_at IS NULL \
             ELSE true END "
    };
}

/// Which accounts the list shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Filter {
    All,
    Disabled,
    Pending,
    /// Every address not confirmed yet, pending registrations included.
    Unconfirmed,
}

impl Filter {
    fn parse(raw: Option<&str>) -> Option<Self> {
        match raw.unwrap_or("all") {
            "all" => Some(Self::All),
            "disabled" => Some(Self::Disabled),
            "pending" => Some(Self::Pending),
            "unconfirmed" => Some(Self::Unconfirmed),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Disabled => "disabled",
            Self::Pending => "pending",
            Self::Unconfirmed => "unconfirmed",
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ListParams {
    // Strings, so that a bad value is a field error, not axum's plain 400.
    q: Option<String>,
    filter: Option<String>,
    offset: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AccountPage {
    pub accounts: Vec<Account>,
    pub total: i64,
}

/// `q` for `ILIKE`: trimmed, `None` when empty, with `\`, `%` and `_`
/// matched literally.
pub(super) fn search_pattern(raw: Option<&str>, errors: &mut FieldErrors) -> Option<String> {
    let q = raw.unwrap_or_default().trim();
    if q.chars().count() > QUERY_MAX {
        errors.insert("q", "too_long");
        return None;
    }
    if q.chars().any(char::is_control) {
        // Postgres refuses NUL in text, which would answer 500.
        errors.insert("q", "invalid");
        return None;
    }
    if q.is_empty() {
        return None;
    }
    let mut pattern = String::with_capacity(q.len() + 2);
    pattern.push('%');
    for c in q.chars() {
        if matches!(c, '\\' | '%' | '_') {
            pattern.push('\\');
        }
        pattern.push(c);
    }
    pattern.push('%');
    Some(pattern)
}

/// A page start: a whole number from 0, default 0.
pub fn offset(raw: Option<&str>, errors: &mut FieldErrors) -> i64 {
    match raw.map(str::parse::<i64>) {
        None => 0,
        Some(Ok(value)) if value >= 0 => value,
        Some(_) => {
            errors.insert("offset", "invalid");
            0
        }
    }
}

/// `GET /api/operator/accounts?q=&filter=all|disabled|pending|unconfirmed&offset=`:
/// newest first, [`PAGE`] at a time. `q` matches name or address.
async fn list(
    State(state): State<AppState>,
    op: Operator,
    Query(params): Query<ListParams>,
) -> Result<Json<AccountPage>, ApiError> {
    let mut errors = FieldErrors::new();
    let pattern = search_pattern(params.q.as_deref(), &mut errors);
    let filter = Filter::parse(params.filter.as_deref()).or_else(|| {
        errors.insert("filter", "invalid");
        None
    });
    let offset = offset(params.offset.as_deref(), &mut errors);
    validate::finish(errors)?;
    let filter = filter.expect("validated");

    let total: i64 =
        sqlx::query_scalar(concat!("SELECT COUNT(*) FROM users u ", account_filter!()))
            .bind(&pattern)
            .bind(filter.as_str())
            .fetch_one(&state.db)
            .await?;
    let rows: Vec<AccountRow> = sqlx::query_as(concat!(
        account_select!(),
        account_filter!(),
        "ORDER BY u.created_at DESC, u.id DESC LIMIT $3 OFFSET $4"
    ))
    .bind(&pattern)
    .bind(filter.as_str())
    .bind(PAGE)
    .bind(offset)
    .fetch_all(&state.db)
    .await?;

    let mut conn = state.db.acquire().await?;
    audit::record(
        &mut conn,
        &op,
        Action::AccountsList,
        Targets::default(),
        json!({
            "q": params.q.as_deref().map(str::trim).unwrap_or_default(),
            "filter": filter.as_str(),
            "offset": offset,
        }),
    )
    .await?;
    Ok(Json(AccountPage {
        accounts: rows
            .into_iter()
            .map(|row| row.into_account(&state))
            .collect(),
        total,
    }))
}

async fn load(
    state: &AppState,
    conn: &mut PgConnection,
    id: Uuid,
) -> Result<Option<Account>, ApiError> {
    let row: Option<AccountRow> = sqlx::query_as(concat!(
        account_select!(),
        "WHERE u.id = $1 AND u.deleted_at IS NULL"
    ))
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row.map(|row| row.into_account(state)))
}

/// The target row, locked for the change: its address, whether that is
/// confirmed, and whether it is disabled. `None` for an account under
/// deletion, which the console no longer shows.
async fn lock_target(
    conn: &mut PgConnection,
    id: Uuid,
) -> Result<Option<(String, bool, bool)>, ApiError> {
    Ok(sqlx::query_as(
        "SELECT email, email_verified_at IS NOT NULL, disabled_at IS NOT NULL \
         FROM users WHERE id = $1 AND deleted_at IS NULL FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?)
}

#[derive(Debug, Deserialize)]
pub struct DisableBody {
    #[serde(default)]
    reason: Option<String>,
}

/// `POST /api/operator/accounts/{id}/disable`. Operators cannot disable
/// themselves nor each other: who is one is set by the deployment. An
/// account that is disabled already stays as it is, without an audit row.
async fn disable(
    State(state): State<AppState>,
    op: Operator,
    Id(id): Id,
    Body(body): Body<DisableBody>,
) -> Result<Json<Account>, ApiError> {
    let reason = body
        .reason
        .as_deref()
        .map(str::trim)
        .filter(|reason| !reason.is_empty());
    if reason.is_some_and(|reason| reason.chars().count() > REASON_MAX) {
        return Err(ApiError::Validation(FieldErrors::from([(
            "reason", "too_long",
        )])));
    }
    if reason.is_some_and(|reason| reason.chars().any(char::is_control)) {
        return Err(ApiError::Validation(FieldErrors::from([(
            "reason", "invalid",
        )])));
    }

    let mut tx = state.db.begin().await?;
    let (email, verified, disabled) = lock_target(&mut tx, id).await?.ok_or(ApiError::NotFound)?;
    if id == op.user.id {
        return Err(ApiError::Validation(FieldErrors::from([(
            "account", "self",
        )])));
    }
    if access::is_operator_address(&state, &email, verified) {
        return Err(ApiError::Forbidden(
            "Operators are set in API_OPERATOR_EMAILS.",
        ));
    }
    if !disabled {
        sqlx::query(
            "UPDATE users SET disabled_at = now(), disabled_reason = $2, updated_at = now() \
             WHERE id = $1",
        )
        .bind(id)
        .bind(reason)
        .execute(&mut *tx)
        .await?;
        audit::record(
            &mut tx,
            &op,
            Action::AccountsDisable,
            Targets {
                user: Some(id),
                ..Targets::default()
            },
            json!({ "reason": reason }),
        )
        .await?;
    }
    tx.commit().await?;

    if !disabled {
        // The account is disabled now whatever happens here: its cookies
        // resolve to nobody. This closes what they still hold open.
        if let Err(err) = session::revoke_all(&state, id).await {
            warn!(user_id = %id, error = err.code(), "sessions of a disabled account not revoked");
        }
    }
    let mut conn = state.db.acquire().await?;
    let account = load(&state, &mut conn, id)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(account))
}

/// `POST /api/operator/accounts/{id}/enable`. The account can sign in
/// again; an account that is not disabled stays as it is. Either way the
/// call writes an audit row, with `changed` saying which.
async fn enable(
    State(state): State<AppState>,
    op: Operator,
    Id(id): Id,
) -> Result<Json<Account>, ApiError> {
    let mut tx = state.db.begin().await?;
    let (_, _, disabled) = lock_target(&mut tx, id).await?.ok_or(ApiError::NotFound)?;
    if disabled {
        sqlx::query(
            "UPDATE users SET disabled_at = NULL, disabled_reason = NULL, updated_at = now() \
             WHERE id = $1",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;
    }
    audit::record(
        &mut tx,
        &op,
        Action::AccountsEnable,
        Targets {
            user: Some(id),
            ..Targets::default()
        },
        json!({ "changed": disabled }),
    )
    .await?;
    let account = load(&state, &mut tx, id).await?.ok_or(ApiError::NotFound)?;
    tx.commit().await?;
    Ok(Json(account))
}

/// `POST /api/operator/accounts/{id}/resend-confirmation`: mails the
/// account a new confirmation link, as a password sign-in would. Only for
/// an address that is not confirmed, of an account that is not disabled
/// (`422 {"account":"stale"}` otherwise: the page was out of date), and
/// only while mail is on (`422 {"mail":"unavailable"}`). The mail comes out
/// of the address's registration mails, so past five an hour the answer is
/// `429` and nothing is written.
async fn resend_confirmation(
    State(state): State<AppState>,
    op: Operator,
    Id(id): Id,
) -> Result<Json<Account>, ApiError> {
    let Some(mailer) = state.mail.clone() else {
        return Err(ApiError::Validation(FieldErrors::from([(
            "mail",
            "unavailable",
        )])));
    };
    let mut tx = state.db.begin().await?;
    let (email, verified, disabled) = lock_target(&mut tx, id).await?.ok_or(ApiError::NotFound)?;
    if verified || disabled {
        return Err(ApiError::Validation(FieldErrors::from([(
            "account", "stale",
        )])));
    }
    // Counted under the same form as the link a sign-in sends.
    let email = validate::canonical(&email);
    audit::record(
        &mut tx,
        &op,
        Action::AccountsResend,
        Targets {
            user: Some(id),
            ..Targets::default()
        },
        json!({}),
    )
    .await?;
    let account = load(&state, &mut tx, id).await?.ok_or(ApiError::NotFound)?;
    limits::mail_reserved(&state, MailBudget::Registration, &email)
        .map_err(|retry_after| ApiError::RateLimited { retry_after })?;
    if let Err(err) = tx.commit().await {
        limits::mail_unsent(&state, MailBudget::Registration, &email);
        return Err(err.into());
    }
    auth::send_confirmation(&state, &mailer, id, &email);
    Ok(Json(account))
}

/// `POST /api/operator/accounts/{id}/confirm`: marks the address
/// confirmed, for an owner the operator checked with, and drops the
/// account's open confirmation links. The audit row's `changed` says
/// whether it was unconfirmed.
///
/// Only for an account registered while mail was off. A pending
/// registration answers `422 {"account":"pending"}`: anyone who registers
/// the address replaces its password (`user::insert_unverified_in`), so
/// neither the operator nor the owner can know whose password confirming
/// it would vouch for. Its owner gets a new link instead. A disabled
/// account stays unconfirmed (`422 {"account":"stale"}`), as its own link
/// would leave it.
async fn confirm(
    State(state): State<AppState>,
    op: Operator,
    Id(id): Id,
) -> Result<Json<Account>, ApiError> {
    let mut tx = state.db.begin().await?;
    let (_, verified, disabled) = lock_target(&mut tx, id).await?.ok_or(ApiError::NotFound)?;
    if !verified {
        if disabled {
            return Err(ApiError::Validation(FieldErrors::from([(
                "account", "stale",
            )])));
        }
        // The row is locked, so no registration replaces it after this.
        let pending: bool =
            sqlx::query_scalar("SELECT registration_pending FROM users WHERE id = $1")
                .bind(id)
                .fetch_one(&mut *tx)
                .await?;
        if pending {
            return Err(ApiError::Validation(FieldErrors::from([(
                "account", "pending",
            )])));
        }
        sqlx::query(
            "UPDATE users SET email_verified_at = now(), updated_at = now() \
             WHERE id = $1",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM account_tokens WHERE user_id = $1 AND purpose = 'verify_email'")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        info!(user_id = %id, "e-mail address confirmed by an operator");
    }
    audit::record(
        &mut tx,
        &op,
        Action::AccountsConfirm,
        Targets {
            user: Some(id),
            ..Targets::default()
        },
        json!({ "changed": !verified }),
    )
    .await?;
    let account = load(&state, &mut tx, id).await?.ok_or(ApiError::NotFound)?;
    tx.commit().await?;
    Ok(Json(account))
}

pub fn router() -> axum::Router<AppState> {
    use axum::routing::{get, post};
    axum::Router::new()
        .route("/api/operator/accounts", get(list))
        .route("/api/operator/accounts/{id}/disable", post(disable))
        .route("/api/operator/accounts/{id}/enable", post(enable))
        .route(
            "/api/operator/accounts/{id}/resend-confirmation",
            post(resend_confirmation),
        )
        .route("/api/operator/accounts/{id}/confirm", post(confirm))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_text_is_matched_literally() {
        let mut errors = FieldErrors::new();
        assert_eq!(
            search_pattern(Some("  50%_a\\b "), &mut errors).as_deref(),
            Some("%50\\%\\_a\\\\b%")
        );
        assert_eq!(search_pattern(Some("   "), &mut errors), None);
        assert_eq!(search_pattern(None, &mut errors), None);
        assert!(errors.is_empty());
        assert_eq!(search_pattern(Some(&"x".repeat(101)), &mut errors), None);
        assert_eq!(errors.get("q"), Some(&"too_long"));
        let mut errors = FieldErrors::new();
        assert!(search_pattern(Some(&"x".repeat(100)), &mut errors).is_some());
        assert_eq!(search_pattern(Some("a\0b"), &mut errors), None);
        assert_eq!(errors.get("q"), Some(&"invalid"));
    }

    #[test]
    fn offsets_are_whole_numbers_from_zero() {
        let mut errors = FieldErrors::new();
        assert_eq!(offset(None, &mut errors), 0);
        assert_eq!(offset(Some("50"), &mut errors), 50);
        assert!(errors.is_empty());
        offset(Some("-1"), &mut errors);
        assert_eq!(errors.get("offset"), Some(&"invalid"));
        let mut errors = FieldErrors::new();
        offset(Some("x"), &mut errors);
        assert_eq!(errors.get("offset"), Some(&"invalid"));
    }

    #[test]
    fn filters_are_closed() {
        assert_eq!(Filter::parse(None), Some(Filter::All));
        assert_eq!(Filter::parse(Some("pending")), Some(Filter::Pending));
        assert_eq!(Filter::parse(Some("disabled")), Some(Filter::Disabled));
        assert_eq!(Filter::parse(Some("blocked")), None);
    }
}
