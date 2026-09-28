# Live claim / Watch integration (08b)

This is a locally tested basis for review, **not final media acceptance**.
Firefox failures below remain open. Pins and lockfiles are unchanged.

## Runtime contract

Requires API03a authorization envelopes and API05b `VOICE_LEASES.md`
(`f18ef345` final contract; tested native API source
`3a30d44924708d9303eed778cd10d276742a5911` on local port 8082).
Old `g=true` tickets alone cannot publish Live. Web awaits the exact room/self
Gateway `sig p/k:l/lc:<nonce>` acknowledgement before adding the Live sender,
then sends media `p/k:l/t:<negotiated MSID>/lc:<nonce>`. Missing acknowledgement
times out after 10 seconds, releases capture and leaves Voice intact.

Media atomically checks membership/channel generations, exact session user and
Redis TIME expiry plus `gb:live:{channel}` JSON `{u,s,c,session,seat,nonce}`.
The Gateway record must match the ticket and nonce and have PTTL 1..5000 ms.
`gb:live:peer:{nonce}` binds that claim to one exact media-peer UUID, with a TTL
no longer than the Gateway claim. Acquire/refresh and claim validation share
one Lua turn. Refresh/release compare the peer UUID; stale cleanup cannot delete
a replacement peer's lease. Media never creates or renews the Gateway claim or
session lease. Checks at announce, publication attach and each second retain
the 500-ms Redis deadline and fail closed.

Each Live publication also carries a conservative monotonic lease deadline,
starting before Redis I/O. Reader and forwarders stop by that deadline, including
when no packets arrive. Live loss withdraws its handshake, stops RTP before
waiting on SDP cleanup or releasing the peer lease, and retains unrelated Voice,
camera and screen. Full authorization loss still closes the entire peer. A new
Gateway nonce needs a fresh publish handshake; an old withdrawn nonce cannot
resurrect Live.

Watch joins send optional `w:<selected publisher UUID>`. That media peer cannot
publish. Existing and newly attached subscriptions contain only that user's
Live video and every audio publication in the room. Web gives each Watch audio
track its own single-track MediaStream/Audio element, with shared Volume,
Deafen, output device and playback retry. Duplicate track events are idempotent;
ended tracks and Watch stop detach playback and blocked-play state.

Repeated explicit video starts reuse a sender per source kind with replaceTrack
and restore its send direction. Identities still come from accepted MID/MSID
bindings. The SFU retains at most 64 remote receiver bindings across inactive
offers, since the pinned driver may not repeat on_track; publications, readers
and subscriptions still stop and disappear. A temporary publisher Gateway
`sig l` keeps Watch intent for at most 20 seconds pending that same publisher's
Live acknowledgement. Explicit `sig u/k:l` and authorization failures remain
terminal. This UI grace does not authorize any RTP.

## Local evidence, 2026-09-28

Rust 1.98.1, Node 26.8.2, webrtc/rtc 0.20.5, Redis 8.10.1, PostgreSQL 18.6;
Chromium 153.0.8010.12 and Firefox 155. Browser autoplay policy is unmodified.
Own SFU/Web use separate local ports; baseline services were not replaced.
Firefox's test ICE adapter uses a real local interface, since its loopback policy
does not establish the loopback-only fixture. No raw ICE addresses are reported.

- 66 media/shared Rust tests, strict Clippy and build pass; 261 Web tests,
  lint and build pass. Logs: `/tmp/gelabber-media-08b-{rust,clippy,web}-final.log`.
- Real WS/Redis tests reject missing/mismatched/unleased claims, invalid seats,
  missing nonce, a second track or media peer, and Watch publishing. They cover
  stale CAS release, recovery peer acquisition, expiry with Voice retained,
  new nonce, renewal beyond five seconds and refusal to recreate a lost claim.
- Deterministic regressions cover claim loss between announcement and attach,
  RTP lease expiry before the next 1-s tick, and ongoing authorization checks
  while Live cleanup is blocked by SDP. Removing the attach recheck makes its
  regression fail (`/tmp/gelabber-media-08b-attach-before.log`).
- Actual Chromium App: Go Live/Watch, 30-s late join, media recovery, navigation,
  exclusive claim/loser cleanup, cancellation, source identity and autoplay retry
  pass. Initial full report exposes Gateway Watch termination and 20-cycle
  transceiver growth 2 -> 21; it is retained as before evidence, not acceptance.
- After the fixes, actual Chromium App Gateway recovery preserves the original
  peer, live capture and Watch frames. All 20 explicit Live cycles decode and
  retain exactly two publisher transceivers. Report:
  `/tmp/gelabber-media-08b-lifecycle-final.json`.
  Targeted Web regressions fail before the Watch grace/sender reuse fixes.
- Actual Chromium App Watch with room sources 440/880 Hz: two inbound audio
  sources, two playing single-track elements, decoded peaks 445.3/878.9 Hz around
  -48 dB, one selected Live video despite active camera/screen, no Watch capture,
  complete playback cleanup. `/tmp/gelabber-media-08b-watch.json`.
