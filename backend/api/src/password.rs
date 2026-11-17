//! Argon2id password hashing (RustCrypto `argon2`, PHC string format).
//!
//! Hashing takes tens of milliseconds by design, so both operations run on
//! Tokio's blocking pool and never stall the request executor. Each run
//! holds 19 MiB, so only [`concurrent_runs`] go at once. That memory is
//! kept for the next run and handed back to the OS once sign-ins stop
//! ([`release_idle_memory`], #614).

use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use argon2::password_hash::phc::{Output, ParamsString, PasswordHash, Salt};
use argon2::password_hash::{PasswordHasher, try_generate_salt};
use argon2::{Algorithm, Argon2, Block, Params, Version};
use tokio::sync::Semaphore;
use tokio::task::spawn_blocking;
use zeroize::Zeroize;

use crate::error::ApiError;

/// OWASP's current recommendation for Argon2id: 19 MiB, 2 iterations, 1 lane.
fn params() -> Params {
    Params::new(19 * 1024, 2, 1, None).expect("static Argon2 params are valid")
}

fn hasher() -> Argon2<'static> {
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params())
}

/// Argon2's working memory, one buffer per run that finished, kept for the
/// next run. Memory of 1 MiB and more goes straight back to the OS when it
/// is freed (#614), so a fresh 19 MiB per run would fault in every page
/// again: a quarter fewer sign-ins per second.
struct Scratch {
    buffers: Vec<Vec<Block>>,
    last_run: Instant,
}

impl Scratch {
    fn release_if_idle(&mut self, idle: Duration) {
        if self.last_run.elapsed() >= idle {
            self.buffers = Vec::new();
        }
    }
}

fn scratch() -> MutexGuard<'static, Scratch> {
    static SCRATCH: OnceLock<Mutex<Scratch>> = OnceLock::new();
    SCRATCH
        .get_or_init(|| {
            Mutex::new(Scratch {
                buffers: Vec::new(),
                last_run: Instant::now(),
            })
        })
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Runs `work` with `blocks` blocks of memory. Buffers of the current
/// parameters' size are reused; any other size (a hash stored with other
/// parameters) gets a buffer of its own that is freed afterwards.
///
/// The pool holds at most one buffer per run that was ever in flight at
/// once, so its size is bounded only by [`concurrent_runs`] (one per core,
/// 19 MiB each). Raising that limit raises the memory kept here with it.
///
/// Every buffer is wiped before it is kept or freed: the blocks hold state
/// derived from the password, and with caller-supplied memory the crate
/// does not wipe them itself.
fn with_memory<T>(blocks: usize, work: impl FnOnce(&mut [Block]) -> T) -> T {
    let pooled = blocks == params().block_count();
    let reused = if pooled {
        scratch().buffers.pop()
    } else {
        None
    };
    let mut memory = reused.unwrap_or_else(|| vec![Block::new(); blocks]);
    let result = work(&mut memory);
    memory.iter_mut().for_each(Zeroize::zeroize);
    if pooled {
        let mut scratch = scratch();
        scratch.buffers.push(memory);
        scratch.last_run = Instant::now();
    }
    result
}

/// How long the working memory outlives the last run.
pub const IDLE_RELEASE: Duration = Duration::from_secs(30);

/// Frees the kept working memory once no run has finished for `idle`.
pub fn release_idle_memory(idle: Duration) {
    scratch().release_if_idle(idle);
}

/// Checks every [`IDLE_RELEASE`] whether the working memory can go.
pub fn spawn_idle_release() {
    tokio::spawn(async {
        let mut tick = tokio::time::interval(IDLE_RELEASE);
        loop {
            tick.tick().await;
            release_idle_memory(IDLE_RELEASE);
        }
    });
}

/// Argon2id with [`params`] and a fresh salt, as a PHC string. The same
/// string `PasswordHasher::hash_password` builds, on reused memory.
fn hash_with_memory(password: &[u8]) -> Result<String, String> {
    let params = params();
    let salt = try_generate_salt().map_err(|err| format!("no salt: {err}"))?;
    let mut buffer = [0u8; Output::MAX_LENGTH];
    let out = buffer
        .get_mut(..params.output_len().unwrap_or(Params::DEFAULT_OUTPUT_LEN))
        .ok_or("output length")?;
    with_memory(params.block_count(), |memory| {
        hasher().hash_password_into_with_memory(password, &salt, out, memory)
    })
    .map_err(|err| err.to_string())?;
    let hash = PasswordHash {
        algorithm: Algorithm::Argon2id.ident(),
        version: Some(Version::V0x13.into()),
        params: ParamsString::try_from(&params).map_err(|err| err.to_string())?,
        salt: Some(Salt::new(&salt).map_err(|err| err.to_string())?),
        hash: Some(Output::new(out).map_err(|err| err.to_string())?),
    };
    Ok(hash.to_string())
}

