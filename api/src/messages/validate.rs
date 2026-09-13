//! Input rules for message bodies and history cursors. Field codes match
//! the rest of the API; the client mirrors the content limit in
//! `web/src/messages/rules.ts`.

use chrono::{DateTime, Utc};
use uuid::Uuid;

pub use crate::auth::validate::finish;
use crate::error::{ApiError, FieldErrors};

/// Locked for v1: long enough for a real paragraph, short enough that a
/// virtualised row stays cheap. The ticket left the number open; 2000 is
/// the figure it named.
pub const CONTENT_MAX: usize = 2000;
pub const LIMIT_DEFAULT: i64 = 50;
pub const LIMIT_MAX: i64 = 100;

/// A history cursor: either a message id (looked up to `(created_at, id)`)
/// or an RFC3339 timestamp (exclusive bound, id-padded so the pair compare
/// is stable).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cursor {
    Id(Uuid),
    Time(DateTime<Utc>),
}

pub fn content(raw: &str, errors: &mut FieldErrors) -> Option<String> {
    let value = raw.replace("\r\n", "\n").replace('\r', "\n");
    let value = value.trim();
    if value.is_empty() {
        errors.insert("content", "required");
        return None;
    }
    if value.chars().count() > CONTENT_MAX {
        errors.insert("content", "too_long");
        return None;
    }
    if value
        .chars()
        .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        errors.insert("content", "invalid");
        return None;
    }
    Some(value.to_owned())
}

pub fn limit(raw: Option<i64>, errors: &mut FieldErrors) -> Option<i64> {
    match raw {
        None => Some(LIMIT_DEFAULT),
        Some(n) if (1..=LIMIT_MAX).contains(&n) => Some(n),
        Some(_) => {
            errors.insert("limit", "invalid");
            None
        }
    }
}

/// Parses `before` / `after`. UUID first (a message id), otherwise RFC3339.
pub fn cursor(raw: Option<&str>, field: &'static str, errors: &mut FieldErrors) -> Option<Cursor> {
    let raw = raw.map(str::trim).filter(|s| !s.is_empty())?;
    if let Ok(id) = raw.parse::<Uuid>() {
        return Some(Cursor::Id(id));
    }
    match DateTime::parse_from_rfc3339(raw) {
        Ok(dt) => Some(Cursor::Time(dt.with_timezone(&Utc))),
        Err(_) => {
            errors.insert(field, "invalid");
            None
        }
    }
}

pub fn both_bounds_rejected(before: Option<Cursor>, after: Option<Cursor>) -> Result<(), ApiError> {
    if before.is_some() && after.is_some() {
        return Err(ApiError::Validation(FieldErrors::from([(
            "before", "invalid",
        )])));
    }
    Ok(())
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
    fn content_trims_and_caps() {
        assert_eq!(run(|e| content("  hi  ", e)).0.as_deref(), Some("hi"));
        assert_eq!(
            run(|e| content("   ", e)).1.get("content"),
            Some(&"required")
        );
        assert_eq!(
            run(|e| content(&"x".repeat(CONTENT_MAX + 1), e))
                .1
                .get("content"),
            Some(&"too_long")
        );
        assert_eq!(
            run(|e| content("ok\nline\t2", e)).0.as_deref(),
            Some("ok\nline\t2")
        );
        assert_eq!(
            run(|e| content("a\u{0000}b", e)).1.get("content"),
            Some(&"invalid")
        );
        assert_eq!(
            run(|e| content("win\r\nline", e)).0.as_deref(),
            Some("win\nline")
        );
    }

    #[test]
    fn limit_defaults_and_rejects() {
        assert_eq!(run(|e| limit(None, e)).0, Some(LIMIT_DEFAULT));
        assert_eq!(run(|e| limit(Some(1), e)).0, Some(1));
        assert_eq!(run(|e| limit(Some(LIMIT_MAX), e)).0, Some(LIMIT_MAX));
        assert_eq!(run(|e| limit(Some(0), e)).1.get("limit"), Some(&"invalid"));
        assert_eq!(
            run(|e| limit(Some(LIMIT_MAX + 1), e)).1.get("limit"),
            Some(&"invalid")
        );
    }

    #[test]
    fn cursor_accepts_uuid_or_rfc3339() {
        let id = Uuid::from_u128(42);
        assert_eq!(
            run(|e| cursor(Some(&id.to_string()), "before", e)).0,
            Some(Cursor::Id(id))
        );
        let (value, errors) = run(|e| cursor(Some("2026-09-13T18:00:00Z"), "before", e));
        assert!(errors.is_empty());
        match value {
            Some(Cursor::Time(t)) => assert_eq!(t.to_rfc3339(), "2026-09-13T18:00:00+00:00"),
            other => panic!("{other:?}"),
        }
        assert_eq!(run(|e| cursor(None, "before", e)).0, None);
        assert_eq!(
            run(|e| cursor(Some("yesterday"), "before", e))
                .1
                .get("before"),
            Some(&"invalid")
        );
    }
}
