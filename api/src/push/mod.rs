//! Web Push (issue #243): direct messages and mentions reach a browser or an
//! installed app that is closed. A browser subscribes with the push service
//! its vendor runs; on a new message the server encrypts a short preview for
//! each subscription of the recipients and posts it there.
//!
//! Nothing is pushed to a user with a page of the app in front on any
//! device (`ClientFrame::Focus`), nor for a conversation they muted or set
//! to "nothing" (#242). In a server channel only a mention pushes.
//!
//! An account that hides the content of its notifications (#268) gets only
//! "Neue Nachricht von Ada": no preview, no conversation.
//!
//! A deleted message is recalled (#277): each subscription that took its
//! push gets another one under the same `Topic`. A push service still
//! holding the first (a phone offline) replaces it; one already shown is
//! replaced on screen by "Nachricht gelöscht" (`web/public/sw.js`).

pub mod webpush;

use std::time::Duration;

use axum::{
    Router,
    extract::State,
    http::StatusCode,
    response::Json,
    routing::{get, post},
};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::{
    auth::session::{CurrentSession, CurrentUser},
    config::PushConfig,
    error::{ApiError, FieldErrors},
    json::Body,
    messages::Message,
    state::AppState,
};
use webpush::{Receiver, Vapid};

/// More subscriptions per account are not kept; the oldest go first.
const MAX_PER_USER: i64 = 20;
const MAX_ENDPOINT: usize = 1024;
/// A push older than this is not worth showing any more. As long, a push
/// service may still hold it, and `push_receipts` keep who took it.
const TTL_SECS: u64 = 3600;
/// `@everyone` in a server with more members than this pushes only to the
/// ones named in the message: a crowd that large is not paged by phone.
const MAX_RECIPIENTS: usize = 500;
/// `@everyone` pushes per server within [`EVERYONE_WINDOW`], whoever wrote
/// them; beyond that only the members named in the message are pushed to.
const EVERYONE_PER_WINDOW: u32 = 5;
const EVERYONE_WINDOW: Duration = Duration::from_secs(10 * 60);
/// Messages being pushed at once across the process, direct messages and
/// server mentions each on their own: a few large `@everyone` fan-outs must
/// not crowd out DMs. A burst beyond that is dropped (and counted as
/// `push_dropped`): pushes are best effort, and the pages show everything on
/// their own.
const MAX_DELIVERIES: usize = 16;

const PREVIEW_CHARS: usize = 180;

/// How often expired `push_receipts` are dropped.
const PRUNE_EVERY: Duration = Duration::from_secs(10 * 60);

/// What a recalled message's notification says instead.
pub const RECALLED: &str = "Nachricht gelöscht";

#[derive(Clone)]
pub struct Push {
    vapid: Vapid,
    hosts: Vec<String>,
    http: reqwest::Client,
    dm_deliveries: std::sync::Arc<tokio::sync::Semaphore>,
    server_deliveries: std::sync::Arc<tokio::sync::Semaphore>,
    /// When this process last dropped expired receipts (Unix seconds).
    pruned_at: std::sync::Arc<std::sync::atomic::AtomicI64>,
}

