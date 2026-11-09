//! The `/ws` gateway as web/src/ws/client.ts runs it: the session cookie on
//! the upgrade (no `Origin`, which backend/api/src/gateway/mod.rs lets
//! through), topic subscriptions with resume cursors, heartbeats both ways,
//! a receive watchdog and the web's reconnect backoff.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use parking_lot::Mutex;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use uuid::Uuid;

use crate::http::Http;
use crate::models::{ClientFrame, EventFrame, ServerFrame, Topic};

/// What the interface shows about the socket (`client.ts` `GatewayStatus`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GatewayStatus {
    #[default]
    Idle,
    /// Only the very first connect after sign-in.
    Connecting,
    Open,
    /// Every drop, at once, until the next open.
    Reconnecting,
}

/// Timers of the web client (`DEFAULT_*_MS`), matching the server's
/// `API_WS_HEARTBEAT_MS`/`API_WS_DEAD_MS` defaults; the reconnect cap is
/// higher than the web's 2 s, so a server that keeps failing is not hit
/// every 2 s by every client.
#[derive(Clone, Copy, Debug)]
pub struct GatewayConfig {
    pub heartbeat: Duration,
    pub dead: Duration,
    pub retry: Duration,
    pub max_retry: Duration,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            heartbeat: Duration::from_millis(15_000),
            dead: Duration::from_millis(30_000),
            retry: Duration::from_millis(200),
            max_retry: Duration::from_millis(10_000),
        }
    }
}

/// After a refused upgrade (403, 404, a certificate) the next try waits
/// this long, not the reconnect backoff: retrying at once will not help.
pub const REFUSED_RETRY: Duration = Duration::from_secs(60);

/// What the socket reports to its owner, in arrival order.
#[derive(Clone, Debug)]
pub enum GatewayEvent {
    Status(GatewayStatus),
    /// Opened and resubscribed: recover what may have been missed.
    Ready,
    /// A sequenced chat event, delivered once (duplicates are dropped).
    Event(EventFrame),
    /// Resume was impossible for this topic: refetch it over REST.
    Gap(Topic),
    /// The server lost its event stream: rediscover everything.
    Resync,
    /// The server or TLS turned the upgrade down (not a network error):
    /// why, until the next open (`None`). Retried only slowly meanwhile.
    Refused(Option<String>),
    /// Every other frame: `sig`, `p`, `y`, `err`, `ns`, `rd`, `dm`.
    Frame(ServerFrame),
}

pub type Handler = Arc<dyn Fn(GatewayEvent) + Send + Sync>;

/// A gap's REST recovery, completed with [`Gateway::complete_gap`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GapRecovery {
    pub topic: Topic,
    pub version: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursor {
    pub ep: Option<Uuid>,
    pub n: u64,
}

#[derive(Clone, Debug)]
struct PendingGap {
    topic: Topic,
    version: u64,
    ep: Option<Uuid>,
    cursor: Option<u64>,
    head_known: bool,
    reconciled: bool,
}

/// Resume cursors and pending gaps per topic, with the web's rules
/// (`client.ts` `onFrame`, `markGap`, `finishGap`). Pure, so it is tested
/// without a socket.
#[derive(Debug, Default)]
pub struct Cursors {
    cursors: HashMap<String, Cursor>,
    gaps: HashMap<String, PendingGap>,
    version: u64,
}

impl Cursors {
    pub fn cursor(&self, topic: &Topic) -> Option<Cursor> {
        self.cursors.get(&topic.key()).copied()
    }

    /// Subscribe frame, with the resume cursor when one is known.
    pub fn subscribe_frame(&self, topic: &Topic) -> ClientFrame {
        let cursor = self.cursor(topic);
        ClientFrame::Subscribe {
            s: topic.s,
            c: topic.c,
            n: cursor.map(|c| c.n),
            ep: cursor.and_then(|c| c.ep),
        }
    }

    fn current_ep(&self, key: &str) -> Option<Option<Uuid>> {
        self.gaps
            .get(key)
            .map(|g| g.ep)
            .or_else(|| self.cursors.get(key).map(|c| c.ep))
    }

