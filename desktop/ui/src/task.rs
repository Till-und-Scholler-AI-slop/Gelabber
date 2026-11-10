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
    /// for CPU or file work that must not hold up the UI or the runtime.
    pub fn thread(ctx: &egui::Context, work: impl FnOnce() -> T + Send + 'static) -> Self {
        let slot = Arc::new(Mutex::new(None));
        let out = slot.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let value = work();
            *out.lock() = Some(value);
            ctx.request_repaint();
        });
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
