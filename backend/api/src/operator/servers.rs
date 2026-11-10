//! `/api/operator/servers`: the page "Server". Operators list every server
//! of the instance, take one out of the directory or let it back in, and
//! block or unblock it.
//!
//! A blocked server stays in its members' list (`blocked: true`), but
//! `membership::load` answers `403 server_blocked` to everything except
//! leaving and deleting it. Blocking ends every member's seat and
//! subscriptions at once (`err server_blocked`). Delisting and blocking
//! both close the server's open reports as `actioned`.

use axum::extract::{Query, State};
use axum::response::Json;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgConnection;
use tracing::warn;
use uuid::Uuid;

use super::access::Operator;
use super::accounts::{PAGE, offset, search_pattern};
use super::audit::{self, Action, Targets};
use crate::auth::validate;
use crate::error::{ApiError, FieldErrors};
use crate::gateway::Recheck;
use crate::json::Body;
use crate::path::Id;
use crate::servers::{membership, moderation};
use crate::state::AppState;

/// Longest reason for delisting or blocking (the column's check).
pub const REASON_MAX: usize = 500;

pub fn router() -> axum::Router<AppState> {
    use axum::routing::{get, post};
    axum::Router::new()
        .route("/api/operator/servers", get(list))
        .route("/api/operator/servers/{id}/delist", post(delist))
        .route("/api/operator/servers/{id}/relist", post(relist))
        .route("/api/operator/servers/{id}/block", post(block))
        .route("/api/operator/servers/{id}/unblock", post(unblock))
}

/// The owner, so the operator can write to them.
#[derive(Debug, Serialize)]
pub struct Owner {
    pub id: Uuid,
    pub name: String,
    pub email: String,
}

/// One server as the operator sees it.
#[derive(Debug, Serialize)]
pub struct ServerAdmin {
    pub id: Uuid,
    pub name: String,
    pub icon_url: Option<String>,
    pub created_at: DateTime<Utc>,
    pub owner: Owner,
    pub members: i64,
    /// Bytes of the attachments of sent messages. Icons and link-preview
    /// images are not counted: their size is not stored.
    pub storage_bytes: i64,
    /// `off`, `listed` or `delisted`; see [`directory`].
    pub directory: &'static str,
    pub open_reports: i64,
    pub blocked_at: Option<DateTime<Utc>>,
    pub blocked_reason: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
struct ServerRow {
    id: Uuid,
    name: String,
    icon_url: Option<String>,
    created_at: DateTime<Utc>,
    owner_id: Uuid,
    owner_name: String,
    owner_email: String,
    members: i64,
    storage_bytes: i64,
    delisted: bool,
    open_reports: i64,
    blocked_at: Option<DateTime<Utc>>,
    blocked_reason: Option<String>,
}

impl From<ServerRow> for ServerAdmin {
    fn from(row: ServerRow) -> Self {
        Self {
            id: row.id,
            name: row.name,
            icon_url: row.icon_url,
            created_at: row.created_at,
            owner: Owner {
                id: row.owner_id,
                name: row.owner_name,
                email: row.owner_email,
            },
            members: row.members,
            storage_bytes: row.storage_bytes,
            directory: directory(row.delisted),
            open_reports: row.open_reports,
            blocked_at: row.blocked_at,
            blocked_reason: row.blocked_reason,
        }
    }
}

/// The server's place in the directory. `delisted` while an operator took it
/// out. The directory itself (`listed`) comes with the discovery package;
/// until then every other server is `off`.
pub fn directory(delisted: bool) -> &'static str {
    if delisted { "delisted" } else { "off" }
}

