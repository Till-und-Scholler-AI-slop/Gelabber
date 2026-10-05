import test from 'node:test';
import assert from 'node:assert/strict';
import { RING, dimensions, publishCallback, drainCallbacks } from '../native-pcm-ring.mjs';
import { nativeNs, validateClockSamples, callbackInterval, callbackRange, boundNativeCallback, sourceTimeInterval, latencyInterval } from '../native-pcm-clock-bounds.mjs';
import { completeMarkerMatches, validateReceiverEvidence, qualifyNativeMarkers } from '../native-pcm-evidence.mjs';
import { NATIVE_PCM_POLICY as P, validateBrowserClock } from '../native-pcm-policy.mjs';
import { validateReplayRequest } from '../native-pcm-bridge.mjs';

function cells(groups = 2, capacity = 8) { return new BigInt64Array(new SharedArrayBuffer(dimensions(groups, capacity).cells * 8)); }
const row = (firstFrame, sequence = firstFrame / 128 + 1) => ({ sequence, firstFrame, frames: 128, inputFrames: 128, flags: 0, lowerMs: 100 + firstFrame / 48, upperMs: 101 + (firstFrame + 128) / 48 });
const clocks = Array.from({ length: 300 }, (_, i) => ({ p0: i * 5, p1: i * 5 + .5, monoNs: String(1000000000n + BigInt(i) * 5000000n) }));
const receiver = { uid: 0, role: 'mic', ssrc: 0x474d4943, identityStable: true, live: true, enabled: true, codec: 'audio/opus', decodedSamplesProgress: true, packetsProgress: true, packetsLost: 0, concealedSamples: 0, silentConcealedSamples: 0 };
const marker = { sequence: 0, source_sample_ordinal: 48000 };
const peak = { sequence: 0, receivedFrame: 512, score: .98, amplitude: .35 };
const evidence = () => ({ observer: { failures: [], missing: 0, rows: [Array.from({ length: 70 }, (_, i) => row(i * 128)), []], clocks }, tap: { uid: 0, sampleRate: 48000, gaps: 0, excessPeaks: 0, clipped: 0, nonfinite: 0, peaks: [peak] }, uid: 0, markers: [marker], timeline: { clock: 'CLOCK_MONOTONIC', startClockNs: '1', conversionBracketNs: 10 }, receiver, contextStates: ['running'], sourceArchiveSha256: 'a'.repeat(64), codebookSha256: 'b'.repeat(64) });

