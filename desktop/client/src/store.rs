//! Everything the interface draws, in one place it reads each frame
//! ([`crate::Client::state`]). Only the client runtime writes it; the
//! reducers here are pure so they are tested without a server.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use uuid::Uuid;

use crate::gateway::GatewayStatus;
use crate::http::ApiError;
use crate::models::*;

/// Remote typing entries expire after the last `on: true` (`TYPING_TTL_MS`).
pub const TYPING_TTL: Duration = Duration::from_millis(6_000);
/// First page and every older page (`messages/rules.ts`).
pub const PAGE_SIZE: u32 = 50;

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Session {
    /// Not asked yet, or the check failed (see `State::session_error`).
    #[default]
    Unknown,
    Anonymous,
    Authenticated(Box<User>),
}

/// Progress of a named command (`"login"`, `"send"`, …) for forms and
/// spinners.
#[derive(Clone, Debug, PartialEq)]
pub enum OpState {
    Pending,
    Done,
    Failed(ApiError),
}

impl PartialEq for ApiError {
    fn eq(&self, other: &Self) -> bool {
        self.to_string() == other.to_string() && self.code() == other.code()
    }
}

/// What the user looks at; drives topics, read marks and badges.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct View {
    pub server: Option<Uuid>,
    /// A text channel of `server`, or a DM when `server` is `None`.
    pub channel: Option<Uuid>,
}

/// A send the server has not confirmed yet (the web's `tmp:<nonce>` row).
#[derive(Clone, Debug, PartialEq)]
pub struct PendingMessage {
    pub nonce: Uuid,
    pub channel_id: Uuid,
    pub content: String,
    pub reply_to: Option<Uuid>,
    pub gif: Option<GifPick>,
    pub files: Vec<OutgoingFile>,
    pub created_at: Timestamp,
    pub failed: Option<ApiError>,
}

/// A file to attach; kept with the pending row so a retry can send it again.
#[derive(Clone, Debug, PartialEq)]
pub struct OutgoingFile {
    pub filename: String,
    pub content_type: String,
    pub data: std::sync::Arc<Vec<u8>>,
}

/// One conversation's loaded history.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChannelMessages {
    /// Oldest → newest, by `(created_at, id)`.
    pub messages: Vec<Message>,
    /// Older messages exist on the server.
    pub has_more: bool,
    pub loaded: bool,
    pub loading: bool,
    pub loading_older: bool,
    pub error: Option<ApiError>,
    pub pending: Vec<PendingMessage>,
    /// Missed events (gap): fetch again when opened.
    pub stale: bool,
}

impl ChannelMessages {
    pub fn get(&self, id: Uuid) -> Option<&Message> {
        self.messages.iter().find(|m| m.id == id)
    }

    /// Inserts or updates by id; a stale copy never replaces a newer one.
    /// Clears the pending row its nonce names.
    pub fn upsert(&mut self, message: Message) {
        if let Some(nonce) = message.nonce {
            self.pending.retain(|p| p.nonce != nonce);
        }
        if let Some(existing) = self.messages.iter_mut().find(|m| m.id == message.id) {
            if message.is_newer_than(existing) {
                *existing = message;
            }
            let updated = existing.clone();
            self.refresh_replies(&updated);
            return;
        }
        let key = message.order_key();
        let at = self.messages.partition_point(|m| m.order_key() < key);
        self.messages.insert(at, message);
    }

    pub fn remove(&mut self, id: Uuid) -> bool {
        let before = self.messages.len();
        self.messages.retain(|m| m.id != id);
        for message in &mut self.messages {
            if let Some(reply) = &mut message.reply
                && reply.id == id
            {
                *reply = Reply {
                    id,
                    deleted: true,
                    author: None,
                    excerpt: None,
                    has_attachment: false,
                    has_gif: false,
                    system: None,
                };
            }
        }
        before != self.messages.len()
    }

    /// Rows that quote an edited message show its new text.
    fn refresh_replies(&mut self, original: &Message) {
        for message in &mut self.messages {
            if let Some(reply) = &mut message.reply
                && reply.id == original.id
                && !reply.deleted
            {
                reply.excerpt = Some(excerpt(&original.content));
                reply.has_attachment = !original.attachments.is_empty();
                reply.has_gif = original.gif.is_some();
            }
        }
    }

