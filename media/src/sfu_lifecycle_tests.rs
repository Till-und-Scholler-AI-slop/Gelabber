use super::*;
use rtc::media_stream::{
    MediaStreamTrackState, MediaTrackCapabilities, MediaTrackConstraints, MediaTrackSettings,
};
use rtc::rtp_transceiver::{RtpStreamId, SSRC};
use webrtc::media_stream::{MediaStreamId, MediaStreamTrackId, Track};

struct Remote {
    local: TrackLocalStaticRTP,
    events: Mutex<mpsc::UnboundedReceiver<TrackRemoteEvent>>,
}

#[async_trait::async_trait]
impl Track for Remote {
    async fn stream_id(&self) -> MediaStreamId {
        self.local.stream_id().await
    }
    async fn track_id(&self) -> MediaStreamTrackId {
        self.local.track_id().await
    }
    async fn label(&self) -> String {
        self.local.label().await
    }
    async fn kind(&self) -> RtpCodecKind {
        self.local.kind().await
    }
    async fn rid(&self, ssrc: SSRC) -> Option<RtpStreamId> {
        self.local.rid(ssrc).await
    }
    async fn codec(&self, ssrc: SSRC) -> Option<RTCRtpCodec> {
        self.local.codec(ssrc).await
    }
    async fn ssrcs(&self) -> Vec<SSRC> {
        self.local.ssrcs().await
    }
    async fn enabled(&self) -> bool {
        self.local.enabled().await
    }
    async fn set_enabled(&self, enabled: bool) {
        self.local.set_enabled(enabled).await
    }
    async fn muted(&self) -> bool {
        self.local.muted().await
    }
    async fn set_muted(&self, muted: bool) {
        self.local.set_muted(muted).await
    }
    async fn ready_state(&self) -> MediaStreamTrackState {
        self.local.ready_state().await
    }
    async fn stop(&self) {
        self.local.stop().await
    }
    async fn get_capabilities(&self) -> MediaTrackCapabilities {
        self.local.get_capabilities().await
    }
    async fn get_constraints(&self) -> MediaTrackConstraints {
        self.local.get_constraints().await
    }
    async fn get_settings(&self) -> MediaTrackSettings {
        self.local.get_settings().await
    }
    async fn apply_constraints(&self, constraints: Option<MediaTrackConstraints>) {
        self.local.apply_constraints(constraints).await
    }
    async fn codings(&self) -> Vec<RTCRtpEncodingParameters> {
        self.local.codings().await
    }
    async fn add_coding(&self, coding: RTCRtpEncodingParameters) {
        self.local.add_coding(coding).await
    }
}

#[async_trait::async_trait]
impl TrackRemote for Remote {
    async fn write_rtcp(&self, _: Vec<Box<dyn rtcp::Packet>>) -> webrtc::error::Result<()> {
        Ok(())
    }
    async fn poll(&self) -> Option<TrackRemoteEvent> {
        self.events.lock().await.recv().await
    }
}

fn remote(
    id: &str,
) -> (
    Arc<dyn TrackRemote>,
    mpsc::UnboundedSender<TrackRemoteEvent>,
) {
    let (tx, rx) = mpsc::unbounded_channel();
    let local = TrackLocalStaticRTP::new(MediaStreamTrack::new(
        "capture".into(),
        id.into(),
        id.into(),
        RtpCodecKind::Video,
        vec![RTCRtpEncodingParameters {
            rtp_coding_parameters: RTCRtpCodingParameters {
                ssrc: Some(1234),
                ..Default::default()
            },
            codec: RTCRtpCodec {
                mime_type: "video/VP8".into(),
                clock_rate: 90000,
                ..Default::default()
            },
            ..Default::default()
        }],
    ));
    (
        Arc::new(Remote {
            local,
            events: Mutex::new(rx),
        }),
        tx,
    )
}

