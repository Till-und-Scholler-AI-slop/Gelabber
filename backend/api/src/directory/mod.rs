//! The server directory "Entdecken" (v0.10, #520). Off unless the operator
//! sets `API_DIRECTORY`; while it is off, `GET /api/directory` says
//! `enabled: false` and the listing routes answer `404`. Signed-in only: there is no
//! anonymous route.
//!
//! An owner lists a server ([`settings`]). A listing is visible to a caller
//! while it is `listed`, the operator neither delisted
//! (`servers.delisted_at`) nor blocked (`servers.blocked_at`) the server, and
//! the caller is not banned from it (`visible_listing_where!`).
//!
//! Signed-in people search the visible listings, join an open server at
//! once, or ask to join one in request mode. A request stays `pending`
//! until the queue answers it or the caller withdraws it; a declined one
//! blocks a new request for [`DECLINE_COOLDOWN_DAYS`]. The owner and the
//! members holding `moderate_members` answer requests ([`requests`]).

pub mod languages;
pub mod requests;
pub mod settings;
pub mod validate;

use std::collections::HashMap;

use axum::Router;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::info;
use uuid::Uuid;

use crate::auth::session::CurrentUser;
use crate::auth::validate::finish;
use crate::error::{ApiError, FieldErrors};
use crate::json::Body;
use crate::limits::{DIRECTORY_JOIN_PER_HOUR, DIRECTORY_SEARCH_PER_MIN, HOUR, MINUTE};
use crate::path::Id;
use crate::servers::ServerView;
use crate::servers::membership;
use crate::state::AppState;

use self::languages::LANGUAGES;
use self::validate::{JoinMode, OFFSET_MAX, PAGE};

/// Days a declined person waits before asking the same server again.
pub const DECLINE_COOLDOWN_DAYS: i32 = 7;
/// Most pending requests to join one person has across all servers.
pub const PENDING_PER_USER: i64 = 10;
/// Most pending requests to join one server has.
pub const PENDING_PER_SERVER: i64 = 100;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/directory", get(status))
        .merge(settings::router())
        .route("/api/directory/listings", get(list_listings))
        .route("/api/directory/listings/{id}/join", post(join))
        .route(
            "/api/directory/listings/{id}/request",
            post(request).delete(withdraw),
        )
        .merge(requests::router())
}

/// The visibility rule over `server_listings l` joined with `servers s`,
/// for the caller `$1`; a macro so the queries stay `&'static str`.
macro_rules! visible_listing_where {
    () => {
        "l.listed AND s.delisted_at IS NULL AND s.blocked_at IS NULL \
         AND NOT EXISTS (SELECT 1 FROM server_bans b \
                          WHERE b.server_id = s.id AND b.user_id = $1)"
    };
}

/// A server's place in the directory as the operator console shows it
/// (`ServerAdmin.directory`, a report's server): `delisted` while an
/// operator took it out, `listed` while the owner has it listed, otherwise
/// `off`. A blocked server keeps its state; the console shows the block on
/// its own.
pub fn state_of(delisted: bool, listed: bool) -> &'static str {
    if delisted {
        "delisted"
    } else if listed {
        "listed"
    } else {
        "off"
    }
}

/// `404` while the directory is off.
pub(crate) fn require_enabled(state: &AppState) -> Result<(), ApiError> {
    if state.directory {
        Ok(())
    } else {
        Err(ApiError::NotFound)
    }
}

#[derive(Debug, Serialize)]
pub struct Language {
    pub code: &'static str,
    pub name: &'static str,
    /// Listings of this language the caller can see.
    pub listings: i64,
}

#[derive(Debug, Serialize)]
pub struct Status {
    pub enabled: bool,
    /// Every language a listing can declare; empty while off.
    pub languages: Vec<Language>,
}

