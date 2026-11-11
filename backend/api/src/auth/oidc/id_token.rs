//! The provider's `id_token`: a compact JWS parsed by hand and checked with
//! aws-lc-rs. Only `RS256` and `ES256` are accepted; `none`, `HS*` and
//! every other algorithm are refused before any key is looked at.

use std::fmt;

use aws_lc_rs::signature::{
    ECDSA_P256_SHA256_FIXED, RSA_PKCS1_2048_8192_SHA256, RsaPublicKeyComponents, UnparsedPublicKey,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD as B64};
use serde_json::{Map, Value};
use subtle::ConstantTimeEq;

use crate::auth::validate;
use crate::error::FieldErrors;

/// Clock skew allowed on `exp` and `iat`, in seconds.
pub const LEEWAY_SECS: i64 = 60;

/// Why a token or key set was refused. The text goes to the log, so it
/// never carries the token, a claim value or an address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejected {
    Malformed,
    Algorithm,
    UnknownKey,
    Signature,
    Issuer,
    Audience,
    AuthorizedParty,
    Expired,
    IssuedInFuture,
    Nonce,
    Subject,
}

impl fmt::Display for Rejected {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Malformed => "id_token is malformed",
            Self::Algorithm => "id_token algorithm is not RS256 or ES256",
            Self::UnknownKey => "id_token kid is not in the key set",
            Self::Signature => "id_token signature is invalid",
            Self::Issuer => "id_token iss does not match",
            Self::Audience => "id_token aud does not contain the client id",
            Self::AuthorizedParty => "id_token azp does not match the client id",
            Self::Expired => "id_token has expired",
            Self::IssuedInFuture => "id_token iat is in the future",
            Self::Nonce => "id_token nonce does not match",
            Self::Subject => "id_token sub is missing",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alg {
    Rs256,
    Es256,
}

impl Alg {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "RS256" => Some(Self::Rs256),
            "ES256" => Some(Self::Es256),
            _ => None,
        }
    }
}

/// The parts of the JOSE header that pick the key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub alg: Alg,
    pub kid: String,
}

/// One verification key from the provider's JWKS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Key {
    pub kid: String,
    material: Material,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Material {
    /// Big-endian modulus and exponent.
    Rsa { n: Vec<u8>, e: Vec<u8> },
    /// The uncompressed P-256 point `04 || x || y`.
    P256(Vec<u8>),
}

impl Key {
    fn alg(&self) -> Alg {
        match self.material {
            Material::Rsa { .. } => Alg::Rs256,
            Material::P256(_) => Alg::Es256,
        }
    }
}

/// The signing keys of a JWKS document. Keys without a `kid`, of another
/// type or curve, for another use or algorithm are skipped.
pub fn parse_jwks(body: &[u8]) -> Result<Vec<Key>, Rejected> {
    let doc: Value = serde_json::from_slice(body).map_err(|_| Rejected::Malformed)?;
    let keys = doc
        .get("keys")
        .and_then(Value::as_array)
        .ok_or(Rejected::Malformed)?;
    Ok(keys.iter().filter_map(jwk).collect())
}

fn jwk(value: &Value) -> Option<Key> {
    let text = |name: &str| value.get(name).and_then(Value::as_str);
    let bytes = |name: &str| text(name).and_then(|raw| B64.decode(raw).ok());
    let kid = text("kid").filter(|kid| !kid.is_empty())?.to_owned();
    if text("use").is_some_and(|usage| usage != "sig") {
        return None;
    }
    let material = match (text("kty")?, text("alg")) {
        ("RSA", None | Some("RS256")) => Material::Rsa {
            n: bytes("n").filter(|n| !n.is_empty())?,
            e: bytes("e").filter(|e| !e.is_empty())?,
        },
        ("EC", None | Some("ES256")) if text("crv") == Some("P-256") => {
            let (x, y) = (bytes("x")?, bytes("y")?);
            if x.len() != 32 || y.len() != 32 {
                return None;
            }
            let mut point = Vec::with_capacity(65);
            point.push(0x04);
            point.extend_from_slice(&x);
            point.extend_from_slice(&y);
            Material::P256(point)
        }
        _ => return None,
    };
    Some(Key { kid, material })
}

