# Spec: Gateway (`/ws`) and media socket (`/media/ws`) protocols

Area: protocol-gateway. Audience: agents writing the native Rust/egui client.
Everything here is taken from the code on branch `claude/native-foundation`
(off `v0.9`). File:line references are to that tree. Where the web client and
the server disagree, both are stated.

Conventions used below:

- `<uuid>` = lowercase hyphenated UUID string (serde `Uuid` default), e.g.
  `"0b9f7c1e-2d4a-4f1e-9a52-6f3f0c9a1b23"`.
- "omitted" = the key is absent from the JSON object (serde
  `skip_serializing_if`), never `null`, unless the table says `null`.
- All frames are WebSocket **text** frames containing one JSON object.
  No binary frames are used by either protocol.

---

## 0. Overview

| Plane | URL path | Process | Auth | Purpose |
|---|---|---|---|---|
| Gateway | `/ws` | `api` (Axum, port 8080 behind Caddy) | `gelabber_session` cookie on the HTTP upgrade | chat events (sequenced), presence, typing, voice occupancy (`sig`), account frames |
| Media control | `/media/ws` (also `/ws` on the media port itself) | `media` (port 8081 behind Caddy) | single-use 12-char ticket in the first frame | mediasoup control RPC: transports, produce, consume. No SDP/ICE candidate exchange |
| REST ticket | `POST /api/channels/{channelId}/media-ticket` | `api` | session cookie + CSRF header (REST rules) | mints the media ticket and ICE servers |

Routing: `deploy/compose/Caddyfile:57-67` (`handle /ws` → `api:8080`, no read/write timeout),
`deploy/compose/Caddyfile:89-100` (`@media path /media /media/*` → `media:8081`).
Both are on the same origin as the web app.

Signaling split (by design, `backend/api/src/gateway/mod.rs:15-17`,
`backend/api/src/gateway/protocol.rs:1-5`):
- `op:"e"` is the only sequenced stream (chat create/edit/delete, server-level kick/ban).
- `op:"sig"` is voice **presence** only (join/leave/publish/mute/deafen/snapshot). SDP/ICE never go to `/ws`.
- Media negotiation is mediasoup parameters on `/media/ws`.

---

## 1. Gateway `/ws` — connection

### 1.1 URL and handshake

| Item | Value | Ref |
|---|---|---|
| URL | `wss://<host>/ws` when the app origin is https, else `ws://<host>/ws` | `web/src/ws/client.ts:53-59` |
| Method | HTTP GET upgrade | `backend/api/src/gateway/mod.rs:44-46` |
| Auth | Cookie `gelabber_session=<raw token>` (the raw token from the login/register `Set-Cookie`) | `backend/api/src/cookies.rs:8`, `backend/api/src/auth/session.rs:282` |
| CSRF | Not needed for `/ws` (GET) | `backend/api/src/auth/session.rs:270-272` |
| `Origin` header | Optional. If **absent** the upgrade passes (non-browser clients: "future native apps"). If present it must equal `Host` (scheme default ports ignored); `null` or non-ASCII is refused. Behind TLS (`API_COOKIE_SECURE=true`) an `http://` origin is refused unless loopback. **Native client: send no `Origin`.** | `backend/api/src/gateway/mod.rs:75-117` |
| Failure: no/expired session | HTTP 401 (`CurrentSession` extractor), no upgrade | `backend/api/src/gateway/mod.rs:48-53` |
| Failure: origin mismatch | HTTP 403, body error `"Cross-origin WebSocket is not allowed."` | `backend/api/src/gateway/mod.rs:54-71` |
| Max inbound text frame | 16 KiB (16384 bytes); larger → `{"op":"err","e":"bad_request"}`, socket stays | `backend/api/src/gateway/conn.rs:22,114-117` |

After the upgrade the server immediately attaches the socket; nothing is sent
until either a server heartbeat tick (first after 15 s) or the client's first
frame (`backend/api/src/gateway/conn.rs:70-74`). There is **no hello/identify
frame**; the session cookie is the identity.

### 1.2 Timing constants

Server (env-configurable, defaults shown) — `backend/api/src/config.rs:20-25,82-89`:

| Constant | Env | Default | Meaning | Ref |
|---|---|---|---|---|
| `ws_heartbeat` | `API_WS_HEARTBEAT_MS` | 15 000 ms | server sends `{"op":"h"}` every tick (first tick skipped, so first `h` at t+15 s) | `conn.rs:70-74,170-191` |
| `ws_dead` | `API_WS_DEAD_MS` | 30 000 ms | server closes when nothing (text/ping/pong/binary) arrived for ≥ 30 s, checked on each heartbeat tick | `conn.rs:177-180` |
| `ws_idle` | `API_WS_IDLE_MS` | 300 000 ms | without an *activity* frame for 5 min the server marks this socket idle (`i`) | `conn.rs:181-184` |
| presence TTL | `API_WS_PRESENCE_TTL_MS` | 45 000 ms | Redis TTL of presence keys, refreshed by heartbeats | `config.rs:86`, `live.rs:241-248` |
| typing TTL | `API_WS_TYPING_TTL_MS` | 6 000 ms | Redis TTL of a typing key. **The server sends no `on:false` at expiry** — the client must time out | `live.rs:441-485` |
| replay buffer | `API_WS_REPLAY` | 256 events per topic | bounded Redis list for resume | `config.rs:84`, `event_log.rs:78` |
| focus TTL | — | `2 × ws_dead` = 60 000 ms | a `f:on` claim, refreshed each heartbeat tick while focused | `conn.rs:68-69,185-187` |
| security backstop | — | 10 s (1 s while holding a voice seat) | session + ACL recheck cadence | `conn.rs:24-31,282-320` |
| voice seat lease | — | 5 000 ms | a seat not renewed (socket gone) expires after 5 s | `leases.rs:9` |
| media ticket TTL | `MEDIA_TICKET_TTL_SECS` | 30 s | | `config.rs:89` |
| outbound queue | — | 1024 frames | overflow closes the socket (client reconnects/resyncs) | `hub.rs:28-36,105-112` |
| catch-up buffer | — | 128 live events buffered across topics while subscribing; overflow closes socket | `hub/fanout.rs:8,226-235` |

Client (web) — `web/src/ws/client.ts:48-51`:

| Constant | Value | Use |
|---|---|---|
| `DEFAULT_HEARTBEAT_MS` | 15 000 | client sends `{"op":"h"}` every 15 s (`setInterval`) — `client.ts:599-603` |
| `DEFAULT_DEAD_MS` | 30 000 | watchdog: if no server frame for ≥ 30 s, close the socket (→ reconnect) — `client.ts:604-611` |
| watchdog period | `min(1000, heartbeatMs)` = 1000 ms | `client.ts:610` |
| `DEFAULT_RETRY_MS` | 200 | backoff base |
| `DEFAULT_MAX_RETRY_MS` | 2 000 | backoff cap |

### 1.3 Heartbeat rules (both directions)

| Direction | Frame | When | Effect | Ref |
|---|---|---|---|---|
| S→C | `{"op":"h"}` | every 15 s | client **must reply** `{"op":"h"}` immediately | `client.ts:471-473` |
| C→S | `{"op":"h"}` | every 15 s on its own timer, plus each reply | liveness only: refreshes presence TTL, does **not** reset idle, does **not** renew the session, never echoed by the server | `conn.rs:119-121,339-344` |
| any S→C frame | — | — | resets the client watchdog (`lastServerAt`) | `client.ts:431` |
| WS Ping from client | — | optional | server answers Pong, counts as liveness | `conn.rs:150-155` |

Native client: replicate exactly — own 15 s `h` timer, reply `h` to every server `h`, 30 s receive watchdog checked every 1 s.

### 1.4 Reconnect and backoff

`web/src/ws/client.ts:614-634`:

```
attempt 0 (first drop):    delay = 0 ms (next tick)
attempt k >= 1:            base  = min(2000, 200 * 2^(k-1))
                           delay = round(base * (0.5 + random[0,1) * 0.5))
retryAttempt resets to 0 on every successful `open`.
```

| attempt | base ms | delay range ms |
|---|---|---|
| 0 | — | 0 |
| 1 | 200 | 100–200 |
| 2 | 400 | 200–400 |
| 3 | 800 | 400–800 |
| 4 | 1600 | 800–1600 |
| ≥5 | 2000 | 1000–2000 |

Retries are unbounded while signed in. Status values exposed to UI:
`"idle" | "connecting" | "open" | "reconnecting"` (`client.ts:22`). Exact transitions:

| Moment | Status | Ref |
|---|---|---|
| `start()` → first `open()` (`retryAttempt == 0`) | `connecting` | `client.ts:388-391` |
| socket `open` event | `open` | `client.ts:407` |
| socket `close` (incl. watchdog close) → `scheduleReconnect()` | `reconnecting` **immediately**, even on the first drop (delay 0) | `client.ts:614-616` |
| every reconnect `open()` after a drop (`retryAttempt ≥ 1` by then) | `reconnecting` | `client.ts:391` |
| `stop()` / `resetSession()` | `idle` | `client.ts:340` |