fn config() -> Config {
    Config::from_source(|key| match key {
        "REDIS_URL" => Some("redis://127.0.0.1:1".into()),
        "MEDIA_ICE_BIND" => Some("127.0.0.1:0".into()),
        "TURN_URLS" => Some("turn:127.0.0.1:3478".into()),
        "TURN_USERNAME" => Some("test".into()),
        "TURN_PASSWORD" => Some("test".into()),
        _ => None,
    })
    .unwrap()
}

async fn join(sfu: &Arc<Sfu>, channel: Uuid) -> (PeerId, mpsc::UnboundedReceiver<ServerFrame>) {
    let (tx, rx) = mpsc::unbounded_channel();
    let id = sfu
        .join(
            TicketClaim {
                u: Uuid::new_v4(),
                s: Uuid::new_v4(),
                c: channel,
                g: true,
            },
            tx,
        )
        .await
        .unwrap();
    (id, rx)
}

#[tokio::test]
async fn reversed_track_arrival_keeps_camera_screen_and_live_identity() {
    let sfu = Arc::new(Sfu::new(&config()));
    let channel = Uuid::new_v4();
    let (publisher, _rx) = join(&sfu, channel).await;
    let (subscriber, _rx) = join(&sfu, channel).await;
    for (kind, id) in [("v", "camera"), ("s", "screen"), ("l", "live")] {
        sfu.announce_track(publisher, channel, kind, Some(id))
            .await
            .unwrap();
    }
    let mut events = Vec::new();
    for (kind, id) in [("l", "live"), ("s", "screen"), ("v", "camera")] {
        let (track, tx) = remote(id);
        sfu.publish(publisher, channel, track).await.unwrap();
        events.push(tx);
        let room = sfu.find_room(channel).await.unwrap();
        let room = room.lock().await;
        let publication = &room.pubs[&format!("{}:{id}", publisher.0)];
        assert!(publication.stream_id.ends_with(&format!(":{kind}")));
    }
    for _ in 0..10 {
        sfu.attach_existing_pubs(subscriber, channel).await;
    }
    let room = sfu.find_room(channel).await.unwrap();
    let gate = room.lock().await.peers[&subscriber].sdp.clone();
    assert_eq!(
        gate.lock().await.subscriptions.len(),
        3,
        "queued attach must be idempotent"
    );
    sfu.leave(publisher, channel).await;
    assert!(gate.lock().await.subscriptions.is_empty());
    sfu.leave(subscriber, channel).await;
    assert_eq!(sfu.room_count(), 0);
}

#[tokio::test]
async fn twenty_stop_start_cycles_cancel_reader_and_pending_subscription() {
    let sfu = Arc::new(Sfu::new(&config()));
    let channel = Uuid::new_v4();
    let (publisher, _rx) = join(&sfu, channel).await;
    let (subscriber, _rx) = join(&sfu, channel).await;
    let room = sfu.find_room(channel).await.unwrap();
    let gate = room.lock().await.peers[&subscriber].sdp.clone();
    for _ in 0..20 {
        sfu.announce_track(publisher, channel, "s", Some("reused-track"))
            .await
            .unwrap();
        let (track, tx) = remote("reused-track");
        sfu.publish(publisher, channel, track).await.unwrap();
        let life = room.lock().await.pubs.values().next().unwrap().life.clone();
        let mut stopped = life.stop.subscribe();
        // Natural remote end is the trigger, not a synthetic peer leave.
        tx.send(TrackRemoteEvent::OnEnded).unwrap();
        tokio::time::timeout(Duration::from_secs(3), stopped.wait_for(|s| *s))
            .await
            .unwrap()
            .unwrap();
        assert!(room.lock().await.pubs.is_empty());
        assert!(gate.lock().await.subscriptions.is_empty());
        assert!(*life.done.borrow(), "publisher reader has finished");
        assert!(room.lock().await.peers[&publisher].remote_tracks.is_empty());
    }
    sfu.leave(publisher, channel).await;
    sfu.leave(subscriber, channel).await;
}

