//! A request in flight whose answer the UI picks up on a later frame, the
//! way a TanStack mutation hands its result to the component.

use std::sync::Arc;

use gelabber_client::Client;
use parking_lot::Mutex;

/// Runs a future on the client runtime; [`Pending::take`] yields its output
/// once, on the first frame after it finished.
pub struct Pending<T> {
    slot: Arc<Mutex<Option<T>>>,
}

impl<T: Send + 'static> Pending<T> {
    pub fn spawn<F>(client: &Client, work: F) -> Self
    where
        F: Future<Output = T> + Send + 'static,
    {
        let slot = Arc::new(Mutex::new(None));
        let out = slot.clone();
        client.spawn(async move {
            let value = work.await;
            *out.lock() = Some(value);
        });
        Self { slot }
    }

    /// Runs `work` on a thread of its own and wakes the window when done:
    /// for CPU or file work that must not hold up the UI or the runtime. A
    /// panic in `work`, or no thread to run it, ends in `lost()`, so the
    /// form does not wait for ever.
    pub fn thread(
        ctx: &egui::Context,
        work: impl FnOnce() -> T + Send + 'static,
        lost: impl FnOnce() -> T + Send + 'static,
    ) -> Self {
        let slot = Arc::new(Mutex::new(None));
        let out = slot.clone();
        let ctx = ctx.clone();
        let lost = Arc::new(Mutex::new(Some(lost)));
        let fallback = lost.clone();
        let spawned = std::thread::Builder::new().spawn(move || {
            let value = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work))
                .unwrap_or_else(|_| (lost.lock().take().expect("once"))());
            *out.lock() = Some(value);
            ctx.request_repaint();
        });
        if spawned.is_err()
            && let Some(lost) = fallback.lock().take()
        {
            *slot.lock() = Some(lost());
        }
        Self { slot }
    }

    /// The output, once it is there.
    pub fn take(&self) -> Option<T> {
        self.slot.lock().take()
    }
}

/// Polls `pending`: returns the output and clears the slot when done.
pub fn poll<T: Send + 'static>(pending: &mut Option<Pending<T>>) -> Option<T> {
    let value = pending.as_ref()?.take()?;
    *pending = None;
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::Pending;

    /// A worker that panics still answers, with what a lost one means.
    #[test]
    fn a_panicking_worker_ends_in_its_failure() {
        let ctx = egui::Context::default();
        let pending: Pending<Result<u8, &str>> =
            Pending::thread(&ctx, || panic!("decoder bug"), || Err("lost"));
        let start = std::time::Instant::now();
        let value = loop {
            if let Some(v) = pending.take() {
                break v;
            }
            assert!(start.elapsed().as_secs() < 10, "no answer");
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        assert_eq!(value, Err("lost"));
    }
}
