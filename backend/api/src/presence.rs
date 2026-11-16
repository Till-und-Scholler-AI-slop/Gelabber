//! Manual presence (#590): Automatic, Away, Do not disturb or Invisible,
//! for an hour, eight, a day or until changed, for the whole account.
//!
//! `users.presence_mode` / `presence_mode_until` are the record. The gateway
//! keeps a copy in Redis that its presence scripts read
//! (`Gateway::set_presence_mode`): others see `i`, `d` or nothing on top of
//! what the sockets report, and the account's own sockets hear the mode
//! itself (`pm`) to show it instead.
//!
//! - Do not disturb: no Web Push of any kind (`dnd_active!`); the
//!   clients play no sounds and show no toasts. Unread badges stay.
//! - Invisible: shown offline, and typing reaches nobody. Voice rosters
//!   still list the account (out of scope).
//!
//! A mode that runs out is automatic again at once for every reader (the
//! record is read with its end, the Redis copy expires with it); the sweep
//! here then clears the record and announces it.

use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::response::Json;
use axum::routing::get;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth::session::CurrentUser;
use crate::auth::validate;
use crate::error::{ApiError, FieldErrors};
use crate::gateway::protocol::{PresenceMode, UserFrame};
use crate::json::Body;
use crate::state::AppState;

/// SQL: user `u` is in Do not disturb right now. Every push query filters
/// on it, as `concat!("…", dnd_active!())` (a literal, for sqlx).
#[macro_export]
macro_rules! dnd_active {
    () => {
        "(u.presence_mode = 'dnd' \
         AND (u.presence_mode_until IS NULL OR u.presence_mode_until > now()))"
    };
}

/// How often modes that ran out are cleared and announced.
const SWEEP_EVERY: Duration = Duration::from_secs(15);

/// Modes cleared per sweep step; the next step takes the rest.
const SWEEP_BATCH: i64 = 200;

pub fn router() -> Router<AppState> {
    Router::new().route("/api/me/presence", get(get_mode).put(put_mode))
}

/// The account's manual presence. `until` is when it ends; none is "until
/// I change it" (and always none for `auto`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct ModeState {
    pub mode: PresenceMode,
    pub until: Option<DateTime<Utc>>,
}

impl ModeState {
    fn from_row((mode, until): (String, Option<DateTime<Utc>>)) -> Self {
        let mode = PresenceMode::parse(&mode).unwrap_or_default();
        match until {
            // Ran out and not swept yet: automatic already.
            Some(until) if until <= Utc::now() => Self::default(),
            _ if mode == PresenceMode::Auto => Self::default(),
            until => Self { mode, until },
        }
    }

    fn frame(self) -> UserFrame {
        UserFrame::PresenceMode {
            m: self.mode,
            u: self.until,
        }
    }
}

/// How long a manual mode lasts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeDuration {
    Hours(i64),
    Forever,
}

impl ModeDuration {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "1h" => Some(Self::Hours(1)),
            "8h" => Some(Self::Hours(8)),
            "24h" => Some(Self::Hours(24)),
            "forever" => Some(Self::Forever),
            _ => None,
        }
    }

    fn until(self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        match self {
            Self::Hours(hours) => Some(now + chrono::Duration::hours(hours)),
            Self::Forever => None,
        }
    }
}

