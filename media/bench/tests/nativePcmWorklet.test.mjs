import test from 'node:test';
import assert from 'node:assert/strict';
import { PCM, markerCode, markerSample } from '../pcm-kernel.mjs';
import { RING, dimensions, drainCallbacks } from '../native-pcm-ring.mjs';
import { completeMarkerMatches } from '../native-pcm-evidence.mjs';

let Tap;
globalThis.sampleRate = 48000; globalThis.currentFrame = 0;
globalThis.AudioWorkletProcessor = class { constructor() { this.port = { postMessage() {} }; } };
globalThis.registerProcessor = (name, klass) => { assert.equal(name, 'gelabber-native-pcm-tap'); Tap = klass; };
await import('../native-pcm-worklet.mjs');
const marker = { sequence: 0, source_sample_ordinal: 48000 };
function render({ uid = 0, detectorUid = uid, starts = [52800], seconds = 3, loseInputAt, nonfiniteAt, clippedAt, jumpAt } = {}) {
  const capacity = 32768, cells = new BigInt64Array(new SharedArrayBuffer(dimensions(2, capacity).cells * 8));
  const tap = new Tap({ processorOptions: { groups: 2, group: 0, capacity, sab: cells.buffer, uid: detectorUid, markerCount: 1, mode: 'receive' } });
  tap.port.onmessage({ data: { type: 'anchor', startClockNs: '1000000000' } });
  const code = markerCode(uid, 0);
  let markerInputSamples = 0;
  for (let frame = 0; frame < seconds * PCM.sampleRate; frame += 128) {
    globalThis.currentFrame = frame + (jumpAt !== undefined && frame >= jumpAt ? 128 : 0);
    Atomics.store(cells, RING.nativeLower, 1000000000n + BigInt(Math.floor(frame / 48000 * 1e9)));
    Atomics.store(cells, RING.heartbeat, BigInt(frame + 1) * 1000000n);
    const input = new Float32Array(128), output = new Float32Array(128);
    for (let i = 0; i < 128; i++) input[i] = starts.reduce((sum, start) => sum + markerSample(code, frame + i - start), 0);
    markerInputSamples += input.filter(value => Math.abs(value) > .001).length;
    if (nonfiniteAt === frame) input[0] = NaN; if (clippedAt === frame) input[0] = 1;
    tap.process(frame === loseInputAt ? [] : [[input]], [[output]]);
    assert.ok(output.every(value => value === 0));
  }
  return { tap, markerInputSamples, rows: drainCallbacks(cells, 0, capacity, 0, () => 1000000000000n).rows };
}
test('actual worklet/kernel decode detector retains one expected PN peak', () => {
  const { tap } = render(); const [match] = completeMarkerMatches(tap.peaks, { uid: 0, markers: [marker] });
  assert.ok(Math.abs(match.peak.receivedFrame - 52800) <= 96); assert.equal(tap.excessPeaks, 0);
});
test('actual worklet duplicate copies retain two peaks and reject ambiguity', () => {
  const { tap } = render({ starts: [52800, 76800] }); assert.equal(tap.peaks.length, 2);
  assert.throws(() => completeMarkerMatches(tap.peaks, { uid: 0, markers: [marker] }), /duplicate/);
});
test('wrong UID and marker-free decoded input do not qualify partial measurements', () => {
  for (const config of [{ uid: 64, detectorUid: 0 }, { starts: [] }]) {
    const { tap } = render(config); assert.throws(() => completeMarkerMatches(tap.peaks, { uid: 0, markers: [marker] }));
  }
});
test('lost input, clipping, nonfinite and discontinuous frames remain explicit ring flags', () => {
  for (const [config, flag] of [[{ loseInputAt: 53248 }, 2], [{ clippedAt: 53248 }, 8], [{ nonfiniteAt: 53248 }, 4], [{ jumpAt: 53248 }, 1]]) {
    const { rows } = render(config); assert.ok(rows.some(row => row.flags & flag));
  }
});
test('markers outside finite candidate window fail completeness rather than inventing a latency', () => {
  const { tap, markerInputSamples } = render({ starts: [240000], seconds: 7 }); assert.ok(markerInputSamples > 5000); assert.equal(tap.peaks.length, 0); assert.throws(() => completeMarkerMatches(tap.peaks, { uid: 0, markers: [marker] }));
});
test('tap rejects foreign sample rate, source UID and repeated timeline anchors', () => {
  const sab = new SharedArrayBuffer(dimensions(1, 2).cells * 8);
  const options = { groups: 1, group: 0, capacity: 2, sab, uid: 0, markerCount: 1, mode: 'receive' };
  assert.throws(() => new Tap({ processorOptions: { ...options, uid: 3 } }));
  globalThis.sampleRate = 44100; assert.throws(() => new Tap({ processorOptions: options })); globalThis.sampleRate = 48000;
  const tap = new Tap({ processorOptions: options }); tap.port.onmessage({ data: { type: 'anchor', startClockNs: '1' } });
  assert.throws(() => tap.port.onmessage({ data: { type: 'anchor', startClockNs: '2' } }));
});
