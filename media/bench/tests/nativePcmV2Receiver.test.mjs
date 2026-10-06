import test from 'node:test';
import assert from 'node:assert/strict';
import { QUALITY_COUNTERS, RECEIVER_COUNTERS, v2Publication, holdTrack, validateHeldObjects, inboundSnapshot, receiverEvidence } from '../native-pcm-v2-receiver.mjs';
import { validateV2Preflight } from '../native-pcm-v2-control.mjs';
import { NATIVE_PCM_POLICY as P } from '../native-pcm-policy.mjs';

// Synthetic receiver/SDP/stat fixtures; no browser, PCM or RTP is executed here.
const tracks = [['audio', 'm', 'fixed-native-mic', 0x474d4943], ['video', 's', 'fixed-native-video', 0x47565038], ['audio', 's', 'fixed-native-source-audio', 0x47534130]];
function offer() {
  return { type: 'offer', sdp: 'v=0\r\n' + tracks.map(([kind, stream, track, ssrc], mid) => `m=${kind} 9 UDP/TLS/RTP/SAVPF ${kind === 'audio' ? 111 : 96}\r\na=mid:${mid}\r\na=sendrecv\r\na=msid:${stream} ${track}\r\na=ssrc:${ssrc} msid:${stream} ${track}\r\n${kind === 'audio' ? 'a=rtpmap:111 opus/48000/2' : 'a=rtpmap:96 VP8/90000'}\r\n`).join('') };
}
function graph() {
  const bindings = v2Publication(offer()), held = new Map(), receivers = [], transceivers = [];
  for (const binding of bindings) {
    const track = { id: binding.track_id, kind: binding.kind, readyState: 'live', enabled: true }, receiver = { track }, transceiver = { receiver, mid: binding.mid, currentDirection: 'recvonly' }, event = { track, receiver, transceiver, streams: [{ id: binding.stream_id }] };
    holdTrack(event, bindings, held); receivers.push(receiver); transceivers.push(transceiver);
  }
  const pc = { connectionState: 'connected', getReceivers: () => receivers, getTransceivers: () => transceivers, getSenders: () => receivers.map(() => ({ track: null })) };
  return { bindings, held, receivers, transceivers, pc };
}
function stats(binding, packets = 0) {
  return [{ type: 'codec', id: 'codec-' + binding.role, mimeType: 'audio/opus', clockRate: 48000, channels: 2 }, { type: 'inbound-rtp', kind: 'audio', id: 'inbound-' + binding.role, ssrc: binding.ssrc, mid: binding.mid, trackIdentifier: binding.track_id, codecId: 'codec-' + binding.role, timestamp: 1000 + packets, packetsReceived: packets, totalSamplesReceived: packets * 960, ...Object.fromEntries(QUALITY_COUNTERS.map(field => [field, 0])) }];
}
function preflight() {
  const g = graph(), initial = g.bindings.filter(binding => binding.kind === 'audio').map(binding => inboundSnapshot(binding, stats(binding), { initial: true }));
  const prepared = { ready: true, offer: offer(), contextState: 'running', sampleRate: 48000, connection: 'connected', graph: { receivers: 3, held: g.bindings.map(binding => ({ ...binding, live: true, enabled: true })) }, initial };
  const native = { sources: {}, peers: { publish: { connection: 'connected', negotiated_senders: g.bindings.map(binding => ({ mid: binding.mid, track_id: binding.track_id, stream_id: binding.stream_id, encodings: [{ ssrc: binding.ssrc, active: true }] })) } } };
  const clock = { browser: { revision: P.chromiumRevision, product: 'HeadlessChrome/' + P.chromiumVersion }, crossOriginIsolated: true, precision: { samples: 100000, minimumStepMs: .005 } };
  return { prepared, native, clock };
}

