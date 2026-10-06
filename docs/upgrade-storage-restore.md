# Local upgrade, paired storage restore and API rollback

`tools/check-upgrade-storage-restore.py` exercises the actual HTTP API against
fresh PostgreSQL, Redis and MinIO containers. It never reads a deployment `.env`,
connects to a remote daemon or accepts an existing database/volume name. Every
container and volume gets a generated nonce and ownership label; cleanup checks
both before removing it. Output must be a new private directory. Keep the
artifacts private: dumps contain disposable fixture password hashes/sessions.

Run from a feature checkout containing the unchanged PR153/154 migrations:

```bash
python3 tools/check-upgrade-storage-restore.py \
  --old-ref v0.3.1 --current-ref HEAD \
  --output /tmp/gelabber-storage-restore-unique
```

The three existing pinned images must be present locally:

- `postgres:18.6-alpine3.24`
- `redis:8.10.1-alpine`
- `ghcr.io/till-und-scholler-ai-slop/gelabber/minio:RELEASE.2025-10-15T17-29-55Z`

No images are pulled or tags rewritten by the runner. It records actual image
IDs and available RepoDigests, and uses the inspected image IDs throughout the
drill. It builds two frozen Git archives with their own `rust-toolchain.toml`
and `cargo build --locked -p gelabber-api`. Revision, archive/binary SHA256,
Cargo.lock, toolchain and every migration hash go into `report.json`. The old
revision must have migrations 0001–0009; the candidate must contain 0010/0011.
There is no claim that a locally built feature binary is a published release.

An existing Cargo cache can be supplied with `--target-dir /absolute/cache`.
The cache is never cleaned. Coordinate shared builds first; the runner refuses
to replace an API executable currently running from that output inode on Linux.
Immediately copied per-run binaries are the only processes used by the drill.
If the daemon socket requires the desktop's `docker` group, run the same command
from a shell with that group, such as `newgrp docker`.

The acceptance sequence is:

1. Boot the frozen old API; register owner, member and outsider through real
   session/CSRF endpoints. Create a server, redeem its invite and open a DM.
2. PUT PNG/PDF/channel and text/DM bytes into real MinIO using API presigns.
   Bind them to messages; keep another uploaded attachment pending. Remove the
   member's SendFiles permission while keeping download/message access.
3. Verify every body SHA256/length/Content-Type and attachment DTO. Verify
   outsider/anonymous denial, pending uploader-only access, denied member
   upload and unsigned private-bucket GET denial.
4. Drain the sole API writer and stop MinIO, including still-valid direct PUTs.
   Take a custom `pg_dump` and complete stopped MinIO volume archive as one pair.
   Record hashes, exact public table rows and storage image identities.
5. Start the candidate on the old data. The real SQLx migrator upgrades it;
   existing users/memberships/messages/attachment/quota rows must survive.
   Exercise scoped PostgreSQL search, account read state and reactions for
   both channel and DM, plus a new attachment write.
6. Replace that new object's bytes with different bytes of exactly the same
   size and type. Its download must fail the expected SHA256 assertion; put
   the original bytes back before taking the candidate snapshot.
7. Restore the candidate pair into a fresh database and fresh object volume.
   Compare every public table's complete rows, including migration ledger,
   attachment metadata, ACLs, quota, reactions and read state. Repeat the real
   API/storage checks and verify the restored creation sequence accepts a new
   message. A modified archive is rejected before a restore mutation.
8. Record whether the actual old binary can boot against the upgraded migration
   ledger. This is separate from restoring the old snapshot. Combine the new
   DB with the old object snapshot as a negative control: the post-upgrade
   attachment metadata exists, but its missing object must reject the restore.
9. Restore the matching old DB and object snapshots to new targets, boot the
   actual frozen old API, authenticate and repeat all legacy download/ACL checks.
   A message write must succeed. The post-snapshot attachment must be absent;
   that explicit recovery-point data-loss boundary is recorded.
10. Stop only owned API children and remove only nonce-labelled local fixture
    containers/volumes. Cleanup failures make the report fail.

`report.json`, build/startup logs, snapshot manifests, dumps, object archives and
frozen executables stay in the chosen artifact directory for review. A failed
run still writes its report; the directory is never silently reused. The final
acceptance report identifies each negative control's exact failure so an
unrelated authentication/network failure cannot count as a passing control.

Fast safety tests:

```bash
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s tools/tests -v
```

This is a slow local gate, not a CI browser/job extension. It verifies API
binary/data rollback, not an immutable Web/Media/API deployment image-set,
production readiness, Caddy/TLS, device behavior or a production recovery time.
An operator must still rehearse the actual archived deployment image set and
configuration before a production rollout. The existing PostgreSQL-only
`tools/check-chat-migration-restore.py` does not satisfy the objectstore/binary
part of this gate.

## Recorded local acceptance

[Full report](upgrade-storage-restore-evidence.json): 20/20 gates passed on frozen
runner/candidate `30764470087104a680353da15cf7dda064d5ff60` and old API source
`5a54a003307465bfaa1430085cbf3f0067fa3b79` (`v0.3.1`). All nine fast safety tests
passed. Actual services were PostgreSQL 18.6, Redis 8.10.1 and MinIO's locked CE
release; both APIs used Rust/Cargo 1.98.1. The raw private artifacts remain at
`/tmp/gelabber-upgrade-storage-drill-20261005-final-3076447` on the test host.
Every nonce-owned container and volume was removed, confirmed by an independent
Docker inventory after the runner completed. No production services were used.

The old API against the upgraded schema exited before HTTP bind with SQLx
`migration 10 was previously applied but is missing in the resolved migrations`.
The same old executable passed boot/login/download/ACL/write checks after the
paired old restore. An image-only rollback is therefore **not** accepted by this
drill. The measured 32.098-second local run used a warm Cargo cache and tiny
disposable fixtures; it is not a production restore-time guarantee. Web/Media
deployment images and the production archived image set remain separate gates.