impl Push {
    pub fn new(config: &PushConfig) -> Result<Self, String> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            // A push service answers itself; a redirect could lead anywhere.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|err| format!("push client: {err}"))?;
        Ok(Self {
            vapid: config.vapid.clone(),
            hosts: config.hosts.clone(),
            http,
            dm_deliveries: std::sync::Arc::new(tokio::sync::Semaphore::new(MAX_DELIVERIES)),
            server_deliveries: std::sync::Arc::new(tokio::sync::Semaphore::new(MAX_DELIVERIES)),
            pruned_at: Default::default(),
        })
    }

    /// Drops receipts past the TTL, at most once per [`PRUNE_EVERY`] and
    /// process: a recall ignores them anyway, so they only take room.
    async fn prune_receipts(&self, state: &AppState) {
        use std::sync::atomic::Ordering;
        let now = chrono::Utc::now().timestamp();
        let last = self.pruned_at.load(Ordering::Relaxed);
        if now - last < PRUNE_EVERY.as_secs() as i64
            || self
                .pruned_at
                .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
                .is_err()
        {
            return;
        }
        let _ = sqlx::query(
            "DELETE FROM push_receipts WHERE sent_at < now() - make_interval(secs => $1)",
        )
        .bind(TTL_SECS as f64)
        .execute(&state.db)
        .await;
    }

    /// The push service's origin for an endpoint this server may post to.
    /// HTTPS on its default port to one of the allowed hosts; plain HTTP,
    /// and any port, only to a loopback address listed explicitly (tests).
    pub fn audience(&self, endpoint: &str) -> Option<String> {
        let url = url::Url::parse(endpoint).ok()?;
        let host = url.host_str()?.to_ascii_lowercase();
        let allowed = self
            .hosts
            .iter()
            .any(|h| host == *h || host.ends_with(&format!(".{h}")));
        let loopback = matches!(
            url.host(),
            Some(url::Host::Ipv4(ip)) if ip.is_loopback()
        );
        let scheme_ok = loopback || (url.scheme() == "https" && url.port().is_none());
        let scheme_ok = scheme_ok && matches!(url.scheme(), "https" | "http");
        (allowed && scheme_ok && url.username().is_empty() && url.password().is_none())
            .then(|| url.origin().ascii_serialization())
    }
}

pub fn router() -> Router<AppState> {
    Router::new().route("/api/push/key", get(key)).route(
        "/api/push/subscriptions",
        post(subscribe).delete(unsubscribe),
    )
}

#[derive(Serialize)]
struct KeyBody {
    /// The server's application server key; null without push.
    key: Option<String>,
}

async fn key(State(state): State<AppState>, CurrentUser(_): CurrentUser) -> Json<KeyBody> {
    Json(KeyBody {
        key: state.push.as_ref().map(|push| push.vapid.public.clone()),
    })
}

#[derive(Deserialize)]
struct Keys {
    p256dh: String,
    auth: String,
}

/// What `PushSubscription.toJSON()` gives.
#[derive(Deserialize)]
struct SubscribeBody {
    endpoint: String,
    keys: Keys,
}

async fn subscribe(
    State(state): State<AppState>,
    session: CurrentSession,
    Body(body): Body<SubscribeBody>,
) -> Result<StatusCode, ApiError> {
    let push = state.push.as_ref().ok_or(ApiError::NotFound)?;
    let mut errors = FieldErrors::new();
    if body.endpoint.len() > MAX_ENDPOINT || push.audience(&body.endpoint).is_none() {
        errors.insert("endpoint", "invalid");
    }
    if Receiver::parse(&body.keys.p256dh, &body.keys.auth).is_err() {
        errors.insert("keys", "invalid");
    }
    if !errors.is_empty() {
        return Err(ApiError::Validation(errors));
    }
    let user = session.user.id;
    // Holds the session row: a logout or revocation in the meantime ends in
    // 401, not in a foreign key violation.
    let mut tx = session
        .lock_live(&state.db)
        .await?
        .ok_or(ApiError::Unauthenticated)?;
    // A browser that signs in as someone else keeps its endpoint and its
    // keys: it moves. Someone who only knows the endpoint cannot take it
    // over; their request changes nothing.
    sqlx::query(
        "INSERT INTO push_subscriptions (endpoint,user_id,session_hash,p256dh,auth)
         VALUES ($1,$2,$3,$4,$5) ON CONFLICT (endpoint) DO UPDATE
         SET user_id=EXCLUDED.user_id, session_hash=EXCLUDED.session_hash,
             p256dh=EXCLUDED.p256dh, auth=EXCLUDED.auth, created_at=now()
         WHERE push_subscriptions.user_id=EXCLUDED.user_id
            OR (push_subscriptions.p256dh=EXCLUDED.p256dh
                AND push_subscriptions.auth=EXCLUDED.auth)",
    )
    .bind(&body.endpoint)
    .bind(user)
    .bind(&session.hash)
    .bind(&body.keys.p256dh)
    .bind(&body.keys.auth)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "DELETE FROM push_subscriptions WHERE user_id=$1 AND endpoint NOT IN (
           SELECT endpoint FROM push_subscriptions WHERE user_id=$1
           ORDER BY created_at DESC LIMIT $2)",
    )
    .bind(user)
    .bind(MAX_PER_USER)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct UnsubscribeBody {
    endpoint: String,
}

