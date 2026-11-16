//! Friend codes (#563): `K7QX-M2PD`, eight characters in two groups from an
//! alphabet without the look-alikes `0`/`O` and `1`/`I`. 32⁸ ≈ 10¹² codes,
//! and lookups are rate limited (`friends::CODE_LOOKUPS_PER_HOUR`), so
//! guessing one is hopeless.

use rand::RngExt;

pub const ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

/// A fresh random code.
pub fn generate() -> String {
    let mut rng = rand::rng();
    let mut out = String::with_capacity(9);
    for i in 0..8 {
        if i == 4 {
            out.push('-');
        }
        out.push(ALPHABET[rng.random_range(0..ALPHABET.len())] as char);
    }
    out
}

/// What someone typed or pasted, as stored: case, spaces and the dash do
/// not matter. `None` for anything that cannot be a code.
pub fn normalize(raw: &str) -> Option<String> {
    let chars: Vec<char> = raw
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if chars.len() != 8
        || !chars
            .iter()
            .all(|c| c.is_ascii() && ALPHABET.contains(&(*c as u8)))
    {
        return None;
    }
    let mut out: String = chars[..4].iter().collect();
    out.push('-');
    out.extend(&chars[4..]);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_codes_have_the_stored_shape() {
        for _ in 0..200 {
            let code = generate();
            assert_eq!(code.len(), 9);
            assert_eq!(&code[4..5], "-");
            assert_eq!(normalize(&code).as_deref(), Some(code.as_str()));
            assert!(!code.contains(['0', 'O', '1', 'I']), "{code}");
        }
    }

    #[test]
    fn typed_codes_are_normalized() {
        assert_eq!(normalize("k7qx-m2pd").as_deref(), Some("K7QX-M2PD"));
        assert_eq!(normalize(" K7QX M2PD ").as_deref(), Some("K7QX-M2PD"));
        assert_eq!(normalize("K7QXM2PD").as_deref(), Some("K7QX-M2PD"));
        assert_eq!(normalize("K7QX-M2P"), None);
        assert_eq!(normalize("K7QX-M2P0"), None);
        assert_eq!(normalize("K7QX-M2PÜ"), None);
        assert_eq!(normalize(""), None);
    }
}
