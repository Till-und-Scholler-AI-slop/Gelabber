//! The JSON shapes of the REST API and the `/ws` gateway, field for field as
//! the backend serialises them (backend/api/src). Keys the server omits when
//! empty are `Option` or `#[serde(default)]` here, so both directions match.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};
use uuid::Uuid;

pub type Timestamp = DateTime<Utc>;

/// Live events pass through Redis cjson, which turns `[]` into `{}`
/// (web/src/messages/types.ts): accept an object or null as an empty list.
fn lenient_vec<'de, D, T>(de: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Either<T> {
        List(Vec<T>),
        Other(serde::de::IgnoredAny),
    }
    Ok(match Option::<Either<T>>::deserialize(de)? {
        Some(Either::List(list)) => list,
        _ => Vec::new(),
    })
}

// ---------------------------------------------------------------- auth / me

/// The signed-in account (`auth/user.rs`), profile card flattened in.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub name: String,
    pub avatar_url: Option<String>,
    pub created_at: Timestamp,
    pub bio: Option<String>,
    pub pronouns: Option<String>,
    pub status_text: Option<String>,
    pub accent_color: Option<String>,
    pub banner_color: Option<String>,
    pub banner_url: Option<String>,
    pub avatar_decoration: Option<String>,
}

/// `GET /api/users/{id}`: a user card without the e-mail address.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PublicProfile {
    pub id: Uuid,
    pub name: String,
    pub avatar_url: Option<String>,
    pub created_at: Timestamp,
    pub bio: Option<String>,
    pub pronouns: Option<String>,
    pub status_text: Option<String>,
    pub accent_color: Option<String>,
    pub banner_color: Option<String>,
    pub banner_url: Option<String>,
    pub avatar_decoration: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionResponse {
    pub user: Option<User>,
    pub csrf_token: String,
}

/// `POST /api/auth/register`: `201` signs in, `202` with
/// `verification_sent` means a confirmation mail went out instead.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RegisterResponse {
    pub user: Option<User>,
    pub csrf_token: String,
    #[serde(default)]
    pub verification_sent: bool,
}

/// Logout and logout-all answer only the rotated token.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CsrfResponse {
    pub csrf_token: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailStatus {
    pub enabled: bool,
}

/// `PATCH /api/me`: absent = unchanged, `""` = clear (not for `name`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ProfilePatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bio: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pronouns: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accent_color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub banner_color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub banner_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_decoration: Option<String>,
}

// ------------------------------------------------------------------- themes

/// `GET/PUT /api/me/themes`, the only camelCase shape of the API. The server
/// refuses unknown keys, so nothing may be added here.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThemeDocument {
    pub version: u8,
    pub revision: i64,
    pub active: String,
    pub custom_themes: Vec<Theme>,
}

