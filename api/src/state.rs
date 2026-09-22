use std::fmt;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use uuid::Uuid;

use gelabber_shared::ice::{self, IceServer};

use crate::config::Config;
use crate::gateway::{ConnTable, EventLog, Gateway, VoiceRoster};
use crate::limits::{Limiter, Limits};
use crate::metrics::HttpMetrics;
use crate::storage::ObjectStore;

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
    /// Native WS gateway (issue #6). Facade over the three realtime jobs.
    pub gateway: Gateway,
    /// Socket table and Redis subscriber.
    pub connections: ConnTable,
    /// Sequenced chat log (`publish` / `catch_up`).
    pub events: EventLog,
    /// Voice roster: seats, publications, Go Live claim.
    pub voice: VoiceRoster,
    pub ws_heartbeat: Duration,
    pub ws_dead: Duration,
    pub ws_idle: Duration,
    pub ws_replay: usize,
    pub ice_servers: Vec<IceServer>,
    pub turn_auth_secret: Option<String>,
    pub turn_cred_ttl: Duration,
    pub media_ticket_ttl: Duration,
    /// MinIO (or in-memory in tests) for attachment bytes.
    pub store: ObjectStore,
    pub limits: Limits,
    pub limiter: Arc<Limiter>,
    pub metrics: Arc<HttpMetrics>,
}

#[derive(Debug)]
pub enum StateError {
    Database(sqlx::Error),
    Redis(redis::RedisError),
    Store(String),
}

impl fmt::Display for StateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(err) => write!(f, "invalid DATABASE_URL: {err}"),
            Self::Redis(err) => write!(f, "invalid REDIS_URL: {err}"),
            Self::Store(err) => write!(f, "invalid object store: {err}"),
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
        let store = ObjectStore::from_minio(config.minio.as_ref())
            .map_err(|err| StateError::Store(err.to_string()))?;
        let gateway = Gateway::new(
            redis.clone(),
            config.ws_replay,
            config.ws_presence_ttl,
            config.ws_typing_ttl,
        );

        Ok(Self {
            db,
            pg_connect,
            redis: redis.clone(),
            ready_timeout: config.ready_timeout,
            cookie_secure: config.cookie_secure,
            session_ttl: config.session_ttl,
            connections: gateway.connections.clone(),
            events: gateway.events.clone(),
            voice: gateway.voice.clone(),
            gateway,
            ws_heartbeat: config.ws_heartbeat,
            ws_dead: config.ws_dead,
            ws_idle: config.ws_idle,
            ws_replay: config.ws_replay,
            ice_servers: config.ice_servers.clone(),
            turn_auth_secret: config.turn_auth_secret.clone(),
            turn_cred_ttl: config.turn_cred_ttl,
            media_ticket_ttl: config.media_ticket_ttl,
            store,
            limits: config.limits,
            limiter: Arc::new(Limiter::new()),
            metrics: Arc::new(HttpMetrics::new()),
        })
    }

    /// Applies pending `api/migrations` to the configured database. Called
    /// once at boot, before the listener accepts traffic.
    pub async fn migrate(&self) -> Result<(), sqlx::migrate::MigrateError> {
        sqlx::migrate!("./migrations").run(&self.db).await
    }

    /// Ticket ICE list. Static username/password unless `TURN_AUTH_SECRET`
    /// is an explicit private secret (coturn REST).
    pub fn ticket_ice_servers(&self, user_id: Uuid) -> Vec<IceServer> {
        let Some(secret) = self.turn_auth_secret.as_deref() else {
            return self.ice_servers.clone();
        };
        if secret.is_empty() || self.ice_servers.is_empty() {
            return self.ice_servers.clone();
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let (username, credential) = ice::turn_rest_credentials(
            secret,
            &user_id.to_string(),
            self.turn_cred_ttl.as_secs(),
            now,
        );
        ice::with_turn_credentials(&self.ice_servers, &username, &credential)
    }
}
