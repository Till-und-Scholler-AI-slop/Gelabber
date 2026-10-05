//! Same-MSID replacement keeps grants and subscribers bound to their generation.
use super::*;

async fn fixture() -> (Arc<Sfu>, Uuid, PeerId, Arc<Mutex<Room>>) {
    let config = Config::from_source(|key| match key {
        "REDIS_URL" => Some("redis://127.0.0.1:1".into()),
        "MEDIA_ICE_BIND" => Some("127.0.0.1:0".into()),
        _ => None,
    })
    .unwrap();
    let sfu = Arc::new(Sfu::new(&config));
    let channel = Uuid::new_v4();
    let (out, _frames) = mpsc::unbounded_channel();
    let peer = sfu
        .join_inner(
            TicketClaim {
                u: Uuid::new_v4(),
                s: Uuid::new_v4(),
                c: channel,
                g: true,
            },
            None,
            None,
            3,
            out,
        )
        .await
        .unwrap();
    let room = sfu.find_room(channel).await.unwrap();
    (sfu, channel, peer, room)
}
fn publication(
    peer: PeerId,
    track: &str,
    kind: &str,
    grant: Arc<()>,
    parent: Option<Arc<()>>,
    done: bool,
) -> Published {
    let (stop, _) = watch::channel(false);
    let (_done_tx, done) = watch::channel(done);
    Published {
        source_grant: Some(grant),
        parent_grant: parent,
        id: format!("{}:{track}", peer.0),
        publisher: peer,
        track_id: track.into(),
        stream_id: format!("{}:{kind}", Uuid::nil()),
        kind: if matches!(kind, "sa" | "la") {
            RtpCodecKind::Audio
        } else {
            RtpCodecKind::Video
        },
        codec: RTCRtpCodec {
            mime_type: if matches!(kind, "sa" | "la") {
                MIME_TYPE_OPUS.into()
            } else {
                MIME_TYPE_VP8.into()
            },
            clock_rate: if matches!(kind, "sa" | "la") {
                48000
            } else {
                90000
            },
            ..Default::default()
        },
        packets: broadcast::channel(1).0,
        keyframe: None,
        life: Arc::new(PublicationLife { stop, done }),
        live_deadline: None,
        watch_gate: None,
        layered: !matches!(kind, "sa" | "la"),
        layer_ids: Arc::new(StdMutex::new(HashMap::new())),
        received_packets: Arc::new(AtomicU64::new(0)),
    }
}

#[tokio::test]
async fn old_parent_cleanup_preserves_same_id_fresh_grant_and_paired_audio() {
    let (sfu, channel, id, room) = fixture().await;
    let old_grant = Arc::new(());
    let fresh_grant = Arc::new(());
    let audio_grant = Arc::new(());
    let old = publication(id, "same-screen", "s", old_grant, None, true);
    let audio = publication(
        id,
        "same-audio",
        "sa",
        audio_grant.clone(),
        Some(fresh_grant.clone()),
        true,
    );
    {
        let mut room = room.lock().await;
        let peer = room.peers.get_mut(&id).unwrap();
        peer.video_kinds.insert("same-screen".into(), "s".into());
        peer.video_kinds.insert("same-audio".into(), "sa".into());
        peer.source_grants
            .insert("same-screen".into(), fresh_grant.clone());
        peer.source_grants
            .insert("same-audio".into(), audio_grant.clone());
        peer.source_parents
            .insert("same-audio".into(), "same-screen".into());
        peer.source_parent_grants
            .insert("same-audio".into(), fresh_grant.clone());
        peer.rid_recovery.allow_track("same-screen");
        room.pubs.insert(old.id.clone(), old.clone());
        room.pubs.insert(audio.id.clone(), audio.clone());
    }
    sfu.remove_publication(channel, &old.id, &old.life).await;
    {
        let room = room.lock().await;
        let peer = &room.peers[&id];
        assert!(Arc::ptr_eq(
            &peer.source_grants["same-screen"],
            &fresh_grant
        ));
        assert!(peer.rid_recovery.is_announced("same-screen"));
        assert_eq!(peer.video_kinds["same-audio"], "sa");
        assert_eq!(peer.source_parents["same-audio"], "same-screen");
        assert!(Arc::ptr_eq(
            &peer.source_parent_grants["same-audio"],
            &fresh_grant
        ));
        assert!(room.pubs.contains_key(&audio.id));
        assert!(!*audio.life.stop.borrow());
    }
    sfu.leave(id, channel).await;
}

