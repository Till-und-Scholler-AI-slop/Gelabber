---
name: test-services
description: Start Postgres, Redis and MinIO with the right environment before running the backend (Rust) tests or the API locally. Use when backend tests cannot connect to a service or a task needs the API running.
---

# Test services

From the repository root, in the same shell command as the tests:

```sh
eval "$(.claude/skills/test-services/services.sh)" && cd backend && cargo test --locked -p gelabber-api
```

The script starts whatever is not running yet (Postgres, Redis, and MinIO
from the deployment's pinned image through Docker) and prints the
`export` lines from `backend/api/.env.example`. Environment variables, and in
some sandboxes the services themselves, do not outlive one command, so chain
them with `&&` instead of starting them in an earlier call.

- The database role needs `CREATEDB` for `sqlx::test`. Where it is missing:
  `CREATE ROLE gelabber LOGIN CREATEDB PASSWORD 'gelabber'` and
  `createdb -O gelabber gelabber` as the `postgres` user.
- Without MinIO the storage and avatar tests fail; the rest run.
- CI runs the tests in the native toolchain image (AGENTS.md, "Commands").
  Running `cargo test` directly works for `gelabber-api` and
  `gelabber-shared`. `gelabber-media` builds mediasoup's worker, which
  downloads sources from github.com; where that is blocked, type-check it
  with `DOCS_RS=1 cargo clippy -p gelabber-media --tests -- -D warnings` and
  leave its tests to CI.
