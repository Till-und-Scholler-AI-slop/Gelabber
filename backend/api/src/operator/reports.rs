//! Reports: `POST /api/servers/{id}/reports` for the people who can see a
//! server, and the page "Meldungen" for operators (`/api/operator/reports`).
//!
//! Every report goes through [`file`], also the directory's report button
//! (discovery). One person has at most one open report per server; filing
//! again returns that one. Reporters are shown only to operators; owners
//! never see reports. Blocking or delisting a server closes its open
//! reports as `actioned` (`operator::servers`).

use std::time::Duration;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::Json;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgConnection;
use tracing::info;
use uuid::Uuid;

use super::access::Operator;
use super::accounts::{PAGE, offset};
use super::audit::{self, Action, Targets};
use crate::auth::session::CurrentUser;
use crate::auth::validate;
use crate::directory::state_of;
use crate::error::{ApiError, FieldErrors};
use crate::json::Body;
use crate::path::Id;
use crate::servers::membership;
use crate::state::AppState;

/// Longest details of a report (the column's check).
pub const DETAILS_MAX: usize = 1000;
/// Longest note an operator leaves when closing a report (the column's check).
pub const NOTE_MAX: usize = 500;
/// Reports one account may file per [`REPORT_WINDOW`] (limiter bucket `report`).
pub const REPORTS_PER_HOUR: u32 = 10;
const REPORT_WINDOW: Duration = Duration::from_secs(3600);

/// The reasons a report may give (the column's check).
pub const REASONS: [&str; 5] = ["spam", "harassment", "sexual", "illegal", "other"];

pub fn router() -> axum::Router<AppState> {
    use axum::routing::{get, post};
    axum::Router::new()
        .route("/api/servers/{id}/reports", post(create))
        .route("/api/operator/reports", get(list))
        .route("/api/operator/reports/{id}/resolve", post(resolve))
}

/// A report to file. `reason` and `details` are checked by [`file`];
/// `snapshot` keeps what the reporter saw (the directory listing), if any.
#[derive(Debug, Clone)]
pub struct NewReport {
    pub server_id: Uuid,
    pub reporter_id: Uuid,
    pub reason: String,
    pub details: Option<String>,
    pub snapshot: Option<serde_json::Value>,
}

/// Text a person typed: no control characters but line breaks and tabs.
/// Postgres refuses NUL, which would otherwise answer 500.
fn has_control(text: &str) -> bool {
    text.chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
}

/// The reason and the trimmed details, or the field errors.
fn checked(report: &NewReport) -> Result<(&'static str, String), ApiError> {
    let mut errors = FieldErrors::new();
    let reason = REASONS
        .iter()
        .copied()
        .find(|reason| *reason == report.reason);
    if reason.is_none() {
        errors.insert("reason", "invalid");
    }
    let details = report.details.as_deref().unwrap_or_default().trim();
    if details.chars().count() > DETAILS_MAX {
        errors.insert("details", "too_long");
    } else if has_control(details) {
        errors.insert("details", "invalid");
    } else if details.is_empty() && reason == Some("other") {
        errors.insert("details", "required");
    }
    validate::finish(errors)?;
    Ok((reason.expect("validated"), details.to_owned()))
}

/// Files `report` on `conn`. Returns its id and whether it is new: when the
/// reporter already has an open report on the server, that one comes back
/// unchanged. The caller has checked that the reporter may see the server;
/// an unknown server or reporter is `404`.
pub async fn file(conn: &mut PgConnection, report: NewReport) -> Result<(Uuid, bool), ApiError> {
    let (reason, details) = checked(&report)?;
    // The open report may be closed between the insert and the read; then
    // the next insert goes through.
    for _ in 0..3 {
        let inserted: Option<Uuid> = sqlx::query_scalar(
            "INSERT INTO reports (server_id, reporter_id, reason, details, snapshot) \
             VALUES ($1, $2, $3, $4, $5) \
             ON CONFLICT (server_id, reporter_id) WHERE status = 'open' DO NOTHING \
             RETURNING id",
        )
        .bind(report.server_id)
        .bind(report.reporter_id)
        .bind(reason)
        .bind(&details)
        .bind(&report.snapshot)
        .fetch_optional(&mut *conn)
        .await
        .map_err(|err| match &err {
            sqlx::Error::Database(db) if db.code().as_deref() == Some("23503") => {
                ApiError::NotFound
            }
            _ => err.into(),
        })?;
        if let Some(id) = inserted {
            info!(report_id = %id, server_id = %report.server_id, "server reported");
            return Ok((id, true));
        }
        let open: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM reports \
             WHERE server_id = $1 AND reporter_id = $2 AND status = 'open'",
        )
        .bind(report.server_id)
        .bind(report.reporter_id)
        .fetch_optional(&mut *conn)
        .await?;
        if let Some(id) = open {
            return Ok((id, false));
        }
    }
    Err(ApiError::Internal(
        "report: the open report kept changing".to_owned(),
    ))
}

