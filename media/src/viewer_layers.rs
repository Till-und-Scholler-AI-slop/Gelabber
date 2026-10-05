//! Independent VP8 simulcast representations, switched only at complete keyframes.
//! No RTP from two encoder sequence/reference spaces reaches the same decoder.
use rtc::rtp;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::time::{Duration, Instant};

const MAX_LAYERS: usize = 3;
const MAX_FRAME_PACKETS: usize = 2048;
const MAX_FRAME_BYTES: usize = 2 * 1024 * 1024;
const MAX_PENDING_FRAMES: usize = 3;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Intent {
    /// Rendered height including display density; zero means unknown/full quality.
    pub height: u16,
    /// Receiver observed packet loss or a sustained late-frame queue.
    pub congested: bool,
}

pub(super) fn effective_intent(mut intent: Intent, pressure_until: Option<Instant>) -> Intent {
    intent.congested |= pressure_until.is_some_and(|until| until > Instant::now());
    intent
}

#[derive(Clone, Debug)]
pub(super) struct Frame {
    pub ssrc: u32,
    pub key: bool,
    pub height: u16,
    pub packets: Vec<rtp::Packet>,
    pub discontinuity: bool,
}

#[derive(Default)]
struct PartialFrame {
    packets: BTreeMap<u16, rtp::Packet>,
    first: Option<u16>,
    last: Option<u16>,
    bytes: usize,
    key: bool,
    height: u16,
}
#[derive(Default)]
struct Assembly {
    frames: VecDeque<(u32, PartialFrame)>,
    last_complete: Option<u32>,
    last_sequence: Option<u16>,
    discontinuity: bool,
    height: u16,
    padding: VecDeque<u16>,
}
#[derive(Default)]
pub(super) struct Assembler {
    layers: HashMap<u32, Assembly>,
}

impl Assembler {
    pub fn push(&mut self, packet: rtp::Packet) -> Option<Frame> {
        let ssrc = packet.header.ssrc;
        if self.layers.len() >= MAX_LAYERS && !self.layers.contains_key(&ssrc) {
            return None;
        }
        let state = self.layers.entry(ssrc).or_default();
        let padding_only = packet.payload.is_empty() && packet.header.padding;
        let timestamp = if padding_only {
            if state.padding.len() == MAX_FRAME_PACKETS {
                state.padding.pop_front();
            }
            state.padding.push_back(packet.header.sequence_number);
            // Padding can arrive after a frame's marker. Recheck that pending
            // frame when it fills the only missing primary sequence number.
            state.frames.iter().find_map(|(ts, frame)| {
                let (Some(first), Some(last)) = (frame.first, frame.last) else {
                    return None;
                };
                (packet.header.sequence_number.wrapping_sub(first) <= last.wrapping_sub(first))
                    .then_some(*ts)
            })?
        } else {
            Descriptor::parse(&packet.payload)?;
            packet.header.timestamp
        };
        if state.last_complete.is_some_and(|last| {
            let ahead = timestamp.wrapping_sub(last);
            ahead == 0 || ahead >= 0x8000_0000
        }) {
            return None;
        }
        if !state.frames.iter().any(|(ts, _)| *ts == timestamp) {
            if state.frames.len() == MAX_PENDING_FRAMES {
                state.frames.pop_front();
                state.discontinuity = true;
            }
            state.frames.push_back((timestamp, PartialFrame::default()));
        }
        let index = state.frames.iter().position(|(ts, _)| *ts == timestamp)?;
        let frame = &mut state.frames[index].1;
        if !padding_only {
            let descriptor = Descriptor::parse(&packet.payload).unwrap();
            let sequence = packet.header.sequence_number;
            if frame.packets.contains_key(&sequence) {
                return None;
            }
            if frame.packets.len() == MAX_FRAME_PACKETS
                || frame.bytes + packet.payload.len() > MAX_FRAME_BYTES
            {
                state.frames.remove(index);
                state.discontinuity = true;
                return None;
            }
            if descriptor.start {
                frame.first = Some(sequence);
                frame.key = packet
                    .payload
                    .get(descriptor.size)
                    .is_some_and(|b| b & 1 == 0);
            }
            if packet.header.marker {
                frame.last = Some(sequence);
            }
            frame.bytes += packet.payload.len();
            frame.packets.insert(sequence, packet);
        }
        let (Some(first), Some(last)) = (frame.first, frame.last) else {
            return None;
        };
        let count = usize::from(last.wrapping_sub(first)) + 1;
        if count > MAX_FRAME_PACKETS
            || !(0..count).all(|offset| {
                let sequence = first.wrapping_add(offset as u16);
                frame.packets.contains_key(&sequence) || state.padding.contains(&sequence)
            })
        {
            return None;
        }
        let mut frame = state.frames.remove(index)?.1;
        let packets = (0..count)
            .filter_map(|offset| frame.packets.remove(&first.wrapping_add(offset as u16)))
            .collect::<Vec<_>>();
        if frame.key {
            // The uncompressed key header can span RTP fragments. Inspect only
            // after the complete contiguous frame has been assembled.
            let header = packets
                .iter()
                .flat_map(|packet| {
                    let descriptor = Descriptor::parse(&packet.payload).unwrap();
                    packet.payload[descriptor.size..].iter().copied()
                })
                .take(10)
                .collect::<Vec<_>>();
            if header.len() < 10 || header[3..6] != [0x9d, 0x01, 0x2a] {
                state.discontinuity = true;
                return None;
            }
            frame.height = u16::from_le_bytes([header[8], header[9]]) & 0x3fff;
        }
        // A completed newer frame overtaking an incomplete frame loses decoder
        // references. Resume only on a keyframe; never conceal this as continuity.
        let mut discarded_older = false;
        state.frames.retain(|(pending, _)| {
            let ahead = pending.wrapping_sub(timestamp);
            let newer = ahead > 0 && ahead < 0x8000_0000;
            discarded_older |= !newer;
            newer
        });
        let discontinuity = state.discontinuity
            || discarded_older
            || state.last_sequence.is_some_and(|last| {
                let gap = first.wrapping_sub(last.wrapping_add(1));
                gap > MAX_FRAME_PACKETS as u16
                    || !(0..gap).all(|offset| {
                        state
                            .padding
                            .contains(&last.wrapping_add(1).wrapping_add(offset))
                    })
            });
        state.padding.retain(|sequence| {
            let ahead = sequence.wrapping_sub(last);
            ahead > 0 && ahead < 0x8000
        });
        state.last_sequence = Some(last);
        state.discontinuity = false;
        state.last_complete = Some(timestamp);
        if frame.key {
            state.height = frame.height;
        }
        Some(Frame {
            ssrc,
            key: frame.key,
            height: state.height,
            packets,
            discontinuity,
        })
    }
}

