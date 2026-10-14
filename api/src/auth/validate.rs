//! Input rules for auth and profile bodies. Every rule yields a field code
//! (`required`, `invalid`, `too_short`, `too_long`) that the web client turns
//! into inline copy; the same limits are mirrored in `web/src/auth/rules.ts`.

use crate::error::{ApiError, FieldErrors};

pub const PASSWORD_MIN: usize = 8;
pub const PASSWORD_MAX: usize = 128;
pub const NAME_MAX: usize = 64;
pub const EMAIL_MAX: usize = 254;
pub const AVATAR_URL_MAX: usize = 2048;
pub const BIO_MAX: usize = 190;
pub const PRONOUNS_MAX: usize = 40;
pub const STATUS_TEXT_MAX: usize = 80;
/// Drawn by the client in CSS; no images, nothing to upload.
pub const AVATAR_DECORATIONS: [&str; 4] = ["ring", "glow", "rainbow", "stars"];

/// Trims and lower-cases; accepts the pragmatic `local@domain.tld` shape.
pub fn email(raw: &str, errors: &mut FieldErrors) -> Option<String> {
    let value = raw.trim().to_lowercase();
    if value.is_empty() {
        errors.insert("email", "required");
        return None;
    }
    if value.chars().count() > EMAIL_MAX {
        errors.insert("email", "too_long");
        return None;
    }
    let ok = match value.split_once('@') {
        Some((local, domain)) => {
            !local.is_empty()
                && !domain.is_empty()
                && domain.contains('.')
                && !domain.starts_with('.')
                && !domain.ends_with('.')
                && !value.chars().any(|c| c.is_whitespace() || c.is_control())
                && value.matches('@').count() == 1
        }
        None => false,
    };
    if !ok {
        errors.insert("email", "invalid");
        return None;
    }
    Some(value)
}

/// Length is measured in characters. No composition rules: length is what
/// Argon2id needs; everything else pushes users towards worse passwords.
pub fn password(raw: &str, errors: &mut FieldErrors) -> Option<String> {
    let count = raw.chars().count();
    if count == 0 {
        errors.insert("password", "required");
        return None;
    }
    if count < PASSWORD_MIN {
        errors.insert("password", "too_short");
        return None;
    }
    if count > PASSWORD_MAX {
        errors.insert("password", "too_long");
        return None;
    }
    Some(raw.to_owned())
}

pub fn name(raw: &str, errors: &mut FieldErrors) -> Option<String> {
    let value = raw.trim();
    if value.is_empty() {
        errors.insert("name", "required");
        return None;
    }
    if value.chars().count() > NAME_MAX {
        errors.insert("name", "too_long");
        return None;
    }
    if value.chars().any(char::is_control) {
        errors.insert("name", "invalid");
        return None;
    }
    Some(value.to_owned())
}

/// `None` (empty string) clears the avatar; otherwise an absolute `https://`
/// URL — plain `http://` would be mixed content once the app sits behind
/// TLS — or an uploaded avatar's own path (`crate::avatars`), which the
/// profile form sends back unchanged with every save.
pub fn avatar_url(raw: &str, errors: &mut FieldErrors) -> Option<Option<String>> {
    let value = raw.trim();
    if value.is_empty() {
        return Some(None);
    }
    if crate::avatars::parse_url(value).is_some() {
        return Some(Some(value.to_owned()));
    }
    if value.chars().count() > AVATAR_URL_MAX {
        errors.insert("avatar_url", "too_long");
        return None;
    }
    const SCHEME: &str = "https://";
    let has_host = value.len() > SCHEME.len()
        && value
            .get(..SCHEME.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(SCHEME))
        && !value[SCHEME.len()..].starts_with(['/', '.', '?', '#']);
    if !has_host || value.chars().any(|c| c.is_whitespace() || c.is_control()) {
        errors.insert("avatar_url", "invalid");
        return None;
    }
    Some(Some(value.to_owned()))
}

