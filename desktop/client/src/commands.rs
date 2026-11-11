//! What the interface asks the client to do. Every command returns at once
//! and runs on the client runtime through [`Client::spawn`], so `busy()`
//! covers it; results land in [`State`] and trigger a repaint. Named
//! commands report progress in `State::ops` under the key they document.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use futures_util::future::join_all;
use uuid::Uuid;

use crate::Client;
use crate::api::HistoryQuery;
use crate::gateway::{Gateway, GatewayEvent};
use crate::http::ApiError;
use crate::models::*;
use crate::store::{
    Notice, OpState, OutgoingFile, PAGE_SIZE, PendingMessage, SendStage, Session, State, View,
    VoiceSelf,
};

/// Session check retries while the server is unreachable (`session.ts`).
const SESSION_RETRY: [Duration; 4] = [
    Duration::from_millis(2_000),
    Duration::from_millis(5_000),
    Duration::from_millis(15_000),
    Duration::from_millis(30_000),
];
const TYPING_REFRESH: Duration = Duration::from_millis(3_000);
const PRESENCE_PULSE: Duration = Duration::from_millis(10_000);
const CLIENT_IDLE: Duration = Duration::from_secs(300);
/// Recovery after a reconnect waits up to this long so clients spread out.
const READY_SPREAD: Duration = Duration::from_millis(1_000);
/// A failed recovery round is tried again after this, doubling up to the
/// max, while the socket is wanted: missed events stay missing until then.
const RECOVERY_RETRY: Duration = Duration::from_secs(1);
const RECOVERY_RETRY_MAX: Duration = Duration::from_secs(30);
/// Failed rounds retried in a row before waiting for the next event.
const RECOVERY_RETRIES: u32 = 5;
/// Unread counts are refetched this long after the last event (web 750 ms,
/// at most 5 s while events keep coming).
const UNREAD_DEBOUNCE: Duration = Duration::from_millis(750);
const UNREAD_MAX_WAIT: Duration = Duration::from_secs(5);
const TICK: Duration = Duration::from_millis(400);
/// The server list and a server's detail are fetched again when shown or
/// focused after this long (the web's `staleTime`): channels, members and
/// permissions change without an event.
const SERVER_STALE: Duration = Duration::from_secs(60);

/// Which notification setting to change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingTarget {
    Server(Uuid),
    /// A text channel or a DM.
    Channel(Uuid),
}

impl Client {
    // ------------------------------------------------------------ helpers

    /// Runs `work` as the named operation: `State::ops[op]` is `Pending`
    /// when this returns, then `Done` or `Failed`; `apply` gets the result on
    /// success, in the same write as `Done`, so no frame sees one without
    /// the other. Writes on the calling thread: not while holding `state()`.
    pub fn run<T, F, A>(&self, op: impl Into<String>, work: F, apply: A)
    where
        T: Send + 'static,
        F: Future<Output = Result<T, ApiError>> + Send + 'static,
        A: FnOnce(&mut State, T) + Send + 'static,
    {
        let op = op.into();
        let epoch = self.write(|s| {
            s.ops.insert(op.clone(), OpState::Pending);
            s.epoch
        });
        let this = self.clone();
        self.spawn(async move {
            match work.await {
                Ok(value) => {
                    this.write_in(epoch, |s| {
                        s.ops.insert(op, OpState::Done);
                        apply(s, value);
                    });
                }
                Err(error) => {
                    if this.epoch() == epoch {
                        this.fail_op(&op, error);
                    }
                }
            }
        });
    }

    /// `None` also when the account changed meanwhile: the answer is dropped.
    async fn op<T>(&self, key: &str, work: impl Future<Output = Result<T, ApiError>>) -> Option<T> {
        let epoch = self.epoch();
        self.write(|s| s.ops.insert(key.to_owned(), OpState::Pending));
        match work.await {
            Ok(value) => self
                .write_in(epoch, |s| s.ops.insert(key.to_owned(), OpState::Done))
                .map(|_| value),
            Err(error) => {
                if self.epoch() == epoch {
                    self.fail_op(key, error);
                }
                None
            }
        }
    }

    /// Records the named operation as failed, as its request does: the op
    /// state, and for an optimistic write the error toast's notice.
    pub fn fail_op(&self, key: &str, error: ApiError) {
        if let Some(failed) = crate::store::failure_text(key) {
            self.notify(Notice::Failed {
                failed,
                gone: crate::store::gone_text(key),
                error: error.clone(),
            });
        }
        self.write(|s| {
            *s.failures.entry(key.to_owned()).or_default() += 1;
            s.ops.insert(key.to_owned(), OpState::Failed(error))
        });
    }

    /// Forgets a finished operation (a form that was closed).
    pub fn clear_op(&self, op: impl Into<String>) {
        let this = self.clone();
        let op = op.into();
        self.spawn(async move {
            this.write(|s| s.ops.remove(&op));
        });
    }

    fn sync_topics(&self) {
        if let Some(gateway) = self.gateway() {
            let topics = self.with_state(State::desired_topics);
            gateway.set_topics(topics);
        }
    }

    fn send_frame(&self, frame: ClientFrame) {
        let tap = self.inner.frame_tap.read().clone();
        if let Some(tap) = tap {
            tap(&frame);
        }
        if let Some(gateway) = self.gateway() {
            gateway.send(frame);
        }
    }

    // ------------------------------------------------------------ session

    /// App start: `GET /api/auth/session`, then the workspace when signed
    /// in. While the server is unreachable it retries like the web
    /// (2, 5, 15, 30 s). Op key `session`.
    pub fn restore_session(&self) {
        self.restore_session_attempt(0);
    }

    fn restore_session_attempt(&self, failures: usize) {
        let this = self.clone();
        self.spawn(async move {
            this.write(|s| s.ops.insert("session".into(), OpState::Pending));
            match this.http().auth_session().await {
                Ok(session) => {
                    this.write(|s| {
                        s.ops.insert("session".into(), OpState::Done);
                        s.session_error = None;
                    });
                    this.adopt_session(session.user).await;
                }
                Err(error) => {
                    let failures = failures + 1;
                    this.write(|s| {
                        s.ops
                            .insert("session".into(), OpState::Failed(error.clone()));
                        s.session_error = Some(error);
                    });
                    let delay = SESSION_RETRY[failures.min(4) - 1];
                    let weak = this.weak();
                    this.runtime().spawn(async move {
                        tokio::time::sleep(delay).await;
                        if let Some(client) = Client::from_weak(&weak)
                            && client.with_state(|s| s.session == Session::Unknown)
                        {
                            client.restore_session_attempt(failures);
                        }
                    });
                }
            }
        });
    }

    /// Signed in (or not): reset what belonged to someone else, load the
    /// workspace. Loaded before the first check, it is this account's (the
    /// same cookie) and stays.
    async fn adopt_session(&self, user: Option<User>) {
        match user {
            Some(user) => {
                self.http().forget_sign_out();
                let other = self.with_state(|s| s.me().is_some_and(|m| m.id != user.id));
                if other {
                    self.stop_gateway();
                    self.write(State::reset_account);
                }
                self.write(|s| s.session = Session::Authenticated(Box::new(user)));
                self.load_workspace().await;
            }
            None => {
                self.stop_gateway();
                self.write(|s| {
                    s.reset_account();
                    s.session = Session::Anonymous;
                });
            }
        }
    }

    /// Op key `login`; on success the workspace loads.
    pub fn login(&self, email: &str, password: &str) {
        let (this, email, password) = (self.clone(), email.to_owned(), password.to_owned());
        self.spawn(async move {
            let http = this.http().clone();
            if let Some(session) = this.op("login", http.sign_in(&email, &password)).await {
                this.adopt_session(session.user).await;
            }
        });
    }

    /// Op key `register`. Signed in at once, or `State::verification_sent`
    /// when the instance sends a confirmation mail first.
    pub fn register(&self, email: &str, password: &str, name: &str) {
        let (this, email, password, name) = (
            self.clone(),
            email.to_owned(),
            password.to_owned(),
            name.to_owned(),
        );
        self.spawn(async move {
            let http = this.http().clone();
            if let Some(answer) = this
                .op("register", http.register(&email, &password, &name))
                .await
            {
                if answer.user.is_some() {
                    this.adopt_session(answer.user).await;
                } else {
                    this.write(|s| s.verification_sent = answer.verification_sent);
                }
            }
        });
    }

    /// Optimistic like the web: signed out at once, then the server is
    /// told. Op key `logout` reports the server's answer; until it confirms,
    /// this repeats a failed [`Self::logout_all`].
    pub fn logout(&self) {
        self.sign_out(false);
    }

    /// Ends every session of the account. Op key `logout`.
    pub fn logout_all(&self) {
        self.sign_out(true);
    }

    fn sign_out(&self, everywhere: bool) {
        let this = self.clone();
        self.spawn(async move {
            this.stop_typing();
            this.stop_gateway();
            this.write(|s| {
                s.reset_account();
                s.session = Session::Anonymous;
            });
            let http = this.http().clone();
            this.op("logout", async move { http.sign_out(everywhere).await })
                .await;
        });
    }

    /// A `401 unauthenticated` anywhere: the session ended on the server.
    pub(crate) fn signed_out_by_server(&self) {
        let this = self.clone();
        self.runtime().spawn(async move {
            // A deletion on its way ends the session itself, and says so.
            let deleting =
                this.with_state(|s| matches!(s.op("delete_account"), Some(OpState::Pending)));
            if !deleting && this.with_state(|s| matches!(s.session, Session::Authenticated(_))) {
                this.stop_gateway();
                this.write(|s| {
                    s.reset_account();
                    s.session = Session::Anonymous;
                    s.session_expired = true;
                });
            }
        });
    }

    /// Op key `password`.
    pub fn change_password(&self, current: &str, password: &str) {
        let (this, current, password) = (self.clone(), current.to_owned(), password.to_owned());
        self.spawn(async move {
            let http = this.http().clone();
            if let Some(session) = this
                .op("password", http.change_password(&current, &password))
                .await
                && let Some(user) = session.user
            {
                this.write(|s| s.session = Session::Authenticated(Box::new(user)));
            }
        });
    }

    /// `GET /api/auth/mail` for the login page into `State::mail`.
    pub fn load_mail_status(&self) {
        let http = self.http().clone();
        self.run("mail", async move { http.mail_status().await }, |s, m| {
            s.mail = Some(m)
        });
    }

    // ---------------------------------------------------------- workspace

    /// The web's startup request set (servers, DMs, unread, notification
    /// settings and privacy, themes) and the gateway. Op key `bootstrap`.
    pub fn bootstrap(&self) {
        // Several screens ask for the workspace in the same frame; marking it
        // pending here, not in the task, keeps that to one load.
        let started = self.write(|s| {
            if matches!(s.ops.get("bootstrap"), Some(OpState::Pending)) {
                return false;
            }
            s.ops.insert("bootstrap".into(), OpState::Pending);
            true
        });
        if started {
            let this = self.clone();
            self.spawn(async move { this.load_workspace().await });
        }
    }

    async fn load_workspace(&self) {
        let http = self.http().clone();
        let epoch = self.epoch();
        self.write(|s| s.ops.insert("bootstrap".into(), OpState::Pending));
        let (servers, dms, unread, settings, privacy, themes) = tokio::join!(
            http.servers(),
            http.dms(),
            http.unread(),
            http.notification_settings(),
            http.notification_privacy(),
            http.themes(),
        );
        let mut failure: Option<ApiError> = None;
        let current = self.write_in(epoch, |s| {
            match servers {
                Ok(list) => {
                    s.servers = list;
                    s.servers_loaded = true;
                    s.servers_fetched = Some(Instant::now());
                }
                Err(e) => failure = Some(e),
            }
            match dms {
                Ok(list) => {
                    s.dms = list;
                    s.dms_loaded = true;
                }
                Err(e) => failure = failure.take().or(Some(e)),
            }
            match unread {
                Ok(rows) => {
                    s.replace_read_states(rows);
                }
                Err(e) => failure = failure.take().or(Some(e)),
            }
            if let Ok(list) = settings {
                s.notification_settings = list;
            }
            // An old server without the route answers 404; keep the default.
            if let Ok(privacy) = privacy {
                s.notification_privacy = Some(privacy);
            }
            if let Ok(doc) = themes {
                s.themes = Some(doc);
            }
            s.bootstrapped = true;
            s.ops.insert(
                "bootstrap".into(),
                match failure.clone() {
                    Some(e) => OpState::Failed(e),
                    None => OpState::Done,
                },
            );
        });
        if current.is_some() && self.with_state(|s| s.me().is_some()) {
            self.ensure_gateway();
            self.activity();
            self.sync_topics();
        }
    }

