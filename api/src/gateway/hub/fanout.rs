//! The Redis side of the connection table: one `PSUBSCRIBE gb:*`
//! subscriber per process, delivery of its messages to local sockets,
//! and the cached command connection.

use super::*;

const SUBSCRIBER_RETRY: Duration = Duration::from_millis(200);
pub(super) const CATCH_UP_CAPACITY: usize = 128;

impl ConnTable {
    pub fn ensure_subscriber(&self) {
        if self.inner.started.swap(true, Ordering::SeqCst) {
            return;
        }
        let this = self.clone();
        tokio::spawn(async move {
            this.run_subscriber().await;
        });
        let roster = VoiceRoster {
            connections: self.clone(),
        };
        tokio::spawn(async move {
            roster.run_rosters().await;
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

    /// Keeps the subscriber connected, reconnecting after a short pause.
    pub(super) async fn run_subscriber(&self) {
        loop {
            match self.subscribe_once().await {
                Ok(()) => warn!("redis pub/sub stream ended"),
                Err(err) => debug!(error = %err, "redis pub/sub not ready; retrying"),
            }
            self.inner.subscribed.store(false, Ordering::SeqCst);
            tokio::time::sleep(SUBSCRIBER_RETRY).await;
        }
    }

    pub(super) async fn resync_sockets(&self) {
        let mut sockets = self.inner.sockets.write().await;
        for socket in sockets.values_mut() {
            socket.catching_up.clear();
            socket.epochs.clear();
            socket.send(ServerFrame::Resync);
        }
    }

    pub(super) async fn subscribe_once(&self) -> Result<(), redis::RedisError> {
        let mut pubsub =
            tokio::time::timeout(Duration::from_secs(1), self.redis.get_async_pubsub())
                .await
                .map_err(|_| {
                    redis::RedisError::from(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "Redis PubSub connect",
                    ))
                })??;
        tokio::time::timeout(
            Duration::from_secs(1),
            pubsub.psubscribe(format!("{REDIS_PREFIX}*")),
        )
        .await
        .map_err(|_| {
            redis::RedisError::from(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "Redis PubSub subscribe",
            ))
        })??;
        // Revocation notices published while this process was not subscribed
        // are gone. The resync frame stands in for them: its delivery runs
        // the session and access check like every dequeued batch.
        self.resync_sockets().await;
        self.inner.subscribed.store(true, Ordering::SeqCst);
        debug!("gateway subscribed to redis pub/sub");
        let (mut sink, mut stream) = pubsub.split();
        let mut ping = tokio::time::interval(Duration::from_secs(1));
        loop {
            let msg = tokio::select! {
                message=stream.next()=> match message {Some(message)=>message,None=>break},
                _=ping.tick()=> {
                    tokio::time::timeout(Duration::from_secs(1),sink.ping::<redis::Value>()).await.map_err(|_|redis::RedisError::from(std::io::Error::new(std::io::ErrorKind::TimedOut,"Redis PubSub ping")))??;
                    continue;
                }
            };
            let channel = msg.get_channel_name().to_owned();
            match msg.get_payload::<String>() {
                Ok(raw) => self.deliver_raw(&channel, &raw).await,
                Err(err) => warn!(error = %err, "invalid redis pub/sub payload"),
            }
        }
        Ok(())
    }

    pub(super) async fn deliver_raw(&self, channel: &str, raw: &str) {
        #[derive(serde::Deserialize)]
        struct Discovery {
            c: Uuid,
        }
        if channel == RECHECK_CHANNEL {
            match serde_json::from_str::<Recheck>(raw) {
                Ok(target) => {
                    Self::recheck_local(&*self.inner.sockets.read().await, &target);
                }
                Err(_) => warn!(channel, "redis payload is not a recheck notice"),
            }
            return;
        }
        if let Some(raw_user) = channel.strip_prefix("gb:dm:") {
            if let (Ok(user), Ok(Discovery { c })) = (
                raw_user.parse::<Uuid>(),
                serde_json::from_str::<Discovery>(raw),
            ) {
                for socket in self
                    .inner
                    .sockets
                    .read()
                    .await
                    .values()
                    .filter(|socket| socket.user_id == user)
                {
                    socket.send(ServerFrame::Dm { c });
                }
            }
            return;
        }
        if let Some(raw_user) = channel.strip_prefix(USER_PREFIX) {
            match (
                raw_user.parse::<Uuid>(),
                serde_json::from_str::<super::super::protocol::UserFrame>(raw),
            ) {
                (Ok(user), Ok(frame)) => {
                    let frame = ServerFrame::from(frame);
                    for socket in self
                        .inner
                        .sockets
                        .read()
                        .await
                        .values()
                        .filter(|socket| socket.user_id == user)
                    {
                        socket.send(frame.clone());
                    }
                }
                _ => warn!(channel, "redis payload is not a user frame"),
            }
            return;
        }
        if let Some(channel_id) = voice_channel_id(channel) {
            match serde_json::from_str::<SigEvent>(raw) {
                Ok(event) => {
                    self.deliver_sig(channel_id, ServerFrame::sig(event)).await;
                }
                Err(_) => warn!(channel, "redis payload is not a sig frame"),
            }
            return;
        }
        if let Some(live) = LiveTopic::from_redis_channel(channel) {
            match serde_json::from_str::<super::super::protocol::LiveFrame>(raw) {
                Ok(frame) => self.deliver_live(live, frame.into()).await,
                Err(err) => warn!(channel, error = %err, "redis payload is not a live frame"),
            }
            return;
        }
        let Some(topic) = Topic::from_redis_channel(channel) else {
            return;
        };
        let Ok(event) = serde_json::from_str::<Event>(raw) else {
            warn!(channel, "redis payload is not a compact event");
            return;
        };
        self.deliver(topic, event).await;
    }

    pub(super) async fn deliver_sig(&self, channel_id: Uuid, frame: ServerFrame) {
        let server_id = match &frame {
            ServerFrame::Sig { s, .. } => *s,
            _ => return,
        };
        let sockets = self.inner.sockets.read().await;
        for socket in sockets.values() {
            let in_room = socket.rooms.contains_key(&channel_id);
            if in_room || socket.servers.contains(&server_id) {
                socket.send(frame.clone());
            }
        }
    }

    pub(super) async fn deliver_live(&self, live: LiveTopic, frame: ServerFrame) {
        let sockets = self.inner.sockets.read().await;
        for socket in sockets.values() {
            let want = match live {
                LiveTopic::Presence(server_id) => socket.servers.contains(&server_id),
                LiveTopic::Typing(channel_id) => {
                    socket.topics.contains(&Topic::Channel(channel_id))
                }
            };
            if want {
                socket.send(frame.clone());
            }
        }
    }

    pub(super) async fn deliver(&self, topic: Topic, event: Event) {
        let mut sockets = self.inner.sockets.write().await;
        for socket in sockets.values_mut() {
            if !socket.topics.contains(&topic) {
                continue;
            }
            if *socket.closed.borrow() {
                continue;
            }
            let buffered: usize = socket.catching_up.values().map(Vec::len).sum();
            if let Some(buf) = socket.catching_up.get_mut(&topic) {
                if buffered >= CATCH_UP_CAPACITY {
                    socket.catching_up.clear();
                    socket.closed.send_replace(true);
                    continue;
                }
                buf.push(event.clone());
                continue;
            }
            if let Some(epoch) = event.ep
                && socket
                    .epochs
                    .insert(topic, epoch)
                    .is_some_and(|previous| previous != epoch)
            {
                socket.send(ServerFrame::gap(event.s, event.c).with_epoch(epoch));
            }
            socket.send(ServerFrame::event(event.clone()));
        }
    }

    pub(super) async fn redis_conn(
        &self,
    ) -> Result<redis::aio::MultiplexedConnection, redis::RedisError> {
        let mut slot = self.inner.conn.lock().await;
        if let Some(conn) = slot.as_ref() {
            return Ok(conn.clone());
        }
        let conn = self
            .redis
            .get_multiplexed_async_connection_with_config(
                &redis::AsyncConnectionConfig::new()
                    .set_response_timeout(Some(Duration::from_secs(1)))
                    .set_connection_timeout(Some(Duration::from_secs(1))),
            )
            .await?;
        *slot = Some(conn.clone());
        Ok(conn)
    }

    pub(super) async fn reset_conn(&self) {
        *self.inner.conn.lock().await = None;
    }

    pub(in crate::gateway) async fn with_conn<T, F, Fut>(
        &self,
        mut op: F,
    ) -> Result<T, redis::RedisError>
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
}

fn voice_channel_id(name: &str) -> Option<Uuid> {
    let rest = name.strip_prefix(REDIS_PREFIX)?;
    let (kind, id) = rest.split_once(':')?;
    if kind != "v" {
        return None;
    }
    Uuid::parse_str(id).ok()
}
