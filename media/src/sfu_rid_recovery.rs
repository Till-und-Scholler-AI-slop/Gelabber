//! Restore only learned, currently negotiated MID/RID metadata before the
//! pinned endpoint forgets RID SSRCs on a subscription reoffer. This uses the
//! public interceptor API; it does not alter the core or infer unknown SSRCs.
//! Recovery also requires a current explicit source announcement. Retraction
//! removes that permission; historical/unknown stops never accumulate state.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use rtc::interceptor::{Interceptor, Packet, StreamInfo, TaggedPacket, interceptor};
use rtc::rtp::header::{EXTENSION_PROFILE_ONE_BYTE, EXTENSION_PROFILE_TWO_BYTE, Header};
use rtc::sansio;
use rtc::shared::error::Error;
use tokio::sync::watch;
use webrtc::peer_connection::RTCSessionDescription;

const MID_URI: &str = "urn:ietf:params:rtp-hdrext:sdes:mid";
const RID_URI: &str = "urn:ietf:params:rtp-hdrext:sdes:rtp-stream-id";
const MAX_SCOPES: usize = 64;
const MAX_SSRC: usize = MAX_SCOPES * 3;
const MAX_IDENTITY_BYTES: usize = 255;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Source {
    mid: String,
    stream: String,
    track: String,
}
#[derive(Clone, PartialEq, Eq)]
struct Scope {
    source: Source,
    rids: HashSet<String>,
    payloads: HashSet<u8>,
    mid_id: u8,
    rid_id: u8,
    repair_id: Option<u8>,
}
#[derive(Clone)]
struct Learned {
    source: Source,
    rid: String,
    bound: Option<ReceiverFence>,
}
/// Only a current, authorized publication may reuse a still-bound public
/// receiver's primary encodings after an inactive/active negotiation.
#[derive(Clone)]
pub(super) struct ReceiverFence {
    pub life: Arc<super::PublicationLife>,
    pub closing: watch::Receiver<bool>,
    pub deadline: Option<Arc<Mutex<std::time::Instant>>>,
}
impl ReceiverFence {
    fn open(&self) -> bool {
        !*self.life.stop.borrow()
            && !*self.life.done.borrow()
            && !*self.closing.borrow()
            && !super::live_expired(&self.deadline)
    }
}
#[derive(Default)]
struct State {
    scopes: Vec<Scope>,
    learned: HashMap<u32, Learned>,
    announced: HashSet<String>,
    active: bool,
    closed: bool,
    #[cfg(test)]
    recovered: usize,
    #[cfg(test)]
    seeded: usize,
}
#[derive(Clone, Default)]
pub(super) struct Recovery(Arc<Mutex<State>>);

