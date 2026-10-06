//! Bounded callers with owned completion of late native creation replies.
use crate::error::SfuError;
use std::{fmt::Display, future::Future, time::Duration};
use tokio::sync::{Semaphore, oneshot};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(2);
// Timed-out operations retain their slot until completion. A responsive worker
// with stuck individual requests must not accumulate unlimited cleanup tasks.
static COMMANDS: Semaphore = Semaphore::const_new(256);

pub(crate) async fn native<R, T, E, F>(
    resource: R,
    command: impl FnOnce(R) -> F + Send + 'static,
) -> Result<T, SfuError>
where
    R: Send + 'static,
    T: Send + 'static,
    E: Display + Send + 'static,
    F: Future<Output = Result<T, E>> + Send + 'static,
{
    let permit = COMMANDS.try_acquire().map_err(|_| SfuError::Unavailable)?;
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
        let result = tokio::time::timeout(
            COMMAND_TIMEOUT + Duration::from_millis(500),
            native(pending, move |pending| async move {
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
            native(7, |value| async move { Ok::<_, std::io::Error>(value) })
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
        let caller = tokio::spawn(native(pending, move |pending| async move {
            started.send(()).unwrap();
            pending.await.unwrap();
            Ok::<_, std::io::Error>(Handle(late))
        }));
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
}
