// Structural/import/replay contract only. Native decoding and real controls
// remain separate gates; these validators never grant comparison/calibration.
import fs from 'node:fs';
import { createHash } from 'node:crypto';
import { PCM, markerCode } from './pcm-kernel.mjs';
import { nativeNs, sourceTimeInterval, boundNativeCallback } from './native-pcm-clock-bounds.mjs';
import { qualifyNativeMarkers } from './native-pcm-evidence.mjs';
import { NATIVE_PCM_POLICY as CLOCK, validateBrowserClock } from './native-pcm-policy.mjs';

const RATE = 48000, FRAME = 960, PERIOD = 20000000n, MAX_BYTES = 8 * 1024 * 1024;
const MARKER = Object.freeze(Object.fromEntries(['sampleRate', 'carrierHz', 'chipFrames', 'chips', 'periodFrames', 'amplitude', 'threshold'].map(key => [key, PCM[key]])));
const ROLES = Object.freeze([{ role: 'mic', uid: 0, ssrc: 0x474d4943, frequencies: [317, 719, 1249, 2027], gain: .07 }, { role: 'source', uid: 64, ssrc: 0x47534130, frequencies: [440], gain: .45 }]);
const imported = new WeakSet();
const digest = data => createHash('sha256').update(data).digest('hex');
const requireThat = (condition, message) => { if (!condition) throw Error(message); };
const sha = value => typeof value === 'string' && /^[a-f0-9]{64}$/.test(value);
const object = value => value !== null && typeof value === 'object' && !Array.isArray(value);
const integer = (value, low, high) => Number.isSafeInteger(value) && value >= low && value <= high;
export const canonical = value => {
  if (Array.isArray(value)) return '[' + value.map(canonical).join(',') + ']';
  if (object(value)) return '{' + Object.keys(value).sort().map(key => JSON.stringify(key) + ':' + canonical(value[key])).join(',') + '}';
  requireThat(value === null || ['string', 'boolean', 'number'].includes(typeof value) && (typeof value !== 'number' || Number.isFinite(value)), 'non-JSON contract value');
  return JSON.stringify(value);
};
const same = (actual, expected, label) => requireThat(canonical(actual) === canonical(expected), label + ' differs');
const freeze = value => { if (object(value) || Array.isArray(value)) { for (const child of Object.values(value)) freeze(child); Object.freeze(value); } return value; };

// JSON.parse silently discards duplicate keys. Parse bounded headers first,
// checking decoded keys as well ("run_id" and "\\u0072un_id" are identical).
export function uniqueJson(text) {
  requireThat(typeof text === 'string' && Buffer.byteLength(text) <= 256 * 1024, 'manifest JSON length exceeds bound');
  let cursor = 0, nodes = 0;
  const whitespace = () => { while (/[\t\r\n ]/.test(text[cursor] ?? '\0')) cursor++; };
  const string = () => {
    const start = cursor++;
    while (cursor < text.length) {
      const code = text.charCodeAt(cursor++);
      if (code === 34) return JSON.parse(text.slice(start, cursor));
      if (code === 92) cursor++;
    }
    throw Error('unterminated manifest JSON string');
  };
  const value = depth => {
    requireThat(depth <= 16 && ++nodes <= 100000, 'manifest JSON depth/node bound exceeded'); whitespace();
    const token = text[cursor];
    if (token === '"') return string();
    if (token === '{') {
      cursor++; whitespace(); const result = Object.create(null), keys = new Set();
      if (text[cursor] === '}') { cursor++; return result; }
      while (true) {
        requireThat(text[cursor] === '"', 'manifest object key required'); const key = string();
        requireThat(!keys.has(key), 'duplicate manifest key: ' + key); keys.add(key); whitespace();
        requireThat(text[cursor++] === ':', 'manifest colon required'); result[key] = value(depth + 1); whitespace();
        const end = text[cursor++]; if (end === '}') return result;
        requireThat(end === ',', 'manifest object separator required'); whitespace();
      }
    }
    if (token === '[') {
      cursor++; whitespace(); const result = [];
      if (text[cursor] === ']') { cursor++; return result; }
      while (true) {
        result.push(value(depth + 1)); whitespace(); const end = text[cursor++]; if (end === ']') return result;
        requireThat(end === ',', 'manifest array separator required');
      }
    }
    const match = /^(?:true|false|null|-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?)/.exec(text.slice(cursor));
    requireThat(match, 'invalid manifest JSON token'); cursor += match[0].length;
    const parsed = JSON.parse(match[0]); requireThat(typeof parsed !== 'number' || Number.isFinite(parsed), 'nonfinite manifest number'); return parsed;
  };
  const result = value(0); whitespace(); requireThat(cursor === text.length, 'trailing manifest JSON'); return result;
}

