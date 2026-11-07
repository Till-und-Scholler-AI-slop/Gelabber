//! Gelabber's native client core: the REST API, the gateway and the state
//! the interface draws, without any UI toolkit. The interface reads
//! [`Client::state`] each frame and sends work with the command methods
//! (`commands.rs`), which run on the client's own tokio runtime.

pub mod api;
mod commands;
pub mod gateway;
pub mod http;
pub mod models;
pub mod store;

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Weak};
use std::time::Instant;

use parking_lot::{Mutex, RwLock, RwLockReadGuard};

pub use api::HistoryQuery;
pub use commands::SettingTarget;
pub use gateway::{Gateway, GatewayConfig, GatewayEvent, GatewayStatus};
pub use http::{ApiError, Http, error_message};
pub use models::*;
pub use store::{
    ChannelMessages, Notice, OpState, OutgoingFile, PendingMessage, Session, State, View,
    VoiceOccupant, VoiceSelf,
};

/// Called whenever the state changes, so the interface repaints.
pub type Repaint = Arc<dyn Fn() + Send + Sync>;

/// Sees every sequenced chat event after the state took it (the desktop
/// app's notifications). Runs on the client runtime: keep it short.
pub type EventTap = Arc<dyn Fn(&EventFrame) + Send + Sync>;

/// Sees the gateway's `ready`, `sig` and `err` after the state took them
/// in, for the call (gelabber-call), which keeps its own room state.
pub type VoiceHook = Arc<dyn Fn(&GatewayEvent) + Send + Sync>;

/// A voice command, run in the order the commands were given.
type VoiceJob = Box<dyn FnOnce(&Client) + Send>;

/// Input-driven state that lives outside [`State`] so the UI thread can
/// update it without taking the state lock.
#[derive(Default)]
struct Local {
    /// The conversation my typing was announced in, `(s, c)`.
    typing: Option<(uuid::Uuid, uuid::Uuid)>,
    typing_sent: Option<Instant>,
    last_activity: Option<Instant>,
    last_pulse: Option<Instant>,
    idle: bool,
    focused: bool,
    in_front: bool,
    muted_before_deafen: bool,
    /// When the oldest event of a pending unread refresh arrived.
    unread_since: Option<Instant>,
}

#[derive(Default)]
struct Recovery {
    running: bool,
    rerun: bool,
}

struct Inner {
    /// `Option` so `Drop` can shut it down from any thread, even a worker.
    runtime: Option<tokio::runtime::Runtime>,
    http: Http,
    busy: AtomicUsize,
    repaint: RwLock<Option<Repaint>>,
    voice_hook: RwLock<Option<VoiceHook>>,
    voice_jobs: Mutex<Option<tokio::sync::mpsc::UnboundedSender<VoiceJob>>>,
    state: RwLock<State>,
    gateway: Mutex<Option<Gateway>>,
    gateway_config: Mutex<GatewayConfig>,
    local: Mutex<Local>,
    recovery: Mutex<Recovery>,
    notices: Mutex<Vec<Notice>>,
    /// Views being opened, so a call every frame spawns one load.
    opening: Mutex<std::collections::HashSet<View>>,
    /// Debounce generations for unread refreshes and read marks.
    unread_generation: AtomicU64,
    read_generation: AtomicU64,
    /// Bumped on every gateway start and stop: events of a socket that was
    /// already replaced are dropped.
    gateway_generation: AtomicU64,
    event_tap: RwLock<Option<EventTap>>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        if let Some(gateway) = self.gateway.get_mut().take() {
            gateway.stop();
        }
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}

/// Cheap to clone; all clones share one connection, cookie jar and state.
#[derive(Clone)]
pub struct Client {
    inner: Arc<Inner>,
}

