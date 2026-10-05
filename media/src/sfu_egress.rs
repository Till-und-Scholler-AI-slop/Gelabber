//! Per-peer audio-first scheduling and cancellable bounded RTC enqueues.
//! Video admission is an entire decoder-reference-safe frame; no bitrate ceiling.
use super::*;
use std::future::Future;

#[derive(Clone, Copy)]
pub(super) struct Pressure {
    pub at: Instant,
    pub discontinuity: bool,
}

pub(super) struct Batch {
    pub local: Arc<TrackLocalStaticRTP>,
    pub packets: VecDeque<rtp::Packet>,
    pub alive: watch::Receiver<bool>,
    pub publication: Published,
    pub queued_at: Instant,
    pub pressure: watch::Sender<Option<Pressure>>,
}
impl Batch {
    fn active(&self) -> bool {
        *self.alive.borrow()
            && !*self.publication.life.stop.borrow()
            && !live_expired(&self.publication.live_deadline)
            && !source_watch_closed(&self.publication)
    }
    async fn write(
        &self,
        packet: rtp::Packet,
        closing: &watch::Receiver<bool>,
        stats: &SfuStats,
    ) -> bool {
        let n = packet.payload.len() as u64;
        let sent = cancellable_write(
            #[cfg(not(test))]
            self.local.write_rtp(packet),
            #[cfg(test)]
            tests::write_rtp(&self.local, packet),
            closing.clone(),
            self.alive.clone(),
            self.publication.life.stop.subscribe(),
            self.publication.watch_gate.clone(),
            self.publication.live_deadline.clone(),
        )
        .await;
        if sent {
            stats.forwarded_bytes.fetch_add(n, Ordering::Relaxed);
        }
        sent
    }
}
struct Worker {
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.task.abort();
    }
}
#[derive(Clone)]
pub(super) struct Egress {
    audio: mpsc::Sender<Batch>,
    video: mpsc::Sender<Batch>,
    _worker: Arc<Worker>,
}
impl Egress {
    pub fn new(stats: Arc<SfuStats>, mut closing: watch::Receiver<bool>) -> Self {
        let (audio, mut audio_rx) = mpsc::channel::<Batch>(16);
        let (video, mut video_rx) = mpsc::channel::<Batch>(4);
        let task = tokio::spawn(async move {
            let mut current: Option<Batch> = None;
            loop {
                if *closing.borrow() {
                    break;
                }
                let mut audio_batch = audio_rx.try_recv().ok();
                if audio_batch.is_none() && current.is_none() {
                    tokio::select! {biased;_=closing.changed()=>break,
                    Some(batch)=audio_rx.recv()=>audio_batch=Some(batch),
                    Some(batch)=video_rx.recv()=>{
                            if batch.queued_at.elapsed()>Duration::from_millis(150){batch.pressure.send_replace(Some(Pressure { at: Instant::now(), discontinuity: false }));}
                            current=Some(batch);
                        },else=>break}
                }
                if let Some(mut batch) = audio_batch {
                    while let Some(packet) = batch.packets.pop_front() {
                        if !batch.active() || !batch.write(packet, &closing, &stats).await {
                            break;
                        }
                    }
                    continue;
                }
                let Some(batch) = current.as_ref() else {
                    continue;
                };
                if !batch.active() {
                    current = None;
                    continue;
                }
                let Some(packet) = batch.packets.front().cloned() else {
                    current = None;
                    continue;
                };
                // Keep the batch on the worker stack instead of allocating on
                // every audio preemption; this transient enum is never queued.
                #[allow(clippy::large_enum_variant)]
                enum Step {
                    Audio(Batch),
                    Video(bool),
                }
                // A pending bounded driver send is cancel-safe. Incoming audio
                // cancels its reservation and gets the next driver queue slot.
                let step = tokio::select! {biased;
                    Some(audio)=audio_rx.recv()=>Step::Audio(audio),
                    sent=batch.write(packet,&closing,&stats)=>Step::Video(sent),
                };
                match step {
                    Step::Audio(mut audio) => {
                        while let Some(packet) = audio.packets.pop_front() {
                            if !audio.active() || !audio.write(packet, &closing, &stats).await {
                                break;
                            }
                        }
                    }
                    Step::Video(true) => {
                        current.as_mut().unwrap().packets.pop_front();
                    }
                    Step::Video(false) => {
                        if let Some(batch) = &current
                            && batch.active()
                            && !*closing.borrow()
                        {
                            // A real write failure may have interrupted a frame.
                            // Repair references before accepting more deltas.
                            batch.pressure.send_replace(Some(Pressure {
                                at: Instant::now(),
                                discontinuity: true,
                            }));
                        }
                        current = None;
                    }
                }
                tokio::task::yield_now().await;
            }
        });
        Self {
            audio,
            video,
            _worker: Arc::new(Worker { task }),
        }
    }
    pub async fn audio(&self, batch: Batch) -> bool {
        self.audio.send(batch).await.is_ok()
    }
    pub async fn legacy_video(&self, batch: Batch) -> bool {
        self.video.send(batch).await.is_ok()
    }
    pub fn video(&self, batch: Batch) -> bool {
        self.video.try_send(batch).is_ok()
    }
}