    /// The first page (or a refetch after a gap) replaces what was loaded.
    /// Rows newer than the page came live while it was requested and stay;
    /// before the first load, every row held came that way.
    pub fn replace(&mut self, page: MessagePage) {
        let first_load = !self.loaded;
        let newest = page.messages.last().map(Message::order_key);
        let held = std::mem::replace(&mut self.messages, page.messages);
        for message in held {
            if first_load || newest.is_some_and(|n| message.order_key() > n) {
                self.upsert(message);
            }
        }
        self.has_more = page.has_more;
        self.loaded = true;
        self.loading = false;
        self.stale = false;
        self.error = None;
    }

    /// An older page goes in front; duplicates are skipped.
    pub fn prepend(&mut self, page: MessagePage) {
        self.has_more = page.has_more;
        self.loading_older = false;
        for message in page.messages {
            if self.get(message.id).is_none() {
                self.upsert(message);
            }
        }
    }

    /// The newest page after missed events: rows in its range that it lacks
    /// were deleted meanwhile; without overlap it replaces everything.
    pub fn merge_latest(&mut self, page: MessagePage) {
        let overlaps = match (page.messages.first(), self.messages.last()) {
            (Some(first), Some(last)) => first.order_key() <= last.order_key(),
            _ => false,
        };
        if !overlaps {
            self.replace(page);
            return;
        }
        let start = page.messages[0].order_key();
        let end = page.messages[page.messages.len() - 1].order_key();
        // Rows past the page's end came live after it was read.
        self.messages.retain(|m| {
            let key = m.order_key();
            key < start || key > end || page.messages.iter().any(|p| p.id == m.id)
        });
        for message in page.messages {
            self.upsert(message);
        }
        self.stale = false;
        self.error = None;
    }

    pub fn oldest_cursor(&self) -> Option<String> {
        self.messages.first().map(Message::cursor)
    }
}

/// The server's reply excerpt (`messages/replies.rs`): one line, spoilers
/// masked, at most 100 characters plus `…`, no cut mention token.
pub fn excerpt(content: &str) -> String {
    let line = content.split_whitespace().collect::<Vec<_>>().join(" ");
    let parts: Vec<&str> = line.split("||").collect();
    let mut masked = String::with_capacity(line.len());
    for (index, part) in parts.iter().enumerate() {
        let inside = index % 2 == 1;
        if inside && index + 1 < parts.len() {
            masked.push_str("▒▒▒");
        } else {
            if inside {
                masked.push_str("||");
            }
            masked.push_str(part);
        }
    }
    let mut chars = masked.chars();
    let mut out: String = chars.by_ref().take(100).collect();
    if chars.next().is_some() {
        if let Some(start) = out.rfind("<@")
            && !out[start..].contains('>')
        {
            out.truncate(start);
        }
        out.push('…');
    }
    out
}

/// Where someone sits in voice (`voice/roster.ts`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VoiceOccupant {
    pub channel_id: Uuid,
    pub muted: bool,
    pub deafened: bool,
}

/// This account's own seat as far as the gateway is concerned.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VoiceSelf {
    pub server: Option<Uuid>,
    pub channel: Option<Uuid>,
    pub muted: bool,
    pub deafened: bool,
}

/// A one-off message for a toast.
#[derive(Clone, Debug, PartialEq)]
pub enum Notice {
    Kicked {
        server: Uuid,
        text: String,
    },
    Banned {
        server: Uuid,
        text: String,
    },
    Error(String),
    /// A new message by someone else (`useMessageToastsBridge`): the UI
    /// decides whether it shows as a toast.
    Message {
        server: Uuid,
        message: Box<Message>,
    },
}