export function expectedBook(measurementSeconds, runId) {
  requireThat(integer(measurementSeconds, 20, 360) && typeof runId === 'string' && /^[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}$/.test(runId), 'invalid finite duration/shared run UUID');
  const ordinals = [];
  for (let start = RATE; start < measurementSeconds * RATE - PCM.chips * PCM.chipFrames; start += PCM.periodFrames) ordinals.push(start);
  const signatures = new Set();
  const sources = ROLES.map(({ role, uid, ssrc }) => ({ kind: role, source_uid: uid, ssrc, markers: ordinals.map((start, sequence) => {
    const code = markerCode(uid, sequence), signature = [code.join(','), code.map(v => -v).join(',')].sort()[0];
    requireThat(!signatures.has(signature), 'duplicate/inverted finite marker code'); signatures.add(signature);
    return { sequence, source_sample_ordinal: start, code };
  }) }));
  const shared = { run_id: runId, measurement_end_sample_ordinal: measurementSeconds * RATE, tail_samples: RATE, marker_policy: MARKER, sources };
  return freeze({ shared, sha256: digest(canonical(shared)) });
}

function validateMetadata(metadata, role, provenance) {
  requireThat(object(metadata), 'archive metadata object required');
  const seconds = metadata.measurement_seconds, book = expectedBook(seconds, metadata.run_id), count = (seconds + 1) * 50, samples = (seconds + 1) * RATE;
  const expected = { schema: 2, codec: 'opus', sample_rate_hz: RATE, channels: 1, packet_duration_ms: 20, packets: count, measurement_seconds: seconds, tail_seconds: 1, duration_seconds: seconds + 1, rtp_clock_hz: RATE, measurement_end_sample_ordinal: seconds * RATE, tail_samples: RATE, rtp_timestamp_step: FRAME, payload_bitrate_bps: 128000, encoded_bytes: count * 320, loop_policy: 'unlooped; stop at archive end; no rewind or modulo', pcm_latency_calibrated: false, comparison_available: false };
  for (const [key, value] of Object.entries(expected)) same(metadata[key], value, 'archive ' + key);
  const source = book.shared.sources.find(v => v.kind === role.role);
  same(metadata.pn, { run_id: metadata.run_id, codebook_sha256: book.sha256, source_uid: role.uid, ssrc: role.ssrc, marker_policy: MARKER, markers: source.markers, shared_run: book.shared }, 'complete role/run/codebook');
  const pcm = metadata.pcm;
  requireThat(object(pcm) && sha(pcm.sha256) && Number.isFinite(pcm.peak) && pcm.peak > 0 && pcm.peak < .999, 'source PCM hash/clip claim invalid');
  for (const [key, value] of Object.entries({ kind: role.role, source_uid: role.uid, frequencies_hz: role.frequencies, gain_per_tone: role.gain, samples, format: 'float32le mono' })) same(pcm[key], value, 'source PCM ' + key);
  const decoded = metadata.decode_control;
  requireThat(object(decoded) && sha(decoded.float32le_sha256) && decoded.samples === samples && Number.isFinite(decoded.peak) && decoded.peak > 0 && decoded.peak < .999 && Number.isFinite(decoded.input_correlation_after_codec_lookahead) && decoded.input_correlation_after_codec_lookahead >= .98 && decoded.input_correlation_after_codec_lookahead <= 1, 'decoded PCM integrity/clip claim invalid');
  const encoder = metadata.encoder;
  requireThat(object(encoder) && encoder.application === 'audio' && encoder.application_constant === 2049 && encoder.lookahead_samples === 312 && encoder.lookahead_ms === 6.5, 'retained actual codec lookahead differs');
  same(encoder.settings_readback, { bitrate: 128000, complexity: 10, dtx: 0, inband_fec: 1, packet_loss_percent: 1, vbr: 0 }, 'actual encoder policy');
  const control = metadata.marker_control;
  requireThat(object(control) && control.samples === samples && control.markers === source.markers.length && Array.isArray(control.checks) && control.checks.length === source.markers.length, 'complete offline decoded marker control required');
  let maximum = 0;
  for (const [sequence, check] of control.checks.entries()) {
    const expectedOrdinal = source.markers[sequence].source_sample_ordinal + encoder.lookahead_samples;
    requireThat(check.sequence === sequence && check.expected_decoded_sample_ordinal === expectedOrdinal && Number.isSafeInteger(check.actual_decoded_sample_ordinal) && check.residual_samples === check.actual_decoded_sample_ordinal - expectedOrdinal && Math.abs(check.residual_samples) <= CLOCK.markerErrorFrames && Number.isFinite(check.score) && check.score >= PCM.threshold && check.score <= 1 && Number.isFinite(check.amplitude) && check.amplitude >= .04, 'offline decoded marker alignment/sequence differs');
    maximum = Math.max(maximum, Math.abs(check.residual_samples));
  }
  same(control.max_alignment_error_samples, maximum, 'offline residual maximum');
  same(control.executed_node, { version: CLOCK.nodeVersion, sha256: provenance.node_sha256 }, 'executed marker Node');
  for (const [key, expectedHash] of Object.entries(provenance)) {
    if (key === 'node_sha256') continue;
    requireThat(sha(expectedHash) && metadata.provenance?.[key] === expectedHash, 'frozen archive provenance differs: ' + key);
  }
  requireThat(metadata.provenance?.library_version === 'libopus 1.6.1' && sha(metadata.encoded_packets_sha256), 'actual Opus/provenance missing');
  return book;
}

