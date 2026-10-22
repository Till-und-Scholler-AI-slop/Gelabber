//! Live publications bound to a Redis lease: renewal while the peer's
//! authority holds, and the stop before the lease runs out.
use super::*;

impl Sfu {
    pub(super) async fn release_unpublished_live(
        self: &Arc<Self>,
        room: &Arc<Room>,
        peer: &Arc<Peer>,
        nonce: Uuid,
    ) {
        let current = room
            .data
            .lock()
            .await
            .peers
            .get(&peer.id)
            .and_then(|p| p.publications.get(&SourceKind::Live))
            .filter(|p| p.live.as_ref().is_some_and(|l| l.nonce == nonce))
            .cloned();
        if let Some(current) = current {
            if current.valid() {
                return;
            }
            // A grant may have expired while native creation was in flight.
            // Confirm the old source is stopped before releasing its lease.
            self.retire_publication(room, &current).await;
        }
        crate::live::release(&self.redis, nonce, peer.id.0).await;
    }
    pub(super) fn watch_authority(weak: Weak<Self>, channel: Uuid, peer: Weak<Peer>) {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                let (Some(sfu), Some(peer)) = (weak.upgrade(), peer.upgrade()) else {
                    break;
                };
                if !peer.grant.valid() {
                    break;
                }
                if !sfu.ready()
                    || !crate::ticket::validate_authority(&sfu.redis, &peer.authority)
                        .await
                        .unwrap_or(false)
                {
                    sfu.leave(peer.id, channel).await;
                    sfu.emit(&peer, ServerFrame::error("unauthorized"));
                    break;
                }
                let Ok(room) = sfu.room(channel).await else {
                    break;
                };
                let pubn = room
                    .data
                    .lock()
                    .await
                    .peers
                    .get(&peer.id)
                    .and_then(|p| p.publications.get(&SourceKind::Live))
                    .cloned();
                if let Some(pubn) = pubn {
                    let _lease = peer.lease_gate.lock().await;
                    if !pubn.grant.valid() {
                        continue;
                    }
                    let Some(live) = &pubn.live else { continue };
                    let checked = Instant::now();
                    if let Some(ttl) = crate::live::validate_and_acquire(
                        &sfu.redis,
                        &peer.authority,
                        live.nonce,
                        peer.id.0,
                    )
                    .await
                    .unwrap_or(None)
                        && pubn.grant.valid()
                        && checked + ttl > Instant::now() + LIVE_STOP_MARGIN
                    {
                        *live.deadline.lock().unwrap() = checked + ttl;
                        live.changed.notify_one();
                        continue;
                    }
                    drop(_lease);
                    sfu.remove_publication(&room, &pubn, true).await;
                }
            }
        });
    }
    pub(super) fn watch_live(weak: Weak<Self>, room: Weak<Room>, pubn: Weak<Publication>) {
        tokio::spawn(async move {
            loop {
                let Some(p) = pubn.upgrade() else { break };
                if !p.grant.valid() {
                    break;
                }
                let Some(live) = p.live.clone() else { break };
                let deadline = *live.deadline.lock().unwrap();
                let stop_at = deadline.checked_sub(LIVE_STOP_MARGIN).unwrap_or(deadline);
                drop(p);
                tokio::select! {_=tokio::time::sleep_until(stop_at.into())=>{},_=live.changed.notified()=>continue}
                let (Some(sfu), Some(room), Some(p)) =
                    (weak.upgrade(), room.upgrade(), pubn.upgrade())
                else {
                    break;
                };
                if Instant::now() + LIVE_STOP_MARGIN < *live.deadline.lock().unwrap() {
                    continue;
                }
                sfu.remove_publication(&room, &p, true).await;
                break;
            }
        });
    }
}