/// Optional free text: trimmed; empty clears (`Some(None)`). Line breaks
/// only where `multiline`, and then as plain `\n`.
fn optional_text(
    field: &'static str,
    raw: &str,
    max: usize,
    multiline: bool,
    errors: &mut FieldErrors,
) -> Option<Option<String>> {
    let normalised = raw.replace("\r\n", "\n");
    let value = normalised.trim();
    if value.is_empty() {
        return Some(None);
    }
    if value.chars().count() > max {
        errors.insert(field, "too_long");
        return None;
    }
    if value
        .chars()
        .any(|c| c.is_control() && !(multiline && c == '\n'))
    {
        errors.insert(field, "invalid");
        return None;
    }
    Some(Some(value.to_owned()))
}

pub fn bio(raw: &str, errors: &mut FieldErrors) -> Option<Option<String>> {
    optional_text("bio", raw, BIO_MAX, true, errors)
}

pub fn pronouns(raw: &str, errors: &mut FieldErrors) -> Option<Option<String>> {
    optional_text("pronouns", raw, PRONOUNS_MAX, false, errors)
}

pub fn status_text(raw: &str, errors: &mut FieldErrors) -> Option<Option<String>> {
    optional_text("status_text", raw, STATUS_TEXT_MAX, false, errors)
}

/// `#rrggbb` in any case, stored lower-case; empty clears. No names, no
/// alpha: the value goes into inline styles and must stay a plain color.
pub fn color(field: &'static str, raw: &str, errors: &mut FieldErrors) -> Option<Option<String>> {
    let value = raw.trim();
    if value.is_empty() {
        return Some(None);
    }
    let ok = value.len() == 7
        && value.starts_with('#')
        && value[1..].chars().all(|c| c.is_ascii_hexdigit());
    if !ok {
        errors.insert(field, "invalid");
        return None;
    }
    Some(Some(value.to_ascii_lowercase()))
}

pub fn avatar_decoration(raw: &str, errors: &mut FieldErrors) -> Option<Option<String>> {
    let value = raw.trim();
    if value.is_empty() {
        return Some(None);
    }
    if !AVATAR_DECORATIONS.contains(&value) {
        errors.insert("avatar_decoration", "invalid");
        return None;
    }
    Some(Some(value.to_owned()))
}

