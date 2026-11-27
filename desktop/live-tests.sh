#!/bin/sh
# The desktop tests against a seeded stack (web/scripts/parity/seed.mjs,
# desktop/README.md "Tests"): the client's, the session keeper's, and a call
# with Chromium people (web/scripts/parity/call.mjs). GELABBER_TEST_SERVER
# names the stack's web origin. e2e-extended.yml runs this after merges.
set -u
: "${GELABBER_TEST_SERVER:?the seeded stack's web origin, e.g. http://127.0.0.1:5180}"
manifest="$(dirname "$0")/Cargo.toml"
status=0
# One at a time: they share the seeded people.
cargo test --locked --manifest-path "$manifest" -p gelabber-client --test live \
  -- --ignored --test-threads=1 || status=1
cargo test --locked --manifest-path "$manifest" -p gelabber-ui --lib \
  platform::session::tests::signed_in_after_a_restart -- --ignored --exact || status=1
cargo test --locked --manifest-path "$manifest" -p gelabber-call --features media --test live \
  -- --ignored --nocapture || status=1
exit "$status"