/// Counts one report of `user` against the `report` bucket (`429` past it).
/// The limiter is in memory, so with several API processes it counts per
/// process.
pub fn rate_limit(state: &AppState, user: Uuid) -> Result<(), ApiError> {
    state
        .limiter
        .hit("report", &user.to_string(), REPORTS_PER_HOUR, REPORT_WINDOW)
        .map_err(|retry_after| {
            state.metrics.rate_limited("report");
            ApiError::RateLimited { retry_after }
        })
}

#[derive(Debug, Deserialize)]
pub struct CreateBody {
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    details: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Filed {
    pub id: Uuid,
    pub created_at: DateTime<Utc>,
}

/// `POST /api/servers/{id}/reports`: `201` with a new report, `200` with the
/// reporter's open one. Members may report, and so may whoever can see the
/// server's directory listing; anyone else gets `404`, like every other
/// server route. When the caller can see the listing, the report keeps a
/// snapshot of it (`directory::report_snapshot`).
async fn create(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(server_id): Id,
    Body(body): Body<CreateBody>,
) -> Result<(StatusCode, Json<Filed>), ApiError> {
    let snapshot = match membership::load(&state.db, server_id, user.id).await {
        Ok(_) => crate::directory::report_snapshot(&state, server_id, user.id).await?,
        Err(ApiError::NotFound) => Some(
            crate::directory::report_snapshot(&state, server_id, user.id)
                .await?
                .ok_or(ApiError::NotFound)?,
        ),
        Err(err) => return Err(err),
    };
    rate_limit(&state, user.id)?;
    let mut conn = state.db.acquire().await?;
    let (id, created) = file(
        &mut conn,
        NewReport {
            server_id,
            reporter_id: user.id,
            reason: body.reason.unwrap_or_default(),
            details: body.details,
            snapshot,
        },
    )
    .await?;
    let created_at: DateTime<Utc> =
        sqlx::query_scalar("SELECT created_at FROM reports WHERE id = $1")
            .bind(id)
            .fetch_one(&mut *conn)
            .await?;
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(Filed { id, created_at })))
}

/// Someone named on a report.
#[derive(Debug, Serialize)]
pub struct Person {
    pub id: Uuid,
    pub name: String,
}

/// The reported server as it is now.
#[derive(Debug, Serialize)]
pub struct ReportedServer {
    pub id: Uuid,
    pub name: String,
    pub icon_url: Option<String>,
    pub blocked: bool,
    /// See [`state_of`].
    pub directory: &'static str,
}

/// One report as the operator sees it.
#[derive(Debug, Serialize)]
pub struct Report {
    pub id: Uuid,
    pub server: ReportedServer,
    /// `null` once the reporter's account is gone.
    pub reporter: Option<Person>,
    pub reason: String,
    pub details: String,
    /// `open`, `dismissed` or `actioned`.
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub resolved_by: Option<Person>,
    pub resolution_note: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
struct ReportRow {
    id: Uuid,
    server_id: Uuid,
    server_name: String,
    server_icon_url: Option<String>,
    server_blocked: bool,
    server_delisted: bool,
    server_listed: bool,
    reporter_id: Option<Uuid>,
    reporter_name: Option<String>,
    reason: String,
    details: String,
    status: String,
    created_at: DateTime<Utc>,
    resolved_at: Option<DateTime<Utc>>,
    resolved_by: Option<Uuid>,
    resolved_by_name: Option<String>,
    resolution_note: Option<String>,
}

fn person(id: Option<Uuid>, name: Option<String>) -> Option<Person> {
    Some(Person {
        id: id?,
        name: name?,
    })
}

impl From<ReportRow> for Report {
    fn from(row: ReportRow) -> Self {
        Self {
            id: row.id,
            server: ReportedServer {
                id: row.server_id,
                name: row.server_name,
                icon_url: row.server_icon_url,
                blocked: row.server_blocked,
                directory: state_of(row.server_delisted, row.server_listed),
            },
            reporter: person(row.reporter_id, row.reporter_name),
            reason: row.reason,
            details: row.details,
            status: row.status,
            created_at: row.created_at,
            resolved_at: row.resolved_at,
            resolved_by: person(row.resolved_by, row.resolved_by_name),
            resolution_note: row.resolution_note,
        }
    }
}

/// The report columns plus server and people, over `reports r`; a macro so
/// the queries stay `&'static str`.
macro_rules! report_select {
    () => {
        "SELECT r.id, r.server_id, s.name AS server_name, s.icon_url AS server_icon_url, \
             s.blocked_at IS NOT NULL AS server_blocked, \
             s.delisted_at IS NOT NULL AS server_delisted, \
             EXISTS (SELECT 1 FROM server_listings l \
                      WHERE l.server_id = s.id AND l.listed) AS server_listed, \
             r.reporter_id, rep.name AS reporter_name, r.reason, r.details, r.status, \
             r.created_at, r.resolved_at, r.resolved_by, res.name AS resolved_by_name, \
             r.resolution_note \
         FROM reports r \
         JOIN servers s ON s.id = r.server_id \
         LEFT JOIN users rep ON rep.id = r.reporter_id \
         LEFT JOIN users res ON res.id = r.resolved_by "
    };
}

/// Which reports the list shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Open,
    Closed,
}

