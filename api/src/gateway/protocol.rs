//! Compact JSON frames for the native WS gateway.
//!
//! Short field names, no envelope beyond `op`. Chat events are `op: "e"`
//! with `t` = create/edit/delete. Signaling (issue 10) must use a later,
//! separate `op` — do not mix it into this stream.

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
    #[serde(rename = "err")]
    Err {
        e: &'static str,
        #[serde(skip_serializing_if = "Option::is_none")]
        s: Option<Uuid>,
        #[serde(skip_serializing_if = "Option::is_none")]
        c: Option<Uuid>,
    },
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

    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
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
}
