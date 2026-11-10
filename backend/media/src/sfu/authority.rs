//! A joined peer's ticket authority, checked again every second.
use super::*;

impl Sfu {
    /// `Revoked` only when Redis answered "no". A check that could not be
    /// made is `Gone`, so the client rejoins with a new ticket.
    pub(super) async fn authorize(
        &self,
        authority: &AuthorizedTicketClaim,
    ) -> Result<(), SfuError> {
        match crate::ticket::validate_authority(&self.redis, authority).await {
            Ok(true) => Ok(()),
            Ok(false) => Err(SfuError::Revoked),
            Err(_) => Err(SfuError::Gone),
        }
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
                // A draining or engine-less pool is not a revocation either.
                let verdict = if sfu.ready() {
                    sfu.authorize(&peer.authority).await
                } else {
                    Err(SfuError::Gone)
                };
                if let Err(error) = verdict {
                    sfu.leave(peer.id, channel).await;
                    sfu.emit(&peer, ServerFrame::error(error.code()));
                    break;
                }
            }
        });
    }
}