    /// Refetches the server list (after join/leave elsewhere).
    pub fn refresh_servers(&self) {
        let this = self.clone();
        self.spawn(async move { this.reload_servers().await });
    }

    async fn reload_servers(&self) {
        let epoch = self.epoch();
        if let Ok(list) = self.http().servers().await {
            self.write_in(epoch, |s| {
                let gone: Vec<Uuid> = s
                    .server_details
                    .keys()
                    .filter(|id| !list.iter().any(|v| v.id == **id))
                    .copied()
                    .collect();
                for id in gone {
                    s.forget_server(id);
                }
                s.servers = list;
                s.servers_loaded = true;
                s.servers_fetched = Some(Instant::now());
            });
            self.sync_topics();
        }
    }

    async fn reload_server(&self, id: Uuid) -> Result<(), ApiError> {
        let epoch = self.epoch();
        match self.http().server(id).await {
            Ok(detail) => {
                self.write_in(epoch, |s| {
                    if let Some(entry) = s.servers.iter_mut().find(|v| v.id == id) {
                        *entry = detail.server.clone();
                    }
                    s.server_details.insert(id, detail);
                    s.server_fetched.insert(id, Instant::now());
                });
                self.sync_topics();
                Ok(())
            }
            Err(e) if e.is_gone() => {
                self.write_in(epoch, |s| s.forget_server(id));
                self.sync_topics();
                Err(e)
            }
            // Blocked meanwhile: it stays listed, marked, without its detail.
            Err(e) if e.code() == "server_blocked" => {
                self.write_in(epoch, |s| {
                    if let Some(entry) = s.servers.iter_mut().find(|v| v.id == id) {
                        entry.blocked = true;
                    }
                    s.server_details.remove(&id);
                });
                self.sync_topics();
                Err(e)
            }
            Err(e) => Err(e),
        }
    }

    /// The access-changed event (`{"k":"a"}`): the caller's rights and the
    /// channels they see may have changed, so the server's detail and the
    /// list are read again. An open channel that is gone takes the
    /// missing-channel path.
    pub fn refresh_access(&self, server: Uuid) {
        let this = self.clone();
        self.spawn(async move {
            this.reload_servers().await;
            let _ = this.reload_server(server).await;
        });
    }

    /// Refetches unread counts and notification levels.
    pub fn refresh_unread(&self) {
        let this = self.clone();
        self.spawn(async move { this.reload_unread().await });
    }

    async fn reload_unread(&self) {
        let epoch = self.epoch();
        if let Ok(rows) = self.http().unread().await {
            self.write_in(epoch, |s| {
                s.replace_read_states(rows);
            });
        }
    }

    // --------------------------------------------------------------- views

    /// Shows a server (the route `/s/{id}`): loads its detail once and
    /// subscribes its topics. Cheap to call every frame. Op `server:<id>`.
    pub fn open_server(&self, id: Uuid) {
        let key = format!("server:{id}");
        let view = View {
            server: Some(id),
            channel: None,
        };
        self.refresh_if_stale(Some(id));
        // A failed load is not retried every frame; `refresh_server` does.
        let needed = self.with_state(|s| {
            s.view != view || (!s.server_details.contains_key(&id) && s.op(&key).is_none())
        });
        if !needed || !self.inner.opening.lock().insert(view) {
            return;
        }
        let this = self.clone();
        self.spawn(async move {
            this.change_view(view);
            // Nor when only the view changed: the shell and a page that
            // both open it would take turns forever.
            if !this.with_state(|s| s.server_details.contains_key(&id) || s.op(&key).is_some()) {
                let work = this.reload_server(id);
                this.op(&key, work).await;
            }
            this.inner.opening.lock().remove(&view);
        });
    }

    /// Fetches the list and `server`'s detail again in the background when
    /// they are older than [`SERVER_STALE`]. Cheap to call every frame.
    fn refresh_if_stale(&self, server: Option<Uuid>) {
        let stale = |at: Option<&Instant>| at.is_some_and(|t| t.elapsed() >= SERVER_STALE);
        let (list, detail) = self.with_state(|s| {
            (
                stale(s.servers_fetched.as_ref()),
                server.filter(|id| stale(s.server_fetched.get(id))),
            )
        });
        if !list && detail.is_none() {
            return;
        }
        // Once per period, also when it fails.
        self.write(|s| {
            let now = Instant::now();
            if list {
                s.servers_fetched = Some(now);
            }
            if let Some(id) = detail {
                s.server_fetched.insert(id, now);
            }
        });
        let this = self.clone();
        self.spawn(async move {
            if list {
                this.reload_servers().await;
            }
            if let Some(id) = detail {
                let _ = this.reload_server(id).await;
            }
        });
    }

    /// Fetches a server's detail again (retry after an error).
    pub fn refresh_server(&self, id: Uuid) {
        let this = self.clone();
        self.spawn(async move {
            let work = this.reload_server(id);
            this.op(&format!("server:{id}"), work).await;
        });
    }

    /// Fetches the newest page of a conversation again (retry after an
    /// error, or a jump back to the present).
    pub fn reload_channel(&self, channel: Uuid) {
        let this = self.clone();
        self.spawn(async move { this.load_latest(channel).await });
    }

    /// Opens a text channel or DM: history (first page) once. Marking it
    /// read is the pane's (at the newest row, focused). The server is looked up; for a route with both
    /// ids use [`Self::open_server_channel`]. Cheap to call every frame.
    pub fn open_channel(&self, channel: Uuid) {
        let server = self.with_state(|s| {
            s.channel(channel)
                .and_then(|c| c.server_id)
                .map(Some)
                .or_else(|| s.dm(channel).map(|_| None))
        });
        match server {
            Some(server) => self.open_conversation(server, channel),
            // Unknown yet: a DM not in the list, or a server not loaded.
            None => self.load_dm(channel),
        }
    }

    /// The DM page's own fetch of its conversation (`useDm`), also for one
    /// the list has: op key `dm_load:<id>`, whose failure the page shows.
    /// A gone DM leaves the list.
    pub fn load_dm(&self, channel: Uuid) {
        let this = self.clone();
        self.spawn(async move {
            let epoch = this.epoch();
            let http = this.http().clone();
            let key = format!("dm_load:{channel}");
            match this.op(&key, http.dm(channel)).await {
                Some(dm) => {
                    let added =
                        this.write_in(epoch, |s| match s.dms.iter_mut().find(|d| d.id == dm.id) {
                            Some(known) => {
                                *known = dm;
                                false
                            }
                            None => {
                                s.dms.insert(0, dm);
                                true
                            }
                        });
                    if added == Some(true) {
                        this.open_conversation(None, channel);
                    }
                }
                None => {
                    let gone = this.with_state(|s| {
                        matches!(s.op(&key), Some(crate::store::OpState::Failed(e)) if e.is_gone())
                    });
                    if gone {
                        this.write_in(epoch, |s| s.dms.retain(|d| d.id != channel));
                    }
                }
            }
        });
    }

    /// `/s/{server}/c/{channel}`.
    pub fn open_server_channel(&self, server: Uuid, channel: Uuid) {
        self.open_conversation(Some(server), channel);
    }

    /// `/d/{id}`.
    pub fn open_dm(&self, dm: Uuid) {
        self.open_conversation(None, dm);
    }

    fn open_conversation(&self, server: Option<Uuid>, channel: Uuid) {
        if server.is_some() {
            self.refresh_if_stale(server);
        }
        let view = View {
            server,
            channel: Some(channel),
        };
        // Errors are not retried every frame; `reload_channel` does.
        let needed = self.with_state(|s| {
            let history = s.channels.get(&channel);
            let history_ok =
                history.is_some_and(|h| (h.loaded && !h.stale) || h.loading || h.error.is_some());
            let detail_ok = server.is_none_or(|id| {
                s.server_details.contains_key(&id) || s.op(&format!("server:{id}")).is_some()
            });
            s.view != view || !history_ok || !detail_ok
        });
        if !needed || !self.inner.opening.lock().insert(view) {
            return;
        }
        let this = self.clone();
        self.spawn(async move {
            this.change_view(view);
            if let Some(id) = server
                && !this.with_state(|s| {
                    s.server_details.contains_key(&id) || s.op(&format!("server:{id}")).is_some()
                })
            {
                let key = format!("server:{id}");
                let work = this.reload_server(id);
                this.op(&key, work).await;
            }
            let fetch = this.with_state(|s| {
                s.channels
                    .get(&channel)
                    .is_none_or(|h| !h.loading && h.error.is_none() && (!h.loaded || h.stale))
            });
            if fetch {
                this.load_latest(channel).await;
            }
            this.inner.opening.lock().remove(&view);
        });
    }

    fn change_view(&self, view: View) {
        let previous = self.with_state(|s| s.view);
        if previous == view {
            return;
        }
        if previous.channel != view.channel {
            self.stop_typing();
        }
        self.write(|s| {
            s.view = view;
            if view.server.is_some() {
                s.last_server = view.server;
            }
        });
        self.sync_topics();
    }

    /// Leaves every view (the DM list, settings without a server).
    pub fn close_view(&self) {
        let this = self.clone();
        self.spawn(async move { this.change_view(View::default()) });
    }

    async fn load_latest(&self, channel: Uuid) {
        let epoch = self.epoch();
        self.write(|s| {
            let h = s.channel_mut(channel);
            h.loading = true;
            h.error = None;
        });
        let result = self
            .http()
            .messages(channel, &HistoryQuery::latest(PAGE_SIZE))
            .await;
        self.write_in(epoch, |s| {
            let h = s.channel_mut(channel);
            match result {
                // A stale history keeps its older pages and live rows.
                Ok(page) if h.loaded => {
                    h.merge_latest(page);
                    h.loading = false;
                }
                Ok(page) => h.replace(page),
                Err(error) => {
                    h.loading = false;
                    h.error = Some(error);
                }
            }
        });
    }

    /// The next older page, while `has_more`.
    pub fn load_older(&self, channel: Uuid) {
        let cursor = self.with_state(|s| {
            s.channels
                .get(&channel)
                .filter(|h| h.loaded && h.has_more && !h.loading_older)
                .and_then(|h| h.oldest_cursor())
        });
        let Some(cursor) = cursor else {
            return;
        };
        let this = self.clone();
        self.spawn(async move {
            let epoch = this.epoch();
            this.write(|s| s.channel_mut(channel).loading_older = true);
            let result = this
                .http()
                .messages(channel, &HistoryQuery::before(cursor, PAGE_SIZE))
                .await;
            this.write_in(epoch, |s| {
                let h = s.channel_mut(channel);
                match result {
                    Ok(page) => h.prepend(page),
                    Err(error) => {
                        h.loading_older = false;
                        h.error = Some(error);
                    }
                }
            });
        });
    }

    // ------------------------------------------------------------ sending

    /// Sends text, optionally as a reply. A pending row with a fresh nonce
    /// shows at once and is swapped for the server's message.
    pub fn send_message(&self, channel: Uuid, text: &str, reply_to: Option<Uuid>) {
        self.send_with(channel, text, reply_to, None, Vec::new());
    }

    /// Text and files (uploaded one after another, then bound).
    pub fn send_files(
        &self,
        channel: Uuid,
        text: &str,
        files: Vec<OutgoingFile>,
        reply_to: Option<Uuid>,
    ) {
        self.send_with(channel, text, reply_to, None, files);
    }

    /// A GIF goes alone: no text, no files.
    pub fn send_gif(&self, channel: Uuid, gif: GifPick, reply_to: Option<Uuid>) {
        self.send_with(channel, "", reply_to, Some(gif), Vec::new());
    }

