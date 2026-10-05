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
