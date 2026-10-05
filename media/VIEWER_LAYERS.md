# Per-viewer video representations (media protocol 3)

This draft depends on the audio-quality work in PR #156. Stack and browser dependency pins stay unchanged. **Blocked on the unmodified rtc 0.20.5 RID reoffer behavior described below; the expanded native matrix is not yet a passed release gate.** The current web client publishes independently decodable VP8 `q` (quarter width/height) and `f` (capture resolution) simulcast representations where the native browser supports RID. Camera, screen and Go Live keep their separate publication identities. Both native senders have no application bitrate ceiling by default; optional user upload/Economy budgets are still distributed across sources and encodings. Browser congestion control and real negotiated codec/receiver limits remain effective. A browser without this capability keeps its existing single-representation path.

A v3 receiver sends its rendered video height including display density and measured per-track RTP loss:

```json
{"op":"q","u":"publisher-uuid","k":"s","h":360,"congested":false}
```

The hint affects only that peer's existing subscription. It cannot create a Watch, select an unauthorized Live publisher, or change another viewer. Camera, screen and Live are distinct. v2 explicit Watch and source-audio behavior remain compatible. The controller samples native counters every two seconds; unknown/restarted/repaired counters are not bandwidth estimates. More than 3% loss over at least 20 packets asks for the small representation; four healthy intervals recover. Hidden/unknown geometry asks for the full representation.

The SFU switches only at a complete target VP8 keyframe. It reassembles bounded, contiguous frames across sequence/timestamp wrap and cross-frame reordering, preserves VP8 temporal metadata, and rewrites RTP sequence/timestamp, PictureID, TL0PICIDX and KEYIDX into one continuous subscriber reference space. Missing references pause deltas until a keyframe. Initial and receiver PLI/FIR requests reach the actual publisher layer SSRC; requests are coalesced. Older VP9/H264/AV1 single-representation sources keep their codec and payload mapping. There is no VP9 SVC implementation or mixing of its reference layers.

The pinned rtc `add_track` chooses the first sender-free video MID without codec matching. Incoming publication MIDs are therefore reserved with data-free, recvonly local sender objects. No placeholder RTP is sent and recvonly reservations produce no outgoing browser tracks. Separate subscriber MIDs stay reusable, including after Watch withdrawal and across legacy codecs. Only the exact simulcast publisher MID negotiates VP8; other receiving MIDs retain native decoder capabilities. The pin has incomplete repaired-RID demultiplexing, so these RID publisher MIDs advertise primary NACK/PLI/FIR rather than unsupported RTX/RRID.

Per-peer egress drains audio before video and can preempt a blocked video enqueue for audio. Video admission is a whole frame; four queued video frames, sixteen audio batches, 2048 packets/2 MiB per assembled frame, three pending frames per layer and at most three layers keep memory bounded. Full admission, a publisher broadcast lag or a queued frame older than 150 ms requests the small representation for eight seconds. A real RTC write failure also invalidates decoder references. A pending driver enqueue is cancelled on peer/subscription/publication/Watch closure or Live expiry; the worker is aborted when its final owner leaves. This is local queue pressure plus receiver loss adaptation, **not** a complete per-viewer TWCC/GCC bandwidth estimator or a bitrate guarantee.

## Local acceptance

Fast media tests cover frame completeness, missing references, reordering/wrap, codec/source contracts, invalid hints/no Watch grants, 600-packet multi-RID broadcast overflow with low-keyframe recovery followed by a complete large full keyframe, and cancellation of a held bounded driver send for all five revocation scopes. Web tests cover native helper fallback, receiver loss hysteresis and source retirement.

The opt-in native acceptance runs an owned in-process SFU on ephemeral loopback ports. It uses actual canvas/tone media, the product `addLayeredVideo`, `ViewerLayerController`, and accepted-MSID helper, and real Chromium/Firefox RTP/decode counters; it does not contact API, Redis or user calls. Install the locked web dependencies first, then run:

```sh
cargo test -p gelabber-media sfu::layer_native_tests::native_browser_rid_feasibility -- --ignored --exact --nocapture
```

The runner requires two simultaneous 90/360px viewers, independent changes and loss-intent recovery with continuing audio; both browser engines; both cross-browser directions; current camera/screen/Live alongside unmodified v2 default-codec and explicit VP9 sources; actual viewport controller association; and twenty alternating foreign-source Watch cycles with stable native/SFU transceiver counts and reusable subscriber MIDs. A missing encoder, empty decoder counter, unsupported VP9 or a phantom receiver fails the test.

These controlled native tests do not prove physical-device audio/video quality, mobile behavior, TURN/WAN behavior, a one-hour call, or a full network bandwidth estimator. Those remain separate release acceptance gates. Native encoders may reduce the full representation under their own congestion control; a fixed capture resolution is not promised as a network invariant.

## Pinned-core blocker and experiment

rtc 0.20.5 `peer_connection/internal.rs::start_rtp` recreates RID coding entries with an unknown SSRC on every remote description, including answers to SFU subscription offers. Its endpoint then requires MID/RID RTP extensions to rediscover that SSRC. Native Chromium can keep sending acknowledged SSRCs without repeating those extensions, so a camera/screen publisher continues encoding while SFU ingress stops. This is before the frame assembler and is not solved by changing a subscriber bitrate. The wrapper exposes no public mutation of the core receiver's coding map.

An isolated `/tmp` experiment retains learned coding entries only for the same MID receiver, stream ID, track ID and RID. It keeps version 0.20.5 and all other crates unchanged. The repository and Cargo registry do not contain this patch. A full native matrix and review of that patch are required before deciding whether to vendor it; this draft must not be merged as a functioning RID feature while the blocker remains.

The SFU adapter independently preserves the codec selected for each incoming publication MID across SFU reoffers. AV1 omitted `profile`, `level-idx` and `tier` are matched only to their specified defaults (0, 5, 0); other real receiver declarations remain distinct. The [AV1 RTP specification §7.2](https://aomediacodec.github.io/av1-rtp-spec/#72-sdp-parameters) defines these defaults and peer limits.