    fn send_with(
        &self,
        channel: Uuid,
        text: &str,
        reply_to: Option<Uuid>,
        gif: Option<GifPick>,
        files: Vec<OutgoingFile>,
    ) {
        if text.trim().is_empty() && gif.is_none() && files.is_empty() {
            return;
        }
        let stage = if files.is_empty() || gif.is_some() {
            SendStage::Bind
        } else {
            SendStage::Presign
        };
        let pending = PendingMessage {
            reply_to,
            gif,
            files,
            stage,
            ..PendingMessage::new(channel, text.to_owned(), self.http().server_now())
        };
        self.stop_typing();
        let this = self.clone();
        self.spawn(async move {
            this.write(|s| s.channel_mut(channel).pending.push(pending.clone()));
            this.deliver(pending).await;
        });
    }

    /// Changes the pending row `nonce` of `channel`, if it is still there
    /// (the live event may have confirmed it meanwhile).
    fn update_pending(
        &self,
        epoch: u64,
        channel: Uuid,
        nonce: Uuid,
        f: impl FnOnce(&mut PendingMessage),
    ) {
        self.write_in(epoch, |s| {
            if let Some(row) = s
                .channel_mut(channel)
                .pending
                .iter_mut()
                .find(|p| p.nonce == nonce)
            {
                f(row);
            }
        });
    }

    /// Uploads the files one after another (a retry continues after those
    /// that arrived), then binds them: each stage is kept on the row, so a
    /// failure names where it stopped (`sendMessageAttempt`).
    async fn deliver(&self, pending: PendingMessage) {
        let epoch = self.epoch();
        let http = self.http().clone();
        let (channel, nonce) = (pending.channel_id, pending.nonce);
        let mut ids = pending.uploaded.clone();
        let mut failure = None;
        if pending.gif.is_none() {
            for file in pending.files.iter().skip(ids.len()) {
                self.update_pending(epoch, channel, nonce, |p| p.stage = SendStage::Presign);
                let request = PresignRequest {
                    filename: file.filename.clone(),
                    content_type: file.content_type.clone(),
                    size: file.data.len() as i64,
                };
                let presigned = match http.presign_attachment(channel, &request).await {
                    Ok(presigned) => presigned,
                    Err(error) => {
                        failure = Some((SendStage::Presign, error));
                        break;
                    }
                };
                self.update_pending(epoch, channel, nonce, |p| p.stage = SendStage::Upload);
                let put = http
                    .put_upload(
                        &presigned.upload_url,
                        &presigned.headers,
                        file.data.as_ref().clone(),
                    )
                    .await;
                if let Err(error) = put {
                    failure = Some((SendStage::Upload, error));
                    break;
                }
                ids.push(presigned.attachment.id);
                let uploaded = ids.clone();
                self.update_pending(epoch, channel, nonce, |p| {
                    p.arrived = p.arrived.max(uploaded.len());
                    p.uploaded = uploaded;
                });
            }
        }
        let result = match failure {
            Some(failure) => Err(failure),
            None => {
                self.update_pending(epoch, channel, nonce, |p| p.stage = SendStage::Bind);
                let body = NewMessage {
                    content: pending.content.clone(),
                    attachment_ids: pending.gif.is_none().then_some(ids),
                    gif: pending.gif.clone(),
                    reply_to: pending.reply_to,
                    nonce: Some(nonce),
                };
                http.create_message(channel, &body)
                    .await
                    .map_err(|e| (SendStage::Bind, e))
            }
        };
        match result {
            Ok(message) => {
                self.write_in(epoch, |s| s.apply_message(message));
            }
            Err((stage, error)) => self.update_pending(epoch, channel, nonce, |row| {
                row.stage = stage;
                row.uncertain = crate::store::send_uncertain(stage, &error);
                // These ids may be taken (a lost bind) or were turned down:
                // a retry uploads the files again.
                if row.uncertain || crate::store::rejected_uploads(&error).is_some() {
                    row.uploaded.clear();
                }
                row.failed = Some(error);
            }),
        }
    }

    /// Sends a failed pending row again with the same nonce; never one
    /// whose bind may have been saved.
    pub fn retry_send(&self, channel: Uuid, nonce: Uuid) {
        let this = self.clone();
        self.spawn(async move {
            let pending = this.write(|s| {
                let row = s
                    .channel_mut(channel)
                    .pending
                    .iter_mut()
                    .find(|p| p.nonce == nonce && p.failed.is_some() && !p.uncertain)?;
                row.failed = None;
                Some(row.clone())
            });
            if let Some(pending) = pending {
                this.deliver(pending).await;
            }
        });
    }

    pub fn discard_pending(&self, channel: Uuid, nonce: Uuid) {
        let this = self.clone();
        self.spawn(async move {
            this.write(|s| s.channel_mut(channel).pending.retain(|p| p.nonce != nonce));
        });
    }

    /// A write to message `id` as the named op; a failed one fetches its
    /// conversation again (`invalidateQueries`): it may be gone or changed.
    fn message_write<T, F, A>(&self, op: String, id: Uuid, work: F, apply: A)
    where
        T: Send + 'static,
        F: Future<Output = Result<T, ApiError>> + Send + 'static,
        A: FnOnce(&mut State, T) + Send + 'static,
    {
        let channel = self.with_state(|s| s.find_message(id).map(|m| m.channel_id));
        let this = self.clone();
        self.spawn(async move {
            let epoch = this.epoch();
            match this.op(&op, work).await {
                Some(value) => {
                    this.write_in(epoch, |s| apply(s, value));
                }
                None => {
                    // Gone on the server: the newer-than-the-page rule of a
                    // refetch would keep the row, so it goes here.
                    let gone = this.with_state(|s| {
                        matches!(s.op(&op), Some(OpState::Failed(e)) if e.code() == "not_found")
                    });
                    if let Some(channel) = channel
                        && this.epoch() == epoch
                    {
                        if gone {
                            this.write_in(epoch, |s| s.apply_delete(channel, id));
                        }
                        this.load_latest(channel).await;
                    }
                }
            }
        });
    }

    /// Op key `edit:<id>`.
    pub fn edit_message(&self, id: Uuid, content: &str) {
        let http = self.http().clone();
        let content = content.to_owned();
        self.message_write(
            format!("edit:{id}"),
            id,
            async move { http.edit_message(id, &content).await },
            |s, m| s.apply_message(m),
        );
    }

    /// Op key `delete:<id>`.
    pub fn delete_message(&self, id: Uuid) {
        let channel = self.with_state(|s| s.find_message(id).map(|m| m.channel_id));
        let http = self.http().clone();
        self.message_write(
            format!("delete:{id}"),
            id,
            async move { http.delete_message(id).await },
            move |s, ()| {
                if let Some(channel) = channel {
                    s.apply_delete(channel, id);
                }
            },
        );
    }

    /// Adds my reaction, or removes it when I already reacted. Op key
    /// `react:<id>`.
    pub fn toggle_reaction(&self, id: Uuid, emoji: &str) {
        let mine = self.with_state(|s| {
            let me = s.me().map(|u| u.id)?;
            s.find_message(id).map(|m| m.reacted(emoji, me))
        });
        self.set_reaction(id, emoji, !mine.unwrap_or(false));
    }

    pub fn set_reaction(&self, id: Uuid, emoji: &str, on: bool) {
        let http = self.http().clone();
        let emoji = emoji.to_owned();
        self.message_write(
            format!("react:{id}"),
            id,
            async move {
                if on {
                    http.add_reaction(id, &emoji).await
                } else {
                    http.remove_reaction(id, &emoji).await
                }
            },
            |s, m| s.apply_message(m),
        );
    }

    /// Removes a message's link previews for good (author or moderator).
    pub fn suppress_previews(&self, id: Uuid) {
        let http = self.http().clone();
        self.run(
            format!("previews:{id}"),
            async move { http.suppress_previews(id).await },
            |_, ()| {},
        );
    }

    // ---------------------------------------------------------------- read

    /// Marks a conversation read up to its newest message.
    pub fn mark_read(&self, channel: Uuid) {
        let this = self.clone();
        self.spawn(async move { this.mark_read_now(channel).await });
    }

    async fn mark_read_now(&self, channel: Uuid) {
        let is_text = self.with_state(|s| {
            s.dm(channel).is_some()
                || s.channel(channel)
                    .is_some_and(|c| c.kind == ChannelKind::Text)
        });
        if !is_text {
            return;
        }
        let epoch = self.epoch();
        let result = self.http().mark_read(channel, None).await;
        self.write_in(epoch, |s| {
            let failed = result.is_err();
            if let Ok(row) = result {
                s.apply_read_states(vec![row]);
            }
            if failed || s.channels.contains_key(&channel) {
                s.channel_mut(channel).read_failed = failed;
            }
        });
    }

    /// "Als gelesen markieren" on one conversation (a row menu, the
    /// notification dialog): op key `read:<id>`, whose failure is a toast,
    /// as the web's `notifyError` there. Reading it marks it by itself.
    pub fn mark_conversation_read(&self, channel: Uuid) {
        let http = self.http().clone();
        self.run(
            format!("read:{channel}"),
            async move { http.mark_read(channel, None).await },
            move |s, row| {
                s.apply_read_states(vec![row]);
                if s.channels.contains_key(&channel) {
                    s.channel_mut(channel).read_failed = false;
                }
            },
        );
    }

    /// "Als gelesen markieren" on a server.
    pub fn mark_server_read(&self, server: Uuid) {
        let http = self.http().clone();
        self.run(
            format!("read:{server}"),
            async move { http.mark_server_read(server).await },
            |s, rows| s.apply_read_states(rows),
        );
    }

    pub fn mark_all_read(&self, scope: ReadScope) {
        let http = self.http().clone();
        self.run(
            "read_all",
            async move { http.mark_all_read(scope).await },
            |s, rows| s.apply_read_states(rows),
        );
    }

    // ------------------------------------------------------------------ DMs

    /// Opens (or creates) the DM with a user and shows it. Op key `dm`.
    pub fn start_dm(&self, user: Uuid) {
        let this = self.clone();
        self.spawn(async move {
            let http = this.http().clone();
            if let Some(dm) = this.op("dm", http.open_dm(user)).await {
                let id = dm.id;
                this.write(|s| {
                    if s.dm(id).is_none() {
                        s.dms.insert(0, dm);
                    }
                });
                this.open_conversation(None, id);
            }
        });
    }

    pub fn load_profile(&self, user: Uuid) {
        let http = self.http().clone();
        self.run(
            format!("profile:{user}"),
            async move { http.user_profile(user).await },
            move |s, p| {
                s.profiles.insert(user, p);
            },
        );
    }

    // ---------------------------------------------------------- live input

    /// Composer text changed (`useTypingInput`): announces typing once and
    /// keeps it fresh every 3 s; empty text stops it. No state lock.
    pub fn typing_input(&self, channel: Uuid, text: &str) {
        if text.is_empty() {
            self.stop_typing();
            return;
        }
        let server = self.with_state(|s| {
            if s.dm(channel).is_some() {
                Some(channel)
            } else {
                s.channel(channel).and_then(|c| c.server_id)
            }
        });
        let Some(server) = server else {
            return;
        };
        let start = {
            let mut local = self.inner.local.lock();
            if local.typing.is_some_and(|(_, c)| c == channel) {
                false
            } else {
                local.typing = Some((server, channel));
                local.typing_sent = Some(Instant::now());
                true
            }
        };
        if start {
            self.send_frame(ClientFrame::Typing {
                s: server,
                c: channel,
                on: true,
            });
        }
    }

    /// Message sent, text cleared or conversation left.
    pub fn stop_typing(&self) {
        let target = {
            let mut local = self.inner.local.lock();
            local.typing_sent = None;
            local.typing.take()
        };
        if let Some((s, c)) = target {
            self.send_frame(ClientFrame::Typing { s, c, on: false });
        }
    }

    /// Pointer or key input (`useIdlePresence`): active again after idle,
    /// otherwise a presence pulse at most every 10 s.
    pub fn activity(&self) {
        let now = Instant::now();
        let (send, front) = {
            let mut local = self.inner.local.lock();
            local.last_activity = Some(now);
            let send = local.idle || local.last_pulse.is_none_or(|t| now - t >= PRESENCE_PULSE);
            local.idle = false;
            if send {
                local.last_pulse = Some(now);
            }
            (send, local.focused)
        };
        if send {
            self.send_frame(ClientFrame::Presence {
                st: Some(PresenceStatus::Online),
            });
        }
        self.update_front(front);
    }