/// The server columns plus owner and counts, over `servers s` joined with
/// its owner `users o`; a macro so the queries stay `&'static str`.
macro_rules! server_select {
    () => {
        "SELECT s.id, s.name, s.icon_url, s.created_at, s.owner_id, \
             o.name AS owner_name, o.email AS owner_email, \
             (SELECT COUNT(*) FROM server_members m WHERE m.server_id = s.id) AS members, \
             COALESCE((SELECT SUM(a.size_bytes) FROM attachments a \
                         JOIN channels c ON c.id = a.channel_id \
                        WHERE c.server_id = s.id AND a.message_id IS NOT NULL), 0)::bigint \
                 AS storage_bytes, \
             s.delisted_at IS NOT NULL AS delisted, \
             (SELECT COUNT(*) FROM reports r \
                 WHERE r.server_id = s.id AND r.status = 'open') AS open_reports, \
             s.blocked_at, s.blocked_reason \
         FROM servers s JOIN users o ON o.id = s.owner_id "
    };
}

/// The list's search (`$1`, an `ILIKE` pattern or NULL: server name, owner
/// name or owner address) and filter (`$2`). `listed` is empty until the
/// directory exists.
macro_rules! server_filter {
    () => {
        "WHERE ($1::text IS NULL OR s.name ILIKE $1 ESCAPE '\\' \
             OR o.name ILIKE $1 ESCAPE '\\' OR o.email ILIKE $1 ESCAPE '\\') \
           AND CASE $2 \
             WHEN 'listed' THEN false \
             WHEN 'reported' THEN EXISTS (SELECT 1 FROM reports r \
                 WHERE r.server_id = s.id AND r.status = 'open') \
             WHEN 'blocked' THEN s.blocked_at IS NOT NULL \
             ELSE true END "
    };
}

/// Which servers the list shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Filter {
    All,
    Listed,
    Reported,
    Blocked,
}