/// All descriptor reads are bounds checked, including every optional extension.
#[derive(Clone, Copy)]
struct Descriptor {
    size: usize,
    start: bool,
    picture: Option<(usize, usize)>,
    tl0: Option<usize>,
    tk: Option<usize>,
    temporal: u8,
}
impl Descriptor {
    fn parse(payload: &[u8]) -> Option<Self> {
        let first = *payload.first()?;
        let mut result = Self {
            size: 1,
            start: first & 0x10 != 0 && first & 0x0f == 0,
            picture: None,
            tl0: None,
            tk: None,
            temporal: 0,
        };
        if first & 0x80 == 0 {
            return (payload.len() > result.size).then_some(result);
        }
        let extension = *payload.get(1)?;
        result.size = 2;
        if extension & 0x80 != 0 {
            let begin = result.size;
            let len = if *payload.get(begin)? & 0x80 != 0 {
                2
            } else {
                1
            };
            payload.get(begin + len - 1)?;
            result.picture = Some((begin, len));
            result.size += len;
        }
        if extension & 0x40 != 0 {
            payload.get(result.size)?;
            result.tl0 = Some(result.size);
            result.size += 1;
        }
        if extension & 0x30 != 0 {
            let value = *payload.get(result.size)?;
            result.tk = Some(result.size);
            if extension & 0x20 != 0 {
                result.temporal = value >> 6;
            }
            result.size += 1;
        }
        (payload.len() > result.size).then_some(result)
    }
}

