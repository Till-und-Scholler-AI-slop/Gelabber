//! GEL-460: a watcher's consumer waits for its layer while a Live starts.
//!
//! mediasoup (0.33.2) gives a simulcast or SVC consumer under bandwidth
//! estimation its layer in a bitrate round of the viewer's transport
//! (`Consumer.cpp`, `SimulcastProducerStreamManager.cpp`,
//! `ProducerStreamManager.cpp`, `BWE/BitrateAllocator.cpp`). A round offers a
//! layer only if its bitrate can be measured, and the rate calculator gives no
//! rate for a single sample (`RateCalculator::GetRate`).
//!
//! A consumer resumed before its producer's first packets gets no round at
//! its resume: there are no producer streams yet. The round that each layer's
//! first packet starts (`Producer::ReceiveRtpPacket` counts the packet, then
//! `ProducerNewRtpStream` → `MayChangeLayers`) sees exactly that one packet,
//! finds no option but "send nothing" and skips the consumer. Nothing asks
//! again until another event: a layer's first RTCP sender report, a change of
//! the viewer's estimate. So it happens every time a consumer is resumed
//! before its producer's first packets. In Extended E2E the watcher of a
//! retried Live got its first layer report 3.6 s after its consumer was ready
//! (run 46084585355), and none within the 5 s of the failed runs, while the
//! publisher encoded every frame.
//!
//! Read from the source, not measured: the same round over one sample starts
//! when a layer's packets come back after more than 3.75 s of silence (1.5
//! times the 2.5 s rate window, `RateCalculator::Update` with
//! `RecentSampleMarginFactor`; before that the old period stays and one
//! sample gives a rate), since the stream's score reset (`RtpStreamRecv.cpp`)
//! asks for it right after counting that packet. A producer that is resumed
//! can be such a case, but no current client pauses a video producer: the web
//! client pauses only its microphone, the desktop app sends no pause for video,
//! and `produce` resumes its paused producer before the room attach. The
//! resume path is there for that window and for a later client.
//!
//! So the SFU asks for rounds itself, after `consumerReady` resumes a layered
//! consumer and after its producer resumes: every `NUDGE_EVERY` until the
//! consumer has a layer, `NUDGE_ROUNDS` rounds at most.
use super::*;
use mediasoup::consumer::ConsumerType;
use tracing::debug;

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

/// What ended a wait for a layer.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(super) enum WaitEnd {
    Settled,
    /// A nudge was refused: command slots full, its deadline, a closed consumer.
    Refused,
    /// Every round was asked for, without a layer.
    Exhausted,
}

/// Asks for a round after each `tick` until the consumer is settled, at most
/// `rounds` times, and stops at the first refused request. Returns why it
/// ended and the rounds asked for.
pub(super) async fn nudge_until_layer<F: Future<Output = ()>>(
    consumer: &impl AwaitsLayer,
    rounds: u32,
    tick: &mut impl FnMut() -> F,
) -> (WaitEnd, u32) {
    for asked in 0..rounds {
        tick().await;
        if consumer.settled() {
            return (WaitEnd::Settled, asked);
        }
        if consumer.nudge().await.is_err() {
            return (WaitEnd::Refused, asked);
        }
    }
    (WaitEnd::Exhausted, rounds)
}

/// Waits with `nudge_until_layer`, then `release`s the wait, which says
/// whether to wait once more: a wait that ended because the consumer looked
/// settled (its producer paused) waits again if it is no longer settled, since
/// a resume between that look and the release found the wait still running and
/// started none.
pub(super) async fn wait_for_layer<F: Future<Output = ()>>(
    consumer: &impl AwaitsLayer,
    rounds: u32,
    mut tick: impl FnMut() -> F,
    release: impl Fn(WaitEnd) -> bool,
) -> (WaitEnd, u32) {
    let mut asked = 0;
    loop {
        let (end, more) = nudge_until_layer(consumer, rounds, &mut tick).await;
        asked += more;
        if !release(end) {
            return (end, asked);
        }
    }
}

