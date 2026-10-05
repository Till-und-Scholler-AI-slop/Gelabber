import { test } from 'node:test';
import assert from 'node:assert/strict';
import { PCM, MarkerDetector, markerCode, markerSample } from '../pcm-kernel.mjs';

function probe(delay, source = 0, detectorSource = source, noise = false) {
  const detections = [], detector = new MarkerDetector(detectorSource, event => detections.push(event));
  const code = markerCode(source), emitted = 24000;
  let random = 19;
  for (let frame = 0; frame < 72000; frame++) {
    random = Math.imul(random, 1664525) + 1013904223 | 0;
    const base = 0.07 * Math.sin(frame * 2 * Math.PI * 719 / PCM.sampleRate);
    detector.push(base + (noise ? (random / 2147483648) * 0.12 : markerSample(code, frame - emitted - delay)), frame);
  }
  return { detections, emitted };
}
test('PCM marker recovers non-quantized known delays within the declared error bound', () => {
  for (const delay of [0, 137, 9600, 17681]) {
    const { detections, emitted } = probe(delay);
    assert.equal(detections.length, 1, `delay=${delay}`);
    const error = Math.abs(detections[0].receivedFrame - emitted - delay) / PCM.sampleRate * 1000;
    assert.ok(error <= PCM.errorBoundMs, `delay=${delay} error=${error}`);
    assert.ok(detections[0].score >= PCM.threshold);
  }
});
test('wrong sources, deterministic noise and silence do not fabricate marker latency', () => {
  assert.equal(probe(9600, 0, 9).detections.length, 0);
  assert.equal(probe(9600, 0, 0, true).detections.length, 0);
  const events = [], detector = new MarkerDetector(0, e => events.push(e));
  for (let frame = 0; frame < 48000; frame++) detector.push(0, frame);
  assert.equal(events.length, 0);
});
test('all 32 microphone codes and separate source audio preserve timing and remain distinct', () => {
  const sources = [...Array.from({ length: 32 }, (_, i) => i), 64];
  assert.equal(new Set(sources.map(source => markerCode(source).join(','))).size, sources.length);
  for (const source of sources) {
    const { detections, emitted } = probe(9600 + source, source);
    assert.equal(detections.length, 1, `source=${source}`);
    assert.ok(Math.abs(detections[0].receivedFrame - emitted - 9600 - source) / PCM.sampleRate * 1000 <= PCM.errorBoundMs);
  }
});
test('markers delayed by a whole period cannot masquerade as the next source cycle', () => {
  const received = [], startFrame = 24000;
  const detector = new MarkerDetector(0, event => received.push(event), startFrame);
  const old = markerCode(0, 0);
  for (let frame = 0; frame < 192000; frame++) detector.push(markerSample(old, frame - startFrame - PCM.periodFrames - 9600), frame);
  assert.equal(received.length, 0);
  const valid = [], current = new MarkerDetector(0, event => valid.push(event), startFrame);
  for (let frame = 0; frame < 192000; frame++) current.push(markerSample(markerCode(0, 1), frame - startFrame - PCM.periodFrames - 9600), frame);
  assert.equal(valid.length, 1); assert.equal(valid[0].sequence, 1);
});
