import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { uniqueJson, canonical, expectedBook, inspectReplayArchive, readReplayPair, replayRequest, validateReplayRuntime, qualifyReplayPair } from '../native-pcm-replay-contract.mjs';
import { NATIVE_PCM_POLICY as P } from '../native-pcm-policy.mjs';

// Contract fixtures only: these bytes/evidence are synthetic, not decoded Opus
// or executed native/browser controls. Real import must decode independently.
const hash = data => createHash('sha256').update(data).digest('hex');
const runId = '6f090a38-31c4-45b5-9c62-c711a3d5e5da';
const provenance = { library_sha256: 'a'.repeat(64), script_sha256: 'b'.repeat(64), fixed_opus_script_sha256: 'c'.repeat(64), pcm_kernel_sha256: 'd'.repeat(64), pn_inspector_sha256: 'e'.repeat(64), node_sha256: P.nodeSha256 };
function manifest(role = 'mic', seconds = 20, id = runId) {
  const book = expectedBook(seconds, id), source = book.shared.sources.find(v => v.kind === role), count = (seconds + 1) * 50;
  return { schema: 2, codec: 'opus', sample_rate_hz: 48000, channels: 1, packet_duration_ms: 20, packets: count, measurement_seconds: seconds, duration_seconds: seconds + 1, tail_samples: 48000, tail_seconds: 1, measurement_end_sample_ordinal: seconds * 48000, rtp_clock_hz: 48000, rtp_timestamp_step: 960, payload_bitrate_bps: 128000, encoded_bytes: count * 320, encoded_packets_sha256: hash(Buffer.alloc(count * 320, 0x55)), run_id: id,
    loop_policy: 'unlooped; stop at archive end; no rewind or modulo', comparison_available: false, pcm_latency_calibrated: false,
    pcm: { kind: role, source_uid: source.source_uid, samples: (seconds + 1) * 48000, frequencies_hz: role === 'mic' ? [317, 719, 1249, 2027] : [440], gain_per_tone: role === 'mic' ? .07 : .45, format: 'float32le mono', sha256: 'f'.repeat(64), peak: .8 },
    decode_control: { float32le_sha256: '1'.repeat(64), samples: (seconds + 1) * 48000, peak: .85, input_correlation_after_codec_lookahead: .999 },
    encoder: { application: 'audio', application_constant: 2049, lookahead_samples: 312, lookahead_ms: 6.5, settings_readback: { bitrate: 128000, complexity: 10, dtx: 0, inband_fec: 1, packet_loss_percent: 1, vbr: 0 } },
    pn: { run_id: id, codebook_sha256: book.sha256, source_uid: source.source_uid, ssrc: source.ssrc, marker_policy: book.shared.marker_policy, markers: source.markers, shared_run: book.shared },
    marker_control: { samples: (seconds + 1) * 48000, markers: source.markers.length, max_alignment_error_samples: 4, executed_node: { version: P.nodeVersion, sha256: P.nodeSha256 }, checks: source.markers.map(marker => ({ sequence: marker.sequence, expected_decoded_sample_ordinal: marker.source_sample_ordinal + 312, actual_decoded_sample_ordinal: marker.source_sample_ordinal + 316, residual_samples: 4, score: .97, amplitude: .35 })) },
    provenance: { ...provenance, library_version: 'libopus 1.6.1' }
  };
}
function archive(meta = manifest(), rawHeader = JSON.stringify(meta)) {
  const header = Buffer.from(rawHeader), prefix = Buffer.alloc(12); prefix.write('GPOPUS2\n'); prefix.writeUInt32BE(header.length, 8);
  const packets = Array.from({ length: meta.packets }, (_, index) => { const packet = Buffer.alloc(332, 0x55); packet.writeBigUInt64BE(BigInt(index) * 20000000n); packet.writeUInt32BE(320, 8); return packet; });
  return Buffer.concat([prefix, header, ...packets]);
}
function inspect(bytes, role = 'mic', p = provenance) { return inspectReplayArchive(bytes, { role, sha256: hash(bytes), provenance: p }); }
function withPair(fn, change = () => {}) {
  const folder = fs.mkdtempSync(path.join(os.tmpdir(), 'gelabber-replay-contract-'));
  try {
    const mic = manifest(), source = manifest('source'); change(mic, source);
    const inputs = {};
    for (const [role, metadata] of Object.entries({ mic, source })) { const bytes = archive(metadata), file = path.join(folder, role + '.opusbin'); fs.writeFileSync(file, bytes); inputs[role] = { path: file, sha256: hash(bytes) }; }
    return fn(readReplayPair({ ...inputs, provenance }));
  } finally { fs.rmSync(folder, { recursive: true, force: true }); }
}
function runtime(pair, hold = 0) {
  const timeline = { clock: 'CLOCK_MONOTONIC', startClockNs: '1000000000', conversionBracketNs: 20, pcm_latency_calibrated: false };
  const summary = { measurement_seconds: pair.measurement_seconds, total_seconds: pair.total_seconds, measurement_end_sample_ordinal: pair.measurement_end_sample_ordinal, tail_samples: pair.tail_samples, timeline, test_hold_enabled: true, audio_hold_ms: hold, comparison_available: false, pcm_latency_calibrated: false };
  const actual = { binary_sha256: '9'.repeat(64), decoder: { version: 'libopus 1.6.1', sha256: provenance.library_sha256 }, comparison_available: false, pcm_latency_calibrated: false };
  const sources = {};
  for (const input of pair.archives) {
    actual[input.role] = { archive_sha256: input.archive_sha256, metadata: input.metadata, import_verified: true };
    const count = input.metadata.packets, due = 1000000000n + BigInt(count - 1) * 20000000n, delay = BigInt(hold) * 1000000n;
    sources[input.role] = { source_uid: input.uid, ssrc: input.ssrc, completed: true, running: false, source_policy_valid: true, end_reached: true, expected_packet_count: count, packets_enqueued: count, last_source_sample_ordinal: (count - 1) * 960, timeline,
      last_planned_mono_ns: String(due), last_enqueue_before_ns: String(due + delay + 1000000n), last_enqueued_mono_ns: String(due + delay + 1100000n), last_enqueue_bracket_ns: '100000', max_schedule_lateness_ns: '1100000', test_hold_enabled: true, audio_hold_ms: hold, hold_applied_packets: count, min_actual_hold_ns: String(delay + 1000000n), max_actual_hold_ns: String(delay + 1100000n) };
  }
  return structuredClone({ greeting: { ready: true, provenance: actual }, start: summary, status: { ...summary, sources }, nativeBinarySha256: actual.binary_sha256, audioHoldMs: hold });
}
function evidence(pair, hold = 0) {
  const rows = Array.from({ length: Math.ceil((pair.total_seconds * 48000 + 5000) / 128) }, (_, i) => ({ sequence: i + 1, firstFrame: i * 128, frames: 128, inputFrames: 128, flags: 0, lowerMs: 100 + i * 128 / 48, upperMs: 101 + (i + 1) * 128 / 48 }));
  const clocks = Array.from({ length: pair.total_seconds * 200 + 50 }, (_, i) => ({ p0: i * 5, p1: i * 5 + .5, monoNs: String(1000000000n + BigInt(i) * 5000000n) }));
  return { pair, runtime: runtime(pair, hold), observer: { failures: [], missing: 0, rows: [rows, rows], clocks }, contextStates: ['running'],
    browserClock: { browser: { revision: P.chromiumRevision, product: 'HeadlessChrome/' + P.chromiumVersion }, crossOriginIsolated: true, precision: { samples: 100000, minimumStepMs: .005 }, chromium_sha256: P.chromiumSha256, node_sha256: P.nodeSha256 },
    groups: pair.archives.map(input => ({ role: input.role, uid: input.uid, archive_sha256: input.archive_sha256, run_id: pair.run_id, codebook_sha256: pair.codebook_sha256,
      tap: { uid: input.uid, sampleRate: 48000, gaps: 0, excessPeaks: 0, clipped: 0, nonfinite: 0, peaks: input.metadata.pn.markers.map(marker => ({ sequence: marker.sequence, receivedFrame: marker.source_sample_ordinal + 4800, score: .97, amplitude: .35 })) },
      receiver: { uid: input.uid, role: input.role, ssrc: input.ssrc, identityStable: true, live: true, enabled: true, codec: 'audio/opus', decodedSamplesProgress: true, packetsProgress: true, packetsLost: 0, concealedSamples: 0, silentConcealedSamples: 0,
        initialInboundReportId: 'inbound-' + input.role, inboundReportId: 'inbound-' + input.role, initialPacketsReceived: 0, initialTotalSamplesReceived: 0, packetsReceived: input.metadata.packets, totalSamplesReceived: input.metadata.decode_control.samples, packetsDiscarded: 0, insertedSamplesForDeceleration: 0, removedSamplesForAcceleration: 0 } })) };
}

