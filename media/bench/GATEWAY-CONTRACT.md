# Candidate gateway acceptance contract

The comparison may eliminate an engine on a demonstrated resource lower bound,
but a passing forwarding fixture does not authorize a product migration. A
candidate must also run Gelabber's access and source-lifetime contract through
its actual adapter. Measure that adapter and every media worker together.

## What the current fixtures establish

| Fixture | Authentication and control executed | Product gap |
| --- | --- | --- |
| Current | `current-probe` mints production-format claims in private Redis; the actual media app consumes and revalidates them and implements Watch | Fixture minting grants access directly; it does not exercise API permission decisions |
| mediasoup | Static `BENCH_TOKEN`, controlled peer/transport/producer/consumer maps, native C++ workers | No product tickets, authority revalidation, moderation, Go Live lease or product Watch enforcement |
| Janus | Private `apisecret`, VideoRoom publisher/subscriber handles | No product gateway; the generator event broker only avoids browser HTTP connection exhaustion |

The latter two are forwarding adapters, not authenticated Gelabber backends.
Their measured resource use is incomplete for migration until the required
gateway runs in the measured server process tree. A resource result above the
limit can establish a lower bound; a result below it cannot establish that the
missing gateway would fit. Generator, TURN and shared Redis resources remain
separate and explicitly reported.

## Join and ongoing authority

Keep ticket issuance in the existing API (`api/src/media.rs`); it checks the
authenticated session, membership, voice-channel access and `join_voice`.
The gateway consumes the API's `AuthorizedTicketClaim` exactly once using the
atomic production implementation in `media/src/ticket.rs`. A plain base claim
or a static benchmark token is insufficient.

Bind every adapter peer to the claim's user, server, channel, session and
membership/channel generations. Never take those identities from subsequent
client RPC fields. Revalidate the actual authority during peer lifetime;
expired sessions, changed generations, missing authority and Redis failures
must close the peer and its backend transports. The gateway refreshes media
demand, while only the API renews session authority. Do not extend authority by
receiving traffic or accepting a repeated ticket.

Use the current revocation cadence and bounded Redis deadlines as the reference,
including races with join, leave, channel/server deletion, kick/ban and logout.
Check decoded media after revocation, not only a removed UI tile or closed socket.

## Source identity, Watch and Go Live

Preserve distinct microphone (`a` in SFU source metadata), camera (`v`), screen (`s`), Go Live (`l`),
screen audio (`sa`) and Go Live audio (`la`) identities from the current media
protocol. Bind backend producer/feed/handle identifiers, actual track MSIDs,
SSRCs and negotiated MIDs to those authorized sources. The native fixture's
microphone stream MSID `m` is not a new signaling source kind. Client-supplied producer
IDs and SDP order must not confer ownership. Microphone audio keeps the voice
room scope; source audio follows its parent's explicit Watch scope.

Go Live requires both the ticket's permission and the current API-issued live
claim, with exclusive acquisition, renewal and withdrawal from `media/src/live.rs`.
Stopping or replacing a source invalidates its particular publication lifetime;
late cleanup for an old lifetime must not retract a new source reusing a track
ID. Parent-source replacement also invalidates old paired audio.

Watch-on must identify an eligible other publisher in the same authorized scope.
Watch-off, access loss and parent-source stop must stop actual delivery to the
existing receiver, including retained decoders outside React. Audio and video
must not arrive again through a stale subscription or delayed negotiation.

## Signaling and adapter boundary

Keep Gelabber's WebSocket/signaling and product logic under its own gateway.
The existing wire contract is `media/src/protocol.rs` and `media/src/ws.rs`:
join, offer/answer, ICE, announce, abort, retract, Watch and leave. The benchmark
RPC contracts are not a replacement for this protocol.

Janus uses publisher and subscriber PeerConnections; the current implementation
uses a combined connection. An eventual frontend adapter must explicitly handle
this difference, renegotiation, source reuse, device/capture errors and recovery.
Preserve microphone continuity, independent source audio and bounded sender/
receiver resources. Backend administrative APIs and secrets remain private.
No candidate may add room, SDP, sender or engine bitrate ceilings to the default
unlimited product mode. Viewer-layer adaptation remains per viewer.

## Required executed evidence before selection

Run current and candidate adapters against the same API-generated authority and
the same media/source fixtures. Require positive delivery followed by bounded
stop for invalid/replayed tickets, expiry, authority-generation changes,
kick/ban, channel/server deletion, cross-room source requests, forged ownership,
Go Live claim races, Watch-off and stale source cleanup. Include delayed control
responses, reconnect, failed SDP and source restart with actual held receivers.

Then repeat the qualifying resource, CPU, media-latency and stability cases with
the complete candidate topology. Record exact engine/library, gateway, browser,
source and runtime hashes, adapter/worker process membership and cleanup. Keep
the original backend and matching state as an executed rollback option. Passing
this document's source review alone is not acceptance of any candidate.