async fn unsubscribe(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Body(body): Body<UnsubscribeBody>,
) -> Result<StatusCode, ApiError> {
    sqlx::query("DELETE FROM push_subscriptions WHERE endpoint=$1 AND user_id=$2")
        .bind(&body.endpoint)
        .bind(user.id)
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// What the service worker shows (`web/public/sw.js`): the same fields the
/// page puts on its own notifications, so taps and dedupe work alike.
#[derive(Serialize)]
struct Payload<'a> {
    title: &'a str,
    body: &'a str,
    path: &'a str,
    user: Uuid,
    channel: Uuid,
    message: Uuid,
    at: chrono::DateTime<chrono::Utc>,
    /// The message was deleted: its notification goes, whatever it said.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    recall: bool,
}

#[derive(sqlx::FromRow)]
struct Target {
    endpoint: String,
    user_id: Uuid,
    p256dh: String,
    auth: String,
    hide_content: bool,
}

/// After a message is stored: push it in the background, unless
/// [`MAX_DELIVERIES`] of its kind are under way already.
pub fn announce(state: &AppState, message: &Message, server: Option<Uuid>) {
    let Some(push) = state.push.clone() else {
        return;
    };
    let deliveries = if server.is_none() {
        &push.dm_deliveries
    } else {
        &push.server_deliveries
    };
    let Ok(slot) = deliveries.clone().try_acquire_owned() else {
        state.metrics.rate_limited("push_dropped");
        debug!(message_id = %message.id, "push dropped, deliveries busy");
        return;
    };
    let state = state.clone();
    let message = message.clone();
    tokio::spawn(async move {
        if let Err(err) = deliver(&state, &push, &message, server).await {
            debug!(error = err.code(), "push not sent");
        }
        drop(slot);
    });
}

