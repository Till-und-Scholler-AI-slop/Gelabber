//! Explicit, cancellable ownership of SDK resources. The Rust SDK closes on
//! last drop; retaining an Arc<Publication> must not retain native forwarding.
use crate::error::SfuError;
use mediasoup::prelude::*;
use std::{
    future::Future,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{sync::watch, time::Instant};
use tracing::{error, warn};

/// Longer than the worker heartbeat's 5s, so a stalled worker fails there first.
const CONFIRM_DEADLINE: Duration = Duration::from_secs(10);
const DUMP_TIMEOUT: Duration = Duration::from_millis(500);
/// The first poll keeps the common case as fast as before; later polls back
/// off so a slow removal does not flood the worker channel with dumps.
const FIRST_POLL: Duration = Duration::from_millis(10);
const MAX_POLL: Duration = Duration::from_millis(250);

#[derive(Debug)]
struct Unconfirmed {
    polls: u32,
    unanswered: u32,
}

/// Waits until `closed` holds or `listed` reports the resource gone. Only a
/// worker that answers and still lists the resource past `deadline` is an
/// unconfirmed close. An unanswered dump (`None`) never ends the wait: that
/// is worker liveness, which the heartbeat owns.
async fn await_native_removal<L, Fut>(
    closed: impl Fn() -> bool,
    mut listed: L,
    deadline: Duration,
) -> Result<(), Unconfirmed>
where
    L: FnMut() -> Fut,
    Fut: Future<Output = Option<bool>>,
{
    let started = Instant::now();
    let mut delay = FIRST_POLL;
    let mut polls = 0;
    let mut unanswered = 0;
    let mut warned = false;
    loop {
        if closed() {
            return Ok(());
        }
        polls += 1;
        match listed().await {
            Some(false) => return Ok(()),
            Some(true) if started.elapsed() >= deadline => {
                return Err(Unconfirmed { polls, unanswered });
            }
            Some(true) => {}
            None => {
                unanswered += 1;
                if !warned && started.elapsed() >= deadline {
                    warned = true;
                    warn!(unanswered, "native close unconfirmed: dump unanswered");
                }
            }
        }
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(MAX_POLL);
    }
}

/// Returning would let terminal events and the Live lease release run while
/// the worker may still forward the resource, so the service stops instead.
fn unconfirmed_close(id: impl std::fmt::Display, failure: Unconfirmed) -> ! {
    error!(
        %id,
        polls = failure.polls,
        unanswered = failure.unanswered,
        "native close unconfirmed: worker still lists the resource"
    );
    crate::sfu::fail_closed("native resource close unconfirmed")
}

pub struct Resource<T> {
    value: Mutex<Option<T>>,
    closed: watch::Sender<bool>,
}
impl<T: Clone> Resource<T> {
    fn new(value: T) -> Self {
        Self {
            value: Mutex::new(Some(value)),
            closed: watch::channel(false).0,
        }
    }
    pub fn with<R>(&self, action: impl FnOnce(&T) -> R) -> Option<R> {
        self.value.lock().unwrap().as_ref().map(action)
    }
    async fn run<R, E: std::fmt::Display, F: Future<Output = Result<R, E>>>(
        &self,
        action: impl FnOnce(T) -> F,
    ) -> Result<R, SfuError> {
        let mut closed = self.closed.subscribe();
        // A subscriber created between close's notification and take must not
        // treat the already-seen true value as permission to start a new RPC.
        if *closed.borrow() {
            return Err(SfuError::Unavailable);
        }
        let value = self.with(Clone::clone).ok_or(SfuError::Unavailable)?;
        tokio::select! {
            biased;
            _ = closed.changed() => Err(SfuError::Unavailable),
            result = action(value) => result.map_err(SfuError::negotiation),
        }
    }
    pub fn close(&self) {
        // Notify before dropping: cancellation releases clones held by awaits.
        self.closed.send_replace(true);
        let value = self.value.lock().unwrap().take();
        drop(value);
    }
}

pub struct NativeProducer {
    pub resource: Resource<Producer>,
    id: ProducerId,
    confirmed: AtomicBool,
    transport: WebRtcTransport,
    /// Set once the owning worker died or hung. Its thread forwards nothing
    /// more, and a hung one would never answer the confirmation dump.
    worker_lost: Arc<AtomicBool>,
}
impl NativeProducer {
    pub fn new(
        producer: Producer,
        transport: WebRtcTransport,
        worker_lost: Arc<AtomicBool>,
    ) -> Self {
        Self {
            id: producer.id(),
            confirmed: AtomicBool::new(false),
            resource: Resource::new(producer),
            transport,
            worker_lost,
        }
    }
    pub fn worker_lost(&self) -> bool {
        self.worker_lost.load(Ordering::SeqCst)
    }
    pub fn id(&self) -> ProducerId {
        self.id
    }
    pub fn transport(&self) -> &WebRtcTransport {
        &self.transport
    }
    pub fn paused(&self) -> bool {
        self.resource
            .with(Producer::paused)
            .unwrap_or_else(|| self.confirmed.load(Ordering::SeqCst))
    }
    pub fn closed(&self) -> bool {
        self.resource
            .with(Producer::closed)
            .unwrap_or_else(|| self.confirmed.load(Ordering::SeqCst))
    }
    pub async fn pause(&self) -> Result<(), SfuError> {
        self.resource.run(|p| async move { p.pause().await }).await
    }
    pub async fn resume(&self) -> Result<(), SfuError> {
        self.resource.run(|p| async move { p.resume().await }).await
    }
    pub async fn get_stats(&self) -> Result<Vec<mediasoup::producer::ProducerStat>, SfuError> {
        self.resource
            .run(|p| async move { p.get_stats().await })
            .await
    }
    pub async fn confirm_closed(&self) {
        self.resource.close();
        // A local drop only enqueues the native close. Confirm removal before
        // terminal events or lease release; the worker heartbeat guards stalls.
        let (id, transport, lost) = (self.id, &self.transport, &self.worker_lost);
        let removed = await_native_removal(
            || transport.closed() || lost.load(Ordering::SeqCst),
            move || async move {
                tokio::time::timeout(DUMP_TIMEOUT, transport.dump())
                    .await
                    .ok()
                    .and_then(Result::ok)
                    .map(|dump| dump.producer_ids.contains(&id))
            },
            CONFIRM_DEADLINE,
        )
        .await;
        if let Err(failure) = removed {
            unconfirmed_close(id, failure);
        }
        self.confirmed.store(true, Ordering::SeqCst);
    }
}

pub struct NativeConsumer {
    pub resource: Resource<Consumer>,
    id: ConsumerId,
    confirmed: AtomicBool,
    transport: WebRtcTransport,
    /// Set once the owning worker died or hung. Its thread forwards nothing
    /// more, and a hung one would never answer the confirmation dump.
    worker_lost: Arc<AtomicBool>,
    rtp: RtpParameters,
}
impl NativeConsumer {
    pub fn new(
        consumer: Consumer,
        transport: WebRtcTransport,
        worker_lost: Arc<AtomicBool>,
    ) -> Self {
        Self {
            id: consumer.id(),
            confirmed: AtomicBool::new(false),
            rtp: consumer.rtp_parameters().clone(),
            resource: Resource::new(consumer),
            transport,
            worker_lost,
        }
    }
    pub fn worker_lost(&self) -> bool {
        self.worker_lost.load(Ordering::SeqCst)
    }
    pub fn id(&self) -> ConsumerId {
        self.id
    }
    pub fn transport(&self) -> &WebRtcTransport {
        &self.transport
    }
    pub fn rtp_parameters(&self) -> &RtpParameters {
        &self.rtp
    }
    pub fn paused(&self) -> bool {
        self.resource
            .with(Consumer::paused)
            .unwrap_or_else(|| self.confirmed.load(Ordering::SeqCst))
    }
    pub fn producer_paused(&self) -> bool {
        self.resource
            .with(Consumer::producer_paused)
            .unwrap_or_else(|| self.confirmed.load(Ordering::SeqCst))
    }
    pub fn closed(&self) -> bool {
        self.resource
            .with(Consumer::closed)
            .unwrap_or_else(|| self.confirmed.load(Ordering::SeqCst))
    }
    pub async fn pause(&self) -> Result<(), SfuError> {
        self.resource.run(|c| async move { c.pause().await }).await
    }
    pub async fn resume(&self) -> Result<(), SfuError> {
        self.resource.run(|c| async move { c.resume().await }).await
    }
    pub async fn set_preferred_layers(&self, layers: ConsumerLayers) -> Result<(), SfuError> {
        self.resource
            .run(|c| async move { c.set_preferred_layers(layers).await })
            .await
    }
    pub async fn confirm_closed(&self) {
        self.resource.close();
        let (id, transport, lost) = (self.id, &self.transport, &self.worker_lost);
        let removed = await_native_removal(
            || transport.closed() || lost.load(Ordering::SeqCst),
            move || async move {
                tokio::time::timeout(DUMP_TIMEOUT, transport.dump())
                    .await
                    .ok()
                    .and_then(Result::ok)
                    .map(|dump| dump.consumer_ids.contains(&id))
            },
            CONFIRM_DEADLINE,
        )
        .await;
        if let Err(failure) = removed {
            unconfirmed_close(id, failure);
        }
        self.confirmed.store(true, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;
    #[tokio::test]
    async fn already_signalled_close_never_starts_a_new_native_operation() {
        let resource = Resource::new(1u8);
        resource.closed.send_replace(true);
        let result = resource
            .run(|_| async {
                panic!("RPC started after close notification");
                #[allow(unreachable_code)]
                Ok::<(), std::io::Error>(())
            })
            .await;
        assert!(matches!(result, Err(SfuError::Unavailable)));
    }

    #[tokio::test]
    async fn closed_transport_confirms_removal_without_a_dump() {
        let dumps = AtomicU32::new(0);
        let removed = await_native_removal(
            || true,
            || {
                dumps.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Some(true))
            },
            CONFIRM_DEADLINE,
        )
        .await;
        assert!(removed.is_ok());
        assert_eq!(dumps.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn removal_is_confirmed_once_the_worker_stops_listing_it() {
        let dumps = AtomicU32::new(0);
        let removed = await_native_removal(
            || false,
            || std::future::ready(Some(dumps.fetch_add(1, Ordering::SeqCst) < 3)),
            CONFIRM_DEADLINE,
        )
        .await;
        assert!(removed.is_ok());
        assert_eq!(dumps.load(Ordering::SeqCst), 4);
    }

    #[tokio::test]
    async fn worker_still_listing_past_the_deadline_is_unconfirmed() {
        let deadline = Duration::from_millis(100);
        let started = Instant::now();
        let failure = await_native_removal(|| false, || std::future::ready(Some(true)), deadline)
            .await
            .unwrap_err();
        assert!(started.elapsed() >= deadline);
        // Dumps back off (10, 20, 40, 80ms) instead of one every 10ms.
        assert!(failure.polls <= 8, "{failure:?}");
        assert_eq!(failure.unanswered, 0);
    }

    #[tokio::test]
    async fn unanswered_dumps_never_make_a_close_unconfirmed() {
        let deadline = Duration::from_millis(100);
        let wait = tokio::time::timeout(
            5 * deadline,
            await_native_removal(|| false, || std::future::ready(None), deadline),
        )
        .await;
        assert!(wait.is_err(), "a dump timeout is liveness, not a leak");
    }

    #[tokio::test]
    async fn worker_that_answers_again_and_still_lists_is_unconfirmed() {
        let deadline = Duration::from_millis(100);
        let started = Instant::now();
        let failure = await_native_removal(
            || false,
            || std::future::ready((started.elapsed() >= deadline).then_some(true)),
            deadline,
        )
        .await
        .unwrap_err();
        assert!(failure.unanswered > 0, "{failure:?}");
        assert!(failure.polls > failure.unanswered, "{failure:?}");
    }
}
