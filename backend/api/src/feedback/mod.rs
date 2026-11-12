//! Feedback (#542): people tell the operators of the instance what they
//! notice, with a kind, a text, one optional image and the technical
//! details their client sends along, and read the operators' replies as a
//! small thread.
//!
//! The feature is on while `API_OPERATOR_EMAILS` names at least one address
//! ([`enabled`]). Off, `GET /api/feedback/status` says so and every other
//! feedback route answers `404 not_found`, as an unknown route would.
//!
//! Images are staged like avatars: `POST /api/feedback/images` stores the
//! object under `feedback/{user}/{file}` with a pending `storage_cleanup`
//! row, and creating the entry claims that row. The `feedback_image_cleanup`
//! trigger (0036) queues the object again once its entry goes.

pub mod notify;
pub mod person;

use std::time::SystemTime;

use axum::Router;
use axum::extract::FromRequestParts;
use axum::http::header::{CACHE_CONTROL, CONTENT_RANGE, RANGE, VARY};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use crate::auth::device::DeviceOs;
use crate::auth::validate;
use crate::error::{ApiError, FieldErrors};
use crate::state::AppState;
use crate::storage::{self, ObjectBody, StoreError};

/// Longest text of an entry or a reply (the columns' checks).
pub const TEXT_MAX: usize = 4000;
/// Largest image (the column's check). The upload route's body limit.
pub const IMAGE_MAX_BYTES: usize = 5 * 1024 * 1024;
/// Largest width and height an image may declare (`image_size::fits`).
pub const IMAGE_MAX_EDGE: u32 = 8192;
/// Most messages one thread holds; a reply past it is `text: too_many`.
pub const MESSAGES_MAX: i64 = 200;
/// Longest `app_version` (the column's check).
pub const APP_VERSION_MAX: usize = 32;
/// Longest `page` (the column's check).
pub const PAGE_MAX: usize = 300;

pub fn router() -> Router<AppState> {
    person::router()
}

/// Feedback is on while at least one operator is named.
pub fn enabled(state: &AppState) -> bool {
    !state.operators.is_empty()
}

/// Guard of every feedback route but the status: `404 not_found` while the
/// feature is off. Put it first, so that the answer is the same for anyone.
pub struct Enabled;

impl FromRequestParts<AppState> for Enabled {
    type Rejection = ApiError;

    async fn from_request_parts(_: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        if enabled(state) {
            Ok(Self)
        } else {
            Err(ApiError::NotFound)
        }
    }
}

/// What an entry is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Idea,
    Problem,
    Praise,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idea => "idea",
            Self::Problem => "problem",
            Self::Praise => "praise",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "idea" => Some(Self::Idea),
            "problem" => Some(Self::Problem),
            "praise" => Some(Self::Praise),
            _ => None,
        }
    }
}

/// `kind`, or `{"kind":"invalid"}`.
pub fn kind(raw: Option<&str>, errors: &mut FieldErrors) -> Option<Kind> {
    let kind = raw.and_then(Kind::parse);
    if kind.is_none() {
        errors.insert("kind", "invalid");
    }
    kind
}

/// The trimmed text of an entry, a reply or the form: 1 to [`TEXT_MAX`]
/// characters, no control characters but line breaks and tabs.
pub fn text(raw: Option<&str>, errors: &mut FieldErrors) -> Option<String> {
    let value = raw.unwrap_or_default().trim();
    if value.is_empty() {
        errors.insert("text", "required");
        None
    } else if value.chars().count() > TEXT_MAX {
        errors.insert("text", "too_long");
        None
    } else if validate::has_control(value) {
        errors.insert("text", "invalid");
        None
    } else {
        Some(value.to_owned())
    }
}

/// The technical details a client sends along, as the body carries them.
/// Strings, so that a bad value is a field error, not a `400`.
#[derive(Debug, Default, Deserialize)]
pub struct ContextBody {
    #[serde(default)]
    pub client: Option<String>,
    #[serde(default)]
    pub os: Option<String>,
    #[serde(default)]
    pub app_version: Option<String>,
    #[serde(default)]
    pub locale: Option<String>,
    #[serde(default)]
    pub page: Option<String>,
}