impl Status {
    fn parse(raw: Option<&str>) -> Option<Self> {
        match raw.unwrap_or("open") {
            "open" => Some(Self::Open),
            "closed" => Some(Self::Closed),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Closed => "closed",
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ListParams {
    // Strings, so that a bad value is a field error, not axum's plain 400.
    status: Option<String>,
    offset: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ReportPage {
    pub reports: Vec<Report>,
    pub total: i64,
}

/// `GET /api/operator/reports?status=open|closed&offset=`: open reports
/// oldest first, closed ones most recently closed first, [`PAGE`] at a time.
async fn list(
    State(state): State<AppState>,
    op: Operator,
    Query(params): Query<ListParams>,
) -> Result<Json<ReportPage>, ApiError> {
    let mut errors = FieldErrors::new();
    let status = Status::parse(params.status.as_deref()).or_else(|| {
        errors.insert("status", "invalid");
        None
    });
    let offset = offset(params.offset.as_deref(), &mut errors);
    validate::finish(errors)?;
    let status = status.expect("validated");

    let (total, rows): (i64, Vec<ReportRow>) = match status {
        Status::Open => (
            sqlx::query_scalar("SELECT COUNT(*) FROM reports WHERE status = 'open'")
                .fetch_one(&state.db)
                .await?,
            sqlx::query_as(concat!(
                report_select!(),
                "WHERE r.status = 'open' ORDER BY r.created_at, r.id LIMIT $1 OFFSET $2"
            ))
            .bind(PAGE)
            .bind(offset)
            .fetch_all(&state.db)
            .await?,
        ),
        Status::Closed => (
            sqlx::query_scalar("SELECT COUNT(*) FROM reports WHERE status <> 'open'")
                .fetch_one(&state.db)
                .await?,
            sqlx::query_as(concat!(
                report_select!(),
                "WHERE r.status <> 'open' \
                 ORDER BY r.resolved_at DESC, r.id DESC LIMIT $1 OFFSET $2"
            ))
            .bind(PAGE)
            .bind(offset)
            .fetch_all(&state.db)
            .await?,
        ),
    };

    let mut conn = state.db.acquire().await?;
    audit::record(
        &mut conn,
        &op,
        Action::ReportsList,
        Targets::default(),
        json!({ "status": status.as_str(), "offset": offset }),
    )
    .await?;
    Ok(Json(ReportPage {
        reports: rows.into_iter().map(Report::from).collect(),
        total,
    }))
}

#[derive(Debug, Deserialize)]
pub struct ResolveBody {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    note: Option<String>,
}

/// The closing status and the trimmed note (empty is `None`), or the field
/// errors.
fn resolution(body: &ResolveBody) -> Result<(&'static str, Option<String>), ApiError> {
    let mut errors = FieldErrors::new();
    let status = match body.status.as_deref() {
        Some("dismissed") => Some("dismissed"),
        Some("actioned") => Some("actioned"),
        _ => {
            errors.insert("status", "invalid");
            None
        }
    };
    let note = body
        .note
        .as_deref()
        .map(str::trim)
        .filter(|note| !note.is_empty());
    if let Some(note) = note {
        if note.chars().count() > NOTE_MAX {
            errors.insert("note", "too_long");
        } else if has_control(note) {
            errors.insert("note", "invalid");
        }
    }
    validate::finish(errors)?;
    Ok((status.expect("validated"), note.map(str::to_owned)))
}

/// `POST /api/operator/reports/{id}/resolve` with `{"status":
/// "dismissed"|"actioned","note"?}`: closes an open report. It changes
/// nothing on the server; blocking and delisting have their own routes.
async fn resolve(
    State(state): State<AppState>,
    op: Operator,
    Id(id): Id,
    Body(body): Body<ResolveBody>,
) -> Result<Json<Report>, ApiError> {
    let (status, note) = resolution(&body)?;
    let mut tx = state.db.begin().await?;
    let (current, server_id): (String, Uuid) =
        sqlx::query_as("SELECT status, server_id FROM reports WHERE id = $1 FOR UPDATE")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(ApiError::NotFound)?;
    if current != "open" {
        return Err(ApiError::Validation(FieldErrors::from([(
            "report", "closed",
        )])));
    }
    sqlx::query(
        "UPDATE reports SET status = $2, resolved_by = $3, resolved_at = now(), \
             resolution_note = $4 \
         WHERE id = $1",
    )
    .bind(id)
    .bind(status)
    .bind(op.user.id)
    .bind(&note)
    .execute(&mut *tx)
    .await?;
    audit::record(
        &mut tx,
        &op,
        Action::ReportsResolve,
        Targets {
            report: Some(id),
            server: Some(server_id),
            ..Targets::default()
        },
        json!({ "status": status, "note": note }),
    )
    .await?;
    let row: ReportRow = sqlx::query_as(concat!(report_select!(), "WHERE r.id = $1"))
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Json(row.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new(reason: &str, details: Option<&str>) -> NewReport {
        NewReport {
            server_id: Uuid::nil(),
            reporter_id: Uuid::nil(),
            reason: reason.to_owned(),
            details: details.map(str::to_owned),
            snapshot: None,
        }
    }

    fn fields(result: Result<impl std::fmt::Debug, ApiError>) -> FieldErrors {
        match result {
            Err(ApiError::Validation(fields)) => fields,
            other => panic!("expected field errors, got {other:?}"),
        }
    }

    #[test]
    fn reports_take_the_known_reasons_and_bounded_details() {
        for reason in REASONS.into_iter().filter(|reason| *reason != "other") {
            assert_eq!(
                checked(&new(reason, None)).unwrap(),
                (reason, String::new())
            );
        }
        assert_eq!(
            checked(&new("other", Some("  Werbung\nüberall  "))).unwrap(),
            ("other", "Werbung\nüberall".to_owned())
        );
        assert_eq!(fields(checked(&new("abuse", None)))["reason"], "invalid");
        assert_eq!(fields(checked(&new("", None)))["reason"], "invalid");
        assert_eq!(fields(checked(&new("other", None)))["details"], "required");
        assert_eq!(
            fields(checked(&new("other", Some("   "))))["details"],
            "required"
        );
        assert!(checked(&new("spam", Some(&"ä".repeat(DETAILS_MAX)))).is_ok());
        assert_eq!(
            fields(checked(&new("spam", Some(&"x".repeat(DETAILS_MAX + 1)))))["details"],
            "too_long"
        );
        assert_eq!(
            fields(checked(&new("spam", Some("a\0b"))))["details"],
            "invalid"
        );
    }

    #[test]
    fn resolutions_take_a_closing_status_and_a_bounded_note() {
        let body = |status: Option<&str>, note: Option<&str>| ResolveBody {
            status: status.map(str::to_owned),
            note: note.map(str::to_owned),
        };
        assert_eq!(
            resolution(&body(Some("dismissed"), Some("  passt  "))).unwrap(),
            ("dismissed", Some("passt".to_owned()))
        );
        assert_eq!(
            resolution(&body(Some("actioned"), Some(" "))).unwrap(),
            ("actioned", None)
        );
        for status in [None, Some("open"), Some("closed")] {
            assert_eq!(fields(resolution(&body(status, None)))["status"], "invalid");
        }
        assert_eq!(
            fields(resolution(&body(
                Some("dismissed"),
                Some(&"x".repeat(NOTE_MAX + 1))
            )))["note"],
            "too_long"
        );
        assert_eq!(
            fields(resolution(&body(Some("dismissed"), Some("a\u{7}"))))["note"],
            "invalid"
        );
    }

    #[test]
    fn statuses_are_closed() {
        assert_eq!(Status::parse(None), Some(Status::Open));
        assert_eq!(Status::parse(Some("closed")), Some(Status::Closed));
        assert_eq!(Status::parse(Some("dismissed")), None);
    }
}
