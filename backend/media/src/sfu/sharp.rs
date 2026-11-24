//! GEL-32: viewers of a sharp Live keep its full size on a weak download.
//!
//! mediasoup moves a viewer to a smaller stream on every dip of its download
//! estimate and back up when the estimate recovers. Each switch costs a
//! keyframe, which mediasoup forwards in one burst; a weak link drops much of
//! it, the estimate falls to mediasoup's 30 kbit/s floor and the picture
//! stands still for seconds, then the next switch starts it over.
//!
//! For a sharp Live the gateway pins each viewer to the full-size stream
//! instead: it keeps the viewer transport's minimum outgoing bitrate at what
//! that stream needs at its lowest frame rate, so mediasoup never steps down
//! a stream and only drops frame rate. A viewer whose link cannot carry even
//! that (its consumer score stays low) is released to mediasoup's own choice
//! for a while, with a small floor that still prevents the collapse. Smooth
//! Lives keep mediasoup's own choice. Each viewer can choose sharp or smooth
//! for itself (`viewMode`); without a choice it follows the sender.
//!
//! With more sharp viewers than one tick samples, the gateway samples them in
//! turns; a viewer keeps its last floor between its turns, and only a
//! transport that no longer watches any sharp Live goes back to no floor.
use super::*;

/// How often the gateway samples sharp Lives and their viewers.
const TICK: Duration = Duration::from_millis(500);
/// Samples a viewer is judged on: the last ten seconds while every viewer is
/// sampled each tick.
const WINDOW: usize = 20;
/// Samples before the first pin: two seconds of watching, likewise.
const WARMUP: usize = 4;
/// Headroom over the full-size stream's highest recent rate, in percent.
const PIN_PERCENT: u64 = 110;
/// Minimum estimate on a viewer transport while it watches a sharp Live:
/// enough for the smallest stream, far below what a call needs anyway.
const FLOOR_MIN: u64 = 300_000;
/// The pin never asks a viewer for more than this per sharp Live.
const PIN_MAX: u64 = 3_000_000;
/// A new floor is applied only when it moves by more than this, in percent.
const FLOOR_STEP_PERCENT: u64 = 15;
/// Consumer score (0-10) at or below which the full-size stream is lost on
/// the way: the link cannot carry it.
const POOR_SCORE: u8 = 5;
/// How long a released viewer gets mediasoup's own choice before the next pin.
const RELEASE: Duration = Duration::from_secs(30);
/// Viewers sampled per tick; the others keep their floor and come next.
const VIEWERS_PER_TICK: usize = 64;
/// Temporal layers a simulcast stream carries at most (L1T3).
const TEMPORAL_LAYERS: usize = 3;

/// Bitrate of each simulcast stream up to each temporal layer, cumulative,
/// in bit/s; streams from smallest to full size, 0 where nothing was seen.
pub(super) type LayerRates = Vec<[u32; TEMPORAL_LAYERS]>;

/// One viewer of one sharp Live.
#[derive(Default)]
pub(super) struct SharpViewer {
    rates: VecDeque<u32>,
    scores: VecDeque<u8>,
    /// Minimum outgoing bitrate this viewer needs on its transport.
    floor: Option<u64>,
    released_until: Option<Instant>,
}
impl SharpViewer {
    /// Records one sample: the Live's layer rates and the consumer's score.
    /// Returns the viewer's new floor when it should change.
    pub(super) fn sample(&mut self, rates: &LayerRates, score: u8, now: Instant) -> Option<u64> {
        // The full-size stream at its lowest frame rate.
        let base = rates.last().map_or(0, |s| s[0]);
        if base > 0 {
            push(&mut self.rates, base);
        }
        push(&mut self.scores, score);
        if self.rates.len() < WARMUP {
            return None;
        }
        let released = self.released_until.is_some_and(|until| now < until);
        let poor = self.scores.len() == WINDOW && self.scores.iter().all(|s| *s <= POOR_SCORE);
        let floor = if released {
            FLOOR_MIN
        } else if poor && self.floor.is_some_and(|f| f > FLOOR_MIN) {
            self.released_until = Some(now + RELEASE);
            self.scores.clear();
            FLOOR_MIN
        } else {
            // Keyframes and scrolling make the rate jumpy: pin to its peak.
            let peak = u64::from(self.rates.iter().copied().max().unwrap_or(0));
            (peak * PIN_PERCENT / 100).clamp(FLOOR_MIN, PIN_MAX)
        };
        let moved = self.floor.is_none_or(|old| {
            floor.abs_diff(old) * 100 > old * FLOOR_STEP_PERCENT
                || (floor == FLOOR_MIN) != (old == FLOOR_MIN)
        });
        if !moved {
            return None;
        }
        self.floor = Some(floor);
        Some(floor)
    }
    /// The floor in force, if one was set.
    pub(super) fn floor(&self) -> Option<u64> {
        self.floor
    }
}

