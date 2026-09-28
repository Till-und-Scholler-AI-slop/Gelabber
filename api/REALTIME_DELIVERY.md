# Realtime recovery contract (05a / Web06b)

## Epoch and transport sequence

Each topic (`s`, optional `c`) has a Redis UUID epoch. A subscribe can carry
`{"op":"s","s":"…","c":"…","n":42,"ep":"<uuid>"}`. Both cursor
fields are optional. `ok`, sequenced `e`, and `gap` include the current `ep`.
The server reads epoch, head, and replay log in one Lua turn. Replays belong to
that epoch and stop at the captured head. A changed epoch, a cursor ahead of
the current head (including head zero), or an unfillable replay hole yields
`gap`, then `ok` with the authoritative current epoch/head. Fresh subscribe
without `n` does not dump history; REST supplies it.

Web06b must keep `{ep,n}` per topic. On epoch change, reset the transport cursor,
accept smaller sequence numbers in the new epoch, and reconcile with REST.
On `gap`, refetch the relevant REST history/state; the following `ok` defines
the resume head. A new background event does not make an unfetched historical
message cache complete.

Pub/Sub reconnect emits `{"op":"resync"}` to every still-authenticated open
socket before live delivery resumes, including sockets with no topic yet.
Web06b refetches bootstrap/server and DM discovery state plus affected message
queries, and resubscribes/catches up its topics. This covers missed Pub/Sub
without requiring a browser disconnect. Subscription errors remove incomplete
server-side catch-up registration. An epoch change during catch-up also causes
resync. The Gateway periodically pings Pub/Sub and bounds connection, subscribe,
and ping waits; TCP failure resumes through the existing retry loop.

## Database revisions and durable delivery

Message REST bodies add integer `revision` (existing rows start at zero).
Message `e` frames carry `r`, the same database revision; create/edit `d`
contains the full message with `revision`, and delete contains `i` plus `r`.
These revisions survive Redis resets and are independent of transport `n`.
Web06b compares revisions per message, upserts by ID, and retains revision
floors/tombstones for deletes. Older HTTP responses / events must not overwrite
newer entities or resurrect a deletion whose revision has already been seen.
Duplicate transport events do not create duplicate messages.

Message create/edit/delete and Go Live text notices persist an outbox row in
the same Postgres transaction as the write. DM creation persists discovery for
both participants in its transaction. Channel advisory transaction locks order
writers and delivery; the database sequence allocates revisions while that
lock is held. Delivery retries automatically, in batches of at most 32 every
200 ms. An occupied channel is skipped so another channel can proceed. Retry
backoff preserves per-channel order. HTTP success means durable commit; Redis
failure retains the outbox rather than reverting a successful database write.

Redis publication assigns sequence, appends the bounded replay log, publishes,
and records the last delivered DB revision in one Lua turn. Retry after an
unknown publication/DB-ack result does not publish it a second time. Redis key
types are checked before writes to avoid partial execution for corrupt key
shapes. If Redis loses all state, the epoch changes; REST reconciliation and
per-entity revisions restore the client even if an unacknowledged delivery is
published again in that new epoch.

## DM discovery

`{"op":"dm","c":"<dm-channel-uuid>"}` is a private discovery notification,
not a sequenced chat event. Redis topic `gb:dm:{user_uuid}` targets only socket
sessions of that user; dequeue rechecks DB participation. It contains no peer
profile or message content. Web06b fetches `/api/dms` or `/api/dms/{c}`, adds the
DM, and subscribes with `s=c` and `c=c`. It fetches REST history (or resumes from
`n:0` with normal gap handling) so the first message sent before subscription
is visible. On resync, refetch the DM list even when no DM was previously known.
Foreign sockets cannot discover or subscribe to the DM.

## Redis and bounds

Existing `gb:n:{s|c}:{id}` / `gb:l:{s|c}:{id}` hold head and bounded replay.
`gb:n:{s|c}:{id}:ep` holds the UUID epoch; `…:delivery` is a hash of last delivered
DB revision and compact event. DM deduplication uses
`gb:dm:delivered:{user_uuid}:{channel_uuid}`. These recovery keys have no TTL;
loss is handled by the epoch/REST contract. Tests modify only UUID-scoped keys,
never FLUSHALL or unrelated clients.

Production socket output is a bounded 128-frame channel. Catch-up has an
aggregate 128-event limit per socket, across topics. Overflow cancels that
socket rather than dropping arbitrary messages or blocking fan-out to another
client. Socket sends have a one-second deadline. Reconnection uses the same
REST/catch-up contract. Outbox batches are bounded; persistent outage can retain
more durable database rows, intentionally, until successful acknowledgement.

## Deployment / acceptance limits

Migration 0007 is additive. Older API code can read the schema, but all writers
need the new API code to guarantee durable, ordered revisions. Existing Web
clients can read additive fields; complete client recovery needs Web06b to
handle epoch/gap/resync/discovery and revisions. This is not a media frame or
browser UI acceptance result. Shared/media/frontend and their pins are unchanged.

## Local verification (2026-09-28)

Pinned Rust 1.98.1 with PostgreSQL 18.6 and Redis 8.10.1: the complete locked
workspace/all-targets run passed 239 tests (`/tmp/05a-workspace.log`). Afterwards
one additional legacy-client future-cursor regression was demonstrated red
with the old planner and green with the fix (`/tmp/05a-legacy-before.log`,
`/tmp/05a-legacy-after.log`). API all-targets Clippy with warnings denied passed
(`/tmp/05a-final-clippy.log`). The full run covers the isolated Pub/Sub TCP
outage, durable Redis-failure retry / duplicate acknowledgement, concurrent
edit ordering, epoch reset, private DM discovery, and bounded slow consumers.
There has been no Web06b or production/browser recovery acceptance in this
worktree.
