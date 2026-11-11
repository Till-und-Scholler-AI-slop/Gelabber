//! Sign-in through OpenID Connect (#518). Off unless the operator sets a
//! provider's client ID and secret ([`crate::config::OidcConfig`]).
//!
//! This module talks to the provider: discovery (cached 24 h), the JWKS
//! (cached 1 h) and the token endpoint. Every failure here is
//! [`ApiError::OidcUnavailable`], whose detail is logged and never returned;
//! the detail never holds a code, token, verifier, nonce or address.

pub mod flow;
pub mod id_token;
pub mod link;
pub mod pkce;
pub mod routes;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use reqwest::header::{ACCEPT, CONTENT_TYPE};
use serde::Deserialize;
use url::Url;

use crate::config::{GOOGLE_ISSUER, OidcConfig, OidcProvider, is_loopback_url};
use crate::error::ApiError;

pub use id_token::Claims;

/// How long a discovery document is trusted.
pub const DISCOVERY_TTL: Duration = Duration::from_secs(24 * 3600);
/// How long a key set is trusted.
pub const JWKS_TTL: Duration = Duration::from_secs(3600);
/// An unknown `kid` refetches the key set at most this often.
pub const JWKS_REFETCH_MIN: Duration = Duration::from_secs(60);
/// Discovery, JWKS and token answers are small; anything larger is refused.
const BODY_MAX: u64 = 256 * 1024;

/// The endpoints of a provider's discovery document, already checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discovery {
    /// As the document spells it; `id_token`s must carry exactly this `iss`.
    pub issuer: String,
    pub authorization_endpoint: Url,
    pub token_endpoint: Url,
    pub jwks_uri: Url,
}

#[derive(Debug)]
struct KeySet {
    keys: Arc<Vec<id_token::Key>>,
    fetched: Instant,
}

/// The configured providers plus the HTTP client that talks to them and
/// the caches of their discovery documents and keys.
/// `AppState.oidc` holds it; tests swap it like `state.mail`.
#[derive(Debug)]
pub struct Oidc {
    pub config: OidcConfig,
    pub http: reqwest::Client,
    discovery: Mutex<HashMap<&'static str, (Instant, Arc<Discovery>)>>,
    jwks: Mutex<HashMap<&'static str, KeySet>>,
}