fn push<T>(samples: &mut VecDeque<T>, value: T) {
    samples.push_back(value);
    while samples.len() > WINDOW {
        samples.pop_front();
    }
}

/// Per-stream, per-temporal-layer bitrates from a producer's stats, with the
/// streams in the order of the producer's encodings. An SVC producer has one
/// encoding whose spatial layers count as streams here.
pub(super) fn layer_rates(
    encodings: &[RtpEncodingParameters],
    stats: &[mediasoup::producer::ProducerStat],
) -> LayerRates {
    let mut rates = vec![[0; TEMPORAL_LAYERS]; encodings.len().max(1)];
    for stat in stats {
        let index = encodings
            .iter()
            .position(|e| match (&e.rid, &stat.rid) {
                (Some(a), Some(b)) => a == b,
                _ => e.ssrc == Some(stat.ssrc),
            })
            .unwrap_or(0);
        // `BitrateByLayer` keeps its fields private; its serde form is public.
        let Ok(Value::Array(layers)) = serde_json::to_value(&stat.bitrate_by_layer) else {
            continue;
        };
        for layer in layers {
            // "spatial.temporal"; simulcast streams report spatial 0.
            let mut key = layer["layer"]
                .as_str()
                .unwrap_or_default()
                .split('.')
                .map(|n| n.parse::<usize>().ok());
            let (Some(Some(spatial)), Some(Some(temporal)), Some(bitrate)) =
                (key.next(), key.next(), layer["bitrate"].as_u64())
            else {
                continue;
            };
            let stream = index + spatial;
            if temporal >= TEMPORAL_LAYERS || stream >= TEMPORAL_LAYERS.max(encodings.len()) {
                continue;
            }
            if rates.len() <= stream {
                rates.resize(stream + 1, [0; TEMPORAL_LAYERS]);
            }
            rates[stream][temporal] = u32::try_from(bitrate).unwrap_or(u32::MAX);
        }
    }
    rates
}

/// Indices of the viewers a tick samples: up to `n` of `sorted`, starting
/// after `after`, the viewer sampled last, and wrapping around, so every
/// viewer gets its turn however many there are.
pub(super) fn turn<K: Ord>(sorted: &[K], after: Option<&K>, n: usize) -> Vec<usize> {
    let start = after.map_or(0, |after| sorted.partition_point(|k| k <= after));
    (start..start + n.min(sorted.len()))
        .map(|i| i % sorted.len())
        .collect()
}

/// The floor each viewer transport needs: the sum of the floors of all its
/// sharp viewers, sampled this tick or not, `FLOOR_MIN` for a viewer without
/// one yet.
pub(super) fn transport_floors<T: Copy + Eq + std::hash::Hash>(
    viewers: impl IntoIterator<Item = (T, Option<u64>)>,
) -> HashMap<T, u64> {
    let mut floors = HashMap::new();
    for (transport, floor) in viewers {
        let sum: &mut u64 = floors.entry(transport).or_default();
        *sum = sum.saturating_add(floor.unwrap_or(FLOOR_MIN));
    }
    floors
}

