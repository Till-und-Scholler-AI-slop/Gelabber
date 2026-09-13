//! Runtime configuration. Everything comes from the environment (Compose
//! injects the same names as `api/.env.example`); there is no config file.

use std::fmt;
use std::net::SocketAddr;
use std::time::Duration;

use crate::media::{IceServer, parse_ice_servers};

pub const API_ADDR: &str = "API_ADDR";
pub const DATABASE_URL: &str = "DATABASE_URL";
pub const REDIS_URL: &str = "REDIS_URL";
pub const API_READY_TIMEOUT_MS: &str = "API_READY_TIMEOUT_MS";
pub const API_DB_MAX_CONNECTIONS: &str = "API_DB_MAX_CONNECTIONS";
pub const API_COOKIE_SECURE: &str = "API_COOKIE_SECURE";
pub const API_SESSION_TTL_HOURS: &str = "API_SESSION_TTL_HOURS";
pub const API_WS_HEARTBEAT_MS: &str = "API_WS_HEARTBEAT_MS";
pub const API_WS_DEAD_MS: &str = "API_WS_DEAD_MS";
pub const API_WS_REPLAY: &str = "API_WS_REPLAY";
pub const API_WS_IDLE_MS: &str = "API_WS_IDLE_MS";
pub const API_WS_PRESENCE_TTL_MS: &str = "API_WS_PRESENCE_TTL_MS";
pub const API_WS_TYPING_TTL_MS: &str = "API_WS_TYPING_TTL_MS";
pub const TURN_URLS: &str = "TURN_URLS";
pub const TURN_USERNAME: &str = "TURN_USERNAME";
pub const TURN_PASSWORD: &str = "TURN_PASSWORD";
pub const MEDIA_TICKET_TTL_SECS: &str = "MEDIA_TICKET_TTL_SECS";
pub const MINIO_ENDPOINT: &str = "MINIO_ENDPOINT";
pub const MINIO_PUBLIC_ENDPOINT: &str = "MINIO_PUBLIC_ENDPOINT";
pub const MINIO_ROOT_USER: &str = "MINIO_ROOT_USER";
pub const MINIO_ROOT_PASSWORD: &str = "MINIO_ROOT_PASSWORD";
pub const MINIO_BUCKET: &str = "MINIO_BUCKET";
/// Read by `telemetry::init`, not by `Config`, because the subscriber has to
/// exist before anything else can be logged.
pub const RUST_LOG: &str = "RUST_LOG";

const DEFAULT_API_ADDR: &str = "0.0.0.0:8080";
const DEFAULT_READY_TIMEOUT_MS: u64 = 2000;
const DEFAULT_DB_MAX_CONNECTIONS: u32 = 5;
const DEFAULT_SESSION_TTL_HOURS: u64 = 24 * 30;
const DEFAULT_WS_HEARTBEAT_MS: u64 = 15_000;
const DEFAULT_WS_DEAD_MS: u64 = 30_000;
const DEFAULT_WS_REPLAY: usize = 256;
const DEFAULT_WS_IDLE_MS: u64 = 300_000;
const DEFAULT_WS_PRESENCE_TTL_MS: u64 = 45_000;
const DEFAULT_WS_TYPING_TTL_MS: u64 = 6_000;
const DEFAULT_MEDIA_TICKET_TTL_SECS: u64 = 30;