// Does NOT decode Opus: native greeting must attest its strict actual import.
// Expected hashes come from an independently frozen source/archive manifest.
export function inspectReplayArchive(bytes, { role: roleName, sha256, provenance }) {
  const role = ROLES.find(v => v.role === roleName);
  requireThat(role && Buffer.isBuffer(bytes) && bytes.length >= 12 && bytes.length <= MAX_BYTES && bytes.subarray(0, 8).equals(Buffer.from('GPOPUS2\n')), 'invalid V2 archive magic/length/role');
  requireThat(sha(sha256) && digest(bytes) === sha256, 'actual archive SHA differs');
  const length = bytes.readUInt32BE(8);
  requireThat(length > 0 && length <= 256 * 1024 && 12 + length <= bytes.length, 'invalid V2 manifest length');
  const text = new TextDecoder('utf-8', { fatal: true }).decode(bytes.subarray(12, 12 + length)), metadata = uniqueJson(text);
  requireThat(object(provenance) && ['library_sha256', 'script_sha256', 'fixed_opus_script_sha256', 'pcm_kernel_sha256', 'pn_inspector_sha256', 'node_sha256'].every(key => sha(provenance[key])) && provenance.node_sha256 === CLOCK.nodeSha256, 'complete frozen importer provenance required');
  const book = validateMetadata(metadata, role, provenance), payloadHash = createHash('sha256');
  let cursor = 12 + length, packets = 0;
  while (cursor < bytes.length) {
    requireThat(packets < metadata.packets && cursor + 12 <= bytes.length, 'extra/truncated V2 packet');
    const due = bytes.readBigUInt64BE(cursor), size = bytes.readUInt32BE(cursor + 8); cursor += 12;
    requireThat(due === BigInt(packets) * PERIOD && size === 320 && cursor + size <= bytes.length, 'V2 packet schedule/length differs');
    payloadHash.update(bytes.subarray(cursor, cursor + size)); cursor += size; packets++;
  }
  requireThat(packets === metadata.packets && payloadHash.digest('hex') === metadata.encoded_packets_sha256, 'missing/altered V2 packets');
  return freeze({ role: roleName, uid: role.uid, ssrc: role.ssrc, archive_sha256: sha256, metadata, book });
}

export function readReplayPair({ mic, source, provenance }) {
  const archives = ROLES.map(({ role }) => { const input = role === 'mic' ? mic : source; requireThat(object(input) && typeof input.path === 'string', 'actual archive path required'); const size = fs.statSync(input.path).size; requireThat(size <= MAX_BYTES, 'archive file exceeds bound'); return inspectReplayArchive(fs.readFileSync(input.path), { role, sha256: input.sha256, provenance }); });
  const [first, second] = archives;
  requireThat(first.metadata.run_id === second.metadata.run_id && first.metadata.measurement_seconds === second.metadata.measurement_seconds && first.book.sha256 === second.book.sha256, 'shared run/duration mismatch between actual archives');
  same(first.book.shared, second.book.shared, 'shared complete finite book');
  const pair = freeze({ archives, run_id: first.metadata.run_id, codebook_sha256: first.book.sha256, measurement_seconds: first.metadata.measurement_seconds, total_seconds: first.metadata.duration_seconds, measurement_end_sample_ordinal: first.metadata.measurement_end_sample_ordinal, tail_samples: RATE, provenance: structuredClone(provenance), comparison_available: false, pcm_latency_calibrated: false });
  imported.add(pair); return pair;
}

