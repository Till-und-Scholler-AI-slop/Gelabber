# Media authority enforcement (03b)

Implements the API contract in `api/ACCESS_REVOCATION.md` at `5f5fd35`.
The API must mint the additive `auth` envelope before this SFU is integrated.
Legacy envelopes are intentionally rejected. No API source, stack pin or lock
was changed here.

Ticket consumption uses one Redis Lua operation: GETDEL, envelope validation,
membership/channel generation comparison, exact session-user comparison and
Redis-time expiration check. Invalid tickets remain one-use. Rust additionally
validates UUIDs and the complete envelope. Join revalidates after building the
peer connection and waiting for the room attach lock. Every active peer checks
the same authority once per second; each Redis operation has a 500 ms deadline.
Missing authority, mismatched generations, session expiration or Redis failure
revokes the peer. Only successful validation refreshes demand with EX 35.
Media never renews the three-second session lease or deletes shared demand on
one tab's leave. The old 120-second deny key no longer blocks a fresh generation.

Revocation removes the room peer and publications, signals both forwarding
directions to stop, then waits for SDP cleanup and actual peer close. Gathering
also observes that stop signal. A busy SDP gate therefore cannot keep RTP
authorized while teardown waits. The server sends an unauthorized error and
WebSocket Close; the negative clients do not voluntarily leave.

## Reproduction and checks

Run with the locked toolchain and isolated local Redis:

```sh
. /tmp/gelabber-toolchain/env.sh
export CARGO_TARGET_DIR=/home/rafi/Projects/Gelabber/target/stability-media
export CARGO_BUILD_JOBS=1
cargo test --locked -p gelabber-media -p gelabber-shared --all-targets -- --test-threads=2
cargo clippy --locked -p gelabber-media -p gelabber-shared --all-targets -- -D warnings
cargo build --locked -p gelabber-media
MEDIA_CASE=revoke node media/tests/browser-lifecycle.mjs
```

Observed 2026-09-28 with Rust 1.98.1, Redis 8.10.1, Node 26.8.2 and Chromium
153.0.8010.12: 50 media tests and seven shared tests passed; strict Clippy passed.
Ten real-WebSocket negative cases cover legacy/malformed envelopes, consumed
ticket replay, stale member/channel generations, exact-session logout, absolute
expiration, API lease expiry, Watch-only demand across two tabs, immediate fresh
rejoin and an isolated Redis connection failure. Other-session/other-room controls
remain authorized. Test keys are unique; no FLUSH or shared Redis interruption.

Two deterministic barriers cover revocation after initial validation but before
room attach, and revocation while the peer's SDP gate is held. The attach test
fails when only the second authority check is removed. The legacy WebSocket
test fails on the prior implementation because it attaches the peer.

The browser case first decodes a canvas publication with room audio, then removes
the publisher's exact-session authority while its browser stays open. It requires
server closure within two seconds, no subsequent RTP progress, continued decoding
in a separate control room, and no revoked publication for a late joiner. The final browser run revoked after 947 ms. The same
case fails against the old SFU: no server revocation within the deadline. Its test
fixture simulates only the API lease writer and does not assert real API logout.

One unrestricted-parallel Rust run hit the pre-existing UDP-port test fixture's
bind gap (`no udp_sockets or tcp_listeners available`). The full two-thread run
passed. Full workspace and API/DB-chain acceptance remain integration checks.

## Actual application check for 02

Web `c482901` was also exercised through the existing real application E2E
harness, using isolated UI-created accounts/server, the then-running baseline
API and a separate compatible SFU `28d4505`. With normal autoplay, Go Live/Watch
decoded the correct changing canvas with zero Watch capture calls. Closing the
publisher's media WebSocket retained the exact live capture and sender binding,
made no additional display request and restored decoded Watch progress. An
eight-second browser-offline case retained capture, but its established UDP could
continue flowing, so it is not a transport-outage proof.

The first Gateway interruption also closed a Vite-HMR socket classified as
Gateway by the adapter, causing a page reload. Retesting the static Web build
against API `3a30d44` and this 03b working-tree SFU targeted exactly one Gateway
socket and passed: the original connected media peer and live capture remained
identical, with one display request. Media-WebSocket replacement also passed on
that combination, including resumed decoded Watch frames. Go Live/Watch decoded
its first correct frame in 613 ms without Watch capture. Navigation controls and
exclusive Live publication remain subsequent 08 work.

Existing lifecycle regression stayed green with the new authorized fixture:
Chromium decoded all three sources through five ICE restarts, 30-second late join
and 20 capture cycles. Firefox AV1 decoded through three PT-changing reoffers
with two transceivers. Production-session capture recovery passed in 9,426 ms
(Chromium) and 12,688 ms (Firefox), with one display request and one denied
microphone request each. Firefox used the documented real-interface test adapter.

Private raw logs are under `/tmp/gelabber-media-auth-*.log`; the application
report is `/tmp/gelabber-media-app-02.json`. No production action occurred.
