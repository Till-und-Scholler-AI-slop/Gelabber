import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { NativeVideo, decodedVideo, nativeExitEvidence, rethrowAfterCleanup, janusSessionAbsence } from '../native-video.mjs';

const sample = (second, change = {}) => ({ stats: [{ type: 'inbound-rtp', kind: 'video', id: 'video-edge', timestamp: second * 1000,
  bytesReceived: second * 500000, packetsReceived: second * 440, packetsLost: 0, framesDecoded: second * 60, frameWidth: 1920, frameHeight: 1080, ...change }] });
test('decoded fixed source qualifies only actual continuous 1080p60 payload', () => {
  assert.equal(decodedVideo([sample(1), sample(2), sample(3)], 4000000).valid, true);
  for (const changed of [{ framesDecoded: 120 }, { bytesReceived: 500000 }, { frameWidth: 1280 }, { packetsLost: 30 }, { id: 'replacement' }]) {
    assert.equal(decodedVideo([sample(1), sample(2), sample(3, changed)], 4000000).valid, false);
  }
});
test('an aggregate decoder delta cannot hide a stalled middle interval', () => {
  assert.equal(decodedVideo([sample(1), sample(2, { framesDecoded: 60 }), sample(3)], 4000000).valid, false);
});
test('missing or duplicate receiver edges never qualify', () => {
  assert.equal(decodedVideo([sample(1), { stats: [] }, sample(3)], 4000000).valid, false);
  const doubled = sample(3); doubled.stats.push({ ...doubled.stats[0], id: 'extra' });
  assert.equal(decodedVideo([sample(1), doubled], 4000000).valid, false);
});
test('mediasoup probator and repair packets are not extra decoded source edges', () => {
  const samples = [sample(1), sample(2), sample(3)];
  for (const value of samples) value.stats.push({ type: 'codec', id: 'repair', mimeType: 'video/rtx' },
    { type: 'inbound-rtp', kind: 'video', packetsReceived: 10, mid: 'probator' },
    { type: 'inbound-rtp', kind: 'video', packetsReceived: 20, codecId: 'repair' });
  assert.equal(decodedVideo(samples, 4000000).valid, true);
});

test('private source RPC serializes concurrent requests and closes only its owned child', async () => {
  const folder = fs.mkdtempSync(path.join(os.tmpdir(), 'gelabber-native-source-test-'));
  const executable = path.join(folder, 'fake-source');
  fs.writeFileSync(executable, `#!${process.execPath}\nconsole.log(JSON.stringify({ready:true,provenance:{test:true}}));\nlet i=0;require('node:readline').createInterface({input:process.stdin}).on('line',line=>{const x=JSON.parse(line);if(x.op==='close')process.exit(0);console.log(JSON.stringify({index:i++}));});\n`, { mode: 0o700 });
  const source = new NativeVideo(executable, 'unused-test-input');
  try {
    await source.ready;
    const results = await Promise.all([source.call({ op: 'status' }), source.call({ op: 'status' })]);
    assert.deepEqual(results, [{ index: 0 }, { index: 1 }]);
    await source.close();
    assert.equal(source.child.exitCode, 0);
  } finally { await source.close(); fs.rmSync(folder, { recursive: true }); }
});
test('missing native executable rejects startup and needs no nonexistent-child cleanup', async () => {
  const source = new NativeVideo('/definitely-missing-gelabber-source', 'unused-test-input');
  await assert.rejects(source.ready);
  await source.close();
});
test('native shutdown requires zero exit and no forced termination signal', () => {
  assert.equal(nativeExitEvidence({ exitCode: 0, signalCode: null }).clean, true);
  for (const child of [{ exitCode: 7, signalCode: null }, { exitCode: null, signalCode: 'SIGTERM' }, { exitCode: null, signalCode: 'SIGKILL' }]) assert.equal(nativeExitEvidence(child).clean, false);
});
test('failed setup and failed cleanup retain both causes and cleanup evidence', async () => {
  const setup = new Error('offer failed'), cleanup = new Error('destroy failed'); cleanup.cleanup_evidence = { sessions: [1] };
  await assert.rejects(rethrowAfterCleanup(setup, async () => { throw cleanup; }), error => {
    assert.deepEqual(error.errors, [setup, cleanup]); assert.equal(error.setup_error, String(setup));
    assert.deepEqual(error.cleanup_errors, [String(cleanup)]); assert.deepEqual(error.cleanup_evidence, { sessions: [1] }); return true;
  });
  await assert.rejects(rethrowAfterCleanup(setup, async () => {}), error => error === setup);
});
test('only explicit Janus 404 or error458 proves destroyed session absence', async () => {
  assert.deepEqual(await janusSessionAbsence(new Response('', { status: 404 })), { absent: true, status: 404 });
  assert.equal((await janusSessionAbsence(Response.json({ janus: 'error', error: { code: 458 } }))).absent, true);
  assert.equal((await janusSessionAbsence(Response.json({ janus: 'success' }))).absent, false);
  await assert.rejects(janusSessionAbsence(new Response('', { status: 200 })), SyntaxError);
});
