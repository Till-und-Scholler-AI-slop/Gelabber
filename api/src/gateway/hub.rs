//! Local socket table + Redis 8 Pub/Sub fan-out.
//!
//! `PUBLISH` is the live path. A bounded Redis list per topic is the
//! reconnect buffer (Pub/Sub itself does not store messages). Each API
//! process keeps one `PSUBSCRIBE gb:*` connection and delivers matching
//! events to sockets subscribed on this process.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use futures_util::StreamExt;
use tokio::sync::{Mutex, RwLock, mpsc};
use tracing::{debug, warn};
use uuid::Uuid;

use super::protocol::{
    CatchUp, Event, EventDraft, REDIS_PREFIX, ServerFrame, Topic, plan_catch_up,
};
use crate::error::ApiError;

const SUBSCRIBER_RETRY: Duration = Duration::from_millis(200);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConnId(u64);

#[derive(Clone)]
pub struct Gateway {
    redis: redis::Client,
    replay: usize,
    inner: Arc<Inner>,
}

struct Inner {
    next_id: AtomicU64,
    started: AtomicBool,
    subscribed: AtomicBool,
    conn: Mutex<Option<redis::aio::MultiplexedConnection>>,
    sockets: RwLock<HashMap<ConnId, Socket>>,
}

struct Socket {
    topics: HashSet<Topic>,
    /// Live Redis events for these topics are held back until catch-up
    /// finishes, so a reconnect never sees seq 10 before 8 and 9.
    catching_up: HashSet<Topic>,
    tx: mpsc::UnboundedSender<ServerFrame>,
}

impl Gateway {
    pub fn new(redis: redis::Client, replay: usize) -> Self {
        Self {
            redis,
            replay: replay.max(1),
            inner: Arc::new(Inner {
                next_id: AtomicU64::new(1),
                started: AtomicBool::new(false),
                subscribed: AtomicBool::new(false),
                conn: Mutex::new(None),
                sockets: RwLock::new(HashMap::new()),
            }),
        }
    }

    pub fn ensure_subscriber(&self) {
        if self.inner.started.swap(true, Ordering::SeqCst) {
            return;
        }
        let this = self.clone();
        tokio::spawn(async move {
            this.run_subscriber().await;
        });
    }

    pub fn is_ready(&self) -> bool {
        self.inner.subscribed.load(Ordering::SeqCst)
    }