    pub fn mark_gap(&mut self, topic: Topic, ep: Option<Uuid>, head_known: bool) {
        let key = topic.key();
        let previous = self.gaps.get(&key);
        let current = self.cursors.get(&key);
        let old_ep = previous.and_then(|g| g.ep).or(current.and_then(|c| c.ep));
        let cursor = if ep.is_some() && ep != old_ep {
            None
        } else {
            previous.and_then(|g| g.cursor).or(current.map(|c| c.n))
        };
        self.version += 1;
        self.gaps.insert(
            key,
            PendingGap {
                topic,
                version: self.version,
                ep: ep.or(old_ep),
                cursor,
                head_known,
                reconciled: false,
            },
        );
    }

    fn finish_gap(&mut self, key: &str) {
        let Some(gap) = self.gaps.get(key) else {
            return;
        };
        if !gap.reconciled || !gap.head_known {
            return;
        }
        let Some(n) = gap.cursor else {
            return;
        };
        let ep = gap.ep;
        self.cursors.insert(key.to_owned(), Cursor { ep, n });
        self.gaps.remove(key);
    }

    /// `ok`: the head after a subscribe.
    pub fn on_ok(&mut self, topic: Topic, n: u64, ep: Option<Uuid>) -> bool {
        let key = topic.key();
        let mut gap_marked = false;
        if let (Some(ep), Some(current)) = (ep, self.current_ep(&key))
            && current != Some(ep)
        {
            self.mark_gap(topic, Some(ep), false);
            gap_marked = true;
        }
        if let Some(gap) = self.gaps.get_mut(&key) {
            gap.ep = ep.or(gap.ep);
            gap.cursor = Some(gap.cursor.unwrap_or(0).max(n));
            gap.head_known = true;
            self.finish_gap(&key);
            return gap_marked;
        }
        let old = self.cursors.get(&key).copied();
        let take = match old {
            None => true,
            Some(old) => n > old.n || (ep.is_some() && old.ep.is_none()),
        };
        if take {
            self.cursors.insert(
                key,
                Cursor {
                    ep: ep.or(old.and_then(|o| o.ep)),
                    n,
                },
            );
        }
        gap_marked
    }

    /// `e`: returns (accept, gap marked). Only accepted events are applied.
    pub fn on_event(&mut self, topic: Topic, n: u64, ep: Option<Uuid>) -> (bool, bool) {
        let key = topic.key();
        let mut gap_marked = false;
        if let (Some(ep), Some(current)) = (ep, self.current_ep(&key))
            && current != Some(ep)
        {
            self.mark_gap(topic, Some(ep), true);
            gap_marked = true;
        }
        let cursor_ep = self.cursors.get(&key).and_then(|c| c.ep);
        let new_epoch = ep.is_some() && ep != cursor_ep;
        if let Some(gap) = self.gaps.get_mut(&key) {
            let (accept, cursor) = if new_epoch && gap.cursor.is_none() {
                (true, n)
            } else {
                next_cursor(gap.cursor, n)
            };
            gap.cursor = Some(cursor);
            gap.ep = ep.or(gap.ep);
            gap.head_known = true;
            self.finish_gap(&key);
            return (accept, gap_marked);
        }
        let old = self.cursors.get(&key).copied();
        let (accept, cursor) = next_cursor(old.map(|c| c.n), n);
        self.cursors.insert(
            key,
            Cursor {
                ep: ep.or(old.and_then(|o| o.ep)),
                n: cursor,
            },
        );
        (accept, gap_marked)
    }

    /// `gap`: the announced head may be below our cursor, so it is unknown.
    pub fn on_gap(&mut self, topic: Topic, ep: Option<Uuid>) {
        self.mark_gap(topic, ep, false);
        if let Some(gap) = self.gaps.get_mut(&topic.key()) {
            gap.cursor = None;
        }
    }

    pub fn gap_recoveries(&self) -> Vec<GapRecovery> {
        let mut out: Vec<GapRecovery> = self
            .gaps
            .values()
            .filter(|g| !g.reconciled)
            .map(|g| GapRecovery {
                topic: g.topic,
                version: g.version,
            })
            .collect();
        out.sort_by_key(|g| g.version);
        out
    }

    /// REST has the topic again; the gap's head becomes the cursor once known.
    pub fn complete_gap(&mut self, recovery: GapRecovery) {
        let key = recovery.topic.key();
        if let Some(gap) = self.gaps.get_mut(&key)
            && gap.version == recovery.version
        {
            gap.reconciled = true;
            self.finish_gap(&key);
        }
    }