/// The JOSE header of `raw`, checked for an accepted algorithm and a `kid`.
pub fn header(raw: &str) -> Result<Header, Rejected> {
    let (header, _, _) = split(raw)?;
    let header = object(header)?;
    let alg = header
        .get("alg")
        .and_then(Value::as_str)
        .and_then(Alg::parse)
        .ok_or(Rejected::Algorithm)?;
    let kid = header
        .get("kid")
        .and_then(Value::as_str)
        .filter(|kid| !kid.is_empty())
        .ok_or(Rejected::UnknownKey)?
        .to_owned();
    Ok(Header { alg, kid })
}

/// What the token must say: the discovery issuer, our client ID and the
/// flow's nonce.
#[derive(Debug, Clone, Copy)]
pub struct Expect<'a> {
    pub issuer: &'a str,
    /// Google documents `accounts.google.com` without a scheme as a valid
    /// `iss` as well; only its preset accepts that form.
    pub schemeless_issuer: bool,
    pub client_id: &'a str,
    pub nonce: &'a str,
}

/// The verified answer, with the email and name already normalised.
#[derive(Clone, PartialEq, Eq)]
pub struct Claims {
    pub subject: String,
    /// Canonical form as registration stores it; `None` when missing or
    /// not a usable address.
    pub email: Option<String>,
    pub email_verified: bool,
    /// The hint for the name step.
    pub name: Option<String>,
}

impl fmt::Debug for Claims {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Claims")
            .field("email_verified", &self.email_verified)
            .finish_non_exhaustive()
    }
}

/// Verifies `raw` against the keys of the provider and `expect`, at `now`
/// (Unix seconds).
pub fn verify(raw: &str, keys: &[Key], expect: Expect<'_>, now: i64) -> Result<Claims, Rejected> {
    let Header { alg, kid } = header(raw)?;
    let key = keys
        .iter()
        .find(|key| key.kid == kid)
        .ok_or(Rejected::UnknownKey)?;
    if key.alg() != alg {
        return Err(Rejected::Algorithm);
    }
    let (_, payload, signature) = split(raw)?;
    let signature = B64.decode(signature).map_err(|_| Rejected::Malformed)?;
    let signed = &raw.as_bytes()[..raw.rfind('.').ok_or(Rejected::Malformed)?];
    let valid = match &key.material {
        Material::Rsa { n, e } => {
            RsaPublicKeyComponents { n, e }.verify(&RSA_PKCS1_2048_8192_SHA256, signed, &signature)
        }
        Material::P256(point) => {
            UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, point).verify(signed, &signature)
        }
    };
    valid.map_err(|_| Rejected::Signature)?;
    claims(&object(payload)?, expect, now)
}