/// The account's mode now; automatic for an unknown account.
pub async fn load(db: &sqlx::PgPool, user_id: Uuid) -> Result<ModeState, ApiError> {
    let row: Option<(String, Option<DateTime<Utc>>)> =
        sqlx::query_as("SELECT presence_mode, presence_mode_until FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(db)
            .await?;
    Ok(row.map(ModeState::from_row).unwrap_or_default())
}

async fn get_mode(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<ModeState>, ApiError> {
    Ok(Json(load(&state.db, user.id).await?))
}

/// `PUT /api/me/presence` with `{"mode":"dnd","duration":"8h"}`. `duration`
/// is one of `1h`, `8h`, `24h`, `forever` (the default); `auto` takes none.
#[derive(Debug, Default, Deserialize)]
pub struct ModeBody {
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub duration: Option<String>,
}

async fn put_mode(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Body(body): Body<ModeBody>,
) -> Result<Json<ModeState>, ApiError> {
    let mut errors = FieldErrors::new();
    let mode = match body.mode.as_deref() {
        None => {
            errors.insert("mode", "required");
            None
        }
        Some(raw) => PresenceMode::parse(raw).or_else(|| {
            errors.insert("mode", "invalid");
            None
        }),
    };
    let duration = match body.duration.as_deref() {
        None => Some(ModeDuration::Forever),
        Some(raw) => ModeDuration::parse(raw).or_else(|| {
            errors.insert("duration", "invalid");
            None
        }),
    };
    validate::finish(errors)?;
    let (mode, duration) = (mode.expect("validated"), duration.expect("validated"));
    let until = match mode {
        PresenceMode::Auto => None,
        _ => duration.until(Utc::now()),
    };
    // The row stays locked until Redis has the copy, so two changes (or a
    // change and the sweep) reach Redis in the order they were saved.
    let mut tx = state.db.begin().await?;
    let row: (String, Option<DateTime<Utc>>) = sqlx::query_as(
        "UPDATE users SET presence_mode = $2, presence_mode_until = $3 \
         WHERE id = $1 AND deleted_at IS NULL \
         RETURNING presence_mode, presence_mode_until",
    )
    .bind(user.id)
    .bind(mode.as_str())
    .bind(until)
    .fetch_optional(&mut *tx)
    .await?
    // The account went between the extractor and the update.
    .ok_or(ApiError::Unauthenticated)?;
    let saved = ModeState::from_row(row);
    announce(&state, user.id, saved).await;
    tx.commit().await?;
    Ok(Json(saved))
}

/// Tell the gateway and the account's sockets. Best effort, like presence
/// itself: the record is saved, and without Redis nobody is shown anything.
async fn announce(state: &AppState, user_id: Uuid, mode: ModeState) {
    if let Err(error) = state
        .gateway
        .set_presence_mode(user_id, mode.mode, mode.until)
        .await
    {
        tracing::debug!(error = error.code(), "presence mode not applied");
    }
    if let Err(error) = state.gateway.publish_user(user_id, &mode.frame()).await {
        tracing::debug!(error = error.code(), "presence mode not announced");
    }
}

/// A socket connects or subscribes: if Redis lost the manual mode (a
/// restart, after which the sockets stay open and only subscribe again),
/// put the record's back before the socket announces itself. A marker that
/// goes with the copy makes this one Redis call once it was checked.
/// `db` is the caller's connection when it holds one (a subscribe), else
/// one is taken from the pool.
pub async fn restore(state: &AppState, db: Option<&mut sqlx::PgConnection>, user_id: Uuid) {
    match state.gateway.claim_mode_check(user_id).await {
        Ok(true) => {}
        // Checked already, or no Redis: nothing to restore into.
        Ok(false) | Err(_) => return,
    }
    let result = match db {
        Some(db) => restore_locked(state, db, user_id).await,
        None => match state.db.acquire().await {
            Ok(mut db) => restore_locked(state, &mut db, user_id).await,
            Err(error) => Err(error.into()),
        },
    };
    if let Err(error) = result {
        tracing::debug!(error = error.code(), "presence mode not restored");
        // The next socket tries again.
        let _ = state.gateway.drop_mode_check(user_id).await;
    }
}

/// The record, locked while Redis takes it, wins over the copy.
async fn restore_locked(
    state: &AppState,
    db: &mut sqlx::PgConnection,
    user_id: Uuid,
) -> Result<(), ApiError> {
    use sqlx::Acquire;
    let mut tx = db.begin().await?;
    let row: Option<(String, Option<DateTime<Utc>>)> = sqlx::query_as(
        "SELECT presence_mode, presence_mode_until FROM users WHERE id = $1 FOR UPDATE",
    )
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?;
    let record = row.map(ModeState::from_row).unwrap_or_default();
    if state.gateway.presence_mode(user_id).await? != record.mode {
        state
            .gateway
            .set_presence_mode(user_id, record.mode, record.until)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Clears modes that ran out, a batch at a time, and announces each; with
/// several API processes each mode is cleared, and announced, once.
pub async fn sweep(state: &AppState) -> Result<usize, ApiError> {
    // Locked until announced, like a change (`put_mode`).
    let mut tx = state.db.begin().await?;
    let expired: Vec<(Uuid, String)> = sqlx::query_as(
        "WITH due AS ( \
             SELECT id, presence_mode FROM users \
             WHERE presence_mode_until IS NOT NULL AND presence_mode_until <= now() \
             ORDER BY presence_mode_until LIMIT $1 FOR UPDATE SKIP LOCKED) \
         UPDATE users u SET presence_mode = 'auto', presence_mode_until = NULL \
         FROM due WHERE u.id = due.id RETURNING u.id, due.presence_mode",
    )
    .bind(SWEEP_BATCH)
    .fetch_all(&mut *tx)
    .await?;
    for (user_id, old) in &expired {
        // The Redis copy ran out with the mode, so the gateway is told what
        // it hid: the members still see that.
        let old = PresenceMode::parse(old).unwrap_or_default();
        if let Err(error) = state.gateway.presence_mode_ended(*user_id, old).await {
            tracing::debug!(error = error.code(), "presence mode end not applied");
        }
        let frame = ModeState::default().frame();
        if let Err(error) = state.gateway.publish_user(*user_id, &frame).await {
            tracing::debug!(error = error.code(), "presence mode end not announced");
        }
    }
    tx.commit().await?;
    Ok(expired.len())
}

pub fn start(state: AppState) {
    if state
        .presence_worker_started
        .swap(true, std::sync::atomic::Ordering::AcqRel)
    {
        return;
    }
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(SWEEP_EVERY);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            if state.db.is_closed() {
                break;
            }
            loop {
                match sweep(&state).await {
                    Ok(n) if n as i64 >= SWEEP_BATCH => continue,
                    Ok(_) => break,
                    Err(error) => {
                        tracing::warn!(error = error.code(), "presence mode sweep failed");
                        break;
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        let now = Utc::now();
        assert_eq!(
            ModeDuration::parse("1h").unwrap().until(now),
            Some(now + chrono::Duration::hours(1))
        );
        assert_eq!(
            ModeDuration::parse("24h").unwrap().until(now),
            Some(now + chrono::Duration::hours(24))
        );
        assert_eq!(ModeDuration::parse("forever").unwrap().until(now), None);
        assert_eq!(ModeDuration::parse("2h"), None);
    }

    #[test]
    fn a_mode_that_ran_out_reads_automatic() {
        let past = Utc::now() - chrono::Duration::seconds(1);
        let future = Utc::now() + chrono::Duration::hours(1);
        assert_eq!(
            ModeState::from_row(("dnd".into(), Some(past))),
            ModeState::default()
        );
        assert_eq!(
            ModeState::from_row(("dnd".into(), Some(future))),
            ModeState {
                mode: PresenceMode::Dnd,
                until: Some(future)
            }
        );
        assert_eq!(
            ModeState::from_row(("invisible".into(), None)).mode,
            PresenceMode::Invisible
        );
        assert_eq!(
            ModeState::from_row(("auto".into(), None)),
            ModeState::default()
        );
    }
}