So `connecting` is only ever seen for the very first connect after sign-in;
the connection notice (§6.6) reacts to `reconnecting`.

On every (re)open (`client.ts:400-422`), in this order:
1. `retryAttempt = 0`, `lastServerAt = now`, status `open`, start heartbeat + watchdog.
2. For every desired topic: send subscribe frame **with resume cursor** (`n`, `ep`) if one is known (§2.3).
3. If the page/app is "in front": send `{"op":"f","on":true}`.
4. Fire `ready` listeners (realtime recovery §6.4, voice re-join §7.4).

Frames sent while no socket is open are dropped silently (`client.ts:284-290`);
everything needed is re-sent on open.

### 1.5 Server-side close reasons (no close frame payload, no error frame unless noted)

`backend/api/src/gateway/conn.rs:87-222`: session revoked/expired (v0.10: also when
an operator disables the account, after which the reconnect gets 401); session lock or
ACL recheck failed; 30 s silence; outbound queue overflow; client Close; send
timeout (1 s per write). The server sends `Message::Close(None)`. The client
treats every close identically → reconnect (§1.4). A 401 on reconnect (logged
out elsewhere) is not special-cased by the gateway; REST 401s drive logout.

---

## 2. Gateway — client → server frames (`ClientFrame`)

Source: `backend/api/src/gateway/protocol.rs:369-419` (serde `tag = "op"`).
Web mirror: `web/src/ws/protocol.ts:25-44`. Unknown `op` or bad JSON →
`{"op":"err","e":"bad_request"}` (socket stays) — `conn.rs:330-336`.

| op | JSON (exact) | Fields | Effect class | Ref |
|---|---|---|---|---|
| `h` | `{"op":"h"}` | — | Liveness | `protocol.rs:374-375` |
| `s` | `{"op":"s","s":"<uuid>"}`<br>`{"op":"s","s":"<uuid>","c":"<uuid>"}`<br>`{"op":"s","s":"<uuid>","c":"<uuid>","n":12,"ep":"<uuid>"}` | `s` server id (for a DM: the DM channel id); `c` channel id (optional, absent = server topic); `n` last applied seq (optional, u64); `ep` epoch of that seq (optional) | Activity | `protocol.rs:376-385`, `conn.rs:345-352,373-466` |
| `u` | `{"op":"u","s":"<uuid>"}` / `{"op":"u","s":"<uuid>","c":"<uuid>"}` | same topic addressing; no reply | Activity | `protocol.rs:386-391` |
| `sig` | `{"op":"sig","t":"j","s":"<srv>","c":"<voiceChan>"}`<br>`{"op":"sig","t":"l","s":…,"c":…}`<br>`{"op":"sig","t":"p","s":…,"c":…,"k":"a"}`<br>`{"op":"sig","t":"u","s":…,"c":…,"k":"s"}`<br>`{"op":"sig","t":"m","s":…,"c":…,"on":true}`<br>`{"op":"sig","t":"d","s":…,"c":…,"on":false}` | `t` ∈ `j l p u m d` (`r` is server-only → `bad_request`); `k` ∈ `a v s sa` required for `p`/`u`; `on` required for `m`/`d`. `s`,`c`,`t` missing → `bad_request` | Activity | `protocol.rs:392-406`, `signal.rs:34-71` |
| `f` | `{"op":"f","on":true}` | page/app in front (focused+visible+not idle) → Web Push held back for the account. `on:true` **also counts as activity** (resets the idle timer, slides the session); `on:false` does not, and does not flip the socket to `i` | Focus | `protocol.rs:407-410`, `conn.rs:134-143` |
| `p` | `{"op":"p"}` / `{"op":"p","st":"o"}` / `{"op":"p","st":"i"}` | absent `st` or `"o"` = active; `"i"` = this socket goes idle | Activity / Idle | `protocol.rs:411-416`, `conn.rs:362-365` |
| `y` | `{"op":"y","s":"<srv-or-dm>","c":"<chan>","on":true}` | typing start/stop; for DMs `s == c == dmId` | Activity | `protocol.rs:417-418`, `conn.rs:366-369,468-510` |

"Activity" frames reset the server idle timer, flip `i`→`o` and slide the
session expiry (throttled) — `conn.rs:122-129,165-168,247-273`. A heartbeat does none of that.

### 2.1 Topics

| Topic | Addressing | Receives | Ref |
|---|---|---|---|
| Server topic | `{s: serverId}` | `e` events with no `c` (kick/ban), `gap` for that topic | `protocol.rs:14-18,61-66` |
| Channel topic | `{s: serverId, c: channelId}` (text **or voice** channel) | `e` message create/edit/delete for that channel, `y` typing for that channel | same |
| DM topic | `{s: dmId, c: dmId}` (s == c) | as a channel topic | `conn.rs:383-385,512-551`, `web/src/ws/useGateway.ts:51-65` |

Topic key used by the client for maps: `"c:<channelId>"` if `c` present, else
`"s:<serverId>"` (`web/src/ws/protocol.ts:131-133`).

Subscribing any topic of server S (server or channel topic) also makes the
socket a member of S's **live** audience: `p` presence frames for S and
`sig` frames for S (`hub.rs:196-201,659-680`). A DM subscribe registers the DM id
the same way (so a presence snapshot keyed by the DM id arrives; ignore or store it).

### 2.2 Subscribe reply sequence

`backend/api/src/gateway/conn.rs:373-466`. For one `{"op":"s",…}` the server sends, in order:

| # | Frame | Condition |
|---|---|---|
| 0 | `{"op":"err","e":"not_found","s":S,"c":C}` and **stop** | server gone / not a member / channel not in server / channel limited to roles the user does not hold / not a DM participant (unknown, foreign and hidden are indistinguishable) |
| 0' | `{"op":"err","e":"server_blocked","s":S,"c":C}` and **stop** | v0.10: the caller is a member, but an operator blocked S (REST answers `403 server_blocked`) |
| 1 | zero or more `{"op":"e",…}` | resume replay: contiguous events `n+1 ..= head` |
| 1' | `{"op":"gap","s":S,"c":C,"ep":E}` | resume impossible (see §2.3) — instead of 1 |
| 2 | `{"op":"ok","s":S,"c":C,"n":HEAD,"ep":E}` | always (`c` omitted for server topic) |
| 3 | buffered live `e` frames that arrived during catch-up with `n > HEAD` (dups dropped); or `{"op":"resync"}` if any of them carries another epoch | |
| 4 | `{"op":"p","s":S,"snap":[{"u":U,"st":"o"},…]}` | always — presence snapshot of S |
| 5 | `{"op":"sig","t":"r","s":S,"snap":[{"u":U,"c":C,"m":true,"s":true},…]}` | always — voice occupancy snapshot of S, without seats in channels limited to roles the user does not hold |

Note: subscribing both `{s}` and `{s,c}` of the same server yields steps 4–5 twice; that is expected.

For a DM subscribe (`s == c == dmId`) steps 4–5 are still sent, keyed by the
DM id: `{"op":"p","s":dmId,"snap":[…]}` and `{"op":"sig","t":"r","s":dmId,"snap":[]}`
(`conn.rs:418,460-464`; `mark_dm` instead of `bind_server`).

If the subscribe fails with a server error (Redis/DB, not an access denial),
the server drops the topic again and answers `{"op":"err","e":"internal"}`
**without** `s`/`c` (`conn.rs:144-147,345-352`). The web client ignores it; the
topic stays in the desired set and is re-sent on the next reconnect/resync.
Every server frame is its own WebSocket text message; batches are written as
consecutive frames, never as a JSON array (`conn.rs:695-713`).

### 2.3 Sequence numbers, epochs, resume

- Every topic has its own monotonically increasing `n` (u64, starts at 1; head 0 = nothing published yet) and an **epoch** `ep` (UUID, created when the Redis key is first used; a Redis flush/restart makes a new epoch) — `event_log.rs:14-42`.
- Fresh subscribe (no `n`): no history replay (`plan_catch_up(None)` → nothing); REST owns history. Reply is just `ok` with the head — `protocol.rs:708-719`.
- Resume (`n` given): `protocol.rs:711-745`, `event_log.rs:160-193`:

| Condition | Server sends |
|---|---|
| `ep` given and ≠ current epoch | `gap` (with new `ep`) then `ok` |
| `n > head` | `gap` then `ok` |
| `n == head` | `ok` only |
| buffer holds exactly `n+1..=head` | those `e` frames (ascending) then `ok` |
| any seq missing from the buffer | `gap` then `ok` |