test('unique JSON rejects duplicate escaped keys and prototype keys do not mutate objects', () => {
  assert.throws(() => uniqueJson('{"run_id":1,"\\u0072un_id":2}'), /duplicate/);
  assert.throws(() => uniqueJson('{"a":{"uid":0,"uid":64}}'), /duplicate/);
  assert.equal(uniqueJson('{"__proto__":{"x":1}}').__proto__.x, 1); assert.equal({}.x, undefined);
  for (const raw of ['{"x":1,}', '[1,]', '{"x":Infinity}', '[01]', '[true false]', '1e999', '"bad\nstring"']) assert.throws(() => uniqueJson(raw));
  assert.throws(() => uniqueJson('['.repeat(18) + '0' + ']'.repeat(18)), /bound/);
  assert.throws(() => uniqueJson(' '.repeat(262145)), /bound/);
});
test('complete book matches independent Python240-second finite book and duration/tail', () => {
  const book = expectedBook(240, runId);
  assert.equal(book.sha256, '91ed1302f9887369e936186168cab9a458c5fbc75d9e142b94a8a7010605c91d');
  assert.equal(book.shared.sources[0].markers.length, 120); assert.equal(book.shared.tail_samples, 48000);
  assert.equal(expectedBook(21, runId).shared.sources[1].markers.length, 10);
  for (const seconds of [19, 361, 20.5, '20']) assert.throws(() => expectedBook(seconds, runId));
});
test('full archive framing/hash and both shared finite books import immutably', () => {
  assert.equal(inspect(archive()).metadata.packets, 1050);
  withPair(pair => { assert.equal(pair.archives.length, 2); assert.equal(pair.total_seconds, 21); assert.equal(pair.comparison_available, false); assert.throws(() => { pair.archives[0].metadata.pn.markers.pop(); }); assert.throws(() => { pair.provenance.library_sha256 = '0'.repeat(64); }); });
});
test('partial book stays forbidden even when the attacker recomputes its shared hash', () => {
  const meta = structuredClone(manifest()); meta.pn.markers.pop(); meta.pn.shared_run.sources.forEach(source => source.markers.pop()); meta.pn.codebook_sha256 = hash(canonical(meta.pn.shared_run));
  assert.throws(() => inspect(archive(meta)), /complete role/);
});
test('foreign UID, roleSSRC, shared policy and marker code are rejected at actual header', () => {
  for (const change of [m => m.pn.source_uid = 64, m => m.pn.ssrc = 17, m => m.pn.markers[0].code[0] *= -1, m => m.pn.shared_run.marker_policy.amplitude = .36, m => m.pn.markers.push(m.pn.markers[0])]) {
    const meta = structuredClone(manifest()); change(meta); assert.throws(() => inspect(archive(meta)));
  }
});
test('different run or whole measurement duration cannot form a pair', () => {
  assert.throws(() => withPair(() => {}, (_, source) => Object.assign(source, manifest('source', 20, '11111111-1111-1111-1111-111111111111'))), /shared run/);
  assert.throws(() => withPair(() => {}, (_, source) => Object.assign(source, manifest('source', 21))), /shared run/);
});
test('missing tail, gain/clipping, wrong actual lookahead and partial decode claims reject', () => {
  for (const change of [m => m.tail_samples = 0, m => m.tail_seconds = 0, m => m.duration_seconds--, m => m.pcm.gain_per_tone = 1, m => m.pcm.peak = 1, m => m.decode_control.peak = .999, m => m.decode_control.samples--, m => m.decode_control.input_correlation_after_codec_lookahead = 2, m => m.encoder.lookahead_samples = 0, m => m.marker_control.checks.pop(), m => m.marker_control.checks[0].actual_decoded_sample_ordinal++, m => m.comparison_available = 'false']) {
    const meta = structuredClone(manifest()); change(meta); assert.throws(() => inspect(archive(meta)));
  }
});
test('duplicate header key, changed bytes/hash, truncated tail and packet modulo/extra fail', () => {
  const meta = manifest(), good = archive(meta), head = JSON.stringify(meta).replace('{', '{"schema":1,');
  assert.throws(() => inspect(archive(meta, head)), /duplicate/);
  assert.throws(() => inspectReplayArchive(good, { role: 'mic', sha256: '0'.repeat(64), provenance }), /SHA/);
  assert.throws(() => inspect(good.subarray(0, good.length - 332)), /missing/);
  assert.throws(() => inspect(Buffer.concat([good, good.subarray(good.length - 332)])), /extra/);
  const badDue = Buffer.from(good), payload = 12 + good.readUInt32BE(8); badDue.writeBigUInt64BE(0n, payload + 332);
  assert.throws(() => inspect(badDue), /schedule/);
  const changed = Buffer.from(good); changed[changed.length - 1] ^= 1; assert.throws(() => inspect(changed), /altered/);
});
test('frozen actual importer hashes required; changing library, Node or kernel rejects', () => {
  for (const field of ['library_sha256', 'node_sha256', 'pcm_kernel_sha256']) assert.throws(() => inspect(archive(), 'mic', { ...provenance, [field]: '0'.repeat(64) }));
  const partial = { ...provenance }; delete partial.script_sha256; assert.throws(() => inspect(archive(), 'mic', partial));
});
test('request always includes entire archive and allowed explicit hold, fake pair rejected', () => {
  withPair(pair => { assert.deepEqual(replayRequest(pair, 200), { op: 'start', peer: 'publish', total_seconds: 21, audio_hold_ms: 200 }); for (const hold of [-1, 25, '50']) assert.throws(() => replayRequest(pair, hold)); assert.throws(() => replayRequest(structuredClone(pair), 0), /actual frozen/); });
});
test('0/50/200/500 controls require complete real hold brackets/counts, not exact hold claim', () => {
  withPair(pair => { for (const hold of [0, 50, 200, 500]) { const proof = validateReplayRuntime(pair, runtime(pair, hold)); assert.equal(proof.qualified, true); assert.equal(proof.audio_hold_ms, hold); assert.equal(proof.pcm_latency_calibrated, false); } });
});
test('native strict import flag, actual metadata and decoder/binary provenance are mandatory', () => {
  withPair(pair => {
    for (const change of [r => r.greeting.ready = 'true', r => r.greeting.provenance.mic.import_verified = false, r => r.greeting.provenance.source.import_verified = 'true', r => r.greeting.provenance.source.archive_sha256 = '0'.repeat(64), r => r.greeting.provenance.mic.metadata.pn.markers.pop(), r => r.greeting.provenance.decoder.sha256 = '0'.repeat(64), r => r.greeting.provenance.binary_sha256 = '0'.repeat(64)]) { const r = runtime(pair); change(r); assert.throws(() => validateReplayRuntime(pair, r)); }
  });
});
test('total/end/tail or shared source anchor mismatch and V1 loop inputs reject', () => {
  withPair(pair => {
    for (const change of [r => r.start.total_seconds--, r => r.status.tail_samples = 0, r => r.status.measurement_end_sample_ordinal--, r => r.status.seconds = 20, r => r.status.loop = true, r => r.status.timeline = { ...r.status.timeline, startClockNs: '123' }, r => r.status.sources.source.timeline = { ...r.status.sources.source.timeline, startClockNs: '234' }, r => r.start.timeline.pcm_latency_calibrated = true]) { const r = runtime(pair); change(r); assert.throws(() => validateReplayRuntime(pair, r)); }
  });
});
test('entire final packet and end-reached/valid/completed flags cannot be omitted or forged', () => {
  withPair(pair => {
    for (const change of [s => s.end_reached = 'true', s => s.completed = false, s => s.running = true, s => s.source_policy_valid = 1, s => s.packets_enqueued--, s => s.expected_packet_count++, s => s.last_source_sample_ordinal = 0, s => s.source_uid = 0, s => s.ssrc = 1]) { const r = runtime(pair); change(r.status.sources.source); assert.throws(() => validateReplayRuntime(pair, r)); }
  });
});
test('actual CLI enablement and hold applied to every real packet are required', () => {
  withPair(pair => {
    for (const change of [r => r.start.test_hold_enabled = false, r => r.status.audio_hold_ms = 0, r => r.status.sources.mic.test_hold_enabled = 'true', r => r.status.sources.mic.hold_applied_packets--, r => r.status.sources.source.min_actual_hold_ns = '1', r => r.status.sources.mic.max_actual_hold_ns = '0']) { const r = runtime(pair, 200); change(r); assert.throws(() => validateReplayRuntime(pair, r)); }
  });
});
test('lossy/negative/regressed clocks and fictitious enqueue/lateness extrema fail', () => {
  withPair(pair => {
    for (const change of [s => s.last_planned_mono_ns = 1, s => s.last_enqueue_bracket_ns = '01', s => s.last_enqueued_mono_ns = s.last_planned_mono_ns, s => s.last_enqueue_before_ns = '0', s => s.max_actual_hold_ns = '500000', s => s.min_actual_hold_ns = '2000000', s => s.max_schedule_lateness_ns = '0', s => { s.max_schedule_lateness_ns = '20000001'; s.max_actual_hold_ns = '70000001'; }]) { const r = runtime(pair, 50); change(r.status.sources.mic); assert.throws(() => validateReplayRuntime(pair, r)); }
  });
});
test('only the actual conversion bracket can bound slightly early minimum hold', () => {
  withPair(pair => {
    const r = runtime(pair, 50), source = r.status.sources.mic;
    source.min_actual_hold_ns = '49999980'; assert.doesNotThrow(() => validateReplayRuntime(pair, r));
    source.min_actual_hold_ns = '49999979'; assert.throws(() => validateReplayRuntime(pair, r));
  });
});
test('both complete received groups qualify intervals with calibration/comparison still false', () => {
  withPair(pair => { const result = qualifyReplayPair(evidence(pair)); assert.equal(result.qualified, true, result.failures.join()); assert.equal(result.groups.length, 2); assert.equal(result.groups[0].intervals.length, 10); assert.equal(result.pcm_latency_calibrated, false); assert.equal(result.comparison_available, false); });
});
test('partial/duplicate groups or subset peaks cannot qualify even with supplied subset markers', () => {
  withPair(pair => {
    for (const change of [e => e.groups.pop(), e => e.groups[1] = e.groups[0], e => e.groups[0].tap.peaks.pop(), e => { e.groups[0].tap.peaks.length = 1; e.groups[0].markers = pair.archives[0].metadata.pn.markers.slice(0, 1); }, e => e.groups[1].uid = 0, e => e.groups[0].run_id = 'foreign', e => e.groups[1].receiver.ssrc = 5]) { const e = evidence(pair); change(e); const result = qualifyReplayPair(e); assert.equal(result.qualified, false); assert.equal(result.groups.length, 0); }
  });
});
test('missing/duplicate markers, PCM clip/PLC, suspension, ring loss and old browser fail', () => {
  withPair(pair => {
    for (const change of [e => e.groups[0].tap.peaks.push(e.groups[0].tap.peaks[0]), e => e.groups[1].tap.clipped++, e => e.groups[1].receiver.concealedSamples = 128, e => e.groups[0].receiver.live = 'false', e => e.contextStates.push('suspended'), e => e.observer.missing++, e => e.observer.clocks.length = 1, e => e.browserClock.chromium_sha256 = '0'.repeat(64), e => e.browserClock.browser.revision = 'old']) { const e = evidence(pair); change(e); const result = qualifyReplayPair(e); assert.equal(result.qualified, false); assert.equal(result.groups.length, 0); }
  });
});
test('whole source enqueues cannot substitute for missing actual receiver tail', () => {
  withPair(pair => {
    const e = evidence(pair); e.groups[0].receiver.packetsReceived -= 50; e.groups[0].receiver.totalSamplesReceived -= 48000;
    const result = qualifyReplayPair(e); assert.equal(result.qualified, false); assert.equal(result.groups.length, 0);
  });
});
test('whole receiver packet counts cannot substitute for missing tail callbacks', () => {
  withPair(pair => {
    const e = evidence(pair); e.observer.rows[1] = e.observer.rows[1].filter(row => row.firstFrame < 20 * 48000);
    const result = qualifyReplayPair(e); assert.equal(result.qualified, false); assert.equal(result.groups.length, 0);
  });
});