async fn deliver(
    state: &AppState,
    push: &Push,
    message: &Message,
    server: Option<Uuid>,
) -> Result<(), ApiError> {
    let author = message.author.id;
    let recipients: Vec<Uuid> = match server {
        None => {
            sqlx::query_scalar(
                "SELECT user_id FROM channel_members WHERE channel_id=$1 AND user_id<>$2",
            )
            .bind(message.channel_id)
            .bind(author)
            .fetch_all(&state.db)
            .await?
        }
        Some(server) if message.mention_everyone && everyone_may_push(state, server) => {
            let members: Vec<Uuid> = sqlx::query_scalar(
                "SELECT user_id FROM server_members WHERE server_id=$1 AND user_id<>$2
                 LIMIT $3",
            )
            .bind(server)
            .bind(author)
            .bind(MAX_RECIPIENTS as i64 + 1)
            .fetch_all(&state.db)
            .await?;
            if members.len() > MAX_RECIPIENTS {
                named(state, message, server).await?
            } else {
                members
            }
        }
        Some(server) => named(state, message, server).await?,
    };
    if recipients.is_empty() {
        return Ok(());
    }
    // Live sessions only, and not where the conversation is muted or set
    // to "nothing"; the channel's own level wins over its server's.
    let targets: Vec<Target> = sqlx::query_as(
        "SELECT p.endpoint, p.user_id, p.p256dh, p.auth,
                u.notifications_hide_content AS hide_content
         FROM push_subscriptions p
         JOIN sessions s ON s.token_hash=p.session_hash AND s.expires_at>now()
         JOIN users u ON u.id=p.user_id
         LEFT JOIN notification_settings cs ON cs.user_id=p.user_id
           AND cs.scope_kind='channel' AND cs.scope_id=$2
         LEFT JOIN notification_settings ss ON ss.user_id=p.user_id
           AND ss.scope_kind='server' AND ss.scope_id=$3
         WHERE p.user_id=ANY($1)
           AND COALESCE(cs.level, ss.level, 'all')<>'none'
           AND NOT (cs.muted IS TRUE AND (cs.muted_until IS NULL OR cs.muted_until>now()))
           AND NOT (ss.muted IS TRUE AND (ss.muted_until IS NULL OR ss.muted_until>now()))",
    )
    .bind(&recipients)
    .bind(message.channel_id)
    .bind(server)
    .fetch_all(&state.db)
    .await?;
    if targets.is_empty() {
        return Ok(());
    }
    let (title, path) = heading(state, message, server).await?;
    let body = if targets.iter().all(|t| t.hide_content) {
        String::new()
    } else {
        preview(state, message).await?
    };
    let hidden = hidden_title(&message.author.name);
    let users: Vec<Uuid> = {
        let mut users: Vec<Uuid> = targets.iter().map(|t| t.user_id).collect();
        users.sort_unstable();
        users.dedup();
        users
    };
    // Without Redis nobody counts as in front: push anyway.
    let in_front = state.gateway.in_front(&users).await.unwrap_or_default();
    let urgency = if server.is_none() { "high" } else { "normal" };
    let results = futures_util::stream::iter(
        targets
            .into_iter()
            .filter(|target| !in_front.contains(&target.user_id))
            .filter_map(|target| {
                let (title, body) = if target.hide_content {
                    (hidden.as_str(), "")
                } else {
                    (title.as_str(), body.as_str())
                };
                let payload = Payload {
                    title,
                    body,
                    path: &path,
                    user: target.user_id,
                    channel: message.channel_id,
                    message: message.id,
                    at: message.created_at,
                    recall: false,
                };
                match serde_json::to_vec(&payload) {
                    Ok(raw) => Some((target, raw)),
                    Err(err) => {
                        warn!(error = %err, "push payload not encoded");
                        None
                    }
                }
            })
            .map(|(target, raw)| async move {
                (
                    send(push, &target, &raw, urgency, message.id).await,
                    target.endpoint,
                )
            }),
    )
    .buffer_unordered(8)
    .collect::<Vec<_>>()
    .await;
    let sent = settle(state, push, results).await;
    if !sent.is_empty() {
        let _ = sqlx::query(
            "INSERT INTO push_receipts (message_id, endpoint)
             SELECT $1, endpoint FROM push_subscriptions WHERE endpoint=ANY($2)
             ON CONFLICT DO NOTHING",
        )
        .bind(message.id)
        .bind(&sent)
        .execute(&state.db)
        .await;
        push.prune_receipts(state).await;
        // Deleted while its pushes were on their way: the delete found no
        // receipts to recall, so recall here. The pushes are out either way,
        // so a failure here is only logged.
        let exists: Result<bool, sqlx::Error> =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM messages WHERE id=$1)")
                .bind(message.id)
                .fetch_one(&state.db)
                .await;
        let recalled = match exists {
            Ok(true) => Ok(()),
            Ok(false) => match take_receipts(state, message.id).await {
                Ok(targets) => send_recall(state, push, message, server, targets).await,
                Err(err) => Err(err),
            },
            Err(err) => Err(err.into()),
        };
        if let Err(err) = recalled {
            debug!(
                error = err.code(),
                "push not recalled after a racing delete"
            );
        }
    }
    let sent = sent.len();
    debug!(message_id = %message.id, sent, "push delivered");
    Ok(())
}

/// Records what the push services answered; returns the endpoints that took
/// the push.
async fn settle(state: &AppState, push: &Push, results: Vec<(Outcome, String)>) -> Vec<String> {
    let mut sent = Vec::new();
    let mut gone = Vec::new();
    // A subscription that has not taken a push for 30 days goes on its next
    // failure (a 413, a 429, a service that is gone).
    let mut failed = Vec::new();
    let mut refused = Vec::new();
    for (outcome, endpoint) in results {
        match outcome {
            Outcome::Sent => sent.push(endpoint),
            Outcome::Gone => gone.push(endpoint),
            Outcome::Refused => refused.push(endpoint),
            Outcome::Failed => failed.push(endpoint),
        }
    }
    // A refusal is the subscription's fault only where the same service took
    // another push of this message. Refused across the board, it is ours (a
    // VAPID token the service rejects, a clock off, a bad header), and
    // deleting would silence every phone until its app is opened again.
    let accepting: Vec<String> = sent.iter().filter_map(|e| push.audience(e)).collect();
    for endpoint in refused {
        if push
            .audience(&endpoint)
            .is_some_and(|origin| accepting.contains(&origin))
        {
            gone.push(endpoint);
        } else {
            failed.push(endpoint);
        }
    }
    if !sent.is_empty() {
        let _ =
            sqlx::query("UPDATE push_subscriptions SET last_ok_at=now() WHERE endpoint=ANY($1)")
                .bind(&sent)
                .execute(&state.db)
                .await;
    }
    if !gone.is_empty() {
        info!(
            count = gone.len(),
            "push subscriptions expired or refused, removed"
        );
        let _ = sqlx::query("DELETE FROM push_subscriptions WHERE endpoint=ANY($1)")
            .bind(&gone)
            .execute(&state.db)
            .await;
    }
    if !failed.is_empty() {
        let _ = sqlx::query(
            "DELETE FROM push_subscriptions WHERE endpoint=ANY($1)
             AND COALESCE(last_ok_at, created_at) < now() - interval '30 days'",
        )
        .bind(&failed)
        .execute(&state.db)
        .await;
    }
    sent
}

