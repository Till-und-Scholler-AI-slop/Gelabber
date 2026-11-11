//! Sign-in through OpenID Connect (#518). Off unless the operator sets a
//! provider's client ID and secret ([`crate::config::OidcConfig`]).

use std::time::Duration;

use crate::config::{OidcConfig, OidcProvider};

/// The configured providers plus the HTTP client that talks to them.
/// `AppState.oidc` holds it; tests swap it like `state.mail`.
#[derive(Debug)]
pub struct Oidc {
    pub config: OidcConfig,
    pub http: reqwest::Client,
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
        })
    }

    /// A configured provider by its route id (`google`).
    pub fn provider(&self, id: &str) -> Option<&OidcProvider> {
        self.config
            .providers
            .iter()
            .find(|provider| provider.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_configured_providers_only() {
        let config = crate::config::Config::from_source(|key| {
            match key {
                crate::config::DATABASE_URL => Some("postgres://u:p@localhost/db"),
                crate::config::REDIS_URL => Some("redis://localhost"),
                crate::config::APP_PUBLIC_URL => Some("https://chat.example.org"),
                crate::config::OIDC_GOOGLE_CLIENT_ID => Some("id"),
                crate::config::OIDC_GOOGLE_CLIENT_SECRET => Some("secret"),
                _ => None,
            }
            .map(str::to_owned)
        })
        .unwrap();
        let oidc = Oidc::new(config.oidc.as_ref().unwrap()).unwrap();
        assert_eq!(oidc.provider("google").map(|p| p.name), Some("Google"));
        assert!(oidc.provider("github").is_none());
    }
}
