//! A test-only OpenID Connect issuer on 127.0.0.1 (#518): discovery, JWKS
//! and a token endpoint, signing with an aws-lc-rs key made per test.
//!
//! The authorization step runs in process: [`MockIssuer::authorize`] reads
//! the URL the API built, remembers the grant and returns the callback URL
//! a browser would be sent to, as the provider would after a sign-in.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::rsa::KeySize;
use aws_lc_rs::signature::{
    ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair, RSA_PKCS1_SHA256, RsaKeyPair,
};
use axum::extract::{Form, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD as B64};
use gelabber_api::auth::oidc::{Oidc, pkce};
use gelabber_api::config::{OidcConfig, OidcProvider};
use serde_json::{Map, Value, json};
use url::Url;

pub const CLIENT_ID: &str = "mock-client.apps.example.test";
pub const CLIENT_SECRET: &str = "mock-secret";

/// Whom the next sign-in at the mock is for. `claims` is merged over the
/// standard claims last, so a test can also forge `iss`, `aud`, `nonce`…
#[derive(Debug, Clone)]
pub struct MockUser {
    pub sub: String,
    pub email: Option<String>,
    pub email_verified: Value,
    pub name: Option<String>,
    pub claims: Map<String, Value>,
}

impl MockUser {
    /// A person whose provider has verified `email`.
    pub fn verified(sub: &str, email: &str, name: &str) -> Self {
        Self {
            sub: sub.to_owned(),
            email: Some(email.to_owned()),
            email_verified: json!(true),
            name: Some(name.to_owned()),
            claims: Map::new(),
        }
    }

    /// The same, with `email_verified: false`.
    pub fn unverified(sub: &str, email: &str, name: &str) -> Self {
        Self {
            email_verified: json!(false),
            ..Self::verified(sub, email, name)
        }
    }

