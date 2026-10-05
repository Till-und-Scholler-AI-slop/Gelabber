import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, writeFile, rm } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { NativeRpc, startBridge, sha256 } from '../native-pcm-bridge.mjs';

test('actual existing hashed nonexecutable program fails EACCES without hanging on missing exit', async () => {
  const folder = await mkdtemp(join(tmpdir(), 'gelabber-native-pcm-spawn-negative-')), binary = join(folder, 'not-executable');
  await writeFile(binary, '#!/bin/sh\nexit 0\n', { mode: 0o644 });
  let timer;
  try {
    await assert.rejects(Promise.race([
      NativeRpc.start({ binary, binarySha256: sha256(binary), video: '/unused', mic: '/unused', source: '/unused', libraryDirectory: '/unused', startupTimeoutMs: 500 }),
      new Promise((_, reject) => { timer = setTimeout(() => reject(Error('cleanup hung instead of EACCES')), 2000); }),
    ]), /EACCES/);
  } finally { clearTimeout(timer); await rm(folder, { recursive: true }); }
});
test('close cancels both pending and queued requests with bounded owned cleanup', async () => {
  const rpc = new NativeRpc();
  rpc.child = { stdin: { write() {}, end() { rpc.exited = { code: 0, signal: null }; rpc.exit.resolve(rpc.exited); } } };
  const first = rpc.call({ op: 'clock' }), second = rpc.call({ op: 'status' }), settled = Promise.allSettled([first, second]);
  await new Promise(resolve => setImmediate(resolve));
  const cleanup = await rpc.close({ queueTimeoutMs: 10, signalTimeoutMs: 10, killTimeoutMs: 10 });
  assert.equal(cleanup.code, 0); assert.ok((await settled).every(row => row.status === 'rejected')); assert.equal(rpc.pending, undefined); assert.equal(rpc.queued, 0);
});
test('false kill and absent close/exit cannot create an unbounded cleanup wait', async () => {
  const rpc = new NativeRpc(); let destroyed = 0, unref = 0;
  rpc.child = { stdin: { end() {}, destroy() { destroyed++; } }, stdout: { destroy() { destroyed++; } }, stderr: { destroy() { destroyed++; } }, kill() { return false; }, unref() { unref++; } };
  const cleanup = await rpc.close({ queueTimeoutMs: 10, signalTimeoutMs: 10, killTimeoutMs: 10 });
  assert.equal(cleanup.cleanupTimedOut, true); assert.equal(cleanup.killSignalAccepted, false); assert.match(cleanup.failure, /timed out/); assert.equal(destroyed, 3); assert.equal(unref, 1);
});

test('native RPC calls serialize before the next request reaches the owned child', async () => {
  const rpc = new NativeRpc(), written = [];
  rpc.child = { stdin: { write: line => written.push(JSON.parse(line)) } };
  const first = rpc.call({ op: 'clock' }), second = rpc.call({ op: 'status' });
  await new Promise(resolve => setImmediate(resolve)); assert.deepEqual(written, [{ op: 'clock' }]);
  rpc.pending.resolve({ monoNs: '1' }); await first;
  await new Promise(resolve => setImmediate(resolve)); assert.deepEqual(written, [{ op: 'clock' }, { op: 'status' }]);
  rpc.pending.resolve({ peers: {} }); await second; assert.equal(rpc.queued, 0);
});
test('native RPC timeout poisons the queue and cannot pair a stale response to another call', async () => {
  const rpc = new NativeRpc(); rpc.child = { stdin: { write() {} } };
  const first = rpc.call({ op: 'clock' }, 10), second = rpc.call({ op: 'status' });
  const settled = await Promise.allSettled([first, second]); assert.ok(settled.every(row => row.status === 'rejected')); assert.match(rpc.failure, /timed out/);
  assert.equal(rpc.pending, undefined); await assert.rejects(rpc.call({ op: 'clock' }), /timed out/);
});
test('bridge only listens on loopback and requires exact private bearer token for clocks', async () => {
  const calls = [], bridge = await startBridge({ call: async request => { calls.push(request); return { clock: 'CLOCK_MONOTONIC', monoNs: '1' }; } }, { staticDirectory: '/unused' });
  try {
    assert.match(bridge.url, /^http:\/\/127\.0\.0\.1:/);
    const missing = await fetch(bridge.url + '/clock'); assert.equal(missing.status, 403);
    const root = await fetch(bridge.url); assert.equal(root.headers.get('cross-origin-opener-policy'), 'same-origin'); assert.equal(root.headers.get('cross-origin-embedder-policy'), 'require-corp');
    const response = await fetch(bridge.url + '/clock', { headers: { authorization: 'Bearer ' + bridge.token } }); assert.deepEqual(await response.json(), { clock: 'CLOCK_MONOTONIC', monoNs: '1' }); assert.deepEqual(calls, [{ op: 'clock' }]);
  } finally { await bridge.close(); }
});
test('clock-only HTTP replay request is refused before any native create/offer call', async () => {
  const calls = [], bridge = await startBridge({ call: async request => { calls.push(request); return {}; } }, { staticDirectory: '/unused' });
  try {
    const response = await fetch(bridge.url + '/rpc', { method: 'POST', headers: { authorization: 'Bearer ' + bridge.token }, body: JSON.stringify({ op: 'create', peer: 'publish', publish: true }) });
    assert.equal(response.status, 500); assert.deepEqual(calls, []);
  } finally { await bridge.close(); }
});
test('bridge rejects a foreign clock domain and noncanonical native clock values', async () => {
  for (const clock of [{ clock: 'REALTIME', monoNs: '1' }, { clock: 'CLOCK_MONOTONIC', monoNs: '01' }]) {
    const bridge = await startBridge({ call: async () => clock }, { staticDirectory: '/unused' });
    try { assert.equal((await fetch(bridge.url + '/clock', { headers: { authorization: 'Bearer ' + bridge.token } })).status, 500); }
    finally { await bridge.close(); }
  }
});