impl Default for ThemeDocument {
    fn default() -> Self {
        Self {
            version: 1,
            revision: 0,
            active: "dark".into(),
            custom_themes: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Theme {
    pub version: u8,
    /// `custom-<uuid>`.
    pub id: String,
    pub name: String,
    /// `light` or `dark`.
    pub mode: String,
    /// `clear`, `soft` or `terminal`.
    pub style: String,
    pub colors: ThemeColors,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ThemeColors {
    pub background: String,
    pub panel: String,
    pub surface: String,
    pub rail: String,
    pub text: String,
    pub muted: String,
    pub accent: String,
    pub border: String,
}

// ------------------------------------------------------------------ servers

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    ManageServer,
    ManageChannels,
    ManageMessages,
    MentionEveryone,
    SendMessages,
    SendFiles,
    JoinVoice,
    GoLive,
    /// A permission this client does not know yet.
    #[serde(other)]
    Unknown,
}

impl Permission {
    /// The canonical order the server serialises (`servers/permissions.rs`).
    pub const ALL: [Permission; 8] = [
        Self::ManageServer,
        Self::ManageChannels,
        Self::ManageMessages,
        Self::MentionEveryone,
        Self::SendMessages,
        Self::SendFiles,
        Self::JoinVoice,
        Self::GoLive,
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Owner,
    Member,
}

/// `ServerView`: one entry of `GET /api/servers`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Server {
    pub id: Uuid,
    pub name: String,
    pub owner_id: Uuid,
    pub icon_url: Option<String>,
    pub created_at: Timestamp,
    pub role: Role,
    /// The caller's effective permissions.
    pub permissions: Vec<Permission>,
    /// What every non-owner holds.
    pub member_permissions: Vec<Permission>,
}

impl Server {
    pub fn can(&self, permission: Permission) -> bool {
        self.permissions.contains(&permission)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ServerDetail {
    #[serde(flatten)]
    pub server: Server,
    pub categories: Vec<Category>,
    pub channels: Vec<Channel>,
    pub members: Vec<Member>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Category {
    pub id: Uuid,
    pub server_id: Uuid,
    pub name: String,
    pub created_at: Timestamp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelKind {
    Text,
    Voice,
    Dm,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Channel {
    pub id: Uuid,
    /// `None` only for DM channels.
    pub server_id: Option<Uuid>,
    pub category_id: Option<Uuid>,
    pub name: String,
    pub kind: ChannelKind,
    pub created_at: Timestamp,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Member {
    pub user_id: Uuid,
    pub name: String,
    pub avatar_url: Option<String>,
    pub joined_at: Timestamp,
    pub status_text: Option<String>,
    pub accent_color: Option<String>,
    pub avatar_decoration: Option<String>,
    pub role: Role,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Ban {
    pub user_id: Uuid,
    pub name: String,
    pub avatar_url: Option<String>,
    pub banned_by: Uuid,
    pub banned_at: Timestamp,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Invite {
    pub code: String,
    pub server_id: Uuid,
    pub created_by: Uuid,
    pub created_at: Timestamp,
    pub expires_at: Option<Timestamp>,
    pub max_uses: Option<i32>,
    pub uses: i32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InvitePreview {
    pub code: String,
    pub server: InviteServer,
    pub expires_at: Option<Timestamp>,
    pub member: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InviteServer {
    pub id: Uuid,
    pub name: String,
    pub icon_url: Option<String>,
    pub member_count: i64,
}

/// `PATCH /api/servers/{id}`: absent = unchanged, `[]` = no permissions.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ServerPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub member_permissions: Option<Vec<Permission>>,
}

/// `PATCH /api/channels/{id}`: `category_id: Some("")` detaches.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ChannelPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category_id: Option<String>,
}

// ---------------------------------------------------------------------- DMs

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DirectMessage {
    /// The DM channel id.
    pub id: Uuid,
    pub kind: ChannelKind,
    pub created_at: Timestamp,
    pub peer: Peer,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Peer {
    pub id: Uuid,
    pub name: String,
    pub avatar_url: Option<String>,
}

// ----------------------------------------------------------------- messages

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Author {
    pub id: Uuid,
    pub name: String,
    pub avatar_url: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub id: Uuid,
    pub channel_id: Uuid,
    pub author: Author,
    pub content: String,
    pub created_at: Timestamp,
    pub edited_at: Option<Timestamp>,
    pub revision: i64,
    pub created_order: i64,
    #[serde(default, deserialize_with = "lenient_vec")]
    pub attachments: Vec<Attachment>,
    #[serde(default, deserialize_with = "lenient_vec")]
    pub reactions: Vec<Reaction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gif: Option<Gif>,
    #[serde(default, deserialize_with = "lenient_vec")]
    pub mentions: Vec<Uuid>,
    #[serde(default)]
    pub mention_everyone: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply: Option<Reply>,
    #[serde(
        default,
        deserialize_with = "lenient_vec",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub previews: Vec<LinkPreview>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<SystemMessage>,
    /// Echo of the sender's attempt, on the create response and its event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nonce: Option<Uuid>,
}

impl Message {
    /// The web's `newerMessage`: higher revision, else the later edit/create.
    pub fn is_newer_than(&self, other: &Message) -> bool {
        if self.revision != other.revision {
            return self.revision > other.revision;
        }
        self.edited_at.unwrap_or(self.created_at) > other.edited_at.unwrap_or(other.created_at)
    }

    /// History order: `created_at`, then id, as the server pages.
    pub fn order_key(&self) -> (Timestamp, Uuid) {
        (self.created_at, self.id)
    }

    /// The `before` cursor of a page starting at this message.
    pub fn cursor(&self) -> String {
        format!(
            "{}|{}",
            self.created_at
                .to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true),
            self.id
        )
    }

    pub fn reacted(&self, emoji: &str, user: Uuid) -> bool {
        self.reactions
            .iter()
            .any(|r| r.emoji == emoji && r.user_ids.contains(&user))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Attachment {
    pub id: Uuid,
    pub filename: String,
    pub content_type: String,
    pub size: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Reaction {
    pub emoji: String,
    #[serde(default, deserialize_with = "lenient_vec")]
    pub user_ids: Vec<Uuid>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Gif {
    pub provider: String,
    pub slug: String,
    #[serde(default)]
    pub title: String,
    pub width: u32,
    pub height: u32,
    pub url: String,
    #[serde(default)]
    pub still: Option<String>,
}

/// The quoted original of a reply; a deleted one is only `{id, deleted}`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Reply {
    pub id: Uuid,
    pub deleted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<ReplyAuthor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub excerpt: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub has_attachment: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub has_gif: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<SystemMessage>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReplyAuthor {
    pub id: Uuid,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LinkPreview {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub site_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<PreviewImage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub youtube_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PreviewImage {
    pub url: String,
    pub width: u32,
    pub height: u32,
}

/// Written by the server; today only `live_started` with `voice_channel_id`
/// and `voice_name` in `params`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SystemMessage {
    pub kind: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MessagePage {
    /// Oldest → newest.
    pub messages: Vec<Message>,
    pub has_more: bool,
}

/// `GET /api/channels/{id}/messages/{message_id}/context`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MessageContext {
    pub target_id: Uuid,
    pub messages: Vec<Message>,
    /// Cursors (`ts|uuid`, `+00:00` offset) for further history.
    pub before: String,
    pub after: String,
}

/// `POST /api/channels/{id}/messages`. A GIF goes alone: empty content and
/// no attachments.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct NewMessage {
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachment_ids: Option<Vec<Uuid>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gif: Option<GifPick>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nonce: Option<Uuid>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GifPick {
    pub slug: String,
    #[serde(default)]
    pub query: String,
}

// ------------------------------------------------------- unread / read state

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NotificationLevel {
    #[default]
    All,
    Mentions,
    None,
}

/// One row of `GET /api/messages/unread`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReadState {
    pub channel_id: Uuid,
    pub server_id: Option<Uuid>,
    pub read_message_id: Option<Uuid>,
    pub read_at: Option<Timestamp>,
    pub read_order: Option<i64>,
    pub membership_at: Timestamp,
    pub latest_order: Option<i64>,
    /// Capped at 100 by the server.
    pub unread_count: i64,
    pub mention_count: i64,
    #[serde(default)]
    pub level: NotificationLevel,
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub muted_until: Option<Timestamp>,
}

impl ReadState {
    /// A mute that holds at `now`; one whose end has passed is over.
    pub fn is_muted(&self, now: Timestamp) -> bool {
        self.muted && self.muted_until.is_none_or(|until| until > now)
    }

    /// The web's `badgeOf`: in a server channel unread is a dot and only
    /// mentions count; a DM always counts. The open conversation shows none.
    pub fn badge(&self, reading: Option<Uuid>, now: Timestamp) -> Badge {
        if reading == Some(self.channel_id) {
            return Badge::default();
        }
        let muted = self.is_muted(now);
        let none = self.level == NotificationLevel::None;
        let unread = self.unread_count.max(0);
        let dot = !muted && !none && unread > 0;
        if self.server_id.is_none() {
            return Badge {
                count: if muted || none { 0 } else { unread },
                dot,
            };
        }
        Badge {
            count: if none { 0 } else { self.mention_count.max(0) },
            dot,
        }
    }

    /// Takes a pushed read position unless it is behind ours (the web's
    /// `applyReadSync`). `Some(true)` = taken, but our row counted messages
    /// the push did not know yet, so the counts must be fetched again.
    pub fn apply_sync(&mut self, sync: &ReadSync) -> Option<bool> {
        let incoming = (sync.membership_at, sync.read_order, sync.latest_order);
        let current = (self.membership_at, self.read_order, self.latest_order);
        if incoming < current {
            return None;
        }
        let recount = self.latest_order > sync.latest_order;
        self.read_message_id = sync.read_message_id;
        self.read_at = sync.read_at;
        self.read_order = sync.read_order;
        self.membership_at = sync.membership_at;
        self.latest_order = sync.latest_order;
        self.unread_count = sync.unread_count;
        self.mention_count = sync.mention_count;
        Some(recount)
    }
}

/// What a sidebar entry shows: a number (mentions, DMs) or a plain dot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Badge {
    pub count: i64,
    pub dot: bool,
}

impl std::ops::Add for Badge {
    type Output = Badge;

    fn add(self, other: Badge) -> Badge {
        Badge {
            count: self.count + other.count,
            dot: self.dot || other.dot,
        }
    }
}

/// An element of the gateway's `rd` frame.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReadSync {
    pub channel_id: Uuid,
    pub server_id: Option<Uuid>,
    pub read_message_id: Option<Uuid>,
    pub read_at: Option<Timestamp>,
    pub read_order: Option<i64>,
    pub membership_at: Timestamp,
    pub latest_order: Option<i64>,
    pub unread_count: i64,
    pub mention_count: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReadScope {
    All,
    Dms,
}

// --------------------------------------------------- notification settings

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SettingScope {
    Server,
    Channel,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NotificationSetting {
    pub scope: SettingScope,
    pub id: Uuid,
    /// `None` inherits.
    pub level: Option<NotificationLevel>,
    pub muted: bool,
    pub muted_until: Option<Timestamp>,
}

/// `PUT …/notification-settings`; `level: None` + not muted = inherit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NotificationSettingBody {
    pub level: Option<NotificationLevel>,
    pub muted: bool,
    /// 1..=527040, only with `muted`; absent mutes until turned back on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mute_minutes: Option<i64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationPrivacy {
    pub hide_content: bool,
}

// ------------------------------------------------------ attachments, media

/// `POST /api/channels/{id}/attachments`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PresignRequest {
    pub filename: String,
    pub content_type: String,
    pub size: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Presigned {
    pub id: Uuid,
    /// Presigned MinIO URL, or `/api/attachments/{id}/upload` in proxy mode.
    pub upload_url: String,
    /// Send every one of these with the PUT.
    pub headers: BTreeMap<String, String>,
    pub expires_in: u64,
    pub attachment: Attachment,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MediaTicket {
    pub ticket: String,
    pub expires_in: u64,
    pub media_path: String,
    pub ice_servers: Vec<IceServer>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IceServer {
    pub urls: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,
}

// ---------------------------------------------------------------------- GIFs

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GifStatus {
    pub enabled: bool,
    pub provider: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GifPage {
    pub gifs: Vec<Gif>,
    pub next_page: Option<u32>,
}

// ------------------------------------------------------------------ gateway

/// Presence as the gateway spells it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PresenceStatus {
    #[serde(rename = "o")]
    Online,
    #[serde(rename = "i")]
    Idle,
    #[serde(rename = "x")]
    Offline,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PresenceEntry {
    pub u: Uuid,
    pub st: PresenceStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EventKind {
    C,
    E,
    D,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SigKind {
    /// Join.
    J,
    /// Leave.
    L,
    /// Publish a track.
    P,
    /// Unpublish a track.
    U,
    /// Mute flag.
    M,
    /// Deafen flag.
    D,
    /// Occupancy snapshot (server only).
    R,
}

/// Microphone, camera, screen (a Live) and the screen's audio.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrackKind {
    A,
    V,
    S,
    Sa,
}

/// One occupant in a `sig` `r` snapshot; `s` = publishes a screen.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VoiceEntry {
    pub u: Uuid,
    pub c: Uuid,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub m: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub d: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub s: bool,
}

/// Frames the client sends on `/ws` (`gateway/protocol.rs` `ClientFrame`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op")]
pub enum ClientFrame {
    #[serde(rename = "h")]
    Heartbeat,
    #[serde(rename = "s")]
    Subscribe {
        s: Uuid,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        c: Option<Uuid>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        n: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ep: Option<Uuid>,
    },
    #[serde(rename = "u")]
    Unsubscribe {
        s: Uuid,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        c: Option<Uuid>,
    },
    #[serde(rename = "sig")]
    Sig {
        t: SigKind,
        s: Uuid,
        c: Uuid,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        k: Option<TrackKind>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        on: Option<bool>,
    },
    #[serde(rename = "f")]
    Focus { on: bool },
    #[serde(rename = "p")]
    Presence {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        st: Option<PresenceStatus>,
    },
    #[serde(rename = "y")]
    Typing { s: Uuid, c: Uuid, on: bool },
}

/// Frames the server sends on `/ws`. Unknown ops decode as `Unknown`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op")]
pub enum ServerFrame {
    #[serde(rename = "h")]
    Heartbeat,
    #[serde(rename = "ok")]
    Ok {
        s: Uuid,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        c: Option<Uuid>,
        n: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ep: Option<Uuid>,
    },
    #[serde(rename = "e")]
    Event(EventFrame),
    #[serde(rename = "gap")]
    Gap {
        s: Uuid,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ep: Option<Uuid>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        c: Option<Uuid>,
    },
    #[serde(rename = "resync")]
    Resync,
    #[serde(rename = "dm")]
    Dm { c: Uuid },
    #[serde(rename = "sig")]
    Sig(SigFrame),
    #[serde(rename = "err")]
    Err {
        e: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        s: Option<Uuid>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        c: Option<Uuid>,
    },
    #[serde(rename = "p")]
    Presence {
        s: Uuid,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        u: Option<Uuid>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        st: Option<PresenceStatus>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        snap: Option<Vec<PresenceEntry>>,
    },
    #[serde(rename = "y")]
    Typing { s: Uuid, c: Uuid, u: Uuid, on: bool },
    #[serde(rename = "ns")]
    NotificationSettings,
    #[serde(rename = "rd")]
    Read { d: Vec<ReadSync> },
    #[serde(other)]
    Unknown,
}

/// A sequenced chat event (`op: "e"`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EventFrame {
    pub t: EventKind,
    pub s: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub c: Option<Uuid>,
    pub n: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ep: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub i: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub d: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r: Option<i64>,
}

impl EventFrame {
    pub fn topic(&self) -> Topic {
        Topic {
            s: self.s,
            c: self.c,
        }
    }

    /// The message of a create/edit event; `r` overrides `d.revision`.
    pub fn message(&self) -> Option<Message> {
        let mut message: Message = serde_json::from_value(self.d.clone()?).ok()?;
        if let Some(r) = self.r {
            message.revision = r;
        }
        Some(message)
    }
}

/// Voice presence (`op: "sig"`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SigFrame {
    pub t: SigKind,
    pub s: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub c: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub u: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub k: Option<TrackKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub m: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub d: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snap: Option<Vec<VoiceEntry>>,
    /// Someone who is there already, replayed to a new occupant: no news.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub replay: bool,
}

/// A gateway topic: a server (`c: None`), a channel, or a DM (`s == c`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Topic {
    pub s: Uuid,
    pub c: Option<Uuid>,
}

impl Topic {
    pub fn server(s: Uuid) -> Self {
        Self { s, c: None }
    }

    pub fn channel(s: Uuid, c: Uuid) -> Self {
        Self { s, c: Some(c) }
    }

    pub fn dm(id: Uuid) -> Self {
        Self { s: id, c: Some(id) }
    }

    /// The web's map key: `c:<channel>` or `s:<server>`.
    pub fn key(&self) -> String {
        match self.c {
            Some(c) => format!("c:{c}"),
            None => format!("s:{}", self.s),
        }
    }
}
