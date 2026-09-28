# Local SFU lifecycle regression

Run with the locked toolchain, installed `web/node_modules`, Playwright Chromium,
and an isolated local Redis (the runner accepts only 127.0.0.1). Build
`gelabber-media`, then run:

```sh
. /tmp/gelabber-toolchain/env.sh
export CARGO_TARGET_DIR=/home/rafi/Projects/Gelabber/target/stability-media
cargo build --locked -p gelabber-media
node media/tests/browser-lifecycle.mjs
```

The runner starts its own SFU on 18081, uses unique one-use test tickets with TTL,
and cleans up its processes. It uses actual canvas encoders/decoders and a gesture
for audio; there is no autoplay override. Its track identity helper is loaded from
production `web/src/voice/media.ts`. No accounts or production data are written.

## Observed on 2026-09-28

Rust 1.98.1, Node 26.8.2, Redis 8.10.1, Chromium 153.0.8010.12.

* Rollback/remove/add reused the exact Chromium sender with a different capture
  track but the old SDP MSID. The announced/retracted identity now comes from the
  negotiated sender MID/MSID, and the following publication decoded.
* Three correctly tagged video sources plus voice decoded. Five ICE restarts kept
  four transceivers and restored decoded progress on all sources.
* After 30 seconds, a new receiving peer decoded all three sources in 882 ms.
* All 20 screen stop/start cycles individually decoded, with four transceivers.
  The final source had 5 decoded frames/1 keyframe; the other sources progressed
  to 185/184 frames. A late peer after live-stop received only camera and screen.
* An oversized Unicode frame produced a controlled error; another peer continued
  decoding all three sources. Unicode unit regression failed before the fix.
* 33 Rust media tests and 232 Web tests passed; Web lint/build and strict
  media Clippy (`--all-targets -- -D warnings`) passed. Pins/locks are unchanged.

## Decoder diagnosis and negative control

With subscription deduplication but without the keyframe feedback interceptor,
one ordinary browser reoffer reset all three decoders to zero frames/keyframes.
Received bytes grew to about 43–47 KiB, each decoder sent 50 PLIs, and publisher
`pliCount` remained 1 per source. Both legs negotiated VP9/PT98. No subscriber
keyframe feedback reached `TrackLocal::poll`.

The pinned `rtc` default interceptor chain consumes RTCP before application polling;
`webrtc`'s `rtcp_processing_webrtc2webrtc` test explicitly adds an outer interceptor
to surface it. The SFU now surfaces only PLI/FIR while retaining the default NACK,
report and TWCC processing. The identical reoffer probe decoded all three sources
with four transceivers, and debug logs showed subscriber feedback reaching the
publisher. A main comparison also decoded but grew from four to eight transceivers.

Independent review found a second pinned routing constraint: the endpoint routes
only the first destination SSRC of a surfaced RTCP message. The follow-up splits
compound PLI packets and every FIR entry into individual routed messages. The real
two-RTC-peer regression failed before that fix; afterward three compound PLI A+B
batches reached both sources three times, and multi-target FIR reached both with
their original sequence numbers. A 100-target FIR also verified the queue bound
of 64 routed messages. Existing media tests and strict Clippy remained green.

## Production web session recovery (assignment 02)

`MEDIA_CASE=recovery node media/tests/browser-lifecycle.mjs` starts an additional
local Vite server on 15173 and imports the actual `session.ts` and media socket.
It denies microphone capture, verifies decoded video and voice reception, starts
one synthetic screen capture with a click, closes the media transport and rejects
ticket requests for eight seconds. On the final 01+02 stand, Chromium 153 recovered
in 9,156 ms and Firefox 155 in 11,946 ms. Each retained the same live capture ID,
with one display request, one denied microphone request, no empty candidate sent
and no reported error. Both directions decoded again; receive-only room audio
continued. Final Leave ended the retained track. Firefox's test stats helper skips
closed-peer SDP reads during transport replacement and final Leave.

The matching capture-preservation unit regression failed before the fix. Web
regressions cover a nine-second ticket outage, seven bounded retry attempts,
single terminal error, leaving during a pending ticket request, timer cancellation,
ended display capture needing a new explicit toggle, fresh TURN credentials on
transport replacement, and receive-only audio when microphone capture is denied.
241 Web tests, lint and build passed. Existing ICE restart recovery remains in place;
transport replacement has seven retries, 250 ms exponential backoff capped at four
seconds and +/-20% jitter. A new permission denial stops rather than retrying.

Firefox separately reproduced an empty-string end-of-candidates object. Sending
it caused a controlled SFU error and an initial Join rollback; Voice/Watch now skip
it. The localhost loopback adapter did not connect in Firefox, including with the
loopback preference. `MEDIA_BROWSER=firefox MEDIA_TEST_LAN=1` binds the isolated
SFU to a real local interface. This is a test adapter choice, not a product ICE
policy or WAN acceptance. Autoplay rules remain unchanged. The real AV1/PT mapping
and reoffer regression is documented separately in `PAYLOAD_BINDING.md`.

To run just that comparison against a separately started local SFU:

```sh
MEDIA_CASE=renegotiate MEDIA_RESTARTS=1 MEDIA_ICE_RESTART=0 \
  node media/tests/browser-lifecycle.mjs
```

## Limits

These are direct local SFU transport regressions, not an integrated application,
Go Live/Watch authorization, relay/WAN, Safari, native display gesture or
perceptual audio acceptance. Integrated lifecycle/Watch checks and production
acceptance remain coordinator work. Fresh TURN credentials are covered by ticket
replacement regressions, not a long-running real TURN expiry trial. Rust barrier
tests separately cover rollback
serialization, last-leave/join and port release only after close finishes.
