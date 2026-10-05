// Offline decoded-PCM inspection only. This is not a wall-clock calibration.
import fs from 'node:fs';
import { PCM, markerCode, MarkerDetector } from './pcm-kernel.mjs';

export function checkMarkers(samples, manifest, lookahead = 0) {
  if (manifest.source_uid !== 0 && manifest.source_uid !== 64) throw Error('wrong source UID');
  if (!manifest.markers?.length) throw Error('no expected markers');
  const policy = manifest.marker_policy;
  for (const key of ['sampleRate', 'carrierHz', 'chipFrames', 'chips', 'periodFrames', 'amplitude', 'threshold']) {
    if (policy?.[key] !== PCM[key]) throw Error('marker kernel policy differs: ' + key);
  }
  const first = manifest.markers[0].source_sample_ordinal;
  const seenCodes = new Set();
  for (const [index, marker] of manifest.markers.entries()) {
    const code = markerCode(manifest.source_uid, index);
    if (marker.sequence !== index || marker.source_sample_ordinal !== first + index * PCM.periodFrames || JSON.stringify(marker.code) !== JSON.stringify(code)) throw Error('forged codebook');
    const signature = [code.join(','), code.map(x => -x).join(',')].sort()[0];
    if (seenCodes.has(signature)) throw Error('duplicate marker code');
    seenCodes.add(signature);
  }
  const peaks = [];
  const detector = new MarkerDetector(manifest.source_uid, peak => peaks.push(peak), first + lookahead);
  let clipped = 0;
  for (let frame = 0; frame < samples.length; frame++) {
    const value = samples[frame];
    if (!Number.isFinite(value)) throw Error('nonfinite decoded PCM');
    if (Math.abs(value) >= .999) clipped++;
    detector.push(value, frame);
  }
  if (clipped) throw Error('clipped decoded PCM');
  if (peaks.length !== manifest.markers.length) throw Error('missing or ambiguous marker count');
  const checks = manifest.markers.map(marker => {
    const matches = peaks.filter(peak => peak.sequence === marker.sequence);
    if (matches.length !== 1) throw Error('missing or ambiguous marker sequence');
    const actual = matches[0], expected = marker.source_sample_ordinal + lookahead;
    const residual = actual.receivedFrame - expected;
    if (Math.abs(residual) > 96) throw Error('marker alignment differs from retained codec lookahead');
    return { sequence: marker.sequence, expected_decoded_sample_ordinal: expected, actual_decoded_sample_ordinal: actual.receivedFrame, residual_samples: residual, score: actual.score, amplitude: actual.amplitude };
  });
  return { samples: samples.length, markers: checks.length, max_alignment_error_samples: Math.max(...checks.map(row => Math.abs(row.residual_samples))), checks, scope: 'offline stored source PCM -> decoded PCM; codec lookahead retained; no native/browser timeline calibration' };
}

if (process.argv[1] && import.meta.url === new URL('file://' + process.argv[1]).href) {
  const [pcmPath, manifestPath, lookaheadText] = process.argv.slice(2);
  if (!pcmPath || !manifestPath || !/^\d+$/.test(lookaheadText ?? '')) throw Error('usage: pn-opus-markers.mjs PCM.f32 MANIFEST.json LOOKAHEAD_SAMPLES');
  const data = fs.readFileSync(pcmPath);
  if (!data.length || data.length % 4) throw Error('float32le PCM length differs');
  const samples = new Float32Array(data.length / 4);
  for (let i = 0; i < samples.length; i++) samples[i] = data.readFloatLE(i * 4);
  console.log(JSON.stringify(checkMarkers(samples, JSON.parse(fs.readFileSync(manifestPath, 'utf8')), Number(lookaheadText))));
}