const trustedPair = pair => requireThat(imported.has(pair), 'replay pair must be imported from actual frozen archive bytes');
export function replayRequest(pair, audioHoldMs) {
  trustedPair(pair); requireThat([0, 50, 200, 500].includes(audioHoldMs), 'unsupported test audio hold');
  return freeze({ op: 'start', peer: 'publish', total_seconds: pair.total_seconds, audio_hold_ms: audioHoldMs });
}

export function validateReplayRuntime(pair, { greeting, start, status, nativeBinarySha256, audioHoldMs }) {
  trustedPair(pair); const actual = greeting?.provenance;
  requireThat(greeting?.ready === true && object(actual) && [0, 50, 200, 500].includes(audioHoldMs) && sha(nativeBinarySha256) && actual.binary_sha256 === nativeBinarySha256, 'actual native binary/hold provenance differs');
  requireThat(actual.comparison_available === false && actual.pcm_latency_calibrated === false, 'native scope must remain uncalibrated');
  // Exact greeting shape is shared with the V2 importer, not a caller summary.
  same(actual.decoder?.version, 'libopus 1.6.1', 'executed decoder version'); same(actual.decoder?.sha256, pair.provenance.library_sha256, 'executed decoder library');
  for (const archive of pair.archives) {
    const native = actual[archive.role];
    requireThat(native?.import_verified === true && native.archive_sha256 === archive.archive_sha256, 'actual strict native archive import is unqualified');
    same(native.metadata, archive.metadata, 'native imported metadata');
  }
  for (const state of [start, status]) {
    requireThat(object(state), 'native replay start/status missing');
    for (const key of ['measurement_seconds', 'total_seconds', 'measurement_end_sample_ordinal', 'tail_samples']) same(state[key], pair[key], 'native ' + key);
    requireThat(state.test_hold_enabled === true && state.audio_hold_ms === audioHoldMs && state.comparison_available === false && state.pcm_latency_calibrated === false, 'actual replay test mode/scope differs');
    requireThat(!['seconds', 'loop', 'loops', 'rewind', 'endOrdinal'].some(key => Object.hasOwn(state, key)), 'V1 seconds/loop/alternate end contract forbidden');
    requireThat(state.timeline?.pcm_latency_calibrated === false && state.timeline?.comparison_available !== true, 'planned timeline must remain uncalibrated');
    sourceTimeInterval(state.timeline, 0);
  }
  same(status.timeline, start.timeline, 'common replay source anchor');
  requireThat(object(status.sources) && Object.keys(status.sources).every(key => ['video', 'mic', 'source'].includes(key)) && object(status.sources.mic) && object(status.sources.source), 'whole two-source replay status required');
  const anchor = nativeNs(start.timeline.startClockNs), hold = BigInt(audioHoldMs) * 1000000n, error = BigInt(start.timeline.conversionBracketNs);
  for (const archive of pair.archives) {
    const source = status.sources[archive.role], count = archive.metadata.packets, lastOrdinal = (count - 1) * FRAME, lastDue = anchor + BigInt(count - 1) * PERIOD;
    requireThat(source.source_uid === archive.uid && source.ssrc === archive.ssrc && source.end_reached === true && source.completed === true && source.running === false && source.source_policy_valid === true && source.packets_enqueued === count && source.expected_packet_count === count && source.last_source_sample_ordinal === lastOrdinal, 'native role/complete packet end differs');
    same(source.timeline, start.timeline, 'per-source common anchor');
    requireThat(source.test_hold_enabled === true && source.audio_hold_ms === audioHoldMs && source.hold_applied_packets === count, 'test hold not proved on every actual enqueue');
    const plan = nativeNs(source.last_planned_mono_ns), before = nativeNs(source.last_enqueue_before_ns), after = nativeNs(source.last_enqueued_mono_ns), bracket = nativeNs(source.last_enqueue_bracket_ns), minimum = nativeNs(source.min_actual_hold_ns), maximum = nativeNs(source.max_actual_hold_ns), late = nativeNs(source.max_schedule_lateness_ns);
    requireThat(plan === lastDue && before + error >= plan + hold && after >= before && bracket === after - before, 'last actual enqueue/hold bracket differs');
    // Extrema are real scheduling delay, not an assertion of exactly hold ms.
    requireThat(before >= plan && minimum + error >= hold && maximum >= minimum && minimum <= before - plan && maximum >= after - plan && maximum === hold + late && late <= PERIOD, 'actual hold extrema/lateness counter differs');
  }
  return freeze({ qualified: true, timeline: structuredClone(start.timeline), run_id: pair.run_id, codebook_sha256: pair.codebook_sha256, audio_hold_ms: audioHoldMs, comparison_available: false, pcm_latency_calibrated: false });
}