    pub fn forget(&mut self, topic: &Topic) {
        let key = topic.key();
        self.cursors.remove(&key);
        self.gaps.remove(&key);
    }

    pub fn clear(&mut self) {
        self.cursors.clear();
        self.gaps.clear();
    }
}

/// The web's `nextCursor`: keep the highest seq, drop what was applied.
fn next_cursor(current: Option<u64>, incoming: u64) -> (bool, u64) {
    match current {
        Some(current) if incoming <= current => (false, current),
        _ => (true, incoming),
    }
}

/// Reconnect delay of attempt `k` (`client.ts` `scheduleReconnect`):
/// 0 for the first drop, then `min(max, retry·2^(k-1))` with 50–100 % jitter.
pub fn backoff(config: &GatewayConfig, attempt: u32, jitter: f64) -> Duration {
    if attempt == 0 {
        return Duration::ZERO;
    }
    let base = config
        .retry
        .saturating_mul(1u32 << (attempt - 1).min(16))
        .min(config.max_retry);
    base.mul_f64(0.5 + jitter.clamp(0.0, 1.0) * 0.5)
}

/// A uniform number in `[0, 1)` without a random crate.
pub(crate) fn jitter() -> f64 {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
    );
    (hasher.finish() >> 11) as f64 / (1u64 << 53) as f64
}

enum Command {
    SetTopics(Vec<Topic>),
    Send(ClientFrame),
    Focus(bool),
    Stop,
}

struct Shared {
    cursors: Mutex<Cursors>,
    status: Mutex<GatewayStatus>,
    desired: Mutex<BTreeMap<String, Topic>>,
}

/// Handle to the running socket task; cheap to clone. Dropping every handle
/// does not stop it, [`Gateway::stop`] does.
#[derive(Clone)]
pub struct Gateway {
    tx: mpsc::UnboundedSender<Command>,
    shared: Arc<Shared>,
}

impl Gateway {
    /// Starts connecting on `runtime`; `handler` receives every event.
    pub fn start(
        runtime: &tokio::runtime::Handle,
        http: Http,
        config: GatewayConfig,
        handler: Handler,
    ) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let shared = Arc::new(Shared {
            cursors: Mutex::new(Cursors::default()),
            status: Mutex::new(GatewayStatus::Idle),
            desired: Mutex::new(BTreeMap::new()),
        });
        let task = Task {
            http,
            config,
            handler,
            shared: shared.clone(),
            rx,
            focus: false,
            confirmed: false,
        };
        runtime.spawn(task.run());
        Self { tx, shared }
    }

    pub fn status(&self) -> GatewayStatus {
        *self.shared.status.lock()
    }

    /// Replaces the subscribed set; only the difference goes on the wire.
    pub fn set_topics(&self, topics: Vec<Topic>) {
        let _ = self.tx.send(Command::SetTopics(topics));
    }

    pub fn topics(&self) -> Vec<Topic> {
        self.shared.desired.lock().values().copied().collect()
    }

    /// Drops a topic the server refused and forgets its cursor.
    pub fn forget_topic(&self, topic: Topic) {
        let mut topics = self.topics();
        topics.retain(|t| t.key() != topic.key());
        self.shared.cursors.lock().forget(&topic);
        self.set_topics(topics);
    }

    /// Sent only while open; the web drops frames on a closed socket too.
    pub fn send(&self, frame: ClientFrame) {
        let _ = self.tx.send(Command::Send(frame));
    }

    /// The app is in front (focused and active): no Web Push meanwhile.
    pub fn set_focus(&self, on: bool) {
        let _ = self.tx.send(Command::Focus(on));
    }

    pub fn gap_recoveries(&self) -> Vec<GapRecovery> {
        self.shared.cursors.lock().gap_recoveries()
    }

    pub fn complete_gap(&self, recovery: GapRecovery) {
        self.shared.cursors.lock().complete_gap(recovery);
    }

    pub fn cursor(&self, topic: &Topic) -> Option<Cursor> {
        self.shared.cursors.lock().cursor(topic)
    }

    /// Closes the socket for good (sign-out).
    pub fn stop(&self) {
        let _ = self.tx.send(Command::Stop);
    }
}

struct Task {
    http: Http,
    config: GatewayConfig,
    handler: Handler,
    shared: Arc<Shared>,
    rx: mpsc::UnboundedReceiver<Command>,
    focus: bool,
    /// The open socket answered a subscribe (`ok`).
    confirmed: bool,
}