/// `GET /api/directory`: whether the directory is on, and the languages
/// with the number of listings the caller can see in each.
async fn status(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<Status>, ApiError> {
    if !state.directory {
        return Ok(Json(Status {
            enabled: false,
            languages: Vec::new(),
        }));
    }
    let counts: HashMap<String, i64> = sqlx::query_as::<_, (String, i64)>(concat!(
        "SELECT l.language, COUNT(*) FROM server_listings l \
         JOIN servers s ON s.id = l.server_id WHERE ",
        visible_listing_where!(),
        " GROUP BY l.language"
    ))
    .bind(user.id)
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .collect();
    let languages = LANGUAGES
        .iter()
        .map(|(code, name)| Language {
            code,
            name,
            listings: counts.get(*code).copied().unwrap_or(0),
        })
        .collect();
    Ok(Json(Status {
        enabled: true,
        languages,
    }))
}

/// The caller's request to join a listed server, as the directory shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RequestState {
    Pending,
    /// Only while the cooldown lasts.
    Declined,
}

/// One listing of the search. Exactly what a non-member may learn about a
/// listed server, plus the caller's own state.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Listing {
    pub server_id: Uuid,
    pub name: String,
    pub icon_url: Option<String>,
    pub description: String,
    pub language: String,
    pub member_count: i64,
    pub join_mode: String,
    /// The caller is a member already.
    pub member: bool,
    /// The caller's request: `pending`, `declined` while its cooldown
    /// lasts, or `null`.
    pub request: Option<String>,
    /// While `request` is `declined`: when the caller may ask again.
    pub retry_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
pub struct ListingPage {
    pub listings: Vec<Listing>,
    /// `null` on the last page.
    pub next_offset: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct ListParams {
    // Strings, so that a bad value is a field error, not axum's plain 400.
    q: Option<String>,
    language: Option<String>,
    offset: Option<String>,
}

/// `GET /api/directory/listings?q=&language=&offset=`: the listings the
/// caller can see, biggest first, [`PAGE`] at a time. `q` matches the name
/// or the description, case-insensitively and with `%` and `_` literal.
async fn list_listings(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Query(params): Query<ListParams>,
) -> Result<Json<ListingPage>, ApiError> {
    require_enabled(&state)?;
    let mut errors = FieldErrors::new();
    let query = validate::query(params.q.as_deref(), &mut errors);
    let language = validate::language_filter(params.language.as_deref(), &mut errors);
    let offset = validate::offset(params.offset.as_deref(), &mut errors);
    finish(errors)?;
    // Each search counts members per matching listing; the gate skips GETs.
    const BUCKET: &str = "dir_search";
    state
        .limiter
        .hit(
            BUCKET,
            &user.id.to_string(),
            DIRECTORY_SEARCH_PER_MIN,
            MINUTE,
        )
        .map_err(|retry_after| {
            state.metrics.rate_limited(BUCKET);
            ApiError::RateLimited { retry_after }
        })?;
    let pattern = query.map(|q| format!("%{}%", validate::like_pattern(&q)));

    // One row past the page tells whether another page follows.
    let mut listings: Vec<Listing> = sqlx::query_as(concat!(
        "SELECT s.id AS server_id, s.name, s.icon_url, l.description, l.language, \
                (SELECT count(*) FROM server_members m WHERE m.server_id = s.id) AS member_count, \
                l.join_mode, \
                EXISTS (SELECT 1 FROM server_members m \
                         WHERE m.server_id = s.id AND m.user_id = $1) AS member, \
                (SELECT r.status FROM server_join_requests r \
                  WHERE r.server_id = s.id AND r.user_id = $1 \
                    AND (r.status = 'pending' \
                         OR r.decided_at > now() - make_interval(days => $6))) AS request, \
                (SELECT r.decided_at + make_interval(days => $6) FROM server_join_requests r \
                  WHERE r.server_id = s.id AND r.user_id = $1 AND r.status = 'declined' \
                    AND r.decided_at > now() - make_interval(days => $6)) AS retry_at \
         FROM server_listings l JOIN servers s ON s.id = l.server_id \
         WHERE ",
        visible_listing_where!(),
        " AND ($2::text IS NULL \
               OR s.name ILIKE $2 ESCAPE '\\' OR l.description ILIKE $2 ESCAPE '\\') \
          AND ($3::text IS NULL OR l.language = $3) \
         ORDER BY member_count DESC, l.listed_at ASC, s.id \
         LIMIT $4 OFFSET $5"
    ))
    .bind(user.id)
    .bind(pattern)
    .bind(language)
    .bind(PAGE + 1)
    .bind(offset)
    .bind(DECLINE_COOLDOWN_DAYS)
    .fetch_all(&state.db)
    .await?;
    let more = listings.len() as i64 > PAGE;
    listings.truncate(PAGE as usize);
    let next_offset = (more && offset + PAGE <= OFFSET_MAX).then_some(offset + PAGE);
    Ok(Json(ListingPage {
        listings,
        next_offset,
    }))
}

/// A listing the caller can see, as [`visible_listing`] loads it.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct VisibleListing {
    pub name: String,
    pub icon_url: Option<String>,
    pub description: String,
    pub language: String,
    pub join_mode: String,
}