pub fn finish(errors: FieldErrors) -> Result<(), ApiError> {
    if errors.is_empty() {
        Ok(())
    } else {
        Err(ApiError::Validation(errors))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run<T>(f: impl FnOnce(&mut FieldErrors) -> Option<T>) -> (Option<T>, FieldErrors) {
        let mut errors = FieldErrors::new();
        let value = f(&mut errors);
        (value, errors)
    }

    #[test]
    fn email_normalises_and_validates() {
        let (value, errors) = run(|e| email("  Ada@Example.COM ", e));
        assert_eq!(value.as_deref(), Some("ada@example.com"));
        assert!(errors.is_empty());

        for (raw, code) in [
            ("", "required"),
            ("   ", "required"),
            ("nope", "invalid"),
            ("@example.com", "invalid"),
            ("ada@", "invalid"),
            ("ada@localhost", "invalid"),
            ("ada@@example.com", "invalid"),
            ("a da@example.com", "invalid"),
        ] {
            let (value, errors) = run(|e| email(raw, e));
            assert!(value.is_none(), "{raw:?}");
            assert_eq!(errors.get("email"), Some(&code), "{raw:?}");
        }

        let long = format!("{}@example.com", "a".repeat(EMAIL_MAX));
        let (_, errors) = run(|e| email(&long, e));
        assert_eq!(errors.get("email"), Some(&"too_long"));
    }

    #[test]
    fn password_checks_length_in_chars() {
        assert!(run(|e| password("12345678", e)).0.is_some());
        assert!(run(|e| password("pässwörd", e)).0.is_some());
        assert_eq!(
            run(|e| password("", e)).1.get("password"),
            Some(&"required")
        );
        assert_eq!(
            run(|e| password("1234567", e)).1.get("password"),
            Some(&"too_short")
        );
        assert_eq!(
            run(|e| password(&"x".repeat(PASSWORD_MAX + 1), e))
                .1
                .get("password"),
            Some(&"too_long")
        );
    }

    #[test]
    fn name_trims_and_rejects_control_chars() {
        assert_eq!(run(|e| name("  Ada ", e)).0.as_deref(), Some("Ada"));
        assert_eq!(run(|e| name("   ", e)).1.get("name"), Some(&"required"));
        assert_eq!(run(|e| name("a\nb", e)).1.get("name"), Some(&"invalid"));
        assert_eq!(
            run(|e| name(&"x".repeat(NAME_MAX + 1), e)).1.get("name"),
            Some(&"too_long")
        );
    }

    #[test]
    fn avatar_url_accepts_https_only_or_clears() {
        assert_eq!(run(|e| avatar_url("", e)).0, Some(None));
        assert_eq!(
            run(|e| avatar_url("https://cdn.example/a.png", e)).0,
            Some(Some("https://cdn.example/a.png".to_owned()))
        );
        assert_eq!(
            run(|e| avatar_url("HTTPS://cdn.example/a.png", e)).0,
            Some(Some("HTTPS://cdn.example/a.png".to_owned()))
        );
        for raw in [
            "aaaaaaaéx",
            "🦀🦀🦀",
            "javascript:alert(1)",
            "ftp://x/y",
            "https://",
            "https:///etc",
            "https://a b",
            "http://cdn.example/a.png",
            "http://a b",
        ] {
            let (value, errors) = run(|e| avatar_url(raw, e));
            assert!(value.is_none(), "{raw:?}");
            assert_eq!(errors.get("avatar_url"), Some(&"invalid"), "{raw:?}");
        }
    }

    #[test]
    fn profile_texts_trim_clear_and_limit() {
        assert_eq!(
            run(|e| pronouns("  sie/ihr ", e)).0,
            Some(Some("sie/ihr".into()))
        );
        assert_eq!(run(|e| status_text("   ", e)).0, Some(None));
        assert_eq!(run(|e| bio("", e)).0, Some(None));
        assert_eq!(
            run(|e| bio(" Hallo\r\nWelt ", e)).0,
            Some(Some("Hallo\nWelt".into()))
        );
        assert_eq!(
            run(|e| status_text("a\nb", e)).1.get("status_text"),
            Some(&"invalid")
        );
        assert_eq!(run(|e| bio("a\tb", e)).1.get("bio"), Some(&"invalid"));
        assert_eq!(run(|e| bio("a\u{85}b", e)).1.get("bio"), Some(&"invalid"));
        assert!(run(|e| bio(&"ä".repeat(BIO_MAX), e)).0.is_some());
        assert_eq!(
            run(|e| bio(&"x".repeat(BIO_MAX + 1), e)).1.get("bio"),
            Some(&"too_long")
        );
        assert_eq!(
            run(|e| pronouns(&"x".repeat(PRONOUNS_MAX + 1), e))
                .1
                .get("pronouns"),
            Some(&"too_long")
        );
        assert_eq!(
            run(|e| status_text(&"x".repeat(STATUS_TEXT_MAX + 1), e))
                .1
                .get("status_text"),
            Some(&"too_long")
        );
    }

    #[test]
    fn colors_are_six_digit_hex_stored_lower_case() {
        assert_eq!(
            run(|e| color("accent_color", " #A1b2C3 ", e)).0,
            Some(Some("#a1b2c3".into()))
        );
        assert_eq!(run(|e| color("accent_color", "", e)).0, Some(None));
        for raw in [
            "a1b2c3",
            "#abc",
            "#a1b2c3d4",
            "#gggggg",
            "red",
            "#ä1b2c",
            "url(x)",
        ] {
            let (value, errors) = run(|e| color("banner_color", raw, e));
            assert!(value.is_none(), "{raw:?}");
            assert_eq!(errors.get("banner_color"), Some(&"invalid"), "{raw:?}");
        }
    }

    #[test]
    fn decorations_come_from_the_fixed_set() {
        for name in AVATAR_DECORATIONS {
            assert_eq!(
                run(|e| avatar_decoration(name, e)).0,
                Some(Some(name.into()))
            );
        }
        assert_eq!(run(|e| avatar_decoration("", e)).0, Some(None));
        assert_eq!(
            run(|e| avatar_decoration("Ring", e))
                .1
                .get("avatar_decoration"),
            Some(&"invalid")
        );
        assert_eq!(
            run(|e| avatar_decoration("../x", e))
                .1
                .get("avatar_decoration"),
            Some(&"invalid")
        );
    }
}
