import test from 'node:test';
import assert from 'node:assert/strict';
import { mediasoupNativePeerPublish, mediasoupNativePeerReceive } from '../mediasoup-native-peer-sdp.mjs';
import { janusReceiveBindings } from '../native-peer-adapters.mjs';
const transport = { iceParameters: { usernameFragment: 'test', password: 'test-only-remote-password-123456', iceLite: true },
  iceCandidates: [{ foundation: 'udp', priority: 1, ip: '127.0.0.1', protocol: 'udp', port: 11001, type: 'host' }],
  dtlsParameters: { role: 'auto', fingerprints: [{ algorithm: 'sha-256', value: Array(32).fill('CD').join(':') }, { algorithm: 'sha-512', value: Array(64).fill('EF').join(':') }] } };
const caps = { codecs: [{ kind: 'audio', mimeType: 'audio/opus', preferredPayloadType: 111, clockRate: 48000, channels: 2, parameters: {}, rtcpFeedback: [] },
  { kind: 'video', mimeType: 'video/VP8', preferredPayloadType: 96, clockRate: 90000, parameters: {}, rtcpFeedback: [{ type: 'nack', parameter: '' }] }], headerExtensions: [] };
const sourceRows = [['audio', 'm', 'fixed-native-mic', 0x474d4943], ['video', 's', 'fixed-native-video', 0x47565038], ['audio', 's', 'fixed-native-source-audio', 0x47534130]];
const offer = { type: 'offer', sdp: ['v=0', 'o=- 1 1 IN IP4 127.0.0.1', 's=-', 't=0 0', 'a=group:BUNDLE 0 1 2', 'a=msid-semantic:WMS m s',
  ...sourceRows.flatMap(([kind, stream, track, ssrc], mid) => [`m=${kind} 9 UDP/TLS/RTP/SAVPF ${kind === 'audio' ? 111 : 96}`, 'c=IN IP4 0.0.0.0',
    `a=mid:${mid}`, 'a=ice-ufrag:native', 'a=ice-pwd:native-test-password-123456', 'a=setup:actpass', 'a=fingerprint:sha-256 ' + Array(32).fill('AB').join(':'),
    'a=sendrecv', 'a=rtcp-mux', `a=msid:${stream} ${track}`, kind === 'audio' ? 'a=rtpmap:111 opus/48000/2' : 'a=rtpmap:96 VP8/90000',
    `a=ssrc:${ssrc} cname:fixed-${mid}`, `a=ssrc:${ssrc} msid:${stream} ${track}`]), ''].join('\r\n') };
test('three-track normal WebRTC publish retains actual role/SSRC/MID and advertised certificate', () => {
  const before = JSON.stringify([offer, caps, transport]), bridge = mediasoupNativePeerPublish(offer, caps, transport);
  assert.deepEqual(bridge.publications.map(value => value.role), ['mic', 'video', 'screen-audio']);
  assert.deepEqual(bridge.publications.map(value => value.rtpParameters.encodings[0].ssrc), sourceRows.map(row => row[3]));
  assert.equal(bridge.dtlsParameters.role, 'server'); assert.match(bridge.description.sdp, /a=setup:active/);
  assert.match(bridge.description.sdp, /a=fingerprint:sha-256 CD:CD:/); assert.doesNotMatch(bridge.description.sdp, /sha-512/);
  assert.equal(JSON.stringify([offer, caps, transport]), before);
  for (const changed of [offer.sdp.replaceAll('fixed-native-source-audio', 'fixed-native-video'), offer.sdp.replaceAll('setup:actpass', 'setup:active')]) assert.throws(() => mediasoupNativePeerPublish({ ...offer, sdp: changed }, caps, transport));
});
const consumer = index => ({ id: 'consumer-' + index, producerId: 'producer-' + index, owner: `peer-${index}/mic`, kind: 'audio',
  rtpParameters: { codecs: [{ mimeType: 'audio/opus', payloadType: 111, clockRate: 48000, channels: 2, parameters: {}, rtcpFeedback: [] }],
    headerExtensions: [], encodings: [{ ssrc: 100 + index }], rtcp: { cname: 'test', reducedSize: true } } });
test('native receivers bind each actual worker producer and rewrite SSRC without ordering inference', () => {
  const rows = [consumer(2), consumer(1)], bridge = mediasoupNativePeerReceive(rows, transport);
  assert.deepEqual(bridge.bindings.map(value => [value.source_name, value.ssrc]), [['peer-2/mic', 102], ['peer-1/mic', 101]]);
  assert.equal((bridge.description.sdp.match(/^m=audio /gm) ?? []).length, 2);
  for (const mutate of [rows => { rows[1].owner = rows[0].owner; }, rows => { rows[1].producerId = rows[0].producerId; },
    rows => { rows[1].rtpParameters.encodings[0].ssrc = rows[0].rtpParameters.encodings[0].ssrc; }, rows => { rows[1].owner = 'peer-1/screen-audio'; },
    rows => { delete rows[1].rtpParameters; }]) { const changed = structuredClone(rows); mutate(changed); assert.throws(() => mediasoupNativePeerReceive(changed, transport)); }
});
test('Janus source mapping requires actual feed_id/feed_mid plus offered audio MID/SSRC', () => {
  const description = { type: 'offer', sdp: 'v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=mid:9\r\na=ssrc:777 cname:test\r\n' },
    publications = [{ source_name: 'peer-1/mic', feed: 12, mid: '0', kind: 'audio' }], streams = [{ feed_id: 12, feed_mid: '0', mid: '9', type: 'audio' }];
  assert.equal(janusReceiveBindings(description, streams, publications)[0].ssrc, 777);
  for (const changed of [[{ ...streams[0], feed_id: 13 }], [{ ...streams[0], feed_mid: '2' }], [...streams, streams[0]]]) assert.throws(() => janusReceiveBindings(description, changed, publications));
  assert.throws(() => janusReceiveBindings({ ...description, sdp: description.sdp.replace('a=mid:9', 'a=mid:8') }, streams, publications));
});
