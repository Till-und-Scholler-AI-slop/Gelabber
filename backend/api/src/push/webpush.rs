//! The wire part of Web Push: the payload encrypted for one browser
//! (RFC 8291, `aes128gcm`) and the server's VAPID signature (RFC 8292).

use aes_gcm::{Aes128Gcm, KeyInit, aead::Aead};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD as B64};
use hkdf::Hkdf;
use p256::{
    PublicKey, SecretKey,
    ecdsa::{Signature, SigningKey, signature::Signer},
    elliptic_curve::sec1::ToSec1Point,
};
use sha2::Sha256;

/// One record: the payload plus its delimiter and tag must fit.
const RECORD_SIZE: u32 = 4096;
/// Plaintext bytes that still fit in that record.
pub const MAX_PAYLOAD: usize = RECORD_SIZE as usize - 1 - 16 - 86;

#[derive(Debug, PartialEq, Eq)]
pub enum PushError {
    /// The browser's `p256dh` or `auth` key is not usable.
    BadKeys,
    TooLarge,
}

pub fn uncompressed(key: &PublicKey) -> Vec<u8> {
    key.to_sec1_point(false).as_bytes().to_vec()
}

/// A fresh P-256 key from the OS random source.
pub fn random_key() -> SecretKey {
    loop {
        let bytes: [u8; 32] = rand::random();
        if let Ok(key) = SecretKey::from_slice(&bytes) {
            return key;
        }
    }
}

/// The browser's keys from its subscription, base64url as the Push API
/// hands them out.
pub struct Receiver {
    public: PublicKey,
    auth: [u8; 16],
}

impl Receiver {
    pub fn parse(p256dh: &str, auth: &str) -> Result<Self, PushError> {
        let public = B64
            .decode(p256dh.trim_end_matches('='))
            .ok()
            .and_then(|raw| PublicKey::from_sec1_bytes(&raw).ok())
            .ok_or(PushError::BadKeys)?;
        let auth: [u8; 16] = B64
            .decode(auth.trim_end_matches('='))
            .ok()
            .and_then(|raw| raw.try_into().ok())
            .ok_or(PushError::BadKeys)?;
        Ok(Self { public, auth })
    }
}

/// `payload` encrypted for `to`, as the body of the push request.
pub fn encrypt(to: &Receiver, payload: &[u8]) -> Result<Vec<u8>, PushError> {
    encrypt_with(to, payload, &random_key(), rand::random())
}

fn encrypt_with(
    to: &Receiver,
    payload: &[u8],
    ephemeral: &SecretKey,
    salt: [u8; 16],
) -> Result<Vec<u8>, PushError> {
    if payload.len() > MAX_PAYLOAD {
        return Err(PushError::TooLarge);
    }
    let ua_public = uncompressed(&to.public);
    let as_public = uncompressed(&ephemeral.public_key());
    let shared = p256::ecdh::diffie_hellman(ephemeral.to_nonzero_scalar(), to.public.as_affine());

    // RFC 8291 §3.3: the auth secret and both keys into the input key.
    let mut key_info = b"WebPush: info\0".to_vec();
    key_info.extend_from_slice(&ua_public);
    key_info.extend_from_slice(&as_public);
    let mut ikm = [0u8; 32];
    Hkdf::<Sha256>::new(Some(&to.auth), shared.raw_secret_bytes())
        .expand(&key_info, &mut ikm)
        .expect("32 bytes is a valid HKDF length");

    // RFC 8188: content key and nonce from the salt.
    let prk = Hkdf::<Sha256>::new(Some(&salt), &ikm);
    let mut cek = [0u8; 16];
    let mut nonce = [0u8; 12];
    prk.expand(b"Content-Encoding: aes128gcm\0", &mut cek)
        .expect("16 bytes is a valid HKDF length");
    prk.expand(b"Content-Encoding: nonce\0", &mut nonce)
        .expect("12 bytes is a valid HKDF length");

    // One record, so it is the last: delimiter 2, no padding.
    let mut plain = payload.to_vec();
    plain.push(2);
    let sealed = Aes128Gcm::new_from_slice(&cek)
        .expect("16-byte key")
        .encrypt(&nonce.into(), plain.as_slice())
        .map_err(|_| PushError::TooLarge)?;

    let mut body = Vec::with_capacity(16 + 4 + 1 + as_public.len() + sealed.len());
    body.extend_from_slice(&salt);
    body.extend_from_slice(&RECORD_SIZE.to_be_bytes());
    body.push(as_public.len() as u8);
    body.extend_from_slice(&as_public);
    body.extend_from_slice(&sealed);
    Ok(body)
}

