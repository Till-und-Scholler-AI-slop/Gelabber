import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { readPcmCalibration, samePcmBrowser } from '../pcm-policy.mjs';

test('PCM calibration is bound to measured browser and actual detector artifacts', () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'gelabber-pcm-policy-'));
  try {
    const files = ['pcm-kernel.mjs', 'pcm-marker.mjs', 'pcm.bundle.js', 'client.bundle.js'];
    files.forEach(name => fs.writeFileSync(path.join(directory, name), name));
    const actual = { product: 'HeadlessChrome/153.0.8010.12', revision: '@fixture', sha256: 'a'.repeat(64) };
    const calibration = { valid: true, failures: [], delay_checks: Array.from({ length: 4 }, () => ({ valid: true })), executed_browser: actual,
      artifact_sha256: Object.fromEntries(files.map(name => [name, createHash('sha256').update(name).digest('hex')])) };
    const filename = path.join(directory, 'calibration.json'); fs.writeFileSync(filename, JSON.stringify(calibration));
    assert.equal(readPcmCalibration(filename, directory).valid, true);
    assert.equal(samePcmBrowser(calibration, actual), true);
    assert.equal(samePcmBrowser(calibration, { ...actual, revision: '@different' }), false);
    for (const name of files) {
      fs.writeFileSync(path.join(directory, name), 'different');
      assert.throws(() => readPcmCalibration(filename, directory), /artifact mismatch/);
      fs.writeFileSync(path.join(directory, name), name);
    }
    fs.writeFileSync(filename, JSON.stringify({ ...calibration, valid: false }));
    assert.throws(() => readPcmCalibration(filename, directory), /did not pass/);
  } finally { fs.rmSync(directory, { recursive: true, force: true }); }
});
