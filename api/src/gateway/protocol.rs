//! Compact JSON frames for the native WS gateway.
//!
//! Short field names, no envelope beyond `op`. Chat events are `op: "e"`
//! with `t` = create/edit/delete. Voice signaling is `op: "sig"` — live
//! only, no seq, never mixed into the chat stream.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

/// Redis Pub/Sub channel / replay-key prefix. Keep it short.
pub const REDIS_PREFIX: &str = "gb:";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Topic {
    Server(Uuid),
    Channel(Uuid),
}

impl Topic {
    pub fn redis_channel(self) -> String {
        match self {
            Self::Server(id) => format!("{REDIS_PREFIX}s:{id}"),
            Self::Channel(id) => format!("{REDIS_PREFIX}c:{id}"),
        }
    }

    pub fn seq_key(self) -> String {
        match self {
            Self::Server(id) => format!("{REDIS_PREFIX}n:s:{id}"),
            Self::Channel(id) => format!("{REDIS_PREFIX}n:c:{id}"),
        }
    }

    pub fn log_key(self) -> String {
        match self {
            Self::Server(id) => format!("{REDIS_PREFIX}l:s:{id}"),
            Self::Channel(id) => format!("{REDIS_PREFIX}l:c:{id}"),
        }
    }

    pub fn from_redis_channel(name: &str) -> Option<Self> {
        let rest = name.strip_prefix(REDIS_PREFIX)?;
        let (kind, id) = rest.split_once(':')?;
        let id = Uuid::parse_str(id).ok()?;
        match kind {
            "s" => Some(Self::Server(id)),
            "c" => Some(Self::Channel(id)),
            _ => None,
        }
    }

    pub fn of(server_id: Uuid, channel_id: Option<Uuid>) -> Self {
        match channel_id {
            Some(id) => Self::Channel(id),
            None => Self::Server(server_id),
        }
    }
}

/// Ephemeral Pub/Sub (issue 8). Not sequenced, not in the replay log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveTopic {
    Presence(Uuid),
    Typing(Uuid),
}

impl LiveTopic {
    pub fn redis_channel(self) -> String {
        match self {
            Self::Presence(id) => format!("{REDIS_PREFIX}p:{id}"),
            Self::Typing(id) => format!("{REDIS_PREFIX}y:{id}"),
        }
    }

    pub fn from_redis_channel(name: &str) -> Option<Self> {
        let rest = name.strip_prefix(REDIS_PREFIX)?;
        let (kind, id) = rest.split_once(':')?;
        let id = Uuid::parse_str(id).ok()?;
        match kind {
            "p" => Some(Self::Presence(id)),
            "y" => Some(Self::Typing(id)),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PresenceStatus {
    #[serde(rename = "o")]
    Online,
    #[serde(rename = "i")]
    Idle,
    #[serde(rename = "x")]
    Offline,
}

impl PresenceStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Online => "o",
            Self::Idle => "i",
            Self::Offline => "x",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "o" => Some(Self::Online),
            "i" => Some(Self::Idle),
            "x" => Some(Self::Offline),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PresenceEntry {
    pub u: Uuid,
    pub st: PresenceStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EventKind {
    /// Create.
    C,
    /// Edit (delta in `d`).
    E,
    /// Delete.
    D,
}

impl EventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::C => "c",
            Self::E => "e",
            Self::D => "d",
        }
    }
}

/// Voice signaling kind (`op: "sig"`). Short letters, not chat `t`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SigKind {
    /// Join a voice channel.
    J,
    /// Leave.
    L,
    /// SDP offer (client → room / future SFU).
    O,
    /// SDP answer.
    A,
    /// Trickle ICE candidate.
    I,
    /// Publish a track.
    P,
    /// Unpublish a track.
    U,
    /// Mute (session-local; `on` is the new value).
    M,
    /// Deafen (session-local; `on` is the new value).
    D,
    /// Occupancy snapshot after a server subscribe (`snap`).
    R,
}