#[derive(Debug, Default)]
pub struct State {
    pub session: Session,
    /// The last failed session check (network, 5xx): retry later.
    pub session_error: Option<ApiError>,
    /// The startup requests have answered.
    pub bootstrapped: bool,
    pub servers: Vec<Server>,
    pub servers_loaded: bool,
    pub server_details: HashMap<Uuid, ServerDetail>,
    pub dms: Vec<DirectMessage>,
    pub dms_loaded: bool,
    pub channels: HashMap<Uuid, ChannelMessages>,
    pub read: HashMap<Uuid, ReadState>,
    pub notification_settings: Vec<NotificationSetting>,
    pub notification_privacy: Option<NotificationPrivacy>,
    pub themes: Option<ThemeDocument>,
    pub mail: Option<MailStatus>,
    pub gifs: Option<GifStatus>,
    pub profiles: HashMap<Uuid, PublicProfile>,
    /// server (or DM id) → user → status; absent = offline.
    pub presence: HashMap<Uuid, HashMap<Uuid, PresenceStatus>>,
    /// channel → user → expiry.
    pub typing: HashMap<Uuid, HashMap<Uuid, Instant>>,
    /// server → user → seat.
    pub voice: HashMap<Uuid, HashMap<Uuid, VoiceOccupant>>,
    /// server → channel → users who are live, in the order they went live.
    pub live: HashMap<Uuid, HashMap<Uuid, Vec<Uuid>>>,
    pub voice_self: VoiceSelf,
    pub gateway: GatewayStatus,
    pub view: View,
    pub ops: HashMap<String, OpState>,
    /// Registration answered with a confirmation mail ("Postfach prüfen").
    pub verification_sent: bool,
    /// The server ended the session (`Sitzung abgelaufen.` on the login page).
    pub session_expired: bool,
    /// The last gateway `err` frame: code, server, channel (voice joins).
    pub gateway_error: Option<(String, Option<Uuid>, Option<Uuid>)>,
    /// The last server shown; its channels stay subscribed in DMs and
    /// settings, like the web's remembered server (`useGateway.ts`).
    pub last_server: Option<Uuid>,
    /// Bumped by every account reset; answers started before are dropped.
    pub(crate) epoch: u64,
}

impl State {
    pub fn me(&self) -> Option<&User> {
        match &self.session {
            Session::Authenticated(user) => Some(user.as_ref()),
            _ => None,
        }
    }

    pub fn op(&self, key: &str) -> Option<&OpState> {
        self.ops.get(key)
    }

    pub fn server(&self, id: Uuid) -> Option<&Server> {
        self.servers.iter().find(|s| s.id == id)
    }

    pub fn detail(&self, id: Uuid) -> Option<&ServerDetail> {
        self.server_details.get(&id)
    }

    /// A channel of any loaded server.
    pub fn channel(&self, id: Uuid) -> Option<&Channel> {
        self.server_details
            .values()
            .flat_map(|d| d.channels.iter())
            .find(|c| c.id == id)
    }

    pub fn dm(&self, id: Uuid) -> Option<&DirectMessage> {
        self.dms.iter().find(|d| d.id == id)
    }

    pub fn messages(&self, channel: Uuid) -> Option<&ChannelMessages> {
        self.channels.get(&channel)
    }

    /// Finds a loaded message in any conversation.
    pub fn find_message(&self, id: Uuid) -> Option<&Message> {
        self.channels.values().find_map(|c| c.get(id))
    }

    /// Clears all account data (sign-out, other account).
    pub fn reset_account(&mut self) {
        let mail = self.mail;
        let epoch = self.epoch + 1;
        *self = State {
            mail,
            epoch,
            ..State::default()
        };
    }

    // ----------------------------------------------------------- badges

    /// The sidebar badge of one conversation (`notifyLevel.ts` `badgeOf`).
    pub fn channel_badge(&self, channel: Uuid, now: Timestamp) -> Badge {
        self.read
            .get(&channel)
            .map(|r| r.badge(self.view.channel, now))
            .unwrap_or_default()
    }

    /// The server rail badge: the sum over its channels.
    pub fn server_badge(&self, server: Uuid, now: Timestamp) -> Badge {
        self.read
            .values()
            .filter(|r| r.server_id == Some(server))
            .fold(Badge::default(), |acc, r| {
                acc + r.badge(self.view.channel, now)
            })
    }

    /// The DM button badge: every DM's count.
    pub fn dms_badge(&self, now: Timestamp) -> Badge {
        self.read
            .values()
            .filter(|r| r.server_id.is_none())
            .fold(Badge::default(), |acc, r| {
                acc + r.badge(self.view.channel, now)
            })
    }

    pub fn apply_read_states(&mut self, rows: Vec<ReadState>) {
        for row in rows {
            self.read.insert(row.channel_id, row);
        }
    }

    /// `rd` from another device; true when the counts must be refetched.
    pub fn apply_read_sync(&mut self, rows: &[ReadSync]) -> bool {
        let mut recount = false;
        for row in rows {
            if let Some(current) = self.read.get_mut(&row.channel_id) {
                recount |= current.apply_sync(row) == Some(true);
            }
        }
        recount
    }

    // ----------------------------------------------------------- messages

    pub fn channel_mut(&mut self, channel: Uuid) -> &mut ChannelMessages {
        self.channels.entry(channel).or_default()
    }

