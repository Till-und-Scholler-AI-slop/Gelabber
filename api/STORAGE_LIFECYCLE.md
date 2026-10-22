# Storage lifecycle / quota contract (10)

## API and configuration

Presign/message/download wire shapes stay unchanged. A message carries up to
ten files (`ATTACHMENTS_MAX`), at most 25 MiB each and 100 MiB together
(`api/src/attachments/validate.rs`).
A pending attachment waits up to one hour for its message (`PENDING_TTL`,
`api/src/attachments/mod.rs`); its presigned PUT URL expires after ten minutes
(`PUT_TTL`, `api/src/storage.rs`). Expiry cleanup and the fifteen-minute grace
run from the pending deadline, not from the URL's.
Expired unbound attachments cannot be bound or downloaded, even before the
background worker removes metadata. Binding rechecks DB wall-clock expiry
after storage HEAD, so a slow HEAD cannot use the transaction's old `now()`
snapshot to attach an already expired upload. Bound attachments remain accessible under
normal channel/DM authorization.

`AppState::from_config` and `with_pool` require MinIO configuration. The only
memory-store opt-in is `API_ALLOW_MEMORY_STORE=true`, intended for tests or
explicitly chosen development. Missing configuration fails API startup visibly;
it no longer silently generates unusable port-1 upload URLs. Supplied MinIO
configuration always takes precedence. Temporary MinIO unavailability retains
retry work; presign/bind expose the existing internal-error response. HTTP
transport errors strip signed URLs before logging. No stack pins changed.

## Download links and access revocation (#201)

`GET /api/attachments/{id}` checks membership on every request it receives,
then redirects (`307`) to a presigned MinIO GET. So that a picture keeps one
URL, and the browser its cached copy, the link is signed as of the start of a
30-minute window (`GET_WINDOW`) and stays valid for 60 minutes (`GET_TTL`)
from that start. The redirect carries `Cache-Control: private, max-age=<rest
of the window>` and `Vary: Cookie`; MinIO answers the object with
`Cache-Control: private, max-age=1800`.

Consequence, accepted by Silas on 2026-10-12: after a member leaves, is
removed, is banned or logs out, a link already handed out, a cached redirect
or a cached picture can still show that attachment for **up to 60 minutes**.
`Vary: Cookie` makes a new session (another account in the same browser, or
logging in again) ask the API anew instead of reusing the old redirect; it
does not shorten the lifetime of a link someone copied. It also does not
cover a session ended on the server while the browser keeps the same cookie
(for example "log out everywhere" from another device, or a revoked or
expired session): that browser can still follow its cached redirect until the
window ends, up to 30 minutes, within the 60-minute bound above.

`?download=1` is the save variant: the signed link also carries
`response-content-disposition=attachment; filename="…"` (plus RFC 6266
`filename*` for non-ASCII names), so MinIO itself asks the browser to save the
file. A browser ignores `<a download>` after a cross-origin redirect. That
redirect is `Cache-Control: private, no-store`. The memory store answers
`attachment` instead of `inline` for it.

## Delivery through the API (#348)

With `ATTACHMENT_DELIVERY=proxy` the browser never talks to MinIO. The presign
answer names `PUT /api/attachments/{id}/upload` as the upload link and lists
the caller's `X-CSRF-Token` among its headers, so a client that only replays
those headers (as the web client always did) passes the CSRF check. That
route keeps the rules of the signed link: only the uploader, only for
`PUT_TTL` after the presign and before binding, the signed `Content-Type` and
exactly the signed `Content-Length`; the API streams the body to MinIO over
the internal endpoint with that length. `GET /api/attachments/{id}` then
streams the object instead of redirecting, passing a single `bytes=` range
on (`206`, or `416` past the end) so players can seek. The answer carries the
headers MinIO would have sent for a signed link, plus
`Content-Security-Policy: sandbox`, because the file now comes from the app's
own origin. Display answers are `Cache-Control: private, max-age=3600`, the
same 60-minute bound as above; the save variant is `private, no-store`. The
in-memory store always works this way. Downloads and uploads use their own
HTTP client without an overall deadline (10 s to connect, 60 s between
reads), because a 25 MiB file may take minutes on a slow line.