/// The technical details of an entry, and nothing else. `os`,
/// `app_version` and `page` are `null` only for the public form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Context {
    pub client: String,
    pub os: Option<String>,
    pub app_version: Option<String>,
    pub locale: String,
    pub page: Option<String>,
}

/// The context of a signed-in person's entry, every field checked; each
/// bad one is `invalid` under its own name.
pub fn context(body: Option<ContextBody>, errors: &mut FieldErrors) -> Option<Context> {
    let body = body.unwrap_or_default();
    let client = body
        .client
        .filter(|client| matches!(client.as_str(), "web" | "desktop"));
    if client.is_none() {
        errors.insert("client", "invalid");
    }
    let os = body.os.filter(|os| is_os(os));
    if os.is_none() {
        errors.insert("os", "invalid");
    }
    let app_version = body.app_version.filter(|version| is_app_version(version));
    if app_version.is_none() {
        errors.insert("app_version", "invalid");
    }
    let locale = body
        .locale
        .filter(|locale| matches!(locale.as_str(), "de" | "en"));
    if locale.is_none() {
        errors.insert("locale", "invalid");
    }
    let page = body.page.filter(|page| is_page(page));
    if page.is_none() {
        errors.insert("page", "invalid");
    }
    Some(Context {
        client: client?,
        os: Some(os?),
        app_version: Some(app_version?),
        locale: locale?,
        page: Some(page?),
    })
}

/// One of the codes of `auth::device::DeviceOs`, as its serde reads them.
fn is_os(raw: &str) -> bool {
    serde_json::from_value::<DeviceOs>(serde_json::Value::String(raw.to_owned())).is_ok()
}

/// `^[0-9A-Za-z.+-]{1,32}$`.
fn is_app_version(raw: &str) -> bool {
    !raw.is_empty()
        && raw.len() <= APP_VERSION_MAX
        && raw
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '+' | '-'))
}

/// A path of the client: starts with `/` but not `//`, at most
/// [`PAGE_MAX`] characters, without query, fragment, whitespace or control
/// characters (clients strip query and fragment before sending).
fn is_page(raw: &str) -> bool {
    raw.starts_with('/')
        && !raw.starts_with("//")
        && raw.chars().count() <= PAGE_MAX
        && !raw
            .chars()
            .any(|c| matches!(c, '?' | '#') || c.is_whitespace() || c.is_control())
}

/// The object key of a staged or attached image.
pub fn image_key(user: Uuid, file: Uuid) -> String {
    format!("feedback/{user}/{file}")
}

/// An entry's image as the clients link it.
#[derive(Debug, Clone, Serialize)]
pub struct Image {
    pub url: String,
    pub content_type: String,
    pub size: i32,
}

/// One message of a thread as the sender sees it: `from` is `you` or
/// `operators` (operators stay unnamed), `new` marks operator replies the
/// person has not seen yet.
#[derive(Debug, Clone, Serialize)]
pub struct Message {
    pub id: Uuid,
    pub from: &'static str,
    pub text: String,
    pub created_at: DateTime<Utc>,
    pub new: bool,
}

/// An entry as its sender sees it.
#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    pub id: Uuid,
    pub kind: String,
    pub status: String,
    pub text: String,
    pub created_at: DateTime<Utc>,
    pub last_activity_at: DateTime<Utc>,
    pub image: Option<Image>,
    pub context: Context,
    pub unread: bool,
    pub messages: Vec<Message>,
}

/// The columns of [`EntryRow`], for `SELECT … FROM feedback`.
macro_rules! entry_columns {
    () => {
        "id, kind, status, body, created_at, last_activity_at, image_type, image_bytes, \
         client, os, app_version, locale, page, person_unread_since"
    };
}
pub(crate) use entry_columns;

#[derive(Debug, Clone, FromRow)]
pub(crate) struct EntryRow {
    pub id: Uuid,
    pub kind: String,
    pub status: String,
    pub body: String,
    pub created_at: DateTime<Utc>,
    pub last_activity_at: DateTime<Utc>,
    pub image_type: Option<String>,
    pub image_bytes: Option<i32>,
    pub client: String,
    pub os: Option<String>,
    pub app_version: Option<String>,
    pub locale: String,
    pub page: Option<String>,
    pub person_unread_since: Option<DateTime<Utc>>,
}