impl SigKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::J => "j",
            Self::L => "l",
            Self::O => "o",
            Self::A => "a",
            Self::I => "i",
            Self::P => "p",
            Self::U => "u",
            Self::M => "m",
            Self::D => "d",
            Self::R => "r",
        }
    }

    /// SDP / ICE stay in the room. Join/leave/mute/deafen/pub fan out to
    /// everyone watching the server so the member list can show voice state.
    pub fn room_only(self) -> bool {
        matches!(self, Self::O | Self::A | Self::I)
    }
}

/// Audio / camera / screen / live track on pub/unpub.
/// Camera (`v`) and screen (`s`) are in-channel (issue 13).
/// Go Live (`l`) is one track per voice channel (issue 14).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrackKind {
    A,
    V,
    S,
    L,
}

impl TrackKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::A => "a",
            Self::V => "v",
            Self::S => "s",
            Self::L => "l",
        }
    }
}

/// One occupant in a voice-state snapshot (`t: "r"`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VoiceEntry {
    pub u: Uuid,
    pub c: Uuid,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub m: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub d: bool,
    /// Occupant holds the channel's Go Live track.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub l: bool,
}

/// Compact signaling payload. Redis Pub/Sub carries this as `{"op":"sig",…}`.
/// No `n` — ICE and SDP go stale; there is no replay.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SigEvent {
    pub t: SigKind,
    pub s: Uuid,
    pub c: Uuid,
    pub u: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sdp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ice: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub k: Option<TrackKind>,
    /// Mute / deafen: the new value. Omitted on join/leave/media.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on: Option<bool>,
    /// Join snapshot: occupant is muted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub m: Option<bool>,
    /// Join snapshot: occupant is deafened.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub d: Option<bool>,
}

impl SigEvent {
    fn base(kind: SigKind, server_id: Uuid, channel_id: Uuid, user_id: Uuid) -> Self {
        Self {
            t: kind,
            s: server_id,
            c: channel_id,
            u: user_id,
            sdp: None,
            ice: None,
            mid: None,
            k: None,
            on: None,
            m: None,
            d: None,
        }
    }

    pub fn join(server_id: Uuid, channel_id: Uuid, user_id: Uuid) -> Self {
        Self::join_state(server_id, channel_id, user_id, false, false)
    }

    pub fn join_state(
        server_id: Uuid,
        channel_id: Uuid,
        user_id: Uuid,
        muted: bool,
        deafened: bool,
    ) -> Self {
        let mut event = Self::base(SigKind::J, server_id, channel_id, user_id);
        event.m = muted.then_some(true);
        event.d = deafened.then_some(true);
        event
    }

    pub fn leave(server_id: Uuid, channel_id: Uuid, user_id: Uuid) -> Self {
        Self::base(SigKind::L, server_id, channel_id, user_id)
    }

    pub fn published(server_id: Uuid, channel_id: Uuid, user_id: Uuid, kind: TrackKind) -> Self {
        let mut event = Self::base(SigKind::P, server_id, channel_id, user_id);
        event.k = Some(kind);
        event
    }

    pub fn unpublished(server_id: Uuid, channel_id: Uuid, user_id: Uuid, kind: TrackKind) -> Self {
        let mut event = Self::base(SigKind::U, server_id, channel_id, user_id);
        event.k = Some(kind);
        event
    }

    pub fn muted(server_id: Uuid, channel_id: Uuid, user_id: Uuid, on: bool) -> Self {
        let mut event = Self::base(SigKind::M, server_id, channel_id, user_id);
        event.on = Some(on);
        event
    }

    pub fn deafened(server_id: Uuid, channel_id: Uuid, user_id: Uuid, on: bool) -> Self {
        let mut event = Self::base(SigKind::D, server_id, channel_id, user_id);
        event.on = Some(on);
        event
    }
}