impl Recovery {
    pub(super) fn suspend(&self) {
        self.0.lock().unwrap().active = false;
    }
    pub(super) fn accept(&self, remote: &str, local: &str) {
        let mut scopes = parse_scopes(remote, local);
        let mut state = self.0.lock().unwrap();
        if state.closed {
            return;
        }
        scopes.retain(|scope| state.announced.contains(&scope.source.track));
        state.learned.retain(|_, learned| {
            scopes
                .iter()
                .any(|scope| scope.source == learned.source && scope.rids.contains(&learned.rid))
        });
        state.scopes = scopes;
        state.active = true;
    }
    pub(super) fn restore(&self, remote: &str, local: &str) {
        let mut scopes = parse_scopes(remote, local);
        let mut state = self.0.lock().unwrap();
        if state.closed {
            return;
        }
        scopes.retain(|scope| state.announced.contains(&scope.source.track));
        // A rollback can only resume the already accepted metadata contract.
        // It must not authorize identities from an abandoned pending offer.
        if scopes == state.scopes {
            state.active = true;
        }
    }
    pub(super) fn stop_track(&self, track: &str) {
        let mut state = self.0.lock().unwrap();
        state.announced.remove(track);
        state.scopes.retain(|scope| scope.source.track != track);
        state
            .learned
            .retain(|_, learned| learned.source.track != track);
    }
    pub(super) fn allow_track(&self, track: &str) {
        let mut state = self.0.lock().unwrap();
        if !state.closed && state.announced.len() < MAX_SCOPES {
            state.announced.insert(track.into());
        }
    }
    pub(super) fn retain_tracks(&self, current: &HashSet<String>) {
        let mut state = self.0.lock().unwrap();
        state.announced.retain(|track| current.contains(track));
        state
            .scopes
            .retain(|scope| current.contains(&scope.source.track));
        state
            .learned
            .retain(|_, learned| current.contains(&learned.source.track));
    }
    pub(super) fn bind_receiver(
        &self,
        mid: &str,
        stream: &str,
        track: &str,
        encodings: &[(u32, String, u8)],
        fence: ReceiverFence,
    ) {
        #[cfg(test)]
        if std::env::var_os("GELABBER_TEST_DISABLE_BOUND_RID_BINDING").is_some() {
            return;
        }
        let mut state = self.0.lock().unwrap();
        if state.closed || !state.active || !state.announced.contains(track) || !fence.open() {
            return;
        }
        let Some(scope) = state
            .scopes
            .iter()
            .find(|scope| {
                scope.source.mid == mid
                    && scope.source.stream == stream
                    && scope.source.track == track
            })
            .cloned()
        else {
            return;
        };
        // All encodings must belong to the same negotiated primary source.
        // Do not partially bind an ambiguous receiver or reuse an SSRC from
        // another MID/RID. Existing explicit contradictory headers still win.
        if encodings.is_empty()
            || encodings.len() > 3
            || encodings.iter().any(|(ssrc, rid, pt)| {
                *ssrc == 0 || !scope.rids.contains(rid) || !scope.payloads.contains(pt)
            })
            || encodings
                .iter()
                .map(|(ssrc, _, _)| ssrc)
                .collect::<HashSet<_>>()
                .len()
                != encodings.len()
            || encodings
                .iter()
                .map(|(_, rid, _)| rid)
                .collect::<HashSet<_>>()
                .len()
                != encodings.len()
            || encodings.iter().any(|(ssrc, rid, _)| {
                state
                    .learned
                    .get(ssrc)
                    .is_some_and(|old| old.source != scope.source || old.rid != *rid)
            })
        {
            return;
        }
        if state.learned.len()
            + encodings
                .iter()
                .filter(|(ssrc, _, _)| !state.learned.contains_key(ssrc))
                .count()
            > MAX_SSRC
        {
            return;
        }
        for (ssrc, rid, _) in encodings {
            state.learned.insert(
                *ssrc,
                Learned {
                    source: scope.source.clone(),
                    rid: rid.clone(),
                    bound: Some(fence.clone()),
                },
            );
            #[cfg(test)]
            {
                state.seeded += 1;
            }
        }
    }
    pub(super) fn close(&self) {
        let mut state = self.0.lock().unwrap();
        state.closed = true;
        state.active = false;
        state.scopes.clear();
        state.learned.clear();
        state.announced.clear();
    }
    #[cfg(test)]
    pub(super) fn is_announced(&self, track: &str) -> bool {
        self.0.lock().unwrap().announced.contains(track)
    }
    #[cfg(test)]
    pub(super) fn observed(&self) -> (usize, usize, usize) {
        let state = self.0.lock().unwrap();
        (state.scopes.len(), state.learned.len(), state.recovered)
    }
    #[cfg(test)]
    pub(super) fn is_active(&self) -> bool {
        let state = self.0.lock().unwrap();
        state.active && !state.closed
    }
    #[cfg(test)]
    pub(super) fn bound_seeds(&self) -> usize {
        self.0.lock().unwrap().seeded
    }
    fn recover(&self, header: &mut Header) {
        let mut state = self.0.lock().unwrap();
        if !state.active
            || state.closed
            || (header.extension
                && !matches!(
                    header.extension_profile,
                    EXTENSION_PROFILE_ONE_BYTE | EXTENSION_PROFILE_TWO_BYTE
                ))
        {
            return;
        }
        // Learning requires BOTH actual identifiers on a valid accepted MID.
        // Existing conflicting metadata always wins over an old cache entry.
        let explicit = state.scopes.iter().find_map(|scope| {
            if !scope.payloads.contains(&header.payload_type)
                || scope
                    .repair_id
                    .is_some_and(|id| header.get_extension(id).is_some())
            {
                return None;
            }
            let mid = header.get_extension(scope.mid_id)?;
            let rid = header.get_extension(scope.rid_id)?;
            let mid = std::str::from_utf8(&mid).ok()?;
            let rid = std::str::from_utf8(&rid).ok()?;
            if mid != scope.source.mid || !scope.rids.contains(rid) || duplicate_ids(header, scope)
            {
                return None;
            }
            Some(Learned {
                source: scope.source.clone(),
                rid: rid.into(),
                bound: None,
            })
        });
        if let Some(mut learned) = explicit {
            if state
                .learned
                .get(&header.ssrc)
                .is_some_and(|old| old.source != learned.source || old.rid != learned.rid)
            {
                state.learned.remove(&header.ssrc);
                return;
            }
            if state.learned.contains_key(&header.ssrc) || state.learned.len() < MAX_SSRC {
                learned.bound = state
                    .learned
                    .get(&header.ssrc)
                    .and_then(|old| old.bound.clone());
                state.learned.insert(header.ssrc, learned);
            }
            return;
        }
        let Some(learned) = state.learned.get(&header.ssrc).cloned() else {
            return;
        };
        if learned.bound.as_ref().is_some_and(|fence| !fence.open()) {
            state.learned.remove(&header.ssrc);
            return;
        }
        let Some(scope) = state
            .scopes
            .iter()
            .find(|scope| {
                scope.source == learned.source
                    && scope.rids.contains(&learned.rid)
                    && scope.payloads.contains(&header.payload_type)
            })
            .cloned()
        else {
            return;
        };
        if scope
            .repair_id
            .is_some_and(|id| header.get_extension(id).is_some())
        {
            return;
        }
        if duplicate_ids(header, &scope) {
            state.learned.remove(&header.ssrc);
            return;
        }
        let mid = header.get_extension(scope.mid_id);
        let rid = header.get_extension(scope.rid_id);
        if mid
            .as_ref()
            .is_some_and(|v| v.as_ref() != learned.source.mid.as_bytes())
            || rid
                .as_ref()
                .is_some_and(|v| v.as_ref() != learned.rid.as_bytes())
        {
            state.learned.remove(&header.ssrc);
            return;
        }
        if mid.is_some() && rid.is_some() {
            return;
        }
        let mut additions = Vec::new();
        if mid.is_none() {
            additions.push((
                scope.mid_id,
                Bytes::copy_from_slice(learned.source.mid.as_bytes()),
            ));
        }
        if rid.is_none() {
            additions.push((scope.rid_id, Bytes::copy_from_slice(learned.rid.as_bytes())));
        }
        let mut restored = header.clone();
        // set_extension chooses a new block's profile only by payload length;
        // choose the correct RFC8285 profile for ids >=15 too. Rebuild when
        // upgrading a one-byte block, retaining every pre-existing extension.
        let two_byte = additions.iter().any(|(id, v)| *id > 14 || v.len() > 16)
            || restored.extension_profile == EXTENSION_PROFILE_TWO_BYTE;
        if !restored.extension
            || (two_byte && restored.extension_profile == EXTENSION_PROFILE_ONE_BYTE)
        {
            let existing = std::mem::take(&mut restored.extensions);
            restored.extension = true;
            restored.extension_profile = if two_byte {
                EXTENSION_PROFILE_TWO_BYTE
            } else {
                EXTENSION_PROFILE_ONE_BYTE
            };
            restored.extensions_padding = 0;
            for extension in existing {
                if restored
                    .set_extension(extension.id, extension.payload)
                    .is_err()
                {
                    return;
                }
            }
        }
        for (id, value) in additions {
            if restored.set_extension(id, value).is_err() {
                return;
            }
        }
        *header = restored;
        #[cfg(test)]
        {
            state.recovered += 1;
        }
    }
}
fn duplicate_ids(header: &Header, scope: &Scope) -> bool {
    [scope.mid_id, scope.rid_id]
        .iter()
        .any(|id| header.extensions.iter().filter(|e| e.id == *id).count() > 1)
}

