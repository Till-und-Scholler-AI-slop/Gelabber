//! PKCE (RFC 7636) with the `S256` method only.

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD as B64};
use rand::Rng;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

/// A fresh code verifier: 32 random bytes as base64url without padding,
/// which is 43 characters.
pub fn verifier() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    B64.encode(bytes)
}

/// The `S256` challenge of `verifier`: base64url(SHA-256(verifier)).
pub fn challenge(verifier: &str) -> String {
    B64.encode(Sha256::digest(verifier.as_bytes()))
}

/// Whether `verifier` belongs to `challenge`, compared in constant time.
/// The desktop app proves with it that it started the flow.
pub fn matches(verifier: &str, challenge: &str) -> bool {
    self::challenge(verifier)
        .as_bytes()
        .ct_eq(challenge.as_bytes())
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn challenge_matches_rfc_7636_appendix_b() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let expected = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
        assert_eq!(challenge(verifier), expected);
        assert!(matches(verifier, expected));
        assert!(!matches(
            verifier,
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cN"
        ));
        assert!(!matches(
            "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXl",
            expected
        ));
    }

    #[test]
    fn verifiers_are_43_url_safe_chars_and_differ() {
        let first = verifier();
        assert_eq!(first.len(), 43);
        assert!(
            first
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        );
        assert_ne!(first, verifier());
        assert_eq!(challenge(&first).len(), 43);
    }
}
