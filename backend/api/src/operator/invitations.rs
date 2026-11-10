//! `/api/operator/invitations`: the page "Beta-Einladungen". Operators
//! create invitation links, optionally for one address and optionally
//! mailed, list them and revoke unused ones. On an invitation-only instance
//! a link admits one new account (`operator::admission`); on an open one it
//! only pre-fills the form.

use axum::extract::{Query, State};
use axum::http::header::HOST;
use axum::http::uri::Authority;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Json, Response};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgConnection;
use tracing::warn;
use uuid::Uuid;

use super::access::Operator;
use super::accounts::{PAGE, offset};
use super::audit::{self, Action, Targets};
use crate::auth::{self, validate};
use crate::error::{ApiError, FieldErrors};
use crate::json::Body;
use crate::limits::{self, MailBudget};
use crate::mail::Mail;
use crate::path::Id;
use crate::servers::invite;
use crate::state::AppState;

/// Longest note (the column's check).
pub const NOTE_MAX: usize = 200;
/// The validity periods the form offers, in days; `null` never expires.
pub const EXPIRES_IN_DAYS: [i64; 3] = [1, 7, 30];

pub fn router() -> axum::Router<AppState> {
    use axum::routing::{delete, get};
    axum::Router::new()
        .route("/api/operator/invitations", get(list).post(create))
        .route("/api/operator/invitations/{id}", delete(revoke))
}

/// Someone an invitation names: who created it, who used it.
#[derive(Debug, Serialize)]
pub struct Person {
    pub id: Uuid,
    pub name: String,
}

/// One invitation as the operator sees it.
#[derive(Debug, Serialize)]
pub struct Invitation {
    pub id: Uuid,
    pub code: String,
    /// `<APP_PUBLIC_URL or the request's origin>/register?invitation=<code>`.
    pub link: String,
    pub email: Option<String>,
    pub note: Option<String>,
    pub created_by: Option<Person>,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    /// `open`, `used`, `expired` or `revoked`.
    pub status: &'static str,
    pub used_by: Option<Person>,
    pub used_at: Option<DateTime<Utc>>,
}

#[derive(Debug, sqlx::FromRow)]
struct InvitationRow {
    id: Uuid,
    code: String,
    email: Option<String>,
    note: Option<String>,
    created_by: Option<Uuid>,
    created_by_name: Option<String>,
    created_at: DateTime<Utc>,
    expires_at: Option<DateTime<Utc>>,
    used_by: Option<Uuid>,
    used_by_name: Option<String>,
    used_at: Option<DateTime<Utc>>,
    revoked_at: Option<DateTime<Utc>>,
    expired: bool,
}

impl InvitationRow {
    fn into_invitation(self, base: &str) -> Invitation {
        let status = if self.used_at.is_some() {
            "used"
        } else if self.revoked_at.is_some() {
            "revoked"
        } else if self.expired {
            "expired"
        } else {
            "open"
        };
        let person = |id: Option<Uuid>, name: Option<String>| match (id, name) {
            (Some(id), Some(name)) => Some(Person { id, name }),
            _ => None,
        };
        Invitation {
            link: format!("{base}/register?invitation={}", self.code),
            status,
            created_by: person(self.created_by, self.created_by_name),
            used_by: person(self.used_by, self.used_by_name),
            id: self.id,
            code: self.code,
            email: self.email,
            note: self.note,
            created_at: self.created_at,
            expires_at: self.expires_at,
            used_at: self.used_at,
        }
    }
}

/// The invitation columns over `beta_invitations b`; a macro so the queries
/// stay `&'static str` (`concat!`).
macro_rules! invitation_select {
    () => {
        "SELECT b.id, b.code, b.email, b.note, b.created_by, c.name AS created_by_name, \
             b.created_at, b.expires_at, b.used_by, u.name AS used_by_name, b.used_at, \
             b.revoked_at, (b.expires_at IS NOT NULL AND b.expires_at <= now()) AS expired \
         FROM beta_invitations b \
         LEFT JOIN users c ON c.id = b.created_by \
         LEFT JOIN users u ON u.id = b.used_by "
    };
}

/// The list's status filter (`$1`).
macro_rules! invitation_filter {
    () => {
        "WHERE CASE $1 \
             WHEN 'open' THEN b.used_at IS NULL AND b.revoked_at IS NULL \
                 AND (b.expires_at IS NULL OR b.expires_at > now()) \
             WHEN 'used' THEN b.used_at IS NOT NULL \
             ELSE true END "
    };
}