## Transactions and cleanup

Migration 0008 adds `storage_cleanup` and an attachment DELETE trigger. Message,
channel, server and user cascades enqueue immutable object keys in the SAME
transaction as metadata deletion. Rollback also rolls back the intent, and no
physical object deletion occurs before the domain commit. Message DELETE returns
after DB/outbox commit; the cleanup worker performs storage I/O independently.
Mismatch binding first rolls back message creation, then quarantines only
unbound rows owned by that uploader/channel. Valid/foreign uploads survive.

Two independent worker loops run at one-second intervals: expired uploads and
object deletes. Each selects at most 32 rows and permits at most four concurrent
S3 operations. Storage calls have the existing ten-second HTTP timeout and hold
no pool connection. Failed HEADs keep pending metadata/reservation and back off
one minute, allowing later pending rows to progress. Failed deletes retain the
job and retry after one second. Successful deletes repeat after ten seconds until
`retain_until`, then remove the intent after one final confirmed deletion.

Delete jobs use `FOR UPDATE SKIP LOCKED`, a random claim UUID, and a six-minute
claim lease. Every acknowledgement/retry update compares that generation. An API
crash leaves durable jobs eligible when their claim expires. Stale workers cannot
remove a replacement worker's job. A referenced object key is excluded before
claim and before acknowledgement. Keys are generated by the API and never reused.

`retain_until` is the original pending deadline plus fifteen minutes. Keeping the
job past a successful delete covers replay of a previously issued PUT URL and
in-flight uploads within that grace. A real MinIO regression reuses the exact old
presigned URL after message deletion and observes the next cleanup removing it.
The fifteen-minute grace is bounded; an upload that remains in flight beyond
that interval is not proven covered by this change. Existing bound-object PUT
URLs remain usable until their ordinary ten-minute expiry. This change does not
seal/copy bound objects into a second immutable storage key.

## Avatars and banners

