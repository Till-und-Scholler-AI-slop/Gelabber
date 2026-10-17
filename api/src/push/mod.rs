//! Web Push (issue #243): direct messages and mentions reach a browser or an
//! installed app that is closed. A browser subscribes with the push service
//! its vendor runs; on a new message the server encrypts a short preview for
//! each subscription of the recipients and posts it there.
//!
//! Nothing is pushed to a user with a page of the app in front on any
//! device (`ClientFrame::Focus`), nor for a conversation they muted or set
//! to "nothing" (#242). In a server channel only a mention pushes.

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
/// A push older than this is not worth showing any more.
const TTL_SECS: u64 = 3600;
/// `@everyone` in a large server pushes to this many members at most.
const MAX_RECIPIENTS: i64 = 500;
const PREVIEW_CHARS: usize = 180;

#[derive(Clone)]
pub struct Push {
    vapid: Vapid,
    hosts: Vec<String>,
    http: reqwest::Client,
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
        })
    }

    /// The push service's origin for an endpoint this server may post to.
    /// HTTPS to one of the allowed hosts; plain HTTP only to a loopback
    /// address listed explicitly (tests).
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
        let scheme_ok = url.scheme() == "https" || (url.scheme() == "http" && loopback);
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
    let mut tx = state.db.begin().await?;
    // A browser that signs in as someone else keeps its endpoint: it moves.
    sqlx::query(
        "INSERT INTO push_subscriptions (endpoint,user_id,session_hash,p256dh,auth)
         VALUES ($1,$2,$3,$4,$5) ON CONFLICT (endpoint) DO UPDATE
         SET user_id=EXCLUDED.user_id, session_hash=EXCLUDED.session_hash,
             p256dh=EXCLUDED.p256dh, auth=EXCLUDED.auth, created_at=now()",
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
}

#[derive(sqlx::FromRow)]
struct Target {
    endpoint: String,
    user_id: Uuid,
    p256dh: String,
    auth: String,
}

/// After a message is stored: push it in the background.
pub fn announce(state: &AppState, message: &Message, server: Option<Uuid>) {
    let Some(push) = state.push.clone() else {
        return;
    };
    let state = state.clone();
    let message = message.clone();
    tokio::spawn(async move {
        if let Err(err) = deliver(&state, &push, &message, server).await {
            debug!(error = err.code(), "push not sent");
        }
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
        Some(server) if message.mention_everyone => {
            sqlx::query_scalar(
                "SELECT user_id FROM server_members WHERE server_id=$1 AND user_id<>$2
                 ORDER BY joined_at LIMIT $3",
            )
            .bind(server)
            .bind(author)
            .bind(MAX_RECIPIENTS)
            .fetch_all(&state.db)
            .await?
        }
        Some(_) => message.mentions.clone(),
    };
    if recipients.is_empty() {
        return Ok(());
    }
    // Live sessions only, and not where the conversation is muted or set
    // to "nothing"; the channel's own level wins over its server's.
    let targets: Vec<Target> = sqlx::query_as(
        "SELECT p.endpoint, p.user_id, p.p256dh, p.auth
         FROM push_subscriptions p
         JOIN sessions s ON s.token_hash=p.session_hash AND s.expires_at>now()
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
    let body = preview(state, message).await?;
    let mut sent = 0usize;
    let mut away = std::collections::HashMap::<Uuid, bool>::new();
    let mut wanted = Vec::new();
    for target in targets {
        let skip = match away.get(&target.user_id) {
            Some(skip) => *skip,
            None => {
                // Without Redis nobody counts as in front: push anyway.
                let skip = state
                    .gateway
                    .has_focus(target.user_id)
                    .await
                    .unwrap_or(false);
                away.insert(target.user_id, skip);
                skip
            }
        };
        if !skip {
            wanted.push(target);
        }
    }
    let urgency = if server.is_none() { "high" } else { "normal" };
    let results = futures_util::stream::iter(wanted.into_iter().map(|target| {
        let payload = Payload {
            title: &title,
            body: &body,
            path: &path,
            user: target.user_id,
            channel: message.channel_id,
            message: message.id,
            at: message.created_at,
        };
        let raw = serde_json::to_vec(&payload).unwrap_or_default();
        async move { (send(push, &target, &raw, urgency).await, target.endpoint) }
    }))
    .buffer_unordered(8)
    .collect::<Vec<_>>()
    .await;
    for (outcome, endpoint) in results {
        match outcome {
            Outcome::Sent => {
                sent += 1;
                let _ =
                    sqlx::query("UPDATE push_subscriptions SET last_ok_at=now() WHERE endpoint=$1")
                        .bind(&endpoint)
                        .execute(&state.db)
                        .await;
            }
            Outcome::Gone => {
                info!("push subscription expired, removed");
                let _ = sqlx::query("DELETE FROM push_subscriptions WHERE endpoint=$1")
                    .bind(&endpoint)
                    .execute(&state.db)
                    .await;
            }
            Outcome::Failed => {}
        }
    }
    debug!(message_id = %message.id, sent, "push delivered");
    Ok(())
}

enum Outcome {
    Sent,
    /// 404/410: the browser dropped the subscription.
    Gone,
    Failed,
}

async fn send(push: &Push, target: &Target, payload: &[u8], urgency: &str) -> Outcome {
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
        .header("Content-Encoding", "aes128gcm")
        .header("Content-Type", "application/octet-stream")
        .body(body)
        .send()
        .await;
    match response {
        Ok(response) if response.status().is_success() => Outcome::Sent,
        Ok(response) if matches!(response.status().as_u16(), 404 | 410) => Outcome::Gone,
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