#[tokio::test]
async fn old_parent_cleanup_still_revokes_its_matching_generation_and_audio() {
    let (sfu, channel, id, room) = fixture().await;
    let grant = Arc::new(());
    let audio_grant = Arc::new(());
    let parent = publication(id, "screen", "s", grant.clone(), None, true);
    let audio = publication(
        id,
        "audio",
        "sa",
        audio_grant.clone(),
        Some(grant.clone()),
        true,
    );
    {
        let mut room = room.lock().await;
        let peer = room.peers.get_mut(&id).unwrap();
        peer.video_kinds.insert("screen".into(), "s".into());
        peer.video_kinds.insert("audio".into(), "sa".into());
        peer.source_grants.insert("screen".into(), grant.clone());
        peer.source_grants.insert("audio".into(), audio_grant);
        peer.source_parents.insert("audio".into(), "screen".into());
        peer.source_parent_grants.insert("audio".into(), grant);
        peer.rid_recovery.allow_track("screen");
        room.pubs.insert(parent.id.clone(), parent.clone());
        room.pubs.insert(audio.id.clone(), audio.clone());
    }
    sfu.remove_publication(channel, &parent.id, &parent.life)
        .await;
    {
        let room = room.lock().await;
        let peer = &room.peers[&id];
        assert!(!peer.source_grants.contains_key("screen"));
        assert!(!peer.rid_recovery.is_announced("screen"));
        assert!(!peer.source_grants.contains_key("audio"));
        assert!(!peer.source_parent_grants.contains_key("audio"));
        assert_eq!(peer.video_kinds["audio"], "");
        assert!(!room.pubs.contains_key(&audio.id));
        assert!(*audio.life.stop.borrow());
    }
    sfu.leave(id, channel).await;
}

#[tokio::test]
async fn old_audio_publication_stops_even_after_metadata_rebinds_to_fresh_parent() {
    let (sfu, channel, id, room) = fixture().await;
    let old_grant = Arc::new(());
    let fresh_grant = Arc::new(());
    let fresh_audio_grant = Arc::new(());
    let parent = publication(id, "same-screen", "s", old_grant.clone(), None, true);
    let old_audio = publication(id, "same-audio", "sa", Arc::new(()), Some(old_grant), true);
    {
        let mut room = room.lock().await;
        let peer = room.peers.get_mut(&id).unwrap();
        peer.video_kinds.insert("same-screen".into(), "s".into());
        peer.video_kinds.insert("same-audio".into(), "sa".into());
        peer.source_grants
            .insert("same-screen".into(), fresh_grant.clone());
        peer.source_grants
            .insert("same-audio".into(), fresh_audio_grant.clone());
        peer.source_parents
            .insert("same-audio".into(), "same-screen".into());
        peer.source_parent_grants
            .insert("same-audio".into(), fresh_grant.clone());
        peer.rid_recovery.allow_track("same-screen");
        room.pubs.insert(parent.id.clone(), parent.clone());
        room.pubs.insert(old_audio.id.clone(), old_audio.clone());
    }
    sfu.remove_publication(channel, &parent.id, &parent.life)
        .await;
    {
        let room = room.lock().await;
        let peer = &room.peers[&id];
        assert!(!room.pubs.contains_key(&old_audio.id));
        assert!(*old_audio.life.stop.borrow());
        assert!(Arc::ptr_eq(
            &peer.source_grants["same-audio"],
            &fresh_audio_grant
        ));
        assert!(Arc::ptr_eq(
            &peer.source_parent_grants["same-audio"],
            &fresh_grant
        ));
        assert_eq!(peer.video_kinds["same-audio"], "sa");
        assert!(peer.rid_recovery.is_announced("same-screen"));
    }
    sfu.leave(id, channel).await;
}

