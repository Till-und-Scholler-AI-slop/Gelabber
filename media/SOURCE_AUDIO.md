# Media protocol v4: source audio and explicit Watch

Gelabber owns tickets, room authority and its WebSocket signaling. The sole media
engine is the official Rust mediasoup 0.29.0 binding; web uses the public
mediasoup-client 3.24.1 SDK. See the [migration document](../docs/v0.4-mediasoup-migration.md)
for the native build and coherent previous-application rollback boundary.

Clients join with `op: "j"`, a positive increasing request `id`, the one-use `tk`
and `v: 4`. A dedicated Live viewer additionally sends `w: <publisher UUID>`.
The correlated result includes `v: 4`, room/user identity, a peer generation and
`routerRtpCapabilities`. Other protocol versions receive `update_required` before
ticket consumption. API, media and web must be updated together.

## Publish and pair

Source kinds are microphone `a`, camera `v`, screen video `s` (a Live) and
screen audio `sa`. The old Go Live kinds `l`/`la` are gone (#353). A display
capture may supply only video.
The public SDK creates send/receive transports and supplies its actual RTP/DTLS
parameters to Gelabber's correlated RPCs; media WebSocket messages carry no SDP.

Each successful `produce` returns an actual native `producerId` and `epoch`.
`epoch` identifies the capture lifetime: paired video/audio share one capture
UUID, and a new capture receives a new UUID. Source audio requires the same
peer's current parent Producer ID and capture epoch. For example, the adapter's
produce callback sends the equivalent of:

```ts
const video = await request("produce", {
  k: "s", rtp: videoRtpParameters, epoch: captureEpoch,
});
await request("produce", {
  k: "sa", rtp: audioRtpParameters, epoch: captureEpoch,
  parent: video.producerId,
});
```

The request helper supplies the increasing `id`; RTP parameters come from the
real SDK callback. The Gateway requires source audio's parent on that exact voice
socket. A child cannot attach to another tab's Producer or an old parent ID or
epoch.

A codec-only Producer replacement keeps the capture epoch and supplies
`expectedOldProducerId`. The server stops the old publication and its children
before committing the replacement; children must then bind to the new actual
parent ID. Late cleanup of an old ID cannot stop a replacement sharing its epoch.
Transport compaction closes the old native resources before republishing retained
captures, video before paired audio. It preserves capture epochs and rewrites
child parent IDs.

Source audio uses Opus at 48 kHz with two channels, stereo and FEC enabled and DTX
disabled. Display constraints disable speech processing independently of the
microphone mode. Default source-audio bitrate has no application cap; an explicit
user economy setting can supply one. The SFU forwards encoded media without
transcoding. Incoming identity is the authenticated Consumer announcement's
`owner`, `k`, actual Producer/Consumer IDs, `epoch`, `generation` and optional
`parent`. Browser track IDs, MSID, SDP order and CNAME are not source classifiers.

## Watch, Ready and privacy

Normal voice peers receive room microphone audio and cameras automatically.
Explicit Watch selects one publisher's screen (their Live) and its paired audio:

```json
{"op":"w","id":10,"u":"publisher-uuid","k":"s","on":true}
{"op":"w","id":11,"u":"publisher-uuid","k":"s","on":false}
```

The only Watch kind is `s`. Watch intent may precede publication and is
replayed on a new media peer. There is no watch-only peer any more: watching
happens inside the voice room. Every peer excludes publications by its own user,
including another tab's publications.

Every native Consumer starts paused. Its `consumer` announcement carries the
actual `consumerId`, `producerId`, source identity, a fresh subscription
`generation`, RTP parameters and the source pause state. The browser first
completes public SDK `consume` and attaches that exact receiver, then sends
`consumerReady` with the same ID/generation. The server checks ownership and
current grants before and after native resume; `consumerState` carries the
confirmed effective pause state. Failed attachment reports `consumerFailed`.
Retired or mismatched generations cannot authorize a new receiver.

Watch-off, parent close, rights revocation and peer leave invalidate all affected
grants before native awaits. Producer/Consumer pauses share a 500-ms aggregate
phase covering queued stops, resource gates and actual native acknowledgements,
with at most 64 jobs in flight. An unconfirmed stop terminates the media process
so native forwarding cannot continue. Graph/UI cleanup follows confirmed stops. Closing audio alone retains video and microphone; closing
or replacing the parent retires its paired audio. Old Ready and resume requests
cannot revive a revoked source. Full authority loss retires the entire peer.

## Verification and limits

Current control tests use real mediasoup resources and the v4 WebSocket/Redis
contract:

- `media/tests/mediasoup_control.rs` covers peer-scoped transports, Producer
  replacement/late cleanup, exact parent/epoch pairing, automatic microphone and
  camera sources, Watch selection and ConsumerReady generation/ownership.
- `media/tests/access_revocation.rs` covers session/channel authority loss and
  redis failure. The stop-batch unit tests in `media/src/sfu/tests.rs` cover concurrent
  acknowledgements and an aggregate timeout while resources remain queued.
- `api/tests/signal.rs` covers same-Gateway-socket pairing and parent cleanup.

These control checks do not establish actual RTP playback. The existing
`web/scripts/smoke-source-audio.mjs` uses the product with synthetic display audio
and real native transport/playback. `GELABBER_SOURCE_AUDIO_RECEIVER=firefox` selects
a Firefox receiver; both Playwright browsers must be installed. Browser results
must identify the exact executed runtime/test freeze, and older SFU reports remain
historical. `media/tests/browser-lifecycle.mjs` (removed in v0.7.1) and the dated
media test documents record the previous engine and are not v4 acceptance gates.
Native tab/system capture availability, audible quality, physical devices and
WAN/relay acceptance remain separate from these local synthetic controls.