- Live delivery: if a live event's `ep` differs from the epoch the socket last saw for that topic, the server first sends `{"op":"gap","s","c","ep":<new>}` then the event — `hub/fanout.rs:238-246`.
- Redis Pub/Sub reconnect on the server → `{"op":"resync"}` to every socket (catch-up and epochs cleared) — `hub/fanout.rs:54-62`.

Client cursor rules (`web/src/ws/client.ts:467-597`, `protocol.ts:169-196`):

| Frame | Client action |
|---|---|
| `ok` | if `ep` differs from known cursor epoch → mark gap (new epoch). If a gap is pending: gap.ep = ep, gap.cursor = max(gap.cursor, n), headKnown = true, try finish gap. Else set cursor `{ep, n}` if no cursor, or `n` larger, or cursor had no `ep`. |
| `e` | if `ep` differs → mark gap (headKnown=true). Accept iff `n > cursor` (`nextCursor`); after an epoch change with unknown gap cursor, accept and take `n`. Update cursor (or gap cursor). Dispatch to event listeners **only if accepted** (dedupe). |
| `gap` | mark gap for topic (ep = frame.ep or old), and set gap.cursor = undefined (head may be below cursor). Triggers REST recovery (§6.4). |
| `resync` | `resyncVersion++`; for every desired topic: mark gap and **re-send subscribe** with current cursor; fire resync listeners (forces DM/server list rediscovery). |

A gap is "finished" (cursor replaced by gap cursor+epoch) only after REST
recovery succeeded **and** the head is known (`completeGap` + `finishGap`,
`client.ts:134-162`). Until then the old cursor stays.

`forgetTopic(key)` drops the topic from the desired set (sends `u`) and deletes its cursor/gap (`client.ts:185-191`).
`resetSession()` (logout/account switch) clears desired topics, cursors, gaps, DM discoveries and stops the socket (`client.ts:343-355`).

### 2.4 `setTopics` diff

`web/src/ws/client.ts:357-386`: replace the desired set; if a socket exists,
send `u` for each removed key and `s` (with resume cursor) for each added key.
Unchanged keys send nothing. Duplicate keys collapse (first wins).

---

## 3. Gateway — server → client frames (`ServerFrame`)

Source: `backend/api/src/gateway/protocol.rs:427-520`; web decode accepts exactly
these ops and drops anything else (`web/src/ws/protocol.ts:139-167`).

| op | Exact JSON shape (omitted keys shown in [brackets]) | Notes | Ref |
|---|---|---|---|
| `h` | `{"op":"h"}` | heartbeat, reply `h` | `protocol.rs:434-435,858-861` |
| `ok` | `{"op":"ok","s":"<uuid>",["c":"<uuid>",]"n":4,["ep":"<uuid>"]}` | subscribe ack with head seq. `ep` is always present from the subscribe path | `protocol.rs:436-444,550-557` |
| `e` | `{"op":"e","t":"c","s":"<uuid>",["c":"<uuid>",]"n":7,["ep":"<uuid>",]["i":"<uuid>",]["d":{…},]["r":123]}` | sequenced event, see §4. Key order on the wire: `op,t,s,c,n,ep,i,d,r` | `protocol.rs:445-460,537-548` |
| `gap` | `{"op":"gap","s":"<uuid>",["ep":"<uuid>",]["c":"<uuid>"]}` | resume impossible / epoch changed → refetch via REST | `protocol.rs:461-468` |
| `resync` | `{"op":"resync"}` | resubscribe everything, rediscover lists | `protocol.rs:430-431` |
| `dm` | `{"op":"dm","c":"<dmChannelId>"}` | a DM involving this account appeared (new DM / first message); verify via REST then subscribe | `protocol.rs:432-433`, `delivery.rs:193-204` |
| `sig` | `{"op":"sig","t":"j","s":"<srv>",["c":"<chan>",]["u":"<user>",]["k":"a",]["on":true,]["m":false,]["d":false,]["snap":[…]]}` | voice presence, see §5 | `protocol.rs:469-489` |
| `err` | `{"op":"err","e":"not_found",["s":"<uuid>",]["c":"<uuid>"]}` | see §3.1 | `protocol.rs:490-497` |
| `p` | update `{"op":"p","s":"<srv>","u":"<user>","st":"o"}`; snapshot `{"op":"p","s":"<srv>","snap":[{"u":"<user>","st":"i"},…]}` | `st` ∈ `o` online, `i` idle, `x` offline | `protocol.rs:96-129,498-508,829-844` |
| `y` | `{"op":"y","s":"<srv>","c":"<chan>","u":"<user>","on":true}` | typing; delivered only to sockets subscribed to channel topic `c` | `protocol.rs:509-511`, `hub/fanout.rs:206-221` |
| `ns` | `{"op":"ns"}` | notification settings changed on some device → refetch them | `protocol.rs:512-515` |
| `rd` | `{"op":"rd","d":[ReadSync,…]}` | read positions moved on another device | `protocol.rs:516-534` |
| `lc` | `{"op":"lc","l":"en"}` | the account's language changed (`PUT /api/me/locale`) → switch to `l` (`de` \| `en`) and keep it as the device's cached language. Servers older than v0.10 never send it; older clients ignore it | `protocol.rs` (`ServerFrame::Locale`, `UserFrame::Locale`), `profile.rs` (`put_locale`) |
| `jn` | `{"op":"jn","s":"<srv>"}` | v0.10: this account is now a member of server `s`, because its request to join was accepted (protocol-rest-auth §4.7) → refetch the server list and show "Du bist jetzt Mitglied von {name}." | `protocol.rs` (`ServerFrame::Joined`, `UserFrame::Joined`), `directory/requests.rs` (`accept`) |
| `jq` | `{"op":"jq","s":"<srv>","n":3}` | v0.10: server `s` now has `n` pending requests to join. Only to the owner and the members holding `moderate_members`, after a new request, a withdrawal, an accept or a decline → set the queue's count badge and refetch the queue if it is open | `protocol.rs` (`ServerFrame::JoinRequests`, `UserFrame::JoinRequests`), `directory/requests.rs` (`publish_queue`) |

`ns`, `rd`, `lc`, `jn`, `jq`, `dm`, `resync` go to every socket of the account regardless of subscriptions (`hub.rs:659-665`, `hub/fanout.rs:123-161`). `jn` and `jq` are best effort: they are not replayed, `n` is read after the change commits (two quick changes can both carry the final count), and a client that missed one catches up on its next REST read. Servers older than v0.10 never send them.

### 3.1 Error codes on `/ws`

| `e` | `s`/`c` present | Cause | Client reaction (web) | Ref |
|---|---|---|---|---|
| `bad_request` | none, or s+c for sig | unparseable frame, >16 KiB, sig missing fields/`t:"r"`, sig `p/u/m/d` without a seat in that room, `sa` without `s` published, non-voice channel join | if it matches a pending voice join → abort join (§7.3) | `conn.rs:115,333`, `signal.rs:45-47,121-128,188-196,217-221` |
| `internal` | none | server error handling a frame | ignore | `conn.rs:144-147` |
| `not_found` | s, c? | subscribe/typing/sig on unknown/foreign server/channel; also sent when a membership recheck drops access to server `s` | pending voice join → abort | `conn.rs:388-408`, `hub.rs:772` |
| `forbidden` | s, c | sig join without `join_voice` permission; sig `p` with `k:"s"`/`"sa"` without `go_live` | join abort, or Live refused (§7.3) | `signal.rs:198-232` |
| `kicked` | s | this account was kicked from server `s` | leave server (§6.3) | `hub.rs:475-515`, `moderation.rs:83` |
| `banned` | s | banned from server `s` | leave server (§6.3) | `moderation.rs:120` |
| `server_blocked` | s, c? | v0.10: an operator blocked server `s`. Sent to every member's sockets when the block happens (with `s` only; voice seats in `s` are ended), and in reply to a subscribe, typing or sig on `s` while it stays blocked (with `c` when the frame named one) | leave voice on `s`, toast, leave the view like `kicked`, but keep the server in the list as blocked (§6.3) | `operator/servers.rs` (`block`), `conn.rs` (`subscribe`, typing), `signal.rs` (`reject`), `hub.rs` (`reconcile_access`) |

A block that another API process made reaches this one's sockets through the access recheck, which then sends `server_blocked` instead of `not_found`. Unblocking sends nothing; the server's `blocked` flag is `false` again on the next `GET /api/servers`, and subscribing works again.

Leaving a server voluntarily also sends `err` `not_found` with `s` (`servers/mod.rs:407`); deleting a server sends `err` `not_found` with `s` to every member's sockets (`servers/mod.rs:368`). The reason reaches the socket via `revoke_server` → `hub.rs:515` (`{"op":"err","e":<reason>,"s":S}`, never with `c`).

### 3.2 `ReadSync` (element of `rd.d`)

`backend/api/src/gateway/protocol.rs:522-534`; web type `web/src/ws/protocol.ts:105-116`.

