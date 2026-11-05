//! Input rules for server, category, channel and invite bodies. Same field
//! codes as `auth::validate`; mirrored in `web/src/servers/rules.ts`.

use uuid::Uuid;

pub use crate::auth::validate::finish;
use crate::error::FieldErrors;

use super::channel::ChannelKind;
use super::permissions::Permissions;

pub const NAME_MAX: usize = 100;
pub const INVITE_MAX_USES_MAX: i32 = 10_000;
pub const INVITE_EXPIRES_HOURS_MAX: i64 = 24 * 365;

/// Server and category names: trimmed, 1–100 characters, no control chars.
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

/// Channel names. Text channels are normalised the way people expect from
/// `#channel` mentions: lower-cased, whitespace and underscores become `-`,
/// runs of `-` collapse, no leading/trailing `-`. Voice channels keep their
/// display name as typed.
pub fn channel_name(raw: &str, kind: ChannelKind, errors: &mut FieldErrors) -> Option<String> {
    let value = name(raw, errors)?;
    if kind == ChannelKind::Voice {
        return Some(value);
    }
    let mut out = String::with_capacity(value.len());
    let mut dash = true; // suppress a leading dash
    for c in value.chars() {
        if c.is_whitespace() || c == '_' || c == '-' {
            if !dash {
                out.push('-');
                dash = true;
            }
        } else if c == '#' {
            continue;
        } else {
            for lower in c.to_lowercase() {
                out.push(lower);
            }
            dash = false;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        errors.insert("name", "invalid");
        return None;
    }
    Some(out)
}

pub fn kind(raw: Option<&str>, errors: &mut FieldErrors) -> Option<ChannelKind> {
    match raw {
        None => Some(ChannelKind::Text),
        Some(raw) => match ChannelKind::from_name(raw) {
            // DMs are opened via `/api/dms`, not as a server channel kind.
            Some(kind) if kind != ChannelKind::Dm => Some(kind),
            Some(_) | None => {
                errors.insert("kind", "invalid");
                None
            }
        },
    }
}

/// `None` = field absent (leave as is); `Some(None)` = explicitly empty, i.e.
/// "no category"; `Some(Some(id))` = move into that category. Whether the id
/// belongs to the same server is checked against the database later.
pub fn category_id(raw: Option<&str>, errors: &mut FieldErrors) -> Option<Option<Uuid>> {
    match raw.map(str::trim) {
        None => None,
        Some("") => Some(None),
        Some(value) => match value.parse::<Uuid>() {
            Ok(id) => Some(Some(id)),
            Err(_) => {
                errors.insert("category_id", "invalid");
                None
            }
        },
    }
}

pub fn permissions(names: &[String], errors: &mut FieldErrors) -> Option<Permissions> {
    match Permissions::from_names(names) {
        Ok(set) => Some(set),
        Err(_) => {
            errors.insert("member_permissions", "invalid");
            None
        }
    }
}

/// `None` = unlimited.
pub fn max_uses(raw: Option<i64>, errors: &mut FieldErrors) -> Option<Option<i32>> {
    match raw {
        None => Some(None),
        Some(n) if (1..=i64::from(INVITE_MAX_USES_MAX)).contains(&n) => Some(Some(n as i32)),
        Some(_) => {
            errors.insert("max_uses", "invalid");
            None
        }
    }
}

/// `None` = never expires.
pub fn expires_in_hours(raw: Option<i64>, errors: &mut FieldErrors) -> Option<Option<i64>> {
    match raw {
        None => Some(None),
        Some(h) if (1..=INVITE_EXPIRES_HOURS_MAX).contains(&h) => Some(Some(h)),
        Some(_) => {
            errors.insert("expires_in_hours", "invalid");
            None
        }
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
    fn server_name_rules() {
        assert_eq!(run(|e| name("  Team  ", e)).0.as_deref(), Some("Team"));
        assert_eq!(run(|e| name("   ", e)).1.get("name"), Some(&"required"));
        assert_eq!(run(|e| name("a\tb", e)).1.get("name"), Some(&"invalid"));
        assert_eq!(
            run(|e| name(&"x".repeat(NAME_MAX + 1), e)).1.get("name"),
            Some(&"too_long")
        );
    }

    #[test]
    fn text_channel_names_are_slugified() {
        for (raw, expected) in [
            ("General", "general"),
            ("  Off Topic ", "off-topic"),
            ("#dev__chat", "dev-chat"),
            ("--a--b--", "a-b"),
            ("Ünïcode Ok", "ünïcode-ok"),
        ] {
            let (value, errors) = run(|e| channel_name(raw, ChannelKind::Text, e));
            assert_eq!(value.as_deref(), Some(expected), "{raw:?}");
            assert!(errors.is_empty(), "{raw:?}");
        }
        let (value, errors) = run(|e| channel_name("###", ChannelKind::Text, e));
        assert!(value.is_none());
        assert_eq!(errors.get("name"), Some(&"invalid"));
    }

    #[test]
    fn voice_channel_names_keep_case_and_spaces() {
        assert_eq!(
            run(|e| channel_name(" Lounge Zwei ", ChannelKind::Voice, e))
                .0
                .as_deref(),
            Some("Lounge Zwei")
        );
    }

    #[test]
    fn kind_defaults_to_text() {
        assert_eq!(run(|e| kind(None, e)).0, Some(ChannelKind::Text));
        assert_eq!(run(|e| kind(Some("voice"), e)).0, Some(ChannelKind::Voice));
        assert_eq!(
            run(|e| kind(Some("stage"), e)).1.get("kind"),
            Some(&"invalid")
        );
        assert_eq!(run(|e| kind(Some("dm"), e)).1.get("kind"), Some(&"invalid"));
    }

    #[test]
    fn category_id_distinguishes_absent_empty_and_set() {
        assert_eq!(run(|e| category_id(None, e)).0, None);
        assert_eq!(run(|e| category_id(Some(""), e)).0, Some(None));
        let id = Uuid::from_u128(42);
        assert_eq!(
            run(|e| category_id(Some(&id.to_string()), e)).0,
            Some(Some(id))
        );
        assert_eq!(
            run(|e| category_id(Some("nope"), e)).1.get("category_id"),
            Some(&"invalid")
        );
    }

    #[test]
    fn invite_limits() {
        assert_eq!(run(|e| max_uses(None, e)).0, Some(None));
        assert_eq!(run(|e| max_uses(Some(5), e)).0, Some(Some(5)));
        assert_eq!(
            run(|e| max_uses(Some(0), e)).1.get("max_uses"),
            Some(&"invalid")
        );
        assert_eq!(run(|e| expires_in_hours(Some(24), e)).0, Some(Some(24)));
        assert_eq!(
            run(|e| expires_in_hours(Some(-1), e))
                .1
                .get("expires_in_hours"),
            Some(&"invalid")
        );
    }
}