impl Oidc {
    pub fn new(config: &OidcConfig) -> Result<Self, String> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(10))
            // The token endpoint answers itself; a redirect could lead anywhere.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|err| format!("oidc client: {err}"))?;
        Ok(Self {
            config: config.clone(),
            http,
            discovery: Mutex::default(),
            jwks: Mutex::default(),
        })
    }

    /// A configured provider by its route id (`google`).
    pub fn provider(&self, id: &str) -> Option<&OidcProvider> {
        self.config
            .providers
            .iter()
            .find(|provider| provider.id == id)
    }

    /// `{APP_PUBLIC_URL}/api/auth/oidc/{id}/callback`, the one redirect URI
    /// registered with the provider.
    pub fn redirect_uri(&self, provider: &OidcProvider) -> String {
        format!(
            "{}/api/auth/oidc/{}/callback",
            self.config.public_url.as_str().trim_end_matches('/'),
            provider.id
        )
    }

    /// The provider's discovery document, fetched on first use and then
    /// cached for [`DISCOVERY_TTL`]. A failed fetch is not cached.
    pub async fn discovery(&self, provider: &OidcProvider) -> Result<Arc<Discovery>, ApiError> {
        if let Some((fetched, doc)) = lock(&self.discovery).get(provider.id)
            && fetched.elapsed() < DISCOVERY_TTL
        {
            return Ok(doc.clone());
        }
        let base = provider.issuer.as_str().trim_end_matches('/');
        let url = format!("{base}/.well-known/openid-configuration");
        let body = self.get(provider, "discovery", &url).await?;
        let doc = Arc::new(
            parse_discovery(provider, &body)
                .map_err(|reason| unavailable(provider, "discovery", reason))?,
        );
        lock(&self.discovery).insert(provider.id, (Instant::now(), doc.clone()));
        Ok(doc)
    }

    /// The provider's signing keys, holding `kid` if the provider has it.
    /// Cached for [`JWKS_TTL`]; a `kid` the cache lacks refetches the set
    /// at most once per [`JWKS_REFETCH_MIN`], so forged `kid`s cannot make
    /// us hammer the provider.
    pub async fn keys(
        &self,
        provider: &OidcProvider,
        kid: &str,
    ) -> Result<Arc<Vec<id_token::Key>>, ApiError> {
        if let Some(set) = lock(&self.jwks).get(provider.id)
            && !needs_fetch(set, kid, Instant::now())
        {
            return Ok(set.keys.clone());
        }
        let jwks_uri = self.discovery(provider).await?.jwks_uri.clone();
        let body = self.get(provider, "jwks", jwks_uri.as_str()).await?;
        let keys = Arc::new(
            id_token::parse_jwks(&body).map_err(|reason| unavailable(provider, "jwks", reason))?,
        );
        lock(&self.jwks).insert(
            provider.id,
            KeySet {
                keys: keys.clone(),
                fetched: Instant::now(),
            },
        );
        Ok(keys)
    }

    /// Checks a provider's `id_token` against its keys, our client ID and
    /// the flow's `nonce`.
    pub async fn verify_id_token(
        &self,
        provider: &OidcProvider,
        raw: &str,
        nonce: &str,
    ) -> Result<Claims, ApiError> {
        let doc = self.discovery(provider).await?;
        let header =
            id_token::header(raw).map_err(|reason| unavailable(provider, "id_token", reason))?;
        let keys = self.keys(provider, &header.kid).await?;
        let expect = id_token::Expect {
            issuer: &doc.issuer,
            schemeless_issuer: is_google_preset(provider),
            client_id: &provider.client_id,
            nonce,
        };
        id_token::verify(raw, &keys, expect, chrono::Utc::now().timestamp())
            .map_err(|reason| unavailable(provider, "id_token", reason))
    }

    /// Trades the callback's `code` at the token endpoint and verifies the
    /// `id_token` that comes back. The userinfo endpoint is not called.
    pub async fn exchange(
        &self,
        provider: &OidcProvider,
        code: &str,
        verifier: &str,
        nonce: &str,
    ) -> Result<Claims, ApiError> {
        let doc = self.discovery(provider).await?;
        let form = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("grant_type", "authorization_code")
            .append_pair("code", code)
            .append_pair("redirect_uri", &self.redirect_uri(provider))
            .append_pair("client_id", &provider.client_id)
            .append_pair("client_secret", &provider.client_secret)
            .append_pair("code_verifier", verifier)
            .finish();
        let response = self
            .http
            .post(doc.token_endpoint.clone())
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header(ACCEPT, "application/json")
            .body(form)
            .send()
            .await
            .map_err(|err| unavailable(provider, "token", err.without_url()))?;
        let status = response.status();
        let body = read_capped(response)
            .await
            .map_err(|reason| unavailable(provider, "token", reason))?;
        let answer: TokenAnswer = serde_json::from_slice(&body).unwrap_or_default();
        if !status.is_success() {
            let error = answer
                .error
                .filter(|code| code.len() <= 64 && code.bytes().all(|c| c.is_ascii_graphic()))
                .unwrap_or_else(|| "-".to_owned());
            return Err(unavailable(
                provider,
                "token",
                format_args!("status {status}, error {error}"),
            ));
        }
        let raw = answer
            .id_token
            .ok_or_else(|| unavailable(provider, "token", "answer has no id_token"))?;
        self.verify_id_token(provider, &raw, nonce).await
    }

    async fn get(
        &self,
        provider: &OidcProvider,
        what: &str,
        url: &str,
    ) -> Result<Vec<u8>, ApiError> {
        let response = self
            .http
            .get(url)
            .header(ACCEPT, "application/json")
            .send()
            .await
            .map_err(|err| unavailable(provider, what, err))?;
        let status = response.status();
        if !status.is_success() {
            return Err(unavailable(provider, what, format_args!("status {status}")));
        }
        read_capped(response)
            .await
            .map_err(|reason| unavailable(provider, what, reason))
    }
}