test('complete direct source offer binds separate actual MID/MSID/SSRC/Opus roles', () => {
  assert.deepEqual(v2Publication(offer()).map(binding => binding.role), ['mic', 'video', 'source']);
  for (const change of [value => value.type = 'answer', value => value.sdp = value.sdp.replace('a=mid:2', 'a=mid:1'), value => value.sdp = value.sdp.replaceAll('fixed-native-source-audio', 'fixed-native-mic'), value => value.sdp = value.sdp.replace('opus/48000/2', 'opus/48000/1'), value => value.sdp = value.sdp.replace('m=audio 9 ', 'm=audio 0 '), value => value.sdp = value.sdp.replace('a=sendrecv', 'a=recvonly'), value => value.sdp += 'a=ssrc:9 cname:foreign\r\n']) { const value = offer(); change(value); assert.throws(() => v2Publication(value)); }
});
test('held graph requires same live enabled receiver/transceiver objects and no local publication', () => {
  assert.throws(() => validateHeldObjects(graph().pc, graph().held), /held receiver object/);
});
test('actual object replacement, duplicate events, disabled/ended tracks and incomplete graph reject', () => {
  const valid = graph(); assert.doesNotThrow(() => validateHeldObjects(valid.pc, valid.held));
  for (const change of [g => g.pc.connectionState = 'disconnected', g => g.receivers.pop(), g => g.transceivers[0] = { ...g.transceivers[0] }, g => g.held.get('mic').receiver.track = { ...g.held.get('mic').track }, g => g.held.get('source').track.enabled = false, g => g.held.get('source').track.readyState = 'ended', g => g.transceivers[0].mid = 'foreign', g => g.transceivers[0].currentDirection = 'inactive', g => g.pc.getSenders = () => [{ track: { id: 'published' } }]]) { const g = graph(); change(g); assert.throws(() => validateHeldObjects(g.pc, g.held)); }
  const g = graph(), binding = g.held.get('mic'); assert.throws(() => holdTrack({ track: binding.track, receiver: binding.receiver, transceiver: binding.transceiver, streams: [binding.stream] }, g.bindings, g.held), /binding differs/);
});
test('genuine zero initial report is mandatory and absent stats never become zero', () => {
  const binding = v2Publication(offer())[0]; assert.doesNotThrow(() => inboundSnapshot(binding, stats(binding), { initial: true }));
  assert.throws(() => inboundSnapshot(binding, []), /unavailable or ambiguous/);
  for (const field of RECEIVER_COUNTERS) for (const value of [undefined, null, '0', 1, -1, .5, NaN, Infinity]) { const values = stats(binding); values[1][field] = value; assert.throws(() => inboundSnapshot(binding, values, { initial: true }), field); }
  assert.throws(() => inboundSnapshot(binding, stats(binding, 1), { initial: true }), /initial receiver counters/);
});
test('held inbound identity, decoder codec, quality counters and monotonic totals stay strict', () => {
  const binding = v2Publication(offer())[0], initial = inboundSnapshot(binding, stats(binding), { initial: true });
  const first = inboundSnapshot(binding, stats(binding, 10), { previous: initial });
  for (const change of [values => values[1].id = 'other', values => values[1].ssrc++, values => values[1].mid = 'other', values => values[1].trackIdentifier = 'other', values => values[0].mimeType = 'audio/PCMU', values => values[0].channels = 1, values => values[1].codecId = 'other', values => values[1].timestamp = 1, values => values[1].packetsReceived = 9, values => values[1].totalSamplesReceived = 9 * 960, ...QUALITY_COUNTERS.map(field => values => values[1][field] = 1)]) { const values = stats(binding, 11); change(values); assert.throws(() => inboundSnapshot(binding, values, { previous: first })); }
  const final = inboundSnapshot(binding, stats(binding, 1050), { previous: first }), evidence = receiverEvidence(initial, final);
  assert.equal(evidence.packetsReceived, 1050); assert.equal(evidence.totalSamplesReceived, 1008000); assert.equal(evidence.initialPacketsDiscarded, 0); assert.equal(evidence.initialConcealedSamples, 0); assert.equal(evidence.initialInboundReportId, evidence.inboundReportId);
});
test('Node preflight accepts only actual retained zero tables, full native senders and unchanged captured offer', () => {
  const good = preflight(); assert.equal(validateV2Preflight(good.prepared, good.native, offer(), good.clock).length, 3);
  for (const change of [v => v.prepared.ready = false, v => v.prepared.initial.pop(), v => v.prepared.initial[0].row.packetsReceived = 1, v => v.prepared.initial[0].stats[1].packetsReceived = 1, v => v.prepared.graph.held[1].live = false, v => v.prepared.graph.held[2].mid = '0', v => v.native.peers.other = {}, v => v.native.sources.mic = { running: true }, v => v.native.peers.publish.connection = 'connecting', v => v.native.peers.publish.negotiated_senders[0].encodings[0].ssrc++, v => v.native.peers.publish.negotiated_senders.pop(), v => v.clock.crossOriginIsolated = false, v => v.prepared.offer.sdp += 'a=other:value\r\n']) { const v = preflight(); change(v); assert.throws(() => validateV2Preflight(v.prepared, v.native, offer(), v.clock)); }
});