fn media_direction<'a>(
    media: &'a [rtc::sdp::description::common::Attribute],
    session: &'a [rtc::sdp::description::common::Attribute],
) -> &'a str {
    media
        .iter()
        .find(|a| {
            matches!(
                a.key.as_str(),
                "sendrecv" | "sendonly" | "recvonly" | "inactive"
            )
        })
        .or_else(|| {
            session.iter().find(|a| {
                matches!(
                    a.key.as_str(),
                    "sendrecv" | "sendonly" | "recvonly" | "inactive"
                )
            })
        })
        .map(|a| a.key.as_str())
        .unwrap_or("sendrecv")
}

fn parse_scopes(remote: &str, local: &str) -> Vec<Scope> {
    let Ok(remote) = RTCSessionDescription::offer(remote.into()).and_then(|d| d.unmarshal()) else {
        return vec![];
    };
    let Ok(local) = RTCSessionDescription::answer(local.into()).and_then(|d| d.unmarshal()) else {
        return vec![];
    };
    let mut scopes = Vec::new();
    let mut seen = HashSet::new();
    for media in &remote.media_descriptions {
        if media.media_name.media != "video"
            || !super::sdp_media_has_transport(&remote, media)
            || !matches!(
                media_direction(&media.attributes, &remote.attributes),
                "sendonly" | "sendrecv"
            )
        {
            continue;
        }
        let values = |key: &str| {
            media
                .attributes
                .iter()
                .filter(|a| a.key == key)
                .filter_map(|a| a.value.as_deref())
                .collect::<Vec<_>>()
        };
        let mid_values = values("mid");
        let [mid] = mid_values.as_slice() else {
            continue;
        };
        if mid.is_empty() || mid.len() > MAX_IDENTITY_BYTES || !seen.insert((*mid).to_owned()) {
            return vec![];
        }
        let msid_values = values("msid");
        let [msid] = msid_values.as_slice() else {
            continue;
        };
        let identity = msid.split_whitespace().collect::<Vec<_>>();
        let [stream, track] = identity.as_slice() else {
            continue;
        };
        if stream.is_empty()
            || track.is_empty()
            || stream.len() > MAX_IDENTITY_BYTES
            || track.len() > MAX_IDENTITY_BYTES
        {
            continue;
        }
        let matches = local
            .media_descriptions
            .iter()
            .filter(|m| {
                m.media_name.media == "video"
                    && super::sdp_media_has_transport(&local, m)
                    && matches!(
                        media_direction(&m.attributes, &local.attributes),
                        "recvonly" | "sendrecv"
                    )
                    && m.attributes
                        .iter()
                        .any(|a| a.key == "mid" && a.value.as_deref() == Some(mid))
            })
            .collect::<Vec<_>>();
        let [accepted] = matches.as_slice() else {
            continue;
        };
        let ext = |attrs: &[rtc::sdp::description::common::Attribute],
                   uri: &str,
                   send: bool|
         -> Option<u8> {
            let mut result = None;
            for a in attrs.iter().filter(|a| a.key == "extmap") {
                let mut fields = a.value.as_deref()?.split_whitespace();
                let mut id = fields.next()?.split('/');
                let number = id.next()?.parse::<u8>().ok()?;
                let direction = id.next().unwrap_or("sendrecv");
                if fields.next() != Some(uri) {
                    continue;
                }
                if number == 0
                    || !matches!(
                        (send, direction),
                        (true, "sendrecv" | "sendonly") | (false, "sendrecv" | "recvonly")
                    )
                    || result.is_some()
                {
                    return None;
                }
                result = Some(number);
            }
            result
        };
        let Some(mid_id) = ext(&media.attributes, MID_URI, true) else {
            continue;
        };
        let Some(rid_id) = ext(&media.attributes, RID_URI, true) else {
            continue;
        };
        if mid_id == rid_id
            || ext(&accepted.attributes, MID_URI, false) != Some(mid_id)
            || ext(&accepted.attributes, RID_URI, false) != Some(rid_id)
        {
            continue;
        }
        let rid_values = |attrs: &[rtc::sdp::description::common::Attribute], direction: &str| {
            attrs
                .iter()
                .filter(|a| a.key == "rid")
                .filter_map(|a| {
                    let mut fields = a.value.as_deref()?.split_whitespace();
                    let rid = fields.next()?;
                    if fields.next() != Some(direction)
                        || rid.is_empty()
                        || rid.len() > MAX_IDENTITY_BYTES
                    {
                        return None;
                    }
                    Some(rid.to_owned())
                })
                .collect::<HashSet<_>>()
        };
        let received = rid_values(&accepted.attributes, "recv");
        let rids = rid_values(&media.attributes, "send")
            .intersection(&received)
            .cloned()
            .collect::<HashSet<_>>();
        if rids.is_empty() || rids.len() > 3 {
            continue;
        }
        let accepted_pts = accepted
            .media_name
            .formats
            .iter()
            .filter_map(|s| s.parse::<u8>().ok())
            .collect::<HashSet<_>>();
        let mut remote_section = remote.clone();
        remote_section.media_descriptions = vec![media.clone()];
        let mut local_section = local.clone();
        local_section.media_descriptions = vec![(*accepted).clone()];
        let payloads = media
            .media_name
            .formats
            .iter()
            .filter_map(|s| s.parse::<u8>().ok())
            .filter(|pt| {
                if !accepted_pts.contains(pt) {
                    return false;
                }
                let Ok(source) = remote_section.get_codec_for_payload_type(*pt) else {
                    return false;
                };
                let Ok(receiver) = local_section.get_codec_for_payload_type(*pt) else {
                    return false;
                };
                matches!(
                    source.name.to_ascii_lowercase().as_str(),
                    "vp8" | "vp9" | "h264" | "av1" | "h265"
                ) && source.name.eq_ignore_ascii_case(&receiver.name)
                    && source.clock_rate == receiver.clock_rate
            })
            .collect::<HashSet<_>>();
        if payloads.is_empty() {
            continue;
        }
        if scopes.len() == MAX_SCOPES {
            return vec![];
        }
        scopes.push(Scope {
            source: Source {
                mid: (*mid).into(),
                stream: (*stream).into(),
                track: (*track).into(),
            },
            rids,
            payloads,
            mid_id,
            rid_id,
            repair_id: ext(
                &media.attributes,
                "urn:ietf:params:rtp-hdrext:sdes:repaired-rtp-stream-id",
                true,
            )
            .filter(|id| {
                ext(
                    &accepted.attributes,
                    "urn:ietf:params:rtp-hdrext:sdes:repaired-rtp-stream-id",
                    false,
                ) == Some(*id)
            }),
        });
    }
    scopes
}