/// After a message is deleted: recall its pushes in the background.
pub fn recall(state: &AppState, message: &Message, server: Option<Uuid>) {
    let Some(push) = state.push.clone() else {
        return;
    };
    let state = state.clone();
    let message = message.clone();
    tokio::spawn(async move {
        let targets = match take_receipts(&state, message.id).await {
            Ok(targets) if targets.is_empty() => return,
            Ok(targets) => targets,
            Err(err) => {
                debug!(error = err.code(), "push not recalled");
                return;
            }
        };
        // Unlike a new message's push, a recall waits for its turn: dropped,
        // the preview it should take down stays.
        let deliveries = if server.is_none() {
            &push.dm_deliveries
        } else {
            &push.server_deliveries
        };
        let Ok(slot) = deliveries.clone().acquire_owned().await else {
            return;
        };
        if let Err(err) = send_recall(&state, &push, &message, server, targets).await {
            debug!(error = err.code(), "push not recalled");
        }
        drop(slot);
    });
}

/// The subscriptions that took the message's push within the TTL. Taking
/// the receipts deletes them, so a recall from the delete and one from a
/// delivery that raced it do not both send.
async fn take_receipts(state: &AppState, message: Uuid) -> Result<Vec<Target>, ApiError> {
    Ok(sqlx::query_as(
        "WITH taken AS (
            DELETE FROM push_receipts WHERE message_id=$1 RETURNING endpoint, sent_at
         )
         SELECT p.endpoint, p.user_id, p.p256dh, p.auth,
                u.notifications_hide_content AS hide_content
         FROM taken t
         JOIN push_subscriptions p ON p.endpoint=t.endpoint
         JOIN users u ON u.id=p.user_id
         WHERE t.sent_at >= now() - make_interval(secs => $2)",
    )
    .bind(message)
    .bind(TTL_SECS as f64)
    .fetch_all(&state.db)
    .await?)
}

/// Sends each of `targets` a replacement under the same topic.
async fn send_recall(
    state: &AppState,
    push: &Push,
    message: &Message,
    server: Option<Uuid>,
    targets: Vec<Target>,
) -> Result<(), ApiError> {
    if targets.is_empty() {
        return Ok(());
    }
    let (title, path) = heading(state, message, server).await?;
    let hidden = hidden_title(&message.author.name);
    let results = futures_util::stream::iter(targets.into_iter().filter_map(|target| {
        let payload = Payload {
            title: if target.hide_content {
                hidden.as_str()
            } else {
                title.as_str()
            },
            body: RECALLED,
            path: &path,
            user: target.user_id,
            channel: message.channel_id,
            message: message.id,
            at: message.created_at,
            recall: true,
        };
        match serde_json::to_vec(&payload) {
            Ok(raw) => Some((target, raw)),
            Err(err) => {
                warn!(error = %err, "push payload not encoded");
                None
            }
        }
    }))
    .map(|(target, raw)| async move {
        (
            send(push, &target, &raw, "normal", message.id).await,
            target.endpoint,
        )
    })
    .buffer_unordered(8)
    .collect::<Vec<_>>()
    .await;
    let sent = settle(state, push, results).await.len();
    debug!(message_id = %message.id, sent, "push recalled");
    Ok(())
}

