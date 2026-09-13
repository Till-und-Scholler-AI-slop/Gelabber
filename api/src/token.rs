//! Opaque random tokens for session cookies and CSRF.

use rand::Rng;
use sha2::{Digest, Sha256};

/// 256 bits from the OS-seeded thread RNG as 64 lower-case hex characters.
/// Hex keeps the value a plain cookie token (no `=`/`;`/`,`) without another
/// dependency.
pub fn generate() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    hex(&bytes)
}

/// Shape check for tokens coming back from a cookie: exactly what
/// `generate` produces. Anything else is rejected before touching the
/// database.
pub fn is_valid(raw: &str) -> bool {
    raw.len() == 64 && raw.bytes().all(|c| c.is_ascii_hexdigit())
}

/// SHA-256 of the raw token; this is what the `sessions` table stores.
pub fn hash(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_64_hex_chars_and_unique() {
        let a = generate();
        let b = generate();
        assert_eq!(a.len(), 64);
        assert!(a.bytes().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
        assert!(is_valid(&a));
    }

    #[test]
    fn is_valid_rejects_other_shapes() {
        assert!(!is_valid(""));
        assert!(!is_valid("not-a-real-token"));
        assert!(!is_valid(&"f".repeat(63)));
        assert!(!is_valid(&"g".repeat(64)));
        assert!(is_valid(&"f".repeat(64)));
    }

    #[test]
    fn hash_is_deterministic_sha256() {
        assert_eq!(hash("abc").len(), 32);
        assert_eq!(hash("abc"), hash("abc"));
        assert_ne!(hash("abc"), hash("abd"));
    }
}
