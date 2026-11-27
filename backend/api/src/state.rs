use std::collections::{HashMap, HashSet};
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use uuid::Uuid;

use gelabber_shared::ice::{self, IceServer};

use crate::config::{Config, RegistrationMode};
use crate::gateway::{ConnTable, EventLog, Gateway, VoiceRoster};
use crate::gifs::Gifs;
use crate::limits::{Limiter, Limits};
use crate::metrics::HttpMetrics;
use crate::previews::LinkPreviews;
use crate::storage::ObjectStore;

/// One subscribe's replay, gap and `ok` together. They are written after the
/// frame's transaction has ended (GEL-42), so no row or pool connection waits
/// on them; a client that reads too slowly for them loses the socket instead
/// of holding its own loop for minutes.
pub const REPLAY_DEADLINE: Duration = Duration::from_secs(5);

/// Shared handles for request handlers. Both clients are created lazily so
/// the process boots even while Postgres/Redis are still starting; `/ready`
/// reports the live state instead.
#[derive(Clone)]
pub struct AppState {
    /// Connection pool for domain queries (later issues).
    pub db: PgPool,
    /// The gateway's per-socket session and access checks (every frame in,
    /// every delivered batch, the security timer). A pool of their own keeps
    /// thousands of sockets from queueing ahead of REST requests and the
    /// outbox worker, whose events they wait for (#611, #612). The same pool
    /// as `db` when `API_DB_GATEWAY_CONNECTIONS` is 0 and in tests.
    pub gateway_db: PgPool,
    /// Parsed `DATABASE_URL`; `/ready` opens a fresh connection from it so a
    /// refused or misconfigured Postgres surfaces its real error immediately
    /// instead of the pool's retry-until-timeout behaviour.
    pub pg_connect: PgConnectOptions,
    pub redis: redis::Client,
    pub ready_timeout: Duration,
    pub cookie_secure: bool,
    pub allow_plain_http: bool,
    pub session_ttl: Duration,
    pub session_idle: Duration,
    /// Native WS gateway (issue #6). Facade over the three realtime jobs.
    pub gateway: Gateway,
    /// Socket table and Redis subscriber.
    pub connections: ConnTable,
    /// Sequenced chat log (`publish` / `catch_up`).
    pub events: EventLog,
    /// Voice roster: seats and publications.
    pub voice: VoiceRoster,
    pub ws_heartbeat: Duration,
    pub ws_dead: Duration,
    pub ws_idle: Duration,
    pub ws_replay: usize,
    /// How long one subscribe's replay may take to write ([`REPLAY_DEADLINE`]).
    pub ws_replay_deadline: Duration,
    pub ice_servers: Vec<IceServer>,
    pub turn_auth_secret: Option<String>,
    pub turn_cred_ttl: Duration,
    pub media_ticket_ttl: Duration,
    /// MinIO (or in-memory in tests) for attachment bytes.
    pub store: ObjectStore,
    pub(crate) storage_worker_started: Arc<AtomicBool>,
    pub(crate) media_worker_started: Arc<AtomicBool>,
    pub limits: Limits,
    pub limiter: Arc<Limiter>,
    pub metrics: Arc<HttpMetrics>,
    /// Klipy relay; `None` when `KLIPY_API_KEY` is unset.
    pub gifs: Option<Gifs>,
    /// Web Push sender (#243); `None` without `VAPID_PRIVATE_KEY`.
    pub push: Option<crate::push::Push>,
    /// Link previews; `None` when `API_LINK_PREVIEWS=false`.
    pub previews: Option<LinkPreviews>,
    /// Account mail (#195); `None` without `SMTP_HOST`.
    pub mail: Option<crate::mail::Mailer>,
    /// `APP_PUBLIC_URL`, for links the API hands out (`Config::public_url`).
    pub public_url: Option<url::Url>,
    /// `API_OPERATOR_EMAILS`, canonical (`operator::access`).
    pub operators: Arc<HashSet<String>>,
    /// `API_REGISTRATION`.
    pub registration: RegistrationMode,
    /// Sign-in through OpenID Connect; `None` while it is off.
    pub oidc: Option<Arc<crate::auth::oidc::Oidc>>,
    /// `API_DIRECTORY`: the server directory is on (`directory`).
    pub directory: bool,
    /// Wakes the purge of deleted accounts (`account::deletion`).
    pub(crate) account_purge: Arc<tokio::sync::Notify>,
    pub(crate) purge_worker_started: Arc<AtomicBool>,
    /// `API_EXPORT_RETENTION_HOURS`.
    pub export_retention: Duration,
    /// `API_EXPORT_MAX_BYTES`.
    pub export_max_bytes: u64,
    /// Wakes the data export worker (`account::export`).
    pub(crate) account_export: Arc<tokio::sync::Notify>,
    pub(crate) export_worker_started: Arc<AtomicBool>,
    /// The sweeper of the public feedback form (`feedback::public`).
    pub(crate) feedback_worker_started: Arc<AtomicBool>,
    pub(crate) presence_worker_started: Arc<AtomicBool>,
    /// How long one media ticket keeps the session's media authority
    /// renewed (`media::MEDIA_DEMAND`).
    pub media_demand: Duration,
    /// How long kick, ban and server delete wait for the server row
    /// (`servers::membership::SERVER_LOCK_WAIT`).
    pub server_lock_wait: Duration,
    /// Which background workers `app()` starts and how often each looks for
    /// work on its own.
    pub workers: Workers,
    /// The passes those workers finished.
    pub worker_passes: Arc<WorkerPasses>,
}

