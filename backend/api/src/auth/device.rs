//! The device behind a session (B2), parsed from `User-Agent` when the
//! session is issued (`auth::signed_in`). Only the two codes are stored, so
//! each client words them in the reader's language; the raw User-Agent is
//! never stored or logged.

use axum::http::HeaderMap;
use axum::http::header::USER_AGENT;
use serde::{Deserialize, Serialize};

/// The app or browser. The codes match `sessions.device_client`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "lowercase")]
#[sqlx(type_name = "text", rename_all = "lowercase")]
pub enum DeviceClient {
    /// The native app (`Gelabber-Desktop/<version> (<os>)`).
    Desktop,
    Firefox,
    Chrome,
    Edge,
    Safari,
    /// Another browser (`Mozilla/` without a known engine token).
    Browser,
    Unknown,
}

/// The operating system. The codes match `sessions.device_os`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "lowercase")]
#[sqlx(type_name = "text", rename_all = "lowercase")]
pub enum DeviceOs {
    Linux,
    Windows,
    Macos,
    Android,
    Ios,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Device {
    pub client: DeviceClient,
    pub os: DeviceOs,
}

const DESKTOP_PREFIX: &str = "Gelabber-Desktop/";

impl Device {
    pub const UNKNOWN: Self = Self {
        client: DeviceClient::Unknown,
        os: DeviceOs::Other,
    };

    /// The device a request comes from; [`Self::UNKNOWN`] without a
    /// readable `User-Agent`.
    pub fn from_headers(headers: &HeaderMap) -> Self {
        headers
            .get(USER_AGENT)
            .and_then(|value| value.to_str().ok())
            .map_or(Self::UNKNOWN, Self::parse)
    }

    pub fn parse(agent: &str) -> Self {
        if let Some(rest) = agent.strip_prefix(DESKTOP_PREFIX) {
            return Self {
                client: DeviceClient::Desktop,
                os: desktop_os(rest),
            };
        }
        let client = if agent.contains("Edg/") {
            DeviceClient::Edge
        } else if agent.contains("Firefox/") {
            DeviceClient::Firefox
        } else if agent.contains("Chrome/") || agent.contains("Chromium/") {
            DeviceClient::Chrome
        } else if agent.contains("Safari/") && !agent.contains("Chrome") {
            DeviceClient::Safari
        } else if agent.starts_with("Mozilla/") {
            DeviceClient::Browser
        } else {
            return Self::UNKNOWN;
        };
        Self {
            client,
            os: browser_os(agent),
        }
    }
}

/// The parenthesis after the native app's version; an app older than v0.10
/// sends none.
fn desktop_os(rest: &str) -> DeviceOs {
    let named = rest
        .split_once('(')
        .and_then(|(_, tail)| tail.split_once(')'))
        .map(|(os, _)| os.trim());
    match named {
        Some("linux") => DeviceOs::Linux,
        Some("windows") => DeviceOs::Windows,
        Some("macos") => DeviceOs::Macos,
        _ => DeviceOs::Other,
    }
}

fn browser_os(agent: &str) -> DeviceOs {
    let any = |needles: &[&str]| needles.iter().any(|needle| agent.contains(needle));
    if any(&["Android"]) {
        DeviceOs::Android
    } else if any(&["iPhone", "iPad", "iPod"]) {
        DeviceOs::Ios
    } else if any(&["Windows"]) {
        DeviceOs::Windows
    } else if any(&["Mac OS X", "Macintosh"]) {
        DeviceOs::Macos
    } else if any(&["Linux", "X11"]) {
        DeviceOs::Linux
    } else {
        DeviceOs::Other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(agent: &str) -> (DeviceClient, DeviceOs) {
        let device = Device::parse(agent);
        (device.client, device.os)
    }

    #[test]
    fn firefox_on_each_system() {
        use DeviceOs::*;
        for (agent, os) in [
            (
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:132.0) Gecko/20100101 Firefox/132.0",
                Windows,
            ),
            (
                "Mozilla/5.0 (X11; Linux x86_64; rv:132.0) Gecko/20100101 Firefox/132.0",
                Linux,
            ),
            (
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 14.7; rv:132.0) Gecko/20100101 Firefox/132.0",
                Macos,
            ),
            (
                "Mozilla/5.0 (Android 14; Mobile; rv:132.0) Gecko/132.0 Firefox/132.0",
                Android,
            ),
        ] {
            assert_eq!(device(agent), (DeviceClient::Firefox, os), "{agent}");
        }
    }

