// Uses causal ordering only; no affine clock fit, rate/drift or DAC-time assumption.
export function nativeNs(value) {
  if (typeof value !== 'string' || !/^(0|[1-9]\d*)$/.test(value)) throw Error('native nanoseconds require canonical decimal string');
  const parsed = BigInt(value);
  if (parsed > 9223372036854775807n) throw Error('native nanoseconds exceed signed SAB range');
  return parsed;
}
export function validateClockSamples(samples) {
  if (!Array.isArray(samples) || samples.length < 2) throw Error('clock probes missing');
  let previous;
  for (const row of samples) {
    if (!Number.isFinite(row.p0) || !Number.isFinite(row.p1) || row.p0 > row.p1) throw Error('invalid clock request bracket');
    const n = nativeNs(row.monoNs);
    if (previous && (row.p0 < previous.p1 || n <= nativeNs(previous.monoNs))) throw Error('clock order regressed');
    previous = row;
  }
  return samples;
}
export function callbackInterval(rows, frame, { errorFrames = 96, epsilonMs = .1 } = {}) {
  if (!Number.isSafeInteger(frame) || frame < 0 || !Number.isInteger(errorFrames) || errorFrames < 0 || !Number.isFinite(epsilonMs) || epsilonMs < 0) throw Error('invalid marker frame policy');
  return callbackRange(rows, frame - errorFrames, frame + errorFrames, epsilonMs);
}
export function callbackRange(rows, low, high, epsilonMs = .1) {
  if (!Array.isArray(rows) || !Number.isSafeInteger(low) || low < 0 || !Number.isSafeInteger(high) || high < low || !Number.isFinite(epsilonMs) || epsilonMs < 0) throw Error('invalid callback range policy');
  const selected = rows.filter(row => row.firstFrame <= high && row.firstFrame + row.frames > low);
  if (!selected.length) throw Error('marker has no callback interval');
  let next = selected[0].firstFrame;
  if (next > low) throw Error('missing uncertainty-intersecting callback');
  const seen = new Set();
  for (const row of selected) {
    if (!Number.isSafeInteger(row.firstFrame) || !Number.isInteger(row.frames) || row.frames <= 0 || row.firstFrame !== next || seen.has(row.sequence)) throw Error('missing/duplicate callback frame');
    if (!Number.isFinite(row.lowerMs) || !Number.isFinite(row.upperMs) || row.lowerMs <= 0 || row.lowerMs > row.upperMs || row.flags || row.inputFrames !== row.frames) throw Error('invalid callback or decoded input');
    seen.add(row.sequence); next += row.frames;
  }
  if (next <= high) throw Error('missing uncertainty-intersecting callback');
  return { lowerMs: Math.min(...selected.map(row => row.lowerMs)) - epsilonMs, upperMs: Math.max(...selected.map(row => row.upperMs)) + epsilonMs, firstFrame: selected[0].firstFrame, endFrame: next, blocks: selected.length };
}
export function boundNativeCallback(interval, clocks, { epsilonMs = .1, maxWidthMs = 25 } = {}) {
  validateClockSamples(clocks);
  if (!Number.isFinite(interval.lowerMs) || !Number.isFinite(interval.upperMs) || interval.lowerMs > interval.upperMs || !Number.isFinite(epsilonMs) || epsilonMs < 0 || !Number.isFinite(maxWidthMs) || maxWidthMs <= 0) throw Error('invalid callback clock policy');
  let before, after;
  for (const clock of clocks) {
    if (clock.p1 + epsilonMs <= interval.lowerMs) before = clock;
    if (clock.p0 - epsilonMs >= interval.upperMs) { after = clock; break; }
  }
  if (!before || !after) throw Error('callback is not bracketed by native clock probes');
  const lower = nativeNs(before.monoNs), upper = nativeNs(after.monoNs);
  if (lower > upper) throw Error('native callback clock regressed');
  const widthMs = Number(upper - lower) / 1e6;
  if (!Number.isFinite(widthMs) || widthMs > maxWidthMs) throw Error('native callback clock interval too wide');
  return { lowerNs: lower.toString(), upperNs: upper.toString(), widthMs, scope: 'physical AudioWorklet callback interval; no output-device clock projection' };
}
export function sourceTimeInterval(timeline, sourceSampleOrdinal) {
  const start = nativeNs(timeline.startClockNs);
  if (timeline.clock !== 'CLOCK_MONOTONIC' || !Number.isSafeInteger(sourceSampleOrdinal) || sourceSampleOrdinal < 0 || !Number.isSafeInteger(timeline.conversionBracketNs) || timeline.conversionBracketNs < 0 || timeline.conversionBracketNs > 100000) throw Error('invalid planned source clock anchor');
  const numerator = BigInt(sourceSampleOrdinal) * 1000000000n;
  const floor = numerator / 48000n, ceil = (numerator + 47999n) / 48000n;
  const error = BigInt(timeline.conversionBracketNs);
  if (start + floor < error) throw Error('source anchor error crosses clock zero');
  return { lowerNs: (start + floor - error).toString(), upperNs: (start + ceil + error).toString() };
}
export function latencyInterval(source, received) {
  if (nativeNs(source.lowerNs) > nativeNs(source.upperNs) || nativeNs(received.lowerNs) > nativeNs(received.upperNs)) throw Error('latency interval endpoints reversed');
  const lower = nativeNs(received.lowerNs) - nativeNs(source.upperNs);
  const upper = nativeNs(received.upperNs) - nativeNs(source.lowerNs);
  return { lowerMs: Number(lower) / 1e6, upperMs: Number(upper) / 1e6 };
}
