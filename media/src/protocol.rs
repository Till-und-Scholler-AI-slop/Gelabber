//! Own mediasoup control protocol. No product SDP or ICE-candidate messages.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

/// v3 belongs to the retired layer prototype; v4 is the mediasoup contract.
pub const MEDIA_PROTOCOL_VERSION: u8 = 4;
// One announcement plus latest state/layers per allowed consumer, with RPC headroom.
pub const OUTBOUND_CAPACITY: usize = 3 * 1024 + 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SourceKind {
    #[serde(rename = "a")]
    Mic,
    #[serde(rename = "v")]
    Camera,
    #[serde(rename = "s")]
    Screen,
    #[serde(rename = "l")]
    Live,
    #[serde(rename = "sa")]
    ScreenAudio,
    #[serde(rename = "la")]
    LiveAudio,
}
impl SourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mic => "a",
            Self::Camera => "v",
            Self::Screen => "s",
            Self::Live => "l",
            Self::ScreenAudio => "sa",
            Self::LiveAudio => "la",
        }
    }
    pub fn is_video(self) -> bool {
        matches!(self, Self::Camera | Self::Screen | Self::Live)
    }
    pub fn parent(self) -> Option<Self> {
        match self {
            Self::ScreenAudio => Some(Self::Screen),
            Self::LiveAudio => Some(Self::Live),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WatchKind {
    #[serde(rename = "s")]
    Screen,
    #[serde(rename = "l")]
    Live,
}
impl WatchKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Screen => "s",
            Self::Live => "l",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TransportDirection {
    Send,
    Recv,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(tag = "op", deny_unknown_fields)]
pub enum ClientFrame {
    #[serde(rename = "j")]
    Join {
        #[serde(default)]
        id: u32,
        #[serde(default)]
        tk: String,
        #[serde(default)]
        w: Option<Uuid>,
        #[serde(default)]
        v: u8,
    },
    #[serde(rename = "capabilities")]
    Capabilities { id: u32, rtp: Value },
    #[serde(rename = "transport")]
    CreateTransport {
        id: u32,
        direction: TransportDirection,
    },
    #[serde(rename = "connect")]
    ConnectTransport {
        id: u32,
        #[serde(rename = "transportId")]
        transport_id: String,
        dtls: Value,
    },
    #[serde(rename = "restartIce")]
    RestartIce {
        id: u32,
        #[serde(rename = "transportId")]
        transport_id: String,
    },
    #[serde(rename = "closeTransport")]
    CloseTransport {
        id: u32,
        #[serde(rename = "transportId")]
        transport_id: String,
    },
    #[serde(rename = "produce")]
    Produce {
        id: u32,
        k: SourceKind,
        rtp: Value,
        epoch: Uuid,
        #[serde(default)]
        parent: Option<String>,
        #[serde(default)]
        lc: Option<Uuid>,
        #[serde(default, rename = "expectedOldProducerId")]
        expected_old_producer_id: Option<String>,
        #[serde(default)]
        height: u16,
        #[serde(default)]
        paused: bool,
    },
    #[serde(rename = "pauseProducer")]
    PauseProducer {
        id: u32,
        #[serde(rename = "producerId")]
        producer_id: String,
    },
    #[serde(rename = "resumeProducer")]
    ResumeProducer {
        id: u32,
        #[serde(rename = "producerId")]
        producer_id: String,
    },
    #[serde(rename = "closeProducer")]
    CloseProducer {
        id: u32,
        #[serde(rename = "producerId")]
        producer_id: String,
    },
    #[serde(rename = "consumerReady")]
    ConsumerReady {
        id: u32,
        #[serde(rename = "consumerId")]
        consumer_id: String,
        generation: Uuid,
    },
    #[serde(rename = "consumerFailed")]
    ConsumerFailed {
        id: u32,
        #[serde(rename = "consumerId")]
        consumer_id: String,
        generation: Uuid,
    },
    #[serde(rename = "w")]
    Watch {
        id: u32,
        u: Uuid,
        k: WatchKind,
        on: bool,
    },
    #[serde(rename = "q")]
    ViewerLayer {
        id: u32,
        #[serde(rename = "consumerId")]
        consumer_id: String,
        generation: Uuid,
        h: u16,
        congested: bool,
    },
    #[serde(rename = "l")]
    Leave { id: u32 },
}
impl ClientFrame {
    pub fn id(&self) -> u32 {
        match self {
            Self::Join { id, .. }
            | Self::Capabilities { id, .. }
            | Self::CreateTransport { id, .. }
            | Self::ConnectTransport { id, .. }
            | Self::RestartIce { id, .. }
            | Self::CloseTransport { id, .. }
            | Self::Produce { id, .. }
            | Self::PauseProducer { id, .. }
            | Self::ResumeProducer { id, .. }
            | Self::CloseProducer { id, .. }
            | Self::ConsumerReady { id, .. }
            | Self::ConsumerFailed { id, .. }
            | Self::Watch { id, .. }
            | Self::ViewerLayer { id, .. }
            | Self::Leave { id } => *id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op")]
pub enum ServerFrame {
    #[serde(rename = "result")]
    Result { id: u32, data: Value },
    #[serde(rename = "err")]
    Err {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<u32>,
        e: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lc: Option<Uuid>,
    },
    #[serde(rename = "consumer")]
    Consumer {
        #[serde(rename = "consumerId")]
        consumer_id: String,
        #[serde(rename = "producerId")]
        producer_id: String,
        owner: Uuid,
        k: SourceKind,
        epoch: Uuid,
        generation: Uuid,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent: Option<String>,
        kind: String,
        #[serde(rename = "rtpParameters")]
        rtp_parameters: Value,
        #[serde(default)]
        paused: bool,
    },
    #[serde(rename = "consumerClosed")]
    ConsumerClosed {
        #[serde(rename = "consumerId")]
        consumer_id: String,
        generation: Uuid,
    },
    #[serde(rename = "consumerState")]
    ConsumerState {
        #[serde(rename = "consumerId")]
        consumer_id: String,
        generation: Uuid,
        paused: bool,
    },
    #[serde(rename = "producerClosed")]
    ProducerClosed {
        #[serde(rename = "producerId")]
        producer_id: String,
        epoch: Uuid,
    },
    #[serde(rename = "layers")]
    Layers {
        #[serde(rename = "consumerId")]
        consumer_id: String,
        generation: Uuid,
        #[serde(rename = "spatialLayer")]
        spatial_layer: Option<u8>,
        #[serde(rename = "temporalLayer")]
        temporal_layer: Option<u8>,
    },
}
impl ServerFrame {
    pub fn error(code: &'static str) -> Self {
        Self::Err {
            id: None,
            e: code.into(),
            lc: None,
        }
    }
    pub fn request_error(id: u32, code: &'static str) -> Self {
        Self::Err {
            id: Some(id),
            e: code.into(),
            lc: None,
        }
    }
    pub fn live_withdrawn(nonce: Uuid) -> Self {
        Self::Err {
            id: None,
            e: "forbidden".into(),
            lc: Some(nonce),
        }
    }
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_old_sdp_and_unknown_fields() {
        for raw in [
            r#"{"op":"o","sdp":"v=0"}"#,
            r#"{"op":"a","sdp":"v=0"}"#,
            r#"{"op":"i","ice":"candidate"}"#,
            r#"{"op":"l","id":1,"user":"other"}"#,
        ] {
            assert!(serde_json::from_str::<ClientFrame>(raw).is_err());
        }
    }
    #[test]
    fn source_and_generation_are_explicit() {
        let epoch = Uuid::new_v4();
        let raw = serde_json::json!({"op":"produce","id":3,"k":"sa","rtp":{},"epoch":epoch,"parent":"parent-producer"});
        let frame: ClientFrame = serde_json::from_value(raw).unwrap();
        assert!(matches!(
            &frame,
            ClientFrame::Produce {
                k: SourceKind::ScreenAudio,
                parent: Some(_),
                ..
            }
        ));
        assert_eq!(frame.id(), 3);
    }
    #[test]
    fn old_join_can_be_rejected_before_consuming_ticket() {
        let old: ClientFrame =
            serde_json::from_str(r#"{"op":"j","tk":"abcdefghijkl","v":2}"#).unwrap();
        assert!(matches!(old, ClientFrame::Join { v: 2, id: 0, .. }));
        assert_eq!(MEDIA_PROTOCOL_VERSION, 4);
    }
    #[test]
    fn errors_correlate_request_or_retired_live_claim() {
        let nonce = Uuid::new_v4();
        let error = serde_json::to_value(ServerFrame::live_withdrawn(nonce)).unwrap();
        assert_eq!(error["lc"], nonce.to_string());
        assert!(error.get("id").is_none());
        let error = serde_json::to_value(ServerFrame::request_error(7, "forbidden")).unwrap();
        assert_eq!(error["id"], 7);
        assert!(error.get("lc").is_none());
    }
}