    pub fn with_claim(mut self, name: &str, value: Value) -> Self {
        self.claims.insert(name.to_owned(), value);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alg {
    Rs256,
    Es256,
}

enum Signer {
    Rsa(RsaKeyPair),
    Ec(EcdsaKeyPair),
}

struct SigningKey {
    kid: String,
    signer: Signer,
}

impl SigningKey {
    fn new(alg: Alg, kid: String) -> Self {
        let signer = match alg {
            Alg::Rs256 => Signer::Rsa(RsaKeyPair::generate(KeySize::Rsa2048).unwrap()),
            Alg::Es256 => {
                Signer::Ec(EcdsaKeyPair::generate(&ECDSA_P256_SHA256_FIXED_SIGNING).unwrap())
            }
        };
        Self { kid, signer }
    }

    fn jwk(&self) -> Value {
        match &self.signer {
            Signer::Rsa(key) => {
                let (n, e) = rsa_components(key.public_key().as_ref());
                json!({ "kty": "RSA", "kid": self.kid, "alg": "RS256", "use": "sig",
                        "n": B64.encode(n), "e": B64.encode(e) })
            }
            Signer::Ec(key) => {
                let point = key.public_key().as_ref();
                json!({ "kty": "EC", "kid": self.kid, "alg": "ES256", "use": "sig",
                        "crv": "P-256",
                        "x": B64.encode(&point[1..33]), "y": B64.encode(&point[33..65]) })
            }
        }
    }

    fn sign(&self, claims: &Value) -> String {
        let alg = match self.signer {
            Signer::Rsa(_) => "RS256",
            Signer::Ec(_) => "ES256",
        };
        let header = json!({ "alg": alg, "kid": self.kid, "typ": "JWT" });
        let input = format!(
            "{}.{}",
            B64.encode(header.to_string()),
            B64.encode(claims.to_string())
        );
        let rng = SystemRandom::new();
        let signature = match &self.signer {
            Signer::Rsa(key) => {
                let mut out = vec![0; key.public_modulus_len()];
                key.sign(&RSA_PKCS1_SHA256, &rng, input.as_bytes(), &mut out)
                    .unwrap();
                out
            }
            Signer::Ec(key) => key.sign(&rng, input.as_bytes()).unwrap().as_ref().to_vec(),
        };
        format!("{input}.{}", B64.encode(signature))
    }
}

/// `n` and `e` from the PKCS#1 `RSAPublicKey` DER aws-lc-rs hands out.
fn rsa_components(der: &[u8]) -> (Vec<u8>, Vec<u8>) {
    fn element(input: &[u8], tag: u8) -> (&[u8], &[u8]) {
        assert_eq!(input[0], tag);
        let (len, header) = match input[1] {
            short if short < 0x80 => (short as usize, 2),
            long => {
                let count = (long & 0x7f) as usize;
                let len = input[2..2 + count]
                    .iter()
                    .fold(0usize, |acc, byte| (acc << 8) | *byte as usize);
                (len, 2 + count)
            }
        };
        (&input[header..header + len], &input[header + len..])
    }
    let (body, _) = element(der, 0x30);
    let (n, rest) = element(body, 0x02);
    let (e, _) = element(rest, 0x02);
    let strip = |v: &[u8]| v.iter().skip_while(|b| **b == 0).copied().collect();
    (strip(n), strip(e))
}

struct Grant {
    challenge: String,
    redirect_uri: String,
    nonce: String,
    user: MockUser,
}

struct Inner {
    issuer: String,
    alg: Alg,
    keys: Mutex<Vec<SigningKey>>,
    grants: Mutex<HashMap<String, Grant>>,
    /// Overrides the discovery document's `issuer` when set.
    discovery_issuer: Mutex<Option<String>>,
    discovery_hits: AtomicUsize,
    jwks_hits: AtomicUsize,
    token_hits: AtomicUsize,
}

/// The running mock. Dropping it leaves the server task running until the
/// test's runtime ends, like the other test servers.
#[derive(Clone)]
pub struct MockIssuer {
    inner: Arc<Inner>,
}

impl MockIssuer {
    /// An issuer signing with ES256.
    pub async fn start() -> Self {
        Self::start_with(Alg::Es256).await
    }

    pub async fn start_with(alg: Alg) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let issuer = format!("http://{}", listener.local_addr().unwrap());
        let inner = Arc::new(Inner {
            issuer,
            alg,
            keys: Mutex::new(vec![SigningKey::new(alg, "key-1".to_owned())]),
            grants: Mutex::default(),
            discovery_issuer: Mutex::default(),
            discovery_hits: AtomicUsize::new(0),
            jwks_hits: AtomicUsize::new(0),
            token_hits: AtomicUsize::new(0),
        });
        let router = Router::new()
            .route("/.well-known/openid-configuration", get(discovery))
            .route("/jwks", get(jwks))
            .route("/token", post(token))
            .with_state(inner.clone());
        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        Self { inner }
    }

    /// `http://127.0.0.1:{port}`, without a trailing slash.
    pub fn issuer(&self) -> &str {
        &self.inner.issuer
    }

    /// An `OidcConfig` with this issuer as the `google` provider (not the
    /// Google preset, since the issuer differs) behind `public_url`.
    pub fn config(&self, public_url: &str) -> OidcConfig {
        OidcConfig {
            public_url: Url::parse(public_url).unwrap(),
            providers: vec![OidcProvider {
                id: "google",
                name: "Google",
                issuer: Url::parse(self.issuer()).unwrap(),
                client_id: CLIENT_ID.to_owned(),
                client_secret: CLIENT_SECRET.to_owned(),
                scopes: "openid email profile",
                extra_auth_params: &[("prompt", "select_account")],
            }],
        }
    }

    /// [`Self::config`] as the `Oidc` for `AppState.oidc`.
    pub fn oidc(&self, public_url: &str) -> Arc<Oidc> {
        Arc::new(Oidc::new(&self.config(public_url)).unwrap())
    }

    /// A sign-in of `user` at the authorization URL the API built. Returns
    /// the callback URL with `code` and `state`, as the provider's redirect.
    pub fn authorize(&self, auth_url: &str, user: MockUser) -> String {
        let url = Url::parse(auth_url).unwrap();
        let query: HashMap<String, String> = url.query_pairs().into_owned().collect();
        assert_eq!(query.get("response_type").map(String::as_str), Some("code"));
        assert_eq!(query.get("client_id").map(String::as_str), Some(CLIENT_ID));
        assert_eq!(
            query.get("code_challenge_method").map(String::as_str),
            Some("S256")
        );
        let redirect_uri = query["redirect_uri"].clone();
        let code = gelabber_api::token::generate();
        self.inner.grants.lock().unwrap().insert(
            code.clone(),
            Grant {
                challenge: query["code_challenge"].clone(),
                redirect_uri: redirect_uri.clone(),
                nonce: query.get("nonce").cloned().unwrap_or_default(),
                user,
            },
        );
        let mut callback = Url::parse(&redirect_uri).unwrap();
        callback
            .query_pairs_mut()
            .append_pair("code", &code)
            .append_pair("state", &query["state"]);
        callback.into()
    }

    /// The person cancels at the provider: `error=access_denied`.
    pub fn deny(&self, auth_url: &str) -> String {
        let url = Url::parse(auth_url).unwrap();
        let query: HashMap<String, String> = url.query_pairs().into_owned().collect();
        let mut callback = Url::parse(&query["redirect_uri"]).unwrap();
        callback
            .query_pairs_mut()
            .append_pair("error", "access_denied")
            .append_pair("state", &query["state"]);
        callback.into()
    }

    /// Stores a grant directly, for tests that call `Oidc::exchange`
    /// without an authorization URL. Returns the code.
    pub fn grant(&self, verifier: &str, redirect_uri: &str, nonce: &str, user: MockUser) -> String {
        let code = gelabber_api::token::generate();
        self.inner.grants.lock().unwrap().insert(
            code.clone(),
            Grant {
                challenge: pkce::challenge(verifier),
                redirect_uri: redirect_uri.to_owned(),
                nonce: nonce.to_owned(),
                user,
            },
        );
        code
    }

    /// Replaces the signing key with a new one under a new `kid`.
    pub fn rotate(&self) {
        let mut keys = self.inner.keys.lock().unwrap();
        let kid = format!("key-{}", keys.len() + 1);
        *keys = vec![SigningKey::new(self.inner.alg, kid)];
    }

    /// Makes the discovery document claim another issuer.
    pub fn claim_issuer(&self, issuer: &str) {
        *self.inner.discovery_issuer.lock().unwrap() = Some(issuer.to_owned());
    }

    pub fn discovery_hits(&self) -> usize {
        self.inner.discovery_hits.load(Ordering::SeqCst)
    }

    pub fn jwks_hits(&self) -> usize {
        self.inner.jwks_hits.load(Ordering::SeqCst)
    }

    pub fn token_hits(&self) -> usize {
        self.inner.token_hits.load(Ordering::SeqCst)
    }
}

async fn discovery(State(inner): State<Arc<Inner>>) -> Json<Value> {
    inner.discovery_hits.fetch_add(1, Ordering::SeqCst);
    let base = &inner.issuer;
    let issuer = inner
        .discovery_issuer
        .lock()
        .unwrap()
        .clone()
        .unwrap_or_else(|| base.clone());
    Json(json!({
        "issuer": issuer,
        "authorization_endpoint": format!("{base}/auth"),
        "token_endpoint": format!("{base}/token"),
        "jwks_uri": format!("{base}/jwks"),
        "response_types_supported": ["code"],
        "subject_types_supported": ["public"],
        "id_token_signing_alg_values_supported": ["RS256", "ES256"],
    }))
}

async fn jwks(State(inner): State<Arc<Inner>>) -> Json<Value> {
    inner.jwks_hits.fetch_add(1, Ordering::SeqCst);
    let keys: Vec<Value> = inner
        .keys
        .lock()
        .unwrap()
        .iter()
        .map(SigningKey::jwk)
        .collect();
    Json(json!({ "keys": keys }))
}

/// The token endpoint checks what a real provider checks: the client, the
/// redirect URI and the PKCE verifier. A code works once.
async fn token(
    State(inner): State<Arc<Inner>>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    inner.token_hits.fetch_add(1, Ordering::SeqCst);
    let field = |name: &str| form.get(name).map(String::as_str).unwrap_or_default();
    let refuse =
        |error: &str| (StatusCode::BAD_REQUEST, Json(json!({ "error": error }))).into_response();
    if field("grant_type") != "authorization_code" {
        return refuse("unsupported_grant_type");
    }
    if field("client_id") != CLIENT_ID || field("client_secret") != CLIENT_SECRET {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "invalid_client" })),
        )
            .into_response();
    }
    let Some(grant) = inner.grants.lock().unwrap().remove(field("code")) else {
        return refuse("invalid_grant");
    };
    if grant.redirect_uri != field("redirect_uri")
        || pkce::challenge(field("code_verifier")) != grant.challenge
    {
        return refuse("invalid_grant");
    }
    let now = chrono::Utc::now().timestamp();
    let mut claims = json!({
        "iss": inner.issuer,
        "aud": CLIENT_ID,
        "azp": CLIENT_ID,
        "sub": grant.user.sub,
        "iat": now,
        "exp": now + 3600,
        "nonce": grant.nonce,
        "email_verified": grant.user.email_verified,
    });
    let object = claims.as_object_mut().unwrap();
    if let Some(email) = grant.user.email {
        object.insert("email".to_owned(), json!(email));
    }
    if let Some(name) = grant.user.name {
        object.insert("name".to_owned(), json!(name));
    }
    object.extend(grant.user.claims);
    let id_token = inner.keys.lock().unwrap()[0].sign(&claims);
    Json(json!({
        "access_token": "mock-access-token",
        "token_type": "Bearer",
        "expires_in": 3600,
        "scope": "openid email profile",
        "id_token": id_token,
    }))
    .into_response()
}
