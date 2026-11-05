//! The update check of the Tauri app (desktop/app/src/update.rs): shortly
//! after the start and every six hours, `latest.json` of the latest release
//! is read; a newer version shows the update notice. What is
//! installed later must carry a minisign signature by `desktop/app/updater.pub`
//! made for the version announced ([`verify`]); installing is packaging's.

use std::time::Duration;

use serde::Deserialize;

const ENDPOINT: &str =
    "https://github.com/Till-und-Scholler-AI-slop/Gelabber/releases/latest/download/latest.json";
/// Where "Herunterladen" leads.
pub const RELEASES: &str = "https://github.com/Till-und-Scholler-AI-slop/Gelabber/releases/latest";
pub const FIRST_CHECK: Duration = Duration::from_secs(15);
pub const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);

/// The releases' key, or the build's own (`GELABBER_UPDATER_PUBKEY`, CI
/// test builds), as desktop/app/build.rs picks it.
const PUBKEY: &str = match option_env!("GELABBER_UPDATER_PUBKEY") {
    Some(key) => key,
    None => include_str!("../../../app/updater.pub"),
};

pub fn pubkey() -> &'static str {
    PUBKEY.trim()
}

/// `latest.json` as desktop-native.yml writes it (Tauri's format).
#[derive(Clone, Debug, Deserialize)]
pub struct Latest {
    pub version: String,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub platforms: std::collections::HashMap<String, Platform>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Platform {
    pub signature: String,
    pub url: String,
}

/// An update the check found.
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub version: String,
    pub notes: Option<String>,
    /// The signed package for this system, when the release has one.
    pub package: Option<(String, String)>,
}

/// Tauri's platform key: `linux-x86_64`, `windows-x86_64`, `darwin-aarch64`.
pub fn target() -> String {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        other => other,
    };
    format!("{os}-{}", std::env::consts::ARCH)
}

/// The newer version in `latest`, if any; `current` is this build's.
pub fn newer(latest: &Latest, current: &str, target: &str) -> Option<Found> {
    let parse = |v: &str| semver::Version::parse(v.trim().trim_start_matches('v')).ok();
    let (next, now) = (parse(&latest.version)?, parse(current)?);
    if next <= now {
        return None;
    }
    Some(Found {
        version: next.to_string(),
        notes: latest.notes.clone(),
        package: latest
            .platforms
            .get(target)
            .map(|p| (p.url.clone(), p.signature.clone())),
    })
}

/// The endpoint, or `GELABBER_UPDATE_ENDPOINT` to try a test release.
pub fn endpoint() -> String {
    std::env::var("GELABBER_UPDATE_ENDPOINT")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| ENDPOINT.to_owned())
}

/// One check over HTTPS.
pub async fn check(http: reqwest::Client) -> Result<Option<Found>, String> {
    let response = http
        .get(endpoint())
        .timeout(Duration::from_secs(60))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(format!("latest.json: {}", response.status()));
    }
    let bytes = response.bytes().await.map_err(|e| e.to_string())?;
    let latest: Latest = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    Ok(newer(&latest, env!("CARGO_PKG_VERSION"), &target()))
}

/// The updater plugin's check of a package (`verify_signature` with
/// `requireSignedVersion`): `signature` and `pubkey` as `tauri signer`
/// writes them (base64 of minisign's text), signed for `version`.
pub fn verify(data: &[u8], signature: &str, pubkey: &str, version: &str) -> Result<(), String> {
    use base64::Engine;
    let decode = |text: &str| {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(text.trim())
            .map_err(|e| e.to_string())?;
        String::from_utf8(bytes).map_err(|e| e.to_string())
    };
    let key = minisign_verify::PublicKey::decode(&decode(pubkey)?).map_err(|e| e.to_string())?;
    let signature =
        minisign_verify::Signature::decode(&decode(signature)?).map_err(|e| e.to_string())?;
    key.verify(data, &signature, true)
        .map_err(|e| e.to_string())?;
    let signed = signature
        .trusted_comment()
        .split('\t')
        .find_map(|field| field.strip_prefix("version:"))
        .ok_or("signed without a version")?;
    (signed.trim_start_matches('v') == version.trim_start_matches('v'))
        .then_some(())
        .ok_or_else(|| format!("signed for {signed}, not {version}"))
}

/// `auto_check` of the Tauri app's `update.json`, on unless switched off.
pub fn auto_check() -> bool {
    super::paths::read_json("update.json")
        .get("autoCheck")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(true)
}