| key | type | notes |
|---|---|---|
| `channel_id` | uuid | text channel or DM id |
| `server_id` | uuid \| `null` | `null` for DMs |
| `read_message_id` | uuid \| `null` | |
| `read_at` | RFC 3339 \| `null` | |
| `read_order` | i64 \| `null` | `created_order` of the read message |
| `membership_at` | RFC 3339 | always present from the current server |
| `latest_order` | i64 \| `null` | |
| `unread_count` | i64 | |
| `mention_count` | i64 | |

Apply forward-only (`web/src/messages/readState.ts:280-330`): compare by
`membership_at` (newer wins), then `read_order`, then `latest_order`; for rows
without orders compare `read_at`. Never move a cached read position backwards.

---

## 4. Sequenced events (`op:"e"`)

### 4.1 Envelope

| key | type | meaning |
|---|---|---|
| `t` | `"c"` \| `"e"` \| `"d"` | create / edit / delete (`protocol.rs:131-150`) |
| `s` | uuid | server id; for DM events the DM channel id (`messages/mod.rs:735-740`) |
| `c` | uuid, optional | channel id; **absent** = server-level event |
| `n` | u64 | per-topic sequence |
| `ep` | uuid | topic epoch |
| `i` | uuid, optional | entity id (message id; for server-level: user id) |
| `d` | object, optional | payload |
| `r` | i64, optional | durable DB revision (`gateway_revision_seq`); present on message events |

### 4.2 Event kinds actually published

| Trigger (REST) | Topic | `t` | `i` | `d` | `r` | Ref |
|---|---|---|---|---|---|---|
| message sent (incl. system message, GIF) | channel | `c` | message id | full `Message` (4.3), incl. `nonce` of the sender's attempt | = `d.revision` | `messages/mod.rs:358-361,564-589` |
| message edited | channel | `e` | message id | full `Message` | yes | `messages/mod.rs:488-491` |
| reaction added/removed | channel | `e` | message id | full `Message` with new `reactions` | yes | `messages/reactions.rs:145-148` |
| link previews resolved after send (background job) | channel | `e` | message id | full `Message` with `previews` | yes | `messages/mod.rs:720` |
| previews removed by the author (`DELETE /api/messages/{id}/previews`) | channel | `e` | message id | full `Message` without `previews` (key omitted) | yes | `messages/mod.rs:641-659` |
| message deleted | channel | `d` | message id | omitted | yes | `messages/mod.rs:542-556` |
| member kicked | server | `d` | kicked user id | `{"k":"k"}` | omitted | `servers/moderation.rs:256-276` |
| member banned | server | `d` | banned user id | `{"k":"b"}` | omitted | same |
| access changed: roles, role assignments, `member_permissions`, a channel's `role_ids` | server | `e` | omitted | `{"k":"a"}` | omitted | `servers/access.rs` |

There are **no** gateway events for channel create/rename/delete, server
rename, member changes other than kick, ban and access (`{"k":"a"}`: refetch
the server detail and the server list), profile changes, or member join. Those surfaces refresh via REST refetch on reconnect/gap (§6.4) or the
user's own mutations. (Gap: a native client cannot see another user's
channel/server edits live; the web client cannot either.)

Delivery is via a transactional outbox (`gateway/delivery.rs`), serialized per
channel, retried every 1 s on Redis failure; a duplicate revision is never
republished (`event_log.rs:24-27`). Events may arrive before or after the
REST response of the same write.

### 4.3 `Message` payload (`d` of `c`/`e`)

Rust: `backend/api/src/messages/mod.rs:64-108`; TS: `web/src/messages/types.ts:5-200`.

| key | type | notes |
|---|---|---|
| `id` | uuid | |
| `channel_id` | uuid | |
| `author` | `{id: uuid, name: string, avatar_url: string\|null}` | |
| `content` | string | empty for GIF/system messages |
| `created_at` | RFC 3339 (µs precision) | |
| `edited_at` | RFC 3339 \| `null` | |
| `revision` | i64 | |
| `created_order` | i64 | immutable insert order |
| `attachments` | `[{id, filename, content_type, size}]` | |
| `reactions` | `[{emoji: string, user_ids: [uuid]}]` | |
| `gif` | omitted or `{provider, slug, title, width, height, url, still: string\|null}` | |
| `mentions` | `[uuid]` | |
| `mention_everyone` | bool | |
| `reply` | omitted or `{id, deleted, [author:{id,name}], [excerpt], [has_attachment:true], [has_gif:true], [system]}`; deleted original = `{id, deleted:true}` | |
| `previews` | omitted when empty; `[{url, title?, description?, site_name?, image?:{url,width,height}, youtube_id?}]` | image url always `/api/previews/…` |
| `system` | omitted or `{kind: string, params: object}` (e.g. `kind:"live_started"`) | |
| `nonce` | omitted or uuid | only on create response + its event: the sender swaps its pending `tmp:<nonce>` row |

Client hardening (keep it): the web normalises lists defensively —
`attachments`, `reactions`, `previews` that are not arrays become `[]`
("cjson turns `[]` into `{}`", `web/src/messages/types.ts:144-147`), and if
`r` is present it overrides `d.revision` (`web/src/messages/queries.ts:984-988`).

### 4.4 Applying events (web semantics to copy)

`web/src/ws/useRealtime.ts:330-358`, `web/src/messages/queries.ts:859-1008`, `web/src/messages/pages.ts:36-133`:

| Event | Action |
|---|---|
| `e` with `c`, `t:"d"`, `i` | remove row `i` from the channel's cached pages; update reply previews in other rows that quote it to `{id, deleted:true}` |
| `e` with `c`, `t:"c"` | if row exists: keep the newer (`newerMessage`); else insert into the page whose start ≤ message (chronological by `created_at` incl. sub-ms fraction, then `id`), or the newest page — **unless** that newest-loaded page `has_more` newer (then skip; paging will fetch). Then `confirmEcho` swaps a pending row with `nonce`. |
| `e` with `c`, `t:"e"` | replace row with `newerMessage(old,new)`; refresh reply previews quoting it |
| `newerMessage` | higher `revision` wins; if equal/absent, later `edited_at ?? created_at` wins |
| `e` without `c`, `t:"d"`, `i` == me | server removed me: `d.k == "b"` → banned else kicked → §6.3 |
| `e` without `c`, `t:"d"`, `i` ≠ me | drop member `i` from that server's member list |

---

## 5. Voice presence (`op:"sig"`)

### 5.1 Kinds

`backend/api/src/gateway/protocol.rs:152-236`:

| `t` | meaning | C→S fields | S→C fields |
|---|---|---|---|
| `j` | join voice channel | `s, c` | `s, c, u, m?, d?` |
| `l` | leave | `s, c` | `s, c, u` |
| `p` | publish track | `s, c, k` | `s, c, u, k` |
| `u` | unpublish track | `s, c, k` | `s, c, u, k` |
| `m` | mute flag | `s, c, on` | `s, c, u, on` |
| `d` | deafen flag | `s, c, on` | `s, c, u, on` |
| `r` | occupancy snapshot (server only; seats in channels hidden from the user are left out) | — (→ `bad_request`) | `s, snap` |

Track kinds `k` (`protocol.rs:187-222`): `a` microphone, `v` camera, `s` screen
(= "Live"; several per channel), `sa` screen audio (requires `s` published by
the same seat; unpublishing `s` also removes `sa`).

`VoiceEntry` (snapshot row, `protocol.rs:224-236`): `{"u":"<user>","c":"<chan>"[,"m":true][,"d":true][,"s":true]}` — `m/d/s` omitted when false; `s` here means "is live (publishes a screen)", not the server.

### 5.2 Server behaviour