#[tokio::test]
async fn offered_sender_is_deduplicated_and_removed_on_failed_offer() {
    let sfu = Arc::new(Sfu::new(&config()));
    let channel = Uuid::new_v4();
    let (publisher, _rx) = join(&sfu, channel).await;
    let (subscriber, _rx) = join(&sfu, channel).await;
    let room = sfu.find_room(channel).await.unwrap();
    let (gate, pc) = {
        let room = room.lock().await;
        (
            room.peers[&subscriber].sdp.clone(),
            room.peers[&subscriber].pc.clone(),
        )
    };
    gate.lock().await.negotiated = true;
    sfu.announce_track(publisher, channel, "v", Some("cam"))
        .await
        .unwrap();
    let (track, _events) = remote("cam");
    sfu.publish(publisher, channel, track).await.unwrap();
    for _ in 0..10 {
        sfu.attach_existing_pubs(subscriber, channel).await;
    }
    assert_eq!(gate.lock().await.subscriptions.len(), 1);
    assert_eq!(pc.get_senders().await.len(), 1);
    sfu.abort_offer(subscriber, channel).await.unwrap();
    assert!(gate.lock().await.subscriptions.is_empty());
    assert!(pc.get_senders().await.is_empty());
    assert!(!gate.lock().await.have_local_offer);
    assert!(pc.pending_local_description().await.is_none());
    sfu.leave(publisher, channel).await;
    sfu.leave(subscriber, channel).await;
}

#[tokio::test]
async fn join_and_last_leave_share_the_room_map_lock() {
    let sfu = Arc::new(Sfu::new(&config()));
    let channel = Uuid::new_v4();
    let (old, _rx) = join(&sfu, channel).await;
    let barrier = Arc::new(tokio::sync::Barrier::new(3));
    let leave = {
        let sfu = sfu.clone();
        let barrier = barrier.clone();
        tokio::spawn(async move {
            barrier.wait().await;
            sfu.leave(old, channel).await;
        })
    };
    let arriving = {
        let sfu = sfu.clone();
        let barrier = barrier.clone();
        tokio::spawn(async move {
            barrier.wait().await;
            join(&sfu, channel).await
        })
    };
    barrier.wait().await;
    leave.await.unwrap();
    let (new, _rx) = arriving.await.unwrap();
    assert!(sfu.has_peer(new, channel).await);
    assert_eq!(sfu.room_count(), 1);
    sfu.leave(new, channel).await;
    assert_eq!(sfu.room_count(), 0);
    // Stale callbacks/ICE never create empty rooms.
    sfu.leave(old, channel).await;
    assert!(sfu.add_ice(old, channel, "bad".into(), None).await.is_err());
    assert_eq!(sfu.room_count(), 0);
}

#[test]
fn legacy_bindings_use_sdp_order_and_ignore_inactive_video() {
    let sdp = "v=0\r\nm=video 9 UDP/TLS/RTP/SAVPF 96\r\na=sendonly\r\na=msid:stream camera\r\na=ssrc:12 msid:stream camera\r\nm=video 9 UDP/TLS/RTP/SAVPF 96\r\na=msid:screen screen\r\nm=video 9 UDP/TLS/RTP/SAVPF 96\r\na=recvonly\r\na=msid:live stopped\r\n";
    assert_eq!(video_sources(sdp), vec!["camera", "screen"]);
}

use rtc::peer_connection::configuration::{RTCAnswerOptions, RTCOfferOptions};
use rtc::rtp_transceiver::RTCRtpTransceiverInit;
use rtc::statistics::{StatsSelector, report::RTCStatsReport};
use webrtc::data_channel::{DataChannel, RTCDataChannelInit};
use webrtc::error::Result;
use webrtc::peer_connection::{RTCConfiguration, RTCSdpType};
use webrtc::rtp_transceiver::{RtpReceiver, RtpTransceiver};

struct BlockPc {
    inner: Arc<dyn PeerConnection>,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
    block_close: bool,
}

