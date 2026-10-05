import { createHash } from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
export const PCM_ARTIFACTS = ['pcm-kernel.mjs', 'pcm-marker.mjs', 'pcm.bundle.js', 'client.bundle.js'];
export function pcmArtifactHashes(folder) {
  return Object.fromEntries(PCM_ARTIFACTS.map(name => [name, createHash('sha256').update(fs.readFileSync(path.join(folder, name))).digest('hex')]));
}

export function readPcmCalibration(filename, folder) {
  if (!filename) throw new Error('PCM latency requires --pcm-calibration FILE from pcm-calibrate.mjs');
  const calibration = JSON.parse(fs.readFileSync(filename, 'utf8'));
  if (!calibration.valid || calibration.failures?.length || calibration.delay_checks?.length !== 4 || !calibration.delay_checks.every(check => check.valid)) throw new Error('PCM calibration did not pass all known-delay controls');
  for (const [name, digest] of Object.entries(pcmArtifactHashes(folder))) {
    if (calibration.artifact_sha256?.[name] !== digest) throw new Error('PCM calibration artifact mismatch: ' + name);
  }
  return calibration;
}
export function samePcmBrowser(calibration, actual) {
  return ['product', 'revision', 'sha256'].every(field => actual?.[field] && calibration.executed_browser?.[field] === actual[field]);
}