#[derive(Debug, Default, Deserialize)]
struct TokenAnswer {
    id_token: Option<String>,
    error: Option<String>,
}

#[derive(Deserialize)]
struct DiscoveryDoc {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    jwks_uri: String,
}

/// The document's `issuer` must be the configured one (a trailing slash
/// aside), and its endpoints `https://`, or `http://` on loopback when the
/// issuer itself is on loopback (the tests' mock issuer).
fn parse_discovery(provider: &OidcProvider, body: &[u8]) -> Result<Discovery, String> {
    let doc: DiscoveryDoc =
        serde_json::from_slice(body).map_err(|err| format!("document: {err}"))?;
    if doc.issuer.trim_end_matches('/') != provider.issuer.as_str().trim_end_matches('/') {
        return Err("issuer does not match the configured one".to_owned());
    }
    let loopback = is_loopback_url(&provider.issuer);
    let endpoint = |name: &str, raw: &str| {
        let url = Url::parse(raw).map_err(|_| format!("{name} is not a URL"))?;
        let allowed = url.scheme() == "https"
            || (loopback && url.scheme() == "http" && is_loopback_url(&url));
        if allowed {
            Ok(url)
        } else {
            Err(format!("{name} is not https"))
        }
    };
    Ok(Discovery {
        authorization_endpoint: endpoint("authorization_endpoint", &doc.authorization_endpoint)?,
        token_endpoint: endpoint("token_endpoint", &doc.token_endpoint)?,
        jwks_uri: endpoint("jwks_uri", &doc.jwks_uri)?,
        issuer: doc.issuer,
    })
}

/// Whether the cached `set` must be fetched again to find `kid` at `now`.
fn needs_fetch(set: &KeySet, kid: &str, now: Instant) -> bool {
    let age = now.saturating_duration_since(set.fetched);
    if age >= JWKS_TTL {
        return true;
    }
    !set.keys.iter().any(|key| key.kid == kid) && age >= JWKS_REFETCH_MIN
}

fn is_google_preset(provider: &OidcProvider) -> bool {
    provider.id == "google" && provider.issuer.as_str().trim_end_matches('/') == GOOGLE_ISSUER
}

