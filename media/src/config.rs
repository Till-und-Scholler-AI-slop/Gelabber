//! Runtime configuration. Everything comes from the environment (Compose
//! injects the same names as `media/.env.example`).

use std::fmt;
use std::net::SocketAddr;
use std::time::Duration;

use crate::ice::{IceServer, parse_ice_servers};

pub const MEDIA_ADDR: &str = "MEDIA_ADDR";
pub const REDIS_URL: &str = "REDIS_URL";
pub const MEDIA_READY_TIMEOUT_MS: &str = "MEDIA_READY_TIMEOUT_MS";
pub const MEDIA_ICE_BIND: &str = "MEDIA_ICE_BIND";
pub const MEDIA_ICE_PORT_MAX: &str = "MEDIA_ICE_PORT_MAX";
pub const MEDIA_ADVERTISED_IP: &str = "MEDIA_ADVERTISED_IP";
pub const TURN_URLS: &str = "TURN_URLS";
pub const TURN_USERNAME: &str = "TURN_USERNAME";
pub const TURN_PASSWORD: &str = "TURN_PASSWORD";
pub const RUST_LOG: &str = "RUST_LOG";

const DEFAULT_MEDIA_ADDR: &str = "0.0.0.0:8081";
const DEFAULT_READY_TIMEOUT_MS: u64 = 2000;
const DEFAULT_ICE_BIND: &str = "0.0.0.0:0";

#[derive(Debug, Clone)]
pub struct Config {
    pub media_addr: SocketAddr,
    pub redis_url: String,
    pub ready_timeout: Duration,
    /// Host ICE bind (`ip:port`). Port `0` is ephemeral (tests). A non-zero
    /// port is the first host UDP port; [`ice_port_max`] is the last.
    pub ice_bind: String,
    /// Inclusive end of the published host UDP range. `None` = only `ice_bind`.
    pub ice_port_max: Option<u16>,
    /// 1:1 NAT advertised address for host candidates. Empty = bind only.
    pub advertised_ip: Option<String>,
    pub ice_servers: Vec<IceServer>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    Missing(&'static str),
    Invalid {
        key: &'static str,
        value: String,
        reason: String,
    },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing(key) => write!(f, "missing required environment variable {key}"),
            Self::Invalid { key, value, reason } => {
                write!(f, "invalid value {value:?} for {key}: {reason}")
            }
        }
    }
}

impl std::error::Error for ConfigError {}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_source(|key| std::env::var(key).ok())
    }

    pub fn from_source(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let get = |key: &str| lookup(key).filter(|value| !value.trim().is_empty());

        let media_addr_raw = get(MEDIA_ADDR).unwrap_or_else(|| DEFAULT_MEDIA_ADDR.to_owned());
        let media_addr = media_addr_raw
            .parse::<SocketAddr>()
            .map_err(|err| invalid(MEDIA_ADDR, &media_addr_raw, err))?;

        let redis_url = get(REDIS_URL).ok_or(ConfigError::Missing(REDIS_URL))?;

        let ready_timeout_ms = match get(MEDIA_READY_TIMEOUT_MS) {
            Some(raw) => parse_positive::<u64>(MEDIA_READY_TIMEOUT_MS, &raw)?,
            None => DEFAULT_READY_TIMEOUT_MS,
        };

        let ice_bind = get(MEDIA_ICE_BIND).unwrap_or_else(|| DEFAULT_ICE_BIND.to_owned());
        let ice_addr = ice_bind.parse::<SocketAddr>().map_err(|_| {
            invalid(
                MEDIA_ICE_BIND,
                &ice_bind,
                "expected ip:port (port 0 is ephemeral)",
            )
        })?;

        let ice_port_max = match get(MEDIA_ICE_PORT_MAX) {
            Some(raw) => {
                let max = raw
                    .trim()
                    .parse::<u16>()
                    .map_err(|err| invalid(MEDIA_ICE_PORT_MAX, &raw, err))?;
                if ice_addr.port() == 0 {
                    return Err(invalid(
                        MEDIA_ICE_PORT_MAX,
                        &raw,
                        "requires a non-zero MEDIA_ICE_BIND port",
                    ));
                }
                if max < ice_addr.port() {
                    return Err(invalid(
                        MEDIA_ICE_PORT_MAX,
                        &raw,
                        "must be >= MEDIA_ICE_BIND port",
                    ));
                }
                Some(max)
            }
            None => None,
        };

        let advertised_ip = get(MEDIA_ADVERTISED_IP);
        let ice_servers = parse_ice_servers(
            get(TURN_URLS).as_deref(),
            get(TURN_USERNAME).as_deref(),
            get(TURN_PASSWORD).as_deref(),
        );

        Ok(Self {
            media_addr,
            redis_url,
            ready_timeout: Duration::from_millis(ready_timeout_ms),
            ice_bind,
            ice_port_max,
            advertised_ip,
            ice_servers,
        })
    }
}