    /// A message from a REST answer or a create/edit event. Histories not
    /// loaded yet stay untouched: opening fetches them.
    pub fn apply_message(&mut self, message: Message) {
        let Some(channel) = self.channels.get_mut(&message.channel_id) else {
            return;
        };
        // While the first page loads, live rows are kept: they may be newer
        // than the page, and `replace` merges them in.
        if !channel.loaded && !channel.loading {
            if let Some(nonce) = message.nonce {
                channel.pending.retain(|p| p.nonce != nonce);
            }
            return;
        }
        channel.upsert(message);
    }

    pub fn apply_delete(&mut self, channel: Uuid, id: Uuid) {
        if let Some(channel) = self.channels.get_mut(&channel) {
            channel.remove(id);
        }
    }

    /// A sequenced chat event (`gateway` already dropped duplicates).
    /// Returns the server whose membership I lost, with whether by ban.
    pub fn apply_event(&mut self, event: &EventFrame) -> Option<(Uuid, bool)> {
        match (event.c, event.t) {
            (Some(channel), EventKind::D) => {
                if let Some(id) = event.i {
                    self.apply_delete(channel, id);
                }
            }
            (Some(_), EventKind::C | EventKind::E) => {
                if let Some(message) = event.message() {
                    // Someone who just wrote stopped typing.
                    if let Some(typers) = self.typing.get_mut(&message.channel_id) {
                        typers.remove(&message.author.id);
                    }
                    self.apply_message(message);
                }
            }
            (None, EventKind::D) => {
                let user = event.i?;
                let me = self.me().map(|u| u.id);
                if Some(user) == me {
                    let banned = event
                        .d
                        .as_ref()
                        .and_then(|d| d.get("k"))
                        .and_then(|k| k.as_str())
                        == Some("b");
                    return Some((event.s, banned));
                }
                if let Some(detail) = self.server_details.get_mut(&event.s) {
                    detail.members.retain(|m| m.user_id != user);
                }
                if let Some(map) = self.presence.get_mut(&event.s) {
                    map.remove(&user);
                }
            }
            _ => {}
        }
        None
    }

    /// Kicked, banned, left or deleted: drop everything of that server.
    pub fn forget_server(&mut self, server: Uuid) {
        self.ops.remove(&format!("server:{server}"));
        self.servers.retain(|s| s.id != server);
        if let Some(detail) = self.server_details.remove(&server) {
            for channel in detail.channels {
                self.channels.remove(&channel.id);
                self.typing.remove(&channel.id);
            }
        }
        self.read.retain(|_, r| r.server_id != Some(server));
        self.presence.remove(&server);
        self.voice.remove(&server);
        self.live.remove(&server);
        if self.view.server == Some(server) {
            self.view = View::default();
        }
        if self.last_server == Some(server) {
            self.last_server = None;
        }
        if self.voice_self.server == Some(server) {
            self.voice_self = VoiceSelf::default();
        }
    }

    // ----------------------------------------------------------- presence

    pub fn apply_presence(
        &mut self,
        server: Uuid,
        user: Option<Uuid>,
        status: Option<PresenceStatus>,
        snap: Option<&[PresenceEntry]>,
    ) {
        if let Some(snap) = snap {
            let map = snap
                .iter()
                .filter(|e| e.st != PresenceStatus::Offline)
                .map(|e| (e.u, e.st))
                .collect();
            self.presence.insert(server, map);
            return;
        }
        let (Some(user), Some(status)) = (user, status) else {
            return;
        };
        let map = self.presence.entry(server).or_default();
        if status == PresenceStatus::Offline {
            map.remove(&user);
        } else {
            map.insert(user, status);
        }
    }

    pub fn presence_of(&self, server: Uuid, user: Uuid) -> PresenceStatus {
        self.presence
            .get(&server)
            .and_then(|m| m.get(&user))
            .copied()
            .unwrap_or(PresenceStatus::Offline)
    }

    /// The member list's groups (`live.ts`): Online, Abwesend, Offline, each
    /// sorted by name; empty groups left out.
    pub fn member_groups(&self, server: Uuid) -> Vec<(PresenceStatus, Vec<&Member>)> {
        let Some(detail) = self.server_details.get(&server) else {
            return Vec::new();
        };
        let mut groups = Vec::new();
        for status in [
            PresenceStatus::Online,
            PresenceStatus::Idle,
            PresenceStatus::Offline,
        ] {
            let mut members: Vec<&Member> = detail
                .members
                .iter()
                .filter(|m| self.presence_of(server, m.user_id) == status)
                .collect();
            members.sort_by_cached_key(|m| collation_key(&m.name));
            if !members.is_empty() {
                groups.push((status, members));
            }
        }
        groups
    }

