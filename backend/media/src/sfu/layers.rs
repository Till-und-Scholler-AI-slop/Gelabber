//! GEL-460: the watcher of a Live that has just started waits for a layer.
//!
//! mediasoup gives a simulcast consumer its layer in a bitrate round of the
//! viewer's transport, and a round offers only the layers whose bitrate it can
//! measure. A layer that has sent a single packet has none yet. So a consumer
//! that is resumed, or whose producer's layers start, while they are that new
//! gets no layer in that round, and the next round waits for some other event:
//! a layer's first RTCP sender report, a change of the viewer's bandwidth
//! estimate. In Extended E2E that took up to 3.7 s while the publisher encoded
//! every frame (GEL-460). Until a resumed consumer has a layer, the SFU asks for
//! a round itself, every `NUDGE_EVERY`, for `NUDGE_ROUNDS` rounds at most.
use super::*;
use mediasoup::consumer::ConsumerType;

pub(super) const NUDGE_EVERY: Duration = Duration::from_millis(250);
/// Ten seconds: longer than any start seen; a Live paused meanwhile stops it.
pub(super) const NUDGE_ROUNDS: u32 = 40;

/// What waiting for a consumer's layer needs of it.
pub(super) trait AwaitsLayer {
    /// It forwards a layer, or cannot get one now: closed, paused, its
    /// producer paused, or no layers at all (one stream, audio).
    fn settled(&self) -> bool;
    /// Asks mediasoup for a bitrate round of the consumer's transport.
    fn nudge(&self) -> impl Future<Output = Result<(), SfuError>> + Send;
}

/// Asks for a round after each `tick` until the consumer is settled, at most
/// `rounds` times, and stops at the first refused request. Returns the rounds
/// asked for.
pub(super) async fn nudge_until_layer<F: Future<Output = ()>>(
    consumer: &impl AwaitsLayer,
    rounds: u32,
    mut tick: impl FnMut() -> F,
) -> u32 {
    for asked in 0..rounds {
        tick().await;
        if consumer.settled() || consumer.nudge().await.is_err() {
            return asked;
        }
    }
    rounds
}

impl AwaitsLayer for NativeConsumer {
    fn settled(&self) -> bool {
        self.closed()
            || self.paused()
            || self.producer_paused()
            || self
                .resource
                .with(|c| {
                    !matches!(c.r#type(), ConsumerType::Simulcast | ConsumerType::Svc)
                        || c.current_layers().is_some()
                })
                .unwrap_or(true)
    }
    async fn nudge(&self) -> Result<(), SfuError> {
        // A changed priority is the API's way to ask for a round; the default
        // priority 1 is restored at once, which asks for a second one.
        self.set_priority(2).await?;
        self.set_priority(1).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;

    /// Gets its layer once it was nudged `after` times; `refuse` makes the
    /// nudges fail, as on a consumer closed meanwhile.
    struct Fake {
        after: u32,
        nudges: AtomicU32,
        refuse: bool,
    }
    impl AwaitsLayer for Fake {
        fn settled(&self) -> bool {
            self.nudges.load(Ordering::SeqCst) >= self.after
        }
        async fn nudge(&self) -> Result<(), SfuError> {
            if self.refuse {
                return Err(SfuError::Unavailable);
            }
            self.nudges.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }
    fn fake(after: u32, refuse: bool) -> Fake {
        Fake {
            after,
            nudges: AtomicU32::new(0),
            refuse,
        }
    }

    #[tokio::test]
    async fn a_consumer_without_a_layer_is_nudged_until_it_has_one() {
        let consumer = fake(3, false);
        assert_eq!(
            nudge_until_layer(&consumer, NUDGE_ROUNDS, || async {}).await,
            3
        );
        assert_eq!(consumer.nudges.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn a_consumer_that_has_its_layer_is_left_alone() {
        let consumer = fake(0, false);
        assert_eq!(
            nudge_until_layer(&consumer, NUDGE_ROUNDS, || async {}).await,
            0
        );
        assert_eq!(consumer.nudges.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn nudging_ends_after_its_rounds_or_at_a_refused_request() {
        let never = fake(u32::MAX, false);
        assert_eq!(nudge_until_layer(&never, 5, || async {}).await, 5);
        assert_eq!(never.nudges.load(Ordering::SeqCst), 5);
        let closed = fake(u32::MAX, true);
        assert_eq!(nudge_until_layer(&closed, 5, || async {}).await, 0);
    }
}
