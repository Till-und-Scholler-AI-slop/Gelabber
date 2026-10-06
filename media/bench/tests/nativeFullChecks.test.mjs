import test from 'node:test';
import assert from 'node:assert/strict';
import { nativeFullTopology, nativeFullGraph } from '../native-full-checks.mjs';

function control(n = 2) {
  const topology = nativeFullTopology(n), bindings = Array.from({ length: n - 1 }, (_, index) => ({ source_name: `peer-${index + 1}/mic`, ssrc: 100 + index }));
  const samples = [0, 10, 20, 30].map(seconds => {
    const stats = [], nativeRows = {};
    for (const [index, edge] of topology.edges.entries()) {
      if (edge.receiver === 'peer-0') {
        const ssrc = bindings.find(value => value.source_name === edge.source).ssrc;
        nativeRows[ssrc] = { ssrc, source_name: edge.source, track_id: 'native-' + index, stream_ids: [edge.source],
          packets_received: 50 * (seconds + 1), payload_bytes_received: 16000 * (seconds + 1), decoded_samples: 48000 * (seconds + 1),
          codec: { mimeType: 'audio/opus', clockRate: 48000, payloadType: 111 }, sequence_gaps: 0, reordered_or_duplicate_packets: 0, timestamp_gaps: 0, decode_errors: 0 };
        continue;
      }
      const endpoint = edge.receiver + '/recv', codecId = 'codec-' + edge.kind, ssrc = 500 + index;
      stats.push({ type: 'codec', id: codecId, mimeType: edge.kind === 'audio' ? 'audio/opus' : 'video/VP8', payloadType: edge.kind === 'audio' ? 111 : 96, clockRate: edge.kind === 'audio' ? 48000 : 90000, _endpoint: endpoint });
      stats.push({ type: 'inbound-rtp', id: 'edge-' + index, kind: edge.kind, codecId, ssrc, timestamp: seconds * 1000,
        packetsReceived: 50 * (seconds + 1), bytesReceived: (edge.kind === 'audio' ? 16000 : 500000) * (seconds + 1),
        totalSamplesReceived: 48000 * (seconds + 1), packetsLost: 0, concealedSamples: 0, framesDecoded: 60 * (seconds + 1), frameWidth: 1920, frameHeight: 1080,
        _peer: edge.receiver, _endpoint: endpoint, _source: edge.source,
        _binding: { kind: edge.kind, source_name: edge.source, ssrc, producer_id: 'producer-' + index, consumer_id: 'consumer-' + index, track_id: 'track-' + index, binding_basis: 'actual worker producer consumer' } });
    }
    for (let index = 1; index < n; index++) { stats.push({ type: 'codec', id: 'send-audio', mimeType: 'audio/opus', clockRate: 48000, payloadType: 111, _endpoint: 'peer-' + index + '/send' }); stats.push({ type: 'outbound-rtp', kind: 'audio', id: 'sender-' + index, ssrc: 100 + index - 1, timestamp: seconds * 1000,
      packetsSent: 50 * (seconds + 1), bytesSent: 16000 * (seconds + 1), codecId: 'send-audio', _endpoint: 'peer-' + index + '/send', _peer: 'peer-' + index }); }
    return { browser: { stats, failures: [], connections: Array.from({ length: n - 1 }, (_, index) => ({ peer: 'peer-' + (index + 1), states: [{ label: 'send', state: 'connected' }, { label: 'recv', state: 'connected' }] })) },
      native: { clock: 'CLOCK_MONOTONIC', monoNs: String((seconds + 1) * 1e9), sources: Object.fromEntries(['mic', 'source', 'video'].map(name => [name, { source_policy_valid: true, timeline: { clock: 'CLOCK_MONOTONIC', startClockNs: '1', conversionBracketNs: 1000, pcm_latency_calibrated: false } }])),
        peers: { receive: { connection: 'connected', received: nativeRows }, publish: { connection: 'connected', negotiated_senders: [[0x474d4943, 'audio', 'm', 'fixed-native-mic', '0'], [0x47565038, 'video', 's', 'fixed-native-video', '1'], [0x47534130, 'audio', 's', 'fixed-native-source-audio', '2']].map(([ssrc, kind, stream_id, track_id, mid]) => ({ mid, stream_id, track_id, basis: 'actual RtpSender parameters', encodings: [{ ssrc, active: true, mimeType: kind === 'audio' ? 'audio/opus' : 'video/VP8', clockRate: kind === 'audio' ? 48000 : 90000 }], codecs: [{ mimeType: kind === 'audio' ? 'audio/opus' : 'video/VP8', clockRate: kind === 'audio' ? 48000 : 90000, payloadType: kind === 'audio' ? 111 : 96 }] })), codecs: ['audio', 'video'].map(kind => ({ type: 'codec', id: 'native-' + kind, mimeType: kind === 'audio' ? 'audio/opus' : 'video/VP8', clockRate: kind === 'audio' ? 48000 : 90000, payloadType: kind === 'audio' ? 111 : 96 })), outbound: [[0x474d4943, 'audio'], [0x47565038, 'video'], [0x47534130, 'audio']].map(([ssrc, kind]) => ({ id: 'sender-' + ssrc, ssrc, kind, codecId: 'native-' + kind, mid: ssrc === 0x474d4943 ? '0' : ssrc === 0x47565038 ? '1' : '2', packetsSent: 50 * (seconds + 1), bytesSent: (kind === 'audio' ? 16000 : 500000) * (seconds + 1) })) } } } };
  });
  return { topology, bindings, samples };
}
const check = value => nativeFullGraph(value.samples, value.topology, value.bindings, 'receive', 4000000, { engine: 'mediasoup', requestedSeconds: 30 });
test('N2/N8 contract includes all N-1 video watchers and every microphone/source-audio edge', () => {
  for (const n of [2, 8, 16, 32]) {
    const value = control(n), result = check(value);
    assert.equal(value.topology.video_watchers, n - 1); assert.equal(value.topology.audio_edges, n * n - 1);
    assert.equal(result.valid, true, result.failures.join('; ')); assert.equal(result.streams.length, n * n + n - 2);
    assert.equal(result.audio_received_bitrate_distribution.min, 128000); assert.equal(result.video_decoder_fps_distribution.min, 60);
  }
});
test('aggregate count cannot hide a displaced/duplicated per-peer graph edge', () => {
  const value = control(8);
  for (const sample of value.samples) sample.browser.stats.find(row => row.type === 'inbound-rtp' && row._peer === 'peer-1')._peer = 'peer-2';
  assert.equal(check(value).valid, false);
});
test('annotation without actual receive binding cannot qualify an edge', () => {
  for (const mutate of [row => { row._binding.ssrc++; }, row => { delete row._binding; }, row => { row._binding.source_name = 'peer-31/mic'; }]) {
    const value = control(); mutate(value.samples[1].browser.stats.find(row => row.type === 'inbound-rtp'));
    assert.equal(check(value).valid, false);
  }
});
test('native unbound/duplicate SSRC, browser decoder loss/stall, FPS, source failure and sender rates fail', () => {
  const mutations = [value => { value.samples[1].native.peers.receive.received[100].source_name = null; },
    value => { value.samples[1].native.peers.receive.received[100].sequence_gaps = 1; },
    value => { value.samples[1].native.sources.video.source_policy_valid = false; },
    value => { value.samples[1].browser.stats.find(row => row.type === 'inbound-rtp').concealedSamples = 960; },
    value => { value.samples[1].browser.stats.find(row => row.type === 'inbound-rtp').totalSamplesReceived = 48000; },
    value => { value.samples[3].browser.stats.find(row => row.type === 'inbound-rtp' && row.kind === 'video').framesDecoded = 130; },
    value => { value.samples[3].browser.stats.find(row => row.type === 'outbound-rtp').bytesSent = 48000; }];
  for (const mutate of mutations) { const value = control(); mutate(value); assert.equal(check(value).valid, false); }
});

