//! Runtime configuration. Everything comes from the environment (Compose
//! injects the same names as `media/.env.example`).

use std::fmt;
use std::net::SocketAddr;
use std::time::Duration;

pub const MEDIA_ADDR: &str = "MEDIA_ADDR";
pub const REDIS_URL: &str = "REDIS_URL";
pub const MEDIA_READY_TIMEOUT_MS: &str = "MEDIA_READY_TIMEOUT_MS";
pub const MEDIA_JOIN_DEADLINE_MS: &str = "MEDIA_JOIN_DEADLINE_MS";
pub const MEDIA_PENDING_PER_IP: &str = "MEDIA_PENDING_PER_IP";
pub const MEDIA_PENDING_MAX: &str = "MEDIA_PENDING_MAX";
pub const MEDIA_HEARTBEAT_MS: &str = "MEDIA_HEARTBEAT_MS";
pub const MEDIA_IDLE_TIMEOUT_MS: &str = "MEDIA_IDLE_TIMEOUT_MS";
pub const MEDIA_ICE_BIND: &str = "MEDIA_ICE_BIND";
pub const MEDIA_ICE_PORT_MAX: &str = "MEDIA_ICE_PORT_MAX";
pub const MEDIA_ADVERTISED_IP: &str = "MEDIA_ADVERTISED_IP";
pub const MEDIA_WORKERS: &str = "MEDIA_WORKERS";
pub const RUST_LOG: &str = "RUST_LOG";

const DEFAULT_MEDIA_ADDR: &str = "0.0.0.0:8081";
const DEFAULT_READY_TIMEOUT_MS: u64 = 2000;
const DEFAULT_JOIN_DEADLINE_MS: u64 = 10_000;
const DEFAULT_PENDING_PER_IP: usize = 16;
const DEFAULT_PENDING_MAX: usize = 1024;
const DEFAULT_HEARTBEAT_MS: u64 = 20_000;
const DEFAULT_IDLE_TIMEOUT_MS: u64 = 60_000;
const DEFAULT_ICE_BIND: &str = "0.0.0.0:10000";
/// Each worker is one native thread with its own UDP port.
const MAX_WORKERS: usize = 64;

#[derive(Debug, Clone)]
pub struct Config {
    pub media_addr: SocketAddr,
    pub redis_url: String,
    pub ready_timeout: Duration,
    /// How long a `/ws` socket may stay open without a joined room, both
    /// after the upgrade and after a leave.
    pub join_deadline: Duration,
    /// Sockets without a joined room one client address may hold.
    pub pending_per_ip: usize,
    /// Sockets without a joined room the whole SFU holds: a safety valve,
    /// far above what one address may hold.
    pub pending_max: usize,
    /// How often a joined socket is pinged.
    pub heartbeat: Duration,
    /// A joined socket from which nothing arrived for this long, Pongs
    /// included, is closed at the next heartbeat. Longer than `heartbeat`.
    pub idle_timeout: Duration,
    /// Native UDP listener of the first worker (`ip:port`). Worker `i` binds
    /// port + i. Test port `0` lets every worker select one port from the
    /// explicit 20000..=30000 range. Production defaults to UDP 10000.
    pub ice_bind: String,
    /// Upper end of the published UDP range. Required with more than one
    /// worker, so the published range covers every worker's port.
    pub ice_port_max: Option<u16>,
    /// Number of native mediasoup workers (`MEDIA_WORKERS`, default 1).
    pub workers: usize,
    /// 1:1 NAT advertised address for host candidates. Empty = bind only.
    pub advertised_ip: Option<String>,
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

        let join_deadline_ms = match get(MEDIA_JOIN_DEADLINE_MS) {
            Some(raw) => parse_positive::<u64>(MEDIA_JOIN_DEADLINE_MS, &raw)?,
            None => DEFAULT_JOIN_DEADLINE_MS,
        };

        let pending_per_ip = match get(MEDIA_PENDING_PER_IP) {
            Some(raw) => parse_positive::<usize>(MEDIA_PENDING_PER_IP, &raw)?,
            None => DEFAULT_PENDING_PER_IP,
        };

        let pending_max = match get(MEDIA_PENDING_MAX) {
            Some(raw) => parse_positive::<usize>(MEDIA_PENDING_MAX, &raw)?,
            None => DEFAULT_PENDING_MAX,
        };

        let heartbeat_ms = match get(MEDIA_HEARTBEAT_MS) {
            Some(raw) => parse_positive::<u64>(MEDIA_HEARTBEAT_MS, &raw)?,
            None => DEFAULT_HEARTBEAT_MS,
        };