    /// The window gained or lost focus/visibility. Gaining it refetches a
    /// stale server list and detail, as the web does on focus.
    pub fn set_focused(&self, focused: bool) {
        let gained = !std::mem::replace(&mut self.inner.local.lock().focused, focused) && focused;
        self.update_front(focused);
        if gained {
            let server = self.with_state(|s| s.view.server);
            self.refresh_if_stale(server);
        }
    }

    fn update_front(&self, focused: bool) {
        let front = {
            let mut local = self.inner.local.lock();
            let front = focused && !local.idle;
            let changed = local.in_front != front;
            local.in_front = front;
            changed.then_some(front)
        };
        if let (Some(front), Some(gateway)) = (front, self.gateway()) {
            gateway.set_focus(front);
        }
    }

    /// Typing expiry, typing refresh and the idle timer, every 400 ms.
    pub(crate) fn start_ticker(&self) {
        let weak = self.weak();
        self.runtime().spawn(async move {
            let mut tick = tokio::time::interval(TICK);
            loop {
                tick.tick().await;
                let Some(client) = Client::from_weak(&weak) else {
                    return;
                };
                client.on_tick();
            }
        });
    }

    fn on_tick(&self) {
        let now = Instant::now();
        let expired = self.with_state(|s| {
            s.typing
                .values()
                .any(|m| m.values().any(|until| *until <= now))
        });
        if expired {
            self.write(|s| s.prune_typing(now));
        }
        let (refresh, idle_now, focused) = {
            let mut local = self.inner.local.lock();
            let refresh = match (local.typing, local.typing_sent) {
                (Some(target), Some(sent)) if now - sent >= TYPING_REFRESH => {
                    local.typing_sent = Some(now);
                    Some(target)
                }
                _ => None,
            };
            let idle_now =
                !local.idle && local.last_activity.is_some_and(|t| now - t >= CLIENT_IDLE);
            if idle_now {
                local.idle = true;
            }
            (refresh, idle_now, local.focused)
        };
        if let Some((s, c)) = refresh {
            self.send_frame(ClientFrame::Typing { s, c, on: true });
        }
        if idle_now {
            self.send_frame(ClientFrame::Presence {
                st: Some(PresenceStatus::Idle),
            });
            self.update_front(focused);
        }
    }

    // --------------------------------------------------------------- voice

    /// Takes a seat in a voice channel on the gateway (presence only; the
    /// media connection is separate). Leaves another seat first.
    pub fn voice_join(&self, server: Uuid, channel: Uuid) {
        self.voice_join_flags(server, channel, None);
    }

    /// [`Self::voice_join`] with the mute and deafen flags to take into the
    /// channel; `None` keeps the current ones.
    pub fn voice_join_flags(&self, server: Uuid, channel: Uuid, flags: Option<(bool, bool)>) {
        self.voice_queue(move |this| {
            let mut current = this.with_state(|s| s.voice_self);
            if let Some((muted, deafened)) = flags {
                current.muted = muted || deafened;
                current.deafened = deafened;
                if !deafened {
                    this.inner.local.lock().muted_before_deafen = muted;
                }
            }
            if let (Some(s), Some(c)) = (current.server, current.channel)
                && (s, c) != (server, channel)
            {
                this.send_frame(ClientFrame::Sig {
                    t: SigKind::L,
                    s,
                    c,
                    k: None,
                    on: None,
                    o: None,
                });
            }
            let me = this.with_state(|s| s.me().map(|u| u.id));
            this.write(|s| {
                s.voice_self = VoiceSelf {
                    server: Some(server),
                    channel: Some(channel),
                    ..current
                };
                if let Some(me) = me {
                    s.apply_sig(&SigFrame {
                        t: SigKind::J,
                        s: server,
                        c: Some(channel),
                        u: Some(me),
                        k: None,
                        on: None,
                        m: Some(current.muted),
                        d: Some(current.deafened),
                        snap: None,
                        replay: false,
                        w: None,
                    });
                }
            });
            this.sync_topics();
            this.send_join_frames();
        });
    }

    /// The join frame has no mute fields: `m`/`d` follow when set.
    fn send_join_frames(&self) {
        let me = self.with_state(|s| s.voice_self);
        let (Some(s), Some(c)) = (me.server, me.channel) else {
            return;
        };
        let sig = |t, on| ClientFrame::Sig {
            t,
            s,
            c,
            k: None,
            on,
            o: None,
        };
        self.send_frame(sig(SigKind::J, None));
        if me.muted {
            self.send_frame(sig(SigKind::M, Some(true)));
        }
        if me.deafened {
            self.send_frame(sig(SigKind::D, Some(true)));
        }
    }

    pub fn voice_leave(&self) {
        self.voice_queue(move |this| {
            let current = this.with_state(|s| s.voice_self);
            let (Some(s), Some(c)) = (current.server, current.channel) else {
                return;
            };
            this.send_frame(ClientFrame::Sig {
                t: SigKind::L,
                s,
                c,
                k: None,
                on: None,
                o: None,
            });
            let me = this.with_state(|st| st.me().map(|u| u.id));
            this.write(|st| {
                st.voice_self = VoiceSelf {
                    muted: current.muted,
                    deafened: current.deafened,
                    ..VoiceSelf::default()
                };
                if let Some(me) = me {
                    st.apply_sig(&SigFrame {
                        t: SigKind::L,
                        s,
                        c: Some(c),
                        u: Some(me),
                        k: None,
                        on: None,
                        m: None,
                        d: None,
                        snap: None,
                        replay: false,
                        w: None,
                    });
                }
            });
            this.sync_topics();
        });
    }

    /// Mute; unmuting while deafened also undeafens (`session.ts`).
    pub fn set_voice_muted(&self, muted: bool) {
        self.voice_queue(move |this| {
            let current = this.with_state(|s| s.voice_self);
            let deafened = current.deafened && muted;
            this.write(|s| {
                s.voice_self.muted = muted;
                s.voice_self.deafened = deafened;
            });
            if let (Some(s), Some(c)) = (current.server, current.channel) {
                if current.muted != muted {
                    this.send_frame(ClientFrame::Sig {
                        t: SigKind::M,
                        s,
                        c,
                        k: None,
                        on: Some(muted),
                        o: None,
                    });
                }
                if current.deafened != deafened {
                    this.send_frame(ClientFrame::Sig {
                        t: SigKind::D,
                        s,
                        c,
                        k: None,
                        on: Some(deafened),
                        o: None,
                    });
                }
            }
        });
    }

    /// Deafen implies mute; undeafen restores the mute from before.
    pub fn set_voice_deafened(&self, deafened: bool) {
        self.voice_queue(move |this| {
            let current = this.with_state(|s| s.voice_self);
            if current.deafened == deafened {
                return;
            }
            let muted = {
                let mut local = this.inner.local.lock();
                if deafened {
                    local.muted_before_deafen = current.muted;
                    true
                } else {
                    local.muted_before_deafen
                }
            };
            this.write(|s| {
                s.voice_self.deafened = deafened;
                s.voice_self.muted = muted;
            });
            if let (Some(s), Some(c)) = (current.server, current.channel) {
                if current.muted != muted {
                    this.send_frame(ClientFrame::Sig {
                        t: SigKind::M,
                        s,
                        c,
                        k: None,
                        on: Some(muted),
                        o: None,
                    });
                }
                this.send_frame(ClientFrame::Sig {
                    t: SigKind::D,
                    s,
                    c,
                    k: None,
                    on: Some(deafened),
                    o: None,
                });
            }
        });
    }

    /// Announces (`on`) or retracts a published track of my seat.
    pub fn voice_publish(&self, kind: TrackKind, on: bool) {
        self.voice_queue(move |this| {
            let me = this.with_state(|s| s.voice_self);
            let (Some(s), Some(c)) = (me.server, me.channel) else {
                return;
            };
            this.send_frame(ClientFrame::Sig {
                t: if on { SigKind::P } else { SigKind::U },
                s,
                c,
                k: Some(kind),
                on: None,
                o: None,
            });
        });
    }

    /// Announces (`on`) or retracts watching `owner`'s Live from my seat;
    /// the gateway tells the room who watches (`sig t:"w"`).
    pub fn voice_watch(&self, owner: Uuid, on: bool) {
        self.voice_queue(move |this| {
            let me = this.with_state(|s| s.voice_self);
            let (Some(s), Some(c)) = (me.server, me.channel) else {
                return;
            };
            this.send_frame(ClientFrame::Sig {
                t: SigKind::W,
                s,
                c,
                k: Some(TrackKind::S),
                on: Some(on),
                o: Some(owner),
            });
        });
    }

    // -------------------------------------------------- servers & channels

    /// Op key `create_server`; the new server joins the list.
    pub fn create_server(&self, name: &str) {
        let name = name.to_owned();
        let this = self.clone();
        self.spawn(async move {
            let epoch = this.epoch();
            let http = this.http().clone();
            if let Some(detail) = this.op("create_server", http.create_server(&name)).await {
                this.write_in(epoch, |s| {
                    if s.server(detail.server.id).is_none() {
                        s.servers.push(detail.server.clone());
                    }
                    s.server_details.insert(detail.server.id, detail);
                });
                // Now that the state has it.
                this.sync_topics();
            }
        });
    }

    /// Op key `join`; refetches the list.
    pub fn join_invite(&self, code: &str) {
        let this = self.clone();
        let code = code.to_owned();
        self.spawn(async move {
            let http = this.http().clone();
            if this.op("join", http.join_invite(&code)).await.is_some() {
                this.reload_servers().await;
                this.reload_unread().await;
            }
        });
    }

    /// Op key `server_update:<id>`.
    pub fn update_server(&self, id: Uuid, patch: ServerPatch) {
        self.server_update(format!("server_update:{id}"), id, patch);
    }

    /// "Alle Mitglieder": the members' permissions, one op per write so
    /// quick toggles each get their own answer. Op key
    /// `server_update:<id>:<seq>`.
    pub fn update_member_permissions(&self, id: Uuid, permissions: Vec<Permission>, seq: u64) {
        let patch = ServerPatch {
            name: None,
            member_permissions: Some(permissions),
        };
        self.server_update(format!("server_update:{id}:{seq}"), id, patch);
    }

    fn server_update(&self, op: String, id: Uuid, patch: ServerPatch) {
        let http = self.http().clone();
        self.run(
            op,
            async move { http.update_server(id, &patch).await },
            move |s, server| {
                if let Some(entry) = s.servers.iter_mut().find(|v| v.id == id) {
                    *entry = server.clone();
                }
                if let Some(detail) = s.server_details.get_mut(&id) {
                    detail.server = server;
                }
            },
        );
    }

    pub fn leave_server(&self, id: Uuid) {
        self.drop_server(id, false);
    }

    /// Owner only.
    pub fn delete_server(&self, id: Uuid) {
        self.drop_server(id, true);
    }

    fn drop_server(&self, id: Uuid, delete: bool) {
        let key = if delete {
            format!("delete_server:{id}")
        } else {
            format!("leave:{id}")
        };
        // Pending at once: the shell reads an earlier failure of this key
        // as this request's end and would offer the server again.
        self.write(|s| s.ops.insert(key.clone(), OpState::Pending));
        let this = self.clone();
        self.spawn(async move {
            let http = this.http().clone();
            let work = async move {
                if delete {
                    http.delete_server(id).await
                } else {
                    http.leave_server(id).await
                }
            };
            if this.op(&key, work).await.is_some() {
                this.write(|s| s.forget_server(id));
                this.sync_topics();
            }
        });
    }

    /// Changes on a server, then its detail is fetched again. Op key `op`.
    pub fn server_change<T, F>(&self, op: impl Into<String>, server: Uuid, work: F)
    where
        T: Send + 'static,
        F: Future<Output = Result<T, ApiError>> + Send + 'static,
    {
        let this = self.clone();
        let op = op.into();
        self.spawn(async move {
            if this.op(&op, work).await.is_some() {
                let _ = this.reload_server(server).await;
            }
        });
    }