function wholeReceiver(archive, receiver) {
  const id = receiver?.inboundReportId;
  requireThat(typeof id === 'string' && id.trim().length > 0 && id.length <= 1024 && receiver.initialInboundReportId === id, 'same actual held inbound report required for whole receiver');
  requireThat(receiver.initialPacketsReceived === 0 && receiver.initialTotalSamplesReceived === 0, 'whole receiver counters must start at zero before finite replay');
  requireThat(receiver.packetsReceived === archive.metadata.packets && receiver.totalSamplesReceived === archive.metadata.decode_control.samples, 'complete actual receiver packet/decoded sample totals required');
  // totalSamplesReceived includes concealed samples. Packet/sample totals alone
  // cannot establish a lossless decode, and time stretching changes PN ordinals.
  // Until actual per-sample stretch mapping exists, reject any insert/remove.
  for (const field of ['packetsLost', 'concealedSamples', 'silentConcealedSamples', 'packetsDiscarded', 'insertedSamplesForDeceleration', 'removedSamplesForAcceleration']) {
    const initialField = 'initial' + field[0].toUpperCase() + field.slice(1);
    requireThat(receiver[initialField] === 0 && receiver[field] === 0, 'actual initial/final whole receiver counter unavailable or nonzero: ' + field);
  }
  return id;
}

function wholeCallbackCoverage(archive, tap, rows, clocks, inboundReportId) {
  const samples = archive.metadata.decode_control.samples, lookahead = archive.metadata.encoder.lookahead_samples;
  let minimumStart = Infinity, maximumStart = -Infinity;
  // qualifyNativeMarkers already matched every peak, sequence and source UID.
  // Decode ordinals include retained actual codec lookahead. Derive bounds from
  // genuine peaks, never an invented end marker or a future callback prediction.
  for (const marker of archive.metadata.pn.markers) {
    const peak = tap.peaks.find(value => value.sequence === marker.sequence);
    const start = peak.receivedFrame - marker.source_sample_ordinal - lookahead;
    requireThat(Number.isSafeInteger(start) && start + CLOCK.markerErrorFrames >= 0, 'decoded stream start cannot precede AudioContext frame zero');
    minimumStart = Math.min(minimumStart, start); maximumStart = Math.max(maximumStart, start);
  }
  requireThat(maximumStart - minimumStart <= 2 * CLOCK.markerErrorFrames, 'genuine PN peaks have no shared unchanged decoded start uncertainty');
  const low = Math.max(0, minimumStart - CLOCK.markerErrorFrames), latest = maximumStart + CLOCK.markerErrorFrames, end = latest + samples;
  requireThat(integer(low, 0, Number.MAX_SAFE_INTEGER) && integer(end, low + 1, Number.MAX_SAFE_INTEGER), 'derived whole decoded callback range invalid');
  requireThat(Array.isArray(rows) && rows.length > 0, 'whole decoded callback rows missing');
  let next, previous, blocks = 0, firstFrame;
  for (const row of rows) {
    requireThat(object(row) && integer(row.firstFrame, 0, Number.MAX_SAFE_INTEGER) && integer(row.frames, 1, Number.MAX_SAFE_INTEGER) && Number.isSafeInteger(row.firstFrame + row.frames) && integer(row.sequence, 1, Number.MAX_SAFE_INTEGER), 'malformed actual callback frame/sequence');
    if (row.firstFrame >= end || row.firstFrame + row.frames <= low) continue;
    if (next === undefined) { firstFrame = row.firstFrame; next = firstFrame; requireThat(firstFrame <= low, 'whole decoded start has no actual callback input'); }
    requireThat(row.firstFrame === next && (!previous || row.sequence === previous.sequence + 1), 'whole decoded callback coverage is missing/duplicate/reordered');
    requireThat(row.flags === 0 && row.inputFrames === row.frames && Number.isFinite(row.lowerMs) && Number.isFinite(row.upperMs) && row.lowerMs > 0 && row.lowerMs <= row.upperMs && (!previous || row.lowerMs >= previous.lowerMs && row.upperMs >= previous.upperMs), 'whole decoded callback input/clock is unqualified');
    // Find the same conservative causal brackets as Clock13, with binary search
    // over its already validated probes. No clock fitting or extrapolation.
    const interval = { lowerMs: row.lowerMs - CLOCK.epsilonMs, upperMs: row.upperMs + CLOCK.epsilonMs };
    let left = 0, right = clocks.length;
    while (left < right) { const middle = Math.floor((left + right) / 2); if (clocks[middle].p1 + CLOCK.epsilonMs <= interval.lowerMs) left = middle + 1; else right = middle; }
    const before = clocks[left - 1]; left = 0; right = clocks.length;
    while (left < right) { const middle = Math.floor((left + right) / 2); if (clocks[middle].p0 - CLOCK.epsilonMs >= interval.upperMs) right = middle; else left = middle + 1; }
    const after = clocks[left];
    requireThat(before && after, 'whole decoded callback lacks actual native clock brackets');
    boundNativeCallback(interval, [before, after], { epsilonMs: CLOCK.epsilonMs, maxWidthMs: CLOCK.maxIntervalWidthMs });
    next += row.frames; previous = row; blocks++;
  }
  requireThat(blocks > 0 && next >= end, 'whole received tail lacks continuous actual callback input');
  return { inboundReportId, packetsReceived: archive.metadata.packets, totalSamplesReceived: samples, codecLookaheadSamples: lookahead, derivedStartFrameLower: low, derivedStartFrameUpper: latest, requiredEndFrameExclusive: end, observedFirstFrame: firstFrame, observedEndFrameExclusive: next, blocks, allCallbacksClockBounded: true };
}

