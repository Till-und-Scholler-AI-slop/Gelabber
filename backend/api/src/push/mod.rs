//! Web Push (issue #243): direct messages and mentions reach a browser or an
//! installed app that is closed. A browser subscribes with the push service
//! its vendor runs; on a new message the server encrypts a short preview for
//! each subscription of the recipients and posts it there.
//!
//! Nothing is pushed for a conversation someone muted or set to "nothing"
//! (#242), nor to a page of the app that is in front and follows the
//! conversation (`ClientFrame::Focus`).
//!
//! A server channel pushes by the level each member has for it (#616), as
//! the pages announce it (`wantsNotification` in
//! web/src/messages/notifyLevel.ts). A mention reaches everyone it names
//! but those at "nothing", at once. A message that names nobody reaches the
//! members at "all": not while they use the app on any device, at most once
//! per channel in [`ORDINARY_WINDOW`], and in a server with more than
//! [`SMALL_SERVER`] members only those who chose "all" themselves.
//!
//! Someone going live in a voice channel pushes like a message that names
//! nobody (`voice`): a voice channel has no setting, so its server's level
//! decides.
//!
//! An account in Do not disturb (#590) gets no push at all while it lasts.
//!
//! An account that hides the content of its notifications (#268) gets only
//! "Neue Nachricht von Ada": no preview, no conversation.
//!
//! A deleted message is recalled (#277): each subscription that took its
//! push gets another one under the same `Topic`. A push service still
//! holding the first (a phone offline) replaces it; one already shown is
//! replaced on screen by "Nachricht gelöscht" (`web/public/sw.js`).
//!
//! The words of a push come from `crate::i18n`, in each recipient's locale
//! (#314).

pub mod last_call;
pub mod voice;
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
    i18n::{self, Locale},
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
/// More who chose "all" ([`listening`]), or who would hear of a voice
/// channel ([`voice`]), than this are paged by none.
const MAX_RECIPIENTS: usize = 500;
/// `@everyone` pushes per server within [`EVERYONE_WINDOW`], whoever wrote
/// them; beyond that only the members named in the message are pushed to.
const EVERYONE_PER_WINDOW: u32 = 5;
const EVERYONE_WINDOW: Duration = Duration::from_secs(10 * 60);
/// Messages being pushed at once across the process; direct messages,
/// server mentions and ordinary server messages each on their own: a few
/// large `@everyone` fan-outs must not crowd out DMs, nor a lively channel
/// the mentions. A burst beyond that is dropped (and counted as
/// `push_dropped`): pushes are best effort, and the pages show everything on
/// their own.
const MAX_DELIVERIES: usize = 16;
/// A server with at most this many members counts as a group that wants to
/// hear every message: "all" is what a member without a setting has there.
/// In a larger one an ordinary message (one that names nobody) pushes only
/// to the members who set the channel or the server to "all" themselves;
/// the default would page a crowd by phone for every line.
const SMALL_SERVER: i64 = 50;
/// The ordinary messages of one channel push at most once in this time: a
/// lively conversation is one buzz on a phone, not one per line. Mentions
/// are not held back by it.
const ORDINARY_WINDOW: Duration = Duration::from_secs(30);

const PREVIEW_CHARS: usize = 180;

/// How often expired `push_receipts` are dropped.
const PRUNE_EVERY: Duration = Duration::from_secs(10 * 60);