#[derive(Interceptor)]
pub(super) struct RidRecovery<P> {
    #[next]
    next: P,
    recovery: Recovery,
    #[cfg(test)]
    enabled: bool,
}
impl<P> RidRecovery<P> {
    pub(super) fn new(next: P, recovery: Recovery) -> Self {
        Self {
            next,
            recovery,
            #[cfg(test)]
            enabled: std::env::var_os("GELABBER_TEST_DISABLE_RID_RECOVERY").is_none(),
        }
    }
}
#[interceptor]
impl<P: Interceptor> RidRecovery<P> {
    #[overrides]
    fn handle_read(&mut self, mut msg: TaggedPacket) -> Result<(), Self::Error> {
        #[cfg(test)]
        if !self.enabled {
            return self.next.handle_read(msg);
        }
        if let Packet::Rtp(packet) = &mut msg.message {
            self.recovery.recover(&mut packet.header);
        }
        self.next.handle_read(msg)
    }
    #[overrides]
    fn close(&mut self) -> Result<(), Self::Error> {
        self.recovery.close();
        self.next.close()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rtc::shared::marshal::{Marshal, Unmarshal};

    fn pair(mid: &str, stream: &str, track: &str, mid_id: u8, rid_id: u8) -> (String, String) {
        let base = format!(
            "v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=-\r\nt=0 0\r\nm=video 9 UDP/TLS/RTP/SAVPF 96 97\r\na=mid:{mid}\r\na=rtpmap:96 VP8/90000\r\na=rtpmap:97 rtx/90000\r\na=fmtp:97 apt=96\r\na=extmap:{mid_id} {MID_URI}\r\na=extmap:{rid_id} {RID_URI}\r\na=extmap:13 urn:ietf:params:rtp-hdrext:sdes:repaired-rtp-stream-id\r\n"
        );
        (
            format!(
                "{base}a=sendonly\r\na=msid:{stream} {track}\r\na=rid:q send\r\na=rid:f send\r\n"
            ),
            format!("{base}a=recvonly\r\na=rid:q recv\r\na=rid:f recv\r\n"),
        )
    }
    fn packet(ssrc: u32) -> Header {
        Header {
            version: 2,
            ssrc,
            payload_type: 96,
            ..Default::default()
        }
    }
    fn learn(store: &Recovery, ssrc: u32, mid: &str, mid_id: u8, rid_id: u8) {
        let mut h = packet(ssrc);
        h.extension = true;
        h.extension_profile = if mid_id > 14 || rid_id > 14 {
            EXTENSION_PROFILE_TWO_BYTE
        } else {
            EXTENSION_PROFILE_ONE_BYTE
        };
        h.set_extension(mid_id, Bytes::copy_from_slice(mid.as_bytes()))
            .unwrap();
        h.set_extension(rid_id, Bytes::from_static(b"q")).unwrap();
        store.recover(&mut h);
    }
    fn fence() -> (ReceiverFence, watch::Sender<bool>, watch::Sender<bool>) {
        let (stop, _) = watch::channel(false);
        let (done_tx, done) = watch::channel(false);
        let (closing, _) = watch::channel(false);
        let life = Arc::new(super::super::PublicationLife { stop, done });
        (
            ReceiverFence {
                life,
                closing: closing.subscribe(),
                deadline: None,
            },
            done_tx,
            closing,
        )
    }
    fn bind(store: &Recovery, fence: ReceiverFence) {
        store.bind_receiver(
            "video",
            "stream",
            "track",
            &[(10, "q".into(), 96), (20, "f".into(), 96)],
            fence,
        );
    }
    #[test]
    fn bound_receiver_requires_fresh_announcement_and_exact_stable_scope() {
        let store = Recovery::default();
        let (r, l) = pair("video", "stream", "track", 3, 4);
        let (current, _done, _closing) = fence();
        store.accept(&r, &l);
        bind(&store, current.clone());
        assert_eq!(store.bound_seeds(), 0);
        assert_eq!(store.observed().1, 0);
        store.allow_track("track");
        store.accept(&r, &l);
        bind(&store, current.clone());
        assert_eq!(store.bound_seeds(), 2);
        let mut h = packet(10);
        store.recover(&mut h);
        assert_eq!(h.get_extension(4).unwrap(), b"q"[..]);
        store.stop_track("track");
        store.accept(&r, &l); // replaying the old active SDP is no new grant
        bind(&store, current.clone());
        let mut h = packet(10);
        store.recover(&mut h);
        assert!(!h.extension);
        store.allow_track("track");
        store.accept(&r, &l);
        store.bind_receiver(
            "other",
            "stream",
            "track",
            &[(10, "q".into(), 96)],
            current.clone(),
        );
        store.bind_receiver(
            "video",
            "changed",
            "track",
            &[(10, "q".into(), 96)],
            current.clone(),
        );
        store.bind_receiver(
            "video",
            "stream",
            "track",
            &[(10, "q".into(), 97)],
            current.clone(),
        );
        assert_eq!(store.observed().1, 0);
        store.suspend();
        bind(&store, current.clone());
        assert_eq!(store.observed().1, 0);
        store.restore(&r, &l);
        bind(&store, current.clone());
        assert_eq!(store.observed().1, 2);
        store.close();
        store.accept(&r, &l);
        bind(&store, current);
        assert_eq!(store.observed().1, 0);
    }
    #[test]
    fn bound_receiver_keeps_publication_close_done_and_expiry_fences() {
        for reason in ["stop", "done", "close", "expiry"] {
            let store = Recovery::default();
            store.allow_track("track");
            let (r, l) = pair("video", "stream", "track", 3, 4);
            store.accept(&r, &l);
            let (mut current, done, closing) = fence();
            let deadline = Arc::new(Mutex::new(
                std::time::Instant::now() + std::time::Duration::from_secs(60),
            ));
            current.deadline = Some(deadline.clone());
            bind(&store, current.clone());
            // Explicit matching headers must not discard a seeded life fence.
            learn(&store, 10, "video", 3, 4);
            match reason {
                "stop" => {
                    current.life.stop.send_replace(true);
                }
                "done" => {
                    done.send_replace(true);
                }
                "close" => {
                    closing.send_replace(true);
                }
                _ => {
                    *deadline.lock().unwrap() = std::time::Instant::now();
                }
            }
            let mut h = packet(10);
            store.recover(&mut h);
            assert!(!h.extension, "{reason}");
            bind(&store, current);
            let mut h = packet(20);
            store.recover(&mut h);
            assert!(!h.extension, "late bind after {reason}");
        }
    }
    #[test]
    fn bound_receiver_rejects_ambiguous_encodings_and_repair_headers() {
        let store = Recovery::default();
        store.allow_track("track");
        let (r, l) = pair("video", "stream", "track", 15, 16);
        store.accept(&r, &l);
        let (current, _done, _closing) = fence();
        store.bind_receiver(
            "video",
            "stream",
            "track",
            &[(10, "q".into(), 96), (10, "f".into(), 96)],
            current.clone(),
        );
        store.bind_receiver(
            "video",
            "stream",
            "track",
            &[(10, "q".into(), 96), (20, "q".into(), 96)],
            current.clone(),
        );
        store.bind_receiver(
            "video",
            "stream",
            "track",
            &[(10, "unknown".into(), 96)],
            current.clone(),
        );
        assert_eq!(store.observed().1, 0);
        bind(&store, current);
        let mut repair = packet(10);
        repair.payload_type = 97;
        store.recover(&mut repair);
        assert!(!repair.extension);
        let mut repair = packet(10);
        repair.set_extension(13, Bytes::from_static(b"q")).unwrap();
        store.recover(&mut repair);
        assert!(repair.get_extension(15).is_none());
        let mut conflict = packet(10);
        conflict
            .set_extension(15, Bytes::from_static(b"wrong"))
            .unwrap();
        store.recover(&mut conflict);
        assert!(conflict.get_extension(16).is_none());
        let mut subsequent = packet(10);
        store.recover(&mut subsequent);
        assert!(!subsequent.extension);
    }
    #[test]
    fn reoffer_restores_only_known_current_primary_ssrc() {
        let store = Recovery::default();
        store.allow_track("track");
        let (r, l) = pair("video", "stream", "track", 3, 4);
        store.accept(&r, &l);
        learn(&store, 10, "video", 3, 4);
        store.suspend();
        let mut suspended = packet(10);
        store.recover(&mut suspended);
        assert!(!suspended.extension);
        store.accept(&r, &l);
        let mut known = packet(10);
        store.recover(&mut known);
        assert_eq!(known.get_extension(3).unwrap(), b"video"[..]);
        assert_eq!(known.get_extension(4).unwrap(), b"q"[..]);
        let mut unknown = packet(11);
        store.recover(&mut unknown);
        assert!(!unknown.extension);
        let mut repair = packet(10);
        repair.payload_type = 97;
        store.recover(&mut repair);
        assert!(!repair.extension);
        assert_eq!(store.observed(), (1, 1, 1));
    }
    #[test]
    fn partial_conflicting_metadata_invalidates_without_combining_sources() {
        let store = Recovery::default();
        store.allow_track("track");
        let (r, l) = pair("video", "stream", "track", 3, 4);
        store.accept(&r, &l);
        learn(&store, 10, "video", 3, 4);
        let mut partial = packet(10);
        partial
            .set_extension(3, Bytes::from_static(b"video"))
            .unwrap();
        store.recover(&mut partial);
        assert_eq!(partial.get_extension(4).unwrap(), b"q"[..]);
        for (id, value) in [(3, b"other".as_slice()), (4, b"f".as_slice())] {
            learn(&store, 10, "video", 3, 4);
            let mut conflict = packet(10);
            conflict
                .set_extension(id, Bytes::copy_from_slice(value))
                .unwrap();
            let old = conflict.clone();
            store.recover(&mut conflict);
            assert_eq!(conflict, old);
            let mut missing = packet(10);
            store.recover(&mut missing);
            assert!(!missing.extension);
        }
    }
    #[test]
    fn new_stream_track_stop_and_close_cannot_reactivate_old_identity() {
        let store = Recovery::default();
        store.allow_track("track");
        let (r, l) = pair("video", "stream", "track", 3, 4);
        for altered in [
            r.replace("stream track", "new-stream track"),
            r.replace("stream track", "stream new-track"),
            r.replace("a=sendonly", "a=inactive"),
            r.replace("m=video 9", "m=video 0"),
            r.replace("a=rid:q send\r\n", ""),
        ] {
            store.accept(&r, &l);
            learn(&store, 10, "video", 3, 4);
            store.accept(&altered, &l);
            let mut h = packet(10);
            store.recover(&mut h);
            assert!(!h.extension);
        }
        store.accept(&r, &l);
        learn(&store, 10, "video", 3, 4);
        store.close();
        store.accept(&r, &l);
        let mut h = packet(10);
        store.recover(&mut h);
        assert!(!h.extension);
        assert_eq!(store.observed().0, 0);
    }
    #[test]
    fn per_mid_negotiated_ids_and_two_byte_conversion_preserve_extensions() {
        let store = Recovery::default();
        store.allow_track("track");
        let (r, l) = pair("video", "stream", "track", 20, 21);
        store.accept(&r, &l);
        learn(&store, 10, "video", 20, 21);
        let mut h = packet(10);
        h.set_extension(8, Bytes::from_static(b"vendor")).unwrap();
        store.recover(&mut h);
        assert_eq!(h.extension_profile, EXTENSION_PROFILE_TWO_BYTE);
        assert_eq!(h.get_extension(8).unwrap(), b"vendor"[..]);
        assert_eq!(h.get_extension(20).unwrap(), b"video"[..]);
        let bytes = h.marshal().unwrap();
        let parsed = Header::unmarshal(&mut bytes.clone()).unwrap();
        assert_eq!(parsed.get_extension(21).unwrap(), b"q"[..]);
        let (r, l) = pair("video", "stream", "track", 15, 16);
        store.accept(&r, &l);
        let mut remapped = packet(10);
        store.recover(&mut remapped);
        assert_eq!(remapped.extension_profile, EXTENSION_PROFILE_TWO_BYTE);
        assert_eq!(remapped.get_extension(15).unwrap(), b"video"[..]);
        assert!(remapped.get_extension(20).is_none());
    }
    #[test]
    fn repaired_rid_and_unnegotiated_header_identity_never_train() {
        let store = Recovery::default();
        store.allow_track("track");
        let (r, l) = pair("video", "stream", "track", 3, 4);
        store.accept(&r, &l);
        let mut h = packet(10);
        for (id, v) in [
            (3, b"video".as_slice()),
            (4, b"q".as_slice()),
            (13, b"q".as_slice()),
        ] {
            h.set_extension(id, Bytes::copy_from_slice(v)).unwrap();
        }
        store.recover(&mut h);
        assert_eq!(store.observed().1, 0);
        learn(&store, 11, "unnegotiated", 3, 4);
        assert_eq!(store.observed().1, 0);
    }
    #[test]
    fn explicit_source_stop_requires_fresh_authorized_announcement_and_headers() {
        let store = Recovery::default();
        store.allow_track("track");
        let (r, l) = pair("video", "stream", "track", 3, 4);
        store.accept(&r, &l);
        learn(&store, 10, "video", 3, 4);
        store.stop_track("track");
        store.accept(&r, &l);
        let mut stopped = packet(10);
        store.recover(&mut stopped);
        assert!(!stopped.extension);
        assert_eq!(store.observed().0, 0);
        store.allow_track("track");
        store.accept(&r, &l);
        let mut missing = packet(10);
        store.recover(&mut missing);
        assert!(!missing.extension);
        learn(&store, 10, "video", 3, 4);
        store.recover(&mut missing);
        assert!(missing.extension);
    }

    #[test]
    fn rollback_restores_only_previous_scope_and_respects_retract_and_close() {
        let store = Recovery::default();
        store.allow_track("track");
        let (r, l) = pair("video", "stream", "track", 3, 4);
        store.accept(&r, &l);
        learn(&store, 10, "video", 3, 4);
        store.suspend();
        store.restore(&r.replace("stream track", "stream abandoned"), &l);
        assert!(!store.is_active());
        store.restore(&r, &l);
        let mut restored = packet(10);
        store.recover(&mut restored);
        assert!(restored.extension);
        store.suspend();
        store.stop_track("track");
        store.restore(&r, &l);
        let mut stopped = packet(10);
        store.recover(&mut stopped);
        assert!(!stopped.extension);
        assert_eq!(store.observed().0, 0);
        store.close();
        store.restore(&r, &l);
        assert!(!store.is_active());
    }

    #[test]
    fn valid_metadata_cache_is_bounded_even_for_many_claimed_ssrcs() {
        let store = Recovery::default();
        store.allow_track("track");
        let (r, l) = pair("video", "stream", "track", 3, 4);
        store.accept(&r, &l);
        for ssrc in 0..MAX_SSRC as u32 + 10 {
            learn(&store, ssrc, "video", 3, 4);
        }
        assert_eq!(store.observed().1, MAX_SSRC);
        let mut too_many = packet(MAX_SSRC as u32 + 1);
        store.recover(&mut too_many);
        assert!(!too_many.extension);
    }

    #[test]
    fn many_retracts_and_sequential_sources_never_exhaust_current_permissions() {
        let store = Recovery::default();
        store.allow_track("track");
        let (r, l) = pair("video", "stream", "track", 3, 4);
        store.accept(&r, &l);
        learn(&store, 10, "video", 3, 4);
        for n in 0..256 {
            store.stop_track(&format!("unknown-{n}"));
        }
        let mut unchanged = packet(10);
        store.recover(&mut unchanged);
        assert!(unchanged.extension);
        store.stop_track("track");
        for n in 0..256 {
            let track = format!("screen-{n}");
            let (r, l) = pair("video", "stream", &track, 3, 4);
            store.allow_track(&track);
            store.accept(&r, &l);
            learn(&store, 10, "video", 3, 4);
            let mut active = packet(10);
            store.recover(&mut active);
            assert!(active.extension, "source {n} still has recovery");
            store.stop_track(&track);
            store.accept(&r, &l);
            let mut replay = packet(10);
            store.recover(&mut replay);
            assert!(!replay.extension, "old SDP cannot undo retract {n}");
            assert!(store.0.lock().unwrap().announced.is_empty());
        }
        for n in 0..MAX_SCOPES {
            store.allow_track(&format!("never-published-{n}"));
        }
        store.retain_tracks(&HashSet::new());
        store.allow_track("track");
        store.accept(&r, &l);
        learn(&store, 10, "video", 3, 4);
        let mut last = packet(10);
        store.recover(&mut last);
        assert!(
            last.extension,
            "SDP cleanup releases abandoned announcement slots"
        );
        assert_eq!(store.0.lock().unwrap().announced.len(), 1);
    }
}
