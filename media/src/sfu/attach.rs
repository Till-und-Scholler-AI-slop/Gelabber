//! Attaching viewers to the room's publications and the watch grants
//! that gate them.
use super::*;

impl Sfu {
    pub(super) async fn schedule_attach(self: &Arc<Self>, channel: Uuid, id: PeerId) {
        let Ok(room) = self.room(channel).await else {
            return;
        };
        let peer = room
            .data
            .lock()
            .await
            .peers
            .get(&id)
            .map(|p| p.peer.clone());
        if let Some(peer) = peer {
            self.schedule_peer_attach(channel, peer);
        }
    }
    pub(super) fn retry_attach(self: &Arc<Self>, channel: Uuid, peer: Arc<Peer>) {
        if !peer.grant.valid() || peer.retry_scheduled.swap(true, Ordering::SeqCst) {
            return;
        }
        let failures = peer.attach_failures.fetch_add(1, Ordering::Relaxed).min(5);
        let delay = Duration::from_millis(250 * (1 << failures));
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            peer.retry_scheduled.store(false, Ordering::SeqCst);
            if let Some(sfu) = weak.upgrade()
                && peer.grant.valid()
            {
                sfu.schedule_peer_attach(channel, peer);
            }
        });
    }
    pub(super) fn schedule_peer_attach(self: &Arc<Self>, channel: Uuid, peer: Arc<Peer>) {
        if !peer.grant.valid() {
            return;
        }
        peer.attach_dirty.store(true, Ordering::SeqCst);
        if peer.attach_scheduled.swap(true, Ordering::SeqCst) {
            return;
        }
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            loop {
                peer.attach_dirty.store(false, Ordering::SeqCst);
                if let Some(sfu) = weak.upgrade()
                    && peer.grant.valid()
                {
                    sfu.attach_existing(channel, peer.id).await;
                }
                peer.attach_scheduled.store(false, Ordering::SeqCst);
                if !peer.grant.valid()
                    || !peer.attach_dirty.load(Ordering::SeqCst)
                    || peer
                        .attach_scheduled
                        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                        .is_err()
                {
                    break;
                }
            }
        });
    }
    pub(super) async fn schedule_room_attach(self: &Arc<Self>, channel: Uuid) {
        let Ok(room) = self.room(channel).await else {
            return;
        };
        let peers: Vec<_> = room
            .data
            .lock()
            .await
            .peers
            .values()
            .map(|p| p.peer.clone())
            .collect();
        for peer in peers {
            self.schedule_peer_attach(channel, peer);
        }
    }
    pub(super) async fn attach_existing(self: &Arc<Self>, channel: Uuid, id: PeerId) {
        let Ok(room) = self.room(channel).await else {
            return;
        };
        let pubs: Vec<_> = room
            .data
            .lock()
            .await
            .publications
            .values()
            .cloned()
            .collect();
        let engine = room.engine.clone();
        let results = self
            .attach_batch(&room, id, pubs, |transport, options| {
                let engine = engine.clone();
                async move {
                    engine
                        .native(transport, move |transport| async move {
                            transport.consume(options).await
                        })
                        .await
                }
            })
            .await;
        for err in results.into_iter().filter_map(Result::err) {
            if !matches!(err, SfuError::Forbidden | SfuError::Unavailable) {
                warn!(?id, code = err.code(), "consumer attach failed");
            }
        }
    }
    /// One attach pass: creates paused consumers for `pubs`, a few at a time,
    /// then checks the viewer's authority once and announces them. The check
    /// follows every creation of the pass, so it is as late for each consumer
    /// as a check per consumer would be, without a Redis round trip each.
    pub(super) async fn attach_batch<C, F>(
        self: &Arc<Self>,
        room: &Arc<Room>,
        id: PeerId,
        pubs: Vec<Arc<Publication>>,
        create: C,
    ) -> Vec<Result<(), SfuError>>
    where
        C: Fn(WebRtcTransport, ConsumerOptions) -> F + Sync,
        F: Future<Output = Result<Consumer, SfuError>> + Send,
    {
        let prepared: Vec<_> = stream::iter(pubs)
            .map(|pubn| self.prepare_consumer(room, id, pubn, &create))
            .buffer_unordered(ATTACH_CONCURRENCY)
            .collect()
            .await;
        let mut results = Vec::with_capacity(prepared.len());
        let mut reserved = Vec::new();
        for item in prepared {
            match item {
                Ok(Some(p)) => reserved.push(p),
                Ok(None) => results.push(Ok(())),
                Err(error) => results.push(Err(error)),
            }
        }
        let Some(peer) = reserved.first().map(|p| p.peer.clone()) else {
            return results;
        };
        let check = if reserved.iter().any(|p| p.created.is_ok()) {
            crate::ticket::validate_authority(&self.redis, &peer.authority).await
        } else {
            Ok(true)
        };
        // A Redis failure denies the pass like a revocation but is retried,
        // since it fails every consumer of the join at once.
        let redis_failed = check.is_err();
        let authorized = check.unwrap_or(false);
        for p in &mut reserved {
            if let Ok(sub) = &p.created
                && (!authorized || !sub.valid())
            {
                stop_consumer(&sub.consumer).await;
                p.created = Err(if redis_failed {
                    SfuError::Unavailable
                } else {
                    SfuError::Revoked
                });
            }
        }
        let (mut recreated, mut retry_native) = (false, redis_failed);
        for p in reserved {
            let result = match p.created {
                Ok(sub) => self.commit_consumer(room, id, &p.transport, sub).await,
                Err(error) => Err(error),
            };
            recreated |= {
                let mut data = room.data.lock().await;
                data.peers.get_mut(&id).is_some_and(|d| {
                    d.pending_consumers.remove(&p.producer_id);
                    d.transports
                        .get(&false)
                        .is_some_and(|t| t.id() != p.transport.id())
                })
            };
            retry_native |= p.retry_native;
            results.push(result);
        }
        if recreated {
            self.schedule_attach(peer.authority.claim.c, id).await;
        } else if retry_native {
            // Reservations are cleared before retry, and the next attempt rechecks
            // grants, capabilities, capacity and current receive transport.
            self.retry_attach(peer.authority.claim.c, peer);
        }
        results
    }
    /// Reserves `pubn` for the viewer and creates its paused native consumer;
    /// announces nothing. `Ok(None)`: already consumed, pending or given up.
    pub(super) async fn prepare_consumer<C, F>(
        &self,
        room: &Room,
        id: PeerId,
        pubn: Arc<Publication>,
        create: &C,
    ) -> Result<Option<PreparedConsumer>, SfuError>
    where
        C: Fn(WebRtcTransport, ConsumerOptions) -> F + Sync,
        F: Future<Output = Result<Consumer, SfuError>> + Send,
    {
        let producer_id = pubn.id();
        let (peer, transport, capabilities, watch) = {
            let mut data = room.data.lock().await;
            let p = data.peers.get_mut(&id).ok_or(SfuError::Revoked)?;
            let watch = receives(p, &pubn)?;
            if p.pending_consumers.contains(&producer_id)
                || p.consumers
                    .values()
                    .any(|s| s.publication.id() == producer_id)
                || p.failed_consumers
                    .get(&producer_id)
                    .is_some_and(|failures| *failures >= CONSUMER_FAILURE_LIMIT)
            {
                return Ok(None);
            }
            if p.consumers.len() + p.pending_consumers.len() >= CONSUMER_LIMIT {
                return Err(SfuError::Unavailable);
            }
            let transport = p
                .transports
                .get(&false)
                .cloned()
                .ok_or(SfuError::Unavailable)?;
            let caps = p.capabilities.clone().ok_or(SfuError::Unavailable)?;
            p.pending_consumers.insert(producer_id.clone());
            (p.peer.clone(), transport, caps, watch)
        };
        let mut retry_native = false;
        let created = async {
            if !room
                .router
                .get()
                .ok_or(SfuError::Unavailable)?
                .can_consume(&pubn.producer.id(), &capabilities)
            {
                return Err(SfuError::UnsupportedCodec);
            }
            let mut options = ConsumerOptions::new(pubn.producer.id(), capabilities);
            options.paused = true;
            // No preferred layers: mediasoup starts on the highest one and its
            // bandwidth estimate steps down, whatever size the tile is.
            let consumer = {
                let _slot = room
                    .engine
                    .attach
                    .acquire()
                    .await
                    .map_err(|_| SfuError::Unavailable)?;
                match create(transport.clone(), options).await {
                    Ok(consumer) => consumer,
                    Err(error) => {
                        retry_native =
                            matches!(error, SfuError::Unavailable | SfuError::Negotiation(_));
                        return Err(error);
                    }
                }
            };
            Ok(Arc::new(Subscription {
                consumer: NativeConsumer::new(
                    consumer,
                    transport.clone(),
                    room.engine.lost.clone(),
                ),
                publication: pubn,
                peer: peer.clone(),
                watch,
                grant: Grant::new(),
                generation: Uuid::new_v4(),
                gate: Mutex::new(()),
            }))
        }
        .await;
        Ok(Some(PreparedConsumer {
            producer_id,
            transport,
            peer,
            created,
            retry_native,
        }))
    }
    /// Installs and announces an authorized consumer unless its grants or the
    /// viewer's receive transport changed meanwhile.
    pub(super) async fn commit_consumer(
        self: &Arc<Self>,
        room: &Arc<Room>,
        id: PeerId,
        transport: &WebRtcTransport,
        sub: Arc<Subscription>,
    ) -> Result<(), SfuError> {
        {
            let mut data = room.data.lock().await;
            let p = data.peers.get_mut(&id).ok_or(SfuError::Revoked)?;
            if !sub.valid()
                || !p
                    .transports
                    .get(&false)
                    .is_some_and(|t| t.id() == transport.id())
            {
                return Err(SfuError::Forbidden);
            }
            p.consumers
                .insert(sub.consumer.id().to_string(), sub.clone());
            self.counters.consumers.fetch_add(1, Ordering::Relaxed);
        }
        self.observe_consumer(room, &sub);
        self.emit(
            &sub.peer,
            ServerFrame::Consumer {
                consumer_id: sub.consumer.id().to_string(),
                producer_id: sub.publication.id(),
                owner: sub.publication.peer.authority.claim.u,
                k: sub.publication.kind,
                epoch: sub.publication.epoch,
                generation: sub.generation,
                parent: sub.publication.parent.as_ref().map(|p| p.id()),
                kind: if sub.publication.kind.is_video() {
                    "video"
                } else {
                    "audio"
                }
                .into(),
                rtp_parameters: serde_json::to_value(sub.consumer.rtp_parameters())
                    .map_err(SfuError::negotiation)?,
                paused: sub.publication.producer.paused(),
            },
        );
        Ok(())
    }
    pub(super) async fn set_watch(
        self: &Arc<Self>,
        channel: Uuid,
        room: &Arc<Room>,
        peer: &Arc<Peer>,
        user: Uuid,
        kind: WatchKind,
        on: bool,
    ) -> Result<(), SfuError> {
        if user.is_nil() || user == peer.authority.claim.u {
            return Err(SfuError::Forbidden);
        }
        let stopped = {
            let mut data = room.data.lock().await;
            let p = data.peers.get_mut(&peer.id).ok_or(SfuError::Revoked)?;
            let key = (user, kind);
            if on {
                if p.watches.len() >= WATCH_LIMIT && !p.watches.contains_key(&key) {
                    return Err(SfuError::BadAnnounce);
                }
                p.watches.entry(key).or_insert_with(Grant::new);
                Vec::new()
            } else {
                if let Some(g) = p.watches.remove(&key) {
                    g.stop()
                }
                p.consumers
                    .values()
                    .filter(|s| !s.valid())
                    .cloned()
                    .collect::<Vec<_>>()
            }
        };
        self.confirmed_stop_resources(&[], &stopped).await;
        for sub in stopped {
            self.forget_consumer(room, &sub).await;
        }
        if on {
            self.schedule_attach(channel, peer.id).await
        }
        Ok(())
    }
}

pub(super) fn receives(
    peer: &PeerData,
    pubn: &Publication,
) -> Result<Option<Arc<Grant>>, SfuError> {
    let owner = pubn.peer.authority.claim.u;
    if !peer.peer.grant.valid() || !pubn.valid() || owner == peer.peer.authority.claim.u {
        return Err(SfuError::Forbidden);
    }
    match pubn.kind {
        SourceKind::Mic | SourceKind::Camera => Ok(None),
        SourceKind::Screen | SourceKind::ScreenAudio => peer
            .watches
            .get(&(owner, WatchKind::Screen))
            .filter(|g| g.valid())
            .cloned()
            .map(Some)
            .ok_or(SfuError::Forbidden),
    }
}