    pub async fn wait_ready(&self, timeout: Duration) -> Result<(), String> {
        self.ensure_subscriber();
        let started = tokio::time::Instant::now();
        while started.elapsed() < timeout {
            if self.is_ready() {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Err("redis pub/sub subscriber did not become ready".into())
    }

    pub async fn attach(&self, _user_id: Uuid, tx: mpsc::UnboundedSender<ServerFrame>) -> ConnId {
        self.ensure_subscriber();
        let id = ConnId(self.inner.next_id.fetch_add(1, Ordering::Relaxed));
        self.inner.sockets.write().await.insert(
            id,
            Socket {
                topics: HashSet::new(),
                catching_up: HashSet::new(),
                tx,
            },
        );
        id
    }

    pub async fn detach(&self, id: ConnId) {
        self.inner.sockets.write().await.remove(&id);
    }

    /// Register the topic but hold live Redis events until [`finish_catch_up`].
    pub async fn begin_catch_up(&self, id: ConnId, topic: Topic) {
        if let Some(socket) = self.inner.sockets.write().await.get_mut(&id) {
            socket.topics.insert(topic);
            socket.catching_up.insert(topic);
        }
    }

    pub async fn finish_catch_up(&self, id: ConnId, topic: Topic) {
        if let Some(socket) = self.inner.sockets.write().await.get_mut(&id) {
            socket.catching_up.remove(&topic);
        }
    }

    pub async fn unsubscribe(&self, id: ConnId, topic: Topic) {
        if let Some(socket) = self.inner.sockets.write().await.get_mut(&id) {
            socket.topics.remove(&topic);
            socket.catching_up.remove(&topic);
        }
    }

    async fn run_subscriber(&self) {
        loop {
            match self.subscribe_once().await {
                Ok(()) => warn!("redis pub/sub stream ended"),
                Err(err) => debug!(error = %err, "redis pub/sub not ready; retrying"),
            }
            self.inner.subscribed.store(false, Ordering::SeqCst);
            tokio::time::sleep(SUBSCRIBER_RETRY).await;
        }
    }

    async fn subscribe_once(&self) -> Result<(), redis::RedisError> {
        let mut pubsub = self.redis.get_async_pubsub().await?;
        pubsub.psubscribe(format!("{REDIS_PREFIX}*")).await?;
        self.inner.subscribed.store(true, Ordering::SeqCst);
        debug!("gateway subscribed to redis pub/sub");
        let mut stream = pubsub.on_message();
        while let Some(msg) = stream.next().await {
            let channel = msg.get_channel_name().to_owned();
            match msg.get_payload::<String>() {
                Ok(raw) => self.deliver_raw(&channel, &raw).await,
                Err(err) => warn!(error = %err, "invalid redis pub/sub payload"),
            }
        }
        Ok(())
    }

    async fn deliver_raw(&self, channel: &str, raw: &str) {
        let Some(topic) = Topic::from_redis_channel(channel) else {
            return;
        };
        let Ok(event) = serde_json::from_str::<Event>(raw) else {
            warn!(channel, "redis payload is not a compact event");
            return;
        };
        self.deliver(topic, ServerFrame::event(event)).await;
    }

    async fn deliver(&self, topic: Topic, frame: ServerFrame) {
        let sockets = self.inner.sockets.read().await;
        for socket in sockets.values() {
            if socket.topics.contains(&topic) && !socket.catching_up.contains(&topic) {
                let _ = socket.tx.send(frame.clone());
            }
        }
    }

    async fn redis_conn(&self) -> Result<redis::aio::MultiplexedConnection, redis::RedisError> {
        let mut slot = self.inner.conn.lock().await;
        if let Some(conn) = slot.as_ref() {
            return Ok(conn.clone());
        }
        let conn = self.redis.get_multiplexed_async_connection().await?;
        *slot = Some(conn.clone());
        Ok(conn)
    }

    async fn reset_conn(&self) {
        *self.inner.conn.lock().await = None;
    }

    async fn with_conn<T, F, Fut>(&self, mut op: F) -> Result<T, redis::RedisError>
    where
        F: FnMut(redis::aio::MultiplexedConnection) -> Fut,
        Fut: std::future::Future<Output = Result<T, redis::RedisError>>,
    {
        let conn = self.redis_conn().await?;
        match op(conn).await {
            Ok(value) => Ok(value),
            Err(err) if err.is_connection_dropped() || err.is_io_error() => {
                self.reset_conn().await;
                let conn = self.redis_conn().await?;
                op(conn).await
            }
            Err(err) => Err(err),
        }
    }

    /// Assign seq, append the replay buffer, PUBLISH. Returns the stored event.
    pub async fn publish(&self, draft: EventDraft) -> Result<Event, ApiError> {
        self.ensure_subscriber();
        let topic = draft.topic();
        let n: u64 = self
            .with_conn(|mut conn| {
                let key = topic.seq_key();
                async move { redis::cmd("INCR").arg(key).query_async(&mut conn).await }
            })
            .await
            .map_err(redis_err)?;

        let event = Event {
            t: draft.kind,
            s: draft.server_id,
            c: draft.channel_id,
            n,
            i: draft.entity_id,
            d: draft.delta,
        };
        let raw = serde_json::to_string(&event)
            .map_err(|err| ApiError::Internal(format!("serialize event: {err}")))?;

        let replay_end = (self.replay - 1) as i64;
        let log_key = topic.log_key();
        let channel = topic.redis_channel();
        self.with_conn(|mut conn| {
            let log_key = log_key.clone();
            let channel = channel.clone();
            let raw = raw.clone();
            async move {
                redis::pipe()
                    .cmd("LPUSH")
                    .arg(log_key.as_str())
                    .arg(raw.as_str())
                    .cmd("LTRIM")
                    .arg(log_key.as_str())
                    .arg(0)
                    .arg(replay_end)
                    .cmd("PUBLISH")
                    .arg(channel.as_str())
                    .arg(raw.as_str())
                    .query_async::<()>(&mut conn)
                    .await
            }
        })
        .await
        .map_err(redis_err)?;

        Ok(event)
    }

    pub async fn current_seq(&self, topic: Topic) -> Result<u64, ApiError> {
        let n: Option<u64> = self
            .with_conn(|mut conn| {
                let key = topic.seq_key();
                async move { redis::cmd("GET").arg(key).query_async(&mut conn).await }
            })
            .await
            .map_err(redis_err)?;
        Ok(n.unwrap_or(0))
    }

    pub async fn load_log(&self, topic: Topic) -> Result<Vec<Event>, ApiError> {
        let rows: Vec<String> = self
            .with_conn(|mut conn| {
                let key = topic.log_key();
                async move {
                    redis::cmd("LRANGE")
                        .arg(key)
                        .arg(0)
                        .arg(-1)
                        .query_async(&mut conn)
                        .await
                }
            })
            .await
            .map_err(redis_err)?;
        let mut events = Vec::with_capacity(rows.len());
        for raw in rows {
            match serde_json::from_str::<Event>(&raw) {
                Ok(event) => events.push(event),
                Err(err) => warn!(error = %err, "dropping unreadable replay row"),
            }
        }
        Ok(events)
    }

    pub async fn catch_up(
        &self,
        topic: Topic,
        client_n: Option<u64>,
    ) -> Result<(u64, CatchUp<Event>), ApiError> {
        let current = self.current_seq(topic).await?;
        let log = self.load_log(topic).await?;
        Ok((current, plan_catch_up(client_n, current, &log, |e| e.n)))
    }
}

fn redis_err(err: redis::RedisError) -> ApiError {
    ApiError::Internal(format!("redis: {err}"))
}