    /// `role_ids` limits the new channel to those roles; `None`: everyone.
    pub fn create_channel(
        &self,
        server: Uuid,
        name: &str,
        kind: ChannelKind,
        category: Option<Uuid>,
        role_ids: Option<Vec<Uuid>>,
    ) {
        let http = self.http().clone();
        let name = name.to_owned();
        self.server_change("create_channel", server, async move {
            http.create_channel(server, &name, kind, category, role_ids.as_deref())
                .await
        });
    }

    pub fn update_channel(&self, server: Uuid, channel: Uuid, patch: ChannelPatch) {
        let http = self.http().clone();
        self.server_change(format!("channel:{channel}"), server, async move {
            http.update_channel(channel, &patch).await
        });
    }

    pub fn delete_channel(&self, server: Uuid, channel: Uuid) {
        let http = self.http().clone();
        self.server_change(format!("delete_channel:{channel}"), server, async move {
            http.delete_channel(channel).await
        });
    }

    pub fn create_category(&self, server: Uuid, name: &str) {
        let http = self.http().clone();
        let name = name.to_owned();
        self.server_change("create_category", server, async move {
            http.create_category(server, &name).await
        });
    }

    pub fn rename_category(&self, server: Uuid, category: Uuid, name: &str) {
        let http = self.http().clone();
        let name = name.to_owned();
        self.server_change(format!("category:{category}"), server, async move {
            http.rename_category(category, &name).await
        });
    }

    pub fn delete_category(&self, server: Uuid, category: Uuid) {
        let http = self.http().clone();
        self.server_change(format!("delete_category:{category}"), server, async move {
            http.delete_category(category).await
        });
    }

    pub fn kick(&self, server: Uuid, user: Uuid) {
        let http = self.http().clone();
        self.server_change(format!("kick:{user}"), server, async move {
            http.kick(server, user).await
        });
    }

    pub fn ban(&self, server: Uuid, user: Uuid) {
        let http = self.http().clone();
        self.server_change(format!("ban:{user}"), server, async move {
            http.ban(server, user).await
        });
    }

    /// A role write, then the server's detail and the list are read again:
    /// the caller's own rights may have changed. Op key `op`.
    fn role_change<T, F>(&self, op: String, server: Uuid, work: F)
    where
        T: Send + 'static,
        F: Future<Output = Result<T, ApiError>> + Send + 'static,
    {
        let this = self.clone();
        self.spawn(async move {
            if this.op(&op, work).await.is_some() {
                this.reload_servers().await;
                let _ = this.reload_server(server).await;
            }
        });
    }

    /// Op key `create_role:<server>`. The new role is in the detail at once.
    pub fn create_role(&self, server: Uuid, role: NewRole) {
        let http = self.http().clone();
        let this = self.clone();
        self.role_change(format!("create_role:{server}"), server, async move {
            let epoch = this.epoch();
            let created = http.create_role(server, &role).await?;
            this.write_in(epoch, |s| {
                if let Some(detail) = s.server_details.get_mut(&server)
                    && !detail.roles.iter().any(|r| r.id == created.id)
                {
                    detail.roles.push(created.clone());
                }
            });
            Ok(created)
        });
    }

    /// Op key `role:<id>:<seq>`: each write its own answer.
    pub fn update_role(&self, server: Uuid, role: Uuid, patch: RolePatch, seq: u64) {
        let http = self.http().clone();
        self.role_change(format!("role:{role}:{seq}"), server, async move {
            http.update_role(role, &patch).await
        });
    }

    /// Op key `delete_role:<id>`.
    pub fn delete_role(&self, server: Uuid, role: Uuid) {
        let http = self.http().clone();
        self.role_change(format!("delete_role:{role}"), server, async move {
            http.delete_role(role).await
        });
    }

    /// Full replacement of a member's custom roles. Op key
    /// `member_roles:<user>:<seq>`: each write its own answer.
    pub fn set_member_roles(&self, server: Uuid, user: Uuid, role_ids: Vec<Uuid>, seq: u64) {
        let http = self.http().clone();
        self.role_change(format!("member_roles:{user}:{seq}"), server, async move {
            http.set_member_roles(server, user, &role_ids).await
        });
    }

    /// Revokes an invite link in the background (one a dialog made and
    /// nobody got). Op key `revoke:<code>`.
    pub fn revoke_invite(&self, code: &str) {
        let http = self.http().clone();
        let code = code.to_owned();
        self.run(
            format!("revoke:{code}"),
            async move { http.revoke_invite(&code).await },
            |_, ()| {},
        );
    }

    // ------------------------------------------------- account & settings

    /// Op key `profile`; the session user is replaced by the answer.
    pub fn update_profile(&self, patch: ProfilePatch) {
        let http = self.http().clone();
        self.run(
            "profile",
            async move { http.update_me(&patch).await },
            |s, user| s.session = Session::Authenticated(Box::new(user)),
        );
    }

    /// Op key `avatar`.
    pub fn upload_avatar(&self, content_type: &str, data: Vec<u8>) {
        let http = self.http().clone();
        let content_type = content_type.to_owned();
        self.run(
            "avatar",
            async move { http.upload_avatar(&content_type, data).await },
            |s, user| s.session = Session::Authenticated(Box::new(user)),
        );
    }

    /// Op key `banner`.
    pub fn upload_banner(&self, content_type: &str, data: Vec<u8>) {
        let http = self.http().clone();
        let content_type = content_type.to_owned();
        self.run(
            "banner",
            async move { http.upload_banner(&content_type, data).await },
            |s, user| s.session = Session::Authenticated(Box::new(user)),
        );
    }

    /// Op key `themes`; `theme_conflict` when another device saved first.
    pub fn save_themes(&self, doc: ThemeDocument) {
        let http = self.http().clone();
        self.run(
            "themes",
            async move { http.put_themes(&doc).await },
            |s, doc| s.themes = Some(doc),
        );
    }

    /// Op key `notifications`; settings and unread levels are refetched.
    /// The dialog shows a failure itself.
    pub fn set_notifications(&self, target: SettingTarget, body: NotificationSettingBody) {
        self.save_notifications("notifications", target, body);
    }

    /// Like `set_notifications`, from a row menu that is gone by the time
    /// the answer comes: op key `notify_menu`, whose failure is a toast.
    pub fn set_notifications_from_menu(
        &self,
        target: SettingTarget,
        body: NotificationSettingBody,
    ) {
        self.save_notifications("notify_menu", target, body);
    }

    fn save_notifications(
        &self,
        op: &'static str,
        target: SettingTarget,
        body: NotificationSettingBody,
    ) {
        let this = self.clone();
        self.spawn(async move {
            let http = this.http().clone();
            let work = async move {
                // Would overwrite a newer server's level with a guess.
                if body.level == Some(NotificationLevel::Unknown) {
                    return Err(ApiError::Api {
                        status: 0,
                        code: "unknown_level".into(),
                        message: gelabber_i18n::shell::notify_unknown_level().into(),
                        fields: serde_json::Value::Null,
                        retry_after: None,
                    });
                }
                match target {
                    SettingTarget::Server(id) => http.set_server_notifications(id, &body).await,
                    SettingTarget::Channel(id) => http.set_channel_notifications(id, &body).await,
                }
            };
            if this.op(op, work).await.is_some() {
                this.reload_notification_settings().await;
            }
        });
    }

    async fn reload_notification_settings(&self) {
        let http = self.http().clone();
        let epoch = self.epoch();
        let (settings, unread) = tokio::join!(http.notification_settings(), http.unread());
        self.write_in(epoch, |s| {
            if let Ok(list) = settings {
                s.notification_settings = list;
            }
            if let Ok(rows) = unread {
                s.replace_read_states(rows);
            }
        });
    }

    pub fn set_notification_privacy(&self, hide_content: bool) {
        let http = self.http().clone();
        self.run(
            "privacy",
            async move { http.set_notification_privacy(hide_content).await },
            |s, p| s.notification_privacy = Some(p),
        );
    }

    pub fn load_gif_status(&self) {
        let http = self.http().clone();
        self.run("gifs", async move { http.gif_status().await }, |s, g| {
            s.gifs = Some(g)
        });
    }

    /// Op key `locale`: saves the account's language, `de` or `en`, which
    /// the interface shows already. Only the newest change speaks for the
    /// account: an older answer that arrives late is not taken, and when
    /// the newest fails the session user keeps what the server confirmed
    /// last, an older change's or the one before.
    pub fn set_locale(&self, locale: &str) {
        let this = self.clone();
        let locale = locale.to_owned();
        let change = self.inner.locale_change.fetch_add(1, Ordering::Relaxed) + 1;
        let epoch = self.epoch();
        // At once, so the page never reads the last change's answer as this one's.
        self.write(|s| s.ops.insert("locale".into(), OpState::Pending));
        self.spawn(async move {
            let result = this.http().set_locale(&locale).await;
            let newest = this.inner.locale_change.load(Ordering::Relaxed) == change;
            match result {
                Ok(saved) if newest => {
                    *this.inner.locale_meanwhile.lock() = None;
                    this.write_in(epoch, |s| {
                        record_locale(s, saved.locale);
                        s.ops.insert("locale".into(), OpState::Done)
                    });
                }
                Ok(saved) => *this.inner.locale_meanwhile.lock() = Some((epoch, saved.locale)),
                Err(error) if newest => {
                    let meanwhile = this.inner.locale_meanwhile.lock().take();
                    this.write_in(epoch, |s| {
                        if let Some((at, locale)) = meanwhile
                            && at == epoch
                        {
                            record_locale(s, locale);
                        }
                        s.ops.insert("locale".into(), OpState::Failed(error))
                    });
                }
                Err(_) => {}
            }
        });
    }

    /// The account's signed-in devices into `State::devices`. Op key
    /// `devices`.
    pub fn load_devices(&self) {
        let http = self.http().clone();
        self.run(
            "devices",
            async move { http.sessions().await },
            |s, list| s.devices = Some(list),
        );
    }

    /// "Abmelden" on another device's row (`signOutDevice`): ended through
    /// the API without asking, then the list is read again; one that is
    /// gone already only refreshes the list. This device's row signs out
    /// as the user panel does ([`Self::logout`]). Op key
    /// `revoke_device:<id>`, done once the list is read again.
    pub fn revoke_device(&self, id: Uuid) {
        let http = self.http().clone();
        self.run(
            format!("revoke_device:{id}"),
            async move {
                match http.revoke_session(id).await {
                    Err(e) if e.code() != "not_found" => return Err(e),
                    _ => {}
                }
                http.sessions().await
            },
            |s, list| s.devices = Some(list),
        );
    }

    /// The data export into `State::export`. While it is being built the
    /// page calls this every [`EXPORT_POLL`] while it is shown. Op key
    /// `export`.
    pub fn load_export(&self) {
        let http = self.http().clone();
        self.run(
            "export",
            async move { http.export_status().await },
            |s, e| s.export = Some(e),
        );
    }

    /// "Export anfordern" / "Erneut anfordern". `export_pending` (409) and
    /// `rate_limited` (429, with `retry_after`) fail the op. Op key
    /// `request_export`.
    pub fn request_export(&self) {
        let http = self.http().clone();
        self.run(
            "request_export",
            async move { http.request_export().await },
            |s, e| s.export = Some(e),
        );
    }

    /// "Herunterladen": streams the ready archive into `target`
    /// ([`Http::download_to`](crate::Http::download_to)) and hands the
    /// outcome to `done`, which says so wherever the person is by then.
    /// Op key `export_download`, pending while it runs and gone after.
    pub fn download_export(
        &self,
        target: std::path::PathBuf,
        done: impl FnOnce(Result<(), crate::DownloadError>) + Send + 'static,
    ) {
        let http = self.http().clone();
        self.write(|s| s.ops.insert("export_download".into(), OpState::Pending));
        let this = self.clone();
        self.spawn(async move {
            let result = http.download_to(EXPORT_DOWNLOAD_PATH, &target).await;
            this.write(|s| s.ops.remove("export_download"));
            done(result.map(drop));
        });
    }

