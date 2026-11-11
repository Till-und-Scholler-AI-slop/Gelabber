//! Input rules for the directory: the listing form, the join request
//! message and the search. Each rule writes a field code (`required`,
//! `too_long`, `invalid`) into the caller's [`FieldErrors`], which
//! `auth::validate::finish` turns into `422 validation_failed`; the web
//! client mirrors them in `discover/rules.ts`.

use serde::Serialize;

use super::languages;
use crate::error::FieldErrors;

/// Longest listing description (the column's check).
pub const DESCRIPTION_MAX: usize = 300;
/// Longest message of a join request (the column's check).
pub const MESSAGE_MAX: usize = 300;
/// Longest search text.
pub const QUERY_MAX: usize = 100;

/// How a person gets in through the directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum JoinMode {
    /// Anyone joins at once.
    Open,
    /// "Beitritt anfragen": the owner or a moderator accepts a request.
    Request,
}

impl JoinMode {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "open" => Some(Self::Open),
            "request" => Some(Self::Request),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Request => "request",
        }
    }
}

/// One line of text a person typed: trimmed, no control characters (which
/// includes line breaks; Postgres would also refuse NUL with a 500), at
/// most `max` characters.
fn line(field: &'static str, raw: &str, max: usize, errors: &mut FieldErrors) -> Option<String> {
    let value = raw.trim();
    if value.chars().any(char::is_control) {
        errors.insert(field, "invalid");
        return None;
    }
    if value.chars().count() > max {
        errors.insert(field, "too_long");
        return None;
    }
    Some(value.to_owned())
}

/// The listing description: one line of at most [`DESCRIPTION_MAX`]
/// characters, required while the server is `listed`. A missing field is
/// `required` either way, since the form always sends it.
pub fn description(raw: Option<&str>, listed: bool, errors: &mut FieldErrors) -> Option<String> {
    let Some(raw) = raw else {
        errors.insert("description", "required");
        return None;
    };
    let value = line("description", raw, DESCRIPTION_MAX, errors)?;
    if listed && value.is_empty() {
        errors.insert("description", "required");
        return None;
    }
    Some(value)
}

/// A code from [`languages::LANGUAGES`].
pub fn language(raw: Option<&str>, errors: &mut FieldErrors) -> Option<&'static str> {
    let Some(raw) = raw.filter(|raw| !raw.is_empty()) else {
        errors.insert("language", "required");
        return None;
    };
    let found = languages::find(raw);
    if found.is_none() {
        errors.insert("language", "invalid");
    }
    found
}

/// `open` or `request`.
pub fn join_mode(raw: Option<&str>, errors: &mut FieldErrors) -> Option<JoinMode> {
    let Some(raw) = raw.filter(|raw| !raw.is_empty()) else {
        errors.insert("join_mode", "required");
        return None;
    };
    let found = JoinMode::parse(raw);
    if found.is_none() {
        errors.insert("join_mode", "invalid");
    }
    found
}

/// The optional message of a join request: one line of at most
/// [`MESSAGE_MAX`] characters; absent is empty.
pub fn message(raw: Option<&str>, errors: &mut FieldErrors) -> Option<String> {
    line("message", raw.unwrap_or_default(), MESSAGE_MAX, errors)
}

/// The search text `q`: trimmed, at most [`QUERY_MAX`] characters, no
/// control characters. Absent or blank is no search (`None`, no error).
pub fn query(raw: Option<&str>, errors: &mut FieldErrors) -> Option<String> {
    line("q", raw.unwrap_or_default(), QUERY_MAX, errors).filter(|q| !q.is_empty())
}