/// Which invitations the list shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Filter {
    Open,
    Used,
    All,
}

impl Filter {
    fn parse(raw: Option<&str>) -> Option<Self> {
        match raw.unwrap_or("all") {
            "open" => Some(Self::Open),
            "used" => Some(Self::Used),
            "all" => Some(Self::All),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Used => "used",
            Self::All => "all",
        }
    }
}

/// Where links point: `APP_PUBLIC_URL`, or else the host the request
/// reached, with `https` when cookies are `Secure`. Without either the
/// link is relative to the page.
fn link_base(state: &AppState, headers: &HeaderMap) -> String {
    if let Some(url) = &state.public_url {
        return url.as_str().trim_end_matches('/').to_owned();
    }
    let host = headers
        .get(HOST)
        .and_then(|value| value.to_str().ok())
        .filter(|host| !host.contains('@') && host.parse::<Authority>().is_ok());
    match host {
        Some(host) => {
            let scheme = if state.cookie_secure { "https" } else { "http" };
            format!("{scheme}://{host}")
        }
        None => String::new(),
    }
}

#[derive(Debug, Deserialize)]
pub struct ListParams {
    // Strings, so that a bad value is a field error, not axum's plain 400.
    status: Option<String>,
    offset: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct InvitationPage {
    pub invitations: Vec<Invitation>,
    pub total: i64,
}

/// `GET /api/operator/invitations?status=open|used|all&offset=`: newest
/// first, [`PAGE`] at a time.
async fn list(
    State(state): State<AppState>,
    op: Operator,
    headers: HeaderMap,
    Query(params): Query<ListParams>,
) -> Result<Json<InvitationPage>, ApiError> {
    let mut errors = FieldErrors::new();
    let filter = Filter::parse(params.status.as_deref()).or_else(|| {
        errors.insert("status", "invalid");
        None
    });
    let offset = offset(params.offset.as_deref(), &mut errors);
    validate::finish(errors)?;
    let filter = filter.expect("validated");

    let total: i64 = sqlx::query_scalar(concat!(
        "SELECT COUNT(*) FROM beta_invitations b ",
        invitation_filter!()
    ))
    .bind(filter.as_str())
    .fetch_one(&state.db)
    .await?;
    let rows: Vec<InvitationRow> = sqlx::query_as(concat!(
        invitation_select!(),
        invitation_filter!(),
        "ORDER BY b.created_at DESC, b.id DESC LIMIT $2 OFFSET $3"
    ))
    .bind(filter.as_str())
    .bind(PAGE)
    .bind(offset)
    .fetch_all(&state.db)
    .await?;

    let mut conn = state.db.acquire().await?;
    audit::record(
        &mut conn,
        &op,
        Action::InvitationsList,
        Targets::default(),
        json!({ "status": filter.as_str(), "offset": offset }),
    )
    .await?;
    let base = link_base(&state, &headers);
    Ok(Json(InvitationPage {
        invitations: rows
            .into_iter()
            .map(|row| row.into_invitation(&base))
            .collect(),
        total,
    }))
}

#[derive(Debug, Deserialize)]
pub struct CreateBody {
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    note: Option<String>,
    /// 1, 7 or 30; absent or `null` never expires.
    #[serde(default)]
    expires_in_days: Option<i64>,
    #[serde(default)]
    send_mail: Option<bool>,
}

/// `POST /api/operator/invitations`. With `send_mail` the link is mailed to
/// `email` after the response, in the operator's language (the invitee has
/// no account yet).
async fn create(
    State(state): State<AppState>,
    op: Operator,
    headers: HeaderMap,
    Body(body): Body<CreateBody>,
) -> Result<Response, ApiError> {
    let mut errors = FieldErrors::new();
    let email = match body.email.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(raw) => validate::new_email(raw, &mut errors).or_else(|| {
            // `required` cannot happen here; every other code is "invalid"
            // to the form.
            errors.insert("email", "invalid");
            None
        }),
    };
    let note = body
        .note
        .as_deref()
        .map(str::trim)
        .filter(|note| !note.is_empty());
    if note.is_some_and(|note| note.chars().count() > NOTE_MAX) {
        errors.insert("note", "too_long");
    } else if note.is_some_and(|note| note.chars().any(char::is_control)) {
        errors.insert("note", "invalid");
    }
    let expires_in_days = body.expires_in_days;
    if expires_in_days.is_some_and(|days| !EXPIRES_IN_DAYS.contains(&days)) {
        errors.insert("expires_in_days", "invalid");
    }
    let send_mail = body.send_mail.unwrap_or(false);
    if send_mail
        && (state.mail.is_none() || body.email.as_deref().is_none_or(|e| e.trim().is_empty()))
    {
        errors.insert("send_mail", "unavailable");
    }
    validate::finish(errors)?;
    let expires_at = expires_in_days.map(|days| Utc::now() + Duration::days(days));