/// The users the message names who are still in the server: the names
/// were resolved before the message was stored, and someone removed since
/// gets no preview.
async fn named(state: &AppState, message: &Message, server: Uuid) -> Result<Vec<Uuid>, ApiError> {
    if message.mentions.is_empty() {
        return Ok(Vec::new());
    }
    Ok(sqlx::query_scalar(
        "SELECT user_id FROM server_members WHERE server_id=$1 AND user_id=ANY($2)
         AND user_id<>$3",
    )
    .bind(server)
    .bind(&message.mentions)
    .bind(message.author.id)
    .fetch_all(&state.db)
    .await?)
}

/// Whether this server's `@everyone` may still page its members by push.
fn everyone_may_push(state: &AppState, server: Uuid) -> bool {
    let allowed = state
        .limiter
        .hit(
            "push-everyone",
            &server.to_string(),
            EVERYONE_PER_WINDOW,
            EVERYONE_WINDOW,
        )
        .is_ok();
    if !allowed {
        state.metrics.rate_limited("push_everyone");
    }
    allowed
}

enum Outcome {
    Sent,
    /// 404/410: the browser dropped the subscription.
    Gone,
    /// 400/403: the service will not take it, e.g. one subscribed under an
    /// old VAPID key. With up to 20 per account and 500 recipients a
    /// message, such rows are not kept for a month of retries; a browser
    /// still subscribed sends it again on its next start (`syncPush`).
    Refused,
    Failed,
}

/// `topic` names what the push is about: a later push with the same topic
/// replaces it at the push service while it is not delivered yet.
async fn send(push: &Push, target: &Target, payload: &[u8], urgency: &str, topic: Uuid) -> Outcome {
    let Some(audience) = push.audience(&target.endpoint) else {
        return Outcome::Gone;
    };
    let Ok(receiver) = Receiver::parse(&target.p256dh, &target.auth) else {
        return Outcome::Gone;
    };
    let Ok(body) = webpush::encrypt(&receiver, payload) else {
        return Outcome::Failed;
    };
    let expires = chrono::Utc::now().timestamp() + 12 * 3600;
    let response = push
        .http
        .post(&target.endpoint)
        .header(
            "Authorization",
            push.vapid.authorization(&audience, expires),
        )
        .header("TTL", TTL_SECS.to_string())
        .header("Urgency", urgency)
        // 32 hex digits: within the 32 URL-safe base64 characters RFC 8030
        // allows a topic.
        .header("Topic", topic.simple().to_string())
        .header("Content-Encoding", "aes128gcm")
        .header("Content-Type", "application/octet-stream")
        .body(body)
        .send()
        .await;
    match response {
        Ok(response) if response.status().is_success() => Outcome::Sent,
        Ok(response) if matches!(response.status().as_u16(), 404 | 410) => Outcome::Gone,
        Ok(response) if matches!(response.status().as_u16(), 400 | 403) => {
            debug!(
                status = response.status().as_u16(),
                "push service refused the subscription"
            );
            Outcome::Refused
        }
        Ok(response) => {
            warn!(status = response.status().as_u16(), "push service refused");
            Outcome::Failed
        }
        Err(err) => {
            warn!(error = %err, "push service unreachable");
            Outcome::Failed
        }
    }
}

/// "Ada" for a DM, "Ada · #allgemein" for a server channel; and where a tap
/// leads (`conversationPath` in web/src/messages/notify.ts).
async fn heading(
    state: &AppState,
    message: &Message,
    server: Option<Uuid>,
) -> Result<(String, String), ApiError> {
    let Some(server) = server else {
        return Ok((
            message.author.name.clone(),
            format!("/d/{}", message.channel_id),
        ));
    };
    let channel: Option<String> = sqlx::query_scalar("SELECT name FROM channels WHERE id=$1")
        .bind(message.channel_id)
        .fetch_optional(&state.db)
        .await?;
    Ok((
        format!(
            "{} · #{}",
            message.author.name,
            channel.unwrap_or_else(|| "Kanal".into())
        ),
        format!("/s/{server}/c/{}", message.channel_id),
    ))
}

/// All a notification says for an account that hides the content: who wrote.
/// The pages show the same (`notificationText` in
/// web/src/messages/notificationPrivacy.ts).
pub fn hidden_title(author: &str) -> String {
    format!("Neue Nachricht von {author}")
}

