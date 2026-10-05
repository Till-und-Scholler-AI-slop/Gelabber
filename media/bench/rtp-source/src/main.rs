//! Bounded fixed-source WebRTC component, controlled privately over JSON stdin.
//! Deliberately separate from the browser matrix until its topology is reviewed.
mod archive;
use archive::{Archive, Result, SSRC};
use rtc::{
    interceptor::Registry,
    media_stream::MediaStreamTrack,
    peer_connection::configuration::{
        RTCConfigurationBuilder,
        interceptor_registry::{configure_nack, configure_rtcp_reports},
        media_engine::{MIME_TYPE_VP8, MediaEngine},
    },
    rtp_transceiver::rtp_sender::{
        RTCRtpCodec, RTCRtpCodecParameters, RTCRtpCodingParameters, RTCRtpEncodingParameters,
        RtpCodecKind,
    },
};
use serde_json::{Value, json};
use std::{
    io::BufRead,
    net::Ipv4Addr,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::{sync::watch, task::JoinHandle};
use webrtc::{
    media_stream::track_local::{TrackLocal, static_rtp::TrackLocalStaticRTP},
    peer_connection::{
        PeerConnection, PeerConnectionBuilder, PeerConnectionEventHandler, RTCIceGatheringState,
        RTCPeerConnectionState, RTCSessionDescription, StatsSelector,
    },
};

struct Handler {
    gathered: watch::Sender<bool>,
    state: watch::Sender<RTCPeerConnectionState>,
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
}

async fn replay(
    archive: Arc<Archive>,
    track: Arc<TrackLocalStaticRTP>,
    seconds: u64,
    report: Arc<Mutex<Value>>,
) -> Result<()> {
    let started = tokio::time::Instant::now();
    let mut packets = 0u64;
    let mut frames = 0u64;
    let mut payload_bytes = 0u64;
    let mut max_late_us = 0u64;
    let mut lateness_sum_us = 0u64;
    for cycle in 0..seconds / archive.period.as_secs() {
        for (index, record) in archive.records.iter().enumerate() {
            let due = started + archive.period * cycle as u32 + record.due;
            tokio::time::sleep_until(due).await;
            let late = tokio::time::Instant::now().saturating_duration_since(due);
            max_late_us = max_late_us.max(late.as_micros() as u64);
            lateness_sum_us += late.as_micros() as u64;
            // Never silently burst/catch up a source delayed by an entire frame.
            if late.as_nanos() > 1_000_000_000 / archive::FPS as u128 {
                return Err(format!(
                    "source scheduling missed its frame interval: {}us",
                    late.as_micros()
                )
                .into());
            }
            let packet = archive.packet(index, cycle);
            payload_bytes += packet.payload.len() as u64;
            if packet.header.marker {
                frames += 1;
            }
            track.write_rtp(packet).await?;
            packets += 1;
            if index + 1 == archive.records.len() || frames % 60 == 0 {
                *report.lock().unwrap() = json!({"running":true, "packets_enqueued":packets, "frames_enqueued":frames,
                    "rtp_payload_bytes_enqueued":payload_bytes, "elapsed_seconds":started.elapsed().as_secs_f64(),
                    "max_schedule_lateness_us":max_late_us, "mean_schedule_lateness_us":lateness_sum_us as f64 / packets as f64});
            }
        }
    }
    tokio::time::sleep_until(started + Duration::from_secs(seconds)).await;
    let mut state = report.lock().unwrap();
    state["running"] = json!(false);
    state["completed"] = json!(true);
    state["elapsed_seconds"] = json!(started.elapsed().as_secs_f64());
    state["intended_duration_seconds"] = json!(seconds);
    state["expected_frames"] = json!(seconds * archive::FPS);
    state["rtp_payload_bitrate_bps"] = json!(payload_bytes * 8 / seconds);
    state["source_policy_valid"] = json!(frames == seconds * archive::FPS);
    Ok(())
}

async fn operation(
    request: &Value,
    pc: &Arc<dyn PeerConnection>,
    gathered: &watch::Sender<bool>,
    connection: &watch::Sender<RTCPeerConnectionState>,
    archive: &Arc<Archive>,
    track: &Arc<TrackLocalStaticRTP>,
    report: &Arc<Mutex<Value>>,
    task: &mut Option<JoinHandle<()>>,
) -> Result<Value> {
    match request["op"].as_str().ok_or("missing operation")? {
        "offer" => {
            pc.set_local_description(pc.create_offer(None).await?)
                .await?;
            let mut state = gathered.subscribe();
            tokio::time::timeout(Duration::from_secs(15), state.wait_for(|done| *done)).await??;
            Ok(json!({"description":pc.local_description().await.ok_or("missing local SDP")?}))
        }
        "remote" => {
            let description: RTCSessionDescription =
                serde_json::from_value(request["description"].clone())?;
            let offer = request["description"]["type"] == "offer";
            pc.set_remote_description(description).await?;
            if offer {
                pc.set_local_description(pc.create_answer(None).await?)
                    .await?;
                Ok(json!({"description":pc.local_description().await.ok_or("missing local SDP")?}))
            } else {
                Ok(json!({"ok":true}))
            }
        }
        "ice" => {
            pc.add_ice_candidate(serde_json::from_value(request["candidate"].clone())?)
                .await?;
            Ok(json!({"ok":true}))
        }
        "start" => {
            if task.is_some() {
                return Err("source can start only once per process".into());
            }
            let seconds = request["seconds"].as_u64().ok_or("missing duration")?;
            if seconds < archive.period.as_secs()
                || seconds > 3600
                || seconds % archive.period.as_secs() != 0
            {
                return Err(
                    "duration must be 1..3600s and a whole number of frozen source periods".into(),
                );
            }
            let mut state = connection.subscribe();
            tokio::time::timeout(
                Duration::from_secs(30),
                state.wait_for(|s| {
                    matches!(
                        s,
                        RTCPeerConnectionState::Connected
                            | RTCPeerConnectionState::Failed
                            | RTCPeerConnectionState::Closed
                    )
                }),
            )
            .await??;
            if *state.borrow() != RTCPeerConnectionState::Connected {
                return Err("WebRTC did not connect".into());
            }
            let a = archive.clone();
            let t = track.clone();
            let r = report.clone();
            *task = Some(tokio::spawn(async move {
                if let Err(error) = replay(a, t, seconds, r.clone()).await {
                    let mut state = r.lock().unwrap();
                    state["running"] = json!(false);
                    state["error"] = json!(error.to_string());
                    state["source_policy_valid"] = json!(false);
                }
            }));
            Ok(json!({"started":true}))
        }
        "status" => {
            let source = report.lock().unwrap().clone();
            let stats = pc.get_stats(Instant::now(), StatsSelector::None).await;
            let outbound: Vec<Value> = stats
                .outbound_rtp_streams()
                .map(serde_json::to_value)
                .collect::<std::result::Result<_, _>>()?;
            Ok(
                json!({"source":source, "connection":connection.borrow().to_string(), "outbound":outbound, "transport":stats.transport()}),
            )
        }
        _ => Err("unknown operation".into()),
    }
}

#[tokio::main(worker_threads = 2)]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3
        || args.len() > 4
        || !matches!(args[1].as_str(), "--archive" | "--inspect")
        || (args[1] == "--inspect" && args.len() != 3)
    {
        return Err("usage: gelabber-fixed-rtp-source --archive|--inspect FILE [BIND_IPV4]".into());
    }
    let archive = Arc::new(Archive::parse(&std::fs::read(&args[2])?)?);
    let provenance = json!({"archive_sha256":archive.sha256,"binary_sha256":archive::hash(&std::fs::read(std::env::current_exe()?)?),"source":archive.metadata,"instrument":"fixed-rtp-webrtc-v1","webrtc":"0.20.5","topology":"one additional video-only WebRTC publisher","comparison_available":false,"production_feature_acceptance":false});
    if args[1] == "--inspect" {
        println!("{provenance}");
        return Ok(());
    }
    let bind: Ipv4Addr = args
        .get(3)
        .map(String::as_str)
        .unwrap_or("127.0.0.1")
        .parse()?;
    if bind.is_unspecified() || bind.is_multicast() || bind.is_broadcast() {
        return Err("bind an explicit, usable IPv4 address".into());
    }
    let mut media = MediaEngine::default();
    let codec = RTCRtpCodec {
        mime_type: MIME_TYPE_VP8.to_owned(),
        clock_rate: 90_000,
        ..Default::default()
    };
    media.register_codec(
        RTCRtpCodecParameters {
            rtp_codec: codec.clone(),
            payload_type: 96,
            ..Default::default()
        },
        RtpCodecKind::Video,
    )?;
    // This source sends the archive's exact RTP headers. Do not advertise TWCC
    // or MID extensions that its packets do not contain. RR/SR and NACK stay on.
    let registry = configure_rtcp_reports(configure_nack(Registry::new(), &mut media));
    let track = Arc::new(TrackLocalStaticRTP::new(MediaStreamTrack::new(
        "fixed-vp8-source".to_string(),
        "fixed-vp8-video".to_string(),
        "frozen VP8 source".to_string(),
        RtpCodecKind::Video,
        vec![RTCRtpEncodingParameters {
            rtp_coding_parameters: RTCRtpCodingParameters {
                ssrc: Some(SSRC),
                ..Default::default()
            },
            codec,
            ..Default::default()
        }],
    )));
    let (gathered, _) = watch::channel(false);
    let (connection, _) = watch::channel(RTCPeerConnectionState::New);
    let pc: Arc<dyn PeerConnection> = Arc::new(
        PeerConnectionBuilder::new()
            .with_configuration(RTCConfigurationBuilder::new().build())
            .with_media_engine(media)
            .with_interceptor_registry(registry)
            .with_handler(Arc::new(Handler {
                gathered: gathered.clone(),
                state: connection.clone(),
            }))
            .with_udp_addrs(vec![format!("{bind}:0")])
            .build()
            .await?,
    );
    pc.add_track(track.clone() as Arc<dyn TrackLocal>).await?;
    // Drain feedback; default NACK responders stay enabled, but no feedback can
    // resize/re-encode this frozen source or add out-of-schedule keyframes.
    let feedback_packets = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (feedback_track, feedback_count) = (track.clone(), feedback_packets.clone());
    let feedback_task = tokio::spawn(async move {
        loop {
            if let Some(event) = feedback_track.poll().await {
                let webrtc::media_stream::track_local::TrackLocalEvent::OnRtcpPacket(packets) =
                    event;
                feedback_count
                    .fetch_add(packets.len() as u64, std::sync::atomic::Ordering::Relaxed);
            } else {
                // poll() returns None before SDP has bound this track.
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    });
    println!("{}", json!({"ready":true,"provenance":provenance}));
    let report = Arc::new(Mutex::new(json!({"running":false,"completed":false})));
    let mut task = None;
    for line in std::io::stdin().lock().lines() {
        let line = line?;
        if line.len() > 1_048_576 {
            break;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(request) if request["op"] == "close" => break,
            Ok(request) => {
                operation(
                    &request,
                    &pc,
                    &gathered,
                    &connection,
                    &archive,
                    &track,
                    &report,
                    &mut task,
                )
                .await
            }
            Err(error) => Err(error.into()),
        };
        println!(
            "{}",
            match reply {
                Ok(mut value) => {
                    value["feedback_packets"] =
                        json!(feedback_packets.load(std::sync::atomic::Ordering::Relaxed));
                    value
                }
                Err(error) => json!({"error":error.to_string()}),
            }
        );
    }
    if let Some(task) = task {
        task.abort();
        let _ = task.await;
    }
    feedback_task.abort();
    pc.close().await?;
    Ok(())
}