#[derive(Debug, Clone)]
pub struct Config {
    pub api_addr: SocketAddr,
    pub database_url: String,
    pub redis_url: String,
    /// Upper bound for each individual `/ready` dependency check.
    pub ready_timeout: Duration,
    pub db_max_connections: u32,
    /// Adds `Secure` to the session and CSRF cookies. Off by default because
    /// the Compose dev entry is plain HTTP on Caddy; set to `true` behind TLS.
    pub cookie_secure: bool,
    /// Lifetime of a session cookie and its database row.
    pub session_ttl: Duration,
    /// How often the gateway sends an application heartbeat (`{"op":"h"}`).
    pub ws_heartbeat: Duration,
    /// Close a socket that has sent nothing for this long (silent death).
    pub ws_dead: Duration,
    /// Bounded Redis replay buffer per topic, for reconnect catch-up.
    pub ws_replay: usize,
    /// Per-connection idle timeout. Heartbeat is liveness only and does
    /// not reset this; after it elapses the connection is `idle`.
    pub ws_idle: Duration,
    /// Redis TTL for presence keys. Must outlive the heartbeat so an idle
    /// but connected client does not fall out of Redis.
    pub ws_presence_ttl: Duration,
    /// Redis TTL for a typing key. Clients also hide locally after this.
    pub ws_typing_ttl: Duration,
    /// Browser-facing STUN/TURN (coturn). Empty if TURN is not configured.
    pub ice_servers: Vec<IceServer>,
    /// How long an SFU join ticket lives in Redis.
    pub media_ticket_ttl: Duration,
    /// MinIO / S3-compatible store for attachments. Absent in unit tests.
    pub minio: Option<MinioConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MinioConfig {
    /// API → MinIO (Compose: `http://minio:9000`).
    pub endpoint: String,
    /// Browser-facing host used on presigned URLs (Compose: `http://localhost:9000`).
    pub public_endpoint: String,
    pub access_key: String,
    pub secret_key: String,
    pub bucket: String,
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

    /// Builds the config from an arbitrary lookup so tests do not have to
    /// mutate process-global environment variables.
    pub fn from_source(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let get = |key: &str| lookup(key).filter(|value| !value.trim().is_empty());

        let api_addr_raw = get(API_ADDR).unwrap_or_else(|| DEFAULT_API_ADDR.to_owned());
        let api_addr = api_addr_raw
            .parse::<SocketAddr>()
            .map_err(|err| invalid(API_ADDR, &api_addr_raw, err))?;

        let database_url = get(DATABASE_URL).ok_or(ConfigError::Missing(DATABASE_URL))?;
        let redis_url = get(REDIS_URL).ok_or(ConfigError::Missing(REDIS_URL))?;

        let ready_timeout_ms = match get(API_READY_TIMEOUT_MS) {
            Some(raw) => parse_positive::<u64>(API_READY_TIMEOUT_MS, &raw)?,
            None => DEFAULT_READY_TIMEOUT_MS,
        };

        let db_max_connections = match get(API_DB_MAX_CONNECTIONS) {
            Some(raw) => parse_positive::<u32>(API_DB_MAX_CONNECTIONS, &raw)?,
            None => DEFAULT_DB_MAX_CONNECTIONS,
        };

        let cookie_secure = match get(API_COOKIE_SECURE) {
            Some(raw) => parse_bool(API_COOKIE_SECURE, &raw)?,
            None => false,
        };

        let session_ttl_hours = match get(API_SESSION_TTL_HOURS) {
            Some(raw) => parse_positive::<u64>(API_SESSION_TTL_HOURS, &raw)?,
            None => DEFAULT_SESSION_TTL_HOURS,
        };

        let ws_heartbeat_ms = match get(API_WS_HEARTBEAT_MS) {
            Some(raw) => parse_positive::<u64>(API_WS_HEARTBEAT_MS, &raw)?,
            None => DEFAULT_WS_HEARTBEAT_MS,
        };
        let ws_dead_ms = match get(API_WS_DEAD_MS) {
            Some(raw) => parse_positive::<u64>(API_WS_DEAD_MS, &raw)?,
            None => DEFAULT_WS_DEAD_MS,
        };
        if ws_dead_ms <= ws_heartbeat_ms {
            return Err(invalid(
                API_WS_DEAD_MS,
                &ws_dead_ms.to_string(),
                "must be greater than API_WS_HEARTBEAT_MS",
            ));
        }
        let ws_replay = match get(API_WS_REPLAY) {
            Some(raw) => parse_positive::<usize>(API_WS_REPLAY, &raw)?,
            None => DEFAULT_WS_REPLAY,
        };
        let ws_idle_ms = match get(API_WS_IDLE_MS) {
            Some(raw) => parse_positive::<u64>(API_WS_IDLE_MS, &raw)?,
            None => DEFAULT_WS_IDLE_MS,
        };
        let ws_presence_ttl_ms = match get(API_WS_PRESENCE_TTL_MS) {
            Some(raw) => parse_positive::<u64>(API_WS_PRESENCE_TTL_MS, &raw)?,
            None => DEFAULT_WS_PRESENCE_TTL_MS,
        };
        if ws_presence_ttl_ms <= ws_heartbeat_ms {
            return Err(invalid(
                API_WS_PRESENCE_TTL_MS,
                &ws_presence_ttl_ms.to_string(),
                "must be greater than API_WS_HEARTBEAT_MS",
            ));
        }
        let ws_typing_ttl_ms = match get(API_WS_TYPING_TTL_MS) {
            Some(raw) => parse_positive::<u64>(API_WS_TYPING_TTL_MS, &raw)?,
            None => DEFAULT_WS_TYPING_TTL_MS,
        };

        let ice_servers = parse_ice_servers(
            get(TURN_URLS).as_deref(),
            get(TURN_USERNAME).as_deref(),
            get(TURN_PASSWORD).as_deref(),
        );
        let media_ticket_ttl_secs = match get(MEDIA_TICKET_TTL_SECS) {
            Some(raw) => parse_positive::<u64>(MEDIA_TICKET_TTL_SECS, &raw)?,
            None => DEFAULT_MEDIA_TICKET_TTL_SECS,
        };
        let minio = parse_minio(&get)?;

        Ok(Self {
            api_addr,
            database_url,
            redis_url,
            ready_timeout: Duration::from_millis(ready_timeout_ms),
            db_max_connections,
            cookie_secure,
            session_ttl: Duration::from_secs(session_ttl_hours * 3600),
            ws_heartbeat: Duration::from_millis(ws_heartbeat_ms),
            ws_dead: Duration::from_millis(ws_dead_ms),
            ws_replay,
            ws_idle: Duration::from_millis(ws_idle_ms),
            ws_presence_ttl: Duration::from_millis(ws_presence_ttl_ms),
            ws_typing_ttl: Duration::from_millis(ws_typing_ttl_ms),
            ice_servers,
            media_ticket_ttl: Duration::from_secs(media_ticket_ttl_secs),
            minio,
        })
    }
}

fn parse_minio(get: &impl Fn(&str) -> Option<String>) -> Result<Option<MinioConfig>, ConfigError> {
    let endpoint = get(MINIO_ENDPOINT);
    let user = get(MINIO_ROOT_USER);
    let password = get(MINIO_ROOT_PASSWORD);
    let bucket = get(MINIO_BUCKET);
    match (endpoint, user, password, bucket) {
        (None, None, None, None) => Ok(None),
        (Some(endpoint), Some(access_key), Some(secret_key), Some(bucket)) => {
            if !endpoint.starts_with("http://") && !endpoint.starts_with("https://") {
                return Err(invalid(MINIO_ENDPOINT, &endpoint, "must be an http(s) URL"));
            }
            let public_endpoint = get(MINIO_PUBLIC_ENDPOINT).unwrap_or_else(|| endpoint.clone());
            if !public_endpoint.starts_with("http://") && !public_endpoint.starts_with("https://") {
                return Err(invalid(
                    MINIO_PUBLIC_ENDPOINT,
                    &public_endpoint,
                    "must be an http(s) URL",
                ));
            }
            Ok(Some(MinioConfig {
                endpoint,
                public_endpoint,
                access_key,
                secret_key,
                bucket,
            }))
        }
        _ => Err(ConfigError::Missing(MINIO_ENDPOINT)),
    }
}

fn parse_bool(key: &'static str, raw: &str) -> Result<bool, ConfigError> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(invalid(key, raw, "expected true or false")),
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
        let config = Config::from_source(source(&[
            (DATABASE_URL, "postgres://u:p@localhost/db"),
            (REDIS_URL, "redis://localhost"),
        ]))
        .expect("valid config");

        assert_eq!(config.api_addr, "0.0.0.0:8080".parse().unwrap());
        assert_eq!(config.ready_timeout, Duration::from_millis(2000));
        assert_eq!(config.db_max_connections, 5);
        assert!(!config.cookie_secure);
        assert_eq!(config.session_ttl, Duration::from_secs(30 * 24 * 3600));
        assert_eq!(config.ws_heartbeat, Duration::from_millis(15_000));
        assert_eq!(config.ws_dead, Duration::from_millis(30_000));
        assert_eq!(config.ws_replay, 256);
        assert_eq!(config.ws_idle, Duration::from_millis(300_000));
        assert_eq!(config.ws_presence_ttl, Duration::from_millis(45_000));
        assert_eq!(config.ws_typing_ttl, Duration::from_millis(6_000));
        assert!(config.ice_servers.is_empty());
        assert_eq!(config.media_ticket_ttl, Duration::from_secs(30));
        assert!(config.minio.is_none());
    }

