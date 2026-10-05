import test from 'node:test';
import assert from 'node:assert/strict';
import { mediasoupNativeAnswer } from '../mediasoup-native-sdp.mjs';

const offer = { type: 'offer', sdp: [
  'v=0', 'o=- 1 1 IN IP4 127.0.0.1', 's=-', 't=0 0', 'a=group:BUNDLE 0', 'a=msid-semantic:WMS source',
  'm=video 9 UDP/TLS/RTP/SAVPF 96', 'c=IN IP4 0.0.0.0', 'a=mid:0', 'a=ice-ufrag:native', 'a=ice-pwd:native-test-password-123456',
  'a=setup:actpass', 'a=fingerprint:sha-256 ' + Array(32).fill('AB').join(':'), 'a=sendonly', 'a=rtcp-mux',
  'a=rtpmap:96 VP8/90000', 'a=rtcp-fb:96 nack', 'a=rtcp-fb:96 nack pli',
  'a=ssrc:1196838968 cname:fixed-source', 'a=ssrc:1196838968 msid:source video', ''
].join('\r\n') };
const caps = { codecs: [{ kind: 'video', mimeType: 'video/VP8', preferredPayloadType: 101, clockRate: 90000, parameters: {}, rtcpFeedback: [{ type: 'nack', parameter: '' }, { type: 'nack', parameter: 'pli' }] }], headerExtensions: [] };
const transport = { iceParameters: { usernameFragment: 'mediasoup', password: 'remote-test-password-123456', iceLite: true }, iceCandidates: [{ foundation: 'udp', priority: 1, ip: '127.0.0.1', address: '127.0.0.1', protocol: 'udp', port: 11001, type: 'host' }], dtlsParameters: { role: 'auto', fingerprints: [{ algorithm: 'sha-256', value: Array(32).fill('CD').join(':') }] } };

test('official helpers map native VP8/SSRC onto a normal DTLS/SRTP transport', () => {
  const before = JSON.stringify([offer, caps, transport]);
  const result = mediasoupNativeAnswer(offer, caps, transport);
  assert.equal(result.dtlsParameters.role, 'client');
  assert.equal(result.rtpParameters.codecs[0].payloadType, 96);
  assert.equal(result.rtpParameters.encodings[0].ssrc, 0x47565038);
  assert.match(result.description.sdp, /a=setup:passive\r\n/);
  assert.match(result.description.sdp, /a=recvonly\r\n/);
  assert.match(result.description.sdp, /a=ice-lite\r\n/);
  assert.match(result.description.sdp, /a=rtpmap:96 VP8\/90000\r\n/);
  assert.equal(JSON.stringify([offer, caps, transport]), before);
});
test('different source topology, SSRC, codec or a PlainTransport cannot silently qualify', () => {
  for (const changed of [offer.sdp.replace('a=sendonly', 'a=recvonly'), offer.sdp.replaceAll('1196838968', '42'), offer.sdp.replace('VP8/90000', 'VP9/90000'), offer.sdp + 'm=audio 9 UDP/TLS/RTP/SAVPF 111\r\n']) {
    assert.throws(() => mediasoupNativeAnswer({ ...offer, sdp: changed }, caps, transport));
  }
  assert.throws(() => mediasoupNativeAnswer(offer, caps, { ip: '127.0.0.1', port: 11001 }));
});