enum Closed {
    Dropped,
    Stopped,
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

impl Task {
    fn set_status(&self, status: GatewayStatus) {
        let changed = {
            let mut current = self.shared.status.lock();
            let changed = *current != status;
            *current = status;
            changed
        };
        if changed {
            (self.handler)(GatewayEvent::Status(status));
        }
    }

    async fn run(mut self) {
        let mut attempt: u32 = 0;
        let mut first = true;
        let mut refused = false;
        loop {
            self.set_status(if first {
                GatewayStatus::Connecting
            } else {
                GatewayStatus::Reconnecting
            });
            first = false;
            let mut slow = false;
            match self.connect().await {
                Err(Closed::Stopped) => break,
                Err(Closed::Dropped) => {}
                Ok(Ok(socket)) => {
                    if std::mem::take(&mut refused) {
                        (self.handler)(GatewayEvent::Refused(None));
                    }
                    let opened = Instant::now();
                    self.confirmed = false;
                    let closed = self.serve(socket).await;
                    // Only a socket that worked resets the backoff: one the
                    // server accepts and drops at once is a failed connect.
                    if self.confirmed || opened.elapsed() >= self.config.heartbeat {
                        attempt = 0;
                    }
                    if let Closed::Stopped = closed {
                        break;
                    }
                }
                Ok(Err(Failure::Network)) => {}
                Ok(Err(failure)) => {
                    tracing::warn!(?failure, "gateway upgrade refused");
                    if let Failure::Unauthenticated = failure {
                        // The session ended (sign-out elsewhere): as a 401 on REST.
                        self.http.report_unauthenticated();
                    }
                    refused = true;
                    (self.handler)(GatewayEvent::Refused(Some(failure.reason())));
                    slow = true;
                }
            }
            self.set_status(GatewayStatus::Reconnecting);
            let delay = if slow {
                REFUSED_RETRY.mul_f64(0.5 + jitter() * 0.5)
            } else {
                backoff(&self.config, attempt, jitter())
            };
            attempt = attempt.saturating_add(1);
            if !self.wait(delay).await {
                break;
            }
        }
        self.set_status(GatewayStatus::Idle);
    }

    /// Connects while still taking commands, so a sign-out during a slow
    /// handshake never opens a socket afterwards. `Err(Stopped)` when stopped.
    async fn connect(&mut self) -> Result<Result<Socket, Failure>, Closed> {
        let connecting = connect(self.http.clone());
        tokio::pin!(connecting);
        loop {
            tokio::select! {
                socket = &mut connecting => return Ok(socket),
                command = self.rx.recv() => match command {
                    None | Some(Command::Stop) => return Err(Closed::Stopped),
                    Some(Command::SetTopics(topics)) => {
                        self.diff_topics(topics);
                    }
                    Some(Command::Focus(on)) => self.focus = on,
                    Some(Command::Send(_)) => {}
                },
            }
        }
    }

    /// Waits out a reconnect delay while still taking topic and focus
    /// changes; false when stopped.
    async fn wait(&mut self, delay: Duration) -> bool {
        let sleep = tokio::time::sleep(delay);
        tokio::pin!(sleep);
        loop {
            tokio::select! {
                _ = &mut sleep => return true,
                command = self.rx.recv() => match command {
                    None | Some(Command::Stop) => return false,
                    Some(Command::SetTopics(topics)) => {
                        self.diff_topics(topics);
                    }
                    Some(Command::Focus(on)) => self.focus = on,
                    Some(Command::Send(_)) => {}
                },
            }
        }
    }

    /// Stores the new set and returns the frames that change the socket's.
    fn diff_topics(&self, topics: Vec<Topic>) -> Vec<ClientFrame> {
        let mut next = BTreeMap::new();
        for topic in topics {
            next.entry(topic.key()).or_insert(topic);
        }
        let mut desired = self.shared.desired.lock();
        let cursors = self.shared.cursors.lock();
        let mut frames = Vec::new();
        for (key, topic) in desired.iter() {
            if !next.contains_key(key) {
                frames.push(ClientFrame::Unsubscribe {
                    s: topic.s,
                    c: topic.c,
                });
            }
        }
        for (key, topic) in next.iter() {
            if !desired.contains_key(key) {
                frames.push(cursors.subscribe_frame(topic));
            }
        }
        *desired = next;
        frames
    }