/// Whether `password` matches `stored`, with the algorithm, version and
/// parameters the stored hash names, as `PasswordVerifier` checks it: the
/// outputs are compared in constant time.
fn verify_with_memory(password: &[u8], stored: &PasswordHash) -> bool {
    let (Some(salt), Some(expected)) = (&stored.salt, &stored.hash) else {
        return false;
    };
    let Ok(algorithm) = Algorithm::try_from(stored.algorithm.as_str()) else {
        return false;
    };
    let Ok(version) = stored.version.map(Version::try_from).transpose() else {
        return false;
    };
    let Ok(params) = Params::try_from(stored) else {
        return false;
    };
    let blocks = params.block_count();
    let argon2 = Argon2::new(algorithm, version.unwrap_or_default(), params);
    let mut buffer = [0u8; Output::MAX_LENGTH];
    let Some(out) = buffer.get_mut(..expected.len()) else {
        return false;
    };
    let hashed = with_memory(blocks, |memory| {
        argon2.hash_password_into_with_memory(password, salt, out, memory)
    });
    hashed.is_ok() && Output::new(out).is_ok_and(|computed| computed == *expected)
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
        hash_with_memory(password.as_bytes())
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
        verify_with_memory(password.as_bytes(), &parsed) && has_account
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

    /// The strings agree with the crate's own hasher and verifier both ways,
    /// so hashes stored before #614 still verify and new ones stay standard.
    #[tokio::test]
    async fn hashes_match_the_crate_both_ways() {
        let ours = hash("correct horse".into()).await.unwrap();
        let parsed = PasswordHash::new(&ours).unwrap();
        use argon2::PasswordVerifier;
        assert!(hasher().verify_password(b"correct horse", &parsed).is_ok());
        assert!(hasher().verify_password(b"wrong", &parsed).is_err());

        let theirs = hasher()
            .hash_password(b"battery staple")
            .unwrap()
            .to_string();
        assert!(
            verify("battery staple".into(), Some(theirs.clone()))
                .await
                .unwrap()
        );
        assert!(!verify("battery".into(), Some(theirs)).await.unwrap());
    }

    /// A hash stored with other parameters verifies with its own, on memory
    /// that is not kept.
    #[tokio::test]
    async fn other_parameters_still_verify() {
        let small = Argon2::new(
            Algorithm::Argon2id,
            Version::V0x13,
            Params::new(8 * 1024, 1, 1, None).unwrap(),
        );
        let stored = small.hash_password(b"old account").unwrap().to_string();
        assert!(stored.contains("m=8192,t=1,p=1"));
        assert!(
            verify("old account".into(), Some(stored.clone()))
                .await
                .unwrap()
        );
        assert!(!verify("other".into(), Some(stored)).await.unwrap());
    }

    #[test]
    fn kept_memory_is_wiped() {
        with_memory(params().block_count(), |memory| {
            for block in memory.iter_mut() {
                block.as_mut().fill(7);
            }
        });
        // Only wiped buffers may ever sit in the pool, whichever test put
        // them there.
        let scratch = scratch();
        for buffer in &scratch.buffers {
            assert!(
                buffer
                    .iter()
                    .all(|block| block.as_ref().iter().all(|&word| word == 0)),
                "a kept buffer still holds run state"
            );
        }
    }

    #[test]
    fn idle_memory_is_released_and_recent_memory_kept() {
        let mut scratch = Scratch {
            buffers: vec![vec![Block::new(); 1]],
            last_run: Instant::now(),
        };
        scratch.release_if_idle(Duration::from_secs(3600));
        assert_eq!(scratch.buffers.len(), 1, "released while recent");
        scratch.release_if_idle(Duration::ZERO);
        assert!(scratch.buffers.is_empty(), "kept after idle");
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
