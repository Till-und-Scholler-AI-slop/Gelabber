//! Router codecs, producer parameter checks and the video codecs every
//! receiver in a room can decode.
use super::*;

impl Sfu {
    pub(super) fn announce_video_codecs(
        self: &Arc<Self>,
        changed: Option<(Vec<Arc<Peer>>, ServerFrame)>,
    ) {
        let Some((peers, frame)) = changed else {
            return;
        };
        for peer in peers {
            self.emit(&peer, frame.clone());
        }
    }
}

pub(super) fn validate_parameters(kind: SourceKind, rtp: &RtpParameters) -> Result<(), SfuError> {
    if rtp.codecs.is_empty()
        || rtp.codecs.len() > 16
        || rtp.encodings.len() > 2
        || rtp.header_extensions.len() > 32
    {
        return Err(SfuError::BadAnnounce);
    }
    let mime = rtp.codecs[0].mime_type().as_str().to_ascii_lowercase();
    if kind.is_video() {
        if !matches!(
            mime.as_str(),
            "video/vp8" | "video/vp9" | "video/h264" | "video/av1"
        ) {
            return Err(SfuError::UnsupportedCodec);
        }
        // mediasoup forwards VP9 and AV1 as one stream or SVC, never as
        // simulcast layers; its worker would refuse the producer later.
        if matches!(mime.as_str(), "video/vp9" | "video/av1") && rtp.encodings.len() > 1 {
            warn!(
                codec = mime.as_str(),
                encodings = rtp.encodings.len(),
                "refused a producer: this codec comes as one stream"
            );
            return Err(SfuError::UnsupportedCodec);
        }
    } else if mime != "audio/opus" || rtp.encodings.len() > 1 {
        return Err(SfuError::UnsupportedCodec);
    }
    Ok(())
}
/// The router's video codecs, in its order, that every one of `receivers`
/// can decode; all of them without receivers. H264 counts in the form
/// publishers send it: packetization mode 1, Constrained Baseline (profile
/// 0x42 with constraint_set1), the profile every WebRTC H264 decoder has
/// and the one mediasoup matches a 42e01f producer against.
pub(super) fn common_video_codecs<'a>(
    receivers: impl Iterator<Item = &'a RtpCapabilities>,
) -> Vec<String> {
    const ROUTER: [&str; 4] = ["video/vp8", "video/vp9", "video/h264", "video/av1"];
    let mut common: Vec<&str> = ROUTER.to_vec();
    for caps in receivers {
        let decodes: Vec<String> = caps
            .codecs
            .iter()
            .filter_map(|codec| serde_json::to_value(codec).ok())
            .filter(|codec| {
                let mime = codec["mimeType"].as_str().unwrap_or_default();
                if !mime.eq_ignore_ascii_case("video/h264") {
                    return true;
                }
                let parameters = &codec["parameters"];
                let mode = parameters["packetization-mode"].as_u64().unwrap_or(0);
                let profile = parameters["profile-level-id"].as_str().unwrap_or("42001f");
                let constrained = profile
                    .get(2..4)
                    .and_then(|iop| u8::from_str_radix(iop, 16).ok())
                    .is_some_and(|iop| iop & 0x40 != 0);
                mode == 1
                    && constrained
                    && profile
                        .get(..2)
                        .is_some_and(|p| p.eq_ignore_ascii_case("42"))
            })
            .filter_map(|codec| codec["mimeType"].as_str().map(str::to_ascii_lowercase))
            .collect();
        common.retain(|mime| decodes.iter().any(|d| d == mime));
    }
    common.into_iter().map(str::to_owned).collect()
}
pub(super) fn router_codecs() -> Result<Vec<RtpCodecCapability>, SfuError> {
    let feedback = json!([{"type":"nack"},{"type":"nack","parameter":"pli"},{"type":"ccm","parameter":"fir"},{"type":"goog-remb"},{"type":"transport-cc"}]);
    let mut codecs = vec![
        json!({"kind":"audio","mimeType":"audio/opus","clockRate":48000,"channels":2,"rtcpFeedback":[{"type":"nack"},{"type":"transport-cc"}]}),
        json!({"kind":"video","mimeType":"video/VP8","clockRate":90000,"rtcpFeedback":feedback}),
    ];
    for profile in [0, 1] {
        codecs.push(json!({"kind":"video","mimeType":"video/VP9","clockRate":90000,"parameters":{"profile-id":profile},"rtcpFeedback":feedback}));
    }
    for (profile, mode) in [
        ("42e01f", 1),
        ("42001f", 1),
        ("42e01f", 0),
        ("42001f", 0),
        ("640032", 1),
    ] {
        codecs.push(json!({"kind":"video","mimeType":"video/H264","clockRate":90000,"parameters":{"profile-level-id":profile,"packetization-mode":mode,"level-asymmetry-allowed":1},"rtcpFeedback":feedback}));
    }
    codecs.push(json!({"kind":"video","mimeType":"video/AV1","clockRate":90000,"parameters":{"profile":0},"rtcpFeedback":feedback}));
    serde_json::from_value(Value::Array(codecs)).map_err(SfuError::negotiation)
}