/// The background workers `app()` starts, each with the interval at which it
/// looks for work without being woken; `None` leaves that worker off. A
/// server runs all of them at [`Workers::default`]. Tests switch off the
/// workers whose steps they drive by hand, so the app's own copy cannot take
/// the work first, and shorten the intervals they would otherwise wait for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Workers {
    /// Gateway outbox delivery (`gateway::delivery`): a write wakes it, and
    /// it polls this often for rows a wake missed.
    pub delivery: Option<Duration>,
    /// Object deletion and upload expiry (`attachments::cleanup`).
    pub storage_cleanup: Option<Duration>,
    /// Picture metadata and thumbnails (`attachments::media`).
    pub pictures: Option<Duration>,
    /// Purge of deleted accounts (`account::deletion`), between wakes.
    pub account_purge: Option<Duration>,
    /// Data exports and their retention (`account::export`), between wakes.
    pub account_export: Option<Duration>,
    /// Link preview jobs, and the sweep of old previews at this interval
    /// (`previews`). Off as well while previews are off.
    pub link_previews: Option<Duration>,
    /// End of timed presence modes (`presence`).
    pub presence_modes: Option<Duration>,
    /// Last-call pushes before a session ends (`push::last_call`). Off as
    /// well while push is off.
    pub last_call: Option<Duration>,
    /// Sweep of the public feedback form's pending entries
    /// (`feedback::public`).
    pub feedback_sweep: Option<Duration>,
}

impl Workers {
    /// Every worker off: a test that drives each step it needs by hand.
    pub const fn none() -> Self {
        Self {
            delivery: None,
            storage_cleanup: None,
            pictures: None,
            account_purge: None,
            account_export: None,
            link_previews: None,
            presence_modes: None,
            last_call: None,
            feedback_sweep: None,
        }
    }
}

impl Default for Workers {
    fn default() -> Self {
        Self {
            delivery: Some(crate::gateway::delivery::POLL),
            storage_cleanup: Some(crate::attachments::cleanup::TICK),
            pictures: Some(crate::attachments::media::TICK),
            account_purge: Some(crate::account::deletion::TICK),
            account_export: Some(crate::account::export::TICK),
            link_previews: Some(crate::previews::SWEEP_EVERY),
            presence_modes: Some(crate::presence::SWEEP_EVERY),
            last_call: Some(crate::push::last_call::SWEEP_EVERY),
            feedback_sweep: Some(crate::feedback::public::SWEEP_EVERY),
        }
    }
}

/// A background worker's loop, as [`WorkerPasses`] counts it. Storage
/// cleanup runs two; link previews count their sweep (their jobs wait for a
/// message, not a timer).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Worker {
    Delivery,
    StorageDeletes,
    UploadExpiry,
    Pictures,
    AccountPurge,
    AccountExport,
    PreviewSweep,
    PresenceModes,
    LastCall,
    FeedbackSweep,
}

/// The passes each background worker finished in this process; a pass counts
/// once its work ran without an error. `tests/workers.rs` waits on it to
/// check that a server starts every worker (GEL-434).
#[derive(Debug)]
pub struct WorkerPasses(tokio::sync::watch::Sender<HashMap<Worker, u64>>);

impl Default for WorkerPasses {
    fn default() -> Self {
        Self(tokio::sync::watch::Sender::new(HashMap::new()))
    }
}

impl WorkerPasses {
    pub(crate) fn finished(&self, worker: Worker) {
        self.0
            .send_modify(|passes| *passes.entry(worker).or_default() += 1);
    }

    /// Returns once `worker` has finished a pass.
    pub async fn first(&self, worker: Worker) {
        let mut passes = self.0.subscribe();
        // The sender lives as long as `self`, so the wait cannot fail.
        let _ = passes.wait_for(|passes| passes.contains_key(&worker)).await;
    }
}

#[derive(Debug)]
pub enum StateError {
    Database(sqlx::Error),
    Redis(redis::RedisError),
    Store(String),
    Http(String),
    Mail(String),
    Oidc(String),
}

impl fmt::Display for StateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(err) => write!(f, "invalid DATABASE_URL: {err}"),
            Self::Redis(err) => write!(f, "invalid REDIS_URL: {err}"),
            Self::Store(err) => write!(f, "invalid object store: {err}"),
            Self::Http(err) => write!(f, "http client: {err}"),
            Self::Mail(err) => write!(f, "mail: {err}"),
            Self::Oidc(err) => write!(f, "sign-in provider: {err}"),
        }
    }
}