/// The columns of [`MessageRow`], for `SELECT … FROM feedback_messages`.
macro_rules! message_columns {
    () => {
        "id, feedback_id, author_kind, body, created_at"
    };
}
pub(crate) use message_columns;

#[derive(Debug, Clone, FromRow)]
pub(crate) struct MessageRow {
    pub id: Uuid,
    pub feedback_id: Uuid,
    pub author_kind: String,
    pub body: String,
    pub created_at: DateTime<Utc>,
}

impl MessageRow {
    /// The message as the sender sees it, against the entry's unread mark.
    pub(crate) fn for_person(self, unread_since: Option<DateTime<Utc>>) -> Message {
        let operator = self.author_kind == "operator";
        Message {
            id: self.id,
            from: if operator { "operators" } else { "you" },
            new: operator && unread_since.is_some_and(|since| self.created_at >= since),
            text: self.body,
            created_at: self.created_at,
        }
    }
}

impl EntryRow {
    pub(crate) fn context(&self) -> Context {
        Context {
            client: self.client.clone(),
            os: self.os.clone(),
            app_version: self.app_version.clone(),
            locale: self.locale.clone(),
            page: self.page.clone(),
        }
    }

    pub(crate) fn image(&self, url: String) -> Option<Image> {
        Some(Image {
            url,
            content_type: self.image_type.clone()?,
            size: self.image_bytes?,
        })
    }

    /// The entry as its sender sees it, with `messages` of this entry in
    /// thread order.
    pub(crate) fn for_person(self, messages: Vec<MessageRow>) -> Entry {
        let unread_since = self.person_unread_since;
        Entry {
            image: self.image(format!("/api/feedback/{}/image", self.id)),
            context: self.context(),
            id: self.id,
            kind: self.kind,
            status: self.status,
            text: self.body,
            created_at: self.created_at,
            last_activity_at: self.last_activity_at,
            unread: unread_since.is_some(),
            messages: messages
                .into_iter()
                .map(|message| message.for_person(unread_since))
                .collect(),
        }
    }
}

