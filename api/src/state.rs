use std::fmt;
use std::str::FromStr;
use std::time::Duration;

use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};

use crate::config::Config;

/// Shared handles for request handlers. Both clients are created lazily so
/// the process boots even while Postgres/Redis are still starting; `/ready`
/// reports the live state instead.
#[derive(Clone)]
pub struct AppState {
    /// Connection pool for domain queries (later issues).
    pub db: PgPool,
    /// Parsed `DATABASE_URL`; `/ready` opens a fresh connection from it so a
    /// refused or misconfigured Postgres surfaces its real error immediately
    /// instead of the pool's retry-until-timeout behaviour.
    pub pg_connect: PgConnectOptions,
    pub redis: redis::Client,
    pub ready_timeout: Duration,
}

#[derive(Debug)]
pub enum StateError {
    Database(sqlx::Error),
    Redis(redis::RedisError),
}

impl fmt::Display for StateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(err) => write!(f, "invalid DATABASE_URL: {err}"),
            Self::Redis(err) => write!(f, "invalid REDIS_URL: {err}"),
        }
    }
}

impl std::error::Error for StateError {}

impl AppState {
    pub fn from_config(config: &Config) -> Result<Self, StateError> {
        let pg_connect =
            PgConnectOptions::from_str(&config.database_url).map_err(StateError::Database)?;

        let db = PgPoolOptions::new()
            .max_connections(config.db_max_connections)
            .acquire_timeout(config.ready_timeout)
            .connect_lazy_with(pg_connect.clone());

        let redis = redis::Client::open(config.redis_url.as_str()).map_err(StateError::Redis)?;

        Ok(Self {
            db,
            pg_connect,
            redis,
            ready_timeout: config.ready_timeout,
        })
    }
}