#[tokio::test]
async fn old_reader_end_preserves_same_wrapper_fresh_camera_or_screen_grant() {
    let (sfu, channel, id, room) = fixture().await;
    sfu.announce_track(id, channel, "v", Some("same-track"))
        .await
        .unwrap();
    let (track, events) = super::lifecycle_tests::remote("same-track");
    sfu.publish(id, channel, track.clone()).await.unwrap();
    let old_life = room.lock().await.pubs[&format!("{}:same-track", id.0)]
        .life
        .clone();
    // A fresh explicit kind grant can arrive while the old reader is ending.
    sfu.announce_track(id, channel, "s", Some("same-track"))
        .await
        .unwrap();
    let fresh_grant = room.lock().await.peers[&id].source_grants["same-track"].clone();
    drop(events);
    let mut done = old_life.done.clone();
    tokio::time::timeout(Duration::from_secs(2), done.wait_for(|done| *done))
        .await
        .unwrap()
        .unwrap();
    // Await actual room cleanup, not just the reader's done notification.
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if !room
                .lock()
                .await
                .pubs
                .contains_key(&format!("{}:same-track", id.0))
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    {
        let room = room.lock().await;
        let peer = &room.peers[&id];
        assert_eq!(peer.video_kinds["same-track"], "s");
        assert!(Arc::ptr_eq(&peer.source_grants["same-track"], &fresh_grant));
        assert!(peer.rid_recovery.is_announced("same-track"));
        assert!(Arc::ptr_eq(&peer.remote_tracks["same-track"], &track));
    }
    sfu.leave(id, channel).await;
}

#[tokio::test]
async fn old_source_audio_parent_cannot_revoke_reused_camera_identity() {
    let (sfu, channel, id, room) = fixture().await;
    sfu.announce_track(id, channel, "s", Some("screen"))
        .await
        .unwrap();
    sfu.announce_track(id, channel, "sa", Some("reused"))
        .await
        .unwrap();
    let (parent, audio) = {
        let room = room.lock().await;
        let peer = &room.peers[&id];
        (
            publication(
                id,
                "screen",
                "s",
                peer.source_grants["screen"].clone(),
                None,
                true,
            ),
            publication(
                id,
                "reused",
                "sa",
                peer.source_grants["reused"].clone(),
                Some(peer.source_grants["screen"].clone()),
                true,
            ),
        )
    };
    {
        let mut room = room.lock().await;
        room.pubs.insert(parent.id.clone(), parent);
        room.pubs.insert(audio.id.clone(), audio);
    }
    sfu.announce_track(id, channel, "v", Some("reused"))
        .await
        .unwrap();
    let (camera, events) = super::lifecycle_tests::remote("reused");
    sfu.publish(id, channel, camera).await.unwrap();
    let fresh = room.lock().await.peers[&id].source_grants["reused"].clone();
    sfu.retract_track(id, channel, "s", Some("screen"))
        .await
        .unwrap();
    {
        let room = room.lock().await;
        let peer = &room.peers[&id];
        assert_eq!(peer.video_kinds["reused"], "v");
        assert!(Arc::ptr_eq(&peer.source_grants["reused"], &fresh));
        assert!(!peer.source_parents.contains_key("reused"));
        assert!(!peer.source_parent_grants.contains_key("reused"));
        let publication = &room.pubs[&format!("{}:reused", id.0)];
        assert!(!*publication.life.stop.borrow());
        assert!(publication.parent_grant.is_none());
    }
    drop(events);
    sfu.leave(id, channel).await;
}

#[tokio::test]
async fn delayed_old_detach_preserves_fresh_same_id_pending_subscription() {
    let (sfu, channel, id, room) = fixture().await;
    let (pc, out, gathered, sdp) = {
        let room = room.lock().await;
        let peer = &room.peers[&id];
        (
            peer.pc.clone(),
            peer.out.clone(),
            peer.gathered.clone(),
            peer.sdp.clone(),
        )
    };
    let old = publication(id, "same", "s", Arc::new(()), None, false);
    let fresh = publication(id, "same", "s", Arc::new(()), None, false);
    let mut gate = sdp.lock().await;
    gate.subscriptions
        .insert(old.id.clone(), SubscriptionState::Pending(old.clone()));
    let cleanup = {
        let sfu = sfu.clone();
        let pc = pc.clone();
        let out = out.clone();
        let gathered = gathered.clone();
        let sdp = sdp.clone();
        let old = old.clone();
        tokio::spawn(async move {
            sfu.detach_subscription(&pc, &out, &gathered, &sdp, &old.id, Some(&old.life))
                .await
        })
    };
    tokio::task::yield_now().await;
    gate.subscriptions
        .insert(fresh.id.clone(), SubscriptionState::Pending(fresh.clone()));
    drop(gate);
    cleanup.await.unwrap();
    {
        let gate = sdp.lock().await;
        assert!(Arc::ptr_eq(
            gate.subscriptions[&fresh.id].life(),
            &fresh.life
        ));
        assert!(!gate.dirty);
    }
    sfu.leave(id, channel).await;
}

