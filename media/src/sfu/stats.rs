//! Prometheus counters, native RTP stats sampling and the producer and
//! consumer event observers that feed them.
use super::*;

impl Sfu {
    pub fn metrics_text(&self) -> String {
        let c = &self.counters;
        let mut text = String::new();
        for (name, value) in [
            ("gelabber_sfu_rooms", self.room_count()),
            ("gelabber_sfu_peers", c.peers.load(Ordering::Relaxed)),
            ("gelabber_media_rooms", self.room_count()),
            ("gelabber_media_peers", c.peers.load(Ordering::Relaxed)),
            (
                "gelabber_mediasoup_producers",
                c.producers.load(Ordering::Relaxed),
            ),
            (
                "gelabber_mediasoup_consumers",
                c.consumers.load(Ordering::Relaxed),
            ),
            (
                "gelabber_mediasoup_transports",
                c.transports.load(Ordering::Relaxed),
            ),
            (
                "gelabber_mediasoup_stats_available",
                usize::from(c.stats_available.load(Ordering::Relaxed)),
            ),
        ] {
            text.push_str(&format!("# TYPE {name} gauge\n{name} {value}\n"));
        }
        if c.stats_available.load(Ordering::Relaxed) || c.transports.load(Ordering::Relaxed) == 0 {
            // Actual delta sum from native RTP counters, including retired transports.
            text.push_str(&format!("# HELP gelabber_media_forwarded_bytes_total Measured native RTP byte deltas including RTP headers; failed final samples may omit bytes.\n# TYPE gelabber_media_forwarded_bytes_total counter\ngelabber_media_forwarded_bytes_total {value}\n# TYPE gelabber_sfu_forwarded_bytes_total counter\ngelabber_sfu_forwarded_bytes_total {value}\n",value=c.rtp_sent.load(Ordering::Relaxed)));
        }
        text.push_str(&format!("# TYPE gelabber_mediasoup_ice_disconnects_total counter\ngelabber_mediasoup_ice_disconnects_total {}\n",c.ice_disconnects.load(Ordering::Relaxed)));
        text.push_str(&format!("# TYPE gelabber_media_outbound_overflows_total counter\ngelabber_media_outbound_overflows_total {}\n", c.outbound_overflows.load(Ordering::Relaxed)));
        let workers = self.workers();
        text.push_str(&format!("# HELP gelabber_media_workers Configured native mediasoup workers.\n# TYPE gelabber_media_workers gauge\ngelabber_media_workers {}\n# HELP gelabber_media_workers_live Workers that accept rooms; fewer than configured while a lost worker is replaced.\n# TYPE gelabber_media_workers_live gauge\ngelabber_media_workers_live {}\n", workers.total, workers.live));
        text.push_str(&format!("# TYPE gelabber_media_worker_failures_total counter\ngelabber_media_worker_failures_total {}\n# TYPE gelabber_media_worker_restarts_total counter\ngelabber_media_worker_restarts_total {}\n", c.worker_failures.load(Ordering::Relaxed), c.worker_restarts.load(Ordering::Relaxed)));
        let engines = self.engines.lock().unwrap().clone();
        text.push_str("# TYPE gelabber_media_worker_rooms gauge\n");
        for engine in &engines {
            text.push_str(&format!(
                "gelabber_media_worker_rooms{{worker=\"{}\"}} {}\n",
                engine.slot,
                engine.rooms.load(Ordering::Relaxed)
            ));
        }
        text.push_str("# HELP gelabber_media_worker_down_seconds Time since the slot lost its worker, 0 while it has a live one; a growing value means the replacement keeps failing (a hung worker holds its port until a restart).\n# TYPE gelabber_media_worker_down_seconds gauge\n");
        for engine in &engines {
            text.push_str(&format!(
                "gelabber_media_worker_down_seconds{{worker=\"{}\"}} {}\n",
                engine.slot,
                engine.down_for().as_secs()
            ));
        }
        text.push_str("# TYPE gelabber_media_worker_peers gauge\n");
        for engine in &engines {
            text.push_str(&format!(
                "gelabber_media_worker_peers{{worker=\"{}\"}} {}\n",
                engine.slot,
                engine.peers.load(Ordering::Relaxed)
            ));
        }
        text.push_str("# HELP gelabber_media_rtp_packets_total Measured native source packet deltas; failed final samples may omit packets; absent when current input stats are unavailable.\n# TYPE gelabber_media_rtp_packets_total counter\n# HELP gelabber_media_rtp_jitter_ms Maximum current native source jitter converted from RTP clock ticks; absent when unavailable.\n# TYPE gelabber_media_rtp_jitter_ms gauge\n# HELP gelabber_media_rtp_lost_total Unavailable: native signed loss gauges do not preserve the previous reorder-aware monotonic counter.\n");
        if c.input_available.load(Ordering::Relaxed) || c.producers.load(Ordering::Relaxed) == 0 {
            text.push_str(&format!(
                "gelabber_media_rtp_packets_total {}\n",
                c.rtp_received.load(Ordering::Relaxed)
            ));
        }
        if c.input_available.load(Ordering::Relaxed) {
            text.push_str(&format!(
                "gelabber_media_rtp_jitter_ms {}\n",
                c.jitter_microseconds.load(Ordering::Relaxed) as f64 / 1000.0
            ));
        }
        text
    }
    pub(super) fn observe_producer(self: &Arc<Self>, room: &Arc<Room>, pubn: &Arc<Publication>) {
        let weak = Arc::downgrade(self);
        let room = Arc::downgrade(room);
        let publication = Arc::downgrade(pubn);
        let runtime = tokio::runtime::Handle::current();
        pubn.producer.resource.with(|producer| {
            producer
                .on_close(move || {
                    if let Some(p) = publication.upgrade() {
                        p.grant.stop()
                    }
                    runtime.spawn(async move {
                        if let (Some(sfu), Some(room), Some(p)) =
                            (weak.upgrade(), room.upgrade(), publication.upgrade())
                        {
                            sfu.retire_publication(&room, &p).await;
                        }
                    });
                })
                .detach();
        });
    }
    pub(super) fn observe_consumer(self: &Arc<Self>, room: &Arc<Room>, sub: &Arc<Subscription>) {
        for paused in [true, false] {
            let weak = Arc::downgrade(self);
            let subweak = Arc::downgrade(sub);
            let runtime = tokio::runtime::Handle::current();
            let callback = move || {
                let weak = weak.clone();
                let subweak = subweak.clone();
                runtime.spawn(async move {
                    if let (Some(sfu), Some(s)) = (weak.upgrade(), subweak.upgrade())
                        && s.grant.valid()
                    {
                        sfu.emit(
                            &s.peer,
                            ServerFrame::ConsumerState {
                                consumer_id: s.consumer.id().to_string(),
                                generation: s.generation,
                                paused: s.consumer.paused() || s.consumer.producer_paused(),
                            },
                        );
                    }
                });
            };
            if paused {
                sub.consumer
                    .resource
                    .with(|c| c.on_producer_pause(callback).detach());
            } else {
                sub.consumer
                    .resource
                    .with(|c| c.on_producer_resume(callback).detach());
            }
        }
        let weak = Arc::downgrade(self);
        let subweak = Arc::downgrade(sub);
        let runtime = tokio::runtime::Handle::current();
        sub.consumer.resource.with(|consumer| {
            consumer
                .on_layers_change(move |layers| {
                    let weak = weak.clone();
                    let subweak = subweak.clone();
                    let layers = *layers;
                    runtime.spawn(async move {
                        if let (Some(sfu), Some(s)) = (weak.upgrade(), subweak.upgrade())
                            && s.valid()
                        {
                            sfu.emit(
                                &s.peer,
                                ServerFrame::Layers {
                                    consumer_id: s.consumer.id().to_string(),
                                    generation: s.generation,
                                    spatial_layer: layers.map(|l| l.spatial_layer),
                                    temporal_layer: layers.and_then(|l| l.temporal_layer),
                                },
                            );
                        }
                    });
                })
                .detach();
        });
        let weak = Arc::downgrade(self);
        let subweak = Arc::downgrade(sub);
        let room = Arc::downgrade(room);
        let runtime = tokio::runtime::Handle::current();
        sub.consumer.resource.with(|consumer| {
            consumer
                .on_close(move || {
                    if let Some(s) = subweak.upgrade() {
                        s.grant.stop()
                    }
                    runtime.spawn(async move {
                        if let (Some(sfu), Some(room), Some(s)) =
                            (weak.upgrade(), room.upgrade(), subweak.upgrade())
                        {
                            sfu.remove_consumer(&room, &s).await;
                        }
                    });
                })
                .detach();
        });
    }
    pub(super) fn watch_stats(weak: Weak<Self>) {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(5));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut cursor = 0usize;
            loop {
                interval.tick().await;
                let Some(sfu) = weak.upgrade() else { break };
                let rooms: Vec<_> = sfu.rooms.lock().await.values().cloned().collect();
                let mut transports = Vec::new();
                let mut publications = Vec::new();
                for room in rooms {
                    let data = room.data.lock().await;
                    transports.extend(
                        data.peers
                            .values()
                            .flat_map(|p| p.transports.values())
                            .cloned(),
                    );
                    publications.extend(data.publications.values().cloned());
                }
                // At most 32 transport + 32 producer samples per five seconds.
                // Rotate stable native IDs so large rooms do not starve later IDs.
                transports.sort_by_key(|t| t.id().to_string());
                publications.sort_by_key(|p| p.id());
                if !transports.is_empty() {
                    let n = transports.len();
                    transports.rotate_left(cursor % n);
                }
                if !publications.is_empty() {
                    let n = publications.len();
                    publications.rotate_left(cursor % n);
                }
                cursor = cursor.wrapping_add(32);
                let mut ok = false;
                for transport in transports.into_iter().take(32) {
                    ok |= sfu.sample_transport(&transport).await;
                }
                sfu.counters.stats_available.store(ok, Ordering::Relaxed);
                let mut input_available = false;
                let mut jitter_microseconds = 0;
                for p in publications.into_iter().take(32) {
                    if let Some(jitter) = sfu.sample_publication(&p, STOP_TIMEOUT).await {
                        input_available = true;
                        jitter_microseconds = jitter_microseconds.max(jitter);
                    }
                }
                sfu.counters
                    .jitter_microseconds
                    .store(jitter_microseconds, Ordering::Relaxed);
                sfu.counters
                    .input_available
                    .store(input_available, Ordering::Relaxed);
            }
        });
    }
    pub(super) async fn sample_publication(
        &self,
        p: &Publication,
        budget: Duration,
    ) -> Option<u64> {
        let Ok(Ok(stats)) = tokio::time::timeout(budget, p.producer.get_stats()).await else {
            return None;
        };
        if stats.is_empty() {
            return None;
        }
        let mut jitter_microseconds = 0;
        for stat in stats {
            let mut samples = p.packet_samples.lock().unwrap();
            let old = samples.entry(stat.ssrc).or_default();
            self.counters
                .rtp_received
                .fetch_add(stat.packet_count.saturating_sub(*old), Ordering::Relaxed);
            *old = (*old).max(stat.packet_count);
            let rate = if stat.kind == MediaKind::Audio {
                48000
            } else {
                90000
            };
            jitter_microseconds =
                jitter_microseconds.max(u64::from(stat.jitter) * 1_000_000 / rate);
        }
        Some(jitter_microseconds)
    }
    pub(super) async fn sample_transport(&self, transport: &WebRtcTransport) -> bool {
        let Ok(Ok(stats)) = tokio::time::timeout(STOP_TIMEOUT, transport.get_stats()).await else {
            return false;
        };
        let available = !stats.is_empty();
        for stat in stats {
            let sample = transport
                .app_data()
                .downcast_ref::<AtomicU64>()
                .expect("gateway transport measurement");
            let old = sample.fetch_max(stat.rtp_bytes_sent, Ordering::SeqCst);
            self.counters
                .rtp_sent
                .fetch_add(stat.rtp_bytes_sent.saturating_sub(old), Ordering::Relaxed);
        }
        available
    }
}
