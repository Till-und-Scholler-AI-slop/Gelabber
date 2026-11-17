//! Pushes about voice channels (#616).
//!
//! **Live.** The pages announce someone going live with a notice
//! (`web/src/voice/liveNotices.ts`); with the app closed nobody heard of it.
//! A Live pushes by the same rule: a voice channel has no notification
//! setting, so its server's decides, and a Live names nobody, so it reaches
//! the members at "all", as an ordinary message does (in a server with more
//! than `SMALL_SERVER` members only those who chose "all" themselves). Not
//! the people in a voice channel of that server, whose app shows it, and
//! nobody who uses the app right now. Once per person and voice channel in
//! [`LIVE_COOLDOWN`], and the push service keeps it no longer than that: a
//! Live heard of an hour late is over.

use std::time::Duration;

use futures_util::StreamExt;
use tracing::{debug, warn};
use uuid::Uuid;

use super::{Payload, Push, Target, default_level, navigate, send, settle};
use crate::{
    auth::user::User, error::ApiError, i18n, servers::permissions::Permission, state::AppState,
};

/// One Live push per person and voice channel in this long, as the pages
/// hold their notice (`LIVE_NOTICE_COOLDOWN_MS`). Also how long a push
/// service keeps it for a phone it cannot reach.
const LIVE_COOLDOWN: Duration = Duration::from_secs(10 * 60);

/// After `who` started a Live in the voice channel: push it in the
/// background.
pub fn live(state: &AppState, who: &User, server: Uuid, channel: Uuid) {
    let Some(push) = state.push.clone() else {
        return;
    };
    if state
        .limiter
        .hit(
            "push-live",
            &format!("{channel}:{}", who.id),
            1,
            LIVE_COOLDOWN,
        )
        .is_err()
    {
        return;
    }
    let Ok(slot) = push.channel_deliveries.clone().try_acquire_owned() else {
        state.metrics.rate_limited("push_dropped");
        return;
    };
    let state = state.clone();
    let (who, name) = (who.id, who.name.clone());
    tokio::spawn(async move {
        if let Err(err) = deliver_live(&state, &push, who, &name, server, channel).await {
            debug!(error = err.code(), "live push not sent");
        }
        drop(slot);
    });
}

async fn deliver_live(
    state: &AppState,
    push: &Push,
    who: Uuid,
    name: &str,
    server: Uuid,
    channel: Uuid,
) -> Result<(), ApiError> {
    let Some((voice, server_name)): Option<(String, String)> = sqlx::query_as(
        "SELECT c.name, s.name FROM channels c
         JOIN servers s ON s.id=c.server_id AND s.blocked_at IS NULL
         WHERE c.id=$1 AND c.server_id=$2 AND c.kind='voice'",
    )
    .bind(channel)
    .bind(server)
    .fetch_optional(&state.db)
    .await?
    else {
        return Ok(());
    };
    // Members who see the voice channel and may join it, at "all" for the
    // server and not muting it; live sessions only, not from someone the
    // recipient blocks (#565), not in Do not disturb (#590).
    let targets: Vec<Target> = sqlx::query_as(concat!(
        "SELECT p.endpoint, p.user_id, p.p256dh, p.auth,
                u.notifications_hide_content AS hide_content, u.locale
         FROM server_members m
         JOIN servers sv ON sv.id=m.server_id
         JOIN push_subscriptions p ON p.user_id=m.user_id
         JOIN sessions s ON s.token_hash=p.session_hash AND s.expires_at>now()
         JOIN users u ON u.id=m.user_id
         LEFT JOIN notification_settings ss ON ss.user_id=m.user_id
           AND ss.scope_kind='server' AND ss.scope_id=m.server_id
         WHERE m.server_id=$1 AND m.user_id<>$2
           AND channel_visible_to($3, m.user_id)
           AND (m.user_id=sv.owner_id
                OR server_member_bits(m.server_id, m.user_id) & $4 <> 0)
           AND COALESCE(ss.level, $5)='all'
           AND NOT (ss.muted IS TRUE AND (ss.muted_until IS NULL OR ss.muted_until>now()))
           AND NOT EXISTS (SELECT 1 FROM user_blocks b
                           WHERE b.blocker_id=m.user_id AND b.blocked_id=$2)
           AND NOT ",
        crate::dnd_active!()
    ))
    .bind(server)
    .bind(who)
    .bind(channel)
    .bind(Permission::JoinVoice as i32)
    .bind(default_level(state, server).await?)
    .fetch_all(&state.db)
    .await?;
    if targets.is_empty() {
        return Ok(());
    }
    // Whoever sits in a voice channel of this server has the app open, and
    // so has whoever has a page in front. Without Redis nobody counts: push
    // anyway.
    let mut users: Vec<Uuid> = targets.iter().map(|t| t.user_id).collect();
    users.sort_unstable();
    users.dedup();
    let mut busy = state.gateway.in_front(&users).await.unwrap_or_default();
    busy.extend(
        state
            .gateway
            .voice_snapshot(server)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|entry| entry.u),
    );
    // Where a tap leads, and the tag: the voice channel, as the pages' own
    // notice (`conversationPath`).
    let path = format!("/s/{server}/c/{channel}");
    let link = navigate(state, &path);
    let (link, path) = (link.as_str(), path.as_str());
    let message = Uuid::new_v4();
    let at = chrono::Utc::now();
    let results = futures_util::stream::iter(
        targets
            .into_iter()
            .filter(|target| !busy.contains(&target.user_id))
            .filter_map(|target| {
                let locale = target.locale;
                let (title, body) = if target.hide_content {
                    (i18n::is_live(locale, name), String::new())
                } else {
                    (
                        format!("{name} · {server_name}"),
                        i18n::live_in(locale, name, &voice),
                    )
                };
                let payload = Payload {
                    title: &title,
                    body: &body,
                    path,
                    user: target.user_id,
                    channel,
                    message,
                    at,
                    recall: false,
                };
                match payload.encode(link, locale) {
                    Ok(raw) => Some((target, raw)),
                    Err(err) => {
                        warn!(error = %err, "push payload not encoded");
                        None
                    }
                }
            })
            .map(|(target, raw)| async move {
                (
                    send(
                        push,
                        &target,
                        &raw,
                        "normal",
                        message,
                        LIVE_COOLDOWN.as_secs(),
                    )
                    .await,
                    target.endpoint,
                )
            }),
    )
    .buffer_unordered(8)
    .collect::<Vec<_>>()
    .await;
    let sent = settle(state, push, results).await.len();
    debug!(channel_id = %channel, sent, "live push delivered");
    Ok(())
}