    #[test]
    fn parses_minio_and_defaults_public_endpoint() {
        let config = Config::from_source(source(&[
            (DATABASE_URL, "postgres://u:p@localhost/db"),
            (REDIS_URL, "redis://localhost"),
            (MINIO_ENDPOINT, "http://minio:9000"),
            (MINIO_ROOT_USER, "gelabber"),
            (MINIO_ROOT_PASSWORD, "gelabbergelabber"),
            (MINIO_BUCKET, "gelabber"),
        ]))
        .expect("valid config");
        let minio = config.minio.expect("minio");
        assert_eq!(minio.endpoint, "http://minio:9000");
        assert_eq!(minio.public_endpoint, "http://minio:9000");
        assert_eq!(minio.bucket, "gelabber");
    }

    #[test]
    fn parses_cookie_and_session_settings() {
        let config = Config::from_source(source(&[
            (DATABASE_URL, "postgres://u:p@localhost/db"),
            (REDIS_URL, "redis://localhost"),
            (API_COOKIE_SECURE, "true"),
            (API_SESSION_TTL_HOURS, "12"),
        ]))
        .expect("valid config");

        assert!(config.cookie_secure);
        assert_eq!(config.session_ttl, Duration::from_secs(12 * 3600));
    }

    #[test]
    fn rejects_unparseable_cookie_secure() {
        let err = Config::from_source(source(&[
            (DATABASE_URL, "postgres://u:p@localhost/db"),
            (REDIS_URL, "redis://localhost"),
            (API_COOKIE_SECURE, "maybe"),
        ]))
        .unwrap_err();
        assert!(matches!(
            err,
            ConfigError::Invalid {
                key: API_COOKIE_SECURE,
                ..
            }
        ));
    }

