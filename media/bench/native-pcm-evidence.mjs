import { PCM, markerCode } from './pcm-kernel.mjs';
import { callbackInterval, callbackRange, boundNativeCallback, sourceTimeInterval, latencyInterval, validateClockSamples } from './native-pcm-clock-bounds.mjs';
import { NATIVE_PCM_POLICY as POLICY } from './native-pcm-policy.mjs';

export function completeMarkerMatches(peaks, { uid, markers }) {
  if (![0, 64].includes(uid) || !Array.isArray(markers) || !markers.length || markers.length > POLICY.maxMarkers || !Array.isArray(peaks)) throw Error('invalid finite marker contract');
  const codes = new Set();
  for (const [sequence, marker] of markers.entries()) {
    if (marker.sequence !== sequence || marker.source_sample_ordinal !== 48000 + sequence * PCM.periodFrames) throw Error('finite expected marker schedule differs');
    const code = markerCode(uid, sequence).join(',');
    if (codes.has(code) || codes.has(markerCode(uid, sequence).map(v => -v).join(','))) throw Error('duplicate finite marker code');
    codes.add(code);
  }
  if (peaks.length !== markers.length) throw Error('missing/duplicate marker: partial measurement forbidden');
  const received = new Set();
  return markers.map(marker => {
    const matches = peaks.filter(peak => peak.sequence === marker.sequence);
    if (matches.length !== 1) throw Error('ambiguous or missing expected marker');
    const peak = matches[0];
    if (!Number.isSafeInteger(peak.receivedFrame) || peak.receivedFrame < POLICY.markerErrorFrames || !Number.isFinite(peak.score) || peak.score < PCM.threshold || !Number.isFinite(peak.amplitude) || peak.amplitude < .04 || received.has(peak.receivedFrame)) throw Error('invalid/duplicate decoded marker peak');
    received.add(peak.receivedFrame); return { ...marker, peak };
  });
}

// Exact held receiver identity must stay live/enabled. Muted may be reported
// during a genuine transport interruption, so it is recorded but not proof.
export function validateReceiverEvidence(evidence, uid) {
  if (!evidence || evidence.uid !== uid || !Number.isInteger(evidence.ssrc) || evidence.ssrc !== (uid === 0 ? 0x474d4943 : 0x47534130) || evidence.role !== (uid === 0 ? 'mic' : 'source')) throw Error('actual source UID/role/SSRC binding differs');
  if (evidence.identityStable !== true || evidence.live !== true || evidence.enabled !== true || evidence.codec !== 'audio/opus' || evidence.decodedSamplesProgress !== true || evidence.packetsProgress !== true) throw Error('receiver identity or decoded progress is unqualified');
  // inputFrames only establishes callback input shape, not absence of PLC.
  // All counters must be actually available from the same inbound report.
  for (const field of ['packetsLost', 'concealedSamples', 'silentConcealedSamples']) if (evidence[field] !== 0) throw Error('actual decoder loss/concealment unavailable or nonzero');
}

export function qualifyNativeMarkers({ observer, tap, uid, markers, timeline, receiver, contextStates, sourceArchiveSha256, codebookSha256 }) {
  const result = { qualified: false, comparison_available: false, pcm_latency_calibrated: false, scope: 'conditional native direct-loopback marker intervals at decoded-input AudioWorklet callback; excludes live capture/encoder/DSP/output-device/acoustics', failures: [], intervals: [], uid, sourceArchiveSha256, codebookSha256 };
  try {
    if (!/^[a-f0-9]{64}$/.test(sourceArchiveSha256) || !/^[a-f0-9]{64}$/.test(codebookSha256)) throw Error('archive/codebook provenance missing');
    if (observer.failures?.length || observer.missing !== 0 || tap.excessPeaks !== 0 || tap.gaps !== 0 || tap.nonfinite !== 0 || tap.clipped !== 0 || tap.sampleRate !== POLICY.sampleRate || tap.uid !== uid) throw Error('callback ring/PCM integrity is unqualified');
    if (!Array.isArray(contextStates) || !contextStates.length || contextStates.some(state => state !== 'running')) throw Error('AudioContext suspension invalidates native qualification');
    validateReceiverEvidence(receiver, uid); validateClockSamples(observer.clocks);
    const matches = completeMarkerMatches(tap.peaks, { uid, markers });
    const rows = observer.rows[uid === 0 ? 0 : 1];
    for (const marker of matches) {
      // Reject lost input anywhere in the entire marker, including tolerance.
      callbackRange(rows, marker.peak.receivedFrame - POLICY.markerErrorFrames, marker.peak.receivedFrame + PCM.chips * PCM.chipFrames + POLICY.markerErrorFrames, POLICY.epsilonMs);
      const callback = callbackInterval(rows, marker.peak.receivedFrame, { errorFrames: POLICY.markerErrorFrames, epsilonMs: POLICY.epsilonMs });
      const received = boundNativeCallback(callback, observer.clocks, { epsilonMs: POLICY.epsilonMs, maxWidthMs: POLICY.maxIntervalWidthMs });
      const source = sourceTimeInterval(timeline, marker.source_sample_ordinal);
      result.intervals.push({ sequence: marker.sequence, sourceSampleOrdinal: marker.source_sample_ordinal, receivedFrame: marker.peak.receivedFrame, callback, received, source, ...latencyInterval(source, received) });
    }
    result.qualified = true;
  } catch (error) { result.failures.push(String(error)); result.intervals = []; }
  return result;
}