#[derive(Clone)]
pub struct Push {
    vapid: Vapid,
    hosts: Vec<String>,
    http: reqwest::Client,
    dm_deliveries: std::sync::Arc<tokio::sync::Semaphore>,
    server_deliveries: std::sync::Arc<tokio::sync::Semaphore>,
    /// Server messages that name nobody.
    channel_deliveries: std::sync::Arc<tokio::sync::Semaphore>,
    /// When this process last dropped expired receipts (Unix seconds).
    pruned_at: std::sync::Arc<std::sync::atomic::AtomicI64>,
    /// The [`last_call`] worker runs once per process.
    last_call_started: std::sync::Arc<std::sync::atomic::AtomicBool>,
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
            channel_deliveries: std::sync::Arc::new(tokio::sync::Semaphore::new(MAX_DELIVERIES)),
            pruned_at: Default::default(),
            last_call_started: Default::default(),
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
             p256dh=EXCLUDED.p256dh, auth=EXCLUDED.auth, created_at=now(),
             warned_at=CASE WHEN push_subscriptions.session_hash=EXCLUDED.session_hash
                            THEN push_subscriptions.warned_at END
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

/// What a push is about: the fields the page puts on its own notifications,
/// so taps and dedupe work alike (`web/public/sw.js`).
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

impl Payload<'_> {
    /// The push message, as a declarative one (Push API, `"web_push": 8030`;
    /// #616). A browser that knows them (Safari from iOS 18.4) builds the
    /// notification itself and, the message being `mutable`, hands it to the
    /// worker first; whatever the worker does not manage (it was removed, it
    /// failed, it showed nothing) the browser shows as declared, and no
    /// subscription is lost over a push that showed nothing. Every other
    /// browser hands the worker the JSON as before.
    ///
    /// `navigate` is where a tap leads when no worker answers it. The fields
    /// of the payload stand at the top level too: a worker installed before
    /// v0.12 reads them there, and stays in charge until its app is opened
    /// again.
    fn encode(&self, navigate: &str, locale: Locale) -> serde_json::Result<Vec<u8>> {
        #[derive(Serialize)]
        struct Message<'a> {
            web_push: u16,
            notification: Declared<'a>,
            mutable: bool,
            #[serde(flatten)]
            payload: &'a Payload<'a>,
        }
        #[derive(Serialize)]
        struct Declared<'a> {
            title: &'a str,
            body: &'a str,
            navigate: &'a str,
            /// The tag of the worker and the pages: one notification per
            /// conversation and account.
            tag: String,
            lang: &'static str,
            renotify: bool,
            silent: bool,
            data: Data<'a>,
        }
        #[derive(Serialize)]
        struct Data<'a> {
            path: &'a str,
            user: Uuid,
            channel: Uuid,
            message: Uuid,
            at: chrono::DateTime<chrono::Utc>,
            #[serde(skip_serializing_if = "std::ops::Not::not")]
            recall: bool,
        }
        serde_json::to_vec(&Message {
            web_push: 8030,
            notification: Declared {
                title: self.title,
                body: self.body,
                navigate,
                tag: format!("gelabber:{}:{}", self.user, self.channel),
                lang: locale.as_str(),
                // A recall replaces what is on screen without a sound.
                renotify: !self.recall,
                silent: self.recall,
                data: Data {
                    path: self.path,
                    user: self.user,
                    channel: self.channel,
                    message: self.message,
                    at: self.at,
                    recall: self.recall,
                },
            },
            mutable: true,
            payload: self,
        })
    }
}

/// Where a tap on a declared notification leads: the page of `path` under
/// `APP_PUBLIC_URL`. Without that setting the path alone, which a browser
/// resolves against the app's own address, or else treats the message as an
/// ordinary one for the worker.
fn navigate(state: &AppState, path: &str) -> String {
    match &state.public_url {
        Some(base) => format!("{}{path}", base.as_str().trim_end_matches('/')),
        None => path.to_owned(),
    }
}

#[derive(sqlx::FromRow)]
struct Target {
    endpoint: String,
    user_id: Uuid,
    p256dh: String,
    auth: String,
    hide_content: bool,
    locale: Locale,
    /// The message is a direct message to them or names them; false for
    /// someone who only listens to every message of the channel.
    #[sqlx(default)]
    addressed: bool,
}

/// After a message is stored: push it in the background, unless
/// [`MAX_DELIVERIES`] of its kind are under way already.
pub fn announce(state: &AppState, message: &Message, server: Option<Uuid>) {
    let Some(push) = state.push.clone() else {
        return;
    };
    let names_someone = message.mention_everyone || !message.mentioned().is_empty();
    let Some((slot, ordinary)) = slot(state, &push, message.channel_id, server, names_someone)
    else {
        return;
    };
    let state = state.clone();
    let message = message.clone();
    tokio::spawn(async move {
        if let Err(err) = deliver(&state, &push, &message, server, ordinary).await {
            debug!(error = err.code(), "push not sent");
        }
        drop(slot);
    });
}

