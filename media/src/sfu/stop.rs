//! Confirmed native stop of consumers and producers before they leave the graph.
use super::*;

impl Sfu {
    pub(super) async fn confirmed_stop_resources(
        &self,
        publications: &[Arc<Publication>],
        subscriptions: &[Arc<Subscription>],
    ) {
        // Fence every resource before starting any native operation. The single
        // deadline includes queued work, gate acquisition and every pause ACK.
        for publication in publications {
            publication.grant.stop();
        }
        for subscription in subscriptions {
            subscription.grant.stop();
        }
        let resources: Vec<_> = publications
            .iter()
            .cloned()
            .map(StopResource::Publication)
            .chain(
                subscriptions
                    .iter()
                    .cloned()
                    .map(StopResource::Subscription),
            )
            .collect();
        let jobs = resources.clone().into_iter().map(|resource| async move {
            match resource {
                StopResource::Publication(p) => {
                    let _gate = stop_gate(&p.gate).await;
                    if _gate.is_some() {
                        stop_producer(&p.producer).await;
                    } else {
                        p.producer.confirm_closed().await;
                    }
                }
                StopResource::Subscription(s) => {
                    let _gate = stop_gate(&s.gate).await;
                    if _gate.is_some() {
                        stop_consumer(&s.consumer).await;
                    } else {
                        s.consumer.confirm_closed().await;
                    }
                }
            }
        });
        if stop_batch(jobs).await.is_err() {
            // Close only this batch, including resources still waiting for a
            // concurrency slot. No unrelated room loses its worker/transport.
            for resource in &resources {
                match resource {
                    StopResource::Publication(p) => p.producer.resource.close(),
                    StopResource::Subscription(s) => s.consumer.resource.close(),
                }
            }
            stream::iter(resources)
                .for_each_concurrent(Some(STOP_CONCURRENCY), |resource| async move {
                    match resource {
                        StopResource::Publication(p) => p.producer.confirm_closed().await,
                        StopResource::Subscription(s) => s.consumer.confirm_closed().await,
                    }
                })
                .await;
        }
    }
    pub(super) async fn remove_consumer(self: &Arc<Self>, room: &Room, sub: &Arc<Subscription>) {
        self.confirmed_stop_resources(&[], std::slice::from_ref(sub))
            .await;
        self.forget_consumer(room, sub).await;
    }
    pub(super) async fn forget_consumer(self: &Arc<Self>, room: &Room, sub: &Arc<Subscription>) {
        let mut data = room.data.lock().await;
        if let Some(p) = data.peers.get_mut(&sub.peer.id)
            && p.consumers
                .get(&sub.consumer.id().to_string())
                .is_some_and(|s| Arc::ptr_eq(s, sub))
        {
            p.consumers.remove(&sub.consumer.id().to_string());
            self.counters.consumers.fetch_sub(1, Ordering::Relaxed);
            self.emit(
                &sub.peer,
                ServerFrame::ConsumerClosed {
                    consumer_id: sub.consumer.id().to_string(),
                    generation: sub.generation,
                },
            );
        }
    }
    pub(super) async fn retire_publication(
        self: &Arc<Self>,
        room: &Arc<Room>,
        pubn: &Arc<Publication>,
    ) {
        let (pubs, subs) = {
            let data = room.data.lock().await;
            let mut pubs = vec![pubn.clone()];
            pubs.extend(
                data.publications
                    .values()
                    .filter(|p| {
                        !Arc::ptr_eq(p, pubn)
                            && p.parent
                                .as_ref()
                                .is_some_and(|parent| Arc::ptr_eq(parent, pubn))
                    })
                    .cloned(),
            );
            for p in &pubs {
                p.grant.stop()
            }
            let subs: Vec<_> = data
                .peers
                .values()
                .flat_map(|p| p.consumers.values())
                .filter(|s| !s.valid())
                .cloned()
                .collect();
            for s in &subs {
                s.grant.stop()
            }
            (pubs, subs)
        };
        self.confirmed_stop_resources(&pubs, &subs).await;
        // Registry removal, UI events and retired-ID ACKs follow the confirmed
        // stop of both owned sources and every affected room subscription.
        let registered = {
            let mut data = room.data.lock().await;
            let mut registered = 0;
            for p in &pubs {
                if data.publications.remove(&p.id()).is_some() {
                    registered += 1;
                }
                if let Some(peer) = data.peers.get_mut(&p.peer.id)
                    && peer
                        .publications
                        .get(&p.kind)
                        .is_some_and(|current| Arc::ptr_eq(current, p))
                {
                    peer.publications.remove(&p.kind);
                }
            }
            registered
        };
        // An idempotent close ACK for an old ID is only legal after its native
        // pause has been confirmed, never merely after graph invalidation.
        // Optional telemetry follows stop; its pre-lease-release budget is 10ms.
        join_all(
            pubs.iter()
                .map(|p| self.sample_publication(p, Duration::from_millis(10))),
        )
        .await;
        {
            let mut data = room.data.lock().await;
            for p in &pubs {
                if let Some(peer) = data.peers.get_mut(&p.peer.id) {
                    remember_retired(&mut peer.retired_producers, p.id());
                }
            }
        }
        for sub in subs {
            self.forget_consumer(room, &sub).await;
        }
        self.counters
            .producers
            .fetch_sub(registered, Ordering::Relaxed);
        for p in pubs {
            self.emit(
                &p.peer,
                ServerFrame::ProducerClosed {
                    producer_id: p.id(),
                    epoch: p.epoch,
                },
            );
        }
    }
}

pub(super) fn remember_retired(ids: &mut VecDeque<String>, id: String) {
    if ids.contains(&id) {
        return;
    }
    if ids.len() == 256 {
        ids.pop_front();
    }
    ids.push_back(id);
}
pub(super) async fn stop_gate(gate: &Mutex<()>) -> Option<tokio::sync::MutexGuard<'_, ()>> {
    tokio::time::timeout(STOP_TIMEOUT, gate.lock()).await.ok()
}
pub(super) async fn stop_batch<F: Future<Output = ()>>(
    jobs: impl IntoIterator<Item = F>,
) -> Result<(), tokio::time::error::Elapsed> {
    tokio::time::timeout(
        STOP_TIMEOUT,
        stream::iter(jobs).for_each_concurrent(Some(STOP_CONCURRENCY), |job| async move {
            // Keep the deadline observable even when many already-stopped
            // resources can finish without a native await.
            tokio::task::yield_now().await;
            job.await;
        }),
    )
    .await
}
pub(super) async fn stop_producer(p: &NativeProducer) {
    if p.worker_lost() || p.closed() || p.paused() {
        return;
    }
    match tokio::time::timeout(STOP_TIMEOUT, p.pause()).await {
        Ok(Ok(())) if p.paused() => {}
        _ => p.confirm_closed().await,
    }
}
pub(super) async fn stop_consumer(c: &NativeConsumer) {
    if c.worker_lost() || c.closed() || c.paused() {
        return;
    }
    match tokio::time::timeout(STOP_TIMEOUT, c.pause()).await {
        Ok(Ok(())) if c.paused() => {}
        _ => c.confirm_closed().await,
    }
}
