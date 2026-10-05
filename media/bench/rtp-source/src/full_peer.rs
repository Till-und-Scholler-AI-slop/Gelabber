//! Native peer0 instrument: ordinary WebRTC, three fixed sources and actual Opus decoding.
//! Controlled only over stdin. Still diagnostic until full N-peer adapters qualify.
mod archive;
mod audio;
mod clock;
mod pn_audio;
mod unique_json;
use archive::{Archive, Result};
use audio::{AudioArchive, Decoder};
use rtc::{
    interceptor::Registry,
    media_stream::MediaStreamTrack,
    peer_connection::configuration::{
        RTCConfigurationBuilder,
        interceptor_registry::{configure_nack, configure_rtcp_reports},
        media_engine::{MIME_TYPE_OPUS, MIME_TYPE_VP8, MediaEngine},
    },
    rtp_transceiver::rtp_sender::{
        RTCRtpCodec, RTCRtpCodecParameters, RTCRtpCodingParameters, RTCRtpEncodingParameters,
        RtpCodecKind,
    },
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::BufRead,
    net::Ipv4Addr,
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::{sync::watch, task::JoinHandle};
use webrtc::{
    media_stream::{
        track_local::{TrackLocal, static_rtp::TrackLocalStaticRTP},
        track_remote::{TrackRemote, TrackRemoteEvent},
    },
    peer_connection::{
        PeerConnection, PeerConnectionBuilder, PeerConnectionEventHandler, RTCIceGatheringState,
        RTCPeerConnectionState, RTCSessionDescription, StatsSelector,
    },
};

const MIC: u32 = 0x474d4943;
const SOURCE: u32 = 0x47534130;
type Reports = Arc<Mutex<BTreeMap<String, Value>>>;

#[derive(Default)]
struct RtpOrder {
    sequence: Option<u16>,
    media_timestamp: Option<u32>,
}
#[derive(Debug, PartialEq)]
enum PacketKind {
    Reordered,
    Padding,
    Media,
    EmptyWithoutPadding,
}
struct PacketProgress {
    kind: PacketKind,
    sequence_gaps: u16,
    timestamp_gap: bool,
}
impl RtpOrder {
    fn packet(
        &mut self,
        sequence: u16,
        timestamp: u32,
        padding: bool,
        empty: bool,
    ) -> PacketProgress {
        let step = self
            .sequence
            .map(|last| sequence.wrapping_sub(last))
            .unwrap_or(1);
        if step == 0 || step >= 32768 {
            return PacketProgress {
                kind: PacketKind::Reordered,
                sequence_gaps: 0,
                timestamp_gap: false,
            };
        }
        self.sequence = Some(sequence);
        let kind = if empty && padding {
            PacketKind::Padding
        } else if empty {
            PacketKind::EmptyWithoutPadding
        } else {
            PacketKind::Media
        };
        // RTP padding has its own sequence number, but no audio frame or media
        // timestamp advance. Empty unpadded packets remain decoder errors.
        let timestamp_gap = kind == PacketKind::Media
            && self
                .media_timestamp
                .is_some_and(|last| timestamp.wrapping_sub(last) != 960);
        if kind == PacketKind::Media {
            self.media_timestamp = Some(timestamp);
        }
        PacketProgress {
            kind,
            sequence_gaps: step - 1,
            timestamp_gap,
        }
    }
}

struct Handler {
    gathered: watch::Sender<bool>,
    state: watch::Sender<RTCPeerConnectionState>,
    received: Reports,
    tasks: Arc<Mutex<Vec<JoinHandle<()>>>>,
}
#[async_trait::async_trait]
impl PeerConnectionEventHandler for Handler {
    async fn on_ice_gathering_state_change(&self, state: RTCIceGatheringState) {
        if state == RTCIceGatheringState::Complete {
            self.gathered.send_replace(true);
        }
    }
    async fn on_connection_state_change(&self, state: RTCPeerConnectionState) {
        self.state.send_replace(state);
    }
    async fn on_track(&self, track: Arc<dyn TrackRemote>) {
        let report = self.received.clone();
        self.tasks.lock().unwrap().push(tokio::spawn(async move {
            if let Err(error) = receive(track, report.clone()).await {
                report.lock().unwrap().insert(
                    "receiver_error".to_owned(),
                    json!({"error":error.to_string()}),
                );
            }
        }));
    }
}

async fn receive(track: Arc<dyn TrackRemote>, reports: Reports) -> Result<()> {
    let mut decoders: BTreeMap<u32, (Decoder, RtpOrder)> = BTreeMap::new();
    while let Some(event) = track.poll().await {
        match event {
            TrackRemoteEvent::OnOpen(init) => {
                reports.lock().unwrap().entry(init.ssrc.to_string()).or_insert(json!({
                    "ssrc":init.ssrc,"track_id":init.track_id,"stream_ids":init.stream_ids,"source_name":null,
                    "rtp_packets_received":0,"padding_packets_received":0,"padding_packet_examples":[],
                    "packets_received":0,"payload_bytes_received":0,"decoded_samples":0,"sequence_gaps":0,
                    "reordered_or_duplicate_packets":0,"timestamp_gaps":0,"decode_errors":0,
                    "output":"actual libopus float32 PCM; no jitter-buffer/playout/acoustic latency claim"}));
            }
            TrackRemoteEvent::OnRtpPacket(packet) => {
                let ssrc = packet.header.ssrc;
                let codec = track
                    .codec(ssrc)
                    .await
                    .ok_or("missing native receive codec")?;
                if codec.mime_type.to_lowercase() != "audio/opus" || codec.clock_rate != 48_000 {
                    return Err("native peer only receives Opus audio".into());
                }
                if !decoders.contains_key(&ssrc) {
                    decoders.insert(ssrc, (Decoder::new()?, RtpOrder::default()));
                }
                let (decoder, order) = decoders.get_mut(&ssrc).unwrap();
                let mut report = reports.lock().unwrap();
                let edge = report
                    .get_mut(&ssrc.to_string())
                    .ok_or("RTP arrived before source identity event")?;
                edge["rtp_packets_received"] =
                    json!(edge["rtp_packets_received"].as_u64().unwrap() + 1);
                edge["codec"] = json!({"mimeType":codec.mime_type,"clockRate":codec.clock_rate,
                    "channels":codec.channels,"payloadType":packet.header.payload_type});
                let progress = order.packet(
                    packet.header.sequence_number,
                    packet.header.timestamp,
                    packet.header.padding,
                    packet.payload.is_empty(),
                );
                if progress.kind == PacketKind::Reordered {
                    edge["reordered_or_duplicate_packets"] =
                        json!(edge["reordered_or_duplicate_packets"].as_u64().unwrap() + 1);
                    continue; // Never feed a duplicate/reordered packet into a stateful decoder as new PCM.
                }
                edge["sequence_gaps"] = json!(
                    edge["sequence_gaps"].as_u64().unwrap() + u64::from(progress.sequence_gaps)
                );
                if progress.timestamp_gap {
                    edge["timestamp_gaps"] = json!(edge["timestamp_gaps"].as_u64().unwrap() + 1);
                }
                if progress.kind == PacketKind::Padding {
                    edge["padding_packets_received"] =
                        json!(edge["padding_packets_received"].as_u64().unwrap() + 1);
                    let examples = edge["padding_packet_examples"].as_array_mut().unwrap();
                    if examples.len() < 8 {
                        examples.push(json!({"sequence":packet.header.sequence_number,
                        "timestamp":packet.header.timestamp,"header_padding":packet.header.padding,"payload_bytes":packet.payload.len()}));
                    }
                    continue;
                }
                edge["packets_received"] = json!(edge["packets_received"].as_u64().unwrap() + 1);
                edge["payload_bytes_received"] = json!(
                    edge["payload_bytes_received"].as_u64().unwrap() + packet.payload.len() as u64
                );
                match decoder.decode(&packet.payload) {
                    Ok(decoded) => {
                        edge["decoded_samples"] = json!(
                            edge["decoded_samples"].as_u64().unwrap()
                                + decoded.samples.len() as u64
                        );
                        edge["encoded_channels"] = json!(decoded.encoded_channels);
                        let energy: f64 = decoded
                            .samples
                            .iter()
                            .map(|sample| f64::from(*sample).powi(2))
                            .sum();
                        edge["last_pcm_rms"] =
                            json!((energy / decoded.samples.len() as f64).sqrt());
                        edge["last_pcm_peak"] = json!(
                            decoded
                                .samples
                                .iter()
                                .map(|sample| sample.abs())
                                .fold(0f32, f32::max)
                        );
                        edge["last_decoded_mono_ns"] = json!(clock::monotonic_ns()?.to_string());
                        edge["last_rtp_timestamp"] = json!(packet.header.timestamp);
                    }
                    Err(error) => {
                        edge["decode_errors"] = json!(edge["decode_errors"].as_u64().unwrap() + 1);
                        edge["error"] = json!(error.to_string());
                        edge["last_invalid_packet"] = json!({"sequence":packet.header.sequence_number,
                            "timestamp":packet.header.timestamp,"header_padding":packet.header.padding,"payload_bytes":packet.payload.len()});
                    }
                }
            }
            TrackRemoteEvent::OnEnded | TrackRemoteEvent::OnError => break,
            _ => {}
        }
    }
    Ok(())
}

struct Peer {
    pc: Arc<dyn PeerConnection>,
    gathered: watch::Sender<bool>,
    state: watch::Sender<RTCPeerConnectionState>,
    tracks: Vec<(String, Arc<TrackLocalStaticRTP>)>,
    received: Reports,
    receiver_tasks: Arc<Mutex<Vec<JoinHandle<()>>>>,
    feedback_tasks: Vec<JoinHandle<()>>,
}
async fn negotiated_senders(peer: &Peer) -> Result<Vec<Value>> {
    let mut values = Vec::new();
    for transceiver in peer.pc.get_transceivers().await {
        if let Some(sender) = transceiver.sender().await? {
            let parameters = sender.get_parameters().await?;
            let track = sender.track().track().await;
            let codecs: Vec<Value> = parameters
                .rtp_parameters
                .codecs
                .iter()
                .map(|codec| {
                    json!({
                "mimeType":codec.rtp_codec.mime_type,"clockRate":codec.rtp_codec.clock_rate,
                "channels":codec.rtp_codec.channels,"payloadType":codec.payload_type,
                "sdpFmtpLine":codec.rtp_codec.sdp_fmtp_line})
                })
                .collect();
            let encodings: Vec<Value> = parameters
                .encodings
                .iter()
                .map(|encoding| {
                    json!({
                "ssrc":encoding.rtp_coding_parameters.ssrc,"active":encoding.active,
                "mimeType":encoding.codec.mime_type,"clockRate":encoding.codec.clock_rate,
                "sdpFmtpLine":encoding.codec.sdp_fmtp_line})
                })
                .collect();
            if !encodings.is_empty() {
                values.push(json!({"mid":transceiver.mid().await?,
                "track_id":track.track_id(),"stream_id":track.stream_id(),"codecs":codecs,"encodings":encodings,
                "basis":"actual RtpSender::get_parameters negotiated send_codecs and SSRC encodings"}));
            }
        }
    }
    Ok(values)
}
async fn peer(bind: Ipv4Addr, publish: bool) -> Result<Peer> {
    let mut media = MediaEngine::default();
    let video = RTCRtpCodec {
        mime_type: MIME_TYPE_VP8.to_owned(),
        clock_rate: 90_000,
        ..Default::default()
    };
    // RFC7587's RTP map remains opus/48000/2 for actual mono Opus packets.
    let opus = RTCRtpCodec {
        mime_type: MIME_TYPE_OPUS.to_owned(),
        clock_rate: 48_000,
        channels: 2,
        sdp_fmtp_line: "minptime=10;useinbandfec=1;usedtx=0".to_owned(),
        ..Default::default()
    };
    for (codec, pt, kind) in [
        (video.clone(), 96, RtpCodecKind::Video),
        (opus.clone(), 111, RtpCodecKind::Audio),
    ] {
        media.register_codec(
            RTCRtpCodecParameters {
                rtp_codec: codec,
                payload_type: pt,
                ..Default::default()
            },
            kind,
        )?;
    }
    let registry = configure_rtcp_reports(configure_nack(Registry::new(), &mut media));
    let (gathered, _) = watch::channel(false);
    let (state, _) = watch::channel(RTCPeerConnectionState::New);
    let received = Arc::new(Mutex::new(BTreeMap::new()));
    let receiver_tasks = Arc::new(Mutex::new(Vec::new()));
    let pc: Arc<dyn PeerConnection> = Arc::new(
        PeerConnectionBuilder::new()
            .with_configuration(RTCConfigurationBuilder::new().build())
            .with_media_engine(media)
            .with_interceptor_registry(registry)
            .with_handler(Arc::new(Handler {
                gathered: gathered.clone(),
                state: state.clone(),
                received: received.clone(),
                tasks: receiver_tasks.clone(),
            }))
            .with_udp_addrs(vec![format!("{bind}:0")])
            .build()
            .await?,
    );
    let mut tracks = Vec::new();
    let mut feedback_tasks = Vec::new();
    if publish {
        for (name, id, stream, kind, codec, ssrc) in [
            (
                "mic",
                "fixed-native-mic",
                "m",
                RtpCodecKind::Audio,
                opus.clone(),
                MIC,
            ),
            (
                "video",
                "fixed-native-video",
                "s",
                RtpCodecKind::Video,
                video,
                archive::SSRC,
            ),
            (
                "source",
                "fixed-native-source-audio",
                "s",
                RtpCodecKind::Audio,
                opus,
                SOURCE,
            ),
        ] {
            let track = Arc::new(TrackLocalStaticRTP::new(MediaStreamTrack::new(
                stream.to_owned(),
                id.to_owned(),
                name.to_owned(),
                kind,
                vec![RTCRtpEncodingParameters {
                    rtp_coding_parameters: RTCRtpCodingParameters {
                        ssrc: Some(ssrc),
                        ..Default::default()
                    },
                    codec,
                    ..Default::default()
                }],
            )));
            pc.add_track(track.clone() as Arc<dyn TrackLocal>).await?;
            let feedback_track = track.clone();
            feedback_tasks.push(tokio::spawn(async move {
                loop {
                    if feedback_track.poll().await.is_none() {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                }
            }));
            tracks.push((name.to_owned(), track));
        }
    }
    Ok(Peer {
        pc,
        gathered,
        state,
        tracks,
        received,
        receiver_tasks,
        feedback_tasks,
    })
}

async fn replay_audio(
    archive: Arc<AudioArchive>,
    track: Arc<TrackLocalStaticRTP>,
    anchor: clock::Anchor,
    seconds: u64,
    reports: Reports,
    ssrc: u32,
) -> Result<()> {
    let mut packets = 0u64;
    let mut max_late = 0u64;
    let key = archive.kind.clone();
    for cycle in 0..seconds / 10 {
        for index in 0..archive.packets.len() {
            let offset = AudioArchive::due(index, cycle);
            let due = anchor.instant + offset;
            tokio::time::sleep_until(due).await;
            let enqueue_before = clock::monotonic_ns()?;
            let late = tokio::time::Instant::now()
                .saturating_duration_since(due)
                .as_nanos() as u64;
            max_late = max_late.max(late);
            if late > 20_000_000 {
                return Err(format!("{} source missed 20ms packet deadline", key).into());
            }
            track.write_rtp(archive.packet(index, cycle, ssrc)).await?;
            let enqueue_after = clock::monotonic_ns()?;
            let enqueue_lateness =
                enqueue_after.saturating_sub(anchor.ns + offset.as_nanos() as u64);
            max_late = max_late.max(enqueue_lateness);
            if enqueue_lateness > 20_000_000 {
                return Err(format!("{} enqueue missed 20ms packet deadline", key).into());
            }
            packets += 1;
            reports.lock().unwrap().insert(key.clone(), json!({"running":true,"completed":false,"source_policy_valid":true,
                "source_policy_scope":"observed enqueue prefix; full packet count checked only on completion","ssrc":ssrc,
                "source_uid":if key=="mic" {0} else {64},"packets_enqueued":packets,"rtp_payload_bytes_enqueued":packets*320,
                "last_source_sample_ordinal":(packets-1)*960,"last_planned_mono_ns":(anchor.ns+offset.as_nanos() as u64).to_string(),
                "last_enqueue_before_ns":enqueue_before.to_string(),"last_enqueued_mono_ns":enqueue_after.to_string(),
                "last_enqueue_bracket_ns":enqueue_after.saturating_sub(enqueue_before),"max_schedule_lateness_ns":max_late,"timeline":anchor.evidence()}));
        }
    }
    tokio::time::sleep_until(anchor.instant + Duration::from_secs(seconds)).await;
    let mut reports = reports.lock().unwrap();
    let state = reports.get_mut(&key).ok_or("missing audio source report")?;
    state["running"] = json!(false);
    state["completed"] = json!(true);
    state["source_policy_valid"] = json!(packets == seconds * 50);
    Ok(())
}

async fn replay_audio_v2(
    archive: Arc<AudioArchive>,
    track: Arc<TrackLocalStaticRTP>,
    anchor: clock::Anchor,
    reports: Reports,
    ssrc: u32,
    hold_ms: u64,
    test_enabled: bool,
) -> Result<()> {
    let total = archive.metadata["duration_seconds"]
        .as_u64()
        .ok_or("finite total missing")?;
    let expected = archive.packets.len() as u64;
    let key = archive.kind.clone();
    let hold_ns = hold_ms * 1_000_000;
    let (mut packets, mut max_late, mut min_hold, mut max_hold) = (0u64, 0u64, u64::MAX, 0u64);
    for index in 0..archive.packets.len() {
        let offset = Duration::from_millis(index as u64 * 20);
        let original_due = anchor.ns + index as u64 * 20_000_000;
        let shifted_due = original_due + hold_ns;
        let due = anchor.instant + offset + Duration::from_millis(hold_ms);
        tokio::time::sleep_until(due).await;
        let before = clock::monotonic_ns()?;
        let before_late = before
            .checked_sub(shifted_due)
            .ok_or("V2 enqueue bracket precedes shifted source plan")?;
        if before_late > 20_000_000 {
            reports
                .lock()
                .unwrap()
                .get_mut(&key)
                .ok_or("missing V2 prefix")?["schedule_failure"] = json!({
                "packet_ordinal":index,"source_sample_ordinal":index*960,"planned_mono_ns":original_due.to_string(),
                "shifted_due_mono_ns":shifted_due.to_string(),"before_enqueue_ns":before.to_string(),
                "before_lateness_ns":before_late.to_string(),"expected_packet_count":expected,"missing_packet_count":expected-packets});
            return Err(format!("{key} V2 source missed 20ms packet deadline").into());
        }
        track.write_rtp(archive.packet(index, 0, ssrc)).await?;
        let after = clock::monotonic_ns()?;
        let late = after
            .checked_sub(shifted_due)
            .ok_or("V2 nonmonotonic enqueue after")?;
        min_hold = min_hold.min(before - original_due);
        max_hold = max_hold.max(after - original_due);
        max_late = max_late.max(late);
        packets += 1;
        let state = json!({"running":true,"completed":false,"end_reached":false,"source_policy_valid":late<=20_000_000,
            "source_policy_scope":"finite observed enqueue prefix; full archive/tail count checked on completion",
            "source_uid":if key=="mic" {0} else {64},"ssrc":ssrc,"archive_sha256":archive.sha256,
            "packets_enqueued":packets,"expected_packet_count":expected,"rtp_payload_bytes_enqueued":packets*320,
            "last_source_sample_ordinal":index*960,"last_planned_mono_ns":original_due.to_string(),
            "last_enqueue_before_ns":before.to_string(),"last_enqueued_mono_ns":after.to_string(),
            "last_enqueue_bracket_ns":(after-before).to_string(),"max_schedule_lateness_ns":max_late.to_string(),
            "test_hold_enabled":test_enabled,"audio_hold_ms":hold_ms,"hold_applied_packets":if test_enabled {packets} else {0},
            "min_actual_hold_ns":min_hold.to_string(),"max_actual_hold_ns":max_hold.to_string(),"timeline":anchor.evidence()});
        reports.lock().unwrap().insert(key.clone(), state);
        if late > 20_000_000 {
            reports.lock().unwrap().get_mut(&key).unwrap()["schedule_failure"] = json!({
                "packet_ordinal":index,"source_sample_ordinal":index*960,"planned_mono_ns":original_due.to_string(),
                "shifted_due_mono_ns":shifted_due.to_string(),"before_enqueue_ns":before.to_string(),
                "after_enqueue_ns":after.to_string(),"after_lateness_ns":late.to_string(),
                "expected_packet_count":expected,"missing_packet_count":expected-packets});
            return Err(format!("{key} V2 enqueue missed 20ms packet deadline").into());
        }
    }
    // Hold also applies to every tail packet. Do not end at unheld total_seconds.
    tokio::time::sleep_until(
        anchor.instant + Duration::from_secs(total) + Duration::from_millis(hold_ms),
    )
    .await;
    let mut reports = reports.lock().unwrap();
    let state = reports.get_mut(&key).ok_or("missing completed V2 report")?;
    state["running"] = json!(false);
    state["completed"] = json!(true);
    state["end_reached"] = json!(true);
    state["source_policy_valid"] = json!(packets == expected);
    Ok(())
}

#[cfg(test)]
mod signaling_tests {
    use super::*;

    #[test]
    fn padding_before_first_media_never_sets_media_timestamp() {
        let mut order = RtpOrder::default();
        assert_eq!(
            order.packet(10, 900_000, true, true).kind,
            PacketKind::Padding
        );
        assert_eq!(order.media_timestamp, None);
        let first = order.packet(11, 123, false, false);
        assert_eq!(first.kind, PacketKind::Media);
        assert!(!first.timestamp_gap);
        assert_eq!(order.packet(12, 1083, false, false).timestamp_gap, false);
    }

    #[test]
    fn padding_sequence_and_media_timestamp_are_independent() {
        let mut order = RtpOrder::default();
        order.packet(10, 1000, false, false);
        for sequence in 11..15 {
            let padding = order.packet(sequence, 1000, true, true);
            assert_eq!(padding.kind, PacketKind::Padding);
            assert_eq!(padding.sequence_gaps, 0);
            assert!(!padding.timestamp_gap);
        }
        let media = order.packet(15, 1960, true, false); // Real media may also carry RTP padding.
        assert_eq!(media.kind, PacketKind::Media);
        assert!(!media.timestamp_gap);
        assert_eq!(media.sequence_gaps, 0);
    }

    #[test]
    fn missing_padding_and_missing_media_are_both_visible() {
        let mut order = RtpOrder::default();
        order.packet(10, 1000, false, false);
        let missing_padding = order.packet(12, 1000, true, true);
        assert_eq!(missing_padding.sequence_gaps, 1);
        assert!(!missing_padding.timestamp_gap);
        assert!(!order.packet(13, 1960, false, false).timestamp_gap);
        order.packet(14, 1960, true, true);
        let missing_media = order.packet(15, 3880, false, false);
        assert_eq!(missing_media.sequence_gaps, 0);
        assert!(missing_media.timestamp_gap); // Padding cannot hide a missing 20 ms media frame.
    }

    #[test]
    fn duplicate_reorder_and_wrap_do_not_corrupt_media_clock() {
        let mut order = RtpOrder::default();
        order.packet(u16::MAX - 1, u32::MAX - 479, false, false);
        order.packet(u16::MAX, 0, true, true);
        assert_eq!(
            order.packet(u16::MAX, 0, true, true).kind,
            PacketKind::Reordered
        );
        assert_eq!(
            order.packet(u16::MAX - 1, 0, false, false).kind,
            PacketKind::Reordered
        );
        assert!(!order.packet(0, 480, false, false).timestamp_gap);
        assert_eq!(
            order.packet(32768, 0, true, true).kind,
            PacketKind::Reordered
        );
        assert!(!order.packet(1, 1440, false, false).timestamp_gap);
    }

    #[test]
    fn empty_unpadded_or_bad_nonempty_payload_is_never_padding() {
        let mut order = RtpOrder::default();
        assert_eq!(
            order.packet(10, 1, false, true).kind,
            PacketKind::EmptyWithoutPadding
        );
        assert_eq!(order.media_timestamp, None);
        assert_eq!(order.packet(11, 1, true, false).kind, PacketKind::Media);
        let mut decoder = Decoder::new().unwrap();
        assert!(decoder.decode(&[]).is_err());
        assert!(decoder.decode(&[255]).is_err());
        // Actual 20 ms Opus silence has a nonempty TOC/payload and is media,
        // regardless of RTP's optional padding bit. It must reach libopus.
        assert_eq!(
            decoder.decode(&[0xf8, 0xff, 0xfe]).unwrap().samples.len(),
            960
        );
    }

    #[tokio::test]
    async fn native_publication_sdp_has_distinct_signal_bound_track_ids() {
        let p = peer(Ipv4Addr::LOCALHOST, true).await.unwrap();
        let offer = p.pc.create_offer(None).await.unwrap();
        // MediaStreamTrack::new takes stream_id before track_id. The track id
        // must match the p/s + p/sa signaling contract, never the common "s"
        // stream id used by the two source tracks.
        for (stream, track) in [
            ("m", "fixed-native-mic"),
            ("s", "fixed-native-video"),
            ("s", "fixed-native-source-audio"),
        ] {
            assert!(offer.sdp.contains(&format!("a=msid:{stream} {track}\r\n")));
        }
        assert_eq!(
            offer
                .sdp
                .lines()
                .filter(|line| line.starts_with("a=msid:"))
                .count(),
            3
        );
        for task in p.feedback_tasks {
            task.abort();
        }
        p.pc.close().await.unwrap();
    }
}

async fn replay_video(
    archive: Arc<Archive>,
    track: Arc<TrackLocalStaticRTP>,
    anchor: clock::Anchor,
    seconds: u64,
    reports: Reports,
    finite: bool,
) -> Result<()> {
    let mut packets = 0u64;
    let mut frames = 0u64;
    let mut bytes = 0u64;
    let mut max_late = 0u64;
    for cycle in 0..seconds.div_ceil(archive.period.as_secs()) {
        for (index, record) in archive.records.iter().enumerate() {
            let offset = archive.period * cycle as u32 + record.due;
            if offset >= Duration::from_secs(seconds) {
                break;
            }
            let due = anchor.instant + offset;
            tokio::time::sleep_until(due).await;
            let late = tokio::time::Instant::now()
                .saturating_duration_since(due)
                .as_nanos() as u64;
            max_late = max_late.max(late);
            if late > 1_000_000_000 / archive::FPS {
                if finite {
                    reports
                        .lock()
                        .unwrap()
                        .get_mut("video")
                        .ok_or("missing V2 video prefix")?["schedule_failure"] = json!({
                    "frame_ordinal":frames,"packet_ordinal":packets,"planned_mono_ns":(anchor.ns+offset.as_nanos() as u64).to_string(),
                    "before_enqueue_ns":clock::monotonic_ns()?.to_string(),"before_lateness_ns":late.to_string(),
                    "expected_frame_count":seconds*60,"missing_frame_count":seconds*60-frames});
                }
                return Err("video source missed frame deadline".into());
            }
            let packet = archive.packet(index, cycle);
            if packet.header.marker {
                frames += 1;
            }
            bytes += packet.payload.len() as u64;
            track.write_rtp(packet).await?;
            let enqueue_late = tokio::time::Instant::now()
                .saturating_duration_since(due)
                .as_nanos() as u64;
            max_late = max_late.max(enqueue_late);
            if enqueue_late > 1_000_000_000 / archive::FPS {
                if finite {
                    reports
                        .lock()
                        .unwrap()
                        .get_mut("video")
                        .ok_or("missing V2 video prefix")?["schedule_failure"] = json!({
                    "frame_ordinal":frames,"packet_ordinal":packets,"planned_mono_ns":(anchor.ns+offset.as_nanos() as u64).to_string(),
                    "after_enqueue_ns":clock::monotonic_ns()?.to_string(),"after_lateness_ns":enqueue_late.to_string(),
                    "expected_frame_count":seconds*60,"missing_frame_count":seconds*60-frames});
                }
                return Err("video enqueue missed frame deadline".into());
            }
            packets += 1;
            if index + 1 == archive.records.len() || frames % 60 == 0 {
                reports.lock().unwrap().insert("video".to_owned(),json!({"running":true,"completed":false,"source_policy_valid":true,
            "source_policy_scope":"observed enqueue prefix; full frame count checked only on completion",
            "packets_enqueued":packets,"frames_enqueued":frames,"rtp_payload_bytes_enqueued":bytes,"max_schedule_lateness_ns":max_late,"timeline":anchor.evidence()}));
            }
        }
    }
    tokio::time::sleep_until(anchor.instant + Duration::from_secs(seconds)).await;
    let mut report = reports.lock().unwrap();
    let state = report
        .get_mut("video")
        .ok_or("missing video source report")?;
    state["running"] = json!(false);
    state["completed"] = json!(true);
    state["source_policy_valid"] = json!(frames == seconds * 60);
    Ok(())
}

#[tokio::main(worker_threads = 2)]
async fn main() -> Result<()> {
    let mut args: Vec<String> = std::env::args().collect();
    let test_hold_enabled = args.get(1).map(String::as_str) == Some("--allow-test-audio-hold");
    if test_hold_enabled {
        args.remove(1);
        if args.get(1).map(String::as_str) != Some("--peer0") {
            return Err("test hold flag is only allowed before --peer0".into());
        }
    }
    if args.get(1).map(String::as_str) == Some("--inspect-audio") && args.len() == 4 {
        let mic = AudioArchive::read(Path::new(&args[2]), "mic")?;
        let source = AudioArchive::read(Path::new(&args[3]), "source")?;
        pn_audio::shared_pair(&mic, &source)?;
        println!(
            "{}",
            json!({"mic":{"sha256":mic.sha256,"metadata":mic.metadata},"source":{"sha256":source.sha256,"metadata":source.metadata},
            "decoder":audio::decoder_provenance()?,"scope":"offline frozen source import; no WebRTC or latency acceptance"})
        );
        return Ok(());
    }
    if args.len() != 5 && args.len() != 6
        || !matches!(
            args.get(1).map(String::as_str),
            Some("--inspect" | "--peer0")
        )
    {
        return Err("usage: gelabber-fixed-native-peer --inspect|--peer0 VIDEO MIC SOURCE [BIND_IPV4]; or --inspect-audio MIC SOURCE".into());
    }
    let video = Arc::new(Archive::parse(&std::fs::read(&args[2])?)?);
    let mic = Arc::new(AudioArchive::read(Path::new(&args[3]), "mic")?);
    let source = Arc::new(AudioArchive::read(Path::new(&args[4]), "source")?);
    pn_audio::shared_pair(&mic, &source)?;
    if test_hold_enabled && !mic.unlooped() {
        return Err("test hold flag requires finite V2 audio archives".into());
    }
    let provenance = json!({"instrument":if mic.unlooped(){"fixed-native-peer0-v2"}else{"fixed-native-peer0-v1"},"binary_sha256":archive::hash(&std::fs::read(std::env::current_exe()?)?),
        "video":{"archive_sha256":video.sha256,"metadata":video.metadata},"mic":{"archive_sha256":mic.sha256,"metadata":mic.metadata,"import_verified":true},
        "source":{"archive_sha256":source.sha256,"metadata":source.metadata,"import_verified":true},"decoder":audio::decoder_provenance()?,
        "topology":"native peer0 replacing one browser; adapters/full N-peer graph not yet qualified","comparison_available":false,"pcm_latency_calibrated":false});
    if args[1] == "--inspect" {
        println!("{provenance}");
        return Ok(());
    }
    let bind: Ipv4Addr = args
        .get(5)
        .map(String::as_str)
        .unwrap_or("127.0.0.1")
        .parse()?;
    if bind.is_unspecified() || bind.is_multicast() || bind.is_broadcast() {
        return Err("explicit usable bind address required".into());
    }
    let mut peers: BTreeMap<String, Peer> = BTreeMap::new();
    let reports: Reports = Arc::new(Mutex::new(BTreeMap::new()));
    let mut replay_tasks: Vec<JoinHandle<()>> = Vec::new();
    let mut start_evidence: Option<Value> = None;
    println!("{}", json!({"ready":true,"provenance":provenance}));
    for line in std::io::stdin().lock().lines() {
        let line = line?;
        if line.len() > 1_048_576 {
            return Err("oversized stdin request".into());
        }
        let result:Result<Value>=async {
            let request:Value=serde_json::from_str(&line)?; let op=request["op"].as_str().ok_or("missing operation")?;
            let id=request["peer"].as_str().unwrap_or("publish");
            if op=="close" {return Ok(json!({"close":true}));}
            if op=="clock" {return Ok(json!({"clock":"CLOCK_MONOTONIC","monoNs":clock::monotonic_ns()?.to_string()}));}
            if op=="create" {
                if peers.len()>=3 || peers.contains_key(id) || id.len()>64 {return Err("duplicate/excess/invalid peer id".into());}
                let publish=request["publish"].as_bool().ok_or("create requires explicit publish boolean")?;
                if publish && peers.values().any(|p|!p.tracks.is_empty()) {return Err("only one peer0 publisher allowed".into());}
                peers.insert(id.to_owned(),peer(bind,publish).await?); return Ok(json!({"created":id}));
            }
            if op=="status" {
                let mut status=serde_json::Map::new();
                for (id,p) in &peers {let stats=p.pc.get_stats(Instant::now(),StatsSelector::None).await;
                    let outbound:Vec<Value>=stats.outbound_rtp_streams().map(serde_json::to_value).collect::<std::result::Result<_,_>>()?;
                    let codecs:Vec<Value>=stats.iter().filter_map(|entry| match entry {
                        rtc::statistics::report::RTCStatsReportEntry::Codec(codec)=>Some(serde_json::to_value(codec)), _=>None
                    }).collect::<std::result::Result<_,_>>()?;
                    let negotiated=negotiated_senders(p).await?;
                    status.insert(id.clone(),json!({"connection":p.state.borrow().to_string(),"outbound":outbound,"codecs":codecs,"negotiated_senders":negotiated,"transport":stats.transport(),"received":p.received.lock().unwrap().clone()}));}
                let mut value=json!({"peers":status,"sources":reports.lock().unwrap().clone(),"clock":"CLOCK_MONOTONIC","monoNs":clock::monotonic_ns()?.to_string()});
                if let Some(start)=&start_evidence {for key in ["timeline","measurement_seconds","total_seconds","measurement_end_sample_ordinal","tail_samples",
                    "test_hold_enabled","audio_hold_ms","comparison_available","pcm_latency_calibrated"] {value[key]=start[key].clone();}}
                return Ok(value);
            }
            let p=peers.get(id).ok_or("unknown peer id; create first")?;
            match op {
                "offer" => {p.pc.set_local_description(p.pc.create_offer(None).await?).await?; let mut gathering=p.gathered.subscribe();
                    tokio::time::timeout(Duration::from_secs(15),gathering.wait_for(|done|*done)).await??;
                    Ok(json!({"description":p.pc.local_description().await.ok_or("missing local SDP")?}))}
                "remote" => {let description:RTCSessionDescription=serde_json::from_value(request["description"].clone())?;
                    let offer=request["description"]["type"]=="offer"; p.pc.set_remote_description(description).await?;
                    if offer {p.pc.set_local_description(p.pc.create_answer(None).await?).await?;
                        let mut gathering=p.gathered.subscribe();tokio::time::timeout(Duration::from_secs(15),gathering.wait_for(|done|*done)).await??;}
                    Ok(json!({"description":p.pc.local_description().await}))}
                "ice" => {p.pc.add_ice_candidate(serde_json::from_value(request["candidate"].clone())?).await?;Ok(json!({"ok":true}))}
                "start" => {
                    if !replay_tasks.is_empty() || p.tracks.len()!=3 {return Err("one start requires full three-track publisher".into());}
                    let (seconds,hold_ms)=pn_audio::start_policy(&request,&mic,test_hold_enabled)?;
                    let mut state=p.state.subscribe();tokio::time::timeout(Duration::from_secs(30),state.wait_for(|s|matches!(s,RTCPeerConnectionState::Connected|RTCPeerConnectionState::Failed|RTCPeerConnectionState::Closed))).await??;
                    if *state.borrow()!=RTCPeerConnectionState::Connected {return Err("native publisher not connected".into());}
                    let anchor=clock::Anchor::new()?;
                    if mic.unlooped() {
                        start_evidence=Some(json!({"started":true,"timeline":anchor.evidence(),"total_seconds":seconds,
                            "measurement_seconds":mic.metadata["measurement_seconds"],"measurement_end_sample_ordinal":mic.metadata["measurement_end_sample_ordinal"],
                            "tail_samples":mic.metadata["tail_samples"],"test_hold_enabled":test_hold_enabled,"audio_hold_ms":hold_ms,
                            "comparison_available":false,"pcm_latency_calibrated":false}));
                        for (name,count) in [("mic",mic.packets.len()),("source",source.packets.len()),("video",0)] {
                            reports.lock().unwrap().insert(name.to_owned(),json!({"running":true,"completed":false,"source_policy_valid":false,
                                "packets_enqueued":0,"expected_packet_count":count,"timeline":anchor.evidence()}));
                        }
                    }
                    for (name,track) in &p.tracks {
                        let name=name.clone(); let track=track.clone(); let reports=reports.clone(); let anchor=anchor.clone();
                        let video=video.clone();let mic=mic.clone();let source=source.clone();
                        let finite=mic.unlooped();
                        replay_tasks.push(tokio::spawn(async move {let result=match name.as_str(){
                            "mic" if mic.unlooped()=>replay_audio_v2(mic,track,anchor,reports.clone(),MIC,hold_ms,test_hold_enabled).await,
                            "source" if source.unlooped()=>replay_audio_v2(source,track,anchor,reports.clone(),SOURCE,hold_ms,test_hold_enabled).await,
                            "mic"=>replay_audio(mic,track,anchor,seconds,reports.clone(),MIC).await,
                            "source"=>replay_audio(source,track,anchor,seconds,reports.clone(),SOURCE).await,
                            _=>replay_video(video,track,anchor,seconds,reports.clone(),finite).await};
                            if let Err(error)=result {
                                let mut all=reports.lock().unwrap();
                                if let Some(state)=all.get_mut(&name).filter(|_|finite) {
                                    state["running"]=json!(false);state["completed"]=json!(false);state["end_reached"]=json!(false);
                                    state["source_policy_valid"]=json!(false);state["error"]=json!(error.to_string());
                                }else{all.insert(name,json!({"running":false,"source_policy_valid":false,"error":error.to_string()}));}
                            }
                        }));
                    }
                    Ok(start_evidence.clone().unwrap_or_else(||json!({"started":true,"timeline":anchor.evidence()})))
                }
                "bind" => {
                    let ssrc=request["ssrc"].as_u64().filter(|ssrc|*ssrc<=u32::MAX as u64).ok_or("valid SSRC required")?;
                    let name=request["source_name"].as_str().ok_or("source_name required")?;
                    if !name.starts_with("peer-") || !name.ends_with("/mic") || name.len()>64 {return Err("explicit remote peer microphone identity required".into());}
                    let mut received=p.received.lock().unwrap();
                    if received.values().any(|edge|edge["source_name"]==name && edge["ssrc"]!=ssrc) {return Err("duplicate native source identity".into());}
                    let edge=received.get_mut(&ssrc.to_string()).ok_or("cannot bind source before actual receive event")?;
                    if !edge["source_name"].is_null() && edge["source_name"]!=name {return Err("cannot change native source binding".into());}
                    edge["source_name"]=json!(name);Ok(json!({"bound":ssrc,"source_name":name}))
                }
                _=>Err("unknown operation".into())
            }
        }.await;
        match result {
            Ok(value) if value["close"] == true => break,
            Ok(value) => println!("{value}"),
            Err(error) => println!("{}", json!({"error":error.to_string()})),
        }
    }
    for task in replay_tasks {
        task.abort();
        let _ = task.await;
    }
    let mut errors = Vec::new();
    for (_, p) in peers {
        for task in p.feedback_tasks {
            task.abort();
            let _ = task.await;
        }
        for task in p.receiver_tasks.lock().unwrap().drain(..) {
            task.abort();
        }
        if let Err(error) = p.pc.close().await {
            errors.push(error.to_string());
        }
    }
    if !errors.is_empty() {
        return Err(format!("native peer cleanup failed: {errors:?}").into());
    }
    Ok(())
}
