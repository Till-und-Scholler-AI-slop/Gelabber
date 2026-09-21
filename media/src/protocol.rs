//! Compact JSON for the **media** WebSocket. This is not the chat gateway:
//! no `op:"e"`, no seq, no session cookie. Join is a short ticket.
//!
//! Inbound ops are an internally tagged enum. The JSON is the same short
//! keys as before. An unknown `op` fails to decode and the socket answers
//! `bad_request`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(tag = "op")]
pub enum ClientFrame {
    #[serde(rename = "j")]
    Join {
        #[serde(default)]
        tk: Option<String>,
    },
    #[serde(rename = "o")]
    Offer {
        #[serde(default)]
        sdp: Option<String>,
    },
    #[serde(rename = "a")]
    Answer {
        #[serde(default)]
        sdp: Option<String>,
    },
    #[serde(rename = "i")]
    Ice {
        #[serde(default)]
        ice: Option<String>,
        #[serde(default)]
        mid: Option<String>,
    },
    /// Next inbound track kind from this peer: `v` (camera), `s` (screen), `l` (live).
    #[serde(rename = "p")]
    Announce {
        #[serde(default)]
        k: Option<String>,
    },
    /// Subscriber could not answer the outstanding offer.
    #[serde(rename = "x")]
    Abort,
    /// Publisher offer failed before the announced track arrived.
    #[serde(rename = "u")]
    Retract {
        #[serde(default)]
        k: Option<String>,
    },
    #[serde(rename = "l")]
    Leave,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op")]
pub enum ServerFrame {
    #[serde(rename = "ok")]
    Ok { c: String, u: String },
    #[serde(rename = "o")]
    Offer { sdp: String },
    #[serde(rename = "a")]
    Answer { sdp: String },
    #[serde(rename = "i")]
    Ice {
        ice: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        mid: Option<String>,
    },
    #[serde(rename = "err")]
    Err { e: &'static str },
}

impl ServerFrame {
    pub fn error(code: &'static str) -> Self {
        Self::Err { e: code }
    }

    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_are_compact_and_not_chat() {
        let json = ServerFrame::Ok {
            c: "1".into(),
            u: "2".into(),
        }
        .to_json()
        .unwrap();
        assert!(json.starts_with(r#"{"op":"ok""#));
        assert!(!json.contains("\"n\""));
        assert!(!json.contains(r#""op":"e""#));
        assert!(!json.contains("livekit"));
    }

    #[test]
    fn pub_announce_is_compact() {
        let frame: ClientFrame = serde_json::from_str(r#"{"op":"p","k":"s"}"#).unwrap();
        assert_eq!(
            frame,
            ClientFrame::Announce {
                k: Some("s".into())
            }
        );
        let live: ClientFrame = serde_json::from_str(r#"{"op":"p","k":"l"}"#).unwrap();
        assert_eq!(
            live,
            ClientFrame::Announce {
                k: Some("l".into())
            }
        );
        let abort: ClientFrame = serde_json::from_str(r#"{"op":"x"}"#).unwrap();
        assert_eq!(abort, ClientFrame::Abort);
        let undo: ClientFrame = serde_json::from_str(r#"{"op":"u","k":"s"}"#).unwrap();
        assert_eq!(
            undo,
            ClientFrame::Retract {
                k: Some("s".into())
            }
        );
    }

    #[test]
    fn unknown_op_is_rejected() {
        assert!(serde_json::from_str::<ClientFrame>(r#"{"op":"mesh"}"#).is_err());
    }
}
