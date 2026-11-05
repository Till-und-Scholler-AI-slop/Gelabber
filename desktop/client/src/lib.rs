//! Gelabber's native client core: the REST API, the gateway and the state
//! the interface draws, without any UI toolkit. The interface reads
//! [`Client::state`] each frame and sends work with the async methods,
//! which run on the client's own tokio runtime.

pub mod http;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use parking_lot::RwLock;

pub use http::{ApiError, Http};

/// Called whenever the state changes, so the interface repaints.
pub type Repaint = Arc<dyn Fn() + Send + Sync>;

struct Inner {
    runtime: tokio::runtime::Runtime,
    http: Http,
    busy: AtomicUsize,
    repaint: RwLock<Option<Repaint>>,
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
        Ok(Self {
            inner: Arc::new(Inner {
                runtime,
                http,
                busy: AtomicUsize::new(0),
                repaint: RwLock::new(None),
            }),
        })
    }

    pub fn http(&self) -> &Http {
        &self.inner.http
    }

    pub fn set_repaint(&self, repaint: Repaint) {
        *self.inner.repaint.write() = Some(repaint);
    }

    pub fn repaint(&self) {
        if let Some(repaint) = self.inner.repaint.read().as_ref() {
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
        self.inner.runtime.spawn(async move {
            work.await;
            this.inner.busy.fetch_sub(1, Ordering::AcqRel);
            this.repaint();
        });
    }

    /// Blocks the calling thread; for tools and tests, never the UI thread.
    pub fn block_on<F: Future>(&self, work: F) -> F::Output {
        self.inner.runtime.block_on(work)
    }

    pub fn login_blocking(&self, email: &str, password: &str) -> Result<(), ApiError> {
        let http = self.inner.http.clone();
        self.block_on(async move { http.login(email, password).await.map(|_| ()) })
    }
}