    /// Deletes the account (`deleteAndLeave`). On `202` this client signs
    /// out as [`Self::logout`] does, without a request of its own, and
    /// `State::account_deleted` asks the login page for its notice; the
    /// interface leaves the call. `409 owns_servers` puts the servers in
    /// the way into `State::owned_servers` and fails the op with that code.
    /// Op key `delete_account`.
    pub fn delete_account(&self, confirmation: DeleteConfirmation) {
        // Pending at once: a 401 from the closing socket meanwhile is this
        // deletion, not an expired session.
        self.write(|s| {
            s.owned_servers = None;
            s.ops.insert("delete_account".into(), OpState::Pending)
        });
        let this = self.clone();
        self.spawn(async move {
            let epoch = this.epoch();
            let http = this.http().clone();
            match http.delete_account(&confirmation).await {
                Ok(AccountDeletion::Deleted) => {
                    this.stop_typing();
                    this.stop_gateway();
                    this.write(|s| {
                        s.reset_account();
                        s.session = Session::Anonymous;
                        s.account_deleted = true;
                        s.ops.insert("delete_account".into(), OpState::Done);
                    });
                }
                Ok(AccountDeletion::OwnsServers(servers)) => {
                    this.write_in(epoch, |s| s.owned_servers = Some(servers));
                    if this.epoch() == epoch {
                        this.fail_op("delete_account", owns_servers());
                    }
                }
                Err(error) => {
                    let ended = error.is_unauthenticated();
                    if this.epoch() == epoch {
                        this.fail_op("delete_account", error);
                    }
                    if ended {
                        this.signed_out_by_server();
                    }
                }
            }
        });
    }

    /// "Übergeben": `user` becomes the server's owner. The answer (the
    /// caller now a member) goes into the list and the detail in the same
    /// write that settles the op, as the web's `mergeServer`, so the
    /// owner's Gefahrenzone is gone at once; then the detail is read
    /// again. Op key `transfer:<server>`.
    pub fn transfer_server(&self, server: Uuid, user: Uuid) {
        let http = self.http().clone();
        let op = format!("transfer:{server}");
        let epoch = self.write(|s| {
            s.ops.insert(op.clone(), OpState::Pending);
            s.epoch
        });
        let this = self.clone();
        self.spawn(async move {
            match http.transfer_server(server, user).await {
                Ok(view) => {
                    this.write_in(epoch, |s| {
                        s.ops.insert(op, OpState::Done);
                        if let Some(entry) = s.servers.iter_mut().find(|v| v.id == server) {
                            *entry = view.clone();
                        }
                        if let Some(detail) = s.server_details.get_mut(&server) {
                            detail.server = view;
                        }
                    });
                    let _ = this.reload_server(server).await;
                }
                Err(error) => {
                    if this.epoch() == epoch {
                        this.fail_op(&op, error);
                    }
                }
            }
        });
    }

    /// "Beitreten" in Entdecken (`useJoinListing`): the server is in the
    /// list as soon as the op is done (`rememberJoinedServer`), then the
    /// list and the unread counts are read again. Op key
    /// `join_listing:<server>`.
    pub fn join_listing(&self, server: Uuid) {
        let key = format!("join_listing:{server}");
        let epoch = self.write(|s| {
            s.ops.insert(key.clone(), OpState::Pending);
            s.epoch
        });
        let this = self.clone();
        self.spawn(async move {
            let http = this.http().clone();
            match http.join_listing(server).await {
                Ok(joined) => {
                    let kept = this.write_in(epoch, |s| {
                        if s.server(joined.id).is_none() {
                            s.servers.push(joined);
                        }
                        s.ops.insert(key, OpState::Done);
                    });
                    if kept.is_some() {
                        this.sync_topics();
                        this.reload_servers().await;
                        this.reload_unread().await;
                    }
                }
                Err(error) => {
                    if this.epoch() == epoch {
                        this.fail_op(&key, error);
                    }
                }
            }
        });
    }

    // ------------------------------------------------------------- gateway

    fn ensure_gateway(&self) {
        let mut slot = self.inner.gateway.lock();
        if slot.is_some() {
            return;
        }
        let generation = self.inner.gateway_generation.fetch_add(1, Ordering::AcqRel) + 1;
        let weak = self.weak();
        // A stopped socket may still hand over a frame or its last status;
        // after a sign-out that belongs to nobody.
        let handler = Arc::new(move |event: GatewayEvent| {
            if let Some(client) = Client::from_weak(&weak)
                && client.inner.gateway_generation.load(Ordering::Acquire) == generation
            {
                client.on_gateway(event);
            }
        });
        let config = *self.inner.gateway_config.lock();
        *slot = Some(Gateway::start(
            self.runtime().handle(),
            self.http().clone(),
            config,
            handler,
        ));
        // A focus claim made before the socket existed.
        let front = self.inner.local.lock().in_front;
        if front && let Some(gateway) = slot.as_ref() {
            gateway.set_focus(true);
        }
    }

    fn stop_gateway(&self) {
        let mut slot = self.inner.gateway.lock();
        self.inner.gateway_generation.fetch_add(1, Ordering::AcqRel);
        if let Some(gateway) = slot.take() {
            gateway.stop();
        }
    }

    pub(crate) fn on_gateway(&self, event: GatewayEvent) {
        match event {
            GatewayEvent::Status(status) => self.write(|s| s.gateway = status),
            GatewayEvent::Ready => {
                // The server marks a new socket online; say so if we idle.
                if self.inner.local.lock().idle {
                    self.send_frame(ClientFrame::Presence {
                        st: Some(PresenceStatus::Idle),
                    });
                }
                if self.with_state(|s| s.voice_self.channel.is_some()) {
                    self.voice_hook(&GatewayEvent::Ready);
                    self.send_join_frames();
                }
                self.schedule_recovery(true);
            }
            GatewayEvent::Event(event) => {
                let (removed, created) = self.write(|s| {
                    let removed = s.apply_event(&event);
                    let me = s.me().map(|u| u.id);
                    let created = (event.t == EventKind::C)
                        .then(|| event.message())
                        .flatten()
                        .filter(|m| Some(m.author.id) != me);
                    (removed, created)
                });
                if let Some((server, banned)) = removed {
                    self.removed_from(server, banned);
                } else if event.is_access_change() {
                    self.refresh_access(event.s);
                }
                // Ownership moved (`k: "o"`): the caller's role and rights
                // may have too.
                if event.c.is_none() && event.t == EventKind::E && event.kind() == Some("o") {
                    let this = self.clone();
                    let server = event.s;
                    self.spawn(async move {
                        this.reload_servers().await;
                        let _ = this.reload_server(server).await;
                    });
                }
                if let Some(message) = created {
                    self.notify(Notice::Message {
                        server: event.s,
                        message: Box::new(message),
                    });
                }
                self.schedule_unread_refresh();
                let tap = self.inner.event_tap.read().clone();
                if let Some(tap) = tap {
                    tap(&event);
                }
            }
            GatewayEvent::Gap(_) => self.schedule_recovery(false),
            GatewayEvent::Resync => self.schedule_recovery(false),
            GatewayEvent::Refused(reason) => self.write(|s| s.gateway_refused = reason),
            GatewayEvent::Frame(frame) => {
                let voice = matches!(frame, ServerFrame::Sig(_) | ServerFrame::Err { .. });
                let event = voice.then(|| GatewayEvent::Frame(frame.clone()));
                self.on_frame(frame);
                if let Some(event) = event {
                    self.voice_hook(&event);
                }
            }
        }
    }