/// Per-viewer rewrite state. The new encoder starts only at a keyframe, while
/// RTP seq/timestamps, VP8 PictureID, TL0PICIDX and KEYIDX remain continuous.
#[derive(Clone, Default)]
pub(super) struct Rewriter {
    sequence: Option<u16>,
    timestamp: Option<u32>,
    source_timestamp: Option<u32>,
    source: Option<u32>,
    picture: u16,
    tl0: u8,
    key_index: u8,
}
impl Rewriter {
    pub fn rewrite(&mut self, frame: Frame) -> Vec<rtp::Packet> {
        let input_ts = frame.packets[0].header.timestamp;
        let switched = self.source != Some(frame.ssrc);
        let step = if switched {
            3000
        } else {
            self.source_timestamp
                .map_or(3000, |previous| input_ts.wrapping_sub(previous).max(1))
        };
        let timestamp = self
            .timestamp
            .map_or(input_ts, |last| last.wrapping_add(step));
        self.source = Some(frame.ssrc);
        self.source_timestamp = Some(input_ts);
        self.timestamp = Some(timestamp);
        self.picture = self.picture.wrapping_add(1) & 0x7fff;
        if frame.key {
            self.key_index = self.key_index.wrapping_add(1) & 0x1f;
        }
        let descriptor = Descriptor::parse(&frame.packets[0].payload).unwrap();
        if descriptor.temporal == 0 {
            self.tl0 = self.tl0.wrapping_add(1);
        }
        frame
            .packets
            .into_iter()
            .map(|mut packet| {
                let d = Descriptor::parse(&packet.payload).unwrap();
                let mut payload = packet.payload.to_vec();
                if let Some(offset) = d.tl0 {
                    payload[offset] = self.tl0;
                }
                if let Some(offset) = d.tk {
                    // K and T share a byte, preserving TID/Y bits.
                    if payload[1] & 0x10 != 0 {
                        payload[offset] = (payload[offset] & 0xe0) | self.key_index;
                    }
                }
                if let Some((offset, len)) = d.picture {
                    payload.splice(
                        offset..offset + len,
                        [0x80 | (self.picture >> 8) as u8, self.picture as u8],
                    );
                } else if payload[0] & 0x80 != 0 {
                    payload[1] |= 0x80;
                    payload.splice(2..2, [0x80 | (self.picture >> 8) as u8, self.picture as u8]);
                } else {
                    payload[0] |= 0x80;
                    payload.splice(
                        1..1,
                        [0x80, 0x80 | (self.picture >> 8) as u8, self.picture as u8],
                    );
                }
                packet.payload = payload.into();
                let sequence = self
                    .sequence
                    .map_or(packet.header.sequence_number, |last| last.wrapping_add(1));
                self.sequence = Some(sequence);
                packet.header.sequence_number = sequence;
                packet.header.timestamp = timestamp;
                packet
            })
            .collect()
    }
}

