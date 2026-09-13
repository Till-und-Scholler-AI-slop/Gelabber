//! Type and size limits for chat attachments. Field codes match the rest
//! of the API; the client mirrors them in `web/src/messages/rules.ts`.

pub use crate::auth::validate::finish;
use crate::error::FieldErrors;

/// Locked for v1: the ticket named 8–25 MB; 25 MiB is the ceiling.
pub const SIZE_MAX: i64 = 25 * 1024 * 1024;
pub const FILENAME_MAX: usize = 255;
pub const ATTACHMENTS_MAX: usize = 1;

pub const ALLOWED_TYPES: &[&str] = &[
    "image/jpeg",
    "image/png",
    "image/gif",
    "image/webp",
    "application/pdf",
    "text/plain",
    "application/zip",
    "audio/mpeg",
    "audio/wav",
    "video/mp4",
];

pub fn is_image(content_type: &str) -> bool {
    matches!(
        content_type,
        "image/jpeg" | "image/png" | "image/gif" | "image/webp"
    )
}

pub fn filename(raw: &str, errors: &mut FieldErrors) -> Option<String> {
    let trimmed = raw.trim();
    let name = trimmed.rsplit(['/', '\\']).next().unwrap_or(trimmed).trim();
    if name.is_empty() || name == "." || name == ".." {
        errors.insert("filename", "required");
        return None;
    }
    if name.chars().count() > FILENAME_MAX {
        errors.insert("filename", "too_long");
        return None;
    }
    if name.chars().any(|c| c.is_control() || c == '\0') {
        errors.insert("filename", "invalid");
        return None;
    }
    Some(name.to_owned())
}

pub fn content_type(raw: &str, errors: &mut FieldErrors) -> Option<String> {
    let value = raw.trim().to_ascii_lowercase();
    let value = value.split(';').next().unwrap_or("").trim().to_owned();
    if value.is_empty() {
        errors.insert("content_type", "required");
        return None;
    }
    if !ALLOWED_TYPES.contains(&value.as_str()) {
        errors.insert("content_type", "invalid");
        return None;
    }
    Some(value)
}

pub fn size(raw: i64, errors: &mut FieldErrors) -> Option<i64> {
    if raw <= 0 {
        errors.insert("size", "required");
        return None;
    }
    if raw > SIZE_MAX {
        errors.insert("size", "too_long");
        return None;
    }
    Some(raw)
}

pub fn attachment_ids(raw: &[uuid::Uuid], errors: &mut FieldErrors) -> Option<Vec<uuid::Uuid>> {
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::with_capacity(raw.len());
    for id in raw {
        if !seen.insert(*id) {
            errors.insert("attachment_ids", "invalid");
            return None;
        }
        out.push(*id);
    }
    if out.len() > ATTACHMENTS_MAX {
        errors.insert("attachment_ids", "too_long");
        return None;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn run<T>(f: impl FnOnce(&mut FieldErrors) -> Option<T>) -> (Option<T>, FieldErrors) {
        let mut errors = FieldErrors::new();
        let value = f(&mut errors);
        (value, errors)
    }

    #[test]
    fn filename_strips_paths_and_rejects_junk() {
        assert_eq!(
            run(|e| filename("  photos/cat.png  ", e)).0.as_deref(),
            Some("cat.png")
        );
        assert_eq!(
            run(|e| filename("..", e)).1.get("filename"),
            Some(&"required")
        );
        assert_eq!(
            run(|e| filename(&"x".repeat(FILENAME_MAX + 1), e))
                .1
                .get("filename"),
            Some(&"too_long")
        );
        assert_eq!(
            run(|e| filename("a\0b.jpg", e)).1.get("filename"),
            Some(&"invalid")
        );
    }

    #[test]
    fn content_type_allowlist_and_params() {
        assert_eq!(
            run(|e| content_type("image/PNG; charset=binary", e))
                .0
                .as_deref(),
            Some("image/png")
        );
        assert_eq!(
            run(|e| content_type("application/x-msdownload", e))
                .1
                .get("content_type"),
            Some(&"invalid")
        );
        assert_eq!(
            run(|e| content_type("  ", e)).1.get("content_type"),
            Some(&"required")
        );
    }

    #[test]
    fn size_bounds() {
        assert_eq!(run(|e| size(1, e)).0, Some(1));
        assert_eq!(run(|e| size(SIZE_MAX, e)).0, Some(SIZE_MAX));
        assert_eq!(run(|e| size(0, e)).1.get("size"), Some(&"required"));
        assert_eq!(
            run(|e| size(SIZE_MAX + 1, e)).1.get("size"),
            Some(&"too_long")
        );
    }

    #[test]
    fn attachment_ids_cap_and_dedup() {
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        assert_eq!(
            run(|e| attachment_ids(&[a], e)).0.as_deref(),
            Some(&[a][..])
        );
        assert_eq!(
            run(|e| attachment_ids(&[a, b], e)).1.get("attachment_ids"),
            Some(&"too_long")
        );
        assert_eq!(
            run(|e| attachment_ids(&[a, a], e)).1.get("attachment_ids"),
            Some(&"invalid")
        );
    }
}
