//! Explicit, cancellable ownership of SDK resources. The Rust SDK closes on
//! last drop; retaining an Arc<Publication> must not retain native forwarding.
use crate::error::SfuError;
use mediasoup::prelude::*;
use std::{
    future::Future,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::watch;

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
}
impl NativeProducer {
    pub fn new(producer: Producer, transport: WebRtcTransport) -> Self {
        Self {
            id: producer.id(),
            confirmed: AtomicBool::new(false),
            resource: Resource::new(producer),
            transport,
        }
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
        loop {
            if self.transport.closed() {
                self.confirmed.store(true, Ordering::SeqCst);
                return;
            }
            if let Ok(Ok(dump)) =
                tokio::time::timeout(std::time::Duration::from_millis(500), self.transport.dump())
                    .await
                && !dump.producer_ids.contains(&self.id)
            {
                self.confirmed.store(true, Ordering::SeqCst);
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }
}

pub struct NativeConsumer {
    pub resource: Resource<Consumer>,
    id: ConsumerId,
    confirmed: AtomicBool,
    transport: WebRtcTransport,
    rtp: RtpParameters,
}
impl NativeConsumer {
    pub fn new(consumer: Consumer, transport: WebRtcTransport) -> Self {
        Self {
            id: consumer.id(),
            confirmed: AtomicBool::new(false),
            rtp: consumer.rtp_parameters().clone(),
            resource: Resource::new(consumer),
            transport,
        }
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
        loop {
            if self.transport.closed() {
                self.confirmed.store(true, Ordering::SeqCst);
                return;
            }
            if let Ok(Ok(dump)) =
                tokio::time::timeout(std::time::Duration::from_millis(500), self.transport.dump())
                    .await
                && !dump.consumer_ids.contains(&self.id)
            {
                self.confirmed.store(true, Ordering::SeqCst);
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
