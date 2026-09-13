//! Argon2id password hashing (RustCrypto `argon2`, PHC string format).
//!
//! Hashing takes tens of milliseconds by design, so both operations run on
//! Tokio's blocking pool and never stall the request executor.

use argon2::password_hash::{PasswordHasher, PasswordVerifier, phc::PasswordHash};
use argon2::{Algorithm, Argon2, Params, Version};
use tokio::task::spawn_blocking;

use crate::error::ApiError;

/// OWASP's current recommendation for Argon2id: 19 MiB, 2 iterations, 1 lane.
fn hasher() -> Argon2<'static> {
    let params = Params::new(19 * 1024, 2, 1, None).expect("static Argon2 params are valid");
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
}

/// Hashes `password` with a fresh random salt. Returns the PHC string that
/// goes into `users.password_hash`.
pub async fn hash(password: String) -> Result<String, ApiError> {
    spawn_blocking(move || {
        hasher()
            .hash_password(password.as_bytes())
            .map(|hash| hash.to_string())
            .map_err(|err| ApiError::Internal(format!("argon2 hash failed: {err}")))
    })
    .await?
}

/// Verifies `password` against a stored PHC string. A malformed stored hash
/// counts as "does not match" so a corrupt row cannot open an account.
pub async fn verify(password: String, stored: String) -> Result<bool, ApiError> {
    Ok(spawn_blocking(move || {
        let Ok(parsed) = PasswordHash::new(&stored) else {
            return false;
        };
        hasher()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok()
    })
    .await?)
}

/// A valid hash of an unguessable value. Login verifies against it when the
/// e-mail is unknown, so "no such user" and "wrong password" take the same
/// time and cannot be told apart by timing.
pub fn dummy_hash() -> &'static str {
    use std::sync::OnceLock;
    static DUMMY: OnceLock<String> = OnceLock::new();
    DUMMY.get_or_init(|| {
        hasher()
            .hash_password(crate::token::generate().as_bytes())
            .expect("hashing a random token cannot fail")
            .to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn round_trips_and_uses_argon2id() {
        let stored = hash("correct horse".into()).await.unwrap();
        assert!(stored.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"));
        assert!(
            verify("correct horse".into(), stored.clone())
                .await
                .unwrap()
        );
        assert!(!verify("wrong".into(), stored).await.unwrap());
    }

    #[tokio::test]
    async fn malformed_stored_hash_never_matches() {
        assert!(!verify("x".into(), "not-a-phc-string".into()).await.unwrap());
    }

    #[tokio::test]
    async fn dummy_hash_rejects_everything_plausible() {
        assert!(!verify("".into(), dummy_hash().to_owned()).await.unwrap());
        assert!(
            !verify("password".into(), dummy_hash().to_owned())
                .await
                .unwrap()
        );
    }
}
