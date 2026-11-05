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
    Notice, OpState, OutgoingFile, PAGE_SIZE, PendingMessage, Session, State, View, VoiceSelf,
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
/// Unread counts are refetched this long after the last event (web 750 ms,
/// at most 5 s while events keep coming).
const UNREAD_DEBOUNCE: Duration = Duration::from_millis(750);
const UNREAD_MAX_WAIT: Duration = Duration::from_secs(5);
const TICK: Duration = Duration::from_millis(400);

/// Which notification setting to change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingTarget {
    Server(Uuid),
    /// A text channel or a DM.
    Channel(Uuid),
}

impl Client {
    // ------------------------------------------------------------ helpers

    /// Runs `work` as the named operation: `State::ops[op]` is `Pending`,
    /// then `Done` or `Failed`; `apply` gets the result on success.
    pub fn run<T, F, A>(&self, op: impl Into<String>, work: F, apply: A)
    where
        T: Send + 'static,
        F: Future<Output = Result<T, ApiError>> + Send + 'static,
        A: FnOnce(&mut State, T) + Send + 'static,
    {
        let this = self.clone();
        let op = op.into();
        self.spawn(async move {
            let epoch = this.epoch();
            if let Some(value) = this.op(&op, work).await {
                this.write_in(epoch, |s| apply(s, value));
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
                self.write_in(epoch, |s| {
                    s.ops.insert(key.to_owned(), OpState::Failed(error))
                });
                None
            }
        }
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
    /// workspace.
    async fn adopt_session(&self, user: Option<User>) {
        match user {
            Some(user) => {
                let same = self.with_state(|s| s.me().map(|m| m.id) == Some(user.id));
                if !same {
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
    /// told. Op key `logout` reports the server's answer.
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
            let work = async move {
                if everywhere {
                    http.logout_all().await
                } else {
                    http.logout().await
                }
            };
            this.op("logout", work).await;
        });
    }

    /// A `401 unauthenticated` anywhere: the session ended on the server.
    pub(crate) fn signed_out_by_server(&self) {
        let this = self.clone();
        self.runtime().spawn(async move {
            if this.with_state(|s| matches!(s.session, Session::Authenticated(_))) {
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
                    s.read.clear();
                    s.apply_read_states(rows);
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
                });
                self.sync_topics();
                Ok(())
            }
            Err(e) if e.is_gone() => {
                self.write_in(epoch, |s| s.forget_server(id));
                self.sync_topics();
                Err(e)
            }
            Err(e) => Err(e),
        }
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
                s.read.clear();
                s.apply_read_states(rows);
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
            if !this.with_state(|s| s.server_details.contains_key(&id)) {
                let work = this.reload_server(id);
                this.op(&key, work).await;
            }
            this.inner.opening.lock().remove(&view);
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

    /// Opens a text channel or DM: history (first page) once, then marks it
    /// read like the web. The server is looked up; for a route with both
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
            None => {
                // Unknown yet: a DM not in the list, or a server not loaded.
                let this = self.clone();
                self.spawn(async move {
                    let epoch = this.epoch();
                    match this.http().dm(channel).await {
                        Ok(dm) => {
                            let current = this.write_in(epoch, |s| {
                                if s.dm(dm.id).is_none() {
                                    s.dms.insert(0, dm);
                                }
                            });
                            if current.is_some() {
                                this.open_conversation(None, channel);
                            }
                        }
                        Err(error) => {
                            this.write_in(epoch, |s| {
                                s.channel_mut(channel).error = Some(error);
                            });
                        }
                    }
                });
            }
        }
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
            let changed = this.with_state(|s| s.view != view);
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
            if changed || fetch {
                this.mark_read_now(channel).await;
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
        let pending = PendingMessage {
            nonce: Uuid::new_v4(),
            channel_id: channel,
            content: text.to_owned(),
            reply_to,
            gif,
            files,
            created_at: self.http().server_now(),
            failed: None,
        };
        self.stop_typing();
        let this = self.clone();
        self.spawn(async move {
            this.write(|s| s.channel_mut(channel).pending.push(pending.clone()));
            this.deliver(pending).await;
        });
    }

    async fn deliver(&self, pending: PendingMessage) {
        let epoch = self.epoch();
        let http = self.http().clone();
        let work = async {
            let mut ids = Vec::with_capacity(pending.files.len());
            for file in &pending.files {
                let attachment = http
                    .upload_attachment(
                        pending.channel_id,
                        &file.filename,
                        &file.content_type,
                        file.data.as_ref().clone(),
                    )
                    .await?;
                ids.push(attachment.id);
            }
            let body = NewMessage {
                content: pending.content.clone(),
                attachment_ids: pending.gif.is_none().then_some(ids),
                gif: pending.gif.clone(),
                reply_to: pending.reply_to,
                nonce: Some(pending.nonce),
            };
            http.create_message(pending.channel_id, &body).await
        };
        let result = work.await;
        self.write_in(epoch, |s| match result {
            Ok(message) => s.apply_message(message),
            Err(error) => {
                if let Some(row) = s
                    .channel_mut(pending.channel_id)
                    .pending
                    .iter_mut()
                    .find(|p| p.nonce == pending.nonce)
                {
                    row.failed = Some(error);
                }
            }
        });
    }

    /// Sends a failed pending row again with the same nonce.
    pub fn retry_send(&self, channel: Uuid, nonce: Uuid) {
        let this = self.clone();
        self.spawn(async move {
            let pending = this.write(|s| {
                let row = s
                    .channel_mut(channel)
                    .pending
                    .iter_mut()
                    .find(|p| p.nonce == nonce && p.failed.is_some())?;
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

    /// Op key `edit:<id>`.
    pub fn edit_message(&self, id: Uuid, content: &str) {
        let http = self.http().clone();
        let content = content.to_owned();
        self.run(
            format!("edit:{id}"),
            async move { http.edit_message(id, &content).await },
            |s, m| s.apply_message(m),
        );
    }

    /// Op key `delete:<id>`.
    pub fn delete_message(&self, id: Uuid) {
        let channel = self.with_state(|s| s.find_message(id).map(|m| m.channel_id));
        let http = self.http().clone();
        self.run(
            format!("delete:{id}"),
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
        self.run(
            format!("react:{id}"),
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
        if let Ok(row) = self.http().mark_read(channel, None).await {
            self.write_in(epoch, |s| s.apply_read_states(vec![row]));
        }
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

    /// The window gained or lost focus/visibility.
    pub fn set_focused(&self, focused: bool) {
        self.inner.local.lock().focused = focused;
        self.update_front(focused);
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
                    });
                }
                if current.deafened != deafened {
                    this.send_frame(ClientFrame::Sig {
                        t: SigKind::D,
                        s,
                        c,
                        k: None,
                        on: Some(deafened),
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
                    });
                }
                this.send_frame(ClientFrame::Sig {
                    t: SigKind::D,
                    s,
                    c,
                    k: None,
                    on: Some(deafened),
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
            });
        });
    }

    // -------------------------------------------------- servers & channels

    /// Op key `create_server`; the new server joins the list.
    pub fn create_server(&self, name: &str) {
        let http = self.http().clone();
        let name = name.to_owned();
        let this = self.clone();
        self.run(
            "create_server",
            async move {
                let detail = http.create_server(&name).await?;
                this.sync_topics();
                Ok(detail)
            },
            |s, detail| {
                if s.server(detail.server.id).is_none() {
                    s.servers.push(detail.server.clone());
                }
                s.server_details.insert(detail.server.id, detail);
            },
        );
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
        let http = self.http().clone();
        self.run(
            format!("server_update:{id}"),
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
            if this.op(&format!("leave:{id}"), work).await.is_some() {
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

    pub fn create_channel(
        &self,
        server: Uuid,
        name: &str,
        kind: ChannelKind,
        category: Option<Uuid>,
    ) {
        let http = self.http().clone();
        let name = name.to_owned();
        self.server_change("create_channel", server, async move {
            http.create_channel(server, &name, kind, category).await
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
        self.server_change(format!("channel:{channel}"), server, async move {
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
        self.server_change(format!("category:{category}"), server, async move {
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
    pub fn set_notifications(&self, target: SettingTarget, body: NotificationSettingBody) {
        let this = self.clone();
        self.spawn(async move {
            let http = this.http().clone();
            let work = async move {
                match target {
                    SettingTarget::Server(id) => http.set_server_notifications(id, &body).await,
                    SettingTarget::Channel(id) => http.set_channel_notifications(id, &body).await,
                }
            };
            if this.op("notifications", work).await.is_some() {
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
                s.read.clear();
                s.apply_read_states(rows);
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

    fn on_gateway(&self, event: GatewayEvent) {
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
                let (removed, read_now) = self.write(|s| {
                    let removed = s.apply_event(&event);
                    let me = s.me().map(|u| u.id);
                    let foreign_create = event.t == EventKind::C
                        && event.message().is_some_and(|m| Some(m.author.id) != me);
                    let read_now = foreign_create && event.c.is_some() && event.c == s.view.channel;
                    (removed, read_now)
                });
                if let Some((server, banned)) = removed {
                    self.removed_from(server, banned);
                }
                if read_now && self.inner.local.lock().in_front {
                    self.schedule_mark_read(event.c.unwrap_or_default());
                }
                self.schedule_unread_refresh();
            }
            GatewayEvent::Gap(_) => self.schedule_recovery(false),
            GatewayEvent::Resync => self.schedule_recovery(false),
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
            ServerFrame::Sig(sig) => self.write(|s| s.apply_sig(&sig)),
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
                text: "Du wurdest vom Server gesperrt.".into(),
            }
        } else {
            Notice::Kicked {
                server,
                text: "Du wurdest vom Server entfernt.".into(),
            }
        });
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
        }
        let this = self.clone();
        self.spawn(async move {
            if spread {
                tokio::time::sleep(READY_SPREAD.mul_f64(crate::gateway::jitter())).await;
            }
            loop {
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
                    continue;
                }
                r.running = false;
                break;
            }
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
            }
            if let Ok(list) = dms {
                s.dms = list;
            }
            if let Ok(rows) = unread {
                s.read.clear();
                s.apply_read_states(rows);
            }
        });
        if current.is_none() {
            return true;
        }
        let loaded: Vec<Uuid> = self.with_state(|s| s.server_details.keys().copied().collect());
        let results = join_all(loaded.into_iter().map(|id| self.reload_server(id))).await;
        ok &= results
            .iter()
            .all(|r| r.as_ref().map_or_else(|e| e.is_gone(), |_| true));
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
                Err(_) => ok = false,
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

    /// A new message in the open conversation while the app is in front.
    fn schedule_mark_read(&self, channel: Uuid) {
        let generation = self
            .inner
            .read_generation
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel)
            + 1;
        let weak = self.weak();
        self.runtime().spawn(async move {
            tokio::time::sleep(UNREAD_DEBOUNCE).await;
            let Some(client) = Client::from_weak(&weak) else {
                return;
            };
            if client
                .inner
                .read_generation
                .load(std::sync::atomic::Ordering::Acquire)
                == generation
                && client.with_state(|s| s.view.channel == Some(channel))
            {
                client.mark_read_now(channel).await;
            }
        });
    }
}