fn claims(claims: &Map<String, Value>, expect: Expect<'_>, now: i64) -> Result<Claims, Rejected> {
    let text = |name: &str| claims.get(name).and_then(Value::as_str);
    let seconds = |name: &str| {
        claims
            .get(name)
            .and_then(Value::as_f64)
            .map(|value| value.floor() as i64)
            .ok_or(Rejected::Malformed)
    };

    let issuer = text("iss").ok_or(Rejected::Issuer)?;
    let schemeless = expect
        .schemeless_issuer
        .then(|| expect.issuer.strip_prefix("https://"))
        .flatten();
    if issuer != expect.issuer && Some(issuer) != schemeless {
        return Err(Rejected::Issuer);
    }

    let audiences: Vec<&str> = match claims.get("aud") {
        Some(Value::String(one)) => vec![one.as_str()],
        Some(Value::Array(many)) => many.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    };
    if !audiences.contains(&expect.client_id) {
        return Err(Rejected::Audience);
    }
    if audiences.len() > 1 && text("azp") != Some(expect.client_id) {
        return Err(Rejected::AuthorizedParty);
    }

    if seconds("exp")? <= now - LEEWAY_SECS {
        return Err(Rejected::Expired);
    }
    if seconds("iat")? >= now + LEEWAY_SECS {
        return Err(Rejected::IssuedInFuture);
    }

    let nonce = text("nonce").ok_or(Rejected::Nonce)?;
    if !bool::from(nonce.as_bytes().ct_eq(expect.nonce.as_bytes())) {
        return Err(Rejected::Nonce);
    }

    let subject = text("sub")
        .filter(|sub| !sub.is_empty())
        .ok_or(Rejected::Subject)?
        .to_owned();

    Ok(Claims {
        subject,
        email: text("email").and_then(email),
        email_verified: matches!(claims.get("email_verified"), Some(Value::Bool(true)))
            || text("email_verified") == Some("true"),
        name: text("name").and_then(name_hint),
    })
}

/// Normalised like registration; an address `validate::email` refuses
/// counts as missing.
fn email(raw: &str) -> Option<String> {
    let mut errors = FieldErrors::new();
    let value = validate::email(raw, &mut errors)?;
    let canonical = validate::canonical(&value);
    Some(validate::ascii_domain(&canonical).unwrap_or(canonical))
}

/// Trimmed, without control characters, at most `NAME_MAX` characters.
fn name_hint(raw: &str) -> Option<String> {
    let clean: String = raw.chars().filter(|c| !c.is_control()).collect();
    let cut: String = clean.trim().chars().take(validate::NAME_MAX).collect();
    let cut = cut.trim_end();
    (!cut.is_empty()).then(|| cut.to_owned())
}

fn split(raw: &str) -> Result<(&str, &str, &str), Rejected> {
    let mut parts = raw.split('.');
    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(header), Some(payload), Some(signature), None) => Ok((header, payload, signature)),
        _ => Err(Rejected::Malformed),
    }
}