- Normal three-peer Voice was not changed. Corrected App captureStream analysis
  checks every captured audio track: two live output tracks and both remote
  tones (880/1320 Hz) around -48 dB. Earlier createMediaElementSource and
  first-captured-track measurements are invalid evidence, not product failures.
  `/tmp/gelabber-media-08b-voice-capture-diagnostic.json`.
- Existing native Chromium SFU regression passes after claim fixture updates:
  rollback/sender reuse, three sources, reoffers, 30-s late join, 20 screen cycles,
  Unicode error isolation and stopped Live cleanup.

The App adapter is a local copy of the E2E owner's harness under
`/tmp/gelabber-media-app-probe`; original E2E source was not edited. It uses static
preview without HMR, since closing Vite's HMR socket is not a Gateway fault.
Firefox rendering uses requestVideoFrameCallback when native playback-quality
counts are zero, and closed peers are not read through Firefox's throwing
localDescription getter. Reports label runtime source as working-tree code, not
an invented commit SHA or a verified deployment digest.

## Open Firefox cases and acceptance limits

The full actual Firefox App report is **not green**:
`/tmp/gelabber-media-08b-firefox.json`.
Go Live, late join, media recovery, navigation, claim exclusivity, all 20 real
Live cycles (two transceivers), cancellation and autoplay retry pass.

1. Gateway-only recovery intermittently ends the retained Live capture while
   the original media peer remains connected. A separate signal trace can pass;
   the timing/race cause still needs a confirmed fix and regression.
2. Camera + Screen + Live: Live decodes (34 frames/keyframe 1 in the failed
   snapshot), but camera/screen have no inbound publication. SFU logs two
   `publish failed / bad announce` events. This is not accepted as a codec or
   ICE defect; track/binding diagnosis remains open.

Final integration with Web04/06/07 and the coordinator's current API, independent
review, CI, HTTPS/WAN/forced relay, native display picker, two-device perceptual
speech and isolated UDP-loss quality remain outside this local basis. Shaping
all eth0 traffic also shapes Redis TCP and must trigger the unchanged fail-closed
authority deadline; it cannot serve as isolated media-loss evidence. No service
push, release, deployment or production action was performed here.

## BUNDLE-only followup

The Firefox camera/screen failure was reproduced in isolation. The actual local
offer has Live on port 9, camera and screen on port 0 **with a=bundle-only** and
active sendrecv directions. The SFU accepts those sections on the BUNDLE
transport, but both identity parsers previously discarded them as rejected.
Only one video identity remained registered and those incoming tracks failed
`bad announce`. This is a confirmed signaling/identity cause, not a codec pin.

Web/SFU now include port-zero bundle-only video only when its MID belongs to a
BUNDLE group with an existing nonzero-port transport anchor. A plain rejected
port-zero section, missing group/anchor, recvonly and inactive sections remain
excluded. The new Web regression fails before this change; Rust checks the
equivalent native-offer shape. 33 media library tests, strict media Clippy,
120 targeted Web media/session tests, lint and build pass.

Actual App camera + screen + Live now passes in both Firefox155 and Chromium153:
`/tmp/gelabber-media-bundle-{firefox,chromium}.json`. Each source is checked by
decoded and changing rendered frames with its expected canvas color. Assertions
and autoplay policy were unchanged. The full basis Firefox report is preserved
separately as `/tmp/gelabber-media-08b-firefox-full-before.json`.
Gateway timing and independent-review reader ownership races remain open;
this followup does not grant final runtime acceptance.

## Ended-reader ownership followup

The independent review's deterministic same-MSID replacement race was
reproduced locally: the replacement nonce was accepted, but delayed old-reader
cleanup cleared its source kind and prevented the replacement publication.
`ended_old_reader_must_not_clear_replacement_live_handshake` fails before the
fix (empty kind, zero publications) and passes after it (`l`, one publication).
Its lock barrier queues the real replacement handshake before ending the old
remote reader. Only its own Redis fixture keys are removed.

Cleanup now rechecks stop status and publication life under the Room lock,
and only mutates a receiver it still owns. Live additionally requires the same
deadline Arc/track binding, so an accepted new nonce with the same MSID cannot
be cleared even before the old publication's stop signal arrives. The existing
publication-removal life guard continues to protect replacement publications.
Live claim validation, lease deadlines and fail-closed authorization are
unchanged. This closes the confirmed reader race; it is not evidence that the
race caused the separate intermittent Firefox Gateway failure.

Before/after logs: `/tmp/gelabber-media-reader-race-{before,after}.log`.
All 61 media tests (including real RTC and WebSocket integrations), strict
media Clippy, media rustfmt and build pass. Logs:
`/tmp/gelabber-media-reader-race-{rust,clippy,build}.log`.
Workspace-wide rustfmt also reports pre-existing formatting in foreign
`api/tests/gateway.rs` and `shared/src/ice.rs`; these files were not changed.