    /// Firefox on iOS is WebKit and says `FxiOS`, not `Firefox/`: it reads
    /// as Safari, which is what the engine is.
    #[test]
    fn firefox_for_ios_reads_as_safari() {
        let agent = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_1 like Mac OS X) AppleWebKit/605.1.15 \
                     (KHTML, like Gecko) FxiOS/132.0 Mobile/15E148 Safari/605.1.15";
        assert_eq!(device(agent), (DeviceClient::Safari, DeviceOs::Ios));
    }

    #[test]
    fn chrome_on_each_system() {
        use DeviceOs::*;
        for (agent, os) in [
            (
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) \
                 Chrome/130.0.0.0 Safari/537.36",
                Windows,
            ),
            (
                "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) \
                 Chrome/130.0.0.0 Safari/537.36",
                Linux,
            ),
            (
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 \
                 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36",
                Macos,
            ),
            (
                "Mozilla/5.0 (Linux; Android 10; K) AppleWebKit/537.36 (KHTML, like Gecko) \
                 Chrome/130.0.0.0 Mobile Safari/537.36",
                Android,
            ),
            (
                "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) \
                 Chromium/130.0.0.0 Safari/537.36",
                Linux,
            ),
        ] {
            assert_eq!(device(agent), (DeviceClient::Chrome, os), "{agent}");
        }
    }

    #[test]
    fn edge_wins_over_chrome() {
        use DeviceOs::*;
        for (agent, os) in [
            (
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) \
                 Chrome/130.0.0.0 Safari/537.36 Edg/130.0.0.0",
                Windows,
            ),
            (
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 \
                 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36 Edg/130.0.0.0",
                Macos,
            ),
            (
                "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) \
                 Chrome/130.0.0.0 Safari/537.36 Edg/130.0.0.0",
                Linux,
            ),
        ] {
            assert_eq!(device(agent), (DeviceClient::Edge, os), "{agent}");
        }
    }

    #[test]
    fn safari_on_mac_and_ios() {
        use DeviceOs::*;
        for (agent, os) in [
            (
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 \
                 (KHTML, like Gecko) Version/18.1 Safari/605.1.15",
                Macos,
            ),
            (
                "Mozilla/5.0 (iPhone; CPU iPhone OS 18_1 like Mac OS X) AppleWebKit/605.1.15 \
                 (KHTML, like Gecko) Version/18.1 Mobile/15E148 Safari/604.1",
                Ios,
            ),
            (
                "Mozilla/5.0 (iPad; CPU OS 18_1 like Mac OS X) AppleWebKit/605.1.15 \
                 (KHTML, like Gecko) Version/18.1 Mobile/15E148 Safari/604.1",
                Ios,
            ),
        ] {
            assert_eq!(device(agent), (DeviceClient::Safari, os), "{agent}");
        }
    }

    /// Chrome on iOS says `CriOS` and `Safari/` but not `Chrome`.
    #[test]
    fn chrome_for_ios_reads_as_safari() {
        let agent = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_1 like Mac OS X) AppleWebKit/605.1.15 \
                     (KHTML, like Gecko) CriOS/130.0.6723.90 Mobile/15E148 Safari/604.1";
        assert_eq!(device(agent), (DeviceClient::Safari, DeviceOs::Ios));
    }

    #[test]
    fn another_browser_is_a_browser() {
        let agent = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/538.1 (KHTML, like Gecko)";
        assert_eq!(device(agent), (DeviceClient::Browser, DeviceOs::Linux));
    }

    #[test]
    fn the_native_app_names_its_system() {
        use DeviceOs::*;
        for (agent, os) in [
            ("Gelabber-Desktop/0.10.0 (windows)", Windows),
            ("Gelabber-Desktop/0.10.0 (linux)", Linux),
            ("Gelabber-Desktop/0.10.0 (macos)", Macos),
            ("Gelabber-Desktop/0.10.0 (plan9)", Other),
        ] {
            assert_eq!(device(agent), (DeviceClient::Desktop, os), "{agent}");
        }
    }

    #[test]
    fn an_older_native_app_has_no_system() {
        assert_eq!(
            device("Gelabber-Desktop/0.9.3"),
            (DeviceClient::Desktop, DeviceOs::Other)
        );
    }

    #[test]
    fn anything_else_is_unknown() {
        assert_eq!(device(""), (DeviceClient::Unknown, DeviceOs::Other));
        assert_eq!(
            device("curl/8.10.1"),
            (DeviceClient::Unknown, DeviceOs::Other)
        );
        assert_eq!(Device::from_headers(&HeaderMap::new()), Device::UNKNOWN);
    }

    #[test]
    fn codes_match_the_columns() {
        assert_eq!(
            serde_json::to_value(Device::parse("Gelabber-Desktop/0.10.0 (macos)")).unwrap(),
            serde_json::json!({ "client": "desktop", "os": "macos" })
        );
    }
}