impl Client {
    /// `server` is the web origin, e.g. `https://gelabber.example`.
    pub fn new(server: &str) -> Result<Self, ApiError> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("gelabber-client")
            .enable_all()
            .build()
            .map_err(|e| ApiError::Transport(e.to_string()))?;
        let http = Http::new(server)?;
        let client = Self {
            inner: Arc::new(Inner {
                runtime: Some(runtime),
                http,
                busy: AtomicUsize::new(0),
                repaint: RwLock::new(None),
                voice_hook: RwLock::new(None),
                voice_jobs: Mutex::new(None),
                state: RwLock::new(State::default()),
                gateway: Mutex::new(None),
                gateway_config: Mutex::new(GatewayConfig::default()),
                local: Mutex::new(Local::default()),
                recovery: Mutex::new(Recovery::default()),
                notices: Mutex::new(Vec::new()),
                opening: Mutex::new(Default::default()),
                unread_generation: AtomicU64::new(0),
                read_generation: AtomicU64::new(0),
                gateway_generation: AtomicU64::new(0),
                event_tap: RwLock::new(None),
            }),
        };
        let weak = Arc::downgrade(&client.inner);
        client
            .inner
            .http
            .set_unauthenticated_hook(Arc::new(move || {
                if let Some(inner) = weak.upgrade() {
                    Client { inner }.signed_out_by_server();
                }
            }));
        client.start_ticker();
        Ok(client)
    }

    fn runtime(&self) -> &tokio::runtime::Runtime {
        self.inner
            .runtime
            .as_ref()
            .expect("runtime lives as long as the client")
    }

    fn weak(&self) -> Weak<Inner> {
        Arc::downgrade(&self.inner)
    }

    fn from_weak(weak: &Weak<Inner>) -> Option<Self> {
        weak.upgrade().map(|inner| Self { inner })
    }

    pub fn http(&self) -> &Http {
        &self.inner.http
    }

    pub fn set_event_tap(&self, tap: Option<EventTap>) {
        *self.inner.event_tap.write() = tap;
    }

    pub fn set_repaint(&self, repaint: Repaint) {
        *self.inner.repaint.write() = Some(repaint);
    }

    pub fn set_voice_hook(&self, hook: Option<VoiceHook>) {
        *self.inner.voice_hook.write() = hook;
    }

    /// Runs voice commands one after the other in the order they were
    /// given (a leave right after a join must not overtake it), off the
    /// calling thread like every command.
    fn voice_queue(&self, job: impl FnOnce(&Client) + Send + 'static) {
        let mut slot = self.inner.voice_jobs.lock();
        let jobs = slot.get_or_insert_with(|| {
            let (jobs, mut queue) = tokio::sync::mpsc::unbounded_channel::<VoiceJob>();
            let weak = self.weak();
            self.runtime().spawn(async move {
                while let Some(job) = queue.recv().await {
                    let Some(client) = Client::from_weak(&weak) else {
                        return;
                    };
                    job(&client);
                }
            });
            jobs
        });
        let _ = jobs.send(Box::new(job));
    }

    fn voice_hook(&self, event: &GatewayEvent) {
        let hook = self.inner.voice_hook.read().clone();
        if let Some(hook) = hook {
            hook(event);
        }
    }

    pub fn repaint(&self) {
        let repaint = self.inner.repaint.read().clone();
        if let Some(repaint) = repaint {
            repaint();
        }
    }

    /// True while any request started through [`Self::spawn`] is running.
    pub fn busy(&self) -> bool {
        self.inner.busy.load(Ordering::Acquire) > 0
    }

    /// Runs `work` on the client runtime, counted by [`Self::busy`], and
    /// repaints when it is done.
    pub fn spawn<F>(&self, work: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let this = self.clone();
        self.inner.busy.fetch_add(1, Ordering::AcqRel);
        self.runtime().spawn(async move {
            work.await;
            this.inner.busy.fetch_sub(1, Ordering::AcqRel);
            this.repaint();
        });
    }

    /// Blocks the calling thread; for tools and tests, never the UI thread.
    pub fn block_on<F: Future>(&self, work: F) -> F::Output {
        self.runtime().block_on(work)
    }

    pub fn login_blocking(&self, email: &str, password: &str) -> Result<(), ApiError> {
        let http = self.inner.http.clone();
        let session = self.block_on(async move { http.login(email, password).await })?;
        if let Some(user) = session
            .get("user")
            .cloned()
            .and_then(|u| serde_json::from_value::<User>(u).ok())
        {
            self.write(|s| {
                if s.me().map(|m| m.id) != Some(user.id) {
                    s.reset_account();
                }
                s.session = Session::Authenticated(Box::new(user));
            });
        }
        Ok(())
    }

    /// The state to draw. Hold the guard only while drawing: the runtime
    /// waits for it to write. Commands never write on the calling thread,
    /// so calling them while holding it is safe.
    pub fn state(&self) -> RwLockReadGuard<'_, State> {
        self.inner.state.read()
    }

    pub fn with_state<R>(&self, read: impl FnOnce(&State) -> R) -> R {
        read(&self.inner.state.read())
    }

    /// Writes the state and repaints. Never call while holding [`Self::state`].
    fn write<R>(&self, change: impl FnOnce(&mut State) -> R) -> R {
        let out = change(&mut self.inner.state.write());
        self.repaint();
        out
    }

    /// The account the state belongs to now; see [`Self::write_in`].
    fn epoch(&self) -> u64 {
        self.inner.state.read().epoch
    }

    /// Writes only while the state still belongs to the account `epoch` was
    /// read under, so an answer that outlived a sign-out is dropped.
    fn write_in<R>(&self, epoch: u64, change: impl FnOnce(&mut State) -> R) -> Option<R> {
        let out = {
            let mut state = self.inner.state.write();
            (state.epoch == epoch).then(|| change(&mut state))
        };
        if out.is_some() {
            self.repaint();
        }
        out
    }

    /// Toasts that arrived since the last call (kicked, banned, errors).
    pub fn take_notices(&self) -> Vec<Notice> {
        std::mem::take(&mut *self.inner.notices.lock())
    }

    fn notify(&self, notice: Notice) {
        let mut notices = self.inner.notices.lock();
        // Nobody may be reading (a headless client): keep the newest only.
        if notices.len() >= 256 {
            notices.remove(0);
        }
        notices.push(notice);
        drop(notices);
        self.repaint();
    }

    pub fn gateway(&self) -> Option<Gateway> {
        self.inner.gateway.lock().clone()
    }

    /// Timers for the next gateway start (tests shorten them).
    pub fn set_gateway_config(&self, config: GatewayConfig) {
        *self.inner.gateway_config.lock() = config;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropping_the_last_clone_inside_a_task_does_not_panic() {
        let client = Client::new("http://127.0.0.1:9").unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let moved = client.clone();
        client.spawn(async move {
            drop(moved);
            let _ = tx.send(());
        });
        rx.recv().unwrap();
        drop(client);
    }

    /// Answers every request with `body` after `delay`.
    fn slow_server(body: &'static str, delay: std::time::Duration) -> String {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                std::thread::spawn(move || {
                    let mut buf = [0u8; 4096];
                    let _ = stream.read(&mut buf);
                    std::thread::sleep(delay);
                    let _ = write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                });
            }
        });
        format!("http://{addr}")
    }

    #[test]
    fn an_answer_for_a_signed_out_account_is_dropped() {
        let servers = include_str!("../tests/fixtures/servers.json");
        let url = slow_server(servers, std::time::Duration::from_millis(300));
        let client = Client::new(&url).unwrap();
        client.refresh_servers();
        std::thread::sleep(std::time::Duration::from_millis(50));
        // Signed out while the list was on its way.
        client.write(State::reset_account);
        let start = Instant::now();
        while client.busy() && start.elapsed() < std::time::Duration::from_secs(5) {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!client.busy());
        assert!(client.with_state(|s| s.servers.is_empty() && !s.servers_loaded));
        // The same request without a sign-out lands.
        client.refresh_servers();
        let start = Instant::now();
        while !client.with_state(|s| s.servers_loaded)
            && start.elapsed() < std::time::Duration::from_secs(5)
        {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(client.with_state(|s| !s.servers.is_empty()));
    }
}
