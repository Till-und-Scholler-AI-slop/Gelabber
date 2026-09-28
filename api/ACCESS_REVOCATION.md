# 03a → 03b authorization contract

03a owns API/auth/server/gateway only. Shared/media are unchanged. This is an
additive ticket envelope; the old shared decoder ignores `auth`, so deploying
03a alone does **not** enforce the new media authorization boundary.

## Ticket payload

`gb:mt:{code}` remains a one-use, short-lived ticket. JSON contains existing
`u`, `s`, `c`, `g` plus:

```json
{"auth":{"session":"<64 lowercase hex SHA-256 characters>","expires_at":1234567890,"member":"<UUID nonce>","channel":"<UUID nonce>"}}
```

`auth.session` identifies the exact DB session, never the raw cookie. `expires_at`
is that session's absolute Unix expiration, rounded down to a whole second.
The HTTP `expires_in` is capped by remaining session lifetime.

## Redis keys

| Key | Value | Lifetime / writer |
| --- | --- | --- |
| `gb:auth:member:{s}:{u}` | UUID nonce | API initializes atomically with mint; rotates before membership removal, server deletion or loss of JoinVoice/GoLive commits |
| `gb:auth:channel:{c}` | UUID nonce | API initializes atomically with mint; rotates before channel deletion commits |
| `gb:auth:session:{hash}` | User UUID string | API sets/refreshes after checking the DB session under `FOR SHARE`; maximum TTL 3000 ms, capped at session expiration |
| `gb:auth:demand:{hash}` | `1` | API mint sets `EX 35`; **03b refreshes `SET 1 EX 35` once per second while an authenticated media peer remains active** |
| `gb:auth:owner:{hash}` | API worker UUID | API-internal, 5000 ms TTL; worker refresh/cleanup compares its generation |

Membership/channel nonces deliberately do not expire. A missing key is a denial,
not generation zero. A new mint after Redis reset creates a fresh nonce.

Media must not delete demand on one peer's leave: another tab or an outstanding
ticket may still need it. Without further mint or peer demand, the API worker
stops within roughly 35 seconds. A fresh mint starts maintenance again. Delayed
worker cleanup/retry deletes the session lease only if it still owns the same
worker UUID. Logout cleanup targets a permanently deleted session hash.

## Required media enforcement in 03b

1. Reject legacy/malformed envelopes. Atomically consume and validate the ticket
   against both nonces, exact session-key user UUID and Redis time `< expires_at`.
2. Validate again before SFU attach; consuming a ticket is not a lasting grant.
3. Every second throughout peer lifetime, atomically revalidate those conditions
   before refreshing demand. Missing/mismatched authority, expiration or Redis
   failure closes the media peer and its publications; do not fail open.
4. Refreshing demand does not itself renew a session lease. Only the API can do
   that after DB validation. API/DB failure therefore revokes within the 3-second
   authority TTL plus the media check interval; this bound needs media testing.
5. Replace the old `gb:deny:{s}:{u}` 120-second block with generation enforcement
   in the integrated change. API currently retains that block for the existing
   SFU. Immediate media rejoin/unban remains blocked until 03b switches over.

Leave/kick/ban/delete serialize against invite join, gateway room registration,
outbound sends and ticket mint using the server row. Session row locks serialize
socket actions/ticket mint against logout. All authorization queries within a
lock use the same DB connection (including the one-connection pool regression).

A Redis failure during member/channel invalidation rejects and rolls back that
REST mutation. A Redis failure on logout does **not** preserve the DB session:
logout commits the targeted DB delete, cancels local sockets, attempts bounded
Redis cleanup and retries; the session lease cannot continue renewing.

Gateway sockets independently recheck DB membership incarnation (`joined_at`)
and media permissions before handling/dequeuing frames and on a one-second
security timer. This covers another API instance and leave/rejoin between ticks.
Existing chat subscriptions survive media-only permission reductions. Grants do
not revoke existing voice or tickets. Queue capacity/voice crash leases remain
05a/05b work.

## API evidence

2026-09-28, Rust 1.98.1, Postgres 18.6 UTF8, Redis 8.10.1; locked dependencies.
`cargo test --locked -p gelabber-api --all-targets -- --test-threads=2` passed
190 tests (83 unit + 107 integration), including 16 new access regressions and
an owner-generation Redis test. The subsequent small gateway/deadline changes
were verified with targeted access/gateway/signal tests and API Clippy.

The controlled Ban/Join regression fails when the shared server lock is removed:
a BEFORE INSERT barrier lets ban commit between the ban check and member insert;
banned membership then coexists. With the fix it passes. Logs outside the repo:
`/tmp/03a-ban-race-before.log`, `/tmp/03a-api-all.log`,
`/tmp/03a-final-targeted.log`, `/tmp/03a-clippy.log`.

No SFU/media acceptance is claimed: old-ticket consumption, live peer teardown,
Watch-only demand, immediate rejoin, and decoded audiovisual behavior require
03b/integrated E2E tests. No push, PR, release or production action occurred.