impl NativeConsumer {
    /// Whether it forwards layers at all: simulcast or SVC video.
    pub(super) fn layered(&self) -> bool {
        self.resource
            .with(|c| matches!(c.r#type(), ConsumerType::Simulcast | ConsumerType::Svc))
            .unwrap_or(false)
    }
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
                // Its native handle is gone: there is nothing left to ask.
                .unwrap_or(true)
    }
    async fn nudge(&self) -> Result<(), SfuError> {
        // A changed priority is the API's way to ask for a round, and putting
        // the previous one back asks for a second. While the raised one holds,
        // its round sorts this consumer first and gives it twice the share of
        // the bitrate left over; another video consumer that loses a step
        // there climbs back only with mediasoup's 1.2 hysteresis. On a viewer
        // link with too little bandwidth, one nudge can so leave another video
        // one step lower until the estimate rises about 20 %.
        let previous = self.priority().ok_or(SfuError::Unavailable)?;
        let raised = previous.checked_add(1).unwrap_or(previous - 1);
        self.set_priority(raised).await?;
        self.set_priority(previous).await
    }
}

/// A subscription waiting for its layer, held weakly: a closed consumer's
/// native handle goes with its last strong owner, not with this task.
struct Waiting {
    sub: Weak<Subscription>,
    engine: Arc<Engine>,
}
impl AwaitsLayer for Waiting {
    fn settled(&self) -> bool {
        self.sub.upgrade().is_none_or(|s| s.consumer.settled())
    }
    async fn nudge(&self) -> Result<(), SfuError> {
        let s = self.sub.upgrade().ok_or(SfuError::Unavailable)?;
        // Through the engine's command slots and deadline, as the resume.
        self.engine
            .native(s.clone(), |s| async move { s.consumer.nudge().await })
            .await?;
        s.layer_nudges.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

impl Sfu {
    /// Asks for bitrate rounds until `sub`'s consumer has a layer; one task
    /// per consumer at a time, none for a consumer without layers.
    pub(super) fn await_layer(&self, engine: &Arc<Engine>, sub: &Arc<Subscription>) {
        if !sub.consumer.layered() || sub.awaiting_layer.swap(true, Ordering::SeqCst) {
            return;
        }
        let waiting = Waiting {
            sub: Arc::downgrade(sub),
            engine: engine.clone(),
        };
        let every = Duration::from_millis(self.nudge_every_ms.load(Ordering::Relaxed));
        let channel = sub.publication.peer.authority.claim.c;
        let consumer = sub.consumer.id();
        tokio::spawn(async move {
            let (end, rounds) = wait_for_layer(
                &waiting,
                NUDGE_ROUNDS,
                || tokio::time::sleep(every),
                |end| match waiting.sub.upgrade() {
                    Some(s) => {
                        s.awaiting_layer.store(false, Ordering::SeqCst);
                        end == WaitEnd::Settled
                            && !s.consumer.settled()
                            && !s.awaiting_layer.swap(true, Ordering::SeqCst)
                    }
                    None => false,
                },
            )
            .await;
            let unsettled = waiting.sub.upgrade().is_some() && !waiting.settled();
            match end {
                WaitEnd::Exhausted if unsettled => warn!(
                    %channel,
                    %consumer,
                    rounds,
                    "consumer has no layer after the SFU asked for bitrate rounds"
                ),
                WaitEnd::Refused if unsettled => debug!(
                    %channel,
                    %consumer,
                    rounds,
                    "consumer's wait for a layer ended on a refused request"
                ),
                _ => {}
            }
            if let Some(s) = waiting.sub.upgrade() {
                s.layer_wait_ended.notify_one();
            }
        });
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
            nudge_until_layer(&consumer, NUDGE_ROUNDS, &mut || async {}).await,
            (WaitEnd::Settled, 3)
        );
        assert_eq!(consumer.nudges.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn a_consumer_that_has_its_layer_is_left_alone() {
        let consumer = fake(0, false);
        assert_eq!(
            nudge_until_layer(&consumer, NUDGE_ROUNDS, &mut || async {}).await,
            (WaitEnd::Settled, 0)
        );
        assert_eq!(consumer.nudges.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn nudging_ends_after_its_rounds_or_at_a_refused_request() {
        let never = fake(u32::MAX, false);
        assert_eq!(
            nudge_until_layer(&never, 5, &mut || async {}).await,
            (WaitEnd::Exhausted, 5)
        );
        assert_eq!(never.nudges.load(Ordering::SeqCst), 5);
        let closed = fake(u32::MAX, true);
        assert_eq!(
            nudge_until_layer(&closed, 5, &mut || async {}).await,
            (WaitEnd::Refused, 0)
        );
    }

    /// `settled()` answers from a script, then true.
    struct Scripted {
        answers: StdMutex<VecDeque<bool>>,
        nudges: AtomicU32,
    }
    impl AwaitsLayer for Scripted {
        fn settled(&self) -> bool {
            self.answers.lock().unwrap().pop_front().unwrap_or(true)
        }
        async fn nudge(&self) -> Result<(), SfuError> {
            self.nudges.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[tokio::test]
    async fn a_producer_resumed_as_the_wait_ends_starts_it_again() {
        // Settled at the first look (its producer paused), unsettled at the
        // release (resumed meanwhile), then one round until its layer.
        let consumer = Scripted {
            answers: StdMutex::new([true, false, false, true].into()),
            nudges: AtomicU32::new(0),
        };
        let flag = AtomicBool::new(true);
        let release = |end| {
            flag.store(false, Ordering::SeqCst);
            end == WaitEnd::Settled && !consumer.settled() && !flag.swap(true, Ordering::SeqCst)
        };
        assert_eq!(
            wait_for_layer(&consumer, NUDGE_ROUNDS, || async {}, release).await,
            (WaitEnd::Settled, 1)
        );
        assert_eq!(consumer.nudges.load(Ordering::SeqCst), 1);
        assert!(!flag.load(Ordering::SeqCst));
    }
}