impl std::error::Error for StateError {}

/// How long a socket's check waits for a connection of the gateway pool
/// before the socket ends.
const GATEWAY_ACQUIRE_TIMEOUT: Duration = Duration::from_secs(10);

impl AppState {
    pub fn from_config(config: &Config) -> Result<Self, StateError> {
        let pg_connect =
            PgConnectOptions::from_str(&config.database_url).map_err(StateError::Database)?;

        let db = PgPoolOptions::new()
            .max_connections(config.db_max_connections)
            .acquire_timeout(config.ready_timeout)
            .connect_lazy_with(pg_connect.clone());
        let mut state = Self::with_pool(config, db)?;
        if config.db_gateway_connections > 0 {
            // Sockets queue for this pool under load; waiting delays their
            // next batch, so it may take longer than a request would.
            state.gateway_db = PgPoolOptions::new()
                .max_connections(config.db_gateway_connections)
                .acquire_timeout(GATEWAY_ACQUIRE_TIMEOUT)
                // A wait of a few seconds is normal here during a connect
                // wave; warn only when it nears the timeout.
                .acquire_slow_threshold(GATEWAY_ACQUIRE_TIMEOUT / 2)
                .connect_lazy_with(pg_connect);
        }
        Ok(state)
    }

    /// Like `from_config` but with a caller-supplied pool. Integration tests
    /// use it to point the app at a per-test database from `#[sqlx::test]`.
    pub fn with_pool(config: &Config, db: PgPool) -> Result<Self, StateError> {
        let pg_connect =
            PgConnectOptions::from_str(&config.database_url).map_err(StateError::Database)?;
        let redis = redis::Client::open(config.redis_url.as_str()).map_err(StateError::Redis)?;
        let store = if config.minio.is_none() && config.allow_memory_store {
            Ok(ObjectStore::memory())
        } else {
            ObjectStore::from_minio(config.minio.as_ref())
        }
        .map_err(|err| StateError::Store(err.to_string()))?;
        let metrics = Arc::new(HttpMetrics::new());
        let gifs = config
            .gifs
            .as_ref()
            .map(|gifs| Gifs::new(gifs, metrics.clone()))
            .transpose()
            .map_err(|err| StateError::Http(err.to_string()))?;
        let push = config
            .push
            .as_ref()
            .map(crate::push::Push::new)
            .transpose()
            .map_err(StateError::Http)?;
        let mail = config
            .mail
            .as_ref()
            .map(crate::mail::Mailer::new)
            .transpose()
            .map_err(StateError::Mail)?;
        let oidc = config
            .oidc
            .as_ref()
            .map(crate::auth::oidc::Oidc::new)
            .transpose()
            .map_err(StateError::Oidc)?
            .map(Arc::new);
        let gateway = Gateway::new(
            redis.clone(),
            config.ws_replay,
            config.ws_presence_ttl,
            config.ws_typing_ttl,
        );

        Ok(Self {
            gateway_db: db.clone(),
            db,
            pg_connect,
            redis: redis.clone(),
            ready_timeout: config.ready_timeout,
            cookie_secure: config.cookie_secure,
            allow_plain_http: config.allow_plain_http,
            session_ttl: config.session_ttl,
            session_idle: config.session_idle,
            connections: gateway.connections.clone(),
            events: gateway.events.clone(),
            voice: gateway.voice.clone(),
            gateway,
            ws_heartbeat: config.ws_heartbeat,
            ws_dead: config.ws_dead,
            ws_idle: config.ws_idle,
            ws_replay: config.ws_replay,
            ws_replay_deadline: REPLAY_DEADLINE,
            ice_servers: config.ice_servers.clone(),
            turn_auth_secret: config.turn_auth_secret.clone(),
            turn_cred_ttl: config.turn_cred_ttl,
            media_ticket_ttl: config.media_ticket_ttl,
            store,
            storage_worker_started: Arc::new(AtomicBool::new(false)),
            media_worker_started: Arc::new(AtomicBool::new(false)),
            limits: config.limits,
            limiter: Arc::new(Limiter::new()),
            metrics,
            gifs,
            push,
            previews: config.previews.as_ref().map(LinkPreviews::new),
            mail,
            public_url: config.public_url.clone(),
            operators: Arc::new(config.operators.clone()),
            registration: config.registration,
            oidc,
            directory: config.directory,
            account_purge: Arc::new(tokio::sync::Notify::new()),
            purge_worker_started: Arc::new(AtomicBool::new(false)),
            export_retention: config.export_retention,
            export_max_bytes: config.export_max_bytes,
            account_export: Arc::new(tokio::sync::Notify::new()),
            export_worker_started: Arc::new(AtomicBool::new(false)),
            feedback_worker_started: Arc::new(AtomicBool::new(false)),
            presence_worker_started: Arc::new(AtomicBool::new(false)),
            media_demand: crate::media::MEDIA_DEMAND,
            server_lock_wait: crate::servers::membership::SERVER_LOCK_WAIT,
            workers: Workers::default(),
            worker_passes: Arc::default(),
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