impl VisibleListing {
    pub fn join_mode(&self) -> JoinMode {
        // The column's check allows only these two.
        JoinMode::parse(&self.join_mode).unwrap_or(JoinMode::Request)
    }
}

/// Server `server_id`'s listing if `user_id` can see it: listed, neither
/// delisted nor blocked, and the caller not banned.
pub async fn visible_listing<'e, E: sqlx::Executor<'e, Database = sqlx::Postgres>>(
    db: E,
    server_id: Uuid,
    user_id: Uuid,
) -> Result<Option<VisibleListing>, ApiError> {
    Ok(sqlx::query_as(concat!(
        "SELECT s.name, s.icon_url, l.description, l.language, l.join_mode \
         FROM server_listings l JOIN servers s ON s.id = l.server_id \
         WHERE l.server_id = $2 AND ",
        visible_listing_where!()
    ))
    .bind(user_id)
    .bind(server_id)
    .fetch_optional(db)
    .await?)
}

/// What a report of server `server_id` keeps of its listing, when
/// `user_id` can see one: name, icon, description, language and join mode
/// as they are now, so later edits cannot hide what was
/// reported. `None` while the directory is off or the listing is not
/// visible to the caller.
pub async fn report_snapshot(
    state: &AppState,
    server_id: Uuid,
    user_id: Uuid,
) -> Result<Option<serde_json::Value>, ApiError> {
    if !state.directory {
        return Ok(None);
    }
    let listing = visible_listing(&state.db, server_id, user_id).await?;
    Ok(listing.map(|listing| {
        serde_json::json!({
            "name": listing.name,
            "icon_url": listing.icon_url,
            "description": listing.description,
            "language": listing.language,
            "join_mode": listing.join_mode,
        })
    }))
}

/// The `dir_join` bucket: joins and requests to join per account and hour.
fn hit_join_limit(state: &AppState, user_id: Uuid) -> Result<(), ApiError> {
    const BUCKET: &str = "dir_join";
    state
        .limiter
        .hit(BUCKET, &user_id.to_string(), DIRECTORY_JOIN_PER_HOUR, HOUR)
        .map_err(|retry_after| {
            state.metrics.rate_limited(BUCKET);
            ApiError::RateLimited { retry_after }
        })
}

async fn is_member(
    conn: &mut sqlx::PgConnection,
    server_id: Uuid,
    user_id: Uuid,
) -> Result<bool, ApiError> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM server_members WHERE server_id = $1 AND user_id = $2)",
    )
    .bind(server_id)
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await?)
}