test('ring retains exact callback values and independent UID groups', () => {
  const c = cells(); publishCallback(c, 0, 8, 1, 123, 256, 2000000n, 0, 256); publishCallback(c, 1, 8, 1, 333, 128, 3000000n, 8, 128);
  assert.deepEqual(drainCallbacks(c, 0, 8, 0, () => 4000000n), { cursor: 1, missing: 0, rows: [{ sequence: 1, firstFrame: 123, frames: 256, lowerMs: 2, upperMs: 4, flags: 0, inputFrames: 256 }] });
  assert.equal(drainCallbacks(c, 1, 8, 0, () => 4000000n).rows[0].flags, 8);
});
test('overwritten ring slots count as missing and never masquerade as early callbacks', () => {
  const c = cells(1, 2); for (let n = 1; n <= 7; n++) publishCallback(c, 0, 2, n, n * 128, 128, 1n, 0, 128);
  const result = drainCallbacks(c, 0, 2, 0, () => 2n); assert.equal(result.missing, 5); assert.deepEqual(result.rows.map(v => v.sequence), [6, 7]);
});
test('negative/in-progress and concurrently replaced slot fences reject snapshots', () => {
  const c = cells(1, 2), slot = RING.header + RING.groupHeader;
  publishCallback(c, 0, 2, 1, 128, 128, 1n, 0, 128); Atomics.store(c, slot, -1n);
  assert.equal(drainCallbacks(c, 0, 2, 0, () => 2n).missing, 1);
  Atomics.store(c, slot, 1n);
  const replaced = drainCallbacks(c, 0, 2, 0, () => { Atomics.store(c, slot, 3n); return 2n; }); assert.equal(replaced.missing, 1); assert.equal(replaced.rows.length, 0);
});
test('sequences above uint32 preserve slot identity; sequence regression rejects', () => {
  const c = cells(1, 8), n = 2 ** 32 + 5;
  publishCallback(c, 0, 8, n, 128, 128, 1n, 0, 128); assert.equal(drainCallbacks(c, 0, 8, n - 1, () => 2n).rows[0].sequence, n);
  assert.throws(() => drainCallbacks(c, 0, 8, n + 1, () => 2n), /regressed/);
});
test('native clock values preserve nanoseconds and reject lossy or noncanonical inputs', () => {
  assert.equal(nativeNs('123456789123456789'), 123456789123456789n);
  for (const value of [1234, '-1', '01', '1e6', '9223372036854775808']) assert.throws(() => nativeNs(value));
});
test('clock request serialization and native monotonicity are mandatory', () => {
  assert.doesNotThrow(() => validateClockSamples(clocks));
  assert.throws(() => validateClockSamples([clocks[0], { ...clocks[1], p0: 0 }]), /regressed/);
  assert.throws(() => validateClockSamples([clocks[0], { ...clocks[1], monoNs: clocks[0].monoNs }]), /regressed/);
});
test('marker uncertainty includes every intersecting callback and actual variable quantum', () => {
  const rows = [row(0), row(128), { ...row(256), frames: 256, inputFrames: 256 }, row(512)];
  const interval = callbackInterval(rows, 300); assert.equal(interval.firstFrame, 128); assert.equal(interval.endFrame, 512); assert.equal(interval.blocks, 2);
  assert.throws(() => callbackInterval(rows.filter(v => v.firstFrame !== 128), 300), /missing/);
});
test('full marker range rejects input loss, clipping, nonfinite or callback duplication', () => {
  for (const invalid of [{ flags: 2 }, { flags: 8 }, { flags: 4 }, { inputFrames: 0 }, { frames: 0 }]) assert.throws(() => callbackRange([row(0), { ...row(128), ...invalid }, row(256)], 50, 300));
  assert.throws(() => callbackRange([row(0), { ...row(128), sequence: 1 }, row(256)], 50, 300));
});
test('causal before/after probes bound callbacks without offset or clock-rate fitting', () => {
  const result = boundNativeCallback({ lowerMs: 51, upperMs: 53 }, clocks);
  assert.equal(result.lowerNs, clocks[10].monoNs); assert.equal(result.upperNs, clocks[11].monoNs); assert.equal(result.widthMs, 5);
  const drifting = clocks.map((v, i) => ({ ...v, monoNs: String(BigInt(v.monoNs) + BigInt(i * i * 1000)) }));
  const bounded = boundNativeCallback({ lowerMs: 51, upperMs: 53 }, drifting); assert.equal(bounded.lowerNs, drifting[10].monoNs); assert.equal(bounded.upperNs, drifting[11].monoNs);
});
test('clock stall, missing bracketing and forged interval policies reject narrow results', () => {
  assert.throws(() => boundNativeCallback({ lowerMs: 51, upperMs: 500 }, clocks), /too wide/);
  assert.throws(() => boundNativeCallback({ lowerMs: 0, upperMs: 1 }, clocks), /not bracketed/);
  assert.throws(() => boundNativeCallback({ lowerMs: 51, upperMs: 53 }, clocks, { maxWidthMs: NaN }), /policy/);
});
test('source time uses exact ordinal ratio and conservative actual conversion bracket', () => {
  const t = sourceTimeInterval({ clock: 'CLOCK_MONOTONIC', startClockNs: '1000000000', conversionBracketNs: 20 }, 1);
  assert.deepEqual(t, { lowerNs: '1000020813', upperNs: '1000020854' });
  assert.deepEqual(latencyInterval(t, { lowerNs: '1200020813', upperNs: '1200020854' }), { lowerMs: 199.999959, upperMs: 200.000041 });
  assert.throws(() => sourceTimeInterval({ clock: 'REALTIME', startClockNs: '1', conversionBracketNs: 0 }, 1));
});
test('every expected finite code must match exactly once; no partial or duplicate acceptance', () => {
  assert.equal(completeMarkerMatches([peak], { uid: 0, markers: [marker] }).length, 1);
  for (const peaks of [[], [peak, { ...peak, receivedFrame: 14512 }], [{ ...peak, sequence: 1 }]]) assert.throws(() => completeMarkerMatches(peaks, { uid: 0, markers: [marker] }));
  assert.throws(() => completeMarkerMatches([peak], { uid: 13, markers: [marker] }));
  assert.throws(() => completeMarkerMatches([peak], { uid: 0, markers: [{ ...marker, source_sample_ordinal: 96000 }] }));
});
test('actual UID/SSRC role and held receiver/live/enabled plus loss/PLC counters are required', () => {
  assert.doesNotThrow(() => validateReceiverEvidence(receiver, 0));
  for (const change of [{ uid: 64 }, { role: 'source' }, { ssrc: 1 }, { live: false }, { enabled: false }, { identityStable: false }, { packetsLost: undefined }, { concealedSamples: 128 }, { silentConcealedSamples: 1 }, { live: 'false' }, { identityStable: 1 }, { enabled: 'true' }, { packetsProgress: 1 }, { decodedSamplesProgress: 'false' }]) assert.throws(() => validateReceiverEvidence({ ...receiver, ...change }, 0));
});
test('qualification is all-or-nothing and cannot imply SFU comparison or native calibration', () => {
  const result = qualifyNativeMarkers(evidence()); assert.equal(result.qualified, true); assert.equal(result.intervals.length, 1); assert.equal(result.comparison_available, false); assert.equal(result.pcm_latency_calibrated, false);
  for (const modify of [e => e.observer.missing++, e => e.contextStates.push('suspended'), e => e.tap.peaks = [], e => e.receiver = { ...receiver, enabled: false }, e => e.tap.excessPeaks++, e => e.observer.rows[0][8].flags = 2]) {
    const e = evidence(); modify(e); const failed = qualifyNativeMarkers(e); assert.equal(failed.qualified, false); assert.equal(failed.intervals.length, 0); assert.ok(failed.failures.length);
  }
});
test('epsilon policy only accepts exact actual browser revision and observed COI precision', () => {
  const valid = { browser: { revision: P.chromiumRevision, product: 'HeadlessChrome/' + P.chromiumVersion }, crossOriginIsolated: true, precision: { samples: 100000, minimumStepMs: .005 } };
  assert.doesNotThrow(() => validateBrowserClock(valid));
  for (const invalid of [{ crossOriginIsolated: false }, { browser: { ...valid.browser, revision: 'other' } }, { precision: { samples: 100000, minimumStepMs: .1 } }]) assert.throws(() => validateBrowserClock({ ...valid, ...invalid }));
});
test('clock-only bridge cannot submit a native peer, SDP, ICE or replay', () => {
  for (const op of ['create', 'offer', 'remote', 'ice', 'start', 'status', 'close']) assert.throws(() => validateReplayRequest({ op, peer: 'publish' }));
});
test('V2 bridge rejects duration truncation/looping, unsupported holds and foreign peers', () => {
  const policy = { allowReplay: true, totalSeconds: 21, audioHoldMs: 200 }, request = { op: 'start', peer: 'publish', total_seconds: 21, audio_hold_ms: 200 };
  assert.deepEqual(validateReplayRequest(request, policy), request);
  for (const change of [{ total_seconds: 20 }, { seconds: 20, total_seconds: undefined }, { audio_hold_ms: 50 }, { peer: 'other' }]) assert.throws(() => validateReplayRequest({ ...request, ...change }, policy));
});
