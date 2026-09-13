//! Input rules for auth and profile bodies. Every rule yields a field code
//! (`required`, `invalid`, `too_short`, `too_long`) that the web client turns
//! into inline copy; the same limits are mirrored in `web/src/auth/rules.ts`.

use crate::error::{ApiError, FieldErrors};

pub const PASSWORD_MIN: usize = 8;
pub const PASSWORD_MAX: usize = 128;
pub const NAME_MAX: usize = 64;
pub const EMAIL_MAX: usize = 254;
pub const AVATAR_URL_MAX: usize = 2048;

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

/// `None` (empty string) clears the avatar; otherwise an absolute http(s)
/// URL. The API stores the reference only — file upload lands with the
/// MinIO/files issue, not here.
pub fn avatar_url(raw: &str, errors: &mut FieldErrors) -> Option<Option<String>> {
    let value = raw.trim();
    if value.is_empty() {
        return Some(None);
    }
    if value.chars().count() > AVATAR_URL_MAX {
        errors.insert("avatar_url", "too_long");
        return None;
    }
    let lower = value.to_ascii_lowercase();
    let is_http = (lower.starts_with("https://") && value.len() > "https://".len())
        || (lower.starts_with("http://") && value.len() > "http://".len());
    if !is_http || value.chars().any(|c| c.is_whitespace() || c.is_control()) {
        errors.insert("avatar_url", "invalid");
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
    fn avatar_url_accepts_http_or_clears() {
        assert_eq!(run(|e| avatar_url("", e)).0, Some(None));
        assert_eq!(
            run(|e| avatar_url("https://cdn.example/a.png", e)).0,
            Some(Some("https://cdn.example/a.png".to_owned()))
        );
        for raw in ["javascript:alert(1)", "ftp://x/y", "https://", "http://a b"] {
            let (value, errors) = run(|e| avatar_url(raw, e));
            assert!(value.is_none(), "{raw:?}");
            assert_eq!(errors.get("avatar_url"), Some(&"invalid"), "{raw:?}");
        }
    }
}
