# Bounded native harness and explicit local ICE adapter

This test-only follow-up is separate from the Media/Access scenario delivery and CI wiring. It follows independently reviewed24211fb. Product code, pins, original smoke assertions and shared services are unchanged. Coordinator owns independent review/integration; no push/release/deploy.

## Confirmed harness defect and boundary

The old polling/observation helper awaited an unbounded probe. Snapshot awaited page.evaluate, which awaited native pc.getStats. The aborted Firefox adapter matrix proves a missing harness bound, **not which native call hung**. Every probe now receives one absolute observation deadline and remaining budget. Already-expired work never starts; late results cannot satisfy assertions. Native getStats timeout is distinguished from outer page/evaluate timeout. Neither returns fabricated empty stats/frames. Interface failure is test-error/FAIL, subsequent selected cases are BLOCKED, and completeAcceptance remains false.

A later DeletedRoom control exposed a final-poll scheduling defect: early timer wake-up could start a tail probe with an already-expired budget. Fixed200ms cadence now waits until the next scheduled poll, or waits out the remaining absolute window without another evaluation. `observe` returns the last actual sample; `until` without a hit throws its normal CheckFailure with last. A deterministic actual-source early-waking timer control covers both, plus no late acceptance. The original red remains and does not prove a native getStats hang.

Timeout quarantines a page before finally and requests one bounded page close. No further evaluate is issued there. Healthy stop evaluations and page/context/browser close waits are bounded; failures become visible red rows. Report writing and independent owned API/proxy/SFU/cache teardown remain reachable after a close rejection/timeout. Native-interface regressions execute actual helpers/sample and actual finish source with infrastructure fakes; these prove control flow, not a browser-native hang cause. Access/Lease's separate held-peer/restore path is completed and reviewed in the subsequent scenario commit.

## Adapter contract

Both GELABBER_E2E_FIREFOX_LOOPBACK_ICE and GELABBER_E2E_SFU_LOOPBACK_ICE default off. Explicit local Firefox forced relay requires both flags plus the mode600 own media-fault-loopback-control.env. Scope guards reject invalid combinations before any API/SFU acquisition. The second flag selects only own SFU18087 with exact copied6e binary and loopback-v4 ephemeral ICE; no MID/index override or inherited port maximum. Direct, Chromium, WAN, persistent profiles and autoplay remain outside this exception.

Actual own process binary and private parameter matches are attested categorically. Adapter reports include both flags/pref, copied runtime SHA/digests, private control-env SHA256, unchanged baseline-env/API-/SFU-manifest hashes before/after and each owned restore failure. Changed baselines, close errors or report-write failure stay Exit1. Missing native pair/STUN stats are unavailable/null, never measured zero. A local adapter PASS is not Firefox-default/WAN acceptance.

Before matrix authorization, native SFU-loopback control selected succeeded Relay→Host with RTP0→211; separate actual-app GoLiveWatch/Gateway controls passed correct640px changing decoded/rendered frames and original Gateway capture/peer. Original default and pref-only non-loopback reds remain. MID-null/index-null native TypeError is a separate open input robustness finding; a diagnostic explicit-index control did not establish a relay pair and is never applied by acceptance code.

## Narrow controls and restore evidence

The bounded WIP actual-app MediaWS control `/tmp/gelabber-e2e/media-ws-firefox-relay-bounded-control.json` SHA256 `fbe0874f84b234bdd394ace4e4438aa4ef0a9c289ed2052160fb5a4164182e59` passes, Exit0, recoveryFirstFrame211ms, retained original capture track, cleanup200/204/200. It uses exact approved immutable Web/SFU6e22170 and API243c86c under both visible adapters. This is author dirty24211fb WIP with per-test-module hashes, **not a clean execution of this later commit**. Restore sidecar SHA256 `6a2a41176124bbe9a02c267cac677013e41d2a017a393242280239c3f2638303` is PASS/baseline unchanged; dated post-control17:41:05Z SHA256 `a8ec3a4fc61b46ef837565df93137f06b379c4bfd39f7b608b60c1f9b293de14` records five own ports free/DBotherclients0/older remaining fixture1.

The original aborted whole adapter matrix and cleanup failure remain FAIL/incomplete. Later API-only recovery deleted only its precisely identified new fixture through ordinary authenticated DELETE204 and removed its single temporary owner session. Older server7de90fa2-d229-4453-8b68-b5db82d1429c and its one attachment are explicitly excluded and untouched. Post-recovery and post-control port proofs are separate later observations, not retrospective matrix cleanup PASS. Native hang cause remains unproven. No whole-matrix retry is claimed.

## Source checks

Pinned Node26.8.2/Playwright1.63.0. Run from web:

```bash
node scripts/e2e/native-evaluate.checks.mjs
node --experimental-vm-modules scripts/e2e/native-finish.checks.mjs
node scripts/e2e/browser-options.checks.mjs
node scripts/e2e/media-probe.checks.mjs
node scripts/e2e/rendered-frames.checks.mjs
npm run test:e2e-harness
node --experimental-vm-modules scripts/e2e/media-runtime.checks.mjs
node --experimental-vm-modules scripts/e2e/teardown.checks.mjs
node scripts/e2e/faults.checks.mjs
```

Native7, finish3, browser guards3, media probe2, native counter3, harness6, runtime3, teardown9 and native fault4 cover deadline/quarantine/late-success, never-resolving close, invalid adapter startup, per-process config/binary attestation and close-all-resources failures. Host permission is needed only for native ephemeral TCP controls; these own their sockets and do not start an API worker. Exact post-commit source checks and hashes are indexed in evidence/native-interface-validation.json; native WIP controls retain their original attribution. Original render/relay/audio/recovery/cleanup assertions remain strict.