Uploaded avatars (#208, `api/src/avatars.rs`) and profile card banners use the
same `storage_cleanup` table and worker, with keys `avatars/{user}/{file}` and
`banners/{user}/{file}`. Everything below holds for both; `PUT /api/me/banner`
behaves like `PUT /api/me/avatar`, and one profile update may replace (and
delete) one of each. `PUT /api/me/avatar` records a pending row (`next_attempt`
and `retain_until` ten minutes ahead) before storing the bytes. A failed PUT returns its error and leaves that row,
because a PUT that only timed out on the client side may still complete in
MinIO; the worker deletes the key once the row is due. The profile update
withdraws the pending row in the same transaction that points the profile at
the object, and records the avatar it really replaced (read under the row
lock) for deletion before commit.

The withdrawal is `DELETE ... WHERE object_key=$1 AND attempts=0`. It relies
on the worker incrementing `attempts` in its claim statement and on nothing
ever resetting it: once the worker has claimed a row (and may be deleting the
object), a late profile save finds no row to withdraw and is refused instead
of pointing the profile at an object that is being deleted. Changing the claim
protocol so that `attempts` is incremented later, or reset, breaks this.

Both uploads are checked for their declared pixel size before anything is
stored (`api/src/image_size.rs`: PNG IHDR, JPEG SOF0-2, WebP VP8/VP8L/VP8X),
at most 256x256 for avatars and 960x320 for banners, the sizes the web client
produces. A file under the byte limit can otherwise declare a canvas that
every viewer's browser has to decode (a 15000x15000 PNG is ~640 KiB). Downloads
repeat the check, so an object stored before it (avatars since v0.6.1) is not
served either. Any signed-in account may download an avatar, like a name, since
avatars travel with message authors and ban lists beyond a shared server; a
banner only to those who may see the card (`GET /api/users/{id}`: the person,
members of a shared server, DM peers).

Avatar and banner bytes count against the daily upload quota
(`API_UPLOAD_QUOTA_BYTES_PER_DAY`, see below) as consumed immediately: after
the type and pixel checks, and before the pending row and the PUT, the body
size is added to today's `consumed`. A denied upload (`429 quota_exceeded`)
stores nothing. The charge is not refunded when the PUT or the profile save
fails afterwards. Storage stays bounded without it, since a replaced image is
deleted; the quota bounds the daily write volume per account, which the
in-process, per-session hourly upload bucket does not.

Migration 0013 (profile card columns) is additive; the previous binary ignores
the new columns. It does refuse to start while `_sqlx_migrations` lists a
version it does not know. Rolling back to a binary without 0013 therefore
means, in one transaction: drop the seven `users_*` checks of 0013, drop the
columns `bio`, `pronouns`, `status_text`, `accent_color`, `banner_color`,
`banner_url`, `avatar_decoration`, and `DELETE FROM _sqlx_migrations WHERE
version = 13`. Only removing the row is not enough: a later upgrade would rerun
0013 and fail on the existing columns. Card data is lost; uploaded banners stay
under `banners/` in the bucket until removed by hand.

## Atomic daily quota

`upload_daily_usage` is keyed by uploader and UTC date, with `reserved` and
`consumed` byte counts. Presign's conditional upsert locks that day's ledger row
until the attachment INSERT trigger records its reservation and the same
transaction commits. Concurrent requests cannot all pass a stale SUM. Failure
rolls back both metadata and reservation; zero configured cap means unlimited.

Avatar and banner uploads have no attachment row and take no reservation.
`limits::consume_upload_quota` adds their bytes to `consumed` in one
autocommit conditional upsert under the same cap (reserved plus consumed plus
the new bytes). It holds no other lock, so it forms no cycle with presign or
the cleanup worker. Those bytes are never refunded, also not after a failed PUT
or profile save.

Binding moves reserved bytes to consumed. Message/channel/server deletion never
refunds consumed bytes. An expiry HEAD404 cannot rule out a valid PUT that began
before expiry but has not finished. The worker therefore keeps the reservation
and expired metadata through `expires_at + 15 minutes`, and defers the next HEAD
to that boundary instead of polling the same upload every second. Expired rows
remain unavailable for bind/download throughout this wait. An uploaded object
becomes consumed before cleanup; a missing object releases its reservation only
when the grace boundary was already passed before the confirming HEAD began.
A PUT completing within grace therefore cannot free capacity for another upload:
its bytes remain reserved until the worker records consumption. Its physical
cleanup may wait until the grace boundary. A cascade
cannot know whether a PUT succeeded, so it queues a conservative charge of an
unknown pending reservation. That reservation stays counted until the worker
atomically transfers it to consumed, guarded by the job claim generation.
Each job takes one ledger lock with `SKIP LOCKED` and retries a busy ledger
without occupying a pool connection waiting for it; cascades do not take ledger locks while holding
multiple parents/attachments. Presign takes user/channel key-share locks before
its ledger lock. This avoids the parent/ledger lock cycle and opposite-order
ledger locking in concurrent cascades. This prevents deletion from buying more upload traffic.
Failed HEADs retain reservations until retry; neither transient failure nor
metadata deletion invents a refund. Prior days do not consume today's quota.

The triggers also record old-schema-style INSERT/bind statements during binary
rollback, keeping the ledger consistent. Only upgraded API writers enforce the
atomic cap. The migration backfills retained existing metadata by its original
UTC creation day. Already deleted historical metadata cannot be reconstructed.

## Migration / rollback / acceptance

0008 is additive and transactional. The regression applies it to populated old
schema, rolls back that migration transaction (metadata unchanged, new tables
absent), reapplies it, and verifies bound/pending backfill and cascade intent.
Binary rollback should keep the new schema, ledger, triggers and durable jobs.
Do not drop them to roll back a binary; the old binary does not process the new
cleanup jobs, so retain an upgraded cleanup worker or resume one promptly.

Local regression coverage includes real PostgreSQL and Redis, plus real MinIO
behind a DELETE-503 failure proxy; quota races, cascade/rollback, expiry, foreign
object safety, 32-row bounds, and a stalled storage request with a pool of one.
The latter replaces the claim while the old worker waits and proves that its
late acknowledgement cannot delete the replacement job. The previously fixed
03a Ban/Invite serialization is rechecked rather than changed again.

This does not constitute production HTTPS/browser attachment acceptance or
physical reclamation of old versions in a versioned bucket. Tests use fresh
unversioned local buckets and only UUID-scoped test objects.

## Executed evidence (2026-09-28)

Rust 1.98.1; PostgreSQL 18.6 UTF8; Redis 8.10.1; local MinIO built from
`RELEASE.2025-10-15T17-29-55Z` (native binary banner `DEVELOPMENT.GOGET`,
Go 1.24.8). The original `cargo test --locked --workspace --all-targets --
--test-threads=2` passed **255 tests, zero failed/ignored**, across 24 targets
(`/tmp/10-workspace-complete.log`). The 12 storage lifecycle regressions also
passed as a targeted suite (`/tmp/10-lifecycle-complete.log`). API all-targets
Clippy with warnings denied passed (`/tmp/10-clippy-complete.log`).

The old SUM/check-before-INSERT quota path was restored temporarily with a
20-ms scheduling pause between admission and insertion: five 4-byte requests
passed an 8-byte cap instead of two. The fixed path admits exactly two
(`/tmp/10-quota-before.log`, `/tmp/10-quota-after.log`). The parent/ledger lock
probe was red before parent locking (`55P03` on the already held ledger),
then green (`/tmp/10-parent-lock-before.log`, `/tmp/10-lifecycle-complete.log`).
A stalled HEAD crossing the pending deadline returned 201 before the wall-clock
recheck, then 422 with no message after it (`/tmp/10-bind-expiry-before.log`,
`/tmp/10-lifecycle-complete.log`). No injected fault remains in the source.

The in-flight PUT follow-up reproduces the independent review against
`521875c6b6988cd99de1f1a34da56e71112a13e3` with real MinIO. It shortens only
the matching signed-URL/DB deadlines to three seconds, sends half the four-byte
body before expiry, observes HEAD404 at expiry, then completes that same PUT
with HTTP200 inside grace. Before the fix, a second four-byte reservation and
actual PUT succeeded under a four-byte daily cap (HTTP201; ledger counted only
the second upload); the expected HTTP429 assertion failed
(`/tmp/10-inflight-before.log`). After the fix, the first reservation stays
counted, the second presign returns HTTP429, and the actual first object settles
to consumed and is deleted after advancing the test's grace deadline. A second
regression checks that a truly absent object remains reserved during grace,
cannot bind/download, then refunds exactly once under concurrent expiry workers.
The targeted suite passed all 14 storage tests (`/tmp/10-inflight-after.log`),
including 32-job bounds and replacement-claim protection. The follow-up changes
no job-claim protocol, migration, wire response, stack pin, or CI configuration.
The final follow-up `cargo test --locked --workspace --all-targets --
--test-threads=2` passed **257 tests, zero failed/ignored**, across 24 targets
(`/tmp/10-inflight-workspace.log`). API all-targets Clippy with warnings denied
also passed (`/tmp/10-inflight-clippy.log`).

An incremental rustc fingerprint ICE occurred during development. The final
checks used `CARGO_INCREMENTAL=0` in this worktree's target directory, with the
pinned toolchain unchanged. The real MinIO regression requires test-only
`MINIO_ENDPOINT` (HTTP loopback), `MINIO_ROOT_USER`, and `MINIO_ROOT_PASSWORD` in
the Rust CI job; its current configuration supplies only PostgreSQL/Redis.
CI wiring belongs to Deploy09 and has been handed off without editing CI files.