fn unavailable(provider: &OidcProvider, what: &str, reason: impl std::fmt::Display) -> ApiError {
    ApiError::OidcUnavailable(format!("{} {what}: {reason}", provider.id))
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The whole body, refused once it passes [`BODY_MAX`].
async fn read_capped(mut response: reqwest::Response) -> Result<Vec<u8>, String> {
    if response
        .content_length()
        .is_some_and(|length| length > BODY_MAX)
    {
        return Err("body too large".to_owned());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|err| format!("body: {}", err.without_url()))?
    {
        if (bytes.len() + chunk.len()) as u64 > BODY_MAX {
            return Err("body too large".to_owned());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(issuer: Option<&str>) -> OidcConfig {
        crate::config::Config::from_source(|key| {
            match key {
                crate::config::DATABASE_URL => Some("postgres://u:p@localhost/db"),
                crate::config::REDIS_URL => Some("redis://localhost"),
                crate::config::APP_PUBLIC_URL => Some("https://chat.example.org"),
                crate::config::OIDC_GOOGLE_CLIENT_ID => Some("id"),
                crate::config::OIDC_GOOGLE_CLIENT_SECRET => Some("secret"),
                crate::config::OIDC_GOOGLE_ISSUER => issuer,
                _ => None,
            }
            .map(str::to_owned)
        })
        .unwrap()
        .oidc
        .unwrap()
    }

    #[test]
    fn finds_configured_providers_only() {
        let oidc = Oidc::new(&config(None)).unwrap();
        assert_eq!(oidc.provider("google").map(|p| p.name), Some("Google"));
        assert!(oidc.provider("github").is_none());
    }

    #[test]
    fn redirect_uri_is_under_the_public_url() {
        let oidc = Oidc::new(&config(None)).unwrap();
        let google = oidc.provider("google").unwrap();
        assert_eq!(
            oidc.redirect_uri(google),
            "https://chat.example.org/api/auth/oidc/google/callback"
        );
        assert!(is_google_preset(google));
    }

    fn document(issuer: &str, endpoint: &str) -> Vec<u8> {
        serde_json::json!({
            "issuer": issuer,
            "authorization_endpoint": format!("{endpoint}/auth"),
            "token_endpoint": format!("{endpoint}/token"),
            "jwks_uri": format!("{endpoint}/jwks"),
            "response_types_supported": ["code"],
        })
        .to_string()
        .into_bytes()
    }

    #[test]
    fn discovery_issuer_must_match_ignoring_a_trailing_slash() {
        let config = config(None);
        let google = &config.providers[0];
        let doc = parse_discovery(
            google,
            &document(
                "https://accounts.google.com",
                "https://oauth2.googleapis.com",
            ),
        )
        .unwrap();
        assert_eq!(doc.issuer, "https://accounts.google.com");
        assert_eq!(
            doc.token_endpoint.as_str(),
            "https://oauth2.googleapis.com/token"
        );
        assert!(
            parse_discovery(
                google,
                &document(
                    "https://accounts.google.com/",
                    "https://oauth2.googleapis.com"
                )
            )
            .is_ok()
        );
        assert!(
            parse_discovery(
                google,
                &document("https://evil.example", "https://oauth2.googleapis.com")
            )
            .is_err()
        );
        assert!(parse_discovery(google, b"{\"issuer\":\"https://accounts.google.com\"}").is_err());
    }

    #[test]
    fn discovery_endpoints_are_https_except_on_loopback_issuers() {
        let config = config(None);
        let google = &config.providers[0];
        assert!(
            parse_discovery(
                google,
                &document("https://accounts.google.com", "http://127.0.0.1:9")
            )
            .is_err()
        );
        let config = self::config(Some("http://127.0.0.1:4000"));
        let mock = &config.providers[0];
        assert!(!is_google_preset(mock));
        assert!(
            parse_discovery(
                mock,
                &document("http://127.0.0.1:4000", "http://127.0.0.1:4000")
            )
            .is_ok()
        );
        assert!(
            parse_discovery(
                mock,
                &document("http://127.0.0.1:4000", "http://evil.example")
            )
            .is_err()
        );
        assert!(
            parse_discovery(
                mock,
                &document("http://127.0.0.1:4000", "https://evil.example")
            )
            .is_ok()
        );
    }

    fn key_set(kids: &[&str], fetched: Instant) -> KeySet {
        let keys = kids
            .iter()
            .map(|kid| {
                serde_json::json!({
                    "kty": "EC", "kid": kid, "crv": "P-256",
                    "x": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
                    "y": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
                })
            })
            .collect::<Vec<_>>();
        let body = serde_json::json!({ "keys": keys }).to_string();
        let keys = id_token::parse_jwks(body.as_bytes()).unwrap();
        assert_eq!(keys.len(), kids.len());
        KeySet {
            keys: Arc::new(keys),
            fetched,
        }
    }

    #[test]
    fn key_set_refetches_after_an_hour_or_for_an_unknown_kid_once_a_minute() {
        let start = Instant::now();
        let set = key_set(&["a"], start);
        assert!(!needs_fetch(&set, "a", start));
        assert!(!needs_fetch(
            &set,
            "a",
            start + JWKS_TTL - Duration::from_secs(1)
        ));
        assert!(needs_fetch(&set, "a", start + JWKS_TTL));
        assert!(!needs_fetch(&set, "b", start + Duration::from_secs(59)));
        assert!(needs_fetch(&set, "b", start + JWKS_REFETCH_MIN));
    }
}