/// A delivery slot for a new message, and whether the message may also go
/// to the members who listen to every message of its channel; a mention
/// goes out either way. `None`: nothing to push, or no slot free.
fn slot(
    state: &AppState,
    push: &Push,
    channel: Uuid,
    server: Option<Uuid>,
    names_someone: bool,
) -> Option<(tokio::sync::OwnedSemaphorePermit, bool)> {
    let ordinary = server.is_some() && ordinary_may_push(state, channel);
    let deliveries = match server {
        None => &push.dm_deliveries,
        Some(_) if names_someone => &push.server_deliveries,
        Some(_) if ordinary => &push.channel_deliveries,
        Some(_) => return None,
    };
    let Ok(slot) = deliveries.clone().try_acquire_owned() else {
        // Nothing went out, so the channel's window is not used up: the
        // next message may push.
        if ordinary {
            state.limiter.refund("push-channel", &channel.to_string());
        }
        state.metrics.rate_limited("push_dropped");
        debug!(channel_id = %channel, "push dropped, deliveries busy");
        return None;
    };
    Some((slot, ordinary))
}

async fn deliver(
    state: &AppState,
    push: &Push,
    message: &Message,
    server: Option<Uuid>,
    ordinary: bool,
) -> Result<(), ApiError> {
    let author = message.author.id;
    // Whom the message is for by name: the other side of a direct message,
    // the people a server message mentions.
    let addressed: Vec<Uuid> = match server {
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
                "SELECT m.user_id FROM server_members m
                 JOIN servers s ON s.id=m.server_id AND s.blocked_at IS NULL
                 WHERE m.server_id=$1 AND m.user_id<>$2
                 AND channel_visible_to($4, m.user_id) LIMIT $3",
            )
            .bind(server)
            .bind(author)
            .bind(MAX_RECIPIENTS as i64 + 1)
            .bind(message.channel_id)
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
    // Who else may hear it, and the level of a member without a setting.
    let (listening, default_level) = match server {
        Some(server) if ordinary => listening(state, message, server).await?,
        _ => (Vec::new(), "all"),
    };
    let recipients: Vec<Uuid> = {
        let mut all = addressed.clone();
        all.extend(&listening);
        all.sort_unstable();
        all.dedup();
        all
    };
    if recipients.is_empty() {
        return Ok(());
    }
    // Live sessions only, not from someone the recipient blocks (#565), and
    // not where the conversation is muted; the channel's own level wins
    // over its server's. Someone addressed is left out only at "nothing",
    // someone listening is in only at "all".
    // Do not disturb (#590): no push of any kind while it lasts.
    let targets: Vec<Target> = sqlx::query_as(concat!(
        "SELECT p.endpoint, p.user_id, p.p256dh, p.auth,
                u.notifications_hide_content AS hide_content, u.locale,
                p.user_id=ANY($5) AS addressed
         FROM push_subscriptions p
         JOIN sessions s ON s.token_hash=p.session_hash AND s.expires_at>now()
         JOIN users u ON u.id=p.user_id
         LEFT JOIN notification_settings cs ON cs.user_id=p.user_id
           AND cs.scope_kind='channel' AND cs.scope_id=$2
         LEFT JOIN notification_settings ss ON ss.user_id=p.user_id
           AND ss.scope_kind='server' AND ss.scope_id=$3
         WHERE p.user_id=ANY($1)
           AND ((p.user_id=ANY($5) AND COALESCE(cs.level, ss.level, 'all')<>'none')
                OR COALESCE(cs.level, ss.level, $6)='all')
           AND NOT (cs.muted IS TRUE AND (cs.muted_until IS NULL OR cs.muted_until>now()))
           AND NOT (ss.muted IS TRUE AND (ss.muted_until IS NULL OR ss.muted_until>now()))
           AND NOT EXISTS (SELECT 1 FROM user_blocks b
                           WHERE b.blocker_id=p.user_id AND b.blocked_id=$4)
           AND NOT ",
        crate::dnd_active!()
    ))
    .bind(&recipients)
    .bind(message.channel_id)
    .bind(server)
    .bind(message.author.id)
    .bind(&addressed)
    .bind(default_level)
    .fetch_all(&state.db)
    .await?;
    if targets.is_empty() {
        return Ok(());
    }
    let (place, path) = heading(state, message, server).await?;
    let link = navigate(state, &path);
    let link = link.as_str();
    let named = if targets.iter().all(|t| t.hide_content) {
        String::new()
    } else {
        with_names(state, message).await?
    };
    let users = |addressed: bool| {
        let mut users: Vec<Uuid> = targets
            .iter()
            .filter(|t| t.addressed == addressed)
            .map(|t| t.user_id)
            .collect();
        users.sort_unstable();
        users.dedup();
        users
    };
    // Someone addressed is spared the push only by a page in front that
    // follows the conversation; someone listening by any page in front: who
    // uses the app sees the channel turn unread there.
    // Without Redis nobody counts as in front: push anyway.
    let mut in_front = state
        .gateway
        .in_front_in(&users(true), message.channel_id)
        .await
        .unwrap_or_default();
    in_front.extend(
        state
            .gateway
            .in_front(&users(false))
            .await
            .unwrap_or_default(),
    );
    let results = futures_util::stream::iter(
        targets
            .into_iter()
            .filter(|target| !in_front.contains(&target.user_id))
            .filter_map(|target| {
                let locale = target.locale;
                let (title, body) = if target.hide_content {
                    (
                        i18n::new_message_from(locale, &message.author.name),
                        String::new(),
                    )
                } else {
                    (
                        title(locale, &message.author.name, &place),
                        preview(locale, message, &named),
                    )
                };
                let payload = Payload {
                    title: &title,
                    body: &body,
                    path: &path,
                    user: target.user_id,
                    channel: message.channel_id,
                    message: message.id,
                    at: message.created_at,
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
                // A phone asleep is woken for what is meant for its owner;
                // the rest may wait for its next look at the network.
                let urgency = if target.addressed { "high" } else { "normal" };
                (
                    send(push, &target, &raw, urgency, message.id, TTL_SECS).await,
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
    // failure (a 413, an unexpected 4xx), but only in a batch where another
    // push went out: when none did, the fault is more likely ours.
    let mut failed = Vec::new();
    let mut refused = Vec::new();
    for (outcome, endpoint) in results {
        match outcome {
            Outcome::Sent => sent.push(endpoint),
            Outcome::Gone => gone.push(endpoint),
            Outcome::Refused => refused.push(endpoint),
            Outcome::Failed => failed.push(endpoint),
            Outcome::Transient => {}
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
    if !failed.is_empty() && !sent.is_empty() {
        // `created_at` moves on every re-subscribe from an opened app, so a
        // quiet but live subscription counts from then, not its last push.
        let _ = sqlx::query(
            "DELETE FROM push_subscriptions WHERE endpoint=ANY($1)
             AND GREATEST(last_ok_at, created_at) < now() - interval '30 days'",
        )
        .bind(&failed)
        .execute(&state.db)
        .await;
    }
    sent
}

/// What a request push (#563) is about. It never carries content.
#[derive(Debug, Clone, Copy)]
pub enum Invite {
    /// A friend request; a tap opens the friend list.
    Friend,
    /// The first message of a message request; a tap opens the requests.
    Message { channel: Uuid, message: Uuid },
}

/// "X wants to message you" / "X sent you a friend request" to `to`, in the
/// background. Like a DM push: not while a page of the app is in front, and
/// a message request not where the recipient set that DM to "nothing" or
/// muted it.
pub fn invite(state: &AppState, to: Uuid, from: &str, kind: Invite) {
    let Some(push) = state.push.clone() else {
        return;
    };
    let Ok(slot) = push.dm_deliveries.clone().try_acquire_owned() else {
        state.metrics.rate_limited("push_dropped");
        return;
    };
    let state = state.clone();
    let from = from.to_owned();
    tokio::spawn(async move {
        if let Err(err) = deliver_invite(&state, &push, to, &from, kind).await {
            debug!(error = err.code(), "request push not sent");
        }
        drop(slot);
    });
}

async fn deliver_invite(
    state: &AppState,
    push: &Push,
    to: Uuid,
    from: &str,
    kind: Invite,
) -> Result<(), ApiError> {
    if state.gateway.has_focus(to).await.unwrap_or(false) {
        return Ok(());
    }
    let channel = match kind {
        Invite::Message { channel, .. } => Some(channel),
        Invite::Friend => None,
    };
    // Do not disturb (#590) holds these back too.
    let targets: Vec<Target> = sqlx::query_as(concat!(
        "SELECT p.endpoint, p.user_id, p.p256dh, p.auth,
                u.notifications_hide_content AS hide_content, u.locale
         FROM push_subscriptions p
         JOIN sessions s ON s.token_hash=p.session_hash AND s.expires_at>now()
         JOIN users u ON u.id=p.user_id
         LEFT JOIN notification_settings cs ON cs.user_id=p.user_id
           AND cs.scope_kind='channel' AND cs.scope_id=$2
         WHERE p.user_id=$1
           AND COALESCE(cs.level, 'all')<>'none'
           AND NOT (cs.muted IS TRUE AND (cs.muted_until IS NULL OR cs.muted_until>now()))
           AND NOT ",
        crate::dnd_active!()
    ))
    .bind(to)
    .bind(channel)
    .fetch_all(&state.db)
    .await?;
    if targets.is_empty() {
        return Ok(());
    }
    let (path, tag, message) = match kind {
        Invite::Friend => ("/d/friends", to, Uuid::new_v4()),
        Invite::Message { channel, message } => ("/d/requests", channel, message),
    };
    let at = chrono::Utc::now();
    let link = navigate(state, path);
    let link = link.as_str();
    let results = futures_util::stream::iter(targets.into_iter().filter_map(|target| {
        let title = match kind {
            Invite::Friend => i18n::friend_request_from(target.locale, from),
            Invite::Message { .. } => i18n::message_request_from(target.locale, from),
        };
        let payload = Payload {
            title: &title,
            body: "",
            path,
            user: target.user_id,
            channel: tag,
            message,
            at,
            recall: false,
        };
        payload
            .encode(link, target.locale)
            .ok()
            .map(|raw| (target, raw))
    }))
    .map(|(target, raw)| async move {
        (
            send(push, &target, &raw, "normal", message, TTL_SECS).await,
            target.endpoint,
        )
    })
    .buffer_unordered(8)
    .collect::<Vec<_>>()
    .await;
    settle(state, push, results).await;
    Ok(())
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
                u.notifications_hide_content AS hide_content, u.locale
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
    let (place, path) = heading(state, message, server).await?;
    let link = navigate(state, &path);
    let link = link.as_str();
    let results = futures_util::stream::iter(targets.into_iter().filter_map(|target| {
        let locale = target.locale;
        let title = if target.hide_content {
            i18n::new_message_from(locale, &message.author.name)
        } else {
            title(locale, &message.author.name, &place)
        };
        let payload = Payload {
            title: &title,
            body: i18n::message_deleted(locale),
            path: &path,
            user: target.user_id,
            channel: message.channel_id,
            message: message.id,
            at: message.created_at,
            recall: true,
        };
        match payload.encode(link, locale) {
            Ok(raw) => Some((target, raw)),
            Err(err) => {
                warn!(error = %err, "push payload not encoded");
                None
            }
        }
    }))
    .map(|(target, raw)| async move {
        (
            send(push, &target, &raw, "normal", message.id, TTL_SECS).await,
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

/// The users the message names who are still in the server and still see
/// its channel: the names were resolved before the message was stored, and
/// someone removed since gets no preview.
async fn named(state: &AppState, message: &Message, server: Uuid) -> Result<Vec<Uuid>, ApiError> {
    // Hidden mentions (GEL-43) too: hidden from other readers, not from
    // whom they name.
    let mentioned = message.mentioned();
    if mentioned.is_empty() {
        return Ok(Vec::new());
    }
    Ok(sqlx::query_scalar(
        "SELECT m.user_id FROM server_members m
         JOIN servers s ON s.id=m.server_id AND s.blocked_at IS NULL
         WHERE m.server_id=$1 AND m.user_id=ANY($2)
         AND m.user_id<>$3 AND channel_visible_to($4, m.user_id)",
    )
    .bind(server)
    .bind(&mentioned)
    .bind(message.author.id)
    .bind(message.channel_id)
    .fetch_all(&state.db)
    .await?)
}

/// Whether an ordinary message of this channel may push now: the first in
/// [`ORDINARY_WINDOW`] that gets a delivery slot does ([`slot`]).
fn ordinary_may_push(state: &AppState, channel: Uuid) -> bool {
    state
        .limiter
        .hit("push-channel", &channel.to_string(), 1, ORDINARY_WINDOW)
        .is_ok()
}

/// The members who may hear an ordinary message of this channel, and the
/// level a member without a setting has for it: everyone who sees the
/// channel and "all" in a small server, else those with an "all" of their
/// own on the channel or the server and "mentions". The levels themselves
/// are read with the subscriptions.
///
/// More than [`MAX_RECIPIENTS`] with an "all" of their own are a crowd, as
/// for `@everyone`, and nobody is paged. A small server has no more than
/// [`SMALL_SERVER`] members to begin with.
async fn listening(
    state: &AppState,
    message: &Message,
    server: Uuid,
) -> Result<(Vec<Uuid>, &'static str), ApiError> {
    if default_level(state, server).await? == "all" {
        let everyone = sqlx::query_scalar(
            "SELECT m.user_id FROM server_members m
             JOIN servers s ON s.id=m.server_id AND s.blocked_at IS NULL
             WHERE m.server_id=$1 AND m.user_id<>$2 AND channel_visible_to($3, m.user_id)",
        )
        .bind(server)
        .bind(message.author.id)
        .bind(message.channel_id)
        .fetch_all(&state.db)
        .await?;
        return Ok((everyone, "all"));
    }
    let mut chose: Vec<Uuid> = sqlx::query_scalar(
        "SELECT DISTINCT n.user_id FROM notification_settings n
         JOIN server_members m ON m.server_id=$1 AND m.user_id=n.user_id
         JOIN servers s ON s.id=m.server_id AND s.blocked_at IS NULL
         WHERE n.level='all' AND n.user_id<>$2
           AND ((n.scope_kind='channel' AND n.scope_id=$3)
                OR (n.scope_kind='server' AND n.scope_id=$1))
           AND channel_visible_to($3, n.user_id)
         LIMIT $4",
    )
    .bind(server)
    .bind(message.author.id)
    .bind(message.channel_id)
    .bind(MAX_RECIPIENTS as i64 + 1)
    .fetch_all(&state.db)
    .await?;
    if chose.len() > MAX_RECIPIENTS {
        chose.clear();
    }
    Ok((chose, "mentions"))
}

/// The level a member without a setting of their own has for push in this
/// server: "all" up to [`SMALL_SERVER`] members, "mentions" above.
async fn default_level(state: &AppState, server: Uuid) -> Result<&'static str, ApiError> {
    let members: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM (SELECT 1 FROM server_members WHERE server_id=$1 LIMIT $2) m",
    )
    .bind(server)
    .bind(SMALL_SERVER + 1)
    .fetch_one(&state.db)
    .await?;
    Ok(if members <= SMALL_SERVER {
        "all"
    } else {
        "mentions"
    })
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
    /// Any other 4xx (a 413, say): counts against a silent subscription.
    Failed,
    /// Unreachable, 429 or 5xx, or our own encryption: says nothing about
    /// the subscription, so it never costs one.
    Transient,
}

/// `topic` names what the push is about: a later push with the same topic
/// replaces it at the push service while it is not delivered yet. `ttl` is
/// how many seconds the service keeps it for a device it cannot reach.
async fn send(
    push: &Push,
    target: &Target,
    payload: &[u8],
    urgency: &str,
    topic: Uuid,
    ttl: u64,
) -> Outcome {
    let Some(audience) = push.audience(&target.endpoint) else {
        return Outcome::Gone;
    };
    let Ok(receiver) = Receiver::parse(&target.p256dh, &target.auth) else {
        return Outcome::Gone;
    };
    let Ok(body) = webpush::encrypt(&receiver, payload) else {
        return Outcome::Transient;
    };
    let expires = chrono::Utc::now().timestamp() + 12 * 3600;
    let response = push
        .http
        .post(&target.endpoint)
        .header(
            "Authorization",
            push.vapid.authorization(&audience, expires),
        )
        .header("TTL", ttl.to_string())
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
        Ok(response) if response.status().is_client_error() && response.status() != 429 => {
            warn!(status = response.status().as_u16(), "push service refused");
            Outcome::Failed
        }
        Ok(response) => {
            warn!(
                status = response.status().as_u16(),
                "push service unavailable"
            );
            Outcome::Transient
        }
        Err(err) => {
            warn!(error = %err, "push service unreachable");
            Outcome::Transient
        }
    }
}

/// Where a message was written: a DM, or a server channel whose name may
/// be gone by now.
enum Place {
    Dm,
    Channel(Option<String>),
}

/// Where the message was written, and where a tap leads
/// (`conversationPath` in web/src/messages/notify.ts).
async fn heading(
    state: &AppState,
    message: &Message,
    server: Option<Uuid>,
) -> Result<(Place, String), ApiError> {
    let Some(server) = server else {
        return Ok((Place::Dm, format!("/d/{}", message.channel_id)));
    };
    let channel: Option<String> = sqlx::query_scalar("SELECT name FROM channels WHERE id=$1")
        .bind(message.channel_id)
        .fetch_optional(&state.db)
        .await?;
    Ok((
        Place::Channel(channel),
        format!("/s/{server}/c/{}", message.channel_id),
    ))
}

/// "Ada" for a DM, "Ada · #allgemein" for a server channel.
fn title(locale: Locale, author: &str, place: &Place) -> String {
    match place {
        Place::Dm => author.to_owned(),
        Place::Channel(name) => format!(
            "{author} · #{}",
            name.as_deref()
                .unwrap_or_else(|| i18n::unnamed_channel(locale))
        ),
    }
}

/// The text with the mentions of people still around as names; the same
/// for every recipient, so it is read once.
async fn with_names(state: &AppState, message: &Message) -> Result<String, ApiError> {
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
    Ok(text)
}

/// The text ([`with_names`]) cut short; a file, GIF or system message
/// named as such in the recipient's language.
fn preview(locale: Locale, message: &Message, named: &str) -> String {
    if let Some(system) = &message.system {
        return cut(&i18n::system_text(locale, system, &message.author.name));
    }
    let text = readable(named, locale);
    if !text.is_empty() {
        cut(&text)
    } else if message.gif.is_some() {
        i18n::gif(locale).into()
    } else if !message.attachments.is_empty() {
        i18n::file(locale).into()
    } else {
        String::new()
    }
}

/// Tokens of people no longer around as "@Unbekannt", whitespace folded.
fn readable(text: &str, locale: Locale) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<@") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        match after.find('>') {
            Some(end) if Uuid::parse_str(&after[..end]).is_ok() => {
                out.push('@');
                out.push_str(i18n::unknown_user(locale));
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

    /// A state with push and no database or Redis behind it.
    fn state() -> AppState {
        let (private, _) = Vapid::generate();
        let config = crate::Config::from_source(|key| match key {
            "API_ALLOW_MEMORY_STORE" => Some("true".to_owned()),
            "DATABASE_URL" => Some("postgres://unused:unused@127.0.0.1:1/unused".to_owned()),
            "REDIS_URL" => Some("redis://127.0.0.1:1".to_owned()),
            "VAPID_PRIVATE_KEY" => Some(private.clone()),
            "VAPID_SUBJECT" => Some("mailto:a@b.c".to_owned()),
            _ => None,
        })
        .unwrap();
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
            .unwrap();
        AppState::with_pool(&config, pool).unwrap()
    }

    #[tokio::test]
    async fn a_dropped_ordinary_message_leaves_its_channel_the_window() {
        let state = state();
        let push = state.push.clone().unwrap();
        let (server, channel) = (Some(Uuid::new_v4()), Uuid::new_v4());
        let held: Vec<_> = (0..MAX_DELIVERIES)
            .map(|_| push.channel_deliveries.clone().try_acquire_owned().unwrap())
            .collect();
        assert!(slot(&state, &push, channel, server, false).is_none());
        drop(held);
        // The message after it is the first that pushes.
        let (_slot, ordinary) = slot(&state, &push, channel, server, false).expect("a slot");
        assert!(ordinary);
        assert!(slot(&state, &push, channel, server, false).is_none());
        // A mention goes out within the window, to whom it names.
        let (_slot, ordinary) = slot(&state, &push, channel, server, true).expect("a slot");
        assert!(!ordinary);
    }

    #[tokio::test]
    async fn a_dropped_mention_leaves_its_channel_the_window() {
        let state = state();
        let push = state.push.clone().unwrap();
        let (server, channel) = (Some(Uuid::new_v4()), Uuid::new_v4());
        let held: Vec<_> = (0..MAX_DELIVERIES)
            .map(|_| push.server_deliveries.clone().try_acquire_owned().unwrap())
            .collect();
        assert!(slot(&state, &push, channel, server, true).is_none());
        drop(held);
        let (_slot, ordinary) = slot(&state, &push, channel, server, false).expect("a slot");
        assert!(ordinary);
        // A direct message knows no window.
        let (_slot, ordinary) = slot(&state, &push, channel, None, false).expect("a slot");
        assert!(!ordinary);
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
            readable("hi  <@0190f7a2-0000-7000-8000-00000000000a>\n!", Locale::De),
            "hi @Unbekannt !"
        );
        assert_eq!(readable("a <@x> b", Locale::De), "a <@x> b");
        let long = "x".repeat(400);
        assert_eq!(cut(&long).chars().count(), PREVIEW_CHARS);
        assert!(cut(&long).ends_with('…'));
    }
}