async fn active(pc: &Arc<dyn PeerConnection>, publication: &Published) -> Subscription {
    let local = Arc::new(TrackLocalStaticRTP::new(MediaStreamTrack::new(
        "capture".into(),
        "active".into(),
        "active".into(),
        RtpCodecKind::Video,
        vec![RTCRtpEncodingParameters {
            codec: publication.codec.clone(),
            ..Default::default()
        }],
    )));
    let sender = pc.add_track(local as Arc<dyn TrackLocal>).await.unwrap();
    Subscription {
        life: publication.life.clone(),
        sender,
        task: tokio::spawn(std::future::pending()),
        codec: publication.codec.clone(),
        payload_type: watch::channel(None).0,
        source: publication.stream_id.clone(),
        layer_intent: watch::channel(viewer_layers::Intent::default()).0,
        alive: watch::channel(true).0,
    }
}

#[tokio::test]
async fn delayed_old_detach_preserves_fresh_same_id_active_subscription() {
    let (sfu, channel, id, room) = fixture().await;
    let (pc, out, gathered, sdp) = {
        let room = room.lock().await;
        let peer = &room.peers[&id];
        (
            peer.pc.clone(),
            peer.out.clone(),
            peer.gathered.clone(),
            peer.sdp.clone(),
        )
    };
    let old = publication(id, "same", "s", Arc::new(()), None, false);
    let fresh = publication(id, "same", "s", Arc::new(()), None, false);
    let subscription = active(&pc, &fresh).await;
    let sender = subscription.sender.clone();
    let alive = subscription.alive.subscribe();
    let task_abort = subscription.task.abort_handle();
    let mut gate = sdp.lock().await;
    gate.subscriptions
        .insert(old.id.clone(), SubscriptionState::Pending(old.clone()));
    let cleanup = {
        let sfu = sfu.clone();
        let pc = pc.clone();
        let out = out.clone();
        let gathered = gathered.clone();
        let sdp = sdp.clone();
        let old = old.clone();
        tokio::spawn(async move {
            sfu.detach_subscription(&pc, &out, &gathered, &sdp, &old.id, Some(&old.life))
                .await
        })
    };
    tokio::task::yield_now().await;
    gate.subscriptions
        .insert(fresh.id.clone(), SubscriptionState::Active(subscription));
    drop(gate);
    cleanup.await.unwrap();
    {
        let gate = sdp.lock().await;
        assert!(Arc::ptr_eq(
            gate.subscriptions[&fresh.id].life(),
            &fresh.life
        ));
        assert!(!gate.dirty);
        assert!(*alive.borrow());
        assert!(!task_abort.is_finished());
    }
    assert!(
        pc.get_senders()
            .await
            .iter()
            .any(|current| Arc::ptr_eq(current, &sender))
    );
    sfu.leave(id, channel).await;
}

#[tokio::test]
async fn fresh_attach_replaces_ended_same_id_active_reservation() {
    let (sfu, channel, id, room) = fixture().await;
    let (pc, out, gathered, sdp) = {
        let room = room.lock().await;
        let peer = &room.peers[&id];
        (
            peer.pc.clone(),
            peer.out.clone(),
            peer.gathered.clone(),
            peer.sdp.clone(),
        )
    };
    let old = publication(id, "same", "s", Arc::new(()), None, false);
    let fresh = publication(id, "same", "s", Arc::new(()), None, false);
    let subscription = active(&pc, &old).await;
    let alive = subscription.alive.subscribe();
    let task_abort = subscription.task.abort_handle();
    sdp.lock()
        .await
        .subscriptions
        .insert(old.id.clone(), SubscriptionState::Active(subscription));
    old.life.stop.send_replace(true);
    sfu.forward_to(Forward {
        pc,
        out,
        gathered,
        sdp: sdp.clone(),
        publication: fresh.clone(),
    })
    .await;
    {
        let gate = sdp.lock().await;
        assert!(matches!(
            gate.subscriptions.get(&fresh.id),
            Some(SubscriptionState::Pending(_))
        ));
        assert!(Arc::ptr_eq(
            gate.subscriptions[&fresh.id].life(),
            &fresh.life
        ));
        assert!(gate.dirty);
        assert!(!*alive.borrow());
        assert!(task_abort.is_finished());
    }
    sfu.leave(id, channel).await;
}