fn object(part: &str) -> Result<Map<String, Value>, Rejected> {
    let bytes = B64.decode(part).map_err(|_| Rejected::Malformed)?;
    match serde_json::from_slice(&bytes) {
        Ok(Value::Object(map)) => Ok(map),
        _ => Err(Rejected::Malformed),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::OnceLock;

    use aws_lc_rs::rand::SystemRandom;
    use aws_lc_rs::rsa::KeySize;
    use aws_lc_rs::signature::{
        ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair, RSA_PKCS1_SHA256, RsaKeyPair,
    };
    use serde_json::json;

    use super::*;

    const ISSUER: &str = "https://issuer.example";
    const CLIENT: &str = "client-1";
    const NONCE: &str = "4f1c0e6a";
    const NOW: i64 = 1_800_000_000;

    fn expect() -> Expect<'static> {
        Expect {
            issuer: ISSUER,
            schemeless_issuer: false,
            client_id: CLIENT,
            nonce: NONCE,
        }
    }

    fn rsa() -> &'static RsaKeyPair {
        static KEY: OnceLock<RsaKeyPair> = OnceLock::new();
        KEY.get_or_init(|| RsaKeyPair::generate(KeySize::Rsa2048).unwrap())
    }

    fn ec() -> &'static EcdsaKeyPair {
        static KEY: OnceLock<EcdsaKeyPair> = OnceLock::new();
        KEY.get_or_init(|| EcdsaKeyPair::generate(&ECDSA_P256_SHA256_FIXED_SIGNING).unwrap())
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

    fn jwks() -> Vec<Key> {
        let (n, e) = rsa_components(rsa().public_key().as_ref());
        let point = ec().public_key().as_ref();
        let body = json!({ "keys": [
            { "kty": "RSA", "kid": "rsa-1", "alg": "RS256", "use": "sig",
              "n": B64.encode(n), "e": B64.encode(e) },
            { "kty": "EC", "kid": "ec-1", "crv": "P-256",
              "x": B64.encode(&point[1..33]), "y": B64.encode(&point[33..65]) },
            { "kty": "oct", "kid": "hmac-1", "k": B64.encode(b"shared secret") },
        ]});
        parse_jwks(body.to_string().as_bytes()).unwrap()
    }

    fn claims_json() -> Value {
        json!({
            "iss": ISSUER, "aud": CLIENT, "sub": "1234567890",
            "exp": NOW + 600, "iat": NOW - 5, "nonce": NONCE,
            "email": "Lena@Example.TEST", "email_verified": true, "name": "  Lena Example ",
        })
    }

    fn sign(header: Value, claims: &Value) -> String {
        let input = format!(
            "{}.{}",
            B64.encode(header.to_string()),
            B64.encode(claims.to_string())
        );
        let signature = match header["alg"].as_str() {
            Some("RS256") => {
                let mut out = vec![0; rsa().public_modulus_len()];
                rsa()
                    .sign(
                        &RSA_PKCS1_SHA256,
                        &SystemRandom::new(),
                        input.as_bytes(),
                        &mut out,
                    )
                    .unwrap();
                out
            }
            Some("ES256") => ec()
                .sign(&SystemRandom::new(), input.as_bytes())
                .unwrap()
                .as_ref()
                .to_vec(),
            _ => b"not a signature".to_vec(),
        };
        format!("{input}.{}", B64.encode(signature))
    }

    fn rs256(claims: &Value) -> String {
        sign(
            json!({ "alg": "RS256", "kid": "rsa-1", "typ": "JWT" }),
            claims,
        )
    }

    fn check(token: &str) -> Result<Claims, Rejected> {
        verify(token, &jwks(), expect(), NOW)
    }

    fn with(change: impl FnOnce(&mut Map<String, Value>)) -> Value {
        let mut claims = claims_json();
        change(claims.as_object_mut().unwrap());
        claims
    }

    #[test]
    fn accepts_a_valid_rs256_token() {
        let claims = check(&rs256(&claims_json())).unwrap();
        assert_eq!(claims.subject, "1234567890");
        assert_eq!(claims.email.as_deref(), Some("lena@example.test"));
        assert!(claims.email_verified);
        assert_eq!(claims.name.as_deref(), Some("Lena Example"));
    }

    #[test]
    fn accepts_a_valid_es256_token() {
        let token = sign(json!({ "alg": "ES256", "kid": "ec-1" }), &claims_json());
        assert_eq!(check(&token).unwrap().subject, "1234567890");
    }

    #[test]
    fn jwks_keeps_only_rsa_and_p256_signing_keys() {
        let kids: Vec<_> = jwks().into_iter().map(|key| key.kid).collect();
        assert_eq!(kids, ["rsa-1", "ec-1"]);
        let other = json!({ "keys": [
            { "kty": "RSA", "kid": "enc", "use": "enc", "n": "AQAB", "e": "AQAB" },
            { "kty": "RSA", "kid": "ps", "alg": "PS256", "n": "AQAB", "e": "AQAB" },
            { "kty": "EC", "kid": "p384", "crv": "P-384", "x": "AA", "y": "AA" },
            { "kty": "RSA", "n": "AQAB", "e": "AQAB" },
        ]});
        assert_eq!(parse_jwks(other.to_string().as_bytes()), Ok(Vec::new()));
        assert_eq!(parse_jwks(b"{}"), Err(Rejected::Malformed));
    }

    #[test]
    fn rejects_alg_none_and_hs256() {
        let unsigned = format!(
            "{}.{}.",
            B64.encode(json!({ "alg": "none", "kid": "rsa-1" }).to_string()),
            B64.encode(claims_json().to_string())
        );
        assert_eq!(check(&unsigned), Err(Rejected::Algorithm));
        let hs = sign(json!({ "alg": "HS256", "kid": "hmac-1" }), &claims_json());
        assert_eq!(check(&hs), Err(Rejected::Algorithm));
        let ps = sign(json!({ "alg": "PS256", "kid": "rsa-1" }), &claims_json());
        assert_eq!(check(&ps), Err(Rejected::Algorithm));
    }

    #[test]
    fn rejects_an_alg_the_key_was_not_made_for() {
        let token = sign(json!({ "alg": "ES256", "kid": "rsa-1" }), &claims_json());
        assert_eq!(check(&token), Err(Rejected::Algorithm));
    }

    #[test]
    fn rejects_an_unknown_or_missing_kid() {
        let token = sign(json!({ "alg": "RS256", "kid": "rsa-2" }), &claims_json());
        assert_eq!(check(&token), Err(Rejected::UnknownKey));
        let token = sign(json!({ "alg": "RS256" }), &claims_json());
        assert_eq!(check(&token), Err(Rejected::UnknownKey));
    }

    #[test]
    fn rejects_a_bad_signature() {
        let token = rs256(&claims_json());
        let (signed, _) = token.rsplit_once('.').unwrap();
        let other = rs256(&with(|c| {
            c.insert("sub".into(), json!("someone-else"));
        }));
        let (_, foreign) = other.rsplit_once('.').unwrap();
        assert_eq!(
            check(&format!("{signed}.{foreign}")),
            Err(Rejected::Signature)
        );
        // Claims swapped under a valid signature.
        let (header, _) = signed.split_once('.').unwrap();
        let forged = format!(
            "{header}.{}.{}",
            B64.encode(
                with(|c| {
                    c.insert("sub".into(), json!("x"));
                })
                .to_string()
            ),
            token.rsplit_once('.').unwrap().1
        );
        assert_eq!(check(&forged), Err(Rejected::Signature));
    }

    #[test]
    fn rejects_malformed_tokens() {
        for raw in ["", "a.b", "a.b.c.d", "!!.!!.!!"] {
            assert!(check(raw).is_err(), "{raw:?}");
        }
    }

    #[test]
    fn rejects_a_wrong_issuer() {
        let token = rs256(&with(|c| {
            c.insert("iss".into(), json!("https://evil.example"));
        }));
        assert_eq!(check(&token), Err(Rejected::Issuer));
        let token = rs256(&with(|c| {
            c.remove("iss");
        }));
        assert_eq!(check(&token), Err(Rejected::Issuer));
    }

    #[test]
    fn schemeless_issuer_only_for_the_google_preset() {
        let token = rs256(&with(|c| {
            c.insert("iss".into(), json!("issuer.example"));
        }));
        assert_eq!(check(&token), Err(Rejected::Issuer));
        let google = Expect {
            schemeless_issuer: true,
            ..expect()
        };
        assert!(verify(&token, &jwks(), google, NOW).is_ok());
        let token = rs256(&with(|c| {
            c.insert("iss".into(), json!("evil.example"));
        }));
        assert_eq!(verify(&token, &jwks(), google, NOW), Err(Rejected::Issuer));
    }

    #[test]
    fn rejects_a_wrong_audience() {
        let token = rs256(&with(|c| {
            c.insert("aud".into(), json!("client-2"));
        }));
        assert_eq!(check(&token), Err(Rejected::Audience));
        let token = rs256(&with(|c| {
            c.insert("aud".into(), json!(["client-2", "client-3"]));
        }));
        assert_eq!(check(&token), Err(Rejected::Audience));
    }

    #[test]
    fn several_audiences_need_azp_to_be_the_client() {
        let token = rs256(&with(|c| {
            c.insert("aud".into(), json!([CLIENT, "client-2"]));
            c.insert("azp".into(), json!("client-2"));
        }));
        assert_eq!(check(&token), Err(Rejected::AuthorizedParty));
        let token = rs256(&with(|c| {
            c.insert("aud".into(), json!([CLIENT, "client-2"]));
        }));
        assert_eq!(check(&token), Err(Rejected::AuthorizedParty));
        let token = rs256(&with(|c| {
            c.insert("aud".into(), json!([CLIENT, "client-2"]));
            c.insert("azp".into(), json!(CLIENT));
        }));
        assert!(check(&token).is_ok());
        let token = rs256(&with(|c| {
            c.insert("aud".into(), json!([CLIENT]));
        }));
        assert!(check(&token).is_ok());
    }

    #[test]
    fn rejects_an_expired_token_after_the_leeway() {
        let token = rs256(&with(|c| {
            c.insert("exp".into(), json!(NOW - LEEWAY_SECS));
        }));
        assert_eq!(check(&token), Err(Rejected::Expired));
        let token = rs256(&with(|c| {
            c.insert("exp".into(), json!(NOW - LEEWAY_SECS + 1));
        }));
        assert!(check(&token).is_ok());
        let token = rs256(&with(|c| {
            c.remove("exp");
        }));
        assert_eq!(check(&token), Err(Rejected::Malformed));
    }

    #[test]
    fn rejects_iat_in_the_future_after_the_leeway() {
        let token = rs256(&with(|c| {
            c.insert("iat".into(), json!(NOW + LEEWAY_SECS));
        }));
        assert_eq!(check(&token), Err(Rejected::IssuedInFuture));
        let token = rs256(&with(|c| {
            c.insert("iat".into(), json!(NOW + LEEWAY_SECS - 1));
        }));
        assert!(check(&token).is_ok());
    }

    #[test]
    fn rejects_a_wrong_or_missing_nonce() {
        let token = rs256(&with(|c| {
            c.insert("nonce".into(), json!("other"));
        }));
        assert_eq!(check(&token), Err(Rejected::Nonce));
        let token = rs256(&with(|c| {
            c.remove("nonce");
        }));
        assert_eq!(check(&token), Err(Rejected::Nonce));
    }

    #[test]
    fn rejects_an_empty_subject() {
        let token = rs256(&with(|c| {
            c.insert("sub".into(), json!(""));
        }));
        assert_eq!(check(&token), Err(Rejected::Subject));
    }

    #[test]
    fn email_verified_is_true_only_for_true_or_the_string_true() {
        for (value, verified) in [
            (Some(json!(true)), true),
            (Some(json!("true")), true),
            (Some(json!(false)), false),
            (Some(json!("false")), false),
            (Some(json!(1)), false),
            (None, false),
        ] {
            let token = rs256(&with(|c| match value.clone() {
                Some(value) => {
                    c.insert("email_verified".into(), value);
                }
                None => {
                    c.remove("email_verified");
                }
            }));
            assert_eq!(check(&token).unwrap().email_verified, verified, "{value:?}");
        }
    }

    #[test]
    fn email_is_normalised_and_a_bad_one_counts_as_missing() {
        let email_of = |value: Value| {
            check(&rs256(&with(|c| {
                c.insert("email".into(), value);
            })))
            .unwrap()
            .email
        };
        assert_eq!(
            email_of(json!(" Lena@Bücher.Example ")).as_deref(),
            Some("lena@xn--bcher-kva.example")
        );
        assert_eq!(email_of(json!("not an address")), None);
        assert_eq!(email_of(json!(42)), None);
    }

    #[test]
    fn name_hint_is_trimmed_cleaned_and_cut() {
        assert_eq!(
            name_hint("  Lena\u{0007} Ex\nample  ").as_deref(),
            Some("Lena Example")
        );
        assert_eq!(name_hint(" \t "), None);
        let long = "x".repeat(validate::NAME_MAX + 10);
        assert_eq!(
            name_hint(&long).map(|name| name.chars().count()),
            Some(validate::NAME_MAX)
        );
    }

    #[test]
    fn claims_debug_hides_subject_email_and_name() {
        let claims = check(&rs256(&claims_json())).unwrap();
        let shown = format!("{claims:?}");
        assert!(
            !shown.contains("1234567890") && !shown.contains("lena"),
            "{shown}"
        );
    }
}
