//! The last call (#616): a push subscription lives and dies with the session
//! that made it, and a session nobody used for `API_SESSION_IDLE_HOURS`, or
//! older than `API_SESSION_TTL_HOURS`, ends without a word. A phone whose app
//! stayed closed that long would simply go silent. So shortly before, each
//! subscription of such a session gets one push that says what keeps the
//! notifications coming: opening the app, or, at the end of the session's
//! life, signing in again.
//!
//! The notice carries no content and no conversation. Do not disturb holds it
//! back until it ends; a page in front does not, since a session someone uses
//! is only ever due at the end of its life, and that they should read.

use std::time::Duration;

use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use tracing::{debug, warn};
use uuid::Uuid;

use super::{Outcome, Payload, Target, navigate, send, settle};
use crate::{error::ApiError, i18n, state::AppState};

/// How long before a session's end its subscriptions are told, at most.
const AHEAD: Duration = Duration::from_secs(48 * 3600);

/// How often sessions about to end are looked for.
const SWEEP_EVERY: Duration = Duration::from_secs(15 * 60);

/// Subscriptions told per sweep step; the next step takes the rest.
const SWEEP_BATCH: i64 = 200;

/// How long before its end a session is due: [`AHEAD`], but with a short idle
/// window no more than a quarter of it, so that a session in use is not told
/// between two renewals.
pub fn ahead(idle: Duration) -> Duration {
    AHEAD.min(idle / 4)
}

#[derive(sqlx::FromRow)]
struct Due {
    #[sqlx(flatten)]
    target: Target,
    expires_at: DateTime<Utc>,
    /// The session is at the end of its life: a use no longer renews it.
    last: bool,
}

/// Tells the subscriptions of sessions about to end, a batch at a time;
/// returns how many are settled: told, or gone for good. One the push
/// service did not take is due again and does not count. With several API
/// processes each subscription is told once: marking it is what claims it.
pub async fn sweep(state: &AppState) -> Result<usize, ApiError> {
    let Some(push) = state.push.clone() else {
        return Ok(0);
    };
    // A use of the session after the notice (`last_used_at` moves with every
    // renewal) arms it again for the next time the session runs low. At the
    // end of its life nothing renews it, so that notice comes once.
    let due: Vec<Due> = sqlx::query_as(concat!(
        "WITH due AS (
            SELECT p.endpoint, s.expires_at,
                   s.created_at + make_interval(secs => $2)
                     <= now() + make_interval(secs => $3) AS last
            FROM push_subscriptions p
            JOIN sessions s ON s.token_hash=p.session_hash
            JOIN users u ON u.id=p.user_id
            WHERE s.expires_at > now()
              AND s.expires_at <= now() + make_interval(secs => $1)
              AND (p.warned_at IS NULL OR p.warned_at < s.last_used_at)
              AND NOT ",
        crate::dnd_active!(),
        "   ORDER BY s.expires_at LIMIT $4 FOR NO KEY UPDATE OF p SKIP LOCKED)
         UPDATE push_subscriptions p SET warned_at=now()
         FROM due, users u
         WHERE p.endpoint=due.endpoint AND u.id=p.user_id
         RETURNING p.endpoint, p.user_id, p.p256dh, p.auth,
                   u.notifications_hide_content AS hide_content, u.locale,
                   due.expires_at, due.last"
    ))
    .bind(ahead(state.session_idle).as_secs_f64())
    .bind(state.session_ttl.as_secs_f64())
    .bind(state.session_idle.as_secs_f64())
    .bind(SWEEP_BATCH)
    .fetch_all(&state.db)
    .await?;
    if due.is_empty() {
        return Ok(0);
    }
    let count = due.len();
    let now = Utc::now();
    let link = navigate(state, "/");
    let link = link.as_str();
    let push = &push;
    let results = futures_util::stream::iter(due.into_iter().filter_map(|due| {
        let Due {
            target,
            expires_at,
            last,
        } = due;
        let message = Uuid::new_v4();
        let payload = Payload {
            title: i18n::last_call_title(target.locale),
            body: i18n::last_call_body(target.locale, last),
            path: "/",
            user: target.user_id,
            // No conversation: a notification of its own, which no message
            // replaces.
            channel: Uuid::nil(),
            message,
            at: now,
            recall: false,
        };
        // Worth delivering for as long as the session lasts, not after.
        let ttl = (expires_at - now).num_seconds().max(60) as u64;
        match payload.encode(link, target.locale) {
            Ok(raw) => Some((target, raw, message, ttl)),
            Err(err) => {
                warn!(error = %err, "push payload not encoded");
                None
            }
        }
    }))
    .map(|(target, raw, message, ttl)| async move {
        (
            send(push, &target, &raw, "normal", message, ttl).await,
            target.endpoint,
        )
    })
    .buffer_unordered(8)
    .collect::<Vec<_>>()
    .await;
    // Whatever the push service did not take is due again at the next sweep.
    let missed: Vec<String> = results
        .iter()
        .filter(|(outcome, _)| !matches!(outcome, Outcome::Sent))
        .map(|(_, endpoint)| endpoint.clone())
        .collect();
    let sent = settle(state, push, results).await.len();
    // Those `settle` removed are not due again; on an error count them all.
    let again = if missed.is_empty() {
        0
    } else {
        sqlx::query("UPDATE push_subscriptions SET warned_at=NULL WHERE endpoint=ANY($1)")
            .bind(&missed)
            .execute(&state.db)
            .await
            .map_or(missed.len(), |done| done.rows_affected() as usize)
    };
    debug!(due = count, sent, again, "ending sessions told by push");
    Ok(count.saturating_sub(again))
}

pub fn start(state: AppState) {
    let Some(push) = state.push.as_ref() else {
        return;
    };
    if push
        .last_call_started
        .swap(true, std::sync::atomic::Ordering::AcqRel)
    {
        return;
    }
    tokio::spawn(async move {
        // The first sweep waits a round: a starting process should not add to
        // the boot rush on the pool, and nothing here is urgent.
        let first = tokio::time::Instant::now() + SWEEP_EVERY;
        let mut tick = tokio::time::interval_at(first, SWEEP_EVERY);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            if state.db.is_closed() {
                break;
            }
            round(&state).await;
        }
    });
}

/// One round of the worker: sweep steps while each settles a whole batch. A
/// step that leaves some due again ends the round, and the next round tries
/// them: with the push service down, the next step would only claim the same
/// batch again, as fast as the service fails.
pub async fn round(state: &AppState) {
    loop {
        match sweep(state).await {
            Ok(n) if n as i64 >= SWEEP_BATCH => continue,
            Ok(_) => break,
            Err(error) => {
                warn!(error = error.code(), "last call sweep failed");
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_idle_window_shortens_the_notice() {
        assert_eq!(ahead(Duration::from_secs(14 * 24 * 3600)), AHEAD);
        assert_eq!(
            ahead(Duration::from_secs(3600)),
            Duration::from_secs(15 * 60)
        );
    }
}
