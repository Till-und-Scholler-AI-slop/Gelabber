# Separate11c teardown correction

a53ccfe remains intact. Its independent clean Chromium core-faults13 and Firefox core11 controls, cleanup200/204/200 and native/harness checks passed. The independent review identified one test-runner P2: a rejected web.close skipped runtime.close and cache removal. Root probe /tmp/gelabber-review11c-teardown-probe.mjs executes the exact committed runner with infrastructure stubs; its JSON records both attempts false.

The follow-up attempts every acquired resource independently: proxy restoration, own Vite, own API runtime, runtime binding and cache. API runtime teardown similarly continues through every lock, proxy restoration, own child termination, reverse-order proxy close and private log close. Failed locker pipe/commit still attempts termination of that owned locker. Child stopping has bounded graceful/forced waits; no foreign PID is used. Proxy arrays are not mutated during close.

Failures contain controlled step names only. Runner writes a separate mode600 `<report>.teardown.json` with FAIL/test-error, failedSteps and completeAcceptance:false, and sets Exit1. Failure to write that artifact remains Exit1 with a fixed redacted stderr reason. Underlying error strings, credentials and signed URLs are discarded. Passing scenario/cleanup rows are not retroactively rewritten; process and teardown artifact make the failed overall run explicit. Existing data, revocation, fixture-restoration and cleanup gates are unchanged.

## Regression evidence

```bash
source /tmp/gelabber-toolchain/env.sh
node --experimental-vm-modules web/scripts/e2e/teardown.checks.mjs
```

Six targeted tests PASS. They execute actual fault-run/startFaultApi source, stubbing infrastructure only: rejected web.close; multiple restore/close failures plus failed evidence write; startup failure; failed API lock/restore/proxy/log stages; failed actual locker pipe with remaining API resources; successful ordering/exit control. Against exact a53ccfe fault-run/fault-runtime sources, the same regression file produced Exit1 with four failures/two passes. No browser data assertion was relaxed. Node emits its standard experimental VM warning; production app APIs are unaffected.

A focused real native success control then started only the authorized own API, created one own fixture server, acquired/released its real DB lock, deleted it204 and closed the API/proxies. Own DB other-clients0, ports18086/16386/19086 independently rebound successfully afterward. No shared service changed, no whole green browser matrix repeated. Script /tmp/gelabber-e2e-teardown-native-control.mjs; private config never emitted. [teardown-validation.json](evidence/teardown-validation.json) records hashes and outcomes.

CI-WIP and deleted-room/media WIP are excluded from this follow-up commit. Full media runtime remains pending. Coordinator owns separate review/integration; no remote push or deployment.