// No caller-supplied marker list or duration can shrink the required evidence.
export function qualifyReplayPair({ pair, runtime, groups, observer, contextStates, browserClock }) {
  const result = { qualified: false, comparison_available: false, pcm_latency_calibrated: false, scope: 'complete native V2 direct-loopback decoded-input callback intervals; excludes live capture/encoder/DSP/playout/acoustics; not SFU comparison or calibration', failures: [], groups: [] };
  try {
    trustedPair(pair); validateBrowserClock(browserClock);
    requireThat(browserClock.chromium_sha256 === CLOCK.chromiumSha256 && browserClock.node_sha256 === CLOCK.nodeSha256, 'actual current browser/Node binary differs');
    const replay = validateReplayRuntime(pair, runtime);
    requireThat(Array.isArray(groups) && groups.length === 2 && groups.every(group => object(group)) && groups.map(group => group.role).sort().join(',') === 'mic,source' && Array.isArray(observer?.rows) && observer.rows.length === 2, 'partial/duplicate/foreign receiver group forbidden');
    const inboundReports = new Set();
    for (const archive of pair.archives) {
      const group = groups.find(v => v.role === archive.role);
      requireThat(group.uid === archive.uid && group.archive_sha256 === archive.archive_sha256 && group.run_id === pair.run_id && group.codebook_sha256 === pair.codebook_sha256, 'received source/run/archive role binding differs');
      const inboundReportId = wholeReceiver(archive, group.receiver);
      requireThat(!inboundReports.has(inboundReportId), 'distinct actual inbound reports required for both held receivers'); inboundReports.add(inboundReportId);
      const qualified = qualifyNativeMarkers({ observer, tap: group.tap, uid: archive.uid, markers: archive.metadata.pn.markers, timeline: replay.timeline, receiver: group.receiver, contextStates, sourceArchiveSha256: archive.archive_sha256, codebookSha256: pair.codebook_sha256 });
      requireThat(qualified.qualified === true, qualified.failures.join('; '));
      const receiverTail = wholeCallbackCoverage(archive, group.tap, observer.rows[archive.uid === 0 ? 0 : 1], observer.clocks, inboundReportId);
      result.groups.push({ role: archive.role, ...qualified, receiverTail });
    }
    result.qualified = true;
  } catch (error) { result.failures.push(String(error)); result.groups = []; }
  return result;
}
