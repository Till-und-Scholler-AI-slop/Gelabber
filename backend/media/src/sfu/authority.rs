//! A joined peer's ticket authority, checked again every second.
use super::*;

impl Sfu {
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
            }
        });
    }
}