    // ------------------------------------------------------------- typing

    pub fn apply_typing(&mut self, channel: Uuid, user: Uuid, on: bool, now: Instant) {
        if on {
            self.typing
                .entry(channel)
                .or_default()
                .insert(user, now + TYPING_TTL);
        } else if let Some(map) = self.typing.get_mut(&channel) {
            map.remove(&user);
        }
    }

    /// Drops expired typing entries; true when something changed.
    pub fn prune_typing(&mut self, now: Instant) -> bool {
        let mut changed = false;
        for map in self.typing.values_mut() {
            let before = map.len();
            map.retain(|_, until| *until > now);
            changed |= map.len() != before;
        }
        self.typing.retain(|_, m| !m.is_empty());
        changed
    }

    /// Who types in `channel`, without me, in a stable order.
    pub fn typers(&self, channel: Uuid, now: Instant) -> Vec<Uuid> {
        let me = self.me().map(|u| u.id);
        let mut users: Vec<Uuid> = self
            .typing
            .get(&channel)
            .map(|m| {
                m.iter()
                    .filter(|(u, until)| Some(**u) != me && **until > now)
                    .map(|(u, _)| *u)
                    .collect()
            })
            .unwrap_or_default();
        users.sort();
        users
    }

    /// A display name for a user seen in `channel`'s server or DM.
    pub fn name_of(&self, channel: Uuid, user: Uuid) -> Option<String> {
        if let Some(dm) = self.dm(channel)
            && dm.peer.id == user
        {
            return Some(dm.peer.name.clone());
        }
        let server = self.channel(channel).and_then(|c| c.server_id);
        server
            .and_then(|s| self.server_details.get(&s))
            .and_then(|d| d.members.iter().find(|m| m.user_id == user))
            .map(|m| m.name.clone())
    }

    /// The typing bar's text (`live.ts`), "" when nobody types.
    pub fn typing_text(&self, channel: Uuid, now: Instant) -> String {
        let names: Vec<String> = self
            .typers(channel, now)
            .into_iter()
            .map(|u| self.name_of(channel, u).unwrap_or_else(|| "Jemand".into()))
            .collect();
        typing_text(&names)
    }

    // -------------------------------------------------------------- voice

    /// A `sig` frame into the server-wide roster (`voice/roster.ts`).
    pub fn apply_sig(&mut self, sig: &SigFrame) {
        let server = sig.s;
        if sig.t == SigKind::R {
            let snap = sig.snap.as_deref().unwrap_or_default();
            let roster = snap
                .iter()
                .map(|e| {
                    (
                        e.u,
                        VoiceOccupant {
                            channel_id: e.c,
                            muted: e.m,
                            deafened: e.d,
                        },
                    )
                })
                .collect();
            self.voice.insert(server, roster);
            let mut live: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
            for entry in snap.iter().filter(|e| e.s) {
                live.entry(entry.c).or_default().push(entry.u);
            }
            self.live.insert(server, live);
            return;
        }
        let (Some(channel), Some(user)) = (sig.c, sig.u) else {
            return;
        };
        let roster = self.voice.entry(server).or_default();
        match sig.t {
            SigKind::J => {
                let previous = roster.get(&user).copied();
                roster.insert(
                    user,
                    VoiceOccupant {
                        channel_id: channel,
                        muted: sig.m.or(previous.map(|p| p.muted)).unwrap_or(false),
                        deafened: sig.d.or(previous.map(|p| p.deafened)).unwrap_or(false),
                    },
                );
            }
            SigKind::L => {
                let shown = roster.get(&user).map(|o| o.channel_id);
                let live = self.live.entry(server).or_default();
                if let Some(users) = live.get_mut(&channel) {
                    users.retain(|u| *u != user);
                }
                if shown.is_some_and(|c| c != channel) {
                    return;
                }
                roster.remove(&user);
                if let Some(old) = shown
                    && let Some(users) = live.get_mut(&old)
                {
                    users.retain(|u| *u != user);
                }
            }
            SigKind::M => {
                if let Some(o) = roster.get_mut(&user) {
                    o.muted = sig.on == Some(true);
                }
            }
            SigKind::D => {
                if let Some(o) = roster.get_mut(&user) {
                    o.deafened = sig.on == Some(true);
                    if o.deafened {
                        o.muted = true;
                    }
                }
            }
            SigKind::P if sig.k == Some(TrackKind::S) => {
                let users = self
                    .live
                    .entry(server)
                    .or_default()
                    .entry(channel)
                    .or_default();
                if !users.contains(&user) {
                    users.push(user);
                }
            }
            SigKind::U if sig.k == Some(TrackKind::S) => {
                if let Some(users) = self.live.get_mut(&server).and_then(|l| l.get_mut(&channel)) {
                    users.retain(|u| *u != user);
                }
            }
            _ => {}
        }
    }