    async fn serve(&mut self, socket: Socket) -> Closed {
        let (mut sink, mut stream) = socket.split();
        self.set_status(GatewayStatus::Open);
        let mut opening: Vec<ClientFrame> = {
            let desired = self.shared.desired.lock();
            let cursors = self.shared.cursors.lock();
            desired
                .values()
                .map(|t| cursors.subscribe_frame(t))
                .collect()
        };
        if self.focus {
            opening.push(ClientFrame::Focus { on: true });
        }
        for frame in opening {
            if send(&mut sink, &frame).await.is_err() {
                return Closed::Dropped;
            }
        }
        (self.handler)(GatewayEvent::Ready);

        let mut last_server = Instant::now();
        let mut heartbeat = tokio::time::interval_at(
            tokio::time::Instant::now() + self.config.heartbeat,
            self.config.heartbeat,
        );
        let watch = self.config.heartbeat.min(Duration::from_secs(1));
        let mut watchdog = tokio::time::interval(watch);
        loop {
            tokio::select! {
                message = stream.next() => {
                    let text = match message {
                        Some(Ok(WsMessage::Text(text))) => text,
                        Some(Ok(WsMessage::Close(_))) | None | Some(Err(_)) => return Closed::Dropped,
                        Some(Ok(_)) => {
                            last_server = Instant::now();
                            continue;
                        }
                    };
                    last_server = Instant::now();
                    let frame = match serde_json::from_str::<ServerFrame>(text.as_str()) {
                        Ok(frame) => frame,
                        Err(error) => {
                            self.on_undecodable(text.as_str(), &error);
                            continue;
                        }
                    };
                    self.confirmed |= matches!(frame, ServerFrame::Ok { .. });
                    let replies = self.on_frame(frame);
                    for reply in replies {
                        if send(&mut sink, &reply).await.is_err() {
                            return Closed::Dropped;
                        }
                    }
                }
                _ = heartbeat.tick() => {
                    if send(&mut sink, &ClientFrame::Heartbeat).await.is_err() {
                        return Closed::Dropped;
                    }
                }
                _ = watchdog.tick() => {
                    if last_server.elapsed() >= self.config.dead {
                        let _ = sink.close().await;
                        return Closed::Dropped;
                    }
                }
                command = self.rx.recv() => {
                    let frames = match command {
                        None | Some(Command::Stop) => {
                            if self.focus {
                                let _ = send(&mut sink, &ClientFrame::Focus { on: false }).await;
                            }
                            let _ = sink.close().await;
                            return Closed::Stopped;
                        }
                        Some(Command::SetTopics(topics)) => self.diff_topics(topics),
                        Some(Command::Send(frame)) => vec![frame],
                        Some(Command::Focus(on)) => {
                            if on == self.focus {
                                Vec::new()
                            } else {
                                self.focus = on;
                                vec![ClientFrame::Focus { on }]
                            }
                        }
                    };
                    for frame in frames {
                        if send(&mut sink, &frame).await.is_err() {
                            return Closed::Dropped;
                        }
                    }
                }
            }
        }
    }

    /// A frame this client cannot read: logged, and its topic recovered
    /// over REST as after a `gap`, since it may have been an event.
    fn on_undecodable(&self, text: &str, error: &serde_json::Error) {
        let topic = undecodable_topic(text);
        tracing::warn!(%error, ?topic, "gateway frame this client cannot read");
        if let Some(topic) = topic {
            self.shared.cursors.lock().on_gap(topic, None);
            (self.handler)(GatewayEvent::Gap(topic));
        }
    }