/// The text with mentions as names, cut short; a file or GIF named as such.
async fn preview(state: &AppState, message: &Message) -> Result<String, ApiError> {
    let mut text = message.content.clone();
    if !message.mentions.is_empty() {
        let names: Vec<(Uuid, String)> =
            sqlx::query_as("SELECT id, name FROM users WHERE id=ANY($1)")
                .bind(&message.mentions)
                .fetch_all(&state.db)
                .await?;
        for (id, name) in names {
            text = text.replace(&format!("<@{id}>"), &format!("@{name}"));
        }
    }
    let text = readable(&text);
    Ok(if !text.is_empty() {
        cut(&text)
    } else if message.gif.is_some() {
        "GIF".into()
    } else if !message.attachments.is_empty() {
        "Datei".into()
    } else {
        String::new()
    })
}

/// Tokens of people no longer around as "@Unbekannt", whitespace folded.
fn readable(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<@") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        match after.find('>') {
            Some(end) if Uuid::parse_str(&after[..end]).is_ok() => {
                out.push_str("@Unbekannt");
                rest = &after[end + 1..];
            }
            _ => {
                out.push_str("<@");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn cut(text: &str) -> String {
    if text.chars().count() <= PREVIEW_CHARS {
        return text.to_owned();
    }
    let mut short: String = text.chars().take(PREVIEW_CHARS - 1).collect();
    short.push('…');
    short
}

#[cfg(test)]
mod tests {
    use super::*;

    fn push(hosts: &[&str]) -> Push {
        let (private, _) = Vapid::generate();
        Push::new(&PushConfig {
            vapid: Vapid::parse(&private, "mailto:a@b.c").unwrap(),
            hosts: hosts.iter().map(|h| (*h).to_owned()).collect(),
        })
        .unwrap()
    }

    #[test]
    fn busy_server_pushes_leave_direct_messages_their_slots() {
        let push = push(&["fcm.googleapis.com"]);
        let held: Vec<_> = (0..MAX_DELIVERIES)
            .map(|_| push.server_deliveries.clone().try_acquire_owned().unwrap())
            .collect();
        assert!(push.server_deliveries.clone().try_acquire_owned().is_err());
        assert!(push.dm_deliveries.clone().try_acquire_owned().is_ok());
        drop(held);
    }

    #[test]
    fn only_the_known_push_services_are_posted_to() {
        let push = push(crate::config::DEFAULT_PUSH_HOSTS);
        assert_eq!(
            push.audience("https://fcm.googleapis.com/fcm/send/abc")
                .as_deref(),
            Some("https://fcm.googleapis.com")
        );
        assert!(
            push.audience("https://updates.push.services.mozilla.com/wpush/v2/x")
                .is_some()
        );
        assert!(
            push.audience("https://wns2-am3p.notify.windows.com/w/?token=x")
                .is_some()
        );
        assert!(push.audience("https://web.push.apple.com/QGx").is_some());
        for bad in [
            "http://fcm.googleapis.com/fcm/send/abc",
            "https://evilfcm.googleapis.com.example.org/x",
            "https://notgoogleapis.com/x",
            "https://user:pw@fcm.googleapis.com/x",
            "https://127.0.0.1/x",
            "https://10.0.0.1/x",
            "https://fcm.googleapis.com:8443/fcm/send/abc",
            "https://fcm.googleapis.com:443x/x",
            "file:///etc/passwd",
            "not a url",
        ] {
            assert!(push.audience(bad).is_none(), "{bad}");
        }
        // A loopback service in tests, only when listed.
        let local = super::tests::push(&["127.0.0.1"]);
        assert!(local.audience("http://127.0.0.1:4000/x").is_some());
        assert!(push.audience("http://127.0.0.1:4000/x").is_none());
    }

    #[test]
    fn previews_are_short_and_readable() {
        assert_eq!(
            readable("hi  <@0190f7a2-0000-7000-8000-00000000000a>\n!"),
            "hi @Unbekannt !"
        );
        assert_eq!(readable("a <@x> b"), "a <@x> b");
        let long = "x".repeat(400);
        assert_eq!(cut(&long).chars().count(), PREVIEW_CHARS);
        assert!(cut(&long).ends_with('…'));
    }
}
