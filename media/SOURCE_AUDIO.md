# Media protocol version 2: source audio and explicit Watch

New clients send `v: 2` in the media join; the acknowledgement carries `v: 2`. The gateway still uses its
existing compact `sig` messages; `k: "sa"` and `k: "la"` are additional track
kinds. Microphone `a`, camera `v`, screen `s`, and Live `l` keep their identities.
A deployment must update media, gateway, and web together. Missing media `v`
means the legacy protocol: a new web client must omit source-audio senders and
announcements and show the unsupported-audio notice, while keeping video-only
capture usable. It must also skip `op: "w"` against a legacy media server.
Older web clients with no join version retain microphone, camera, and automatic
screen/Live video forwarding. The SFU never sends source audio to those legacy
peers and refuses their source-audio announcements. Version 2 voice peers receive
source video and audio only through explicit Watch. This preserves legacy video
behavior while ensuring legacy viewers cannot mix source audio into voice audio.

## Publish and pair

A display capture can provide video without audio. Announce each present track
by its exact native MSID track ID before sending its offer:

```json
{"op":"p","k":"s","t":"display-video-id"}
{"op":"p","k":"sa","t":"display-audio-id"}
```

For Live, `l` and `la` both include the same gateway-issued `lc`. Audio never
acquires a new exclusive claim. An audio announcement requires one matching
parent video announcement on that same peer; source audio requires explicit
`t` and cannot use legacy SDP-order fallback. The gateway also requires the
parent publication on that exact voice socket, so another tab cannot attach
audio to its sibling's screen or Live claim. Live audio has the same current
claim, peer ownership, access authority, permission, and local lease deadline
as its parent. Browser tracks without explicit source-audio announcements remain
legacy microphone tracks; clients must never send source audio to legacy media.

Source audio uses Opus at 48 kHz, two channels, stereo and sprop-stereo enabled,
192000 bits/s, FEC enabled and DTX disabled. Frontend display constraints and
sender tuning keep speech processing off independently of microphone settings.
The SFU forwards the encoded stream without transcoding. Internal publication
identities use `<user>:sa` / `<user>:la`; outgoing native stream IDs and CNAME
match `<user>:s` / `<user>:l`, while outgoing audio track IDs remain
`<user>:sa-<ssrc>` / `<user>:la-<ssrc>`. Prefer the current parent stream ID
for audio source classification, then the tagged audio track ID as fallback.
Browsers keep a receiver track ID immutable when a stopped sender is reused;
its current MSID can move from screen to Live while that old track ID remains.
This lets the browser associate captured video and audio for synchronization.

## Watch

Version 2 voice peers receive microphone and camera by default. Explicit Watch
selects one publisher and parent kind, including both its video and source audio:

```json
{"op":"w","u":"publisher-uuid","k":"s","on":true}
{"op":"w","u":"publisher-uuid","k":"s","on":false}
```

The allowed parent kinds are `s` and `l`. Intent can precede publication and must
be replayed after rebuilding the media peer. A dedicated Live watch peer joined
with `w: <publisher-uuid>` continues to receive that publisher's `l`/`la` and
channel microphone audio; it cannot publish or change its selection via `op:w`.
The SFU excludes every publication by the viewer's own user, including when
publisher and watcher are different media peers. Closing Watch revokes a
per-viewer forwarding gate before waiting for subscription SDP cleanup.

Retracting or ending a parent source removes its paired audio immediately;
source-audio stop alone leaves the parent video and microphone running. A
stopped or revoked source cannot resume through an old pending track event.
Gateway snapshots omit audio associated with a stale Live claim. Parent `u`
also emits the paired audio `u` when no other live seat still publishes it.

## Verification

`media/src/sfu_lifecycle_tests.rs` covers source identity, Watch selection,
unsubscribe before a busy SDP gate, parent cleanup and independent microphone.
`media/tests/live_claim.rs` verifies the same exact claim on the media wire;
`api/tests/signal.rs` checks same-socket pairing and removal on the gateway wire.
`web/scripts/smoke-source-audio.mjs` exercises two Chromium clients with synthetic
display audio and real signaling/SFU. Set `GELABBER_SOURCE_AUDIO_RECEIVER=firefox`
to run the receiver in Firefox, including screen/Live video decoding, source audio,
and stop/re-Watch. Install both Playwright browsers first. The Firefox profile
allows loopback ICE for the required local test stack; it is not WAN acceptance.
Browser/OS native tab or system capture
availability still depends on the selected source and browser capture support.