test('strict clock/policy and exact per-peer transport inventories cannot be forged by graph rows', () => {
  for (const mutate of [v => { v.samples[1].browser.connections = []; },
    v => { v.samples[1].browser.connections[0].peer = 'peer-31'; },
    v => { v.samples[1].browser.connections[0].states = [{ label: 'send', state: 'connected' }]; },
    v => { v.samples[1].native.peers.publish.connection = 'disconnected'; },
    v => { v.samples[1].native.sources.mic.source_policy_valid = 'false'; },
    v => { for (const row of Object.values(v.samples[1].native.sources)) delete row.timeline; },
    v => { v.samples[1].native.sources.mic.timeline.startClockNs = '00'; },
    v => { v.samples[1].native.sources.mic.timeline.conversionBracketNs = 100001; },
    v => { v.samples[1].native.monoNs = 'invalid'; },
    v => { v.samples[1].native.monoNs = v.samples[0].native.monoNs; },
    v => { for (const row of v.samples[3].browser.stats) row.timestamp = 10000; },
    v => { v.samples[1].native.peers.publish.outbound[0].kind = 'video'; }, v => { v.samples[1].native.peers.publish.codecs[0].clockRate = 8000; }, v => { v.samples[1].native.peers.receive.received[100].codec.payloadType = undefined; }]) {
    const value = control(); mutate(value); assert.equal(check(value).valid, false);
  }
});