fn parse_positive<T>(key: &'static str, raw: &str) -> Result<T, ConfigError>
where
    T: std::str::FromStr + PartialOrd + Default,
    T::Err: fmt::Display,
{
    let value = raw
        .trim()
        .parse::<T>()
        .map_err(|err| invalid(key, raw, err))?;
    if value <= T::default() {
        return Err(invalid(key, raw, "must be greater than zero"));
    }
    Ok(value)
}

fn invalid(key: &'static str, value: &str, reason: impl fmt::Display) -> ConfigError {
    ConfigError::Invalid {
        key,
        value: value.to_owned(),
        reason: reason.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn source(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |key| map.get(key).cloned()
    }

    #[test]
    fn applies_defaults() {
        let config =
            Config::from_source(source(&[(REDIS_URL, "redis://localhost")])).expect("valid config");
        assert_eq!(
            config.media_addr,
            "0.0.0.0:8081".parse::<SocketAddr>().unwrap()
        );
        assert_eq!(config.ice_bind, "0.0.0.0:0");
        assert!(config.ice_port_max.is_none());
        assert!(config.ice_servers.is_empty());
        assert!(config.advertised_ip.is_none());
    }

    #[test]
    fn parses_turn_and_advertised_ip() {
        let config = Config::from_source(source(&[
            (REDIS_URL, "redis://localhost"),
            (TURN_URLS, "stun:127.0.0.1:3478,turn:127.0.0.1:3478"),
            (TURN_USERNAME, "gelabber"),
            (TURN_PASSWORD, "gelabberturn"),
            (MEDIA_ADVERTISED_IP, "127.0.0.1"),
            (MEDIA_ICE_BIND, "127.0.0.1:0"),
        ]))
        .expect("valid");
        assert_eq!(config.ice_bind, "127.0.0.1:0");
        assert!(config.ice_port_max.is_none());
        assert_eq!(config.advertised_ip.as_deref(), Some("127.0.0.1"));
        assert_eq!(config.ice_servers.len(), 2);
        assert!(config.ice_servers[1].username.is_some());
    }

    #[test]
    fn parses_published_udp_range() {
        let config = Config::from_source(source(&[
            (REDIS_URL, "redis://localhost"),
            (MEDIA_ICE_BIND, "0.0.0.0:10000"),
            (MEDIA_ICE_PORT_MAX, "10031"),
            (MEDIA_ADVERTISED_IP, "127.0.0.1"),
        ]))
        .expect("valid");
        assert_eq!(config.ice_bind, "0.0.0.0:10000");
        assert_eq!(config.ice_port_max, Some(10031));
    }

    #[test]
    fn rejects_missing_redis() {
        let err = Config::from_source(source(&[])).unwrap_err();
        assert_eq!(err, ConfigError::Missing(REDIS_URL));
    }
}