| Client sends | Checks | Server emits | Ref |
|---|---|---|---|
| `j` | member of `s` with `JoinVoice`; `c` is a voice channel of `s` | (a) to **this socket only**, before anything else: for each other occupant `{"op":"sig","t":"j","s","c","u":<other>[,"m":true][,"d":true]}` followed by one `{"op":"sig","t":"p",…,"k":<kind>}` per published kind; (b) to everyone in the server audience **and** this socket: `{"op":"sig","t":"j","s","c","u":<me>,"m":<bool>,"d":<bool>}` (explicit booleans; aggregated over all of the user's seats in that channel) | `signal.rs:73-92`, `leases.rs:216-294`, `leases.rs:64-98` |
| `j` while seated elsewhere on the same socket | — | old seat is dropped first (other channel gets `l` or flag/pub deltas) | `leases.rs:224-240` |
| `l` | still `JoinVoice` member and `c` still a voice channel of `s` (else `forbidden`/`not_found`/`bad_request` with `s`,`c`); seat exists in that room on this socket, else `bad_request` | `l` if the user has no seat left in `c`; otherwise `m`, `d` and `u` deltas for kinds no other seat still publishes | `signal.rs:94-104`, `leases.rs:83-86` |
| `p` | seat in room; `s`/`sa` need `go_live` (`forbidden`); `sa` needs `s` (`bad_request`) | `p` (only if now published) | `signal.rs:131-162`, `leases.rs:403-464` |
| `u` | seat in room | `u` for each kind no longer published by any of the user's seats (`s` also removes `sa`) | same |
| `m on` | seat in room | `m` with aggregated `on` (AND over the user's seats) | `leases.rs:338-401` |
| `d on=true` | seat in room | `d` then `m` (deafen implies mute) | same, `leases.rs:87-88` |
| `d on=false` | seat in room | `d` only (mute unchanged) | same |

Audience for broadcast `sig`: sockets that subscribed any topic of server `s`, plus sockets holding a seat in `c` (`hub/fanout.rs:190-203`, `hub.rs:668-677`).

**Wire quirk (verified):** the per-occupant replay frames sent to a joiner are
built with `replay = true` in `SigEvent`, but `ServerFrame::sig` drops that
field (`protocol.rs:583-595` has no `replay`), so **`replay` never appears on the
wire**. The web type has `replay?` (`web/src/ws/protocol.ts:69-70`) and checks it
(`web/src/voice/session.ts:404-405`) but it is always `undefined`. A native client
must not depend on it; treat sig `j` frames that arrive between sending `j` and
receiving your own `j` as existing occupants.

Seats are 5 s leases renewed by the socket's 1 s security tick (`conn.rs:275-320`).
If the gateway socket dies the seat expires ≤ 5 s later and others see `l`
(or flag deltas). After reconnect the client must send `j` again (§7.4).

### 5.3 Client-side roster (server-wide occupancy)

`web/src/voice/roster.ts:218-268`, wired in `web/src/ws/useLive.ts:20-46` (file line numbers):

| Frame | Store update |
|---|---|
| `t:"r"` | replace `byServer[s]` with `{u → {channelId:c, muted:m===true, deafened:d===true}}`; replace `live[s]` with `{c → {u → {s:true}}}` for entries with `s:true` |
| `t:"j"` | `byServer[s][u] = {channelId:c, muted: m ?? prev ?? false, deafened: d ?? prev ?? false}` |
| `t:"l"` | if user is shown in another channel than `c`: only drop their live entry in `c`; else delete `byServer[s][u]` and their live entries in `c` and their old channel |
| `t:"m"` | patch `muted = on===true` (only if user present) |
| `t:"d"` | patch `deafened = on===true`; if `on` also `muted = true` |
| `t:"p"`, `k:"s"` | `live[s][c][u].s = true` (insertion order = "went live" order) |
| `t:"u"`, `k:"s"` | remove `live[s][c][u]` |
| other `k` on `p/u` | ignored by the roster (call participants handle them, §7) |

---

## 6. Presence, typing, focus, account frames

### 6.1 Presence

Server: socket status is `o` on connect/subscribe, `i` after 300 s without
activity or on `{"op":"p","st":"i"}`; aggregated per user across sockets;
`x` when the last socket goes (`live.rs:221-240,564-575`). Broadcast as
`{"op":"p","s","u","st"}` to each server the user is announced in. Snapshot
per subscribe (§2.2 step 4); entries with missing keys come back as `x`
(`live.rs:337-360,774`).

Client (`web/src/ws/live.ts:8-77`, `web/src/ws/useLive.ts:48-90`):

| Constant | Value |
|---|---|
| `CLIENT_IDLE_MS` | 300 000 (5 min) |
| `PRESENCE_PULSE_MS` | 10 000 |
| activity events | `pointerdown`, `keydown`, `mousemove` (not visibility) |

- On sign-in and on each activity event: if idle → send `{"op":"p","st":"o"}`; else send it at most every 10 s. Re-arm a 5 min timer that sends `{"op":"p","st":"i"}` once.
- Store `byServer[s][u] = st`; `x` deletes the key. Snapshot replaces the server's map (dropping `x`). Unknown user ⇒ `x`.
- Member list groups (`live.ts:157-177`): order `o` "Online", `i` "Abwesend", `x` "Offline"; each sorted by `name.localeCompare(b, "de")`; empty groups hidden.
- Wire form: `sendPresence("o")` sends `{"op":"p","st":"o"}`, `sendPresence("i")` sends `{"op":"p","st":"i"}` (`client.ts:292-294`); frames while the socket is closed are dropped.
- Reconnect quirk (copy it): nothing re-sends `p` on reconnect. A new socket becomes `o` on the server with its first subscribe (`announce_server` writes `Online`, `live.rs:287-293`), so a client that was idle when the socket dropped shows as `o` until its next own `i` (5 min later, or never if no input arrives: the idle timer already fired). The native client may re-send `{"op":"p","st":"i"}` after `open` while locally idle; the web does not.

### 6.2 Typing

Client (`web/src/ws/live.ts:20-23,79-150`, `web/src/ws/useLive.ts:135-183`):

| Constant | Value |
|---|---|
| `TYPING_TTL_MS` | 6 000 — remote typing entry expires `now + 6000` after the last `on:true` |
| `TYPING_REFRESH_MS` | 3 000 — while typing, re-send `on:true` every 3 s |
| prune tick | 400 ms (`useLive.ts:38`) |
| `TYPING_BAR_PX` | 28 — reserved strip under the message pane |

Send rules: on input change with non-empty text and not yet typing → send
`{"op":"y","s","c","on":true}` and start the 3 s refresh; empty text, message
sent, or channel change → `{"op":"y",…,"on":false}` once (to the channel it
was started in). Receive: `on:true` → `byChannel[c][u] = now+6000`; `on:false`
→ delete. Display excludes the own user.

Strings (`live.ts:145-150`), German verbatim:
- 1: `"{name} tippt…"`
- 2: `"{a} und {b} tippen…"`
- ≥3: `"{a}, {b} und {n-2} weitere tippen…"`
- unknown member id → name `"Jemand"` (`web/src/components/TypingBar.tsx:28`)
- 0 typers → empty string, bar keeps its 28 px height (no relayout).

Typing bar visual (`web/src/components/TypingBar.tsx:31-46`, `web/src/components/chat.css:1810-1852`):

| Property | Value |
|---|---|
| height | 28 px fixed (`TYPING_BAR_PX`, inline style), `flex-shrink: 0` |
| layout | flex row, `align-items: center`, gap 7 px |
| padding | `0 22px`; at viewport width ≤ 800 px `padding-inline: 18px` (1280 / 1600 px: 22 px) |
| text | 12 px, weight inherited (400), font `"Gelabber Inter"`, colour `--lr-muted` = dark `#a4abb1`, light `#5c646b`; single line, truncated with ellipsis (`truncate`) |
| dots | shown only when ≥1 typer; inline-flex, gap 3 px; three 4×4 px circles (`border-radius: 50%`), fill `currentColor` (= muted) |
| dot animation | `lr-typing` 1.2 s infinite ease-in-out; delays 0 / 0.15 s / 0.3 s; keyframes 0 %/60 %/100 % opacity 0.35, translateY 0; 30 % opacity 1, translateY −2 px |

### 6.3 Kicked / banned

`web/src/ws/useRealtime.ts:306-358`, `web/src/ws/leaveView.ts:1-8`. Triggered by
`err` `kicked`/`banned` with `s`, or a server-topic `e` `t:"d"` with `i == me`
(`d.k == "b"` ⇒ banned). Actions in order:
1. If in voice on that server: `leaveVoice()` (sends sig `l`).
2. Toast: banned → `"Du wurdest vom Server gesperrt."`; kicked → `"Du wurdest vom Server entfernt."`
3. If currently viewing that server: navigate to `/` (home) and forget the server's cached data; otherwise just forget it.

`err` `server_blocked` with `s` (v0.10) takes the same steps, except that the
server stays in the server list, marked `blocked: true` (REST `ServerView`),
and opens to the blocked notice instead of its channels.

### 6.4 Recovery after ready / gap / resync / dm

`web/src/ws/useRealtime.ts:31-291` (`attachRealtimeRecovery`):

| Trigger | Effect |
|---|---|
| socket `ready` (every open) | schedule a round with `force`; wait `random(0, READY_SPREAD_MS=1000)` ms first |
| `gap` | schedule a round (no spread wait if none running) |
| `resync` | mark "rediscover" + force round |
| `dm` (`c`) | add `c` to pending DMs, schedule round |

A round: snapshot pending gaps/DMs → cancel in-flight reads → re-fetch:
message histories **only if currently visible** (others stay invalidated and
refetch when opened), and all server/DM lists and details. Queries that fail
with `not_found`/`forbidden` are treated as removed (their topic is forgotten
via `forgetTopic("c:<id>")`). If rediscovering: fetch DM list and server list;
`addTopic({s:dm,c:dm})` for every DM; fetch first history for newly
announced/listed DMs. On success, `completeGap` for each gap in the snapshot
and `completeResync`. Bursts coalesce into one round; a reconnect during the
spread wait joins the waiting round.

Read state has its own recovery on ready (`messages/readState.ts:112-215`, `onReady` at :211):
refresh read state and invalidate contexts.

### 6.5 Focus (`f`)

`web/src/ws/useLive.ts:101-133`, `client.ts:296-301`: in front = page visible
AND not idle (5 min without input). Send `{"op":"f","on":<bool>}` only on
change; re-send `on:true` after each reconnect; send `on:false` on sign-out.
Native equivalent: window focused/visible and user active within 5 min.

### 6.6 Connection notice (UI behaviour driven by gateway status)

`web/src/ws/connectionNotice.ts:1-95`, `web/src/components/ConnectionNotice.tsx:11-61`:

| Constant | Value |
|---|---|
| `NOTICE_DELAY_MS` | 3000 — a dropped socket shows "reconnecting" only after 3 s |
| `RESTORED_MS` | 3000 — "restored" stays 3 s |

| State | Text (verbatim) | Icon |
|---|---|---|
| `offline` (OS reports offline; shown at once) | `Keine Verbindung – dein Gerät ist offline` | dot |
| `reconnecting` | `Keine Verbindung – wird wiederhergestellt…` | dot |
| `restored` | `Verbindung wiederhergestellt` | `CheckIcon` (from `components/Icons.tsx`) size 14 px |

Logic: status `idle` → hide. Offline → `offline`. `open` → if a trouble notice
was shown, show `restored` for 3 s, else nothing. `connecting`/`reconnecting`
while online → if already troubled show `reconnecting`, else (only for
`reconnecting`) start the 3 s delay, then show `reconnecting`. Starting that
delay also hides a still-visible `restored` pill at once
(`connectionNotice.ts:78-87`). `connecting` (first connect) never shows a
notice unless the device is offline. Inputs re-evaluated on every gateway
status change and on the OS `online`/`offline` events
(`ConnectionNotice.tsx:17-36`).

Pill visual (`web/src/components/connection.css:3-53`). Values for the
default themes "Gelabber Dunkel" / "Gelabber Hell" (`web/src/theme/defaults.json`,
`themeVariables()` in `web/src/theme/model.ts:295-399`, computed with
`fixContrast` + `derivedColors`):

| Property | Value |
|---|---|
| container | `position: fixed`, `top: 60px` (+ safe-area top, 0 on desktop), left/right inset 16 px, `z-index: 55`, flex, `justify-content: center`, `pointer-events: none`; it overlays the page (header is 52 px, so the pill starts 8 px below it); identical at 1280×800 and 1600×900 |
| pill layout | flex row, `align-items: center`, gap 8 px (`--lr-space-2`), `max-width: 100%`, `white-space: nowrap` (≤ 480 px viewport: `normal`) |
| padding | 6 px top/bottom, 14 px right, 12 px left |
| border | 1 px solid `--lr-border`: dark `#2c3032`, light `#dcd6cc` |
| radius | 999 px (full pill) |
| background | `--lr-elevated`: dark `#252829`, light `#fcfbf9` |
| text colour | `--lr-text`: dark `#eef0ee`, light `#252829`; `restored` uses `--lr-online-text`: dark `#85d69b`, light `#277643` (icon inherits it) |
| shadow | `--lr-shadow-1`: dark `0 8px 24px rgb(0 0 0 / 0.32)`, light `0 8px 24px rgb(60 45 20 / 0.12)` |
| font | `--lr-type-ui` = weight 500, 13 px, line-height 1.35 (17.55 px), `"Gelabber Inter"` (Inter 4.1 variable) |
| dot | 8×8 px circle, `flex-shrink: 0`, fill `--lr-warning`: dark `#d8a657`, light `#a27e44` |
| enter | from opacity 0, translate `0 -6px` to rest, 180 ms `cubic-bezier(0.2, 0.8, 0.2, 1)` (`--lr-dur`, `--lr-ease-enter`); 0 ms under reduced motion. Exit is instant (element removed). The `restored` pill re-enters (new React `key`), the offline→reconnecting swap does not |
| resulting height | 6 + 17.55 + 6 + 2 = ≈ 31.5 px |

---

## 7. Voice session: gateway + media together (web `voice/session.ts`)

### 7.1 Topics while in a call

`web/src/ws/useGateway.ts:87-99`: while `status == "joined"` the subscribe set
additionally contains `{s: voiceServerId}` and `{s: voiceServerId, c: voiceChannelId}`
so `sig` keeps arriving even outside that server's view.

### 7.2 Join (order of operations)

`web/src/voice/session.ts:1207-1278` then `startPeer` `session.ts:1538-1700`:

1. If already joined elsewhere: local roster vacate + send `{"op":"sig","t":"l","s":old,"c":old}`.
2. Tear down the old media peer. Local state → `status:"joined"`, `connection:"connecting"`, participants `{me:{pubs:[]}}`; mute/deafen carried over only if moving between channels while joined.
3. Optimistically put self into the roster (`applyVoiceJoin`).
4. Send `{"op":"sig","t":"j","s":S,"c":C}`; then, if muted, `{"op":"sig","t":"m","s":S,"c":C,"on":true}`; if deafened, `{"op":"sig","t":"d",…,"on":true}` ("The join frame has no mute fields").
5. `POST /api/channels/{C}/media-ticket` → §8.1.
6. Open media socket at `ws(s)://<host>` + `media_path`; send `{"op":"j","id":1,"tk":"<ticket>","v":4}`.
7. On result: `device.load(routerRtpCapabilities)` → `capabilities` → `transport{direction:"recv"}` → flush queued consumer events (§8.4).
8. `connection = "connected"`, `sourceWatchSupported = true`; re-send `w on:true` for each remembered Live subscription.
9. Microphone: capture, `produce {k:"a",…}` (send transport created lazily: `transport{direction:"send"}`, then `connect` from the SDK's connect event), then gateway `{"op":"sig","t":"p","s":S,"c":C,"k":"a"}`.

Own `j` echo: when the gateway broadcast `j` for my own user id arrives and a
join is pending for that channel → join confirmed: play `joinSelf` sound if
entering a new channel; if this was a re-join after reconnect with live media,
re-announce pubs (§7.4).

### 7.3 Gateway error handling during voice

`web/src/voice/session.ts:497-536`:

| Condition | Action | Message (verbatim) |
|---|---|---|
| pending join and `err` with matching `s`,`c` and `e` ∈ `forbidden`/`not_found`/`bad_request` | stop media, reset voice state to idle, show error | `errorMessage(code)`: forbidden → `Dafür fehlt dir die Berechtigung.`; not_found → `Das gibt es nicht (mehr).`; bad_request → `Da ist etwas schiefgelaufen. Bitte noch einmal versuchen.` (`web/src/auth/rules.ts:369-416`) |
| pending Live (`p k:"s"` sent) and `err forbidden` for that room | keep seat, stop screen capture | `In diesem Server darfst du nicht live gehen.` (`session.ts:142`) |

### 7.4 Gateway reconnect while in a call (`onReady`)

`web/src/voice/session.ts:538-572`: on every gateway `ready` while joined:
clear peer bookkeeping; participants reset to `{me}` (keep remote video
streams if the media socket is still open); re-occupy self in the roster;
mark join pending; send `sig j`, then `m on:true` / `d on:true` if set. If the
media socket is still open, the own `j` echo triggers `reannounceActive`
(`session.ts:1084-1096`): `p a` (if mic sender live), `p v` (camera), `p s` and `p sa` (Live with audio). If the media socket is gone → seat rebuild (§8.6).

### 7.5 In-call frames from the client

| Action | Gateway frames | Ref |
|---|---|---|
| Leave | `{"op":"sig","t":"l","s","c"}` (after local teardown; media `l` sent first) | `session.ts:1280-1308` |
| Toggle mute | `m` with new value; if it also changed deafen, `d` | `session.ts:1313-1339` |
| Toggle deafen | on: `m on:true` (if changed) + `d on:true`; off: `m on:<pre-deafen mute>` (if changed) + `d on:false` | `session.ts:1341-1366` |
| Camera on | media `produce k:"v"` → then `p k:"v"` | `localVideo.ts:543-575` |
| Live (screen) on | media `produce k:"s"` → `p k:"s"`; if audio: `produce k:"sa", parent:<screenProducerId>` → `p k:"sa"` | `localVideo.ts:575-606` |
| Camera/Live off | `u k` (and `u sa` for screen) **first**, then media `closeProducer` | `localVideo.ts:496-540` |
| Media attempt failed but seat kept | `u` for every kind announced in this attempt | `session.ts:1061-1079` |

### 7.6 Incoming `sig` inside the current room

`web/src/voice/session.ts:391-495` (only frames with `s,c` == current room and `u` present):

| `t` | Action |
|---|---|
| `j` (other) | add participant; play `joinOther` unless already a participant; remember `{since: now, muted: m===true}` |
| `j` (me, pending) | join confirmed (see 7.2) |
| `l` (other) | play `leaveOther` if they were a participant; remove participant (their media tiles stay until their tracks end) |
| `m` (other) | play `muteOther`/`unmuteOther` only if value changed and peer joined ≥ `PEER_SETTLE_MS = 2000` ms ago |
| `p`/`u` | update participant `pubs`; for remote `k:"s"`: `liveStart` on new Live (settled peers only), `liveEnd` when it ends; `u v`/`u s` → forget received video; `u sa` → drop that Live's audio |

---

## 8. Media socket `/media/ws`

### 8.1 Ticket (REST)

`POST /api/channels/{channelId}/media-ticket` — `backend/api/src/media.rs:33-90`.
No request body. Headers (REST CSRF rules, `backend/api/src/csrf.rs:1-60`):
`Cookie: gelabber_session=<tok>; gelabber_csrf=<csrf>` and `X-CSRF-Token: <csrf>`
(the header must equal the `gelabber_csrf` cookie; the value comes from the
`csrf_token` field of the JSON body of `GET /api/auth/session`, login,
register and logout, which rotate it — `backend/api/src/auth/mod.rs:6-7`).
Do not send `Sec-Fetch-Site: cross-site` (refused). Missing/mismatched → REST `csrf_invalid`.
Requires session; server row locked; caller member with `JoinVoice`; channel
kind voice (DM → 404; non-voice → 400 `"Media tickets are for voice channels."`).

Response 200:

```json
{
  "ticket": "abcdefghjkmn",
  "expires_in": 30,
  "media_path": "/media/ws",
  "ice_servers": [
    {"urls": ["stun:stun.example:3478"]},
    {"urls": ["turn:turn.example:3478?transport=udp"], "username": "…", "credential": "…"}
  ]
}
```

| Field | Rule | Ref |
|---|---|---|
| `ticket` | 12 chars from `abcdefghjkmnpqrstuvwxyz23456789` (no 0/o/1/l/i); single use (Redis `GETDEL`) | `backend/shared/src/ticket.rs:7-29`, `media/src/ticket.rs:38-57` |
| `expires_in` | seconds, default 30 | `config.rs:89` |
| `media_path` | always `"/media/ws"` | `media.rs:87` |
| `ice_servers[]` | `{urls:[string], username?, credential?}`; pass as WebRTC ICE servers | `backend/shared/src/ice.rs:16-22` |

The ticket also encodes `g` (may go live) — the SFU refuses `produce` of `s`/`sa` without it (`media/src/sfu/rpc.rs:296-300`).

### 8.2 Connection

| Item | Value | Ref |
|---|---|---|
| URL | `wss://<host>/media/ws` (scheme from app origin) | `web/src/voice/media.ts:191-194` |
| Auth | **no cookie, no Origin check**; first frame `j` with ticket | `media/src/ws.rs:1-7` |
| Protocol version | `v: 4` (`MEDIA_PROTOCOL_VERSION`) | `media/src/protocol.rs:6-7`, `web/src/voice/media.ts:11` |
| Upgrade refused | HTTP 429 (too many pending sockets per client IP, default 16) or 503 (total pending, default 1024), header `Retry-After: 1` | `ws.rs:50-65`, `media/src/config.rs:25-26` |
| Max frame | 256 KiB inbound (server); web drops inbound > 1 MiB | `ws.rs:31,66-68`, `media.ts:227` |
| Join deadline | 10 000 ms from connect (and after each `l`) to complete a join, else server closes | `config.rs:24`, `ws.rs:190-201,228-238` |
| Heartbeat | server sends WS **Ping** every 20 000 ms once joined; must answer **Pong** (auto in most WS libs) | `config.rs:27`, `ws.rs:205-214` |
| Idle timeout | closed if nothing (incl. Pong) received for 60 000 ms | `config.rs:28`, `ws.rs:208-212` |
| Binary frame | answered `{"op":"err","e":"bad_request"}` | `ws.rs:249` |
| Client pre-open queue | web queues up to 64 frames before `open`, else `request_overflow` | `media.ts:238-244` |

### 8.3 Request/response framing

Every client frame carries `"id"` (u32). Rules (`ws.rs:288-337`):
- `id` must be > 0 and strictly greater than the previous id on this socket, else `{"op":"err","id":<id>,"e":"bad_request"}`.
- Success: `{"op":"result","id":<id>,"data":<object>}`.
- Failure: `{"op":"err","id":<id>,"e":"<code>"}`; unsolicited/connection errors have no `id`.
- Unknown fields are rejected (`deny_unknown_fields`): the frame fails to decode → `err` (with `id` if one was readable) — `protocol.rs:61-62`, `ws.rs:128-138`.
- Before a successful `j`, any other op → `{"op":"err","id":<id>,"e":"unauthorized"}`, and because `unauthorized` is terminal the server **closes the socket** right after (`ws.rs:113-115,330-336`). Never send anything before the `j` result.
- `j` with `v ≠ 4` is answered before the id check: with `id > 0` → `{"op":"err","id":<id>,"e":"update_required"}`, with `id` 0/absent → `{"op":"err","e":"update_required"}`; terminal either way (`ws.rs:296-305`).
- Results and server events are **not ordered** relative to each other (`ws.rs:147-149`): a `consumer` event may overtake the `result` of the RPC that caused it.

Web client: ids start at 1 and increase; at most 64 outstanding; per-request
deadline `MEDIA_REQUEST_DEADLINE_MS = 10 000` → local `request_timeout`
(`web/src/voice/mediaPeer.ts:13-14,69-151`). On teardown it sends a final
`{"op":"l","id":<next>}` without awaiting (`mediaPeer.ts:187-224`).

### 8.4 Client → server ops (`ClientFrame`)

`backend/media/src/protocol.rs:61-164`; web request/result types `web/src/voice/media.ts:19-65`.

| op | Exact request | `data` of result | Server notes | Ref |
|---|---|---|---|---|
| `j` | `{"op":"j","id":1,"tk":"<ticket>","v":4}` | `{"c":"<chan>","u":"<user>","v":4,"generation":"<uuid>","routerRtpCapabilities":{…}}` | `v ≠ 4` → `update_required` (terminal) before the ticket is consumed; bad/used/expired ticket → `unauthorized` (terminal); second `j` → `bad_request` | `ws.rs:296-364`, `sfu/mod.rs:691-697` |
| `capabilities` | `{"op":"capabilities","id":2,"rtp":<RtpCapabilities>}` | `{"codecs":{"video":["video/vp8",…],"revision":3}}` | once per peer (second → `bad_request`); ≤64 codecs, ≤32 header exts; triggers consumer attach | `rpc.rs:34-53` |
| `transport` | `{"op":"transport","id":3,"direction":"recv"}` (or `"send"`) | `{"id":"<transportId>","iceParameters":{…},"iceCandidates":[…],"dtlsParameters":{…}}` | one per direction (second → `bad_request`); recv creation triggers attach | `rpc.rs:54-100` |
| `connect` | `{"op":"connect","id":4,"transportId":"…","dtls":<DtlsParameters>}` | `{}` | | `rpc.rs:101-117` |
| `restartIce` | `{"op":"restartIce","id":5,"transportId":"…"}` | `{"iceParameters":{…}}` | | `rpc.rs:118-128` |
| `closeTransport` | `{"op":"closeTransport","id":6,"transportId":"…"}` | `{}` | | `rpc.rs:129-132` |
| `produce` | `{"op":"produce","id":7,"k":"a","rtp":<RtpParameters>,"epoch":"<uuid>"[,"parent":"<screenProducerId>"][,"expectedOldProducerId":"…"][,"height":1080][,"paused":false]}` | `{"producerId":"…","epoch":"<uuid>"}` | `k` ∈ `a v s sa`; `epoch` non-nil (capture id); `s`/`sa` need ticket `g` → `forbidden`; `sa` needs `parent` = current `s` producer with same epoch; `expectedOldProducerId` must equal current producer of that kind (replace) else `forbidden`; created paused then resumed unless `paused:true` | `rpc.rs:133-153,281-470` |
| `pauseProducer` | `{"op":"pauseProducer","id":8,"producerId":"…"}` | `{}` | | `rpc.rs:154-159` |
| `resumeProducer` | `{"op":"resumeProducer","id":9,"producerId":"…"}` | `{}` | | `rpc.rs:160-181` |
| `closeProducer` | `{"op":"closeProducer","id":10,"producerId":"…"}` | `{}` | idempotent for already retired ids | `rpc.rs:182-196` |
| `consumerReady` | `{"op":"consumerReady","id":11,"consumerId":"…","generation":"<uuid>"}` | `{}` | resumes the consumer; server then emits `consumerState` | `rpc.rs:197-236` |
| `consumerFailed` | `{"op":"consumerFailed","id":12,"consumerId":"…","generation":"<uuid>"}` | `{}` | server removes it and retries attach within a failure budget per producer | `rpc.rs:237-267` |
| `w` | `{"op":"w","id":13,"u":"<ownerUserId>","k":"s","on":true}` | `{}` | watch someone's Live: screen + screen audio consumers are only created for watched owners; ≤64 watches; own id → `forbidden` | `rpc.rs:268-271`, `sfu/attach.rs:336-398`, `sfu/mod.rs:50` |
| `q` | `{"op":"q","id":14,"consumerId":"…","generation":"<uuid>","h":360,"congested":false}` | `{}` | accepted and ignored (legacy) — do not send | `rpc.rs:272-274` |
| `l` | `{"op":"l","id":15}` | `{}` | leave room; socket stays, new 10 s join deadline | `rpc.rs:275-278`, `ws.rs:318-321` |

Mic (`a`) and camera (`v`) are consumed automatically by every other peer; no watch needed.

### 8.5 Server → client events

`backend/media/src/protocol.rs:187-250`:

| op | Exact JSON | Client action (web `mediasoupConnection.ts:990-1200`) |
|---|---|---|
| `result` | `{"op":"result","id":7,"data":{…}}` | resolve pending request |
| `err` | `{"op":"err","id":7,"e":"forbidden"}` / `{"op":"err","e":"gone"}` | reject request; or room-level handling (§8.6) |
| `consumer` | `{"op":"consumer","consumerId":"…","producerId":"…","owner":"<uuid>","k":"v","epoch":"<uuid>","generation":"<uuid>"[,"parent":"…"],"kind":"video","rtpParameters":{…},"paused":false}` | validate (`kind` must be `audio` for `a`/`sa`, `video` for `v`/`s`; `sa` needs `parent`); queue until recv transport exists; `recvTransport.consume(...)`; pause locally if `paused`; then `consumerReady`; on failure `consumerFailed` |
| `consumerClosed` | `{"op":"consumerClosed","consumerId":"…","generation":"<uuid>"}` | remove consumer (if generation matches); drop queued events for it |
| `consumerState` | `{"op":"consumerState","consumerId":"…","generation":"<uuid>","paused":true}` | pause/resume local consumer |
| `producerClosed` | `{"op":"producerClosed","producerId":"…","epoch":"<uuid>"}` | close own producer with that id+epoch |
| `codecs` | `{"op":"codecs","video":["video/vp8","video/h264"],"revision":4}` | video codecs every receiver can decode; publisher must use one; higher `revision` wins (may overtake the `capabilities` result) |
| `layers` | `{"op":"layers","consumerId":"…","generation":"<uuid>","spatialLayer":2,"temporalLayer":null}` | diagnostics only |

Event queue cap in the web client: 3×1024 queued, 1024 pending consumes; overflow fails the recv side (`mediasoupConnection.ts:1046-1080`).

### 8.6 Media error codes and client reactions

Server codes (`backend/media/src/error.rs:32-43`, `ws.rs:113-115`):

| `e` | Meaning | Terminal (server closes socket) |
|---|---|---|
| `bad_request` | malformed / out-of-order id / protocol misuse | no |
| `forbidden` | not allowed (go-live bit, replace mismatch, watch self) | no |
| `unsupported_media_version` | | no |
| `unsupported_codec` | | no |
| `unavailable` | SFU worker/router unavailable, room full | no |
| `unauthorized` | ticket invalid, or access revoked mid-call | **yes** |
| `negotiation_failed` | mediasoup rejected parameters | no |
| `ice_failed` | | no |
| `update_required` | client protocol version ≠ 4 | **yes** |
| `gone` | SFU shutting down | **yes** |

Web reactions to unsolicited errors (`web/src/voice/session.ts:1149-1199`):

| Code | Action | Message (verbatim) |
|---|---|---|
| `ice_failed` | diagnostics only | — |
| `update_required` | leave call | `Bitte Gelabber aktualisieren und den Sprachkanal erneut betreten.` |
| `unauthorized` | leave call | `Bitte melde dich erneut an.` |
| `unavailable` | during rebuild: retry; else retract pubs, `connection:"failed"` | `Kein freier Sprachplatz.` |
| `forbidden` | toast | `Dafür fehlt dir die Berechtigung.` |
| other, before join accepted | leave call with `MediaError(code)` | — |
| other, after join | toast once per connection | `Der Stream konnte nicht verbunden werden. Der Sprachkanal bleibt aktiv.` (`sessionState.ts:433-441`) |

### 8.7 Media recovery constants

| Constant | Value | Ref |
|---|---|---|
| Seat rebuild backoff (`MediaRetry`) | attempt n (0-based): `min(250·2^n, 4000) × (0.8..1.2)` ms → 250, 500, 1000, 2000, 4000, 4000, 4000; after 7 attempts give up | `web/src/voice/mediaPeer.ts:16-43` |
| Give-up message | `Die Sprachverbindung konnte nicht wiederhergestellt werden. Bitte erneut beitreten.`; state `connection:"failed"`, `lostCall {serverId, channelId, channelName ?? "Sprachkanal"}` | `session.ts:886-908,1489-1513` |
| ICE `disconnected` | wait `ICE_DISCONNECTED_RESTART_DELAY_MS = 2000`, then `restartIce` | `session.ts:144,1099-1147` |
| ICE `failed` | `restartIce` immediately | same |
| ICE recovery deadline | `ICE_RECOVERY_DEADLINE_MS = 10 000` after restart → full seat rebuild | `session.ts:146` |
| Media socket close while joined | seat rebuild (new ticket, new socket, new `j`) | `session.ts:1560-1569` |
| Ticket errors retried during rebuild | `network`, `timeout`, `internal`, `rate_limited` (or non-API errors) | `session.ts:1531-1536` |

Voice connection states exposed to UI (`web/src/voice/voiceStore.ts:16-21`):
`status: "idle" | "joined"`, `connection: "idle" | "connecting" | "connected" | "reconnecting" | "failed"`.

---

## 9. Web startup order (to reproduce in native)

`web/src/components/AppShell.tsx:28-57`:

1. User known (session restored via REST `/api/auth/me` or login).
2. `useGatewaySession(userId)` → `gateway.start()` → open `/ws` (status `connecting`). On sign-out/user change: `resetSession()` (`web/src/ws/useGateway.ts:20-33`).
3. `useIdlePresence` → immediately `goActive` → `{"op":"p","st":"o"}` (dropped if the socket is not open yet; harmless) and arm the 5 min idle timer (`useLive.ts:48-90`).
4. `useFocusReport` → `setInFront(visible && !idle)`; remembered and re-sent on open.
5. `AuthenticatedRealtime`: `useAuthenticatedSubscriptions` computes topics = merge(view, notifications, media) and calls `setTopics` (`useGateway.ts:105-166`):
   - view: route server `{s}` + route channel `{s,c}`; or DM route `{s:dm,c:dm}`.
   - notifications: last-visited ("remembered") server `{s}` + every **text** channel `{s,c}` of it + every DM `{s:dm,c:dm}` from `GET` DM list.
   - media: voice server `{s}` + voice channel `{s,c}` while joined.
   - Mounts `useLiveBridge` (presence/typing/sig → stores, 400 ms typing prune) and `useRealtimeBridge` (events, kick/ban, recovery).
   - `useReadBridge` (`rd`) and `useNotificationSettingsBridge` (`ns`).
6. Socket opens → subscribes (§1.4) → per topic: replay/gap, `ok`, presence snapshot, voice snapshot → `ready` listeners → recovery round after 0–1000 ms random wait refetches visible histories and lists.
7. Voice module (`ensureBound`, `session.ts:194-215`) registers `onSig`, `onErr`, `onReady` once.

---

## 10. Gaps / caveats for implementers

- `replay` on sig `j` is never transmitted (§5.2). Distinguish existing occupants by timing (frames received before your own `j` echo).
- No live events for channel/server/role/profile changes; only kick/ban. Native must refetch on reconnect/gap like the web, or poll.
- Typing expiry is client-side only (6 s); the server never sends `on:false` for a vanished typer except on its socket's disconnect cleanup.
- Media `result` vs event ordering is not guaranteed; queue consumer events until the recv transport exists, and key consumers by `(consumerId, generation)`.
- The desktop app of that time (`desktop/app`, removed in 0.9) embedded the web UI via Tauri and a native libwebrtc media device (`web/src/voice/native/*`, `desktop/core`); a pure egui client needs its own mediasoup-compatible client (device.load, transports, produce/consume) — mediasoup-client semantics (RTP capabilities/parameters JSON) are what `/media/ws` expects.
- `Origin`: omit it from native upgrades (`/ws` passes without Origin; `/media/ws` never checks).