#[async_trait::async_trait]
impl PeerConnection for BlockPc {
    async fn close(&self) -> Result<()> {
        if self.block_close {
            self.entered.notify_one();
            self.release.notified().await;
        }
        self.inner.close().await
    }
    async fn create_offer(
        &self,
        options: Option<RTCOfferOptions>,
    ) -> Result<RTCSessionDescription> {
        self.inner.create_offer(options).await
    }
    async fn create_answer(
        &self,
        options: Option<RTCAnswerOptions>,
    ) -> Result<RTCSessionDescription> {
        self.inner.create_answer(options).await
    }
    async fn set_local_description(&self, desc: RTCSessionDescription) -> Result<()> {
        if desc.sdp_type == RTCSdpType::Rollback {
            self.entered.notify_one();
            self.release.notified().await;
        }
        self.inner.set_local_description(desc).await
    }
    async fn local_description(&self) -> Option<RTCSessionDescription> {
        self.inner.local_description().await
    }
    async fn current_local_description(&self) -> Option<RTCSessionDescription> {
        self.inner.current_local_description().await
    }
    async fn pending_local_description(&self) -> Option<RTCSessionDescription> {
        self.inner.pending_local_description().await
    }
    async fn can_trickle_ice_candidates(&self) -> Option<bool> {
        self.inner.can_trickle_ice_candidates().await
    }
    async fn set_remote_description(&self, desc: RTCSessionDescription) -> Result<()> {
        self.inner.set_remote_description(desc).await
    }
    async fn remote_description(&self) -> Option<RTCSessionDescription> {
        self.inner.remote_description().await
    }
    async fn current_remote_description(&self) -> Option<RTCSessionDescription> {
        self.inner.current_remote_description().await
    }
    async fn pending_remote_description(&self) -> Option<RTCSessionDescription> {
        self.inner.pending_remote_description().await
    }
    async fn add_ice_candidate(&self, candidate: RTCIceCandidateInit) -> Result<()> {
        self.inner.add_ice_candidate(candidate).await
    }
    async fn restart_ice(&self) -> Result<()> {
        self.inner.restart_ice().await
    }
    async fn get_configuration(&self) -> RTCConfiguration {
        self.inner.get_configuration().await
    }
    async fn set_configuration(&self, configuration: RTCConfiguration) -> Result<()> {
        self.inner.set_configuration(configuration).await
    }
    async fn create_data_channel(
        &self,
        label: &str,
        options: Option<RTCDataChannelInit>,
    ) -> Result<Arc<dyn DataChannel>> {
        self.inner.create_data_channel(label, options).await
    }
    async fn get_senders(&self) -> Vec<Arc<dyn RtpSender>> {
        self.inner.get_senders().await
    }
    async fn get_receivers(&self) -> Vec<Arc<dyn RtpReceiver>> {
        self.inner.get_receivers().await
    }
    async fn get_transceivers(&self) -> Vec<Arc<dyn RtpTransceiver>> {
        self.inner.get_transceivers().await
    }
    async fn add_track(&self, track: Arc<dyn TrackLocal>) -> Result<Arc<dyn RtpSender>> {
        self.inner.add_track(track).await
    }
    async fn remove_track(&self, sender: &Arc<dyn RtpSender>) -> Result<()> {
        self.inner.remove_track(sender).await
    }
    async fn add_transceiver_from_track(
        &self,
        track: Arc<dyn TrackLocal>,
        init: Option<RTCRtpTransceiverInit>,
    ) -> Result<Arc<dyn RtpTransceiver>> {
        self.inner.add_transceiver_from_track(track, init).await
    }
    async fn add_transceiver_from_kind(
        &self,
        kind: RtpCodecKind,
        init: Option<RTCRtpTransceiverInit>,
    ) -> Result<Arc<dyn RtpTransceiver>> {
        self.inner.add_transceiver_from_kind(kind, init).await
    }
    async fn get_stats(&self, now: Instant, selector: StatsSelector) -> RTCStatsReport {
        self.inner.get_stats(now, selector).await
    }
}