/// The floors a tick applies, 0 to reset. A transport that watches a sharp
/// Live gets what its viewers need (`wanted`) when that differs from the floor
/// last applied to it (`applied`); one without an applied floor waits until
/// one of its viewers is sampled (`sampled`). A transport that no longer
/// watches one goes back to 0. The caller updates `applied` only for requests
/// that succeeded, so a failed one is asked again: on the next tick for a
/// transport that had a floor, on its next turn for one that had none.
pub(super) fn floor_changes<T: Copy + Eq + std::hash::Hash>(
    applied: &HashMap<T, u64>,
    wanted: &HashMap<T, u64>,
    sampled: &HashSet<T>,
) -> Vec<(T, u64)> {
    let set = wanted.iter().filter(|(id, floor)| match applied.get(id) {
        Some(old) => old != *floor,
        None => sampled.contains(id),
    });
    let reset = applied
        .keys()
        .filter(|id| !wanted.contains_key(id))
        .map(|id| (id, &0));
    set.chain(reset).map(|(id, floor)| (*id, *floor)).collect()
}

impl Sfu {
    /// Pins every viewer of a sharp Live to its full size, see the module docs.
    pub(super) fn keep_sharp(weak: Weak<Self>) {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(TICK);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut viewers: HashMap<ConsumerId, SharpViewer> = HashMap::new();
            // Floor last applied per viewer transport.
            let mut floors: HashMap<TransportId, (Arc<Engine>, WebRtcTransport, u64)> =
                HashMap::new();
            // The viewer sampled last; the next tick goes on after it.
            let mut cursor: Option<ConsumerId> = None;
            loop {
                interval.tick().await;
                let Some(sfu) = weak.upgrade() else { break };
                let rooms: Vec<_> = sfu.rooms.lock().await.values().cloned().collect();
                let mut sharp = Vec::new();
                for room in rooms {
                    let data = room.data.lock().await;
                    for p in data.peers.values() {
                        sharp.extend(
                            p.consumers
                                .values()
                                .filter(|s| s.publication.kind == SourceKind::Screen && s.valid())
                                .filter(|s| wants_sharp(p, &s.publication))
                                .map(|s| (room.engine.clone(), s.clone())),
                        );
                    }
                }
                sharp.sort_by_key(|(_, s)| s.consumer.id());
                let ids: Vec<ConsumerId> = sharp.iter().map(|(_, s)| s.consumer.id()).collect();
                let watching: HashSet<ConsumerId> = ids.iter().copied().collect();
                // Viewers not sampled this tick keep their state and floor.
                viewers.retain(|id, _| watching.contains(id));
                let mut rates: HashMap<ProducerId, LayerRates> = HashMap::new();
                let mut sampled: HashSet<TransportId> = HashSet::new();
                for index in turn(&ids, cursor.as_ref(), VIEWERS_PER_TICK) {
                    let s = &sharp[index].1;
                    let producer = s.publication.producer.id();
                    if let std::collections::hash_map::Entry::Vacant(slot) = rates.entry(producer) {
                        let stats =
                            tokio::time::timeout(STOP_TIMEOUT, s.publication.producer.get_stats())
                                .await
                                .ok()
                                .and_then(Result::ok)
                                .unwrap_or_default();
                        let encodings = s
                            .publication
                            .producer
                            .resource
                            .with(|p| p.rtp_parameters().encodings.clone())
                            .unwrap_or_default();
                        slot.insert(layer_rates(&encodings, &stats));
                    }
                    let score = s.consumer.resource.with(|c| c.score().score).unwrap_or(10);
                    let viewer = viewers.entry(s.consumer.id()).or_default();
                    if let Some(floor) = viewer.sample(&rates[&producer], score, Instant::now()) {
                        tracing::debug!(consumer = %s.consumer.id(), floor, "sharp live: viewer floor");
                    }
                    sampled.insert(s.consumer.transport().id());
                    cursor = Some(s.consumer.id());
                }
                let mut handles: HashMap<TransportId, (Arc<Engine>, WebRtcTransport)> =
                    HashMap::new();
                let wanted = transport_floors(sharp.iter().map(|(engine, s)| {
                    let transport = s.consumer.transport();
                    let id = transport.id();
                    handles
                        .entry(id)
                        .or_insert_with(|| (engine.clone(), transport.clone()));
                    (
                        id,
                        viewers.get(&s.consumer.id()).and_then(SharpViewer::floor),
                    )
                }));
                let applied = floors.iter().map(|(id, (_, _, f))| (*id, *f)).collect();
                for (id, floor) in floor_changes(&applied, &wanted, &sampled) {
                    let Some((engine, transport)) = handles
                        .get(&id)
                        .map(|(e, t)| (e, t))
                        .or_else(|| floors.get(&id).map(|(e, t, _)| (e, t)))
                    else {
                        continue;
                    };
                    let (engine, transport) = (engine.clone(), transport.clone());
                    if transport.closed() {
                        floors.remove(&id);
                        continue;
                    }
                    let t = transport.clone();
                    match engine
                        .native(t, move |t| async move {
                            t.set_min_outgoing_bitrate(floor).await
                        })
                        .await
                    {
                        Ok(()) if floor == 0 => {
                            floors.remove(&id);
                        }
                        Ok(()) => {
                            floors.insert(id, (engine, transport, floor));
                        }
                        // The last applied floor stays recorded, so the
                        // request is asked again.
                        Err(error) => {
                            tracing::debug!(%error, floor, "sharp live: floor not applied");
                        }
                    }
                }
            }
        });
    }
}

