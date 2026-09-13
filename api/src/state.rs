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
    pub cookie_secure: bool,
    pub session_ttl: Duration,
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

        Self::with_pool(config, db)
    }

    /// Like `from_config` but with a caller-supplied pool. Integration tests
    /// use it to point the app at a per-test database from `#[sqlx::test]`.
    pub fn with_pool(config: &Config, db: PgPool) -> Result<Self, StateError> {
        let pg_connect =
            PgConnectOptions::from_str(&config.database_url).map_err(StateError::Database)?;
        let redis = redis::Client::open(config.redis_url.as_str()).map_err(StateError::Redis)?;

        Ok(Self {
            db,
            pg_connect,
            redis,
            ready_timeout: config.ready_timeout,
            cookie_secure: config.cookie_secure,
            session_ttl: config.session_ttl,
        })
    }

    /// Applies pending `api/migrations` to the configured database. Called
    /// once at boot, before the listener accepts traffic.
    pub async fn migrate(&self) -> Result<(), sqlx::migrate::MigrateError> {
        sqlx::migrate!("./migrations").run(&self.db).await
    }
}
