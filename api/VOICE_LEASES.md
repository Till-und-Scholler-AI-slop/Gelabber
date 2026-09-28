# API voice / Live contract (05b)

Each Gateway connection has a process-independent UUID. Every voice join has a
fresh seat generation UUID; leave/rejoin never reuses its Redis seat key.
Redis TIME determines expirations, not API wall clocks.

* `gb:voice:seat:{seat_uuid}`: JSON `{id,u,s,c,m,d,p}`, PX 5000.
* `gb:voice:channel:{channel_uuid}` and `gb:voice:server:{server_uuid}`:
  sorted sets of seat keys, scores = expiration in milliseconds; PX 10000.
  Reads atomically prune expired scores and missing records. Seats aggregate
  by user; mute/deafen are true only if every active seat has that flag.
* `gb:live:{channel_uuid}`: JSON `{u,s,c,session,seat,nonce}`, PX 5000.
  `session` is the hex session fingerprint from 03a; `seat` is the join
  generation; `nonce` is a fresh Gateway UUID. It is not a bearer secret.
  Claim is SET-if-absent in Lua. Refresh/release compare the entire serialized
  owner. Even the same user's other tab/session cannot claim or release it.

Gateway renews active seats / claims every second under the 03a DB-session and
membership checks. Lost claims clear Live and emit `sig u/k:l`; a refresh never
reclaims an expired claim. Explicit start issues a new nonce. A dead process or
failed cleanup leaves at most 5 seconds of occupancy / claim. Passive server
watchers receive `sig r/snap` replacements on roster changes every second.

Live start acknowledgement / peer snapshot: `{"op":"sig","t":"p",
"s":"…","c":"…","u":"…","k":"l","lc":"<nonce>"}`.
Other signaling payloads are unchanged. Start is idempotent for its existing
owner. No ownership is selected from client signaling fields.

## Required Media03b / Web08b integration

Web awaits the Gateway Live `p` acknowledgement before announcing Live to SFU,
and forwards `lc`. The SFU must check the current `gb:live:{c}` record against
`lc` and the ticket's exact `u,s,c,auth.session`, in addition to all 03a auth
leases/nonces. Missing / expired claims and Redis errors fail closed. The old
`g=true` ticket flag alone does not authorize a Live publication.

SFU must also bind one exact media peer to that claim (e.g.
`gb:live:peer:{nonce}` = media-peer UUID, PX 5000, acquire NX and refresh/release
CAS). Validate the Gateway claim and acquire/refresh this peer lease atomically
in Lua; an expired/changed claim must not renew or create a media publication.
Renew while Live is active, recheck at attach and at least each second. Delayed
peer cleanup cannot delete a replacement peer's lease. API never touches this
Media-owned peer key. Claim replacement requires a new Live publication handshake.

Auth-demand/session contract remains as in ACCESS_REVOCATION.md. These API
changes do not by themselves enforce the SFU contract; shared/media are owned
by 03b/08b and were not changed here.

## Verification

Six real-Redis lease tests cover abrupt renewal loss, expiry, same-user
cross-process claim race, idempotent start, stale owner/seat cleanup, corrupt
index partial join, active renewal beyond TTL, Redis claim loss, and passive
watcher replacement. Separate API integration checks cover signaling, Gateway,
and all 03a regressions including pool max_connections=1. This models process
crash by stopping renewals without detach; a native kill + SFU frame test remains
part of integrated media acceptance. All keys are UUID scoped; no FLUSHALL.

## Multi-seat event followup

User-scoped join/mute/deafen/unpublish/leave deltas are derived from the current
Redis aggregate, and that aggregate read plus Pub/Sub emission is one Lua turn.
Leaving one seat updates remaining flags; unpublish removes a track only when
no valid remaining seat publishes it. Live union additionally requires the
current claim's exact seat generation. A resumed stale owner never emits a
user-scoped Live unpublish while the same user's replacement still holds it.
Live publish checks the exact serialized current owner in that same Lua turn.
Three real-PubSub regressions fail on 84cd5a3c and pass after this followup;
they cover both review findings plus camera/leave aggregation. Wire and Redis
key formats above are unchanged.