/// One compact create/edit/delete event. This is what Redis stores and
/// what the client sees as `{"op":"e", …}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub t: EventKind,
    pub s: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub c: Option<Uuid>,
    pub n: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub i: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub d: Option<Value>,
}

impl Event {
    pub fn topic(&self) -> Topic {
        Topic::of(self.s, self.c)
    }
}

/// Draft an event before a sequence number is assigned. Issue 5 fills this
/// after a successful REST write; the gateway owns seq + fan-out.
#[derive(Debug, Clone)]
pub struct EventDraft {
    pub kind: EventKind,
    pub server_id: Uuid,
    pub channel_id: Option<Uuid>,
    pub entity_id: Option<Uuid>,
    pub delta: Option<Value>,
}

impl EventDraft {
    pub fn topic(&self) -> Topic {
        Topic::of(self.server_id, self.channel_id)
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct ClientFrame {
    pub op: String,
    #[serde(default)]
    pub s: Option<Uuid>,
    #[serde(default)]
    pub c: Option<Uuid>,
    #[serde(default)]
    pub n: Option<u64>,
    /// Signaling kind when `op` is `sig`.
    #[serde(default)]
    pub t: Option<SigKind>,
    #[serde(default)]
    pub sdp: Option<String>,
    #[serde(default)]
    pub ice: Option<String>,
    #[serde(default)]
    pub mid: Option<String>,
    #[serde(default)]
    pub k: Option<TrackKind>,
    /// Presence: `o` / `i`. Absent on a `p` frame means "I am active".
    #[serde(default)]
    pub st: Option<PresenceStatus>,
    /// Typing start/stop, and mute/deafen (`t: "m"|"d"`).
    #[serde(default)]
    pub on: Option<bool>,
}

impl ClientFrame {
    pub fn is_heartbeat(&self) -> bool {
        self.op == "h"
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op")]
pub enum ServerFrame {
    #[serde(rename = "h")]
    Heartbeat,
    #[serde(rename = "ok")]
    Ok {
        s: Uuid,
        #[serde(skip_serializing_if = "Option::is_none")]
        c: Option<Uuid>,
        n: u64,
    },
    #[serde(rename = "e")]
    Event {
        t: EventKind,
        s: Uuid,
        #[serde(skip_serializing_if = "Option::is_none")]
        c: Option<Uuid>,
        n: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        i: Option<Uuid>,
        #[serde(skip_serializing_if = "Option::is_none")]
        d: Option<Value>,
    },
    #[serde(rename = "gap")]
    Gap {
        s: Uuid,
        #[serde(skip_serializing_if = "Option::is_none")]
        c: Option<Uuid>,
    },
    /// Voice signaling. Separate `op` so chat clients never see SDP/ICE
    /// on the sequenced event stream.
    #[serde(rename = "sig")]
    Sig {
        t: SigKind,
        s: Uuid,
        #[serde(skip_serializing_if = "Option::is_none")]
        c: Option<Uuid>,
        #[serde(skip_serializing_if = "Option::is_none")]
        u: Option<Uuid>,
        #[serde(skip_serializing_if = "Option::is_none")]
        sdp: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        ice: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        mid: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        k: Option<TrackKind>,
        #[serde(skip_serializing_if = "Option::is_none")]
        on: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        m: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        d: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        snap: Option<Vec<VoiceEntry>>,
    },
    #[serde(rename = "err")]
    Err {
        e: &'static str,
        #[serde(skip_serializing_if = "Option::is_none")]
        s: Option<Uuid>,
        #[serde(skip_serializing_if = "Option::is_none")]
        c: Option<Uuid>,
    },
    /// Presence update or snapshot. Never sequenced — not a chat event.
    #[serde(rename = "p")]
    Presence {
        s: Uuid,
        #[serde(skip_serializing_if = "Option::is_none")]
        u: Option<Uuid>,
        #[serde(skip_serializing_if = "Option::is_none")]
        st: Option<PresenceStatus>,
        #[serde(skip_serializing_if = "Option::is_none")]
        snap: Option<Vec<PresenceEntry>>,
    },
    /// Typing start / stop in a channel.
    #[serde(rename = "y")]
    Typing { s: Uuid, c: Uuid, u: Uuid, on: bool },
}

impl ServerFrame {
    pub fn event(event: Event) -> Self {
        Self::Event {
            t: event.t,
            s: event.s,
            c: event.c,
            n: event.n,
            i: event.i,
            d: event.d,
        }
    }

    pub fn subscribed(server_id: Uuid, channel_id: Option<Uuid>, n: u64) -> Self {
        Self::Ok {
            s: server_id,
            c: channel_id,
            n,
        }
    }

    pub fn gap(server_id: Uuid, channel_id: Option<Uuid>) -> Self {
        Self::Gap {
            s: server_id,
            c: channel_id,
        }
    }

    pub fn error(code: &'static str, server_id: Option<Uuid>, channel_id: Option<Uuid>) -> Self {
        Self::Err {
            e: code,
            s: server_id,
            c: channel_id,
        }
    }

    pub fn sig(event: SigEvent) -> Self {
        Self::Sig {
            t: event.t,
            s: event.s,
            c: Some(event.c),
            u: Some(event.u),
            sdp: event.sdp,
            ice: event.ice,
            mid: event.mid,
            k: event.k,
            on: event.on,
            m: event.m,
            d: event.d,
            snap: None,
        }
    }

    pub fn voice_snap(server_id: Uuid, snap: Vec<VoiceEntry>) -> Self {
        Self::Sig {
            t: SigKind::R,
            s: server_id,
            c: None,
            u: None,
            sdp: None,
            ice: None,
            mid: None,
            k: None,
            on: None,
            m: None,
            d: None,
            snap: Some(snap),
        }
    }

    pub fn presence(server_id: Uuid, user_id: Uuid, status: PresenceStatus) -> Self {
        Self::Presence {
            s: server_id,
            u: Some(user_id),
            st: Some(status),
            snap: None,
        }
    }

    pub fn presence_snap(server_id: Uuid, snap: Vec<PresenceEntry>) -> Self {
        Self::Presence {
            s: server_id,
            u: None,
            st: None,
            snap: Some(snap),
        }
    }

    pub fn typing(server_id: Uuid, channel_id: Uuid, user_id: Uuid, on: bool) -> Self {
        Self::Typing {
            s: server_id,
            c: channel_id,
            u: user_id,
            on,
        }
    }

    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }
}

/// Owned presence/typing payload on Redis Pub/Sub. Separate from
/// [`ServerFrame`] so we do not deserialize the `'static` `err` variant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op")]
pub enum LiveFrame {
    #[serde(rename = "p")]
    Presence {
        s: Uuid,
        #[serde(skip_serializing_if = "Option::is_none")]
        u: Option<Uuid>,
        #[serde(skip_serializing_if = "Option::is_none")]
        st: Option<PresenceStatus>,
        #[serde(skip_serializing_if = "Option::is_none")]
        snap: Option<Vec<PresenceEntry>>,
    },
    #[serde(rename = "y")]
    Typing { s: Uuid, c: Uuid, u: Uuid, on: bool },
}

impl From<LiveFrame> for ServerFrame {
    fn from(frame: LiveFrame) -> Self {
        match frame {
            LiveFrame::Presence { s, u, st, snap } => Self::Presence { s, u, st, snap },
            LiveFrame::Typing { s, c, u, on } => Self::Typing { s, c, u, on },
        }
    }
}

impl From<Event> for ServerFrame {
    fn from(event: Event) -> Self {
        Self::event(event)
    }
}

/// What to send a reconnecting client that last saw `client_n`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatchUp<T> {
    /// Already at (or past) the head — nothing to replay.
    None,
    /// Contiguous events `client_n+1 ..= current`.
    Replay(Vec<T>),
    /// The bounded buffer cannot fill the hole; client must refetch REST.
    Gap,
}

/// Decides replay vs gap. `events` is the Redis log (any order). `current`
/// is the topic's latest seq. `client_n == None` means a fresh subscribe:
/// do not dump history (REST owns that).
pub fn plan_catch_up<T: Clone>(
    client_n: Option<u64>,
    current: u64,
    events: &[T],
    seq_of: impl Fn(&T) -> u64,
) -> CatchUp<T> {
    let Some(client_n) = client_n else {
        return CatchUp::None;
    };
    if current == 0 || client_n >= current {
        return CatchUp::None;
    }

    let mut replay: Vec<T> = events
        .iter()
        .filter(|event| seq_of(event) > client_n)
        .cloned()
        .collect();
    replay.sort_by_key(|event| seq_of(event));

    let mut expect = client_n + 1;
    for event in &replay {
        if seq_of(event) != expect {
            return CatchUp::Gap;
        }
        expect += 1;
    }
    if expect != current + 1 {
        return CatchUp::Gap;
    }
    CatchUp::Replay(replay)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(n: &u64) -> u64 {
        *n
    }

    #[test]
    fn topic_keys_round_trip() {
        let id = Uuid::from_u128(1);
        let topic = Topic::Channel(id);
        assert_eq!(
            Topic::from_redis_channel(&topic.redis_channel()),
            Some(topic)
        );
        assert!(topic.seq_key().starts_with("gb:n:c:"));
        assert!(topic.log_key().starts_with("gb:l:c:"));
        let live = LiveTopic::Presence(id);
        assert_eq!(
            LiveTopic::from_redis_channel(&live.redis_channel()),
            Some(live)
        );
        assert!(Topic::from_redis_channel(&live.redis_channel()).is_none());
    }

    #[test]
    fn event_frame_is_compact() {
        let event = Event {
            t: EventKind::C,
            s: Uuid::from_u128(1),
            c: Some(Uuid::from_u128(2)),
            n: 7,
            i: Some(Uuid::from_u128(3)),
            d: Some(serde_json::json!({"b":"hi"})),
        };
        let json = ServerFrame::event(event).to_json().unwrap();
        assert!(json.starts_with(r#"{"op":"e","t":"c""#));
        assert!(!json.contains("kind"));
        assert!(!json.contains("server_id"));
        assert!(!json.contains("payload"));
    }

    #[test]
    fn presence_and_typing_frames_are_compact_and_unsequenced() {
        let json = ServerFrame::presence(
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            PresenceStatus::Online,
        )
        .to_json()
        .unwrap();
        assert_eq!(
            json,
            format!(
                r#"{{"op":"p","s":"{}","u":"{}","st":"o"}}"#,
                Uuid::from_u128(1),
                Uuid::from_u128(2)
            )
        );
        let json = ServerFrame::typing(
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            Uuid::from_u128(3),
            true,
        )
        .to_json()
        .unwrap();
        assert!(json.starts_with(r#"{"op":"y""#));
        assert!(!json.contains("\"n\""));
        assert!(!json.contains("seq"));
    }

    #[test]
    fn omits_absent_optional_fields() {
        let json = ServerFrame::Heartbeat.to_json().unwrap();
        assert_eq!(json, r#"{"op":"h"}"#);

        let json = ServerFrame::subscribed(Uuid::from_u128(1), None, 4)
            .to_json()
            .unwrap();
        assert_eq!(
            json,
            format!(r#"{{"op":"ok","s":"{}","n":4}}"#, Uuid::from_u128(1))
        );
    }

    #[test]
    fn catch_up_none_without_resume_or_when_current() {
        assert_eq!(plan_catch_up(None, 9, &[1, 2, 3], ev), CatchUp::None);
        assert_eq!(plan_catch_up(Some(9), 9, &[7, 8, 9], ev), CatchUp::None);
        assert_eq!(plan_catch_up(Some(0), 0, &[] as &[u64], ev), CatchUp::None);
    }

    #[test]
    fn catch_up_replays_contiguous_gap() {
        assert_eq!(
            plan_catch_up(Some(5), 8, &[8, 6, 7, 5], ev),
            CatchUp::Replay(vec![6, 7, 8])
        );
    }

    #[test]
    fn catch_up_gap_when_buffer_dropped_the_next_seq() {
        assert_eq!(plan_catch_up(Some(2), 8, &[6, 7, 8], ev), CatchUp::Gap);
        assert_eq!(plan_catch_up(Some(5), 8, &[6, 8], ev), CatchUp::Gap);
        assert_eq!(plan_catch_up(Some(5), 8, &[] as &[u64], ev), CatchUp::Gap);
    }

    #[test]
    fn client_frame_parses_subscribe_with_resume() {
        let frame: ClientFrame =
            serde_json::from_str(r#"{"op":"s","s":"00000000-0000-0000-0000-000000000001","c":"00000000-0000-0000-0000-000000000002","n":12}"#).unwrap();
        assert_eq!(frame.op, "s");
        assert_eq!(frame.n, Some(12));
        assert_eq!(frame.s, Some(Uuid::from_u128(1)));
        assert_eq!(frame.c, Some(Uuid::from_u128(2)));
    }

    #[test]
    fn sig_frame_is_compact_and_not_a_chat_event() {
        let json = ServerFrame::sig(SigEvent::join(
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            Uuid::from_u128(3),
        ))
        .to_json()
        .unwrap();
        assert!(json.starts_with(r#"{"op":"sig","t":"j""#));
        assert!(!json.contains("\"n\""));
        assert!(!json.contains("offer"));
        assert!(!json.contains("payload"));
        assert!(!json.contains(r#""op":"e""#));
    }

    #[test]
    fn mute_deafen_and_roster_frames_are_compact() {
        let json = ServerFrame::sig(SigEvent::muted(
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            Uuid::from_u128(3),
            true,
        ))
        .to_json()
        .unwrap();
        assert!(json.starts_with(r#"{"op":"sig","t":"m""#));
        assert!(json.contains(r#""on":true"#));
        assert!(!json.contains("mute"));

        let json = ServerFrame::voice_snap(
            Uuid::from_u128(1),
            vec![VoiceEntry {
                u: Uuid::from_u128(3),
                c: Uuid::from_u128(2),
                m: true,
                d: false,
                l: false,
            }],
        )
        .to_json()
        .unwrap();
        assert!(json.starts_with(r#"{"op":"sig","t":"r""#));
        assert!(json.contains(r#""m":true"#));
        assert!(!json.contains(r#""d""#));
        assert!(!json.contains("\"n\""));
    }

    #[test]
    fn camera_and_screen_pub_kinds_are_compact() {
        let json = ServerFrame::sig(SigEvent::published(
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            Uuid::from_u128(3),
            TrackKind::V,
        ))
        .to_json()
        .unwrap();
        assert!(json.contains(r#""t":"p""#));
        assert!(json.contains(r#""k":"v""#));
        let json = ServerFrame::sig(SigEvent::published(
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            Uuid::from_u128(3),
            TrackKind::S,
        ))
        .to_json()
        .unwrap();
        assert!(json.contains(r#""k":"s""#));
        assert!(!json.contains("livekit"));
        assert!(!json.contains("go_live"));
        let json = ServerFrame::sig(SigEvent::published(
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            Uuid::from_u128(3),
            TrackKind::L,
        ))
        .to_json()
        .unwrap();
        assert!(json.contains(r#""k":"l""#));
        assert!(!json.contains("livekit"));
        assert!(!json.contains("\"go_live\""));
    }
}
