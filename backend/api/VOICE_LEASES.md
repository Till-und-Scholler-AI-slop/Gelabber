# API voice / Live contract (05b)

Each Gateway connection has a process-independent UUID. Every voice join has a
fresh seat generation UUID; leave/rejoin never reuses its Redis seat key.
Redis TIME determines expirations, not API wall clocks.

* `gb:voice:seat:{seat_uuid}`: JSON `{id,u,s,c,m,d,p}`, PX 5000.
* `gb:voice:channel:{channel_uuid}` and `gb:voice:server:{server_uuid}`:
  sorted sets of seat keys, scores = expiration in milliseconds; PX 10000.
  Reads atomically prune expired scores and missing records. Seats aggregate
  by user; mute/deafen are true only if every active seat has that flag.
* The one-per-channel Live claim (`gb:live:{channel_uuid}`, the `lc` nonce and
  track kinds `l`/`la`) is gone (#353). A screen is the seat's own `s`
  publication, several per channel.

Gateway renews active seats every second, right after the 03a DB-session and
membership checks of the same tick (the session transaction ends before Redis
I/O). A dead process or failed cleanup leaves at most 5 seconds of occupancy.
Passive server watchers receive `sig r/snap` replacements on roster changes
every second. Each API process reads a server's roster once per second for all
its sockets on that server and queues it to those whose last roster differs;
roster frames are authorized again at dequeue. A socket stops getting a
server's rosters, presence and signaling when it unsubscribes the last topic of
that server and holds no seat there.

No ownership is selected from client signaling fields. Events a join replays
to the newcomer (occupants' `j` and their `p`) carry `"replay":true`.

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
no valid remaining seat publishes it.
Three real-PubSub regressions fail on 84cd5a3c and pass after this followup;
they cover both review findings plus camera/leave aggregation. Wire and Redis
key formats above are unchanged.
