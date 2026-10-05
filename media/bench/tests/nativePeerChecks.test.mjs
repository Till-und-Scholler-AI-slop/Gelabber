import test from 'node:test';
import assert from 'node:assert/strict';
import { nativeBrowserAudio, nativePublicationIdentity, NATIVE_AUDIO_SOURCES } from '../native-peer-checks.mjs';
const control = () => [0, 1000, 2000].map((timestamp, index) => ({ connection: 'connected', errors: [], stats: [
  { type: 'codec', id: 'opus', mimeType: 'audio/opus', clockRate: 48000 },
  ...[...NATIVE_AUDIO_SOURCES.keys()].map((ssrc, offset) => ({ type: 'inbound-rtp', kind: 'audio', id: 'edge-' + offset,
    ssrc, codecId: 'opus', timestamp, packetsReceived: 10 + index * 50, bytesReceived: 3200 + index * 16000,
    totalSamplesReceived: 9600 + index * 48000, packetsLost: 0, concealedSamples: 0 }))
] }));
test('actual SDP source IDs match all three distinct publication roles', () => {
  const rows = [['audio', 'm', 'fixed-native-mic', 0x474d4943], ['video', 's', 'fixed-native-video', 0x47565038], ['audio', 's', 'fixed-native-source-audio', 0x47534130]];
  const offer = { type: 'offer', sdp: 'v=0\r\n' + rows.map(([kind, stream, track, ssrc], mid) =>
    `m=${kind} 9 UDP/TLS/RTP/SAVPF 111\r\na=mid:${mid}\r\na=msid:${stream} ${track}\r\na=ssrc:${ssrc} msid:${stream} ${track}\r\n`).join('') };
  assert.deepEqual(nativePublicationIdentity(offer).map(row => row.track_id), rows.map(row => row[2]));
  for (const changed of [offer.sdp.replaceAll('s fixed-native-video', 'fixed-native-video s'),
    offer.sdp.replaceAll('fixed-native-source-audio', 'fixed-native-video'), offer.sdp.replace('a=mid:2', 'a=mid:1'),
    offer.sdp.replace(`a=ssrc:${0x474d4943}`, 'a=ssrc:42')]) assert.throws(() => nativePublicationIdentity({ ...offer, sdp: changed }));
});
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