impl Filter {
    fn parse(raw: Option<&str>) -> Option<Self> {
        match raw.unwrap_or("all") {
            "all" => Some(Self::All),
            "listed" => Some(Self::Listed),
            "reported" => Some(Self::Reported),
            "blocked" => Some(Self::Blocked),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Listed => "listed",
            Self::Reported => "reported",
            Self::Blocked => "blocked",
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
pub struct ServerPage {
    pub servers: Vec<ServerAdmin>,
    pub total: i64,
}

/// `GET /api/operator/servers?q=&filter=all|listed|reported|blocked&offset=`:
/// newest first, [`PAGE`] at a time.
async fn list(
    State(state): State<AppState>,
    op: Operator,
    Query(params): Query<ListParams>,
) -> Result<Json<ServerPage>, ApiError> {
    let mut errors = FieldErrors::new();
    let pattern = search_pattern(params.q.as_deref(), &mut errors);
    let filter = Filter::parse(params.filter.as_deref()).or_else(|| {
        errors.insert("filter", "invalid");
        None
    });
    let offset = offset(params.offset.as_deref(), &mut errors);
    validate::finish(errors)?;
    let filter = filter.expect("validated");

    let total: i64 = sqlx::query_scalar(concat!(
        "SELECT COUNT(*) FROM servers s JOIN users o ON o.id = s.owner_id ",
        server_filter!()
    ))
    .bind(&pattern)
    .bind(filter.as_str())
    .fetch_one(&state.db)
    .await?;
    let rows: Vec<ServerRow> = sqlx::query_as(concat!(
        server_select!(),
        server_filter!(),
        "ORDER BY s.created_at DESC, s.id DESC LIMIT $3 OFFSET $4"
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
        Action::ServersList,
        Targets::default(),
        json!({
            "q": params.q.as_deref().map(str::trim).unwrap_or_default(),
            "filter": filter.as_str(),
            "offset": offset,
        }),
    )
    .await?;
    Ok(Json(ServerPage {
        servers: rows.into_iter().map(ServerAdmin::from).collect(),
        total,
    }))
}

async fn load(conn: &mut PgConnection, id: Uuid) -> Result<ServerAdmin, ApiError> {
    let row: ServerRow = sqlx::query_as(concat!(server_select!(), "WHERE s.id = $1"))
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(row.into())
}

/// The server row locked for the change: whether it is delisted and
/// whether it is blocked. `None` for an unknown id.
async fn lock(conn: &mut PgConnection, id: Uuid) -> Result<Option<(bool, bool)>, ApiError> {
    Ok(sqlx::query_as(
        "SELECT delisted_at IS NOT NULL, blocked_at IS NOT NULL FROM servers \
         WHERE id = $1 FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?)
}

/// Closes the server's open reports as `actioned` by `op`. Returns how many.
async fn action_reports(
    conn: &mut PgConnection,
    server_id: Uuid,
    op: &Operator,
) -> Result<u64, ApiError> {
    Ok(sqlx::query(
        "UPDATE reports SET status = 'actioned', resolved_by = $2, resolved_at = now() \
         WHERE server_id = $1 AND status = 'open'",
    )
    .bind(server_id)
    .bind(op.user.id)
    .execute(&mut *conn)
    .await?
    .rows_affected())
}

fn server_target(id: Uuid) -> Targets {
    Targets {
        server: Some(id),
        ..Targets::default()
    }
}

/// A reason: trimmed, at most [`REASON_MAX`] characters, no control
/// characters. Empty is `None`, which `required` refuses.
fn reason(raw: Option<&str>, required: bool) -> Result<Option<String>, ApiError> {
    let reason = raw.map(str::trim).filter(|reason| !reason.is_empty());
    let error = match reason {
        None if required => Some("required"),
        Some(reason) if reason.chars().count() > REASON_MAX => Some("too_long"),
        Some(reason) if reason.chars().any(char::is_control) => Some("invalid"),
        _ => None,
    };
    match error {
        Some(code) => Err(ApiError::Validation(FieldErrors::from([("reason", code)]))),
        None => Ok(reason.map(str::to_owned)),
    }
}

#[derive(Debug, Deserialize)]
pub struct ReasonBody {
    #[serde(default)]
    reason: Option<String>,
}

/// `POST /api/operator/servers/{id}/delist`: out of the directory, and its
/// owner cannot list it again. Its open reports close as `actioned`, also
/// when it was delisted already; only a call that changes nothing writes
/// no audit row.
async fn delist(
    State(state): State<AppState>,
    op: Operator,
    Id(id): Id,
    Body(body): Body<ReasonBody>,
) -> Result<Json<ServerAdmin>, ApiError> {
    let reason = reason(body.reason.as_deref(), false)?;
    let mut tx = state.db.begin().await?;
    let (delisted, _) = lock(&mut tx, id).await?.ok_or(ApiError::NotFound)?;
    if !delisted {
        sqlx::query("UPDATE servers SET delisted_at = now(), updated_at = now() WHERE id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    let reports = action_reports(&mut tx, id, &op).await?;
    if !delisted || reports > 0 {
        audit::record(
            &mut tx,
            &op,
            Action::ServersDelist,
            server_target(id),
            json!({ "reason": reason, "reports": reports }),
        )
        .await?;
    }
    let server = load(&mut tx, id).await?;
    tx.commit().await?;
    Ok(Json(server))
}

/// `POST /api/operator/servers/{id}/relist`: clears the delisting. It does
/// not list the server; its owner may list it again.
async fn relist(
    State(state): State<AppState>,
    op: Operator,
    Id(id): Id,
) -> Result<Json<ServerAdmin>, ApiError> {
    let mut tx = state.db.begin().await?;
    let (delisted, _) = lock(&mut tx, id).await?.ok_or(ApiError::NotFound)?;
    if delisted {
        sqlx::query("UPDATE servers SET delisted_at = NULL, updated_at = now() WHERE id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        audit::record(
            &mut tx,
            &op,
            Action::ServersRelist,
            server_target(id),
            json!({}),
        )
        .await?;
    }
    let server = load(&mut tx, id).await?;
    tx.commit().await?;
    Ok(Json(server))
}

/// `POST /api/operator/servers/{id}/block` with a required reason. Inside
/// the server lock: set the block, close the open reports, write the audit
/// row. After the commit every member loses their seat and subscriptions
/// (`err server_blocked`), and every socket rechecks.
async fn block(
    State(state): State<AppState>,
    op: Operator,
    Id(id): Id,
    Body(body): Body<ReasonBody>,
) -> Result<Json<ServerAdmin>, ApiError> {
    let reason = reason(body.reason.as_deref(), true)?;
    let mut tx = membership::lock_server_bounded(&state.db, id).await?;
    let (_, blocked) = lock(&mut tx, id).await?.ok_or(ApiError::NotFound)?;
    let removal = if blocked {
        None
    } else {
        sqlx::query(
            "UPDATE servers SET blocked_at = now(), blocked_reason = $2, updated_at = now() \
             WHERE id = $1",
        )
        .bind(id)
        .bind(&reason)
        .execute(&mut *tx)
        .await?;
        let members: Vec<Uuid> =
            sqlx::query_scalar("SELECT user_id FROM server_members WHERE server_id = $1")
                .bind(id)
                .fetch_all(&mut *tx)
                .await?;
        let channel_ids = moderation::channel_ids(&mut tx, id).await?;
        Some((members, channel_ids))
    };
    let reports = action_reports(&mut tx, id, &op).await?;
    if !blocked || reports > 0 {
        audit::record(
            &mut tx,
            &op,
            Action::ServersBlock,
            server_target(id),
            json!({ "reason": reason, "reports": reports }),
        )
        .await?;
    }
    let server = load(&mut tx, id).await?;
    tx.commit().await?;

    if let Some((members, channel_ids)) = removal {
        // The block holds whatever happens here: `membership::load` refuses
        // the server. This ends what members still hold open.
        if let Err(err) =
            moderation::after_removal_with(&state, id, &members, &channel_ids, "server_blocked")
                .await
        {
            warn!(server_id = %id, error = err.code(), "members of a blocked server not removed");
        }
        state.gateway.announce_recheck(Recheck::Server(id)).await;
    }
    Ok(Json(server))
}

/// `POST /api/operator/servers/{id}/unblock`: only clears the block.
/// Members get the server back with their next fetch.
async fn unblock(
    State(state): State<AppState>,
    op: Operator,
    Id(id): Id,
) -> Result<Json<ServerAdmin>, ApiError> {
    let mut tx = state.db.begin().await?;
    let (_, blocked) = lock(&mut tx, id).await?.ok_or(ApiError::NotFound)?;
    if blocked {
        sqlx::query(
            "UPDATE servers SET blocked_at = NULL, blocked_reason = NULL, updated_at = now() \
             WHERE id = $1",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;
        audit::record(
            &mut tx,
            &op,
            Action::ServersUnblock,
            server_target(id),
            json!({}),
        )
        .await?;
    }
    let server = load(&mut tx, id).await?;
    tx.commit().await?;
    Ok(Json(server))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(result: Result<Option<String>, ApiError>) -> Option<&'static str> {
        match result {
            Err(ApiError::Validation(fields)) => fields.get("reason").copied(),
            _ => None,
        }
    }

    #[test]
    fn reasons_are_trimmed_and_bounded() {
        assert_eq!(
            reason(Some("  Spam  "), true).unwrap().as_deref(),
            Some("Spam")
        );
        assert_eq!(reason(Some("   "), false).unwrap(), None);
        assert_eq!(reason(None, false).unwrap(), None);
        assert_eq!(field(reason(None, true)), Some("required"));
        assert_eq!(field(reason(Some(" "), true)), Some("required"));
        assert!(reason(Some(&"x".repeat(REASON_MAX)), true).is_ok());
        assert_eq!(
            field(reason(Some(&"x".repeat(REASON_MAX + 1)), false)),
            Some("too_long")
        );
        assert_eq!(field(reason(Some("a\0b"), true)), Some("invalid"));
    }

    #[test]
    fn filters_are_closed() {
        assert_eq!(Filter::parse(None), Some(Filter::All));
        assert_eq!(Filter::parse(Some("listed")), Some(Filter::Listed));
        assert_eq!(Filter::parse(Some("reported")), Some(Filter::Reported));
        assert_eq!(Filter::parse(Some("blocked")), Some(Filter::Blocked));
        assert_eq!(Filter::parse(Some("disabled")), None);
    }

    #[test]
    fn directory_is_off_until_discovery() {
        assert_eq!(directory(true), "delisted");
        assert_eq!(directory(false), "off");
    }
}