        let idle_timeout_raw = get(MEDIA_IDLE_TIMEOUT_MS);
        let idle_timeout_ms = match &idle_timeout_raw {
            Some(raw) => parse_positive::<u64>(MEDIA_IDLE_TIMEOUT_MS, raw)?,
            None => DEFAULT_IDLE_TIMEOUT_MS,
        };
        if idle_timeout_ms <= heartbeat_ms {
            return Err(invalid(
                MEDIA_IDLE_TIMEOUT_MS,
                &idle_timeout_raw.unwrap_or_else(|| idle_timeout_ms.to_string()),
                "must be greater than MEDIA_HEARTBEAT_MS",
            ));
        }

        let ice_bind = get(MEDIA_ICE_BIND).unwrap_or_else(|| DEFAULT_ICE_BIND.to_owned());
        let ice_addr = ice_bind.parse::<SocketAddr>().map_err(|_| {
            invalid(
                MEDIA_ICE_BIND,
                &ice_bind,
                "expected ip:port (test port 0 selects from 20000..=30000)",
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

        let workers = match get(MEDIA_WORKERS) {
            Some(raw) => {
                let workers = parse_positive::<usize>(MEDIA_WORKERS, &raw)?;
                if workers > MAX_WORKERS {
                    return Err(invalid(
                        MEDIA_WORKERS,
                        &raw,
                        format!("must be at most {MAX_WORKERS}"),
                    ));
                }
                workers
            }
            None => 1,
        };
        if workers > 1 && ice_addr.port() != 0 {
            // Without a stated range an unpublished worker port would fail ICE
            // silently for the channels placed on that worker only.
            let last = u16::try_from(usize::from(ice_addr.port()) + workers - 1).map_err(|_| {
                invalid(MEDIA_WORKERS, &workers.to_string(), "UDP ports past 65535")
            })?;
            match ice_port_max {
                None => {
                    return Err(invalid(
                        MEDIA_WORKERS,
                        &workers.to_string(),
                        format!("needs {MEDIA_ICE_PORT_MAX} >= {last} (one UDP port per worker)"),
                    ));
                }
                Some(max) if max < last => {
                    return Err(invalid(
                        MEDIA_ICE_PORT_MAX,
                        &max.to_string(),
                        format!("must be >= {last} for {workers} workers"),
                    ));
                }
                Some(_) => {}
            }
        }

        let advertised_ip = get(MEDIA_ADVERTISED_IP);

        Ok(Self {
            media_addr,
            redis_url,
            ready_timeout: Duration::from_millis(ready_timeout_ms),
            join_deadline: Duration::from_millis(join_deadline_ms),
            pending_per_ip,
            pending_max,
            heartbeat: Duration::from_millis(heartbeat_ms),
            idle_timeout: Duration::from_millis(idle_timeout_ms),
            ice_bind,
            ice_port_max,
            workers,
            advertised_ip,
        })
    }

    /// The UDP ports the workers bind, `None` for test port 0.
    pub fn ice_ports(&self) -> Option<std::ops::RangeInclusive<u16>> {
        let port = self.ice_bind.parse::<SocketAddr>().ok()?.port();
        // `from_source` checked that the last port fits.
        (port != 0).then(|| port..=port + (self.workers - 1) as u16)
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
        assert_eq!(config.ice_bind, "0.0.0.0:10000");
        assert!(config.ice_port_max.is_none());
        assert!(config.advertised_ip.is_none());
        assert_eq!(config.join_deadline, Duration::from_secs(10));
        assert_eq!(config.workers, 1);
        assert_eq!(config.ice_ports(), Some(10000..=10000));
        assert_eq!(config.pending_per_ip, 16);
        assert_eq!(config.pending_max, 1024);
        assert_eq!(config.heartbeat, Duration::from_secs(20));
        assert_eq!(config.idle_timeout, Duration::from_secs(60));
    }

    #[test]
    fn parses_socket_limits() {
        let config = Config::from_source(source(&[
            (REDIS_URL, "redis://localhost"),
            (MEDIA_PENDING_PER_IP, "4"),
            (MEDIA_PENDING_MAX, "64"),
            (MEDIA_HEARTBEAT_MS, "100"),
            (MEDIA_IDLE_TIMEOUT_MS, "300"),
        ]))
        .expect("valid");
        assert_eq!(config.pending_per_ip, 4);
        assert_eq!(config.pending_max, 64);
        assert_eq!(config.heartbeat, Duration::from_millis(100));
        assert_eq!(config.idle_timeout, Duration::from_millis(300));
    }

    #[test]
    fn rejects_zero_socket_limits() {
        for key in [
            MEDIA_PENDING_PER_IP,
            MEDIA_PENDING_MAX,
            MEDIA_HEARTBEAT_MS,
            MEDIA_IDLE_TIMEOUT_MS,
        ] {
            let err = Config::from_source(source(&[(REDIS_URL, "redis://localhost"), (key, "0")]))
                .unwrap_err();
            assert!(
                matches!(&err, ConfigError::Invalid { key: bad, .. } if *bad == key),
                "{key}: {err}"
            );
        }
    }

    #[test]
    fn idle_timeout_must_outlast_the_heartbeat() {
        for pairs in [
            &[(MEDIA_HEARTBEAT_MS, "500"), (MEDIA_IDLE_TIMEOUT_MS, "500")][..],
            // A longer heartbeat alone must not pass the default timeout by.
            &[(MEDIA_HEARTBEAT_MS, "60000")][..],
        ] {
            let mut all = vec![(REDIS_URL, "redis://localhost")];
            all.extend_from_slice(pairs);
            let err = Config::from_source(source(&all)).unwrap_err();
            assert!(
                matches!(&err, ConfigError::Invalid { key, .. } if *key == MEDIA_IDLE_TIMEOUT_MS),
                "{pairs:?}: {err}"
            );
        }
    }

    #[test]
    fn worker_pool_needs_a_published_port_per_worker() {
        let config = Config::from_source(source(&[
            (REDIS_URL, "redis://localhost"),
            (MEDIA_WORKERS, "4"),
            (MEDIA_ICE_PORT_MAX, "10003"),
        ]))
        .expect("valid");
        assert_eq!(config.workers, 4);
        assert_eq!(config.ice_ports(), Some(10000..=10003));

        for (workers, max, key) in [
            ("4", None, MEDIA_WORKERS),
            ("4", Some("10002"), MEDIA_ICE_PORT_MAX),
            ("0", None, MEDIA_WORKERS),
            ("65", Some("10100"), MEDIA_WORKERS),
            ("two", None, MEDIA_WORKERS),
        ] {
            let mut pairs = vec![(REDIS_URL, "redis://localhost"), (MEDIA_WORKERS, workers)];
            if let Some(max) = max {
                pairs.push((MEDIA_ICE_PORT_MAX, max));
            }
            let err = Config::from_source(source(&pairs)).unwrap_err();
            assert!(
                matches!(&err, ConfigError::Invalid { key: found, .. } if *found == key),
                "{workers} {max:?}: {err}"
            );
        }

        let err = Config::from_source(source(&[
            (REDIS_URL, "redis://localhost"),
            (MEDIA_ICE_BIND, "0.0.0.0:65534"),
            (MEDIA_WORKERS, "3"),
            (MEDIA_ICE_PORT_MAX, "65535"),
        ]))
        .unwrap_err();
        assert!(matches!(&err, ConfigError::Invalid { key, .. } if *key == MEDIA_WORKERS));
    }

    #[test]
    fn test_port_zero_pool_needs_no_range() {
        let config = Config::from_source(source(&[
            (REDIS_URL, "redis://localhost"),
            (MEDIA_ICE_BIND, "127.0.0.1:0"),
            (MEDIA_WORKERS, "3"),
        ]))
        .expect("valid");
        assert_eq!(config.workers, 3);
        assert_eq!(config.ice_ports(), None);
    }

    #[test]
    fn parses_join_deadline() {
        let config = Config::from_source(source(&[
            (REDIS_URL, "redis://localhost"),
            (MEDIA_JOIN_DEADLINE_MS, "250"),
        ]))
        .expect("valid");
        assert_eq!(config.join_deadline, Duration::from_millis(250));

        let blank = Config::from_source(source(&[
            (REDIS_URL, "redis://localhost"),
            (MEDIA_JOIN_DEADLINE_MS, "  "),
        ]))
        .expect("blank falls back to the default");
        assert_eq!(blank.join_deadline, Duration::from_secs(10));
    }

    #[test]
    fn rejects_zero_or_non_numeric_join_deadline() {
        for raw in ["0", "abc"] {
            let err = Config::from_source(source(&[
                (REDIS_URL, "redis://localhost"),
                (MEDIA_JOIN_DEADLINE_MS, raw),
            ]))
            .unwrap_err();
            assert!(
                matches!(&err, ConfigError::Invalid { key, .. } if *key == MEDIA_JOIN_DEADLINE_MS),
                "{raw}: {err}"
            );
        }
    }

    #[test]
    fn parses_advertised_ip() {
        let config = Config::from_source(source(&[
            (REDIS_URL, "redis://localhost"),
            (MEDIA_ADVERTISED_IP, "127.0.0.1"),
            (MEDIA_ICE_BIND, "127.0.0.1:0"),
        ]))
        .expect("valid");
        assert_eq!(config.ice_bind, "127.0.0.1:0");
        assert!(config.ice_port_max.is_none());
        assert_eq!(config.advertised_ip.as_deref(), Some("127.0.0.1"));
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