    #[test]
    fn rejects_missing_database_url() {
        let err = Config::from_source(source(&[(REDIS_URL, "redis://localhost")])).unwrap_err();
        assert_eq!(err, ConfigError::Missing(DATABASE_URL));
        assert!(err.to_string().contains("DATABASE_URL"));
    }

    #[test]
    fn treats_blank_values_as_missing() {
        let err = Config::from_source(source(&[
            (DATABASE_URL, "postgres://u:p@localhost/db"),
            (REDIS_URL, "   "),
        ]))
        .unwrap_err();
        assert_eq!(err, ConfigError::Missing(REDIS_URL));
    }

    #[test]
    fn rejects_non_positive_timeout() {
        let err = Config::from_source(source(&[
            (DATABASE_URL, "postgres://u:p@localhost/db"),
            (REDIS_URL, "redis://localhost"),
            (API_READY_TIMEOUT_MS, "0"),
        ]))
        .unwrap_err();
        assert!(matches!(
            err,
            ConfigError::Invalid {
                key: API_READY_TIMEOUT_MS,
                ..
            }
        ));
    }

    #[test]
    fn rejects_presence_ttl_not_greater_than_heartbeat() {
        let err = Config::from_source(source(&[
            (DATABASE_URL, "postgres://u:p@localhost/db"),
            (REDIS_URL, "redis://localhost"),
            (API_WS_HEARTBEAT_MS, "100"),
            (API_WS_PRESENCE_TTL_MS, "100"),
        ]))
        .unwrap_err();
        assert!(matches!(
            err,
            ConfigError::Invalid {
                key: API_WS_PRESENCE_TTL_MS,
                ..
            }
        ));
    }

    #[test]
    fn rejects_dead_timeout_not_greater_than_heartbeat() {
        let err = Config::from_source(source(&[
            (DATABASE_URL, "postgres://u:p@localhost/db"),
            (REDIS_URL, "redis://localhost"),
            (API_WS_HEARTBEAT_MS, "100"),
            (API_WS_DEAD_MS, "100"),
        ]))
        .unwrap_err();
        assert!(matches!(
            err,
            ConfigError::Invalid {
                key: API_WS_DEAD_MS,
                ..
            }
        ));
    }

    #[test]
    fn rejects_unparseable_addr() {
        let err = Config::from_source(source(&[
            (API_ADDR, "not-an-addr"),
            (DATABASE_URL, "postgres://u:p@localhost/db"),
            (REDIS_URL, "redis://localhost"),
        ]))
        .unwrap_err();
        assert!(matches!(err, ConfigError::Invalid { key: API_ADDR, .. }));
    }
}