    fn on_frame(&self, frame: ServerFrame) {
        match frame {
            ServerFrame::Sig(sig) => {
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_millis() as i64);
                let at = self.http().server_now();
                self.write(|s| {
                    s.live_event(&sig, now_ms, at);
                    s.apply_sig(&sig)
                })
            }
            ServerFrame::Presence { s, u, st, snap } => {
                self.write(|state| state.apply_presence(s, u, st, snap.as_deref()));
            }
            ServerFrame::Typing { c, u, on, .. } => {
                let now = Instant::now();
                self.write(|s| s.apply_typing(c, u, on, now));
            }
            ServerFrame::Err { e, s, c } => {
                self.write(|st| st.gateway_error = Some((e.clone(), s, c)));
                match (e.as_str(), s, c) {
                    ("kicked", Some(server), _) => self.removed_from(server, false),
                    ("banned", Some(server), _) => self.removed_from(server, true),
                    ("server_blocked", Some(server), _) => self.server_blocked(server),
                    ("not_found", Some(server), Some(channel)) => {
                        if let Some(gateway) = self.gateway() {
                            gateway.forget_topic(Topic::channel(server, channel));
                        }
                    }
                    ("not_found", Some(_), None) => self.schedule_recovery(false),
                    _ => {}
                }
            }
            ServerFrame::NotificationSettings => {
                let this = self.clone();
                self.spawn(async move { this.reload_notification_settings().await });
            }
            ServerFrame::Read { d } => {
                let recount = self.write(|s| s.apply_read_sync(&d));
                if recount {
                    self.refresh_unread();
                }
            }
            ServerFrame::Dm { .. } => self.schedule_recovery(false),
            ServerFrame::Locale { l } => self.write(|s| record_locale(s, l)),
            ServerFrame::Joined { s } => {
                let this = self.clone();
                self.spawn(async move { this.joined(s).await });
            }
            ServerFrame::JoinRequests { s, n } => self.write(|st| st.set_join_requests(s, n)),
            _ => {}
        }
    }

    /// Kicked or banned (`useRealtime.ts`): leave voice there, toast, drop
    /// the server.
    fn removed_from(&self, server: Uuid, banned: bool) {
        let in_voice = self.with_state(|s| s.voice_self.server == Some(server));
        if in_voice {
            self.voice_leave();
        }
        self.write(|s| s.forget_server(server));
        self.sync_topics();
        self.notify(if banned {
            Notice::Banned {
                server,
                text: gelabber_i18n::shell::removed_banned().into(),
            }
        } else {
            Notice::Kicked {
                server,
                text: gelabber_i18n::shell::removed_kicked().into(),
            }
        });
    }

    /// An operator blocked the server (`applyServerBlocked` in
    /// useRealtime.ts): leave voice there; the first time, mark it in the
    /// list, drop the rest of it and toast. Later subscribes meet the same
    /// error and only drop voice again.
    fn server_blocked(&self, server: Uuid) {
        let in_voice = self.with_state(|s| s.voice_self.server == Some(server));
        if in_voice {
            self.voice_leave();
        }
        if !self.write(|s| s.mark_server_blocked(server)) {
            return;
        }
        self.sync_topics();
        self.notify(Notice::Blocked {
            server,
            text: gelabber_i18n::auth::error_server_blocked().into(),
        });
    }

    /// `jn` (`applyDirectoryFrame`): the server list is read again, then a
    /// toast names the new server. Nothing when the list does not hold it
    /// or the account changed meanwhile.
    async fn joined(&self, server: Uuid) {
        let epoch = self.epoch();
        self.reload_servers().await;
        let name = self.with_state(|s| {
            (s.epoch() == epoch)
                .then(|| s.server(server).map(|v| v.name.clone()))
                .flatten()
        });
        if let Some(name) = name {
            self.notify(Notice::Joined {
                server,
                text: gelabber_i18n::discovery::toasts_joined(&name),
            });
        }
    }

    /// Coalesces gap/ready/resync/dm into REST rounds (`useRealtime.ts`):
    /// lists, details and the visible history again; other histories are
    /// marked stale and refetch when opened.
    fn schedule_recovery(&self, spread: bool) {
        {
            let mut r = self.inner.recovery.lock();
            if r.running {
                r.rerun = true;
                return;
            }
            r.running = true;
            // A retry still waiting is replaced by this round.
            r.retry += 1;
        }
        self.recovery_rounds(spread, 0);
    }

    /// Runs rounds until one succeeds or nothing asks for another. A failed
    /// one is retried after a wait that neither counts as `busy` nor keeps
    /// the client alive, at most [`RECOVERY_RETRIES`] times in a row.
    fn recovery_rounds(&self, spread: bool, failures: u32) {
        let this = self.clone();
        self.spawn(async move {
            if spread {
                tokio::time::sleep(READY_SPREAD.mul_f64(crate::gateway::jitter())).await;
            }
            let mut failures = failures;
            let token = loop {
                let gaps = this
                    .gateway()
                    .map(|g| g.gap_recoveries())
                    .unwrap_or_default();
                let ok = this.recover().await;
                if ok && let Some(gateway) = this.gateway() {
                    for gap in gaps {
                        gateway.complete_gap(gap);
                    }
                }
                let mut r = this.inner.recovery.lock();
                if r.rerun {
                    r.rerun = false;
                    failures = 0;
                    continue;
                }
                r.running = false;
                // Something that keeps failing (one server's detail) is not
                // fetched with everything else for the whole session.
                if ok || this.gateway().is_none() || failures >= RECOVERY_RETRIES {
                    return;
                }
                r.retry += 1;
                break r.retry;
            };
            let wait = RECOVERY_RETRY
                .saturating_mul(1 << failures.min(5))
                .min(RECOVERY_RETRY_MAX);
            let weak = this.weak();
            this.runtime().spawn(async move {
                tokio::time::sleep(wait).await;
                let Some(client) = Client::from_weak(&weak) else {
                    return;
                };
                {
                    let mut r = client.inner.recovery.lock();
                    // A round started meanwhile, or the retry was replaced.
                    if r.running || r.retry != token {
                        return;
                    }
                    r.running = true;
                }
                client.recovery_rounds(false, failures + 1);
            });
        });
    }

    /// Every round refetches the server and DM lists, so a resync's
    /// rediscovery needs nothing extra.
    async fn recover(&self) -> bool {
        if self.with_state(|s| s.me().is_none()) {
            return true;
        }
        let http = self.http().clone();
        let epoch = self.epoch();
        let (servers, dms, unread) = tokio::join!(http.servers(), http.dms(), http.unread());
        let mut ok = servers.is_ok() && dms.is_ok() && unread.is_ok();
        let current = self.write_in(epoch, |s| {
            if let Ok(list) = servers {
                let gone: Vec<Uuid> = s
                    .server_details
                    .keys()
                    .filter(|id| !list.iter().any(|v| v.id == **id))
                    .copied()
                    .collect();
                for id in gone {
                    s.forget_server(id);
                }
                s.servers = list;
                s.servers_fetched = Some(Instant::now());
            }
            if let Ok(list) = dms {
                s.dms = list;
            }
            if let Ok(rows) = unread {
                s.replace_read_states(rows);
            }
        });
        if current.is_none() {
            return true;
        }
        let loaded: Vec<Uuid> = self.with_state(|s| s.server_details.keys().copied().collect());
        let results = join_all(loaded.into_iter().map(|id| self.reload_server(id))).await;
        ok &= results.iter().all(|r| {
            r.as_ref()
                .map_or_else(|e| e.is_gone() || e.code() == "server_blocked", |_| true)
        });
        let visible = self.with_state(|s| s.view.channel);
        self.write(|s| {
            for (id, history) in s.channels.iter_mut() {
                if Some(*id) != visible && history.loaded {
                    history.stale = true;
                }
            }
        });
        if let Some(channel) = visible
            && self.with_state(|s| s.channels.get(&channel).is_some_and(|h| h.loaded))
        {
            match self
                .http()
                .messages(channel, &HistoryQuery::latest(PAGE_SIZE))
                .await
            {
                Ok(page) => {
                    self.write_in(epoch, |s| s.channel_mut(channel).merge_latest(page));
                }
                Err(e) if e.is_gone() => {}
                Err(_) => {
                    // Opening it fetches once more, then shows the error row.
                    self.write_in(epoch, |s| s.channel_mut(channel).stale = true);
                    ok = false;
                }
            }
        }
        self.sync_topics();
        ok
    }

    /// Unread counts follow events after a short debounce.
    fn schedule_unread_refresh(&self) {
        let generation = self
            .inner
            .unread_generation
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel)
            + 1;
        let first = {
            let mut local = self.inner.local.lock();
            *local.unread_since.get_or_insert(Instant::now())
        };
        let weak = self.weak();
        self.runtime().spawn(async move {
            let wait = UNREAD_DEBOUNCE.min(UNREAD_MAX_WAIT.saturating_sub(first.elapsed()));
            tokio::time::sleep(wait).await;
            let Some(client) = Client::from_weak(&weak) else {
                return;
            };
            let fire = {
                let mut local = client.inner.local.lock();
                let latest = client.inner.unread_generation.load(Ordering::Acquire);
                // Past the max wait only the round's first waker fetches;
                // the others of that burst would repeat the same request.
                let overdue =
                    first.elapsed() >= UNREAD_MAX_WAIT && local.unread_since == Some(first);
                let fire = latest == generation || overdue;
                if fire {
                    local.unread_since = None;
                }
                fire
            };
            if fire {
                client.reload_unread().await;
            }
        });
    }

    /// `GET /public-pages.json` for the sign-in pages' legal links into
    /// `State::public_pages`. Op key `public-pages`. Asked once, as the web
    /// does (`staleTime: Infinity`, no retry): a failure publishes nothing.
    /// Not tied to the account: an answer that comes after a sign-in or
    /// sign-out still counts.
    pub fn load_public_pages(&self) {
        let this = self.clone();
        self.write(|s| s.ops.insert("public-pages".into(), OpState::Pending));
        self.spawn(async move {
            let pages = this.http().public_pages().await.unwrap_or_default();
            this.write(|s| {
                s.ops.insert("public-pages".into(), OpState::Done);
                s.public_pages = Some(pages);
            });
        });
    }
}

/// The signed-in user's language, as the server confirmed or announced it.
pub(crate) fn record_locale(state: &mut State, locale: String) {
    if let Session::Authenticated(user) = &mut state.session {
        user.locale = Some(locale);
    }
}