/// The image behind `key` for the browser: a redirect to MinIO or the
/// bytes, by `ATTACHMENT_DELIVERY`, as `account::download_export` serves
/// the archive. Shown inline; the key never changes its content.
pub(crate) async fn serve_image(
    state: &AppState,
    key: &str,
    content_type: &str,
    size: i32,
    request_headers: &HeaderMap,
) -> Result<Response, ApiError> {
    let range = request_headers
        .get(RANGE)
        .and_then(|value| value.to_str().ok());
    let now = SystemTime::now();
    let body = state
        .store
        .serve(key, content_type, Some("inline"), range, now)
        .await;
    match body {
        Ok(ObjectBody::Redirect(url)) => {
            let mut response = Redirect::temporary(&url).into_response();
            let headers = response.headers_mut();
            headers.insert(VARY, HeaderValue::from_static("Cookie"));
            // The signed link is the same for the rest of its window.
            let (_, left) = storage::get_window(now);
            if let Ok(value) = HeaderValue::from_str(&format!("private, max-age={left}")) {
                headers.insert(CACHE_CONTROL, value);
            }
            Ok(response)
        }
        Ok(ObjectBody::Stream(object)) => {
            let mut response = crate::attachments::streamed("inline", content_type, false, object);
            response.headers_mut().insert(
                CACHE_CONTROL,
                HeaderValue::from_static("private, max-age=31536000, immutable"),
            );
            Ok(response)
        }
        Err(StoreError::NotFound) => Err(ApiError::NotFound),
        Err(StoreError::RangeNotSatisfiable) => {
            let mut response = StatusCode::RANGE_NOT_SATISFIABLE.into_response();
            if let Ok(value) = HeaderValue::from_str(&format!("bytes */{size}")) {
                response.headers_mut().insert(CONTENT_RANGE, value);
            }
            Ok(response)
        }
        Err(err) => Err(ApiError::Internal(format!("feedback image: {err}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context_of(body: ContextBody) -> (Option<Context>, FieldErrors) {
        let mut errors = FieldErrors::new();
        let context = context(Some(body), &mut errors);
        (context, errors)
    }

    fn good() -> ContextBody {
        ContextBody {
            client: Some("web".into()),
            os: Some("linux".into()),
            app_version: Some("0.10.0".into()),
            locale: Some("de".into()),
            page: Some("/s/abc/c/def".into()),
        }
    }

    #[test]
    fn a_full_context_passes() {
        let (context, errors) = context_of(good());
        assert!(errors.is_empty(), "{errors:?}");
        let context = context.expect("context");
        assert_eq!(context.client, "web");
        assert_eq!(context.os.as_deref(), Some("linux"));
        assert_eq!(context.page.as_deref(), Some("/s/abc/c/def"));
        let (_, errors) = context_of(ContextBody {
            client: Some("desktop".into()),
            ..good()
        });
        assert!(errors.is_empty());
    }

    #[test]
    fn os_takes_the_device_codes() {
        for os in ["linux", "windows", "macos", "android", "ios", "other"] {
            assert!(is_os(os), "{os}");
        }
        for os in ["", "Linux", "macOS", "bsd", " linux"] {
            assert!(!is_os(os), "{os}");
        }
    }

    #[test]
    fn app_version_is_short_and_plain() {
        for version in ["0.10.0", "0.10.0-dev+abc", "1", &"9".repeat(32)] {
            assert!(is_app_version(version), "{version}");
        }
        for version in ["", "0.10 .0", "v0.10/0", "0.10.0_1", &"9".repeat(33), "ü"] {
            assert!(!is_app_version(version), "{version}");
        }
    }

    #[test]
    fn page_is_a_bare_path() {
        for page in [
            "/",
            "/settings/darstellung",
            &format!("/{}", "a".repeat(299)),
        ] {
            assert!(is_page(page), "{page}");
        }
        for page in [
            "",
            "settings",
            "//evil.example/x",
            "/s?x=1",
            "/s#frag",
            "/a b",
            "/a\tb",
            "/a\u{0}",
            &format!("/{}", "a".repeat(300)),
        ] {
            assert!(!is_page(page), "{page:?}");
        }
    }

    #[test]
    fn every_bad_field_is_named() {
        let (context, errors) = context_of(ContextBody {
            client: Some("form".into()),
            os: Some("beos".into()),
            app_version: Some("".into()),
            locale: Some("fr".into()),
            page: Some("/x?y".into()),
        });
        assert!(context.is_none());
        for field in ["client", "os", "app_version", "locale", "page"] {
            assert_eq!(errors.get(field), Some(&"invalid"), "{field}");
        }
        let mut errors = FieldErrors::new();
        assert!(super::context(None, &mut errors).is_none());
        assert_eq!(errors.len(), 5);
    }

    #[test]
    fn text_is_trimmed_and_bounded() {
        let check = |raw: Option<&str>| {
            let mut errors = FieldErrors::new();
            let value = text(raw, &mut errors);
            (value, errors.get("text").copied())
        };
        assert_eq!(
            check(Some("  hallo\nwelt ")),
            (Some("hallo\nwelt".into()), None)
        );
        assert_eq!(check(None), (None, Some("required")));
        assert_eq!(check(Some(" \n ")), (None, Some("required")));
        let longest = "ä".repeat(TEXT_MAX);
        assert_eq!(check(Some(&longest)).1, None);
        assert_eq!(
            check(Some(&format!("{longest}x"))),
            (None, Some("too_long"))
        );
        assert_eq!(check(Some("a\u{0}b")), (None, Some("invalid")));
        assert_eq!(check(Some("a\u{1b}b")), (None, Some("invalid")));
    }

    #[test]
    fn kinds_are_the_three() {
        for raw in ["idea", "problem", "praise"] {
            assert_eq!(Kind::parse(raw).map(Kind::as_str), Some(raw));
        }
        let mut errors = FieldErrors::new();
        assert!(kind(Some("bug"), &mut errors).is_none());
        assert_eq!(errors.get("kind"), Some(&"invalid"));
    }
}
