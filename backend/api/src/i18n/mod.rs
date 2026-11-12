//! Texts the server itself puts in front of people (issues #314, #515): push
//! titles and bodies, account mail and a new server's default channels, in
//! the account's [`Locale`] (`users.locale`, migration 0030). Messages in the
//! app are rendered by the client; a push or a mail has no client code to
//! run, so its words come from here. One file per area; later areas add
//! their own.
//!
//! Error messages (`ApiError::message`) stay an English fallback: clients
//! switch on the code.

pub mod account;
pub mod feedback;
pub mod mail;
pub mod push;
pub mod servers;

pub use push::*;
pub use servers::{DefaultChannels, default_channels};

use axum::http::HeaderValue;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::ApiError;

/// The language an account reads: `de` or `en`. Existing accounts and
/// clients older than v0.10 are German.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "lowercase")]
#[sqlx(type_name = "text", rename_all = "lowercase")]
pub enum Locale {
    #[default]
    De,
    En,
}

impl Locale {
    /// `"de"` or `"en"`, exactly; anything else is `None`.
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "de" => Some(Self::De),
            "en" => Some(Self::En),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::De => "de",
            Self::En => "en",
        }
    }

    /// The language an `Accept-Language` header asks for. Ranges are walked
    /// by descending `q` (ties keep their order); the first whose primary
    /// subtag is `de` or `en` wins. `*`, ranges with `q=0` and other
    /// languages are skipped, but a header that names only other languages
    /// gets English. `None` when the header has no usable range at all.
    pub fn from_accept_language(header: &HeaderValue) -> Option<Self> {
        let raw = header.to_str().ok()?;
        let mut ranges: Vec<(&str, f32)> = raw
            .split(',')
            .filter_map(|item| {
                let mut parts = item.split(';');
                let tag = parts.next()?.trim();
                if tag.is_empty() || tag == "*" {
                    return None;
                }
                let mut q = 1.0_f32;
                for param in parts {
                    if let Some((key, value)) = param.split_once('=')
                        && key.trim().eq_ignore_ascii_case("q")
                    {
                        q = value.trim().parse().ok()?;
                    }
                }
                (q > 0.0).then_some((tag, q))
            })
            .collect();
        if ranges.is_empty() {
            return None;
        }
        ranges.sort_by(|a, b| b.1.total_cmp(&a.1));
        let found = ranges.iter().find_map(|(tag, _)| {
            let primary = tag.split(['-', '_']).next().unwrap_or_default();
            if primary.eq_ignore_ascii_case("de") {
                Some(Self::De)
            } else if primary.eq_ignore_ascii_case("en") {
                Some(Self::En)
            } else {
                None
            }
        });
        Some(found.unwrap_or(Self::En))
    }
}

/// The language `user` reads (`users.locale`); German for a row that is gone.
pub async fn of_recipient(db: &PgPool, user: Uuid) -> Result<Locale, ApiError> {
    Ok(
        sqlx::query_scalar::<_, Locale>("SELECT locale FROM users WHERE id = $1")
            .bind(user)
            .fetch_optional(db)
            .await?
            .unwrap_or_default(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn accept(raw: &str) -> Option<Locale> {
        Locale::from_accept_language(&HeaderValue::from_str(raw).unwrap())
    }

    #[test]
    fn parses_exactly_de_and_en() {
        assert_eq!(Locale::parse("de"), Some(Locale::De));
        assert_eq!(Locale::parse("en"), Some(Locale::En));
        for raw in ["", "fr", "EN", "en-GB", " de"] {
            assert_eq!(Locale::parse(raw), None, "{raw:?}");
        }
        assert_eq!(Locale::De.as_str(), "de");
        assert_eq!(Locale::En.as_str(), "en");
        assert_eq!(serde_json::to_string(&Locale::En).unwrap(), "\"en\"");
        assert_eq!(
            serde_json::from_str::<Locale>("\"de\"").unwrap(),
            Locale::De
        );
    }

    #[test]
    fn accept_language_takes_the_first_supported_range_by_q() {
        assert_eq!(accept("de-DE,de;q=0.9,en;q=0.8"), Some(Locale::De));
        assert_eq!(accept("en-US"), Some(Locale::En));
        assert_eq!(accept("en-GB;q=0.5, de-AT;q=0.7"), Some(Locale::De));
        assert_eq!(accept("fr-FR, en;q=0.4, de;q=0.3"), Some(Locale::En));
        assert_eq!(accept("de;q=0.2, *;q=0.9, en;q=0.5"), Some(Locale::En));
        assert_eq!(accept("DE-ch"), Some(Locale::De));
        // A range the client refuses does not count.
        assert_eq!(accept("de;q=0, en;q=0.1"), Some(Locale::En));
    }

    #[test]
    fn accept_language_falls_back_to_english_or_nothing() {
        assert_eq!(accept("fr"), Some(Locale::En));
        assert_eq!(accept("fr-FR, it;q=0.5"), Some(Locale::En));
        assert_eq!(accept("*"), None);
        assert_eq!(accept(""), None);
        assert_eq!(accept(" , ;q=0.5"), None);
        assert_eq!(accept("de;q=0"), None);
    }
}