/// The op failure of a deletion that `409 owns_servers` refused.
fn owns_servers() -> ApiError {
    ApiError::Api {
        status: 409,
        code: "owns_servers".into(),
        message: crate::http::error_message("owns_servers").into(),
        fields: serde_json::Value::Null,
        retry_after: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Answers every request with a body nothing decodes from.
    fn broken_server() -> (String, Arc<parking_lot::Mutex<Vec<String>>>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let log = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let log = log.clone();
                std::thread::spawn(move || {
                    let mut buf = [0u8; 4096];
                    let n = stream.read(&mut buf).unwrap_or(0);
                    let head = String::from_utf8_lossy(&buf[..n]);
                    log.lock()
                        .push(head.lines().next().unwrap_or("").to_owned());
                    let _ = stream.write_all(
                        b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    );
                });
            }
        });
        (format!("http://{addr}"), seen)
    }

    fn signed_in(client: &Client, dm: Uuid) {
        client.write(|s| {
            s.session = Session::Authenticated(Box::new(
                serde_json::from_value(serde_json::json!({
                    "id": Uuid::from_u128(1), "email": "ich@example.test", "name": "Ich",
                    "created_at": "2026-10-01T00:00:00Z",
                }))
                .unwrap(),
            ));
            s.view = View {
                server: None,
                channel: Some(dm),
            };
            let h = s.channel_mut(dm);
            h.loaded = true;
        });
    }

    /// The visible conversation's refetch failed: it is stale, so opening
    /// it fetches again instead of keeping the hole.
    #[test]
    fn a_failed_recovery_leaves_the_visible_history_stale() {
        let (url, _) = broken_server();
        let client = Client::new(&url).unwrap();
        let dm = Uuid::from_u128(5);
        signed_in(&client, dm);
        let ok = client.block_on(client.recover());
        assert!(!ok);
        assert!(client.with_state(|s| s.channels[&dm].stale));
    }

    /// Leaving again after a failed leave is pending from the call on: the
    /// old failure is not read as the new request's end.
    #[test]
    fn a_second_leave_is_pending_at_once() {
        let (url, _seen) = broken_server();
        let client = Client::new(&url).unwrap();
        let id = Uuid::from_u128(5);
        let key = format!("leave:{id}");
        client.leave_server(id);
        let start = std::time::Instant::now();
        while !client.with_state(|s| matches!(s.op(&key), Some(OpState::Failed(_)))) {
            assert!(start.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(10));
        }
        client.leave_server(id);
        assert!(client.with_state(|s| matches!(s.op(&key), Some(OpState::Pending))));
    }

    /// A level this client does not know is never written back: muting a
    /// conversation that has one fails instead of saving "all".
    #[test]
    fn an_unknown_level_is_not_saved_back() {
        let (url, seen) = broken_server();
        let client = Client::new(&url).unwrap();
        client.set_notifications(
            SettingTarget::Channel(Uuid::from_u128(5)),
            NotificationSettingBody {
                level: Some(NotificationLevel::Unknown),
                muted: true,
                mute_minutes: None,
            },
        );
        let start = std::time::Instant::now();
        while !client.with_state(|s| matches!(s.op("notifications"), Some(OpState::Failed(_)))) {
            assert!(start.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            seen.lock().iter().all(|l| !l.starts_with("PUT")),
            "{:?}",
            seen.lock()
        );
    }

    /// Waiting to retry a failed round is not `busy` (the sidebar's retry
    /// button would read "Wird geladen…") and does not keep the client alive.
    #[test]
    fn a_recovery_retry_waits_idle_and_lets_the_client_go() {
        let (url, seen) = broken_server();
        let client = Client::new(&url).unwrap();
        signed_in(&client, Uuid::from_u128(5));
        client.ensure_gateway();
        client.schedule_recovery(false);
        let start = std::time::Instant::now();
        while seen.lock().is_empty() || client.busy() {
            assert!(start.elapsed() < Duration::from_secs(2), "still busy");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(client.inner.recovery.lock().retry > 1, "no retry waiting");
        let weak = client.weak();
        drop(client);
        // The finished round's task lets go a moment after `busy` drops;
        // the retry waits a whole second.
        let start = std::time::Instant::now();
        while weak.upgrade().is_some() {
            assert!(start.elapsed() < Duration::from_millis(500), "still alive");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// `err server_blocked` (`applyServerBlocked`): the server stays listed,
    /// marked, its view goes, and a toast says why; a repeat says nothing.
    #[test]
    fn a_blocked_server_is_marked_with_one_notice() {
        let (url, _) = broken_server();
        let client = Client::new(&url).unwrap();
        let server = Uuid::from_u128(7);
        signed_in(&client, Uuid::from_u128(5));
        client.write(|s| {
            s.servers = vec![
                serde_json::from_value(serde_json::json!({
                    "id": server, "name": "Wohnzimmer", "owner_id": Uuid::from_u128(9),
                    "icon_url": null, "created_at": "2026-10-01T00:00:00Z", "role": "member",
                    "permissions": [], "member_permissions": [],
                }))
                .unwrap(),
            ];
            s.view = View {
                server: Some(server),
                channel: None,
            };
        });
        let blocked = || {
            client.on_gateway(GatewayEvent::Frame(ServerFrame::Err {
                e: "server_blocked".into(),
                s: Some(server),
                c: None,
            }))
        };
        blocked();
        assert!(client.with_state(|s| s.server(server).is_some_and(|v| v.blocked)));
        assert_eq!(client.with_state(|s| s.view), View::default());
        let notices = client.take_notices();
        assert_eq!(
            notices,
            [Notice::Blocked {
                server,
                text: "Dieser Server wurde vom Betreiber gesperrt.".into(),
            }]
        );
        blocked();
        assert!(client.take_notices().is_empty());
        assert_eq!(client.with_state(|s| s.servers.len()), 1);
    }

    /// A failed round is tried again while the socket is wanted.
    #[test]
    fn a_failed_recovery_round_is_retried() {
        let (url, seen) = broken_server();
        let client = Client::new(&url).unwrap();
        signed_in(&client, Uuid::from_u128(5));
        client.ensure_gateway();
        client.schedule_recovery(false);
        std::thread::sleep(Duration::from_millis(2_500));
        client.stop_gateway();
        let rounds = seen
            .lock()
            .iter()
            .filter(|l| l.starts_with("GET /api/servers "))
            .count();
        assert!(rounds >= 2, "{rounds} rounds");
    }
    /// Answers `GET /api/servers` with `list`, everything else with 500.
    fn server_list(list: serde_json::Value) -> String {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let body = list.to_string();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let body = body.clone();
                std::thread::spawn(move || {
                    let mut buf = [0u8; 4096];
                    let n = stream.read(&mut buf).unwrap_or(0);
                    let head = String::from_utf8_lossy(&buf[..n]);
                    let answer = if head.starts_with("GET /api/servers ") {
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        )
                    } else {
                        "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned()
                    };
                    let _ = stream.write_all(answer.as_bytes());
                });
            }
        });
        format!("http://{addr}")
    }

    fn listed(id: Uuid, name: &str) -> serde_json::Value {
        serde_json::json!({
            "id": id, "name": name, "owner_id": Uuid::from_u128(9),
            "icon_url": null, "created_at": "2026-10-01T00:00:00Z", "role": "member",
            "permissions": [], "member_permissions": [],
        })
    }

    /// `jn` (`applyDirectoryFrame`): the list is read again and a toast
    /// names the new server.
    #[test]
    fn an_accepted_request_reads_the_list_and_names_the_server() {
        let keller = Uuid::from_u128(7);
        let url = server_list(serde_json::json!([
            listed(Uuid::from_u128(6), "Wohnzimmer"),
            listed(keller, "Modellbau-Keller"),
        ]));
        let client = Client::new(&url).unwrap();
        signed_in(&client, Uuid::from_u128(5));
        client.on_gateway(GatewayEvent::Frame(ServerFrame::Joined { s: keller }));
        let start = std::time::Instant::now();
        let notices = loop {
            let notices = client.take_notices();
            if !notices.is_empty() {
                break notices;
            }
            assert!(start.elapsed() < Duration::from_secs(5), "no notice");
            std::thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(
            notices,
            [Notice::Joined {
                server: keller,
                text: "Du bist jetzt Mitglied von Modellbau-Keller.".into(),
            }]
        );
        assert!(client.with_state(|s| s.server(keller).is_some()));
    }

    /// A list without the server (or a failed read) says nothing.
    #[test]
    fn a_join_the_list_does_not_hold_says_nothing() {
        let url = server_list(serde_json::json!([listed(
            Uuid::from_u128(6),
            "Wohnzimmer"
        )]));
        let client = Client::new(&url).unwrap();
        signed_in(&client, Uuid::from_u128(5));
        client.on_gateway(GatewayEvent::Frame(ServerFrame::Joined {
            s: Uuid::from_u128(7),
        }));
        let start = std::time::Instant::now();
        while !client.with_state(|s| s.servers_loaded) {
            assert!(start.elapsed() < Duration::from_secs(5), "list not read");
            std::thread::sleep(Duration::from_millis(10));
        }
        std::thread::sleep(Duration::from_millis(50));
        assert!(client.take_notices().is_empty());
    }

    /// `jq`: the count is set and every frame counts as a change; no toast.
    #[test]
    fn a_queue_frame_sets_the_count() {
        let (url, seen) = broken_server();
        let client = Client::new(&url).unwrap();
        signed_in(&client, Uuid::from_u128(5));
        let (home, other) = (Uuid::from_u128(7), Uuid::from_u128(8));
        let queue = |n| {
            client.on_gateway(GatewayEvent::Frame(ServerFrame::JoinRequests {
                s: home,
                n,
            }));
        };
        queue(3);
        queue(3);
        let count = client.with_state(|s| s.join_requests.get(&home).copied());
        assert_eq!(
            count,
            Some(crate::store::JoinRequestCount {
                pending: 3,
                changes: 2
            })
        );
        assert!(client.with_state(|s| !s.join_requests.contains_key(&other)));
        assert!(client.take_notices().is_empty());
        assert!(seen.lock().is_empty(), "{:?}", seen.lock());
        // A server that goes takes its count along.
        client.write(|s| s.forget_server(home));
        assert!(client.with_state(|s| s.join_requests.is_empty()));
    }

    /// A fake API: the session route hands out a CSRF token, `answer`
    /// gives status and body for the rest by request line, which is kept.
    fn routes(
        answer: fn(&str) -> (&'static str, String),
    ) -> (String, Arc<parking_lot::Mutex<Vec<String>>>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let log = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let log = log.clone();
                std::thread::spawn(move || {
                    let mut buf = [0u8; 8192];
                    let n = stream.read(&mut buf).unwrap_or(0);
                    let head = String::from_utf8_lossy(&buf[..n]);
                    let line = head.lines().next().unwrap_or("").to_owned();
                    let (status, body) = if line.starts_with("GET /api/auth/session ") {
                        ("200 OK", r#"{"user":null,"csrf_token":"t"}"#.to_owned())
                    } else {
                        log.lock().push(line.clone());
                        answer(&line)
                    };
                    let _ = stream.write_all(
                        format!(
                            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        )
                        .as_bytes(),
                    );
                });
            }
        });
        (format!("http://{addr}"), seen)
    }

    fn wait(what: &str, done: impl Fn() -> bool) {
        let start = std::time::Instant::now();
        while !done() {
            assert!(start.elapsed() < Duration::from_secs(5), "{what}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// `k: "o"` (`isOwnerChange`): the list and that server's detail are
    /// read again, since the caller's role may have moved too.
    #[test]
    fn an_ownership_move_reads_the_server_again() {
        let (url, seen) = broken_server();
        let client = Client::new(&url).unwrap();
        signed_in(&client, Uuid::from_u128(5));
        let home = Uuid::from_u128(7);
        let event = |k: &str| EventFrame {
            t: EventKind::E,
            s: home,
            c: None,
            n: 1,
            ep: None,
            i: None,
            d: Some(serde_json::json!({"k": k, "owner_id": Uuid::from_u128(1)})),
            r: None,
        };
        client.on_gateway(GatewayEvent::Event(event("o")));
        let detail = format!("GET /api/servers/{home} HTTP/1.1");
        wait("no refetch", || {
            let seen = seen.lock();
            seen.iter().any(|l| l == "GET /api/servers HTTP/1.1") && seen.contains(&detail)
        });
        // Another server-level edit reads nothing here.
        seen.lock().clear();
        client.on_gateway(GatewayEvent::Event(EventFrame { n: 2, ..event("z") }));
        std::thread::sleep(Duration::from_millis(100));
        assert!(
            !seen
                .lock()
                .iter()
                .any(|l| l.starts_with("GET /api/servers")),
            "{:?}",
            seen.lock()
        );
    }

    /// A deletion that went through signs this client out and asks the
    /// login page for its notice; a 401 meanwhile is no expired session.
    #[test]
    fn a_deleted_account_signs_out_with_its_notice() {
        let (url, seen) = routes(|line| {
            if line.starts_with("DELETE /api/me ") {
                ("202 Accepted", r#"{"csrf_token":"n"}"#.to_owned())
            } else {
                ("500 Internal Server Error", String::new())
            }
        });
        let client = Client::new(&url).unwrap();
        signed_in(&client, Uuid::from_u128(5));
        client.write(|s| s.ops.insert("delete_account".into(), OpState::Pending));
        client.signed_out_by_server();
        std::thread::sleep(Duration::from_millis(50));
        assert!(client.with_state(|s| s.me().is_some() && !s.session_expired));
        client.delete_account(DeleteConfirmation::Password("pw".into()));
        wait("not deleted", || client.with_state(|s| s.account_deleted));
        client.with_state(|s| {
            assert!(matches!(s.session, Session::Anonymous));
            assert!(!s.session_expired);
            assert!(matches!(s.op("delete_account"), Some(OpState::Done)));
        });
        assert!(seen.lock().contains(&"DELETE /api/me HTTP/1.1".to_owned()));
    }

    /// `409 owns_servers` keeps the session and names the servers.
    #[test]
    fn a_deletion_refused_for_owned_servers_names_them() {
        let (url, _) = routes(|_| {
            (
                "409 Conflict",
                r#"{"error":"owns_servers","message":"x","servers":[{"id":"00000000-0000-0000-0000-000000000007","name":"Wohnzimmer","member_count":5}]}"#
                    .to_owned(),
            )
        });
        let client = Client::new(&url).unwrap();
        signed_in(&client, Uuid::from_u128(5));
        client.delete_account(DeleteConfirmation::Password("pw".into()));
        wait("not refused", || {
            client.with_state(|s| matches!(s.op("delete_account"), Some(OpState::Failed(_))))
        });
        client.with_state(|s| {
            let Some(OpState::Failed(error)) = s.op("delete_account") else {
                unreachable!()
            };
            assert_eq!(error.code(), "owns_servers");
            let owned = s.owned_servers.as_ref().unwrap();
            assert_eq!(owned[0].name, "Wohnzimmer");
            assert_eq!(owned[0].member_count, Some(5));
            assert!(s.me().is_some() && !s.account_deleted);
        });
    }

    /// "Abmelden" on another device: a row that is gone already only
    /// refreshes the list. The export lands in the state.
    #[test]
    fn devices_and_the_export_land_in_the_state() {
        let (url, seen) = routes(|line| {
            if line.starts_with("DELETE /api/auth/sessions/") {
                (
                    "404 Not Found",
                    r#"{"error":"not_found","message":"x"}"#.to_owned(),
                )
            } else if line.starts_with("GET /api/auth/sessions ") {
                (
                    "200 OK",
                    r#"{"sessions":[{"id":"00000000-0000-0000-0000-000000000001","client":"desktop","os":"linux","created_at":"2026-11-01T18:00:00Z","last_used_at":"2026-11-10T19:02:00Z","current":true}]}"#
                        .to_owned(),
                )
            } else if line.starts_with("POST /api/me/export ") {
                (
                    "202 Accepted",
                    r#"{"export":{"state":"pending","requested_at":"2026-11-11T10:00:00Z","finished_at":null,"expires_at":null,"size_bytes":null},"mail":false}"#
                        .to_owned(),
                )
            } else {
                ("500 Internal Server Error", String::new())
            }
        });
        let client = Client::new(&url).unwrap();
        signed_in(&client, Uuid::from_u128(5));
        let other = Uuid::from_u128(2);
        client.revoke_device(other);
        let key = format!("revoke_device:{other}");
        wait("not revoked", || {
            client.with_state(|s| matches!(s.op(&key), Some(OpState::Done)))
        });
        client.with_state(|s| assert_eq!(s.devices.as_ref().unwrap().len(), 1));
        client.request_export();
        wait("no export", || client.with_state(|s| s.export.is_some()));
        assert!(client.with_state(|s| s.export.as_ref().unwrap().building()));
        assert_eq!(
            seen.lock().clone(),
            [
                format!("DELETE /api/auth/sessions/{other} HTTP/1.1"),
                "GET /api/auth/sessions HTTP/1.1".to_owned(),
                "POST /api/me/export HTTP/1.1".to_owned(),
            ]
        );
    }

    /// "Beitreten" (`useJoinListing`): the server is in the list once the
    /// op is done, and the list is read again after it.
    #[test]
    fn a_directory_join_puts_the_server_in_the_list() {
        use std::io::{Read, Write};
        let filmclub = Uuid::from_u128(7);
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let seen: Arc<parking_lot::Mutex<Vec<String>>> = Arc::default();
        let log = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let log = log.clone();
                std::thread::spawn(move || {
                    let mut buf = [0u8; 4096];
                    let n = stream.read(&mut buf).unwrap_or(0);
                    let head = String::from_utf8_lossy(&buf[..n]).into_owned();
                    let line = head.lines().next().unwrap_or("").to_owned();
                    log.lock().push(line.clone());
                    let body = if line.starts_with("POST /api/directory/listings/") {
                        listed(filmclub, "Filmclub").to_string()
                    } else if line.starts_with("GET /api/servers ") {
                        serde_json::json!([listed(filmclub, "Filmclub")]).to_string()
                    } else {
                        "[]".to_owned()
                    };
                    let _ = stream.write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        )
                        .as_bytes(),
                    );
                });
            }
        });
        let client = Client::new(&format!("http://{addr}")).unwrap();
        signed_in(&client, Uuid::from_u128(5));
        client.join_listing(filmclub);
        let key = format!("join_listing:{filmclub}");
        let start = std::time::Instant::now();
        while !client.with_state(|s| matches!(s.op(&key), Some(OpState::Done))) {
            assert!(start.elapsed() < Duration::from_secs(5), "not joined");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(client.with_state(|s| s.server(filmclub).is_some()));
        while !seen
            .lock()
            .iter()
            .any(|l| l.starts_with("GET /api/servers "))
        {
            assert!(start.elapsed() < Duration::from_secs(5), "list not read");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            seen.lock()
                .iter()
                .any(|l| l == &format!("POST /api/directory/listings/{filmclub}/join HTTP/1.1"))
        );
    }
}