/// `POST /api/directory/listings/{id}/join`: joins an open listed server.
/// Idempotent for a member, whose repeat neither counts against nor is
/// refused by the `dir_join` limit; a server in request mode answers
/// `409 join_request_required`.
async fn join(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(server_id): Id,
) -> Result<Json<ServerView>, ApiError> {
    require_enabled(&state)?;
    // A member's repeat is idempotent and spends nothing of the limit.
    let mut conn = state.db.acquire().await?;
    if !is_member(&mut conn, server_id, user.id).await? {
        hit_join_limit(&state, user.id)?;
    }
    drop(conn);
    let mut tx = membership::lock_server(&state.db, server_id, true).await?;
    let listing = visible_listing(&mut *tx, server_id, user.id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let already = is_member(&mut tx, server_id, user.id).await?;
    if !already {
        if listing.join_mode() != JoinMode::Open {
            return Err(ApiError::JoinRequestRequired);
        }
        sqlx::query("INSERT INTO server_members (server_id, user_id) VALUES ($1, $2)")
            .bind(server_id)
            .bind(user.id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM server_join_requests WHERE server_id = $1 AND user_id = $2")
            .bind(server_id)
            .bind(user.id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    if !already {
        info!(server_id = %server_id, user_id = %user.id, "member joined via directory");
    }
    let member = membership::load(&state.db, server_id, user.id).await?;
    Ok(Json(member.into()))
}

#[derive(Debug, Default, Deserialize)]
pub struct RequestBody {
    #[serde(default)]
    message: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RequestAnswer {
    pub request: RequestState,
}

/// `POST /api/directory/listings/{id}/request`: asks to join a listed
/// server. A pending request answers `200` unchanged, a new one `201`.
async fn request(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(server_id): Id,
    Body(body): Body<RequestBody>,
) -> Result<Response, ApiError> {
    require_enabled(&state)?;
    let mut errors = FieldErrors::new();
    let message = validate::message(body.message.as_deref(), &mut errors);
    finish(errors)?;
    let message = message.expect("validated");
    hit_join_limit(&state, user.id)?;

    let mut tx = membership::lock_server(&state.db, server_id, true).await?;
    // Serialises one person's requests, so concurrent ones to different
    // servers cannot pass the per-person cap together.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('dir_join:' || $1::text, 0))")
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    visible_listing(&mut *tx, server_id, user.id)
        .await?
        .ok_or(ApiError::NotFound)?;
    if is_member(&mut tx, server_id, user.id).await? {
        return Err(ApiError::AlreadyMember);
    }

    // The existing row: whether it is pending, and for a declined one the
    // seconds left of its cooldown (0 once it is over).
    let existing: Option<(bool, i64)> = sqlx::query_as(
        "SELECT status = 'pending', \
                GREATEST(0, CEIL(EXTRACT(EPOCH FROM \
                    COALESCE(decided_at, now()) + make_interval(days => $3) - now())))::bigint \
         FROM server_join_requests WHERE server_id = $1 AND user_id = $2",
    )
    .bind(server_id)
    .bind(user.id)
    .bind(DECLINE_COOLDOWN_DAYS)
    .fetch_optional(&mut *tx)
    .await?;
    match existing {
        Some((false, left)) if left > 0 => {
            return Err(ApiError::JoinRequestDeclined {
                retry_after: left as u64,
            });
        }
        Some((true, _)) => {
            tx.commit().await?;
            return Ok(Json(RequestAnswer {
                request: RequestState::Pending,
            })
            .into_response());
        }
        _ => {}
    }

    let (mine, theirs): (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM server_join_requests \
                  WHERE user_id = $2 AND status = 'pending'), \
                (SELECT count(*) FROM server_join_requests \
                  WHERE server_id = $1 AND status = 'pending')",
    )
    .bind(server_id)
    .bind(user.id)
    .fetch_one(&mut *tx)
    .await?;
    if mine >= PENDING_PER_USER || theirs >= PENDING_PER_SERVER {
        return Err(ApiError::JoinRequestsFull);
    }

    // Replaces a declined row whose cooldown is over.
    sqlx::query(
        "INSERT INTO server_join_requests (server_id, user_id, message) VALUES ($1, $2, $3) \
         ON CONFLICT (server_id, user_id) DO UPDATE SET \
             message = EXCLUDED.message, status = 'pending', created_at = now(), \
             decided_at = NULL, decided_by = NULL",
    )
    .bind(server_id)
    .bind(user.id)
    .bind(&message)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    info!(server_id = %server_id, user_id = %user.id, "join requested");
    requests::publish_queue(&state, server_id).await;
    Ok((
        StatusCode::CREATED,
        Json(RequestAnswer {
            request: RequestState::Pending,
        }),
    )
        .into_response())
}

/// `DELETE /api/directory/listings/{id}/request`: withdraws the caller's
/// pending request. No visibility check, so it still works after the
/// listing went away.
async fn withdraw(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Id(server_id): Id,
) -> Result<StatusCode, ApiError> {
    require_enabled(&state)?;
    let deleted = sqlx::query(
        "DELETE FROM server_join_requests \
         WHERE server_id = $1 AND user_id = $2 AND status = 'pending'",
    )
    .bind(server_id)
    .bind(user.id)
    .execute(&state.db)
    .await?
    .rows_affected();
    if deleted == 0 {
        return Err(ApiError::NotFound);
    }
    info!(server_id = %server_id, user_id = %user.id, "join request withdrawn");
    requests::publish_queue(&state, server_id).await;
    Ok(StatusCode::NO_CONTENT)
}
