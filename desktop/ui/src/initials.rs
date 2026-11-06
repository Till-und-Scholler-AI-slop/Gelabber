//! Letters and tints for avatars and server tiles without a picture
//! (web/src/components/initials.ts), so a person or server has the same
//! colour on both clients.

/// Up to two initials; `?` when the name is blank.
pub fn initials(name: &str) -> String {
    let letters: String = name
        .split_whitespace()
        .take(2)
        .filter_map(|part| part.chars().next())
        .collect();
    if letters.is_empty() {
        "?".into()
    } else {
        letters.to_uppercase()
    }
}

/// A server tile's letters: a one-word name keeps its second letter
/// ("Gaming" → "Ga").
pub fn tile_initials(name: &str) -> String {
    let parts: Vec<&str> = name.split_whitespace().collect();
    if parts.len() != 1 {
        return initials(name);
    }
    let mut chars = parts[0].chars();
    let first: String = chars
        .next()
        .map(|c| c.to_uppercase().collect())
        .unwrap_or_default();
    let second: String = chars
        .next()
        .map(|c| c.to_lowercase().collect())
        .unwrap_or_default();
    first + &second
}

/// Which of the eight tile tints (1-based) a server id gets.
pub fn tile_tone(id: &str) -> usize {
    let mut hash: u32 = 0;
    // `for (const char of id) … char.charCodeAt(0)`: the first UTF-16 unit
    // of every code point.
    for c in id.chars() {
        let mut units = [0u16; 2];
        let first = c.encode_utf16(&mut units)[0];
        hash = hash.wrapping_mul(31).wrapping_add(u32::from(first));
    }
    (hash % 8) as usize + 1
}

/// The tint (1-based) for a person's initials: FNV-1a with a final scramble.
pub fn person_tone(name: &str) -> usize {
    let mut hash: u32 = 0x811c_9dc5;
    for c in name.chars() {
        hash ^= c as u32;
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash ^= hash >> 16;
    hash = hash.wrapping_mul(0x045d_9f3b);
    hash ^= hash >> 16;
    (hash % 8) as usize + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters() {
        assert_eq!(initials("  "), "?");
        assert_eq!(initials("lena maier extra"), "LM");
        assert_eq!(tile_initials("Gaming"), "Ga");
        assert_eq!(tile_initials("Die Gilde"), "DG");
    }

    #[test]
    fn tones_match_the_web() {
        // Values from initials.ts in node.
        let people = ["Lena", "Jonas", "Mira", "Tom", "Ada", "Zoë 🙂"].map(person_tone);
        assert_eq!(people, [1, 6, 1, 3, 8, 8]);
        let tiles = [
            "3796349f-6f8f-4bcf-826f-d2237e2fe9d3",
            "7cd32018-b2d5-4a9d-af2e-eb2b2a8366ab",
            "b841af64-31ae-4d74-929e-9c4c3525413d",
        ]
        .map(tile_tone);
        assert_eq!(tiles, [5, 8, 1]);
    }
}
