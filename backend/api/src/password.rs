//! Argon2id password hashing (RustCrypto `argon2`, PHC string format).
//!
//! Hashing takes tens of milliseconds by design, so both operations run on
//! Tokio's blocking pool and never stall the request executor. Each run
//! holds 19 MiB, so only [`concurrent_runs`] go at once.

use std::sync::OnceLock;
use std::time::Duration;

use argon2::password_hash::{PasswordHasher, PasswordVerifier, phc::PasswordHash};
use argon2::{Algorithm, Argon2, Params, Version};
use tokio::sync::Semaphore;
use tokio::task::spawn_blocking;

use crate::error::ApiError;

/// OWASP's current recommendation for Argon2id: 19 MiB, 2 iterations, 1 lane.
fn hasher() -> Argon2<'static> {
    let params = Params::new(19 * 1024, 2, 1, None).expect("static Argon2 params are valid");
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
}

/// How long a request waits for a free run before it is answered `429`.
const RUN_WAIT: Duration = Duration::from_secs(5);

/// One run per core: more cannot finish sooner, and each one waiting in the
/// blocking pool would already hold its memory.
fn concurrent_runs() -> &'static Semaphore {
    static RUNS: OnceLock<Semaphore> = OnceLock::new();
    RUNS.get_or_init(|| {
        let cores = std::thread::available_parallelism().map_or(2, |n| n.get());
        Semaphore::new(cores.max(2))
    })
}

/// Runs `work` on the blocking pool once `runs` has a permit, or refuses
/// like a rate limit after `wait`. The run itself holds the permit: a caller
/// that is dropped (client gone) must not free it while the hash goes on.
async fn limited<T: Send + 'static>(
    runs: &'static Semaphore,
    wait: Duration,
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, ApiError> {
    let Ok(Ok(permit)) = tokio::time::timeout(wait, runs.acquire()).await else {
        tracing::warn!("password hashing at capacity; request refused");
        return Err(ApiError::RateLimited { retry_after: 1 });
    };
    Ok(spawn_blocking(move || {
        let _permit = permit;
        work()
    })
    .await?)
}

/// Hashes `password` with a fresh random salt. Returns the PHC string that
/// goes into `users.password_hash`.
pub async fn hash(password: String) -> Result<String, ApiError> {
    limited(concurrent_runs(), RUN_WAIT, move || {
        hasher()
            .hash_password(password.as_bytes())
            .map(|hash| hash.to_string())
            .map_err(|err| ApiError::Internal(format!("argon2 hash failed: {err}")))
    })
    .await?
}

/// Verifies `password` against a stored PHC string, or — when there is no
/// account — against the dummy hash, so both paths cost one Argon2 run on
/// the blocking pool. A malformed stored hash counts as "does not match"
/// so a corrupt row cannot open an account. Always `false` for `None`.
pub async fn verify(password: String, stored: Option<String>) -> Result<bool, ApiError> {
    limited(concurrent_runs(), RUN_WAIT, move || {
        let has_account = stored.is_some();
        let stored = stored.unwrap_or_else(|| dummy_hash().to_owned());
        let Ok(parsed) = PasswordHash::new(&stored) else {
            return false;
        };
        let matches = hasher()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok();
        matches && has_account
    })
    .await
}

static DUMMY: OnceLock<String> = OnceLock::new();

/// A valid hash of an unguessable value. Login verifies against it when the
/// e-mail is unknown, so "no such user" and "wrong password" take the same
/// time and cannot be told apart by timing.
fn dummy_hash() -> &'static str {
    DUMMY.get_or_init(|| {
        hasher()
            .hash_password(crate::token::generate().as_bytes())
            .expect("hashing a random token cannot fail")
            .to_string()
    })
}

/// Computes the dummy hash ahead of the first request. Without this the
/// first unknown-e-mail login would pay for two Argon2 runs — exactly the
/// timing signal the dummy exists to hide.
pub async fn warm_up() -> Result<(), ApiError> {
    spawn_blocking(|| {
        dummy_hash();
    })
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn round_trips_and_uses_argon2id() {
        let stored = hash("correct horse".into()).await.unwrap();
        assert!(stored.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"));
        assert!(
            verify("correct horse".into(), Some(stored.clone()))
                .await
                .unwrap()
        );
        assert!(!verify("wrong".into(), Some(stored)).await.unwrap());
    }

    #[tokio::test]
    async fn a_full_pool_answers_rate_limited_after_the_wait() {
        let pool: &'static Semaphore = Box::leak(Box::new(Semaphore::new(1)));
        let held = pool.acquire().await.unwrap();
        let wait = Duration::from_millis(50);
        let busy = limited(pool, wait, || 1).await;
        assert!(
            matches!(busy, Err(ApiError::RateLimited { retry_after: 1 })),
            "{busy:?}"
        );
        drop(held);
        assert_eq!(limited(pool, wait, || 2).await.unwrap(), 2);
    }

    /// A caller that goes away (client disconnect) must not free the run's
    /// permit while the hash is still running and holding its memory.
    #[tokio::test]
    async fn a_dropped_caller_keeps_the_permit_until_the_run_ends() {
        let pool: &'static Semaphore = Box::leak(Box::new(Semaphore::new(1)));
        let (finish, finished) = std::sync::mpsc::channel::<()>();
        let caller = tokio::spawn(limited(pool, Duration::from_secs(1), move || {
            finished.recv().ok();
        }));
        while pool.available_permits() != 0 {
            tokio::task::yield_now().await;
        }
        caller.abort();
        assert!(caller.await.unwrap_err().is_cancelled());
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(pool.available_permits(), 0, "released while hashing");

        finish.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while pool.available_permits() != 1 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("released once the run ended");
    }

    #[tokio::test]
    async fn malformed_stored_hash_never_matches() {
        assert!(
            !verify("x".into(), Some("not-a-phc-string".into()))
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn missing_account_never_matches() {
        warm_up().await.unwrap();
        assert!(!verify("".into(), None).await.unwrap());
        assert!(!verify("password".into(), None).await.unwrap());
        assert!(dummy_hash().starts_with("$argon2id$"));
    }
}
