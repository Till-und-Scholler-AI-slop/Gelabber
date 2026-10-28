//! Signaling RPCs from a joined peer: transports, produce and consume.
use super::*;

impl Sfu {
    pub async fn rpc(
        self: &Arc<Self>,
        id: PeerId,
        channel: Uuid,
        frame: ClientFrame,
    ) -> Result<Value, SfuError> {
        let room = self.room(channel).await?;
        let peer = room
            .data
            .lock()
            .await
            .peers
            .get(&id)
            .ok_or(SfuError::NotInRoom)?
            .peer
            .clone();
        let _rpc = peer.rpc_gate.lock().await;
        if !peer.grant.valid() || !self.room_ready(&room) {
            self.leave(id, channel).await;
            return Err(SfuError::Revoked);
        }
        if !crate::ticket::validate_authority(&self.redis, &peer.authority)
            .await
            .unwrap_or(false)
        {
            self.leave(id, channel).await;
            return Err(SfuError::Revoked);
        }
        match frame {
            ClientFrame::Capabilities { rtp, .. } => {
                let caps: RtpCapabilities =
                    serde_json::from_value(rtp).map_err(|_| SfuError::BadAnnounce)?;
                if caps.codecs.len() > 64 || caps.header_extensions.len() > 32 {
                    return Err(SfuError::BadAnnounce);
                }
                let mut data = room.data.lock().await;
                let p = data.peers.get_mut(&id).ok_or(SfuError::Revoked)?;
                if p.capabilities.is_some() {
                    return Err(SfuError::BadAnnounce);
                }
                p.capabilities = Some(caps);
                let changed = data.update_video_codecs(Some(id));
                let codecs =
                    json!({ "video": data.video_codecs, "revision": data.codecs_revision });
                drop(data);
                self.announce_video_codecs(changed);
                self.schedule_attach(channel, id).await;
                Ok(json!({ "codecs": codecs }))
            }
            ClientFrame::CreateTransport { direction, .. } => {
                let send = direction == TransportDirection::Send;
                if room
                    .data
                    .lock()
                    .await
                    .peers
                    .get(&id)
                    .ok_or(SfuError::Revoked)?
                    .transports
                    .contains_key(&send)
                {
                    return Err(SfuError::BadAnnounce);
                }
                let opts = gateway_transport_options(room.engine.server.clone());
                let transport = room
                    .engine
                    .native(
                        room.router.get().ok_or(SfuError::Unavailable)?.clone(),
                        move |router| async move { router.create_webrtc_transport(opts).await },
                    )
                    .await?;
                let weak = Arc::downgrade(self);
                transport
                    .on_ice_state_change(move |state| {
                        if state == IceState::Disconnected
                            && let Some(sfu) = weak.upgrade()
                        {
                            sfu.counters.ice_disconnects.fetch_add(1, Ordering::Relaxed);
                        }
                    })
                    .detach();
                let result = json!({"id":transport.id(),"iceParameters":transport.ice_parameters(),"iceCandidates":transport.ice_candidates(),"dtlsParameters":transport.dtls_parameters()});
                let mut data = room.data.lock().await;
                let p = data
                    .peers
                    .get_mut(&id)
                    .filter(|p| p.peer.grant.valid())
                    .ok_or(SfuError::Revoked)?;
                p.transports.insert(send, transport);
                self.counters.transports.fetch_add(1, Ordering::Relaxed);
                drop(data);
                if !send {
                    self.schedule_attach(channel, id).await
                }
                Ok(result)
            }
            ClientFrame::ConnectTransport {
                transport_id, dtls, ..
            } => {
                let t = self.transport(&room, id, &transport_id).await?;
                let dtls_parameters = serde_json::from_value::<DtlsParameters>(dtls)
                    .map_err(|_| SfuError::BadAnnounce)?;
                room.engine
                    .native(t, move |t| async move {
                        t.connect(WebRtcTransportRemoteParameters { dtls_parameters })
                            .await
                    })
                    .await?;
                if !peer.grant.valid() {
                    return Err(SfuError::Revoked);
                }
                Ok(json!({}))
            }
            ClientFrame::RestartIce { transport_id, .. } => {
                let t = self.transport(&room, id, &transport_id).await?;
                let ice = room
                    .engine
                    .native(t, |t| async move { t.restart_ice().await })
                    .await?;
                if !peer.grant.valid() {
                    return Err(SfuError::Revoked);
                }
                Ok(json!({"iceParameters":ice}))
            }
            ClientFrame::CloseTransport { transport_id, .. } => {
                self.close_transport(&room, &peer, &transport_id).await?;
                Ok(json!({}))
            }
            ClientFrame::Produce {
                k,
                rtp,
                epoch,
                parent,
                expected_old_producer_id,
                paused,
                ..
            } => {
                self.produce(
                    &room,
                    &peer,
                    k,
                    rtp,
                    epoch,
                    parent,
                    expected_old_producer_id,
                    paused,
                )
                .await
            }
            ClientFrame::PauseProducer { producer_id, .. } => {
                let p = self.own_publication(&room, id, &producer_id).await?;
                let _gate = p.gate.lock().await;
                stop_producer(&p.producer).await;
                Ok(json!({}))
            }
            ClientFrame::ResumeProducer { producer_id, .. } => {
                let p = self.own_publication(&room, id, &producer_id).await?;
                let _gate = p.gate.lock().await;
                if !p.valid() {
                    return Err(SfuError::Forbidden);
                }
                if let Err(error) = room
                    .engine
                    .native(p.clone(), |resource| async move {
                        resource.producer.resume().await
                    })
                    .await
                {
                    p.producer.confirm_closed().await;
                    return Err(error);
                }
                if !p.valid() {
                    stop_producer(&p.producer).await;
                    return Err(SfuError::Forbidden);
                }
                Ok(json!({}))
            }
            ClientFrame::CloseProducer { producer_id, .. } => {
                if room
                    .data
                    .lock()
                    .await
                    .peers
                    .get(&id)
                    .is_some_and(|p| p.retired_producers.contains(&producer_id))
                {
                    return Ok(json!({}));
                }
                let p = self.own_publication(&room, id, &producer_id).await?;
                self.retire_publication(&room, &p).await;
                Ok(json!({}))
            }
            ClientFrame::ConsumerReady {
                consumer_id,
                generation,
                ..
            } => {
                let s = self
                    .own_consumer(&room, id, &consumer_id, generation)
                    .await?;
                let _gate = s.gate.lock().await;
                if !s.valid() {
                    return Err(SfuError::Forbidden);
                }
                if let Err(error) = room
                    .engine
                    .native(s.clone(), |resource| async move {
                        resource.consumer.resume().await
                    })
                    .await
                {
                    s.consumer.confirm_closed().await;
                    drop(_gate);
                    self.remove_consumer(&room, &s).await;
                    self.retry_attach(channel, peer.clone());
                    return Err(error);
                }
                if !s.valid() {
                    stop_consumer(&s.consumer).await;
                    return Err(SfuError::Forbidden);
                }
                peer.attach_failures.store(0, Ordering::Relaxed);
                self.emit(
                    &peer,
                    ServerFrame::ConsumerState {
                        consumer_id,
                        generation,
                        paused: s.consumer.paused() || s.consumer.producer_paused(),
                    },
                );
                Ok(json!({}))
            }
            ClientFrame::ConsumerFailed {
                consumer_id,
                generation,
                ..
            } => {
                let s = self
                    .own_consumer(&room, id, &consumer_id, generation)
                    .await?;
                self.remove_consumer(&room, &s).await;
                let retry = {
                    let mut data = room.data.lock().await;
                    let RoomData {
                        peers,
                        publications,
                        ..
                    } = &mut *data;
                    peers.get_mut(&id).is_some_and(|p| {
                        note_consumer_failure(
                            &mut p.failed_consumers,
                            |producer| publications.contains_key(producer),
                            s.publication.id(),
                        )
                    })
                };
                // A producer this browser cannot decode would otherwise be
                // re-consumed forever; a new producer ID starts a fresh budget.
                if retry {
                    self.retry_attach(channel, peer.clone());
                }
                Ok(json!({}))
            }
            ClientFrame::Watch { u, k, on, .. } => {
                self.set_watch(channel, &room, &peer, u, k, on).await?;
                Ok(json!({}))
            }
            // Older clients still send tile sizes; consumers start on the top
            // layer and mediasoup's estimate steps down, so they are ignored.
            ClientFrame::ViewerLayer { .. } => Ok(json!({})),
            ClientFrame::Leave { .. } => {
                self.leave(id, channel).await;
                Ok(json!({}))
            }
            ClientFrame::Join { .. } => Err(SfuError::BadAnnounce),
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn produce(
        self: &Arc<Self>,
        room: &Arc<Room>,
        peer: &Arc<Peer>,
        kind: SourceKind,
        rtp: Value,
        epoch: Uuid,
        parent_id: Option<String>,
        expected_old: Option<String>,
        paused: bool,
    ) -> Result<Value, SfuError> {
        if epoch.is_nil() {
            return Err(SfuError::Forbidden);
        }
        // Going live (a screen and its sound) needs the ticket's go_live
        // bit, not only the API's publish check.
        if !matches!(kind, SourceKind::Mic | SourceKind::Camera) && !peer.authority.claim.g {
            return Err(SfuError::Forbidden);
        }
        let mut parameters: RtpParameters =
            serde_json::from_value(rtp).map_err(|_| SfuError::BadAnnounce)?;
        // The pinned public Chrome handler returns an empty CNAME for RID-only
        // offers. Treat that absent identity as None so the official Rust SDK
        // chooses its documented transport CNAME. Some("") would poison that
        // SDK's transport-wide cache before the native worker rejects it.
        if parameters.rtcp.cname.as_deref() == Some("") {
            parameters.rtcp.cname = None;
        }
        validate_parameters(kind, &parameters)?;
        let (transport, old, parent) = {
            let data = room.data.lock().await;
            let p = data.peers.get(&peer.id).ok_or(SfuError::Revoked)?;
            let old = p.publications.get(&kind).cloned();
            if old.as_ref().map(|p| p.id()) != expected_old {
                return Err(SfuError::Forbidden);
            }
            let parent = if let Some(pk) = kind.parent() {
                let parent = p
                    .publications
                    .get(&pk)
                    .filter(|p| {
                        p.valid() && p.epoch == epoch && Some(p.id()).as_ref() == parent_id.as_ref()
                    })
                    .ok_or(SfuError::Forbidden)?
                    .clone();
                Some(parent)
            } else {
                if parent_id.is_some() {
                    return Err(SfuError::BadAnnounce);
                }
                None
            };
            (
                p.transports
                    .get(&true)
                    .cloned()
                    .ok_or(SfuError::BadAnnounce)?,
                old,
                parent,
            )
        };
        let result = async {
            let mut options = ProducerOptions::new(
                if kind.is_video() {
                    MediaKind::Video
                } else {
                    MediaKind::Audio
                },
                parameters,
            );
            options.paused = true;
            let producer = room
                .engine
                .native(transport.clone(), move |transport| async move {
                    transport.produce(options).await
                })
                .await?;
            let pubn = Arc::new(Publication {
                producer: NativeProducer::new(
                    producer,
                    transport.clone(),
                    room.engine.lost.clone(),
                ),
                peer: peer.clone(),
                kind,
                epoch,
                parent,
                grant: Grant::new(),
                gate: Mutex::new(()),
                packet_samples: StdMutex::new(HashMap::new()),
            });
            if !crate::ticket::validate_authority(&self.redis, &peer.authority)
                .await
                .unwrap_or(false)
                || !pubn.valid()
            {
                stop_producer(&pubn.producer).await;
                return Err(SfuError::Revoked);
            }
            // Atomically replace the slot with a still-paused new Producer. This actual
            // ID fences timeout/close callbacks from the retired Producer, even if the
            // browser kept the same capture epoch for a codec-only replacement.
            {
                let mut data = room.data.lock().await;
                let p = data
                    .peers
                    .get_mut(&peer.id)
                    .filter(|p| p.peer.grant.valid())
                    .ok_or(SfuError::Revoked)?;
                if p.publications.get(&kind).map(|p| p.id()) != expected_old
                    || old.as_ref().is_some_and(|o| !o.valid())
                    || !pubn.valid()
                {
                    return Err(SfuError::Forbidden);
                }
                if let Some(old) = &old {
                    old.grant.stop()
                }
                p.publications.insert(kind, pubn.clone());
            }
            if let Some(old) = &old {
                self.retire_publication(room, old).await;
            }
            let committed = {
                let mut data = room.data.lock().await;
                if !pubn.valid()
                    || !data
                        .peers
                        .get(&peer.id)
                        .and_then(|p| p.publications.get(&kind))
                        .is_some_and(|p| Arc::ptr_eq(p, &pubn))
                {
                    false
                } else {
                    data.publications.insert(pubn.id(), pubn.clone());
                    self.counters.producers.fetch_add(1, Ordering::Relaxed);
                    true
                }
            };
            if !committed {
                stop_producer(&pubn.producer).await;
                return Err(SfuError::Revoked);
            }
            if !paused {
                let _gate = pubn.gate.lock().await;
                if !pubn.valid() {
                    return Err(SfuError::Forbidden);
                }
                if let Err(error) = room
                    .engine
                    .native(pubn.clone(), |resource| async move {
                        resource.producer.resume().await
                    })
                    .await
                {
                    pubn.producer.confirm_closed().await;
                    return Err(error);
                }
                if !pubn.valid() {
                    stop_producer(&pubn.producer).await;
                    return Err(SfuError::Forbidden);
                }
            }
            self.observe_producer(room, &pubn);
            self.schedule_room_attach(peer.authority.claim.c).await;
            Ok(json!({"producerId":pubn.producer.id(),"epoch":epoch}))
        }
        .await;
        if result.is_err() {
            let failed = room
                .data
                .lock()
                .await
                .peers
                .get(&peer.id)
                .and_then(|p| p.publications.get(&kind))
                .filter(|p| Some(p.id()) != expected_old)
                .cloned();
            if let Some(failed) = failed {
                self.retire_publication(room, &failed).await;
            }
        }
        result
    }
    pub(super) async fn transport(
        &self,
        room: &Room,
        peer: PeerId,
        id: &str,
    ) -> Result<WebRtcTransport, SfuError> {
        room.data
            .lock()
            .await
            .peers
            .get(&peer)
            .ok_or(SfuError::Revoked)?
            .transports
            .values()
            .find(|t| t.id().to_string() == id)
            .cloned()
            .ok_or(SfuError::Forbidden)
    }
    pub(super) async fn close_transport(
        self: &Arc<Self>,
        room: &Arc<Room>,
        peer: &Arc<Peer>,
        id: &str,
    ) -> Result<(), SfuError> {
        let (direction, transport, publications, consumers) = {
            let data = room.data.lock().await;
            let p = data.peers.get(&peer.id).ok_or(SfuError::Revoked)?;
            if p.retired_transports.iter().any(|owned| owned == id) {
                return Ok(());
            }
            let (direction, transport) = p
                .transports
                .iter()
                .find(|(_, t)| t.id().to_string() == id)
                .ok_or(SfuError::Forbidden)?;
            let publications = p
                .publications
                .values()
                .filter(|p| p.producer.transport().id().to_string() == id)
                .cloned()
                .collect::<Vec<_>>();
            let consumers = p
                .consumers
                .values()
                .filter(|s| s.consumer.transport().id().to_string() == id)
                .cloned()
                .collect::<Vec<_>>();
            (*direction, transport.clone(), publications, consumers)
        };
        self.confirmed_stop_resources(&publications, &consumers)
            .await;
        for publication in publications {
            self.retire_publication(room, &publication).await;
        }
        for consumer in consumers {
            self.forget_consumer(room, &consumer).await;
        }
        self.sample_transport(&transport).await;
        let mut data = room.data.lock().await;
        if let Some(p) = data.peers.get_mut(&peer.id)
            && p.transports
                .get(&direction)
                .is_some_and(|t| t.id() == transport.id())
        {
            p.transports.remove(&direction);
            remember_retired(&mut p.retired_transports, id.to_owned());
            self.counters.transports.fetch_sub(1, Ordering::Relaxed);
        }
        Ok(())
    }
    pub(super) async fn own_publication(
        &self,
        room: &Room,
        peer: PeerId,
        id: &str,
    ) -> Result<Arc<Publication>, SfuError> {
        room.data
            .lock()
            .await
            .publications
            .get(id)
            .filter(|p| p.peer.id == peer && p.grant.valid())
            .cloned()
            .ok_or(SfuError::Forbidden)
    }
    pub(super) async fn own_consumer(
        &self,
        room: &Room,
        peer: PeerId,
        id: &str,
        generation: Uuid,
    ) -> Result<Arc<Subscription>, SfuError> {
        room.data
            .lock()
            .await
            .peers
            .get(&peer)
            .and_then(|p| p.consumers.get(id))
            .filter(|s| s.generation == generation)
            .cloned()
            .ok_or(SfuError::Forbidden)
    }
}

/// Counts a browser consumer failure; returns whether another attempt is allowed.
/// Entries for producers that left the room are dropped first.
pub(super) fn note_consumer_failure(
    failed: &mut HashMap<String, u8>,
    live: impl Fn(&str) -> bool,
    producer: String,
) -> bool {
    failed.retain(|id, _| live(id));
    let failures = failed.entry(producer).or_default();
    *failures = failures.saturating_add(1);
    *failures < CONSUMER_FAILURE_LIMIT
}

/// Options for every WebRTC transport the gateway creates.
pub(super) fn gateway_transport_options(server: WebRtcServer) -> WebRtcTransportOptions {
    let mut opts = WebRtcTransportOptions::new_with_server(server);
    opts.enable_tcp = false;
    opts.prefer_udp = true;
    // mediasoup's 600 kbit/s default starts every video consumer on a low
    // layer; its estimate climbs from here instead.
    opts.initial_available_outgoing_bitrate = 8_000_000;
    // Last sample belongs to the native transport lifetime, keeping
    // historical totals without retaining an unbounded ID cache.
    opts.app_data = AppData::new(AtomicU64::new(0));
    opts
}