#[tokio::test]
async fn rollback_holds_gate_until_next_publication_can_create_offer() {
    let sfu = Arc::new(Sfu::new(&config()));
    let channel = Uuid::new_v4();
    let (publisher, _rx) = join(&sfu, channel).await;
    let (subscriber, _rx) = join(&sfu, channel).await;
    let room = sfu.find_room(channel).await.unwrap();
    let (gate, blocked) = {
        let mut room = room.lock().await;
        let peer = room.peers.get_mut(&subscriber).unwrap();
        let blocked = Arc::new(BlockPc {
            inner: peer.pc.clone(),
            entered: tokio::sync::Notify::new(),
            release: tokio::sync::Notify::new(),
            block_close: false,
        });
        peer.pc = blocked.clone();
        (peer.sdp.clone(), blocked)
    };
    gate.lock().await.negotiated = true;
    sfu.announce_track(publisher, channel, "s", Some("screen"))
        .await
        .unwrap();
    let (track, _events) = remote("screen");
    sfu.publish(publisher, channel, track).await.unwrap();
    let rollback = {
        let sfu = sfu.clone();
        tokio::spawn(async move {
            sfu.abort_offer(subscriber, channel).await.unwrap();
        })
    };
    blocked.entered.notified().await;
    assert!(gate.try_lock().is_err(), "rollback must still own the gate");
    let publication = room.lock().await.pubs.values().next().unwrap().clone();
    let (out, gathered) = {
        let room = room.lock().await;
        (
            room.peers[&subscriber].out.clone(),
            room.peers[&subscriber].gathered.clone(),
        )
    };
    let job = Forward {
        pc: blocked.clone(),
        out,
        gathered,
        sdp: gate.clone(),
        publication,
    };
    let mut next = Box::pin(sfu.forward_to(job));
    std::future::poll_fn(|cx| {
        assert!(
            next.as_mut().poll(cx).is_pending(),
            "new offer must wait for rollback"
        );
        std::task::Poll::Ready(())
    })
    .await;
    blocked.release.notify_one();
    rollback.await.unwrap();
    next.await;
    assert!(gate.lock().await.have_local_offer);
    assert_eq!(blocked.inner.get_senders().await.len(), 1);
    // Release the second rollback during leave if needed; close itself is unblocked.
    sfu.leave(publisher, channel).await;
    sfu.leave(subscriber, channel).await;
}

#[tokio::test]
async fn udp_port_stays_reserved_until_close_completes() {
    let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let addr = socket.local_addr().unwrap();
    drop(socket);
    let mut config = config();
    config.ice_bind = addr.to_string();
    let sfu = Arc::new(Sfu::new(&config));
    let channel = Uuid::new_v4();
    let (peer_id, _rx) = join(&sfu, channel).await;
    let blocked = {
        let room = sfu.find_room(channel).await.unwrap();
        let mut room = room.lock().await;
        let peer = room.peers.get_mut(&peer_id).unwrap();
        let blocked = Arc::new(BlockPc {
            inner: peer.pc.clone(),
            entered: tokio::sync::Notify::new(),
            release: tokio::sync::Notify::new(),
            block_close: true,
        });
        peer.pc = blocked.clone();
        blocked
    };
    let leave = {
        let sfu = sfu.clone();
        tokio::spawn(async move {
            sfu.leave(peer_id, channel).await;
        })
    };
    blocked.entered.notified().await;
    assert!(
        sfu.ice_ports.take().await.is_none(),
        "port is still bound during close"
    );
    assert_eq!(sfu.room_count(), 0);
    blocked.release.notify_one();
    leave.await.unwrap();
    assert_eq!(sfu.ice_ports.take().await, Some(addr.to_string()));
}

#[path = "../tests/support/authority.rs"]
mod auth_fixture;