#[derive(Default)]
pub(super) struct Selector {
    layers: HashMap<u32, (u16, u8)>,
    active: Option<u32>,
    synced: bool,
    last_request: Option<Instant>,
}
impl Selector {
    pub fn register(&mut self, ssrc: u32, rid: &str) {
        let rank = match rid {
            "q" => 0,
            "h" => 1,
            "f" => 2,
            _ => 3,
        };
        if self.layers.len() < 3 {
            self.layers.entry(ssrc).or_insert((0, rank));
        }
    }
    pub fn accept(&mut self, frame: &Frame, intent: Intent) -> bool {
        if let Some(layer) = self.layers.get_mut(&frame.ssrc) {
            layer.0 = frame.height;
        }
        let desired = self.wanted(intent);
        if desired != self.active && desired == Some(frame.ssrc) && frame.key {
            self.active = desired;
            self.synced = true;
        }
        if self.active != Some(frame.ssrc) {
            return false;
        }
        if frame.discontinuity {
            self.synced = false;
        }
        if frame.key {
            self.synced = true;
        }
        self.synced
    }
    pub fn wanted(&self, intent: Intent) -> Option<u32> {
        if intent.congested {
            return self
                .layers
                .iter()
                .min_by_key(|(_, (_, rank))| *rank)
                .map(|(ssrc, _)| *ssrc);
        }
        if intent.height > 0
            && let Some((ssrc, _)) = self
                .layers
                .iter()
                .filter(|(_, (height, _))| *height >= intent.height)
                .min_by_key(|(_, (height, _))| *height)
        {
            return Some(*ssrc);
        }
        // Unknown geometry requests the full representation's keyframe rather
        // than permanently staying at the first (usually low) incoming RID.
        self.layers
            .iter()
            .max_by_key(|(_, (_, rank))| *rank)
            .map(|(ssrc, _)| *ssrc)
    }
    pub fn request(&mut self, intent: Intent) -> Option<u32> {
        let wanted = self.wanted(intent)?;
        if (self.active != Some(wanted) || !self.synced)
            && self
                .last_request
                .is_none_or(|last| last.elapsed() >= Duration::from_millis(500))
        {
            self.last_request = Some(Instant::now());
            Some(wanted)
        } else {
            None
        }
    }
    pub fn reset(&mut self) {
        self.synced = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn packet(ssrc: u32, seq: u16, ts: u32, start: bool, marker: bool, key: bool) -> rtp::Packet {
        let mut payload = vec![if start { 0x10 } else { 0 }];
        payload.extend(if key {
            vec![0, 0, 0, 0x9d, 0x01, 0x2a, 0x80, 0x02, 0x68, 0x01]
        } else {
            vec![1, 2, 3]
        });
        rtp::Packet {
            header: rtp::Header {
                ssrc,
                sequence_number: seq,
                timestamp: ts,
                marker,
                ..Default::default()
            },
            payload: payload.into(),
        }
    }
    fn frame(ssrc: u32, seq: u16, ts: u32, key: bool, height: u16) -> Frame {
        Frame {
            ssrc,
            key,
            height,
            packets: vec![packet(ssrc, seq, ts, true, true, key)],
            discontinuity: false,
        }
    }
    #[test]
    fn complete_frame_reorders_wrap_and_never_emits_a_partial_keyframe() {
        let mut a = Assembler::default();
        assert!(a.push(packet(1, 0, 9000, false, true, false)).is_none());
        assert!(
            a.push(packet(1, u16::MAX - 1, 9000, true, false, true))
                .is_none()
        );
        let result = a
            .push(packet(1, u16::MAX, 9000, false, false, false))
            .unwrap();
        assert!(result.key);
        assert_eq!(result.height, 360);
        assert_eq!(
            result
                .packets
                .iter()
                .map(|p| p.header.sequence_number)
                .collect::<Vec<_>>(),
            vec![u16::MAX - 1, u16::MAX, 0]
        );
        assert!(
            a.push(packet(1, 0, 9000, false, true, false)).is_none(),
            "duplicate complete frame"
        );
    }
    #[test]
    fn primary_padding_preserves_reference_continuity_even_between_reordered_fragments() {
        let mut a = Assembler::default();
        assert!(
            !a.push(packet(1, 10, 100, true, true, true))
                .unwrap()
                .discontinuity
        );
        let padding = |seq, ts| rtp::Packet {
            header: rtp::Header {
                ssrc: 1,
                sequence_number: seq,
                timestamp: ts,
                padding: true,
                ..Default::default()
            },
            payload: bytes::Bytes::new(),
        };
        // Padding may retain the timestamp of an already completed frame.
        assert!(a.push(padding(11, 100)).is_none());
        assert!(
            !a.push(packet(1, 12, 200, true, true, false))
                .unwrap()
                .discontinuity
        );
        assert!(a.push(packet(1, 13, 300, true, false, false)).is_none());
        assert!(a.push(packet(1, 15, 300, false, true, false)).is_none());
        let frame = a.push(padding(14, 300)).unwrap();
        assert!(!frame.discontinuity);
        assert_eq!(
            frame.packets.len(),
            2,
            "padding has no codec payload to forward"
        );
        assert!(
            a.push(packet(1, 17, 400, true, true, false))
                .unwrap()
                .discontinuity,
            "unknown missing payload sequence remains a real reference loss"
        );
    }

    #[test]
    fn keyframe_uncompressed_header_may_span_rtp_fragments() {
        let mut a = Assembler::default();
        let header = [0, 0, 0, 0x9d, 0x01, 0x2a, 0x80, 0x02, 0x68, 0x01];
        let mut start = packet(1, 10, 100, true, false, true);
        start.payload = [vec![0x10], header[..3].to_vec()].concat().into();
        let mut end = packet(1, 11, 100, false, true, false);
        end.payload = [vec![0], header[3..].to_vec()].concat().into();
        assert!(a.push(start).is_none());
        let complete = a.push(end).unwrap();
        assert!(complete.key);
        assert_eq!(complete.height, 360);
        assert_eq!(complete.packets.len(), 2);
    }

    #[test]
    fn same_source_idle_preserves_the_full_rtp_clock_gap() {
        let mut r = Rewriter::default();
        let first = r.rewrite(frame(1, 10, 100, true, 360));
        let after_idle = r.rewrite(frame(1, 11, 180100, false, 360));
        assert_eq!(
            after_idle[0]
                .header
                .timestamp
                .wrapping_sub(first[0].header.timestamp),
            180000
        );
    }

    #[tokio::test]
    async fn bounded_multi_rid_burst_recovers_small_then_accepts_large_complete_keyframe() {
        // A 600-packet full keyframe overtakes the actual publisher ring. The
        // receiver cannot emit a partial reference frame or keep asking only f.
        let (tx, mut rx) = tokio::sync::broadcast::channel(super::super::RTP_Q);
        let mut assembler = Assembler::default();
        let mut selector = Selector::default();
        selector.register(1, "f");
        selector.register(2, "q");
        assert!(selector.accept(&frame(1, 10, 100, true, 360), Intent::default()));
        assert!(!selector.accept(&frame(2, 10, 100, true, 90), Intent::default()));
        for seq in 0..600 {
            tx.send(packet(1, seq, 9000, seq == 0, seq == 599, seq == 0))
                .unwrap();
            if seq % 100 == 0 {
                tx.send(packet(
                    2,
                    20 + seq / 100,
                    9000 + u32::from(seq),
                    true,
                    true,
                    false,
                ))
                .unwrap();
            }
        }
        assert!(matches!(
            rx.recv().await,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_))
        ));
        selector.reset();
        let pressured = effective_intent(
            Intent::default(),
            Some(Instant::now() + Duration::from_secs(8)),
        );
        assert_eq!(selector.wanted(pressured), Some(2));
        while let Ok(packet) = rx.try_recv() {
            if let Some(frame) = assembler.push(packet) {
                assert_ne!(
                    frame.ssrc, 1,
                    "missing start cannot become a complete full keyframe"
                );
                assert!(
                    !selector.accept(&frame, pressured),
                    "low delta cannot repair a lost reference"
                );
            }
        }
        let mut low = packet(2, 30, 10000, true, true, true);
        let n = low.payload.len();
        let mut payload = low.payload.to_vec();
        payload[n - 2..].copy_from_slice(&90u16.to_le_bytes());
        low.payload = payload.into();
        let recovered = assembler.push(low).unwrap();
        assert!(selector.accept(&recovered, pressured));
        let mut rewriter = Rewriter::default();
        let low_sequence = rewriter.rewrite(recovered)[0].header.sequence_number;
        // Draining the same bounded ring as the sender progresses admits a
        // complete frame larger than the ring; no rate/bitrate ceiling is added.
        let normal = effective_intent(
            Intent::default(),
            Some(Instant::now() - Duration::from_millis(1)),
        );
        assert_eq!(selector.wanted(normal), Some(1));
        let mut full = None;
        for seq in 600..1200 {
            tx.send(packet(1, seq, 18000, seq == 600, seq == 1199, seq == 600))
                .unwrap();
            full = assembler.push(rx.recv().await.unwrap()).or(full);
        }
        let full = full.unwrap();
        assert_eq!(full.packets.len(), 600);
        assert!(selector.accept(&full, normal));
        let rewritten = rewriter.rewrite(full);
        assert_eq!(
            rewritten[0].header.sequence_number,
            low_sequence.wrapping_add(1)
        );
        assert_eq!(rewritten.len(), 600);
        assert!(
            assembler
                .layers
                .values()
                .all(|layer| layer.frames.len() <= MAX_PENDING_FRAMES)
        );
    }
    #[test]
    fn lossless_cross_frame_reordering_keeps_newer_pending_fragments() {
        for (a_ts, b_ts, start) in [(100, 200, 10), (u32::MAX - 20, 10, u16::MAX - 1)] {
            let mut a = Assembler::default();
            assert!(a.push(packet(1, start, a_ts, true, false, true)).is_none());
            assert!(
                a.push(packet(1, start.wrapping_add(2), b_ts, true, false, false))
                    .is_none()
            );
            let first = a
                .push(packet(1, start.wrapping_add(1), a_ts, false, true, false))
                .unwrap();
            assert!(!first.discontinuity);
            assert_eq!(a.layers[&1].frames.len(), 1);
            let second = a
                .push(packet(1, start.wrapping_add(3), b_ts, false, true, false))
                .unwrap();
            assert!(!second.discontinuity);
            assert_eq!(second.packets.len(), 2);
        }
    }

    #[test]
    fn missing_whole_frame_or_partial_frame_invalidates_decoder_references() {
        let mut a = Assembler::default();
        assert!(
            !a.push(packet(1, 20, 90, true, true, true))
                .unwrap()
                .discontinuity
        );
        assert!(
            a.push(packet(1, 22, 180, true, true, false))
                .unwrap()
                .discontinuity,
            "entire lost frame sequence gap"
        );
        assert!(a.push(packet(1, 23, 270, true, false, false)).is_none());
        assert!(
            a.push(packet(1, 25, 360, true, true, false))
                .unwrap()
                .discontinuity,
            "incomplete earlier frame"
        );
    }
    #[test]
    fn keyframe_switch_is_independent_per_viewer_and_loss_waits_for_refresh() {
        let mut low = Selector::default();
        let mut high = Selector::default();
        for s in [&mut low, &mut high] {
            s.register(11, "q");
            s.register(22, "f");
        }
        let q = frame(11, 9, 9000, true, 90);
        let f = frame(22, 70, 9000, true, 360);
        let small = Intent {
            height: 64,
            congested: false,
        };
        let full = Intent::default();
        assert!(low.accept(&q, small));
        assert!(!high.accept(&q, full));
        assert!(high.accept(&f, full));
        assert!(!low.accept(&f, small));
        assert!(!low.accept(&frame(22, 71, 12000, false, 360), full));
        assert!(
            low.accept(&frame(11, 10, 12000, false, 90), full),
            "old layer continues until new keyframe"
        );
        assert_eq!(low.request(full), Some(22));
        assert_eq!(low.request(full), None, "PLI cooldown");
        assert!(low.accept(&frame(22, 72, 15000, true, 360), full));
        low.reset();
        assert!(!low.accept(&frame(22, 73, 18000, false, 360), full));
        assert!(low.accept(&frame(22, 74, 21000, true, 360), full));
        assert!(!high.accept(&q, full), "other viewer selection unchanged");
    }
    #[test]
    fn rtp_and_vp8_identity_spaces_survive_encoder_switch_and_wrap() {
        let mut r = Rewriter::default();
        r.picture = 0x7ffe;
        r.tl0 = 254;
        r.key_index = 30;
        let first = r.rewrite(frame(1, 65535, 0xfffffff0, true, 90));
        let next = r.rewrite(frame(2, 120, 100, true, 360));
        assert_eq!(first[0].header.sequence_number, 65535);
        assert_eq!(next[0].header.sequence_number, 0);
        assert_eq!(next[0].header.timestamp, 0xfffffff0u32.wrapping_add(3000));
        assert_eq!(&first[0].payload[1..4], &[0x80, 0xff, 0xff]);
        assert_eq!(&next[0].payload[1..4], &[0x80, 0x80, 0]);
        assert_eq!(r.tl0, 0);
        assert_eq!(r.key_index, 0);
        let mut extended = frame(2, 121, 3100, false, 360);
        extended.packets[0].payload = vec![0x90, 0xf0, 0x7f, 44, 0b11011111, 1, 2, 3].into();
        let rewritten = r.rewrite(extended);
        assert_eq!(rewritten[0].payload[1], 0xf0);
        assert_eq!(&rewritten[0].payload[2..4], &[0x80, 1]);
        assert_eq!(rewritten[0].payload[4], 0);
        assert_eq!(rewritten[0].payload[5] & 0xe0, 0xc0);
    }
    #[test]
    fn every_truncated_descriptor_and_oversized_frame_is_safe() {
        for length in 0..7 {
            assert!(Descriptor::parse(&vec![0xff; length]).is_none());
        }
        let mut a = Assembler::default();
        for source in 1..20 {
            a.push(packet(source, 1, 1, true, false, true));
        }
        assert_eq!(a.layers.len(), 3);
        for n in 0..10 {
            let mut p = packet(1, n, 100 + n as u32, true, false, true);
            let mut payload = p.payload.to_vec();
            payload.resize(MAX_FRAME_BYTES + 1, 0);
            p.payload = payload.into();
            assert!(a.push(p).is_none());
        }
        assert!(a.layers[&1].frames.len() <= 3);
    }
}
