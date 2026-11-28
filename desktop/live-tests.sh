#!/usr/bin/env bash
# The desktop tests against a seeded stack (web/scripts/parity/seed.mjs,
# desktop/README.md "Tests"): the client's, the session keeper's, and a call
# with Chromium people (web/scripts/parity/call.mjs). GELABBER_TEST_SERVER
# names the stack's web origin. e2e-extended.yml runs this after merges.
set -uo pipefail
: "${GELABBER_TEST_SERVER:?the web origin of the seeded stack, e.g. http://127.0.0.1:5180}"
manifest="$(dirname "$0")/Cargo.toml"
out=$(mktemp)
trap 'rm -f "$out"' EXIT
status=0

# Runs one test binary's ignored tests. cargo test passes when nothing
# matches, so a renamed test, or one that lost its #[ignore], would leave
# this green with nothing run: it fails when no test ran, and with `whole`
# when a test of the binary was left out (one without #[ignore]).
live() {
  local whole=$1
  shift
  cargo test --locked --manifest-path "$manifest" "$@" 2>&1 | tee "$out" || status=1
  if ! grep -Eq '^running [1-9][0-9]* tests?$' "$out"; then
    echo "live-tests.sh: no test ran for: $*" >&2
    status=1
  elif [ "$whole" = whole ] && ! grep -Eq '^test result: .* 0 filtered out' "$out"; then
    echo "live-tests.sh: a test without #[ignore] was left out for: $*" >&2
    status=1
  fi
}

# One at a time: they share the seeded people.
live whole -p gelabber-client --test live -- --ignored --test-threads=1
live part -p gelabber-ui --lib platform::session::tests::signed_in_after_a_restart -- --ignored --exact
live whole -p gelabber-call --features media --test live -- --ignored --nocapture
exit "$status"