/// Whether the next automatic check is due; a clock that went back counts.
pub fn due(last: Option<std::time::SystemTime>, now: std::time::SystemTime) -> bool {
    last.is_none_or(|last| {
        now.duration_since(last)
            .is_ok_and(|since| since >= CHECK_EVERY)
    }) || last.is_some_and(|last| last > now)
}

#[cfg(test)]
mod tests {
    use super::*;

    // `tauri signer sign --app-version 0.7.0` of b"gelabber", the test key
    // of desktop/app/src/update.rs.
    const KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEVBNjEyRDkzOEVDOTk3MjgKUldRb2w4bU9reTFoNmswZldrczNPK1kveENldW5IVGo4Qm9XQ3lqdzZFL2hhZlB2SFRZdmxrLzEK";
    const SIGNATURE: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVRb2w4bU9reTFoNm1ibm9XSmovRi9QNFBMcjdVZVQ3a1ozYVd3NFRxc2N1a3ExTmZHMkxWNUhuTDczS04vZm4wNG5EemhtWXg0NHphaXRxWUZoMWpDZnZPRFlaMzFlWUE0PQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzkyMTk0NjY2CWZpbGU6Zml4dHVyZQl2ZXJzaW9uOjAuNy4wCkN4KzVxdTlBaEg3Nzc0SmU0L0tnZGJPaVFNemJ4aUQ3VmpqMFRaaTQ5dEVrZ2V5dGtzT0N6YjZFcXlncFNlTXJtVVZVdlN4VGdMUGR0WGV4NFcwa0NnPT0K";

    #[test]
    fn signatures_hold_for_their_bytes_key_and_version_only() {
        assert_eq!(verify(b"gelabber", SIGNATURE, KEY, "0.7.0"), Ok(()));
        assert!(verify(b"gelabbex", SIGNATURE, KEY, "0.7.0").is_err());
        assert!(verify(b"gelabber", SIGNATURE, KEY, "0.7.1").is_err());
        assert!(verify(b"gelabber", "", KEY, "0.7.0").is_err());
        // The releases' key is a different one.
        assert!(!pubkey().is_empty());
        assert!(verify(b"gelabber", SIGNATURE, pubkey(), "0.7.0").is_err());
    }

    #[test]
    fn only_a_newer_version_is_offered_with_this_systems_package() {
        let latest: Latest = serde_json::from_str(
            r#"{"version":"v0.9.1","notes":"Neu","pub_date":"2026-11-01T00:00:00Z",
                "platforms":{"windows-x86_64":{"signature":"sig","url":"https://x/setup.exe"}}}"#,
        )
        .unwrap();
        let found = newer(&latest, "0.9.0", "windows-x86_64").unwrap();
        assert_eq!(found.version, "0.9.1");
        assert_eq!(found.notes.as_deref(), Some("Neu"));
        assert_eq!(
            found.package,
            Some(("https://x/setup.exe".into(), "sig".into()))
        );
        assert_eq!(
            newer(&latest, "0.9.0", "linux-x86_64").unwrap().package,
            None
        );
        assert!(newer(&latest, "0.9.1", "linux-x86_64").is_none());
        assert!(newer(&latest, "1.0.0", "linux-x86_64").is_none());
        assert!(newer(&latest, "kaputt", "linux-x86_64").is_none());
        assert!(target().contains('-'));
    }

    #[test]
    fn checks_are_due_every_six_hours_and_after_the_clock_went_back() {
        let now = std::time::SystemTime::now();
        assert!(due(None, now));
        assert!(!due(Some(now - Duration::from_secs(60)), now));
        assert!(due(Some(now - CHECK_EVERY), now));
        assert!(due(Some(now + Duration::from_secs(3600)), now));
    }

    #[test]
    fn a_check_reads_latest_json_from_the_endpoint() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 2048];
                let _ = stream.read(&mut buf);
                let body = r#"{"version":"99.0.0","platforms":{}}"#;
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        let _config = crate::platform::paths::testing::TempConfig::new("update");
        // SAFETY: the TempConfig lock keeps environment tests apart.
        unsafe {
            std::env::set_var(
                "GELABBER_UPDATE_ENDPOINT",
                format!("http://{addr}/latest.json"),
            )
        };
        let client = gelabber_client::Client::new("http://127.0.0.1:9").unwrap();
        let http = client.http().reqwest().clone();
        let found = client.block_on(check(http));
        // SAFETY: as above.
        unsafe { std::env::remove_var("GELABBER_UPDATE_ENDPOINT") };
        assert_eq!(found.unwrap().unwrap().version, "99.0.0");
        assert!(auto_check());
    }
}