    /// Applies cursor rules and hands the frame on; returns frames to send.
    fn on_frame(&self, frame: ServerFrame) -> Vec<ClientFrame> {
        match frame {
            ServerFrame::Heartbeat => return vec![ClientFrame::Heartbeat],
            ServerFrame::Ok { s, c, n, ep } => {
                let topic = Topic { s, c };
                let gap = self.shared.cursors.lock().on_ok(topic, n, ep);
                if gap {
                    (self.handler)(GatewayEvent::Gap(topic));
                }
            }
            ServerFrame::Event(event) => {
                let topic = event.topic();
                let (accept, gap) = self
                    .shared
                    .cursors
                    .lock()
                    .on_event(topic, event.n, event.ep);
                if gap {
                    (self.handler)(GatewayEvent::Gap(topic));
                }
                if accept {
                    (self.handler)(GatewayEvent::Event(event));
                }
            }
            ServerFrame::Gap { s, ep, c } => {
                let topic = Topic { s, c };
                self.shared.cursors.lock().on_gap(topic, ep);
                (self.handler)(GatewayEvent::Gap(topic));
            }
            ServerFrame::Resync => {
                let frames: Vec<ClientFrame> = {
                    let desired = self.shared.desired.lock();
                    let mut cursors = self.shared.cursors.lock();
                    desired
                        .values()
                        .map(|topic| {
                            let ep = cursors.cursor(topic).and_then(|c| c.ep);
                            cursors.mark_gap(*topic, ep, false);
                            cursors.subscribe_frame(topic)
                        })
                        .collect()
                };
                (self.handler)(GatewayEvent::Resync);
                return frames;
            }
            ServerFrame::Unknown => {}
            other => (self.handler)(GatewayEvent::Frame(other)),
        }
        Vec::new()
    }
}

/// Why an upgrade did not open a socket.
#[derive(Debug, PartialEq, Eq)]
enum Failure {
    /// Offline, a timeout, a 5xx: the reconnect backoff.
    Network,
    /// 401: the session is over.
    Unauthenticated,
    /// Another 4xx, or the certificate was not accepted.
    Refused(String),
}

impl Failure {
    fn of(error: &tokio_tungstenite::tungstenite::Error) -> Self {
        use tokio_tungstenite::tungstenite::Error;
        match error {
            Error::Http(response) => {
                let status = response.status();
                if status == 401 {
                    Self::Unauthenticated
                } else if status.is_client_error() && status != 408 && status != 429 {
                    Self::Refused(format!("HTTP {}", status.as_u16()))
                } else {
                    Self::Network
                }
            }
            Error::Tls(error) => Self::Refused(format!("TLS: {error}")),
            Error::Io(io) => match io.get_ref().and_then(|e| e.downcast_ref::<rustls::Error>()) {
                Some(error) => Self::Refused(format!("TLS: {error}")),
                None => Self::Network,
            },
            _ => Self::Network,
        }
    }

    fn reason(&self) -> String {
        match self {
            Self::Network => "network".into(),
            Self::Unauthenticated => "HTTP 401".into(),
            Self::Refused(reason) => reason.clone(),
        }
    }
}

async fn connect(http: Http) -> Result<Socket, Failure> {
    let mut request = http
        .ws_url("/ws")
        .into_client_request()
        .map_err(|e| Failure::Refused(e.to_string()))?;
    if let Some(cookie) = http.cookie_header()
        && let Ok(value) = cookie.parse()
    {
        request.headers_mut().insert("Cookie", value);
    }
    let connector =
        crate::http::ws_connector().map_err(|e| Failure::Refused(format!("TLS: {e}")))?;
    let connect =
        tokio_tungstenite::connect_async_tls_with_config(request, None, false, Some(connector));
    match tokio::time::timeout(Duration::from_secs(10), connect).await {
        Ok(Ok((socket, _))) => Ok(socket),
        Ok(Err(error)) => {
            tracing::debug!(%error, "gateway connect failed");
            Err(Failure::of(&error))
        }
        Err(_) => Err(Failure::Network),
    }
}

/// The topic a frame names (`s`, `c`), read without its other fields.
fn undecodable_topic(text: &str) -> Option<Topic> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let id = |key: &str| value.get(key)?.as_str()?.parse::<Uuid>().ok();
    Some(Topic {
        s: id("s")?,
        c: id("c"),
    })
}