test('all engines require actual negotiated sender MID/SSRC/codec; optional CodecStats must agree', () => {
  const value = control();
  for (const sample of value.samples) for (const row of sample.native.peers.publish.outbound) if (row.kind === 'audio') row.codecId = '';
  assert.equal(check(value).valid, true);
  for (const mutate of [v => { delete v.samples[1].native.peers.publish.negotiated_senders; },
    v => { v.samples[1].native.peers.publish.negotiated_senders[0].encodings[0].ssrc++; },
    v => { v.samples[1].native.peers.publish.negotiated_senders[0].mid = '2'; },
    v => { v.samples[1].native.peers.publish.negotiated_senders[0].track_id = 'fixed-native-source-audio'; },
    v => { v.samples[1].native.peers.publish.negotiated_senders[0].codecs[0].clockRate = 8000; },
    v => { v.samples[1].native.peers.publish.negotiated_senders[0].encodings[0].mimeType = 'audio/PCMU'; }]) {
    const bad = structuredClone(value); mutate(bad); assert.equal(check(bad).valid, false);
  }
});

function withQuality(value = control()) {
  for (const sample of value.samples) for (const row of sample.browser.stats.filter(row => row.type === 'inbound-rtp')) {
    const emitted = row.kind === 'audio' ? row.totalSamplesReceived : row.framesDecoded;
    Object.assign(row, { packetsDiscarded: 0, jitterBufferEmittedCount: emitted,
      jitterBufferDelay: emitted * .02, jitterBufferTargetDelay: emitted * .025, jitterBufferMinimumDelay: emitted * .015 });
    if (row.kind === 'audio') Object.assign(row, { silentConcealedSamples: 0, concealmentEvents: 0,
      insertedSamplesForDeceleration: 0, removedSamplesForAcceleration: 0 });
    else Object.assign(row, { framesDropped: 0, freezeCount: 0, pauseCount: 0, totalFreezesDuration: 0, totalPausesDuration: 0 });
  }
  return value;
}
test('nonzero PLC is an observation against the baseline while the historical strict pilot remains FAIL', () => {
  const value = withQuality();
  for (const sample of value.samples.slice(1)) {
    const row = sample.browser.stats.find(row => row.type === 'inbound-rtp' && row.kind === 'audio');
    Object.assign(row, { concealedSamples: 514, silentConcealedSamples: 10, concealmentEvents: 1,
      insertedSamplesForDeceleration: 8, removedSamplesForAcceleration: 4 });
  }
  const result = check(value), quality = result.quality.edges.find(edge => edge.receiver === 'peer-1' && edge.source === 'peer-0/mic');
  assert.equal(result.valid, false); assert.equal(result.source_graph.valid, true);
  assert.equal(result.quality.complete, true); assert.equal(result.source_graph.measurement_comparable, true);
  assert.equal(quality.counters.concealedSamples.delta, 514); assert.equal(quality.counters.silentConcealedSamples.delta, 10);
  assert.equal(quality.counters.concealmentEvents.delta, 1); assert.equal(quality.counters.insertedSamplesForDeceleration.delta, 8);
  assert.equal(quality.counters.removedSamplesForAcceleration.delta, 4); assert.equal(quality.sample_rate, 48000);
  assert.equal(quality.nonconcealed_sample_rate, (1440000 - 514) / 30);
  assert.equal(result.comparison_available, false); assert.equal(result.quality.comparison_available, false);
});
test('interval jitter-buffer means exclude warmup counters and native decode does not invent browser PLC', () => {
  const value = withQuality();
  for (const sample of value.samples) for (const row of sample.browser.stats.filter(row => row.type === 'inbound-rtp')) row.jitterBufferDelay += 1000;
  const result = check(value), audio = result.quality.edges.find(edge => edge.receiver === 'peer-1' && edge.kind === 'audio');
  assert.equal(audio.jitter_buffer_mean_seconds.actual, .02); assert.equal(audio.jitter_buffer_mean_seconds.target, .025);
  assert.equal(audio.jitter_buffer_mean_seconds.minimum, .015);
  const native = result.quality.edges.find(edge => edge.receiver === 'peer-0');
  assert.equal(native.counters.concealedSamples, undefined); assert.equal(native.jitter_buffer_mean_seconds, null);
  assert.equal(native.sample_rate, 48000); assert.match(native.scope, /no browser/);
});
test('missing, malformed, reset or impossible quality counters never become zero or a complete comparison', () => {
  for (const mutate of [row => { delete row.silentConcealedSamples; }, row => { row.concealmentEvents = NaN; },
    row => { row.jitterBufferDelay = -1; }, row => { row.silentConcealedSamples = 1; }, row => { row.jitterBufferTargetDelay = 0; }]) {
    const value = withQuality(); mutate(value.samples[1].browser.stats.find(row => row.type === 'inbound-rtp' && row.kind === 'audio'));
    const result = check(value);
    assert.equal(result.quality.complete, false); assert.equal(result.source_graph.measurement_comparable, false);
    assert.equal(result.comparison_available, false);
  }
});
test('record real stalls and signed loss correction independently of source schedule/inventory qualification', () => {
  const value = withQuality();
  const video = sample => sample.browser.stats.find(row => row.type === 'inbound-rtp' && row.kind === 'video');
  video(value.samples[1]).framesDecoded = video(value.samples[0]).framesDecoded;
  for (const sample of value.samples) video(sample).packetsLost = 10;
  video(value.samples[3]).packetsLost = 8; // late received packets may reduce WebRTC cumulative loss
  const result = check(value), quality = result.quality.edges.find(edge => edge.kind === 'video');
  assert.equal(result.valid, false); assert.equal(result.source_graph.valid, true); assert.equal(result.quality.complete, true);
  assert.deepEqual(quality.decoder_stalls, [{ interval: 0, seconds: 10 }]); assert.equal(quality.counters.packetsLost.delta, -2);
  assert.equal(quality.decoded_fps, 60);
});
test('missing edges, unequal native schedules, extra source inventory and changed sender bitrate invalidate source comparability', () => {
  for (const mutate of [v => { v.samples[1].browser.stats = v.samples[1].browser.stats.filter(row => !(row.type === 'inbound-rtp' && row.kind === 'video')); },
    v => { for (const source of Object.values(v.samples[1].native.sources)) source.timeline.startClockNs = '2'; },
    v => { v.samples[1].native.sources.extra = structuredClone(v.samples[1].native.sources.mic); },
    v => { v.samples[3].browser.stats.find(row => row.type === 'outbound-rtp').bytesSent *= .5; },
    v => { v.samples[1].browser.stats.find(row => row.type === 'inbound-rtp').ssrc++; },
    v => { delete v.samples[1].browser.stats.find(row => row.type === 'inbound-rtp').bytesReceived; },
    v => { v.topology.edges.push(structuredClone(v.topology.edges[0])); },
    v => { v.samples[1].native.peers.publish.negotiated_senders[0].encodings[0].active = false; }]) {
    const value = withQuality(); mutate(value); const result = check(value);
    assert.equal(result.source_graph.valid, false); assert.equal(result.source_graph.measurement_comparable, false);
  }
});
