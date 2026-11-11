//! The languages a listing can declare: ISO 639-1 codes with their
//! endonyms. The names are data served by `GET /api/directory`, not
//! translated copy, so neither client hardcodes the list.

/// Code and endonym, in the order the clients show them.
pub const LANGUAGES: &[(&str, &str)] = &[
    ("de", "Deutsch"),
    ("en", "English"),
    ("es", "Español"),
    ("fr", "Français"),
    ("it", "Italiano"),
    ("nl", "Nederlands"),
    ("pl", "Polski"),
    ("pt", "Português"),
    ("tr", "Türkçe"),
    ("uk", "Українська"),
];

/// The listed code equal to `code`, if any.
pub fn find(code: &str) -> Option<&'static str> {
    LANGUAGES
        .iter()
        .map(|(known, _)| *known)
        .find(|known| *known == code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_unique_two_letter_lower_case() {
        for (i, (code, name)) in LANGUAGES.iter().enumerate() {
            assert_eq!(code.len(), 2, "{code}");
            assert!(code.chars().all(|c| c.is_ascii_lowercase()), "{code}");
            assert!(!name.is_empty());
            assert!(LANGUAGES[i + 1..].iter().all(|(other, _)| other != code));
        }
    }

    #[test]
    fn find_knows_only_listed_codes() {
        assert_eq!(find("de"), Some("de"));
        assert_eq!(find("uk"), Some("uk"));
        assert_eq!(find("DE"), None);
        assert_eq!(find("xx"), None);
        assert_eq!(find(""), None);
    }
}