async fn send<S>(sink: &mut S, frame: &ClientFrame) -> Result<(), ()>
where
    S: futures_util::Sink<WsMessage> + Unpin,
{
    let text = serde_json::to_string(frame).map_err(|_| ())?;
    sink.send(WsMessage::text(text)).await.map_err(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    #[test]
    fn backoff_doubles_up_to_the_cap() {
        let c = GatewayConfig::default();
        assert_eq!(backoff(&c, 0, 0.7), Duration::ZERO);
        assert_eq!(backoff(&c, 1, 0.0), Duration::from_millis(100));
        assert_eq!(backoff(&c, 1, 1.0), Duration::from_millis(200));
        assert_eq!(backoff(&c, 3, 1.0), Duration::from_millis(800));
        assert_eq!(backoff(&c, 5, 1.0), Duration::from_millis(3200));
        assert_eq!(backoff(&c, 7, 1.0), Duration::from_millis(10_000));
        assert_eq!(backoff(&c, 40, 0.0), Duration::from_millis(5_000));
        let j = jitter();
        assert!((0.0..1.0).contains(&j));
    }

    #[test]
    fn events_dedupe_and_resume_cursor() {
        let mut c = Cursors::default();
        let t = Topic::channel(id(1), id(2));
        let ep = Some(id(9));
        assert!(!c.on_ok(t, 4, ep));
        assert_eq!(c.cursor(&t), Some(Cursor { ep, n: 4 }));
        assert_eq!(c.on_event(t, 5, ep), (true, false));
        assert_eq!(c.on_event(t, 5, ep), (false, false));
        assert_eq!(c.on_event(t, 3, ep), (false, false));
        assert_eq!(
            serde_json::to_value(c.subscribe_frame(&t)).unwrap(),
            serde_json::json!({"op":"s","s":id(1),"c":id(2),"n":5,"ep":id(9)})
        );
        let fresh = Topic::server(id(3));
        assert_eq!(
            serde_json::to_value(c.subscribe_frame(&fresh)).unwrap(),
            serde_json::json!({"op":"s","s":id(3)})
        );
    }

    #[test]
    fn gap_finishes_after_recovery_and_head() {
        let mut c = Cursors::default();
        let t = Topic::channel(id(1), id(2));
        c.on_ok(t, 10, Some(id(7)));
        // New epoch on resume: gap, then ok with the new head.
        c.on_gap(t, Some(id(8)));
        assert!(!c.on_ok(t, 3, Some(id(8))));
        // The old cursor stays until REST recovered.
        assert_eq!(c.cursor(&t).unwrap().n, 10);
        let recoveries = c.gap_recoveries();
        assert_eq!(recoveries.len(), 1);
        c.complete_gap(recoveries[0]);
        assert_eq!(
            c.cursor(&t),
            Some(Cursor {
                ep: Some(id(8)),
                n: 3
            })
        );
        assert!(c.gap_recoveries().is_empty());
    }

    #[test]
    fn live_epoch_change_accepts_and_marks_gap() {
        let mut c = Cursors::default();
        let t = Topic::dm(id(5));
        c.on_ok(t, 20, Some(id(1)));
        let (accept, gap) = c.on_event(t, 1, Some(id(2)));
        assert!(accept && gap);
        let rec = c.gap_recoveries();
        c.complete_gap(rec[0]);
        assert_eq!(
            c.cursor(&t),
            Some(Cursor {
                ep: Some(id(2)),
                n: 1
            })
        );
    }

    #[test]
    fn frames_decode_like_the_server_sends_them() {
        let f: ServerFrame = serde_json::from_str(
            r#"{"op":"e","t":"c","s":"00000000-0000-0000-0000-000000000001","c":"00000000-0000-0000-0000-000000000002","n":7,"ep":"00000000-0000-0000-0000-000000000003","i":"00000000-0000-0000-0000-000000000004","d":{"x":1},"r":12}"#,
        )
        .unwrap();
        let ServerFrame::Event(e) = f else { panic!() };
        assert_eq!((e.n, e.r), (7, Some(12)));
        let f: ServerFrame = serde_json::from_str(r#"{"op":"zz","x":1}"#).unwrap();
        assert_eq!(f, ServerFrame::Unknown);
        let f: ServerFrame = serde_json::from_str(r#"{"op":"ns"}"#).unwrap();
        assert_eq!(f, ServerFrame::NotificationSettings);
        let f: ServerFrame = serde_json::from_str(
            r#"{"op":"sig","t":"r","s":"00000000-0000-0000-0000-000000000001","snap":[{"u":"00000000-0000-0000-0000-000000000002","c":"00000000-0000-0000-0000-000000000003","m":true}]}"#,
        )
        .unwrap();
        let ServerFrame::Sig(sig) = f else { panic!() };
        assert!(sig.snap.unwrap()[0].m);
        let f: ServerFrame = serde_json::from_str(
            r#"{"op":"err","e":"kicked","s":"00000000-0000-0000-0000-000000000001"}"#,
        )
        .unwrap();
        assert!(matches!(f, ServerFrame::Err { ref e, c: None, .. } if e == "kicked"));
    }

    #[test]
    fn an_unreadable_frame_names_its_topic_for_a_gap() {
        let bad = r#"{"op":"e","t":"c","s":"00000000-0000-0000-0000-000000000001","c":"00000000-0000-0000-0000-000000000002","n":"sieben"}"#;
        assert!(serde_json::from_str::<ServerFrame>(bad).is_err());
        let topic = undecodable_topic(bad).unwrap();
        assert_eq!(topic, Topic::channel(id(1), id(2)));
        assert_eq!(undecodable_topic(r#"{"op":"e","n":1}"#), None);
        assert_eq!(undecodable_topic("nicht json"), None);
        // As a gap: recovered over REST before the cursor moves on.
        let mut c = Cursors::default();
        c.on_ok(topic, 6, None);
        c.on_gap(topic, None);
        assert_eq!(c.gap_recoveries().len(), 1);
    }

    /// A 401 on the upgrade (signed out elsewhere) ends the session as a
    /// 401 on REST does, and is not retried every second meanwhile.
    #[test]
    fn a_refused_upgrade_signs_out_and_waits() {
        use std::io::{Read, Write};
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let upgrades = Arc::new(AtomicUsize::new(0));
        let count = upgrades.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                count.fetch_add(1, Ordering::SeqCst);
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let _ = stream.write_all(
                    b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                );
            }
        });
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let http = Http::new(&format!("http://{addr}")).unwrap();
        let signed_out = Arc::new(AtomicBool::new(false));
        let flag = signed_out.clone();
        http.set_unauthenticated_hook(Arc::new(move || flag.store(true, Ordering::SeqCst)));
        let events = Arc::new(Mutex::new(Vec::new()));
        let seen = events.clone();
        let gateway = Gateway::start(
            runtime.handle(),
            http,
            GatewayConfig::default(),
            Arc::new(move |e| seen.lock().push(e)),
        );
        std::thread::sleep(Duration::from_millis(2_000));
        assert!(signed_out.load(Ordering::SeqCst));
        assert_eq!(upgrades.load(Ordering::SeqCst), 1);
        assert!(
            events
                .lock()
                .iter()
                .any(|e| matches!(e, GatewayEvent::Refused(Some(reason)) if reason == "HTTP 401"))
        );
        gateway.stop();
    }

    /// A server that accepts the socket and drops it at once is retried
    /// with a growing delay, not at once every time.
    #[test]
    fn an_accept_then_drop_server_is_backed_off() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let listener = runtime
            .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let opens = Arc::new(AtomicUsize::new(0));
        let count = opens.clone();
        runtime.spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                if let Ok(mut socket) = tokio_tungstenite::accept_async(stream).await {
                    count.fetch_add(1, Ordering::SeqCst);
                    let _ = socket.close(None).await;
                }
            }
        });
        let http = Http::new(&format!("http://{addr}")).unwrap();
        let gateway = Gateway::start(
            runtime.handle(),
            http,
            GatewayConfig::default(),
            Arc::new(|_| {}),
        );
        std::thread::sleep(Duration::from_millis(1_500));
        gateway.stop();
        let opens = opens.load(Ordering::SeqCst);
        assert!((2..10).contains(&opens), "{opens} opens in 1.5 s");
    }

    #[test]
    fn client_frames_encode_exactly() {
        let s = id(1);
        let c = id(2);
        let cases = [
            (ClientFrame::Heartbeat, serde_json::json!({"op":"h"})),
            (
                ClientFrame::Unsubscribe { s, c: None },
                serde_json::json!({"op":"u","s":s}),
            ),
            (
                ClientFrame::Typing { s, c, on: true },
                serde_json::json!({"op":"y","s":s,"c":c,"on":true}),
            ),
            (
                ClientFrame::Presence {
                    st: Some(crate::models::PresenceStatus::Idle),
                },
                serde_json::json!({"op":"p","st":"i"}),
            ),
            (
                ClientFrame::Focus { on: true },
                serde_json::json!({"op":"f","on":true}),
            ),
            (
                ClientFrame::Sig {
                    t: crate::models::SigKind::P,
                    s,
                    c,
                    k: Some(crate::models::TrackKind::Sa),
                    on: None,
                },
                serde_json::json!({"op":"sig","t":"p","s":s,"c":c,"k":"sa"}),
            ),
        ];
        for (frame, json) in cases {
            assert_eq!(serde_json::to_value(&frame).unwrap(), json);
        }
    }
}
