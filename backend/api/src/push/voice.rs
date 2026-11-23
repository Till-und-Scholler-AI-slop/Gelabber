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
//!
//! **A call begins.** Someone sits down in a voice channel nobody was in.
//! The pages have no notice for that (the channel list shows it), so this
//! push is for those who asked for it: `voice` on their setting for the
//! server, off by default, whatever their level. The same people are left
//! out, and it wakes a sleeping phone (`Urgency: high`): who asked to be
//! called wants to hear it while the call lasts. Once per voice channel in
//! [`STARTED_COOLDOWN`], so someone who drops out and comes back does not
//! ring twice.

use std::time::Duration;

use futures_util::StreamExt;
use tracing::{debug, warn};
use uuid::Uuid;

use super::{MAX_RECIPIENTS, Payload, Push, Target, default_level, navigate, send, settle};
use crate::{
    auth::user::User, error::ApiError, i18n, servers::permissions::Permission, state::AppState,
};

/// One Live push per person and voice channel in this long, as the pages
/// hold their notice (`LIVE_NOTICE_COOLDOWN_MS`). Also how long a push
/// service keeps it for a phone it cannot reach.
const LIVE_COOLDOWN: Duration = Duration::from_secs(10 * 60);

/// One "a call began" push per voice channel in this long.
const STARTED_COOLDOWN: Duration = Duration::from_secs(10 * 60);

/// What a voice push is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Occasion {
    Live,
    Started,
}

/// After `who` sat down in a voice channel nobody else was in: push it in
/// the background to those who asked.
pub fn started(state: &AppState, who: &User, server: Uuid, channel: Uuid) {
    if state.push.is_none()
        || state
            .limiter
            .hit("push-call", &channel.to_string(), 1, STARTED_COOLDOWN)
            .is_err()
    {
        return;
    }
    announce(state, who, server, channel, Occasion::Started);
}

/// After `who` started a Live in the voice channel: push it in the
/// background.
pub fn live(state: &AppState, who: &User, server: Uuid, channel: Uuid) {
    if state.push.is_none() {
        return;
    }
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
    announce(state, who, server, channel, Occasion::Live);
}

fn announce(state: &AppState, who: &User, server: Uuid, channel: Uuid, occasion: Occasion) {
    let Some(push) = state.push.clone() else {
        return;
    };
    let Ok(slot) = push.channel_deliveries.clone().try_acquire_owned() else {
        state.metrics.rate_limited("push_dropped");
        return;
    };
    let state = state.clone();
    let (who, name) = (who.id, who.name.clone());
    tokio::spawn(async move {
        if let Err(err) = deliver(&state, &push, who, &name, server, channel, occasion).await {
            debug!(error = err.code(), ?occasion, "voice push not sent");
        }
        drop(slot);
    });
}

async fn deliver(
    state: &AppState,
    push: &Push,
    who: Uuid,
    name: &str,
    server: Uuid,
    channel: Uuid,
    occasion: Occasion,
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
    // Members who see the voice channel and may join it and do not mute the
    // server: for a Live those at "all", for a call those who asked. Live
    // sessions only, not from someone the recipient blocks (#565), not in Do
    // not disturb (#590). More than `MAX_RECIPIENTS` of them are a crowd,
    // as for `@everyone`, and nobody is paged.
    let targets: Vec<Target> = sqlx::query_as(concat!(
        "WITH hearing AS (
            SELECT m.user_id FROM server_members m
            JOIN servers sv ON sv.id=m.server_id
            JOIN users u ON u.id=m.user_id
            LEFT JOIN notification_settings ss ON ss.user_id=m.user_id
              AND ss.scope_kind='server' AND ss.scope_id=m.server_id
            WHERE m.server_id=$1 AND m.user_id<>$2
              AND channel_visible_to($3, m.user_id)
              AND (m.user_id=sv.owner_id
                   OR server_member_bits(m.server_id, m.user_id) & $4 <> 0)
              AND CASE WHEN $6 THEN ss.voice IS TRUE ELSE COALESCE(ss.level, $5)='all' END
              AND NOT (ss.muted IS TRUE AND (ss.muted_until IS NULL OR ss.muted_until>now()))
              AND NOT EXISTS (SELECT 1 FROM user_blocks b
                              WHERE b.blocker_id=m.user_id AND b.blocked_id=$2)
              AND NOT ",
        crate::dnd_active!(),
        "   LIMIT $7 + 1)
         SELECT p.endpoint, p.user_id, p.p256dh, p.auth,
                u.notifications_hide_content AS hide_content, u.locale
         FROM hearing h
         JOIN push_subscriptions p ON p.user_id=h.user_id
         JOIN sessions s ON s.token_hash=p.session_hash AND s.expires_at>now()
         JOIN users u ON u.id=h.user_id
         WHERE (SELECT count(*) FROM hearing) <= $7"
    ))
    .bind(server)
    .bind(who)
    .bind(channel)
    .bind(Permission::JoinVoice as i32)
    .bind(default_level(state, server).await?)
    .bind(occasion == Occasion::Started)
    .bind(MAX_RECIPIENTS as i64)
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
    let (urgency, ttl) = match occasion {
        Occasion::Live => ("normal", LIVE_COOLDOWN.as_secs()),
        Occasion::Started => ("high", STARTED_COOLDOWN.as_secs()),
    };
    let results = futures_util::stream::iter(
        targets
            .into_iter()
            .filter(|target| !busy.contains(&target.user_id))
            .filter_map(|target| {
                let locale = target.locale;
                let (title, body) = match (target.hide_content, occasion) {
                    (true, Occasion::Live) => (i18n::is_live(locale, name), String::new()),
                    (true, Occasion::Started) => {
                        (i18n::in_a_voice_channel(locale, name), String::new())
                    }
                    (false, Occasion::Live) => (
                        format!("{name} · {server_name}"),
                        i18n::live_in(locale, name, &voice),
                    ),
                    (false, Occasion::Started) => (
                        format!("{name} · {server_name}"),
                        i18n::in_voice(locale, name, &voice),
                    ),
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
                    send(push, &target, &raw, urgency, message, ttl).await,
                    target.endpoint,
                )
            }),
    )
    .buffer_unordered(8)
    .collect::<Vec<_>>()
    .await;
    let sent = settle(state, push, results).await.len();
    debug!(channel_id = %channel, ?occasion, sent, "voice push delivered");
    Ok(())
}
