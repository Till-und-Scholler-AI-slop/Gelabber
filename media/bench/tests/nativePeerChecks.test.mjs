import test from 'node:test';
import assert from 'node:assert/strict';
import { nativeBrowserAudio, NATIVE_AUDIO_SOURCES } from '../native-peer-checks.mjs';
const control = () => [0, 1000, 2000].map((timestamp, index) => ({ connection: 'connected', errors: [], stats: [
  { type: 'codec', id: 'opus', mimeType: 'audio/opus', clockRate: 48000 },
  ...[...NATIVE_AUDIO_SOURCES.keys()].map((ssrc, offset) => ({ type: 'inbound-rtp', kind: 'audio', id: 'edge-' + offset,
    ssrc, codecId: 'opus', timestamp, packetsReceived: 10 + index * 50, bytesReceived: 3200 + index * 16000,
    totalSamplesReceived: 9600 + index * 48000, packetsLost: 0, concealedSamples: 0 }))
] }));
test('actual distinct decoder rows qualify both independent source roles', () => {
  const result = nativeBrowserAudio(control()); assert.equal(result.valid, true);
  assert.deepEqual(result.roles.map(row => row.role), ['peer-0/mic', 'peer-0/screen-audio']);
});
test('duplicated microphone rows cannot qualify the separate source audio', () => {
  const samples = control(); samples.forEach(sample => sample.stats[2].ssrc = sample.stats[1].ssrc);
  assert.equal(nativeBrowserAudio(samples).valid, false);
});
test('wrong codec, decoder identity, loss, concealment, stalled counters and connection/errors fail', () => {
  const mutations = [samples => { samples[1].stats[0].mimeType = 'audio/PCMU'; },
    samples => { samples[1].stats[1].id = 'other'; }, samples => { samples[2].stats[1].packetsLost = 1; },
    samples => { samples[2].stats[2].concealedSamples = 960; }, samples => { samples[1].stats[1].bytesReceived = 3200; },
    samples => { samples[1].connection = 'disconnected'; }, samples => { samples[1].errors.push('stats error'); },
    samples => { samples[1].stats[1].packetsLost = 1; }, samples => { delete samples[1].stats[0].mimeType; },
    samples => { delete samples[2].stats[1].totalSamplesReceived; }];
  for (const mutation of mutations) { const samples = control(); mutation(samples); assert.equal(nativeBrowserAudio(samples).valid, false); }
});
