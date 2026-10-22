//! System messages (issue #314): rows the server writes on its own, stored
//! as a kind plus parameters (`system_kind`, `system_params`) instead of a
//! finished sentence. Clients render the text in the reader's language
//! (`web/src/messages/system.ts`); a push renders it per recipient
//! (`crate::i18n`). The author is the person the message is about, and
//! `content` stays empty.
//!
//! Kinds (`system_kind`, constrained in migration 0026):
//! - `live_started`: the author went live in a voice channel.
//!   Params: `voice_channel_id`, and `voice_name` as it was then, for when the
//!   channel is gone or renamed.

use serde::Serialize;
use serde_json::json;
use uuid::Uuid;

pub const LIVE_STARTED: &str = "live_started";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SystemMessage {
    pub kind: String,
    pub params: serde_json::Value,
}

impl SystemMessage {
    pub fn live_started(voice_channel_id: Uuid, voice_name: &str) -> Self {
        Self {
            kind: LIVE_STARTED.into(),
            params: json!({
                "voice_channel_id": voice_channel_id,
                "voice_name": voice_name,
            }),
        }
    }

    /// From the row's two columns; both are set or neither (migration 0026).
    pub(crate) fn from_columns(
        kind: Option<String>,
        params: Option<sqlx::types::Json<serde_json::Value>>,
    ) -> Option<Self> {
        Some(Self {
            kind: kind?,
            params: params.map(|p| p.0).unwrap_or_else(|| json!({})),
        })
    }

    /// A string parameter, empty when absent.
    pub fn param(&self, name: &str) -> &str {
        self.params
            .get(name)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_started_names_the_voice_channel() {
        let id = Uuid::new_v4();
        let system = SystemMessage::live_started(id, "Lounge");
        assert_eq!(
            serde_json::to_value(&system).unwrap(),
            json!({
                "kind": "live_started",
                "params": { "voice_channel_id": id, "voice_name": "Lounge" },
            })
        );
        assert_eq!(system.param("voice_name"), "Lounge");
        assert_eq!(system.param("missing"), "");
    }

    #[test]
    fn columns_without_a_kind_are_no_system_message() {
        assert_eq!(SystemMessage::from_columns(None, None), None);
        let system = SystemMessage::from_columns(Some(LIVE_STARTED.into()), None).unwrap();
        assert_eq!(system.params, json!({}));
    }
}