async fn authorized_fixture(
    sfu: &Arc<Sfu>,
    redis: &redis::Client,
) -> (
    crate::ticket::AuthorizedTicketClaim,
    auth_fixture::TestAuthority,
) {
    let code = gelabber_shared::ticket::generate();
    let lease = auth_fixture::mint(
        redis,
        &code,
        TicketClaim {
            u: Uuid::new_v4(),
            s: Uuid::new_v4(),
            c: Uuid::new_v4(),
            g: true,
        },
    )
    .await;
    let claim = crate::ticket::consume(redis, &code).await.unwrap().unwrap();
    assert_eq!(sfu.room_count(), 0);
    (claim, lease)
}
fn authority_redis() -> redis::Client {
    redis::Client::open(
        std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into()),
    )
    .unwrap()
}

#[tokio::test]
async fn revoke_while_join_waits_for_attach_lock_is_rechecked_after_build() {
    let redis = authority_redis();
    let sfu = Arc::new(Sfu::with_redis(&config(), Some(redis.clone())));
    let (claim, _lease) = authorized_fixture(&sfu, &redis).await;
    let mut conn = redis.get_multiplexed_async_connection().await.unwrap();
    let demand = gelabber_shared::ticket::media_demand_key(&claim.auth.session);
    let _: () = redis::cmd("DEL")
        .arg(&demand)
        .query_async(&mut conn)
        .await
        .unwrap();
    let guard = sfu.rooms.write().await;
    let join = {
        let sfu = sfu.clone();
        let claim = claim.clone();
        tokio::spawn(async move {
            let (out, _) = mpsc::unbounded_channel();
            sfu.join_authorized(claim, out).await
        })
    };
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let valid: bool = redis::cmd("EXISTS")
                .arg(&demand)
                .query_async(&mut conn)
                .await
                .unwrap();
            if valid {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    // First validation passed; attachment is blocked behind our barrier.
    let _: () = redis::cmd("SET")
        .arg(gelabber_shared::ticket::member_authority_key(
            claim.claim.s,
            claim.claim.u,
        ))
        .arg(Uuid::new_v4().to_string())
        .query_async(&mut conn)
        .await
        .unwrap();
    drop(guard);
    assert!(matches!(join.await.unwrap(), Err(SfuError::Revoked)));
    assert_eq!(sfu.room_count(), 0);
}

#[tokio::test]
async fn revoke_signals_publisher_stop_before_busy_sdp_cleanup() {
    let redis = authority_redis();
    let sfu = Arc::new(Sfu::with_redis(&config(), Some(redis.clone())));
    let (claim, _lease) = authorized_fixture(&sfu, &redis).await;
    let (out, _rx) = mpsc::unbounded_channel();
    let id = sfu.join_authorized(claim.clone(), out).await.unwrap();
    let channel = claim.claim.c;
    sfu.announce_track(id, channel, "l", Some("live"))
        .await
        .unwrap();
    let (track, _events) = remote("live");
    sfu.publish(id, channel, track).await.unwrap();
    let room = sfu.find_room(channel).await.unwrap();
    let (gate, closing, life) = {
        let room = room.lock().await;
        (
            room.peers[&id].sdp.clone(),
            room.peers[&id].closing.clone(),
            room.pubs[&format!("{}:live", id.0)].life.clone(),
        )
    };
    let guard = gate.lock().await;
    let mut conn = redis.get_multiplexed_async_connection().await.unwrap();
    let _: () = redis::cmd("SET")
        .arg(gelabber_shared::ticket::member_authority_key(
            claim.claim.s,
            claim.claim.u,
        ))
        .arg(Uuid::new_v4().to_string())
        .query_async(&mut conn)
        .await
        .unwrap();
    let mut stopped = life.stop.subscribe();
    let mut closed = closing.subscribe();
    tokio::time::timeout(Duration::from_millis(1700), async {
        closed.wait_for(|closed| *closed).await.unwrap();
        stopped.wait_for(|stopped| *stopped).await.unwrap();
    })
    .await
    .expect("authority removal stops media before waiting for SDP");
    assert_eq!(sfu.room_count(), 0);
    drop(guard);
    let mut done = life.done.clone();
    tokio::time::timeout(Duration::from_secs(1), done.wait_for(|done| *done))
        .await
        .unwrap()
        .unwrap();
}
