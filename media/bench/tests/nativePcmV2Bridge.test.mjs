import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, writeFile, rm } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { V2StageTimeout, withV2Deadline, v2StageBudgets, browserProcessIdentity, sameOwnedBrowserIdentity, closeV2Resources, v2NativeArgv, v2BrowserRequest, startV2Native, startV2Bridge } from '../native-pcm-v2-bridge.mjs';
import { parseV2Options } from '../native-pcm-v2-control.mjs';
import { sha256 } from '../native-pcm-bridge.mjs';

test('V2 static starter places the actual parser hold flag BEFORE peer0 with loopback bind', () => {
  assert.deepEqual(v2NativeArgv({ video: '/video', mic: '/mic', source: '/source' }), ['--allow-test-audio-hold', '--peer0', '/video', '/mic', '/source', '127.0.0.1']);
  assert.throws(() => v2NativeArgv({ video: 'relative', mic: '/mic', source: '/source' }), /absolute/);
});
test('V2 failed executable spawn inherits bounded EACCES cleanup without launching any native peer', async () => {
  const folder = await mkdtemp(join(tmpdir(), 'gelabber-native-pcm-v2-eacces-')), binary = join(folder, 'not-executable'); let timer;
  await writeFile(binary, '#!/bin/sh\nexit 0\n', { mode: 0o644 });
  try {
    await assert.rejects(Promise.race([startV2Native({ binary, binarySha256: sha256(binary), video: '/unused', mic: '/unused', source: '/unused', library: '/unused/libopus.so', startupTimeoutMs: 500 }), new Promise((_, reject) => { timer = setTimeout(() => reject(Error('V2 cleanup hung')), 2000); })]), /EACCES/);
  } finally { clearTimeout(timer); await rm(folder, { recursive: true }); }
});
test('browser V2 whitelist refuses finite start, close, foreign peer and extra legacy keys', () => {
  for (const request of [{ op: 'start', peer: 'publish', total_seconds: 21, audio_hold_ms: 0 }, { op: 'close' }, { op: 'create', peer: 'foreign', publish: true }, { op: 'create', peer: 'publish', publish: false }, { op: 'offer', peer: 'publish', loop: true }, { op: 'status', peer: 'publish' }, { op: 'remote', peer: 'publish', description: { type: 'offer', sdp: 'v=0' } }]) assert.throws(() => v2BrowserRequest(request));
  assert.deepEqual(v2BrowserRequest({ op: 'create', peer: 'publish', publish: true }), { op: 'create', peer: 'publish', publish: true });
  assert.deepEqual(v2BrowserRequest({ op: 'status' }), { op: 'status' });
});
test('V2 bridge enforces private loopback, ordered single signaling and Node-owned start', async () => {
  const calls = [], bridge = await startV2Bridge({ call: async request => { calls.push(request); return request.op === 'clock' ? { clock: 'CLOCK_MONOTONIC', monoNs: '1' } : { ok: true }; } }, { staticDirectory: '/unused' });
  const post = request => fetch(bridge.url + '/rpc', { method: 'POST', headers: { authorization: 'Bearer ' + bridge.token }, body: JSON.stringify(request) });
  try {
    assert.match(bridge.url, /^http:\/\/127\.0\.0\.1:/);
    assert.equal((await fetch(bridge.url + '/clock')).status, 403);
    assert.equal((await post({ op: 'offer', peer: 'publish' })).status, 500); assert.equal(calls.length, 0);
    assert.equal((await post({ op: 'start', peer: 'publish', total_seconds: 21, audio_hold_ms: 200 })).status, 500); assert.equal(calls.length, 0);
    assert.equal((await post({ op: 'create', peer: 'publish', publish: true })).status, 200);
    assert.equal((await post({ op: 'create', peer: 'publish', publish: true })).status, 500);
    assert.equal((await post({ op: 'offer', peer: 'publish' })).status, 200);
    assert.equal((await post({ op: 'remote', peer: 'publish', description: { type: 'answer', sdp: 'v=0' } })).status, 200);
    const clock = await fetch(bridge.url + '/clock', { headers: { authorization: 'Bearer ' + bridge.token } }); assert.equal(clock.status, 200); assert.equal(bridge.clockCalls(), 1);
    assert.deepEqual(calls.map(value => value.op), ['create', 'offer', 'remote', 'clock']); assert.equal(bridge.signaling.length, 3);
    const root = await fetch(bridge.url); assert.equal(root.headers.get('cross-origin-opener-policy'), 'same-origin'); assert.equal(root.headers.get('cross-origin-embedder-policy'), 'require-corp');
  } finally { await bridge.close(); }
});
test('V2 bridge rejects duplicate JSON request keys, invalid clocks and oversized bodies', async () => {
  const calls = [], bridge = await startV2Bridge({ call: async request => { calls.push(request); return { clock: 'REALTIME', monoNs: '01' }; } }, { staticDirectory: '/unused' });
  try {
    const headers = { authorization: 'Bearer ' + bridge.token };
    for (const body of ['{"op":"create","op":"offer","peer":"publish","publish":true}', ' '.repeat(300001)]) assert.equal((await fetch(bridge.url + '/rpc', { method: 'POST', headers, body })).status, 500);
    assert.equal(calls.length, 0); assert.equal((await fetch(bridge.url + '/clock', { headers })).status, 500);
  } finally { await bridge.close(); }
});
test('V2 CLI defaults to read-only, derives duration, and accepts only explicit bounded holds/faults', () => {
  const args = ['--pair', '/pair.json', '--pair-sha256', 'a'.repeat(64), '--generation', '/generation.json', '--generation-sha256', 'b'.repeat(64), '--video', '/video.rtpbin', '--video-sha256', 'c'.repeat(64), '--binary', '/native', '--library', '/libopus.so', '--chromium', '/chrome', '--output', '/output'];
  assert.equal(parseV2Options(args).execute, false); assert.equal(parseV2Options([...args, '--execute']).execute, true);
  for (const hold of ['0', '50', '200', '500']) assert.equal(parseV2Options([...args, '--audio-hold-ms', hold]).audioHoldMs, Number(hold));
  for (const extra of [['--seconds', '20'], ['--total-seconds', '21'], ['--loop'], ['--audio-hold-ms', '25'], ['--audio-hold-ms', ' 0'], ['--capacity', '1'], ['--capacity', '32769'], ['--observer-pause-ms', '-1'], ['--clock-delay-ms', '1001'], ['--suspend-ms', 'NaN']]) assert.throws(() => parseV2Options([...args, ...extra]));
  assert.throws(() => parseV2Options(args.map(value => value === '/pair.json' ? 'relative' : value)), /absolute/);
});
test('Node stage deadline rejects a never-resolving browser promise with retained timing evidence', async () => {
  const started = performance.now();
  await assert.rejects(withV2Deadline('browser-prepare', 15, () => new Promise(() => {})), error => {
    assert.ok(error instanceof V2StageTimeout); assert.equal(error.evidence.stage, 'browser-prepare'); assert.equal(error.evidence.budgetMs, 15); assert.ok(error.evidence.elapsedMs >= 10); assert.ok(BigInt(error.evidence.endedNs) > BigInt(error.evidence.beganNs)); return true;
  });
  assert.ok(performance.now() - started < 1000);
  const budgets = v2StageBudgets(21, 500); assert.equal(budgets.prepareMs, 120000); assert.equal(budgets.finishMs, 41500); assert.ok(budgets.disposeMs <= 10000); assert.equal(budgets.browserGraceMs + budgets.browserForceMs, 10000);
  assert.equal(v2StageBudgets(361, 0).finishMs, 381000); assert.throws(() => v2StageBudgets(3600, 0));
});
const blockNodeLoop = ms => { const end = process.hrtime.bigint() + BigInt(ms) * 1000000n; while (process.hrtime.bigint() < end) {} };
test('actual deadline rejects successful operations and resolutions after a blocked Node loop', async () => {
  for (const operation of [() => { blockNodeLoop(20); return 'late synchronous success'; }, () => new Promise(resolve => setTimeout(() => { resolve('late promise success'); blockNodeLoop(20); }, 0))]) {
    await assert.rejects(withV2Deadline('blocked-loop-resolution', 5, operation), error => {
      assert.ok(error instanceof V2StageTimeout); assert.equal(error.evidence.stage, 'blocked-loop-resolution'); assert.equal(error.evidence.budgetMs, 5); assert.ok(error.evidence.elapsedMs >= 20); return true;
    });
  }
});
test('a microtask delayed beyond its actual deadline never invokes the operation', async () => {
  let called = false;
  const pending = withV2Deadline('late-microtask-start', 5, () => { called = true; return 'must not execute'; });
  blockNodeLoop(20);
  await assert.rejects(pending, error => { assert.ok(error instanceof V2StageTimeout); assert.equal(error.evidence.stage, 'late-microtask-start'); assert.ok(error.evidence.elapsedMs >= 20); return true; });
  assert.equal(called, false);
});
test('fresh process identity reads actual Node exe but only a new owned detached group can match', () => {
  const identity = browserProcessIdentity(process.pid); assert.equal(identity.pid, process.pid); assert.equal(identity.executableSha256, sha256(process.execPath)); assert.match(identity.startTicks, /^\d+$/);
  assert.equal(sameOwnedBrowserIdentity(identity, identity), false);
  const owned = { pid: process.pid + 1000000, parentPid: process.pid, processGroup: process.pid + 1000000, sessionId: process.pid + 1000000, startTicks: '12345', executable: '/synthetic/browser', executableSha256: 'a'.repeat(64) };
  assert.equal(sameOwnedBrowserIdentity(owned, { ...owned }), true);
  for (const key of ['pid', 'parentPid', 'processGroup', 'sessionId', 'startTicks', 'executable', 'executableSha256']) assert.equal(sameOwnedBrowserIdentity(owned, { ...owned, [key]: 'changed' }), false);
});
function cleanupFixture({ foreignIdentity = false, forceHang = false } = {}) {
  const calls = [], failures = [], never = () => new Promise(() => {}); let connected = true, exited = false;
  const pid = process.pid + 1000000, identity = { pid, parentPid: process.pid, processGroup: pid, sessionId: pid, startTicks: '12345', executable: '/synthetic/browser', executableSha256: 'a'.repeat(64) };
  const ownership = { identity, currentIdentity: () => ({ ...identity, startTicks: foreignIdentity ? 'changed' : identity.startTicks }), terminal: () => exited ? { code: null, signal: 'SIGKILL', terminalEvent: 'synthetic unit exit' } : undefined, server: { close: async () => { calls.push('server-close'); }, kill: async () => { calls.push('force-kill'); if (forceHang) return never(); connected = false; exited = true; } } };
  const args = { dispose: () => { calls.push('dispose'); return never(); }, browser: { close: () => { calls.push('browser-close'); return never(); }, isConnected: () => connected }, ownership, bridge: { close: async () => { calls.push('http-close'); } }, native: { close: async () => { calls.push('native-close'); return { code: 0, signal: null }; } }, budgets: { disposeMs: 10, browserGraceMs: 10, browserForceMs: 10, bridgeCloseMs: 10, nativeCloseMs: 10 }, onFailure: error => failures.push(error) };
  return { calls, failures, args };
}
test('hung disposal and browser close force only verified own API and still close HTTP/native', async () => {
  const fixture = cleanupFixture(), started = performance.now(), result = await closeV2Resources(fixture.args);
  assert.deepEqual(fixture.calls, ['dispose', 'browser-close', 'force-kill', 'http-close', 'native-close']);
  assert.deepEqual(fixture.failures.map(error => error.evidence?.stage), ['browser-dispose', 'browser-close']);
  assert.equal(result.cleanup.browser.forced, true); assert.equal(result.cleanup.browser.processExited, true); assert.equal(result.cleanup.bridge.closed, true); assert.equal(result.cleanup.native.code, 0); assert.ok(performance.now() - started < 1000);
});
test('changed PID/start/executable fence refuses force API while subsequent cleanup still runs', async () => {
  const fixture = cleanupFixture({ foreignIdentity: true }), result = await closeV2Resources(fixture.args);
  assert.deepEqual(fixture.calls, ['dispose', 'browser-close', 'http-close', 'native-close']); assert.match(fixture.failures.at(-1).message, /refusing browser group kill/); assert.equal(result.cleanup.browser.forced, false); assert.equal(result.cleanup.browser.processExited, false); assert.equal(result.cleanup.native.code, 0);
});
test('even never-resolving own force API cannot prevent bounded HTTP/native cleanup', async () => {
  const fixture = cleanupFixture({ forceHang: true }), started = performance.now(), result = await closeV2Resources(fixture.args);
  assert.deepEqual(fixture.calls, ['dispose', 'browser-close', 'force-kill', 'http-close', 'native-close']); assert.equal(fixture.failures.at(-1).evidence.stage, 'browser-force-kill'); assert.equal(result.cleanup.browser.forceCompleted, false); assert.equal(result.cleanup.browser.processExited, false); assert.equal(result.cleanup.native.code, 0); assert.ok(performance.now() - started < 1000);
});
test('normal client/server close retains actual terminal event and never calls force API', async () => {
  const calls = [], failures = []; let connected = true, terminal;
  const ownership = { identity: { pid: process.pid + 1000000 }, terminal: () => terminal, server: { close: async () => { calls.push('server-close'); terminal = { code: 0, signal: null, terminalEvent: 'synthetic unit exit' }; }, kill: async () => assert.fail('normal closure must not force kill') } };
  const result = await closeV2Resources({ dispose: async () => ({ cleanup: { audioContext: 'closed' } }), browser: { close: async () => { calls.push('browser-close'); connected = false; }, isConnected: () => connected }, ownership, bridge: { close: async () => calls.push('http-close') }, native: { close: async () => { calls.push('native-close'); return { code: 0, signal: null }; } }, budgets: { disposeMs: 10, browserGraceMs: 10, browserForceMs: 10, bridgeCloseMs: 10, nativeCloseMs: 10 }, onFailure: error => failures.push(error) });
  assert.deepEqual(calls, ['browser-close', 'server-close', 'http-close', 'native-close']); assert.deepEqual(failures, []); assert.equal(result.cleanup.browser.graceful, true); assert.equal(result.cleanup.browser.forced, false); assert.equal(result.cleanup.browser.processExited, true); assert.equal(result.cleanup.browser.exit.code, 0); assert.equal(result.browserDisposal.cleanup.audioContext, 'closed');
});
