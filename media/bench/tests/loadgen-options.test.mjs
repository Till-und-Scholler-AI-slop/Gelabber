import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

test('invalid Firefox protocol logging exits without leaving a listening proxy', () => {
  const entry = fileURLToPath(new URL('../loadgen.mjs', import.meta.url));
  const result = spawnSync(process.execPath, [entry, '--engine', 'current', '--backend', 'http://test.invalid',
    '--output', '/tmp/gelabber-invalid-protocol-options.json', '--browser', 'firefox', '--protocol-logs', 'true'],
  { env: { ...process.env, BENCH_TOKEN: 'test-only-token-not-used-for-network' }, encoding: 'utf8', timeout: 2000 });
  assert.equal(result.error, undefined);
  assert.equal(result.status, 1);
  assert.match(result.stderr, /Chromium RTC event logging required/);
});

test('invalid fixed source controls exit before starting a proxy', () => {
  const entry = fileURLToPath(new URL('../loadgen.mjs', import.meta.url));
  for (const args of [['--browser', 'firefox', '--video', 'true'], ['--video', 'false'], ['--video', 'true', '--video-bitrate', '4000001']]) {
    const result = spawnSync(process.execPath, [entry, '--engine', 'current', '--backend', 'http://test.invalid',
      '--output', '/tmp/gelabber-invalid-fixed-options.json', '--fixed-video-fixture', 'true', ...args],
    { env: { ...process.env, BENCH_TOKEN: 'test-only-token-not-used-for-network' }, encoding: 'utf8', timeout: 2000 });
    assert.equal(result.error, undefined);
    assert.equal(result.status, 1);
    assert.match(result.stderr, /Fixed video fixture requires Chromium video and whole kbit/);
  }
});

test('PCM measurement requires calibration and a sufficient measurement interval before proxy startup', () => {
  const entry = fileURLToPath(new URL('../loadgen.mjs', import.meta.url));
  for (const [args, message] of [[[], /PCM latency requires --pcm-calibration/], [['--duration', '7000'], /at least eight seconds/], [['--browser', 'firefox'], /PCM latency requires Chromium/]]) {
    const result = spawnSync(process.execPath, [entry, '--engine', 'current', '--backend', 'http://test.invalid',
      '--output', '/tmp/gelabber-invalid-pcm-options.json', '--pcm-latency', 'true', ...args],
    { env: { ...process.env, BENCH_TOKEN: 'test-only-token-not-used-for-network' }, encoding: 'utf8', timeout: 2000 });
    assert.equal(result.error, undefined); assert.equal(result.status, 1); assert.match(result.stderr, message);
  }
});