    /// Who sits in a voice channel, sorted by name like the sidebar.
    pub fn voice_occupants(&self, server: Uuid, channel: Uuid) -> Vec<(Uuid, VoiceOccupant)> {
        let mut out: Vec<(Uuid, VoiceOccupant)> = self
            .voice
            .get(&server)
            .map(|r| {
                r.iter()
                    .filter(|(_, o)| o.channel_id == channel)
                    .map(|(u, o)| (*u, *o))
                    .collect()
            })
            .unwrap_or_default();
        out.sort_by_cached_key(|(u, _)| {
            let name = self
                .server_details
                .get(&server)
                .and_then(|d| d.members.iter().find(|m| m.user_id == *u))
                .map(|m| collation_key(&m.name))
                .unwrap_or_default();
            (name, *u)
        });
        out
    }

    pub fn is_live(&self, server: Uuid, channel: Uuid, user: Uuid) -> bool {
        self.live
            .get(&server)
            .and_then(|l| l.get(&channel))
            .is_some_and(|users| users.contains(&user))
    }

    // ------------------------------------------------------------- topics

    /// The gateway topics the web subscribes (`useGateway.ts`): the open
    /// server and channel, every text channel of the open (or last) server,
    /// every DM, and the voice room while in a call.
    pub fn desired_topics(&self) -> Vec<Topic> {
        let mut topics = Vec::new();
        if let Some(server) = self.view.server {
            topics.push(Topic::server(server));
            if let Some(channel) = self.view.channel {
                topics.push(Topic::channel(server, channel));
            }
        } else if let Some(dm) = self.view.channel {
            topics.push(Topic::dm(dm));
        }
        if let Some(server) = self.view.server.or(self.last_server) {
            topics.push(Topic::server(server));
            if let Some(detail) = self.server_details.get(&server) {
                for channel in detail
                    .channels
                    .iter()
                    .filter(|c| c.kind == ChannelKind::Text)
                {
                    topics.push(Topic::channel(server, channel.id));
                }
            }
        }
        for dm in &self.dms {
            topics.push(Topic::dm(dm.id));
        }
        if let (Some(server), Some(channel)) = (self.voice_self.server, self.voice_self.channel) {
            topics.push(Topic::server(server));
            topics.push(Topic::channel(server, channel));
        }
        topics
    }
}

/// `"{name} tippt…"` and friends, verbatim from web/src/ws/live.ts.
pub fn typing_text(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => format!("{one} tippt…"),
        [a, b] => format!("{a} und {b} tippen…"),
        [a, b, rest @ ..] => format!("{a}, {b} und {} weitere tippen…", rest.len()),
    }
}