/// The browser's side of [`encrypt`]: `private` is the browser key
/// (base64url, 32 bytes). Only the tests play browser.
#[doc(hidden)]
pub fn decrypt(private: &str, auth: &str, body: &[u8]) -> Option<Vec<u8>> {
    let secret = SecretKey::from_slice(&B64.decode(private).ok()?).ok()?;
    let auth: [u8; 16] = B64.decode(auth).ok()?.try_into().ok()?;
    let salt = body.get(..16)?;
    let id_len = *body.get(20)? as usize;
    let as_public = body.get(21..21 + id_len)?;
    let sealed = body.get(21 + id_len..)?;
    let sender = PublicKey::from_sec1_bytes(as_public).ok()?;
    let shared = p256::ecdh::diffie_hellman(secret.to_nonzero_scalar(), sender.as_affine());
    let mut key_info = b"WebPush: info\0".to_vec();
    key_info.extend_from_slice(&uncompressed(&secret.public_key()));
    key_info.extend_from_slice(as_public);
    let mut ikm = [0u8; 32];
    Hkdf::<Sha256>::new(Some(&auth), shared.raw_secret_bytes())
        .expand(&key_info, &mut ikm)
        .ok()?;
    let prk = Hkdf::<Sha256>::new(Some(salt), &ikm);
    let mut cek = [0u8; 16];
    let mut nonce = [0u8; 12];
    prk.expand(b"Content-Encoding: aes128gcm\0", &mut cek)
        .ok()?;
    prk.expand(b"Content-Encoding: nonce\0", &mut nonce).ok()?;
    let mut plain = Aes128Gcm::new_from_slice(&cek)
        .ok()?
        .decrypt(&nonce.into(), sealed)
        .ok()?;
    (plain.pop()? == 2).then_some(plain)
}

/// The server's own key pair, its "application server key".
#[derive(Clone)]
pub struct Vapid {
    key: SigningKey,
    /// Uncompressed public key, base64url: what browsers subscribe with.
    pub public: String,
    /// `mailto:` or `https:` contact for push services.
    pub subject: String,
}

impl std::fmt::Debug for Vapid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Vapid")
            .field("public", &self.public)
            .field("subject", &self.subject)
            .finish_non_exhaustive()
    }
}

impl Vapid {
    /// `private`: the raw 32-byte key, base64url (`gelabber-api vapid-keys`).
    pub fn parse(private: &str, subject: &str) -> Option<Self> {
        let raw = B64.decode(private.trim().trim_end_matches('=')).ok()?;
        let secret = SecretKey::from_slice(&raw).ok()?;
        Some(Self {
            public: B64.encode(uncompressed(&secret.public_key())),
            key: SigningKey::from(secret),
            subject: subject.to_owned(),
        })
    }

    /// A new key pair: the private half, base64url, and the public half.
    pub fn generate() -> (String, String) {
        let secret = random_key();
        (
            B64.encode(secret.to_bytes()),
            B64.encode(uncompressed(&secret.public_key())),
        )
    }