    let mut tx = state.db.begin().await?;
    if let Some(email) = &email
        && has_account(&mut tx, email).await?
    {
        return Err(ApiError::Validation(FieldErrors::from([(
            "email", "taken",
        )])));
    }
    let id = insert(&mut tx, &op, email.as_deref(), note, expires_at).await?;
    audit::record(
        &mut tx,
        &op,
        Action::InvitationsCreate,
        Targets {
            invitation: Some(id),
            ..Targets::default()
        },
        json!({
            "email": email,
            "note": note,
            "expires_in_days": expires_in_days,
            "send_mail": send_mail,
        }),
    )
    .await?;
    let row: InvitationRow = sqlx::query_as(concat!(invitation_select!(), "WHERE b.id = $1"))
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;

    let invitation = row.into_invitation(&link_base(&state, &headers));
    if send_mail && let Some(email) = &email {
        if limits::mail_allowed(&state, MailBudget::Invitation, email) {
            auth::send_mail_to(
                &state,
                email,
                op.user.locale,
                Mail::BetaInvitation {
                    link: invitation.link.clone(),
                },
                MailBudget::Invitation,
            );
        } else {
            warn!(invitation_id = %id, "invitation mail dropped, hourly allowance used up");
        }
    }
    Ok((StatusCode::CREATED, Json(invitation)).into_response())
}

/// Whether `email` already belongs to an account that is not a pending
/// registration: one confirmed, or one from a time without mail. A pending
/// one is taken over by the next registration, so it may be invited.
async fn has_account(conn: &mut PgConnection, email: &str) -> Result<bool, ApiError> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM users WHERE lower(email) = $1 \
             AND NOT (registration_pending AND email_verified_at IS NULL))",
    )
    .bind(email)
    .fetch_one(&mut *conn)
    .await?)
}

/// Inserts the row with a fresh code; a code collision (≈5·10¹⁷ codes)
/// is retried.
async fn insert(
    conn: &mut PgConnection,
    op: &Operator,
    email: Option<&str>,
    note: Option<&str>,
    expires_at: Option<DateTime<Utc>>,
) -> Result<Uuid, ApiError> {
    for _ in 0..3 {
        let id: Option<Uuid> = sqlx::query_scalar(
            "INSERT INTO beta_invitations (code, email, note, created_by, expires_at) \
             VALUES ($1, $2, $3, $4, $5) ON CONFLICT (code) DO NOTHING RETURNING id",
        )
        .bind(invite::generate_code())
        .bind(email)
        .bind(note)
        .bind(op.user.id)
        .bind(expires_at)
        .fetch_optional(&mut *conn)
        .await?;
        if let Some(id) = id {
            return Ok(id);
        }
    }
    Err(ApiError::Internal(
        "could not allocate a unique invitation code".to_owned(),
    ))
}

/// `DELETE /api/operator/invitations/{id}`: the link stops working. A used
/// invitation cannot be revoked; a revoked one stays as it is. Either way
/// the call writes an audit row, with `changed` saying which.
async fn revoke(
    State(state): State<AppState>,
    op: Operator,
    Id(id): Id,
) -> Result<StatusCode, ApiError> {
    let mut tx = state.db.begin().await?;
    let (used, revoked): (bool, bool) = sqlx::query_as(
        "SELECT used_at IS NOT NULL, revoked_at IS NOT NULL FROM beta_invitations \
         WHERE id = $1 FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(ApiError::NotFound)?;
    if used {
        return Err(ApiError::Validation(FieldErrors::from([(
            "invitation",
            "used",
        )])));
    }
    if !revoked {
        sqlx::query("UPDATE beta_invitations SET revoked_at = now() WHERE id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    audit::record(
        &mut tx,
        &op,
        Action::InvitationsRevoke,
        Targets {
            invitation: Some(id),
            ..Targets::default()
        },
        json!({ "changed": !revoked }),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_are_closed() {
        assert_eq!(Filter::parse(None), Some(Filter::All));
        assert_eq!(Filter::parse(Some("open")), Some(Filter::Open));
        assert_eq!(Filter::parse(Some("used")), Some(Filter::Used));
        assert_eq!(Filter::parse(Some("revoked")), None);
    }
}
