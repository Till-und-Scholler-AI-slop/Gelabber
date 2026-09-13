//! Runtime configuration. Everything comes from the environment (Compose
//! injects the same names as `api/.env.example`); there is no config file.

use std::fmt;
use std::net::SocketAddr;
use std::time::Duration;

pub const API_ADDR: &str = "API_ADDR";
pub const DATABASE_URL: &str = "DATABASE_URL";
pub const REDIS_URL: &str = "REDIS_URL";
pub const API_READY_TIMEOUT_MS: &str = "API_READY_TIMEOUT_MS";
pub const API_DB_MAX_CONNECTIONS: &str = "API_DB_MAX_CONNECTIONS";
/// Read by `telemetry::init`, not by `Config`, because the subscriber has to
/// exist before anything else can be logged.
pub const RUST_LOG: &str = "RUST_LOG";

const DEFAULT_API_ADDR: &str = "0.0.0.0:8080";
const DEFAULT_READY_TIMEOUT_MS: u64 = 2000;
const DEFAULT_DB_MAX_CONNECTIONS: u32 = 5;

#[derive(Debug, Clone)]
pub struct Config {
    pub api_addr: SocketAddr,
    pub database_url: String,
    pub redis_url: String,
    /// Upper bound for each individual `/ready` dependency check.
    pub ready_timeout: Duration,
    pub db_max_connections: u32,
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

        Ok(Self {
            api_addr,
            database_url,
            redis_url,
            ready_timeout: Duration::from_millis(ready_timeout_ms),
            db_max_connections,
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
        let config = Config::from_source(source(&[
            (DATABASE_URL, "postgres://u:p@localhost/db"),
            (REDIS_URL, "redis://localhost"),
        ]))
        .expect("valid config");

        assert_eq!(config.api_addr, "0.0.0.0:8080".parse().unwrap());
        assert_eq!(config.ready_timeout, Duration::from_millis(2000));
        assert_eq!(config.db_max_connections, 5);
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