    /// `Authorization` for a request to `audience` (the endpoint's origin),
    /// valid until `expires` (Unix seconds).
    pub fn authorization(&self, audience: &str, expires: i64) -> String {
        let header = B64.encode(br#"{"typ":"JWT","alg":"ES256"}"#);
        let claims = B64.encode(
            serde_json::json!({ "aud": audience, "exp": expires, "sub": self.subject }).to_string(),
        );
        let signing_input = format!("{header}.{claims}");
        let signature: Signature = self.key.sign(signing_input.as_bytes());
        let jwt = format!("{signing_input}.{}", B64.encode(signature.to_bytes()));
        format!("vapid t={jwt}, k={}", self.public)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::ecdsa::{VerifyingKey, signature::Verifier};

    /// RFC 8291, Appendix A.
    #[test]
    fn encrypts_the_rfc_example() {
        let ua_private = SecretKey::from_slice(
            &B64.decode("q1dXpw3UpT5VOmu_cf_v6ih07Aems3njxI-JWgLcM94")
                .unwrap(),
        )
        .unwrap();
        let ua_public = B64.encode(uncompressed(&ua_private.public_key()));
        assert_eq!(
            ua_public,
            "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4"
        );
        let receiver = Receiver::parse(&ua_public, "BTBZMqHH6r4Tts7J_aSIgg").unwrap();
        let ephemeral = SecretKey::from_slice(
            &B64.decode("yfWPiYE-n46HLnH0KqZOF1fJJU3MYrct3AELtAQ-oRw")
                .unwrap(),
        )
        .unwrap();
        let salt: [u8; 16] = B64
            .decode("DGv6ra1nlYgDCS1FRnbzlw")
            .unwrap()
            .try_into()
            .unwrap();
        let body = encrypt_with(
            &receiver,
            b"When I grow up, I want to be a watermelon",
            &ephemeral,
            salt,
        )
        .unwrap();
        assert_eq!(
            B64.encode(body),
            "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPTpK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_Qulcy4a-fN"
        );
    }

    #[test]
    fn refuses_bad_keys_and_large_payloads() {
        assert_eq!(
            Receiver::parse("nope", "BTBZMqHH6r4Tts7J_aSIgg").err(),
            Some(PushError::BadKeys)
        );
        let (private, public) = Vapid::generate();
        let receiver = Receiver::parse(&public, "BTBZMqHH6r4Tts7J_aSIgg").unwrap();
        assert!(Receiver::parse(&public, "c2hvcnQ").is_err());
        assert!(encrypt(&receiver, &vec![b'x'; MAX_PAYLOAD]).is_ok());
        assert_eq!(
            encrypt(&receiver, &vec![b'x'; MAX_PAYLOAD + 1]),
            Err(PushError::TooLarge)
        );
        assert!(Vapid::parse(&private, "mailto:a@b.c").is_some());
        assert!(Vapid::parse("short", "mailto:a@b.c").is_none());
        assert_eq!(
            decrypt(
                &private,
                "BTBZMqHH6r4Tts7J_aSIgg",
                &encrypt(&receiver, b"hi").unwrap()
            ),
            Some(b"hi".to_vec())
        );
    }

    #[test]
    fn signs_a_vapid_token_the_public_key_verifies() {
        let (private, public) = Vapid::generate();
        let vapid = Vapid::parse(&private, "mailto:admin@example.org").unwrap();
        assert_eq!(vapid.public, public);
        let header = vapid.authorization("https://fcm.googleapis.com", 1_900_000_000);
        let (t, k) = header
            .strip_prefix("vapid t=")
            .unwrap()
            .split_once(", k=")
            .unwrap();
        assert_eq!(k, public);
        let (input, signature) = t.rsplit_once('.').unwrap();
        let claims: serde_json::Value =
            serde_json::from_slice(&B64.decode(input.split('.').nth(1).unwrap()).unwrap()).unwrap();
        assert_eq!(claims["aud"], "https://fcm.googleapis.com");
        assert_eq!(claims["sub"], "mailto:admin@example.org");
        let key = VerifyingKey::from_sec1_bytes(&B64.decode(k).unwrap()).unwrap();
        let signature = Signature::from_slice(&B64.decode(signature).unwrap()).unwrap();
        key.verify(input.as_bytes(), &signature).unwrap();
    }
}