/// The viewer's own choice for this Live, else the sender's.
pub(super) fn wants_sharp(viewer: &PeerData, live: &Publication) -> bool {
    viewer
        .view_modes
        .get(&live.peer.authority.claim.u)
        .copied()
        .unwrap_or_else(|| live.sharp.load(Ordering::Relaxed))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Three streams; the full-size one needs `full` kbit/s at its lowest
    /// frame rate.
    fn live(full: u32) -> LayerRates {
        vec![
            [100_000, 150_000, 250_000],
            [300_000, 450_000, 700_000],
            [full * 1000, full * 1500, full * 3000],
        ]
    }

    fn feed(viewer: &mut SharpViewer, full: u32, score: u8, n: usize, at: Instant) -> Option<u64> {
        let mut last = None;
        for _ in 0..n {
            last = viewer.sample(&live(full), score, at).or(last);
        }
        last
    }

    #[test]
    fn pins_to_the_full_size_stream_after_a_short_warmup() {
        let mut viewer = SharpViewer::default();
        let now = Instant::now();
        assert_eq!(feed(&mut viewer, 900, 10, WARMUP - 1, now), None);
        // 900 kbit/s at the lowest frame rate, plus headroom.
        assert_eq!(feed(&mut viewer, 900, 10, 1, now), Some(990_000));
    }

    #[test]
    fn keyframe_peaks_raise_the_pin() {
        let mut viewer = SharpViewer::default();
        let now = Instant::now();
        feed(&mut viewer, 900, 10, WARMUP, now);
        assert_eq!(feed(&mut viewer, 1600, 10, 1, now), Some(1_760_000));
        // The peak holds for the whole window, then the pin follows the rate down.
        assert_eq!(feed(&mut viewer, 900, 10, WINDOW - 1, now), None);
        assert_eq!(feed(&mut viewer, 900, 10, 1, now), Some(990_000));
    }

    #[test]
    fn small_moves_are_not_applied() {
        let mut viewer = SharpViewer::default();
        let now = Instant::now();
        feed(&mut viewer, 900, 10, WARMUP, now);
        assert_eq!(feed(&mut viewer, 1000, 10, 1, now), None);
        assert_eq!(viewer.floor(), Some(990_000));
    }

    #[test]
    fn the_pin_has_a_ceiling() {
        let mut viewer = SharpViewer::default();
        assert_eq!(
            feed(&mut viewer, 5000, 10, WARMUP, Instant::now()),
            Some(PIN_MAX)
        );
    }

    #[test]
    fn a_link_that_cannot_carry_full_size_is_released_for_a_while() {
        let mut viewer = SharpViewer::default();
        let start = Instant::now();
        feed(&mut viewer, 900, 10, WARMUP, start);
        // Ten seconds of a poor score: the full-size stream does not get through.
        assert_eq!(feed(&mut viewer, 900, 3, WINDOW, start), Some(FLOOR_MIN));
        assert_eq!(
            feed(&mut viewer, 900, 10, WINDOW, start + RELEASE / 2),
            None
        );
        assert_eq!(viewer.floor(), Some(FLOOR_MIN));
        assert_eq!(
            feed(&mut viewer, 900, 10, 1, start + RELEASE),
            Some(990_000)
        );
    }

    #[test]
    fn a_short_poor_spell_keeps_the_pin() {
        let mut viewer = SharpViewer::default();
        let now = Instant::now();
        feed(&mut viewer, 900, 10, WINDOW, now);
        assert_eq!(feed(&mut viewer, 900, 3, WINDOW - 1, now), None);
        assert_eq!(feed(&mut viewer, 900, 10, 1, now), None);
        assert_eq!(viewer.floor(), Some(990_000));
    }

    #[test]
    fn no_rate_yet_means_no_pin() {
        let mut viewer = SharpViewer::default();
        let now = Instant::now();
        for _ in 0..WINDOW {
            assert_eq!(viewer.sample(&vec![[0; 3]; 3], 10, now), None);
        }
    }

    #[test]
    fn every_viewer_gets_its_turn() {
        let ids: Vec<u32> = (0..150).collect();
        let mut cursor = None;
        let mut ticks = Vec::new();
        for _ in 0..3 {
            let turn = turn(&ids, cursor.as_ref(), VIEWERS_PER_TICK);
            cursor = turn.last().map(|i| ids[*i]);
            ticks.push(turn);
        }
        assert_eq!(ticks[0], (0..64).collect::<Vec<_>>());
        assert_eq!(ticks[1], (64..128).collect::<Vec<_>>());
        assert_eq!(ticks[2], (128..150).chain(0..42).collect::<Vec<_>>());
    }

    #[test]
    fn the_turn_goes_on_after_a_viewer_that_left() {
        // The viewer sampled last (5) is gone; the next one in order comes.
        let ids = [1, 3, 7, 9];
        assert_eq!(turn(&ids, Some(&5), 2), vec![2, 3]);
        assert_eq!(turn(&ids, Some(&9), 2), vec![0, 1]);
        assert_eq!(turn(&ids, Some(&3), 64), vec![2, 3, 0, 1]);
        assert_eq!(turn::<u32>(&[], Some(&3), 64), Vec::<usize>::new());
    }

    #[test]
    fn a_transport_needs_the_floors_of_all_its_viewers() {
        // Transport 1: a viewer sampled this tick (990 kbit/s) and one whose
        // turn comes later (1.76 Mbit/s); transport 2: a new viewer only.
        assert_eq!(
            transport_floors([(1, Some(990_000)), (1, Some(1_760_000)), (2, None)]),
            HashMap::from([(1, 2_750_000), (2, FLOOR_MIN)])
        );
    }

    #[test]
    fn a_skipped_transport_keeps_its_floor() {
        let applied = HashMap::from([(1, 990_000), (2, FLOOR_MIN)]);
        let wanted = applied.clone();
        assert_eq!(floor_changes(&applied, &wanted, &HashSet::new()), vec![]);
    }

    #[test]
    fn only_a_transport_that_stopped_watching_is_reset() {
        let applied = HashMap::from([(1, 990_000), (2, 990_000)]);
        let wanted = HashMap::from([(1, 990_000)]);
        assert_eq!(
            floor_changes(&applied, &wanted, &HashSet::new()),
            vec![(2, 0)]
        );
    }

    #[test]
    fn a_new_transport_gets_its_floor_on_its_turn() {
        let wanted = HashMap::from([(1, FLOOR_MIN)]);
        assert_eq!(
            floor_changes(&HashMap::new(), &wanted, &HashSet::new()),
            vec![]
        );
        assert_eq!(
            floor_changes(&HashMap::new(), &wanted, &HashSet::from([1])),
            vec![(1, FLOOR_MIN)]
        );
    }

    #[test]
    fn a_floor_that_was_not_applied_is_asked_for_again() {
        // The request for 1.76 Mbit/s failed, so 990 kbit/s stays recorded and
        // the next tick asks again, sampled or not.
        let applied = HashMap::from([(1, 990_000)]);
        let wanted = HashMap::from([(1, 1_760_000)]);
        for _ in 0..2 {
            assert_eq!(
                floor_changes(&applied, &wanted, &HashSet::new()),
                vec![(1, 1_760_000)]
            );
        }
    }

    #[test]
    fn layer_rates_follow_the_encoding_order() {
        let encodings: Vec<RtpEncodingParameters> = serde_json::from_value(json!([
            {"rid": "r0"}, {"rid": "r1"}, {"rid": "r2"}
        ]))
        .unwrap();
        let stat = |rid: &str, layers: Value| -> mediasoup::producer::ProducerStat {
            serde_json::from_value(json!({
                "timestamp": 0, "ssrc": 1, "kind": "video", "mimeType": "video/VP8",
                "packetsLost": 0, "fractionLost": 0, "packetsDiscarded": 0,
                "packetsRetransmitted": 0, "packetsRepaired": 0, "nackCount": 0,
                "nackPacketCount": 0, "pliCount": 0, "firCount": 0, "score": 10,
                "rid": rid, "byteCount": 0, "packetCount": 0, "bitrate": 0,
                "roundTripTime": null, "rtxPacketsDiscarded": 0, "jitter": 0,
                "bitrateByLayer": layers,
            }))
            .unwrap()
        };
        let stats = [
            stat(
                "r2",
                json!([{"layer": "0.0", "bitrate": 900}, {"layer": "0.2", "bitrate": 2600}]),
            ),
            stat("r0", json!([{"layer": "0.1", "bitrate": 150}])),
        ];
        assert_eq!(
            layer_rates(&encodings, &stats),
            vec![[0, 150, 0], [0, 0, 0], [900, 0, 2600]]
        );
    }

    #[test]
    fn an_svc_producer_counts_its_spatial_layers_as_streams() {
        let encodings: Vec<RtpEncodingParameters> =
            serde_json::from_value(json!([{"ssrc": 1, "scalabilityMode": "L3T3"}])).unwrap();
        let stat: mediasoup::producer::ProducerStat = serde_json::from_value(json!({
            "timestamp": 0, "ssrc": 1, "kind": "video", "mimeType": "video/VP9",
            "packetsLost": 0, "fractionLost": 0, "packetsDiscarded": 0,
            "packetsRetransmitted": 0, "packetsRepaired": 0, "nackCount": 0,
            "nackPacketCount": 0, "pliCount": 0, "firCount": 0, "score": 10,
            "byteCount": 0, "packetCount": 0, "bitrate": 0,
            "roundTripTime": null, "rtxPacketsDiscarded": 0, "jitter": 0,
            "bitrateByLayer": [
                {"layer": "0.0", "bitrate": 100},
                {"layer": "2.1", "bitrate": 1500},
                {"layer": "7.0", "bitrate": 1}
            ],
        }))
        .unwrap();
        assert_eq!(
            layer_rates(&encodings, &[stat]),
            vec![[100, 0, 0], [0, 0, 0], [0, 1500, 0]]
        );
    }
}
