//! Bounded callers with owned completion of late native creation replies.
use crate::error::SfuError;
use std::{fmt::Display, future::Future, sync::Arc, time::Duration};
use tokio::sync::{Semaphore, oneshot};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(2);
// Timed-out operations retain their slot until completion. A responsive worker
// with stuck individual requests must not accumulate unlimited cleanup tasks.
const COMMAND_SLOTS: usize = 256;

/// One worker's command slots. Each worker has its own, so a stuck worker
/// cannot take the slots of the healthy ones.
pub(crate) fn command_slots() -> Arc<Semaphore> {
    Arc::new(Semaphore::new(COMMAND_SLOTS))
}

pub(crate) async fn native<R, T, E, F>(
    slots: &Arc<Semaphore>,
    resource: R,
    command: impl FnOnce(R) -> F + Send + 'static,
) -> Result<T, SfuError>
where
    R: Send + 'static,
    T: Send + 'static,
    E: Display + Send + 'static,
    F: Future<Output = Result<T, E>> + Send + 'static,
{
    let permit = slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| SfuError::Unavailable)?;
    let (send, receive) = oneshot::channel();
    tokio::spawn(async move {
        let _permit = permit;
        let result = command(resource).await.map_err(SfuError::negotiation);
        // If the caller timed out or was cancelled, send fails and drops the
        // returned SDK handle, closing the late object instead of orphaning it.
        let _ = send.send(result);
    });
    tokio::time::timeout(COMMAND_TIMEOUT, receive)
        .await
        .map_err(|_| SfuError::Unavailable)?
        .map_err(|_| SfuError::Unavailable)?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    #[derive(Debug)]
    struct Handle(Arc<AtomicBool>);
    impl Drop for Handle {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn timeout_returns_before_completion_and_drops_the_late_handle() {
        let dropped = Arc::new(AtomicBool::new(false));
        let (complete, pending) = oneshot::channel();
        let late = dropped.clone();
        let slots = command_slots();
        let result = tokio::time::timeout(
            COMMAND_TIMEOUT + Duration::from_millis(500),
            native(&slots, pending, move |pending| async move {
                pending.await.unwrap();
                Ok::<_, std::io::Error>(Handle(late))
            }),
        )
        .await
        .expect("caller must not wait for the late native reply");
        assert!(matches!(result, Err(SfuError::Unavailable)));
        assert!(!dropped.load(Ordering::SeqCst));
        // Other operations can still complete while this one awaits its reply.
        assert_eq!(
            native(
                &slots,
                7,
                |value| async move { Ok::<_, std::io::Error>(value) }
            )
            .await
            .unwrap(),
            7
        );
        complete.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            while !dropped.load(Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn cancelling_the_caller_still_retires_a_late_handle() {
        let dropped = Arc::new(AtomicBool::new(false));
        let (started, ready) = oneshot::channel();
        let (complete, pending) = oneshot::channel();
        let late = dropped.clone();
        let slots = command_slots();
        let caller = tokio::spawn(async move {
            native(&slots, pending, move |pending| async move {
                started.send(()).unwrap();
                pending.await.unwrap();
                Ok::<_, std::io::Error>(Handle(late))
            })
            .await
        });
        ready.await.unwrap();
        caller.abort();
        assert!(caller.await.unwrap_err().is_cancelled());
        complete.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            while !dropped.load(Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn a_stuck_worker_does_not_take_the_slots_of_another() {
        let (stuck, healthy) = (command_slots(), command_slots());
        let mut pending = Vec::new();
        for _ in 0..COMMAND_SLOTS {
            let (hold, wait) = oneshot::channel::<()>();
            pending.push(hold);
            let slots = stuck.clone();
            tokio::spawn(async move {
                let _ = native(&slots, wait, |wait| async move {
                    let _ = wait.await;
                    Ok::<_, std::io::Error>(())
                })
                .await;
            });
        }
        tokio::time::timeout(Duration::from_secs(1), async {
            while stuck.available_permits() > 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(matches!(
            native(&stuck, (), |()| async { Ok::<_, std::io::Error>(()) }).await,
            Err(SfuError::Unavailable)
        ));
        assert_eq!(
            native(&healthy, 3, |value| async move {
                Ok::<_, std::io::Error>(value)
            })
            .await
            .unwrap(),
            3
        );
        drop(pending);
    }
}
