# Negotiated SFU payload binding regression

Observed locally on 2026-09-28 with Rust 1.98.1, Node 26.8.2,
webrtc/rtc 0.20.5, Redis 8.10.1, Firefox 155 and Chromium 153.0.8010.12.
Pins and locks are unchanged.

## Reproduction

```sh
. /tmp/gelabber-toolchain/env.sh
export CARGO_TARGET_DIR=/home/rafi/Projects/Gelabber/target/stability-media
cargo build --locked -p gelabber-media
MEDIA_CASE=renegotiate MEDIA_BROWSER=firefox MEDIA_TEST_LAN=1 \
  MEDIA_ASSERT_PT_MAPPING=1 MEDIA_RESTARTS=3 MEDIA_ICE_RESTART=0 \
  node media/tests/browser-lifecycle.mjs
```

Firefox uses a real local interface for the isolated SFU ICE adapter. Binding only
to loopback did not establish this test connection; the Firefox loopback preference
alone did not resolve it. HTTP/signaling/Redis remain local, accounts are not
created, and no raw ICE addresses are reported. Empty end-of-candidates objects
are filtered in the test adapter. Autoplay rules are unchanged.

Against pre-fix `28d4505`, this exact browser probe failed its decoder budget:
the source encoded 100 AV1 frames / 23,812 bytes on PT99, but the subscriber had
no inbound video stream. Core trace confirmed incoming PT99 rejection against
subscriber AV1/PT41. The two-real-RTC-peer Opus PT109 -> PT111 regression also
failed before the fix.

After the fix, Firefox decoded AV1/PT41 from publisher PT99. Reoffers changed
Firefox's offered subscriber PT to 99; every accepted reoffer decoded again with
two transceivers and no errors. The real RTC regression additionally verifies
unchanged RTP payload, rewritten SSRC/PT111, and fresh packets after a reoffer
without adding transceivers. Unit cases cover exact MID selection, multiple VP9
profiles, rejected sections and a PT change.

## Pinned-library interoperability boundary

`TrackLocalStaticRTP::write_rtp` success only means driver queue acceptance.
The driver/core validate PT later. Sender parameters can expose PT0 preferences;
setting a local answer can restore an earlier leg's PT. The SDP media engine also
retains earlier mappings on reoffer. Firefox then offers AV1/PT99 while the generated
answer still contains PT41, which cannot establish a usable new decoder mapping.

The SFU resolves publication codec/profile on the sender's own transceiver MID,
normalizes only that codec's wire-answer PT and associated rtpmap/fmtp/rtcp-fb
attributes to the matching offered PT, and reconciles the concrete sender before
releasing RTP. The core rejects modified local descriptions, so its original local
answer remains internal; the wire answer and sender use the agreed mapping.
SFU-originated offers are resolved again from the browser's accepted answer.
Missing/ambiguous video formats release no RTP. No first-video-PT or default codec
fallback is used. Opus receiver tuning has one unambiguous MIME/clock/channel match.
The binding update also reopens feedback and requests a publisher keyframe.

## Checks and limits

38 media tests and strict media Clippy passed. The complete Chromium lifecycle
probe passed: rollback/sender reuse, three sources, five ICE restarts, 30-second
late join (1,053 ms), 20 separately decoded screen cycles, Unicode error isolation
and live-stop cleanup. Transceivers remained at four during the cycles.

The Firefox PT probe covers one actual AV1 source and reoffers; its generic
multi-source capture helper has a separate Firefox MSID limitation. Chromium
covers simultaneous three-source lifecycle. This is local transport evidence,
not production rollout, WAN/relay acceptance, integrated Go Live/Watch permissions,
Safari, native display capture or perceptual audio acceptance.
