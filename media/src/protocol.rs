//! Compact JSON for the **media** WebSocket. This is not the chat gateway:
//! no `op:"e"`, no seq, no session cookie. Join is a short ticket.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ClientFrame {
    pub op: String,
    #[serde(default)]
    pub tk: Option<String>,
    #[serde(default)]
    pub sdp: Option<String>,
    #[serde(default)]
    pub ice: Option<String>,
    #[serde(default)]
    pub mid: Option<String>,
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
}