/// `text` as a literal inside an `ILIKE … ESCAPE '\'` pattern: `\`, `%` and
/// `_` are escaped. The caller adds the surrounding `%`.
pub fn like_pattern(text: &str) -> String {
    let mut pattern = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, '\\' | '%' | '_') {
            pattern.push('\\');
        }
        pattern.push(c);
    }
    pattern
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run<T>(f: impl FnOnce(&mut FieldErrors) -> Option<T>) -> (Option<T>, FieldErrors) {
        let mut errors = FieldErrors::new();
        let value = f(&mut errors);
        (value, errors)
    }

    fn code<T>(field: &str, f: impl FnOnce(&mut FieldErrors) -> Option<T>) -> Option<&'static str> {
        run(f).1.get(field).copied()
    }

    #[test]
    fn like_pattern_escapes_the_wildcards_and_the_escape() {
        assert_eq!(like_pattern("rust"), "rust");
        assert_eq!(like_pattern("100%"), "100\\%");
        assert_eq!(like_pattern("a_b"), "a\\_b");
        assert_eq!(like_pattern("c:\\x"), "c:\\\\x");
        assert_eq!(like_pattern("%_\\"), "\\%\\_\\\\");
        assert_eq!(like_pattern("Größe"), "Größe");
    }

    #[test]
    fn description_is_one_trimmed_line_required_while_listed() {
        let (value, errors) = run(|e| description(Some("  Jeden Freitag  "), true, e));
        assert_eq!(value.as_deref(), Some("Jeden Freitag"));
        assert!(errors.is_empty());
        // Empty is fine for an unlisted server, not for a listed one.
        assert_eq!(
            run(|e| description(Some("   "), false, e)).0.as_deref(),
            Some("")
        );
        assert_eq!(
            code("description", |e| description(Some(" "), true, e)),
            Some("required")
        );
        assert_eq!(
            code("description", |e| description(None, false, e)),
            Some("required")
        );
        let max = "ä".repeat(DESCRIPTION_MAX);
        assert!(run(|e| description(Some(&max), true, e)).1.is_empty());
        let long = "ä".repeat(DESCRIPTION_MAX + 1);
        assert_eq!(
            code("description", |e| description(Some(&long), true, e)),
            Some("too_long")
        );
        for bad in ["zwei\nZeilen", "tab\there", "nul\0"] {
            assert_eq!(
                code("description", |e| description(Some(bad), false, e)),
                Some("invalid"),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn language_must_be_listed() {
        assert_eq!(run(|e| language(Some("de"), e)).0, Some("de"));
        assert_eq!(code("language", |e| language(None, e)), Some("required"));
        assert_eq!(
            code("language", |e| language(Some(""), e)),
            Some("required")
        );
        for bad in ["xx", "DE", "deu", " de"] {
            assert_eq!(
                code("language", |e| language(Some(bad), e)),
                Some("invalid"),
                "{bad}"
            );
        }
    }

    #[test]
    fn join_mode_is_open_or_request() {
        assert_eq!(run(|e| join_mode(Some("open"), e)).0, Some(JoinMode::Open));
        assert_eq!(
            run(|e| join_mode(Some("request"), e)).0,
            Some(JoinMode::Request)
        );
        assert_eq!(code("join_mode", |e| join_mode(None, e)), Some("required"));
        assert_eq!(
            code("join_mode", |e| join_mode(Some("closed"), e)),
            Some("invalid")
        );
        assert_eq!(JoinMode::Open.as_str(), "open");
        assert_eq!(serde_json::to_value(JoinMode::Request).unwrap(), "request");
    }

    #[test]
    fn message_is_an_optional_line() {
        assert_eq!(run(|e| message(None, e)).0.as_deref(), Some(""));
        assert_eq!(
            run(|e| message(Some(" Hallo! "), e)).0.as_deref(),
            Some("Hallo!")
        );
        let long = "x".repeat(MESSAGE_MAX + 1);
        assert_eq!(
            code("message", |e| message(Some(&long), e)),
            Some("too_long")
        );
        assert_eq!(
            code("message", |e| message(Some("a\nb"), e)),
            Some("invalid")
        );
    }

    #[test]
    fn query_is_trimmed_and_blank_means_none() {
        assert_eq!(
            run(|e| query(Some("  rust "), e)).0.as_deref(),
            Some("rust")
        );
        let (value, errors) = run(|e| query(Some("   "), e));
        assert_eq!(value, None);
        assert!(errors.is_empty());
        assert_eq!(run(|e| query(None, e)), (None, FieldErrors::new()));
        assert!(run(|e| query(Some(&"q".repeat(QUERY_MAX)), e)).1.is_empty());
        let long = "q".repeat(QUERY_MAX + 1);
        assert_eq!(code("q", |e| query(Some(&long), e)), Some("too_long"));
        assert_eq!(code("q", |e| query(Some("a\0"), e)), Some("invalid"));
    }
}
