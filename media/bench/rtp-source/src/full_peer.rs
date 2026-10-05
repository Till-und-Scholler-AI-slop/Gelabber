//! Native peer0 instrument: ordinary WebRTC, three fixed sources and actual Opus decoding.
//! Controlled only over stdin. Still diagnostic until full N-peer adapters qualify.
mod archive;
mod audio;
mod clock;
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
    let mut decoders: BTreeMap<u32, (Decoder, Option<(u16, u32)>)> = BTreeMap::new();
    while let Some(event) = track.poll().await {
        match event {
            TrackRemoteEvent::OnOpen(init) => {
                reports.lock().unwrap().entry(init.ssrc.to_string()).or_insert(json!({
                    "ssrc":init.ssrc,"track_id":init.track_id,"stream_ids":init.stream_ids,"source_name":null,
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
                    decoders.insert(ssrc, (Decoder::new()?, None));
                }
                let (decoder, previous) = decoders.get_mut(&ssrc).unwrap();
                let mut report = reports.lock().unwrap();
                let edge = report
                    .get_mut(&ssrc.to_string())
                    .ok_or("RTP arrived before source identity event")?;
                if let Some((sequence, timestamp)) = *previous {
                    let step = packet.header.sequence_number.wrapping_sub(sequence);
                    if step == 0 || step > 32768 {
                        edge["reordered_or_duplicate_packets"] =
                            json!(edge["reordered_or_duplicate_packets"].as_u64().unwrap() + 1);
                        continue; // Never feed a duplicate/reordered packet into a stateful decoder as new PCM.
                    }
                    edge["sequence_gaps"] =
                        json!(edge["sequence_gaps"].as_u64().unwrap() + u64::from(step - 1));
                    if packet.header.timestamp.wrapping_sub(timestamp) != u32::from(step) * 960 {
                        edge["timestamp_gaps"] =
                            json!(edge["timestamp_gaps"].as_u64().unwrap() + 1);
                    }
                }
                *previous = Some((packet.header.sequence_number, packet.header.timestamp));
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
            reports.lock().unwrap().insert(key.clone(), json!({"running":true,"completed":false,"ssrc":ssrc,
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

#[cfg(test)]
mod signaling_tests {
    use super::*;

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
) -> Result<()> {
    let mut packets = 0u64;
    let mut frames = 0u64;
    let mut bytes = 0u64;
    let mut max_late = 0u64;
    for cycle in 0..seconds / archive.period.as_secs() {
        for (index, record) in archive.records.iter().enumerate() {
            let due = anchor.instant + archive.period * cycle as u32 + record.due;
            tokio::time::sleep_until(due).await;
            let late = tokio::time::Instant::now()
                .saturating_duration_since(due)
                .as_nanos() as u64;
            max_late = max_late.max(late);
            if late > 1_000_000_000 / archive::FPS {
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
                return Err("video enqueue missed frame deadline".into());
            }
            packets += 1;
            if index + 1 == archive.records.len() || frames % 60 == 0 {
                reports.lock().unwrap().insert("video".to_owned(),json!({"running":true,"completed":false,
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
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--inspect-audio") && args.len() == 4 {
        let mic = AudioArchive::read(Path::new(&args[2]), "mic")?;
        let source = AudioArchive::read(Path::new(&args[3]), "source")?;
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
    let provenance = json!({"instrument":"fixed-native-peer0-v1","binary_sha256":archive::hash(&std::fs::read(std::env::current_exe()?)?),
        "video":{"archive_sha256":video.sha256,"metadata":video.metadata},"mic":{"archive_sha256":mic.sha256,"metadata":mic.metadata},
        "source":{"archive_sha256":source.sha256,"metadata":source.metadata},"decoder":audio::decoder_provenance()?,
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
                    status.insert(id.clone(),json!({"connection":p.state.borrow().to_string(),"outbound":outbound,"transport":stats.transport(),"received":p.received.lock().unwrap().clone()}));}
                return Ok(json!({"peers":status,"sources":reports.lock().unwrap().clone(),"clock":"CLOCK_MONOTONIC","monoNs":clock::monotonic_ns()?.to_string()}));
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
                    let seconds=request["seconds"].as_u64().ok_or("duration required")?;
                    if seconds==0 || seconds>3600 || seconds%10!=0 {return Err("duration must be whole ten-second periods <=3600".into());}
                    let mut state=p.state.subscribe();tokio::time::timeout(Duration::from_secs(30),state.wait_for(|s|matches!(s,RTCPeerConnectionState::Connected|RTCPeerConnectionState::Failed|RTCPeerConnectionState::Closed))).await??;
                    if *state.borrow()!=RTCPeerConnectionState::Connected {return Err("native publisher not connected".into());}
                    let anchor=clock::Anchor::new()?;
                    for (name,track) in &p.tracks {
                        let name=name.clone(); let track=track.clone(); let reports=reports.clone(); let anchor=anchor.clone();
                        let video=video.clone();let mic=mic.clone();let source=source.clone();
                        replay_tasks.push(tokio::spawn(async move {let result=match name.as_str(){
                            "mic"=>replay_audio(mic,track,anchor,seconds,reports.clone(),MIC).await,
                            "source"=>replay_audio(source,track,anchor,seconds,reports.clone(),SOURCE).await,
                            _=>replay_video(video,track,anchor,seconds,reports.clone()).await};
                            if let Err(error)=result {reports.lock().unwrap().insert(name,json!({"running":false,"source_policy_valid":false,"error":error.to_string()}));}
                        }));
                    }
                    Ok(json!({"started":true,"timeline":anchor.evidence()}))
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