/// Approximates `localeCompare(…, "de")`: case- and umlaut-insensitive.
pub fn collation_key(name: &str) -> String {
    name.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'ä' => 'a',
            'ö' => 'o',
            'ü' => 'u',
            'ß' => 's',
            'é' | 'è' | 'ê' => 'e',
            'á' | 'à' | 'â' => 'a',
            other => other,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn message(n: u128, secs: i64, revision: i64) -> Message {
        Message {
            id: id(n),
            channel_id: id(100),
            author: Author {
                id: id(1),
                name: "Jonas".into(),
                avatar_url: None,
            },
            content: format!("m{n}"),
            created_at: chrono::DateTime::from_timestamp(secs, 0).unwrap(),
            edited_at: None,
            revision,
            created_order: n as i64,
            attachments: vec![],
            reactions: vec![],
            gif: None,
            mentions: vec![],
            mention_everyone: false,
            reply: None,
            previews: vec![],
            system: None,
            nonce: None,
        }
    }

    #[test]
    fn upsert_keeps_order_and_newest_revision() {
        let mut c = ChannelMessages::default();
        c.upsert(message(2, 20, 2));
        c.upsert(message(1, 10, 1));
        c.upsert(message(3, 30, 3));
        assert_eq!(
            c.messages.iter().map(|m| m.id).collect::<Vec<_>>(),
            vec![id(1), id(2), id(3)]
        );
        let mut edited = message(2, 20, 5);
        edited.content = "neu".into();
        c.upsert(edited);
        let stale = message(2, 20, 4);
        c.upsert(stale);
        assert_eq!(c.get(id(2)).unwrap().content, "neu");
    }

    #[test]
    fn nonce_confirms_pending_and_delete_marks_replies() {
        let mut c = ChannelMessages::default();
        let nonce = id(77);
        c.pending.push(PendingMessage {
            nonce,
            channel_id: id(100),
            content: "x".into(),
            reply_to: None,
            gif: None,
            files: vec![],
            created_at: chrono::Utc::now(),
            failed: None,
        });
        let mut original = message(1, 10, 1);
        original.nonce = Some(nonce);
        c.upsert(original);
        assert!(c.pending.is_empty());
        let mut reply = message(2, 20, 2);
        reply.reply = Some(Reply {
            id: id(1),
            deleted: false,
            author: Some(ReplyAuthor {
                id: id(1),
                name: "Jonas".into(),
            }),
            excerpt: Some("m1".into()),
            has_attachment: false,
            has_gif: false,
            system: None,
        });
        c.upsert(reply);
        let mut edit = message(1, 10, 3);
        edit.content = "geändert ||geheim||".into();
        c.upsert(edit);
        assert_eq!(
            c.get(id(2))
                .unwrap()
                .reply
                .as_ref()
                .unwrap()
                .excerpt
                .as_deref(),
            Some("geändert ▒▒▒")
        );
        assert!(c.remove(id(1)));
        let quoted = c.get(id(2)).unwrap().reply.clone().unwrap();
        assert!(quoted.deleted && quoted.author.is_none());
    }

    fn page(messages: Vec<Message>) -> MessagePage {
        serde_json::from_value(serde_json::json!({
            "messages": messages,
            "has_more": false,
        }))
        .unwrap()
    }

    #[test]
    fn live_rows_survive_a_racing_page() {
        let mut s = State::default();
        s.channel_mut(id(100)).loading = true;
        // Created live while the first page was requested.
        s.apply_message(message(3, 30, 3));
        s.channel_mut(id(100))
            .replace(page(vec![message(1, 10, 1), message(2, 20, 2)]));
        let ids = |s: &State| {
            s.channels[&id(100)]
                .messages
                .iter()
                .map(|m| m.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&s), vec![id(1), id(2), id(3)]);
        // A stale refetch read before 4 arrived: 2 was deleted meanwhile.
        s.apply_message(message(4, 40, 4));
        s.channel_mut(id(100))
            .merge_latest(page(vec![message(1, 10, 1), message(3, 30, 3)]));
        assert_eq!(ids(&s), vec![id(1), id(3), id(4)]);
        // Without overlap the page replaces everything but newer live rows.
        s.channel_mut(id(100))
            .merge_latest(page(vec![message(5, 50, 5)]));
        assert_eq!(ids(&s), vec![id(5)]);
    }

    #[test]
    fn last_server_stays_subscribed_and_reset_bumps_the_epoch() {
        let mut s = State::default();
        let (server, dm) = (id(1), id(2));
        s.last_server = Some(server);
        s.view = View {
            server: None,
            channel: Some(dm),
        };
        let keys: Vec<String> = s.desired_topics().iter().map(Topic::key).collect();
        assert!(keys.contains(&Topic::server(server).key()));
        assert!(keys.contains(&Topic::dm(dm).key()));
        s.forget_server(server);
        assert_eq!(s.last_server, None);
        let epoch = s.epoch;
        s.reset_account();
        assert_eq!(s.epoch, epoch + 1);
    }

    #[test]
    fn excerpt_matches_the_server() {
        let long = "a ".repeat(80);
        let e = excerpt(&long);
        assert_eq!(e.chars().count(), 101);
        assert!(e.ends_with('…'));
        assert_eq!(excerpt("x\n\ny  z"), "x y z");
        assert_eq!(excerpt("a || b"), "a || b");
    }

    #[test]
    fn typing_text_is_german() {
        let n = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(typing_text(&n(&[])), "");
        assert_eq!(typing_text(&n(&["Lena"])), "Lena tippt…");
        assert_eq!(typing_text(&n(&["A", "B"])), "A und B tippen…");
        assert_eq!(
            typing_text(&n(&["A", "B", "C", "D"])),
            "A, B und 2 weitere tippen…"
        );
        let mut s = State::default();
        let now = Instant::now();
        s.apply_typing(id(5), id(6), true, now);
        assert_eq!(s.typing_text(id(5), now), "Jemand tippt…");
        assert!(s.prune_typing(now + TYPING_TTL));
        assert_eq!(s.typing_text(id(5), now), "");
    }

    #[test]
    fn presence_snapshot_and_updates() {
        let mut s = State::default();
        let snap = [
            PresenceEntry {
                u: id(1),
                st: PresenceStatus::Online,
            },
            PresenceEntry {
                u: id(2),
                st: PresenceStatus::Offline,
            },
        ];
        s.apply_presence(id(9), None, None, Some(&snap));
        assert_eq!(s.presence_of(id(9), id(1)), PresenceStatus::Online);
        assert_eq!(s.presence[&id(9)].len(), 1);
        s.apply_presence(id(9), Some(id(1)), Some(PresenceStatus::Idle), None);
        assert_eq!(s.presence_of(id(9), id(1)), PresenceStatus::Idle);
        s.apply_presence(id(9), Some(id(1)), Some(PresenceStatus::Offline), None);
        assert_eq!(s.presence_of(id(9), id(1)), PresenceStatus::Offline);
    }

    #[test]
    fn voice_roster_follows_sig() {
        let mut s = State::default();
        let (srv, a, b, u) = (id(1), id(2), id(3), id(4));
        let sig = |t, c: Option<Uuid>| SigFrame {
            t,
            s: srv,
            c,
            u: Some(u),
            k: None,
            on: None,
            m: None,
            d: None,
            snap: None,
        };
        s.apply_sig(&SigFrame {
            snap: Some(vec![VoiceEntry {
                u,
                c: a,
                m: true,
                d: false,
                s: true,
            }]),
            u: None,
            ..sig(SigKind::R, None)
        });
        assert!(s.voice_occupants(srv, a)[0].1.muted);
        assert!(s.is_live(srv, a, u));
        s.apply_sig(&sig(SigKind::J, Some(b)));
        assert!(s.voice_occupants(srv, a).is_empty());
        assert!(s.voice_occupants(srv, b)[0].1.muted);
        s.apply_sig(&SigFrame {
            on: Some(true),
            ..sig(SigKind::D, Some(b))
        });
        let o = s.voice_occupants(srv, b)[0].1;
        assert!(o.deafened && o.muted);
        // A leave of the old room only drops the Live there.
        s.apply_sig(&sig(SigKind::L, Some(a)));
        assert!(!s.is_live(srv, a, u));
        assert_eq!(s.voice_occupants(srv, b).len(), 1);
        s.apply_sig(&sig(SigKind::L, Some(b)));
        assert!(s.voice_occupants(srv, b).is_empty());
    }

    #[test]
    fn badges_follow_the_web_rules() {
        let now = chrono::Utc::now();
        let row = |channel: u128, server: Option<Uuid>, unread, mentions| ReadState {
            channel_id: id(channel),
            server_id: server,
            read_message_id: None,
            read_at: None,
            read_order: None,
            membership_at: now,
            latest_order: None,
            unread_count: unread,
            mention_count: mentions,
            level: NotificationLevel::All,
            muted: false,
            muted_until: None,
        };
        let mut s = State::default();
        s.apply_read_states(vec![
            row(1, Some(id(50)), 3, 0),
            row(2, Some(id(50)), 1, 1),
            row(3, None, 2, 0),
        ]);
        assert_eq!(
            s.channel_badge(id(1), now),
            Badge {
                count: 0,
                dot: true
            }
        );
        assert_eq!(
            s.server_badge(id(50), now),
            Badge {
                count: 1,
                dot: true
            }
        );
        assert_eq!(
            s.dms_badge(now),
            Badge {
                count: 2,
                dot: true
            }
        );
        s.view.channel = Some(id(3));
        assert_eq!(s.dms_badge(now), Badge::default());
    }

    #[test]
    fn collation_ignores_case_and_umlauts() {
        let mut names = vec!["Zoe", "ärger", "Anna", "bob"];
        names.sort_by_cached_key(|n| collation_key(n));
        assert_eq!(names, vec!["Anna", "ärger", "bob", "Zoe"]);
    }
}