/// The pin clones its driver sender before awaiting bounded send and never
/// rechecks unbind afterwards. Retain the old subscription-task abort fence.
async fn cancellable_write<F>(
    write: F,
    mut closing: watch::Receiver<bool>,
    mut alive: watch::Receiver<bool>,
    mut stopped: watch::Receiver<bool>,
    mut intent: Option<watch::Receiver<bool>>,
    deadline: Option<Arc<StdMutex<Instant>>>,
) -> bool
where
    F: Future<Output = webrtc::error::Result<()>>,
{
    tokio::pin!(write);
    loop {
        if *closing.borrow()
            || !*alive.borrow()
            || *stopped.borrow()
            || live_expired(&deadline)
            || intent.as_ref().is_some_and(|w| !*w.borrow())
        {
            return false;
        }
        tokio::select! {biased;
            _=closing.changed()=>return false,
            _=alive.changed()=>return false,
            _=stopped.changed()=>return false,
            _=async{if let Some(w)=&mut intent{let _=w.changed().await;}else{std::future::pending::<()>().await;}}=>return false,
            _=tokio::time::sleep_until(live_wakeup(&deadline)),if deadline.is_some()=>{},
            result=&mut write=>return result.is_ok(),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::OnceLock;

    #[derive(Clone)]
    struct Driver {
        packets: mpsc::Sender<rtp::Packet>,
        entered: mpsc::UnboundedSender<(u32, u16)>,
    }
    fn drivers() -> &'static StdMutex<HashMap<usize, Driver>> {
        static DRIVERS: OnceLock<StdMutex<HashMap<usize, Driver>>> = OnceLock::new();
        DRIVERS.get_or_init(Default::default)
    }
    struct DriverGuard {
        // Retaining the track prevents address reuse before this registry entry
        // is removed, including when an assertion unwinds the fixture.
        track: Arc<TrackLocalStaticRTP>,
    }
    impl Drop for DriverGuard {
        fn drop(&mut self) {
            drivers()
                .lock()
                .unwrap()
                .remove(&(Arc::as_ptr(&self.track) as usize));
        }
    }
    fn register_driver(local: &Arc<TrackLocalStaticRTP>, driver: Driver) -> DriverGuard {
        drivers()
            .lock()
            .unwrap()
            .insert(Arc::as_ptr(local) as usize, driver);
        DriverGuard {
            track: local.clone(),
        }
    }
    pub(super) async fn write_rtp(
        local: &Arc<TrackLocalStaticRTP>,
        packet: rtp::Packet,
    ) -> webrtc::error::Result<()> {
        let driver = drivers()
            .lock()
            .unwrap()
            .get(&(Arc::as_ptr(local) as usize))
            .cloned();
        if let Some(driver) = driver {
            let _ = driver
                .entered
                .send((packet.header.ssrc, packet.header.sequence_number));
            // The pinned TrackLocalStaticRTP delegates to this same cancel-safe
            // bounded send. Only registered fixture tracks use this seam.
            driver
                .packets
                .send(packet)
                .await
                .map_err(|_| webrtc::error::Error::Other("fixture driver closed".into()))
        } else {
            local.write_rtp(packet).await
        }
    }
    fn local(kind: RtpCodecKind) -> Arc<TrackLocalStaticRTP> {
        Arc::new(TrackLocalStaticRTP::new(MediaStreamTrack::new(
            "egress-fixture".into(),
            Uuid::new_v4().to_string(),
            "egress-fixture".into(),
            kind,
            vec![],
        )))
    }
    fn batch(
        local: Arc<TrackLocalStaticRTP>,
        kind: RtpCodecKind,
        packets: Vec<rtp::Packet>,
        alive: watch::Receiver<bool>,
    ) -> Batch {
        let (stop, _) = watch::channel(false);
        let (_, done) = watch::channel(false);
        let (pressure, _) = watch::channel(None);
        Batch {
            local,
            packets: packets.into(),
            alive,
            publication: Published {
                source_grant: None,
                parent_grant: None,
                id: "egress-fixture".into(),
                publisher: PeerId(Uuid::new_v4()),
                track_id: "egress-fixture".into(),
                stream_id: "egress-fixture".into(),
                kind,
                codec: RTCRtpCodec::default(),
                packets: broadcast::channel(1).0,
                keyframe: None,
                life: Arc::new(PublicationLife { stop, done }),
                live_deadline: None,
                watch_gate: None,
                layered: kind == RtpCodecKind::Video,
                layer_ids: Arc::new(StdMutex::new(HashMap::new())),
                received_packets: Arc::new(AtomicU64::new(0)),
            },
            queued_at: Instant::now(),
            pressure,
        }
    }
    fn packet(ssrc: u32, sequence_number: u16) -> rtp::Packet {
        rtp::Packet {
            header: rtp::Header {
                ssrc,
                sequence_number,
                ..Default::default()
            },
            payload: vec![ssrc as u8, sequence_number as u8].into(),
        }
    }

    #[tokio::test]
    async fn audio_preempts_a_held_video_enqueue_and_video_retries_exactly_once() {
        let (driver, mut outgoing) = mpsc::channel(1);
        let (entered, mut attempts) = mpsc::unbounded_channel();
        // No application packet can get a slot until the fixture releases it.
        driver.send(packet(99, 0)).await.unwrap();
        let video = local(RtpCodecKind::Video);
        let audio = local(RtpCodecKind::Audio);
        let seam = Driver {
            packets: driver,
            entered,
        };
        let video_guard = register_driver(&video, seam.clone());
        let audio_guard = register_driver(&audio, seam);
        let (peer, closing) = watch::channel(false);
        let (_owner, alive) = watch::channel(true);
        let stats = Arc::new(SfuStats::default());
        let egress = Egress::new(stats.clone(), closing);
        assert!(egress.video(batch(
            video.clone(),
            RtpCodecKind::Video,
            vec![packet(2, 20), packet(2, 21)],
            alive.clone(),
        )));
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), attempts.recv())
                .await
                .unwrap(),
            Some((2, 20)),
            "video entered the full driver queue",
        );
        assert!(
            egress
                .audio(batch(
                    audio.clone(),
                    RtpCodecKind::Audio,
                    vec![packet(1, 10)],
                    alive
                ))
                .await
        );
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), attempts.recv())
                .await
                .unwrap(),
            Some((1, 10)),
            "audio cancelled the blocked video reservation before a slot was freed",
        );
        assert_eq!(outgoing.recv().await.unwrap().header.ssrc, 99);
        let mut delivered = Vec::new();
        for _ in 0..3 {
            let sent = tokio::time::timeout(Duration::from_secs(2), outgoing.recv())
                .await
                .unwrap()
                .unwrap();
            delivered.push((
                sent.header.ssrc,
                sent.header.sequence_number,
                sent.payload.to_vec(),
            ));
        }
        assert_eq!(
            delivered,
            vec![
                (1, 10, vec![1, 10]),
                (2, 20, vec![2, 20]),
                (2, 21, vec![2, 21])
            ],
            "the freed slot sends audio first and retains both original video packets",
        );
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), attempts.recv())
                .await
                .unwrap(),
            Some((2, 20)),
            "retry the cancelled video packet",
        );
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), attempts.recv())
                .await
                .unwrap(),
            Some((2, 21)),
        );
        tokio::task::yield_now().await;
        assert!(outgoing.try_recv().is_err(), "no duplicate driver enqueue");
        assert!(attempts.try_recv().is_err(), "no extra video retry");
        assert_eq!(stats.forwarded_bytes.load(Ordering::Relaxed), 6);
        peer.send_replace(true);
        drop(egress);
        let keys = [Arc::as_ptr(&video) as usize, Arc::as_ptr(&audio) as usize];
        drop(video_guard);
        drop(audio_guard);
        assert!(
            keys.into_iter()
                .all(|key| !drivers().lock().unwrap().contains_key(&key))
        );
    }

    #[test]
    fn fixture_driver_is_scoped_and_removed_when_an_assertion_unwinds() {
        let fixture = local(RtpCodecKind::Video);
        let unrelated = local(RtpCodecKind::Video);
        let key = Arc::as_ptr(&fixture) as usize;
        let unrelated_key = Arc::as_ptr(&unrelated) as usize;
        let (packets, _) = mpsc::channel(1);
        let (entered, _) = mpsc::unbounded_channel();
        let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = register_driver(&fixture, Driver { packets, entered });
            let (registered, unrelated_registered) = {
                let entries = drivers().lock().unwrap();
                (
                    entries.contains_key(&key),
                    entries.contains_key(&unrelated_key),
                )
            };
            assert!(registered);
            assert!(!unrelated_registered);
            panic!("fixture assertion failure");
        }));
        assert!(unwind.is_err());
        assert!(!drivers().lock().unwrap().contains_key(&key));
    }

    #[tokio::test]
    async fn revoked_scopes_cancel_a_held_driver_send_without_a_late_enqueue() {
        for scope in ["peer", "subscription", "publication", "watch", "expiry"] {
            let (tx, mut rx) = mpsc::channel::<u8>(1);
            tx.send(1).await.unwrap();
            let (peer, closing) = watch::channel(false);
            let (owner, alive) = watch::channel(true);
            let (publication, stopped) = watch::channel(false);
            let (viewer, intent) = watch::channel(true);
            let deadline = (scope == "expiry")
                .then(|| Arc::new(StdMutex::new(Instant::now() + Duration::from_millis(20))));
            let write = async move {
                tx.send(2)
                    .await
                    .map_err(|_| webrtc::error::Error::ErrUnknownType)
            };
            let job = tokio::spawn(cancellable_write(
                write,
                closing,
                alive,
                stopped,
                Some(intent),
                deadline,
            ));
            tokio::task::yield_now().await;
            match scope {
                "peer" => {
                    peer.send_replace(true);
                }
                "subscription" => {
                    owner.send_replace(false);
                }
                "publication" => {
                    publication.send_replace(true);
                }
                "watch" => {
                    viewer.send_replace(false);
                }
                _ => {}
            }
            assert!(
                !tokio::time::timeout(Duration::from_millis(200), job)
                    .await
                    .unwrap()
                    .unwrap(),
                "{scope}"
            );
            assert_eq!(rx.recv().await, Some(1));
            assert!(rx.try_recv().is_err(), "no old RTP after {scope} release");
        }
    }
}
