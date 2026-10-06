// Separate V2 launcher/HTTP adapter; reviewed Clock13 launcher stays unchanged.
import { spawn } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import { readFileSync, realpathSync, readlinkSync } from 'node:fs';
import { createServer } from 'node:http';
import { dirname, join } from 'node:path';
import { NativeRpc, sha256 } from './native-pcm-bridge.mjs';
import { nativeNs } from './native-pcm-clock-bounds.mjs';
import { canonical, replayRequest, uniqueJson } from './native-pcm-replay-contract.mjs';

export const V2_RUNTIME = Object.freeze({ nativeSha256: '328d63a837a1db474fd3078ade33f7082a97daf029be1e4e641a5ba448966baf', librarySha256: 'ce07b3578b14e1d25ed603670f2336cd2b32b7f24c2c5b9aab8bbeb0f410b8d6' });
const requireThat = (condition, message) => { if (!condition) throw Error(message); };
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
export class V2StageTimeout extends Error {
  constructor(stage, budgetMs, beganNs) {
    super('owned V2 stage timed out: ' + stage + ' after ' + budgetMs + 'ms');
    this.evidence = { stage, budgetMs, beganNs: beganNs.toString(), endedNs: process.hrtime.bigint().toString(), clock: 'Node hrtime; deadline diagnostic only, not native PCM clock calibration' };
    this.evidence.elapsedMs = Number(BigInt(this.evidence.endedNs) - beganNs) / 1e6;
  }
}
export async function withV2Deadline(stage, budgetMs, operation) {
  requireThat(typeof stage === 'string' && Number.isSafeInteger(budgetMs) && budgetMs > 0 && budgetMs <= 500000 && typeof operation === 'function', 'bounded V2 stage deadline required');
  const began = process.hrtime.bigint(), deadline = began + BigInt(budgetMs) * 1000000n; let timer;
  const check = () => { if (process.hrtime.bigint() >= deadline) throw new V2StageTimeout(stage, budgetMs, began); };
  try {
    const value = await Promise.race([Promise.resolve().then(() => { check(); return operation(); }), new Promise((_, reject) => { timer = setTimeout(() => reject(new V2StageTimeout(stage, budgetMs, began)), budgetMs); })]);
    // Timers cannot run while synchronous code blocks Node's event loop.
    // A late start or successful resolution still has to meet actual elapsed time.
    check(); return value;
  }
  finally { clearTimeout(timer); }
}
export function v2StageBudgets(totalSeconds, audioHoldMs) {
  requireThat(Number.isSafeInteger(totalSeconds) && totalSeconds >= 21 && totalSeconds <= 361 && [0, 50, 200, 500].includes(audioHoldMs), 'whole finite V2 duration/hold required for deadline');
  return Object.freeze({ launchMs: 35000, connectMs: 15000, provenanceMs: 10000, pageMs: 15000, navigateMs: 15000, prepareMs: 120000, finishMs: totalSeconds * 1000 + audioHoldMs + 20000, disposeMs: 10000, browserGraceMs: 5000, browserForceMs: 5000, bridgeCloseMs: 5000, nativeCloseMs: 10000 });
}
export function browserProcessIdentity(pid) {
  requireThat(Number.isSafeInteger(pid) && pid > 0, 'fresh owned browser child PID required');
  const stat = () => { const text = readFileSync(`/proc/${pid}/stat`, 'utf8'); return text.slice(text.lastIndexOf(')') + 2).trim().split(/\s+/); };
  const before = stat(), executable = readlinkSync(`/proc/${pid}/exe`), executableSha256 = sha256(`/proc/${pid}/exe`), after = stat();
  requireThat(before[19] === after[19] && /^\d+$/.test(after[19] ?? ''), 'owned browser process changed during fresh identity measurement');
  return { pid, parentPid: Number(after[1]), processGroup: Number(after[2]), sessionId: Number(after[3]), startTicks: after[19], executable, executableSha256 };
}
export function sameOwnedBrowserIdentity(expected, current) {
  return expected?.parentPid === process.pid && expected?.processGroup === expected.pid && expected?.sessionId === expected.pid && ['pid', 'parentPid', 'processGroup', 'sessionId', 'startTicks', 'executable', 'executableSha256'].every(key => expected[key] === current?.[key]);
}
export function captureOwnedBrowser(server) {
  const child = server.process(), identity = browserProcessIdentity(child?.pid);
  requireThat(sameOwnedBrowserIdentity(identity, identity), 'browser server child is not the newly owned detached process group');
  let terminal;
  child.once('exit', (code, signal) => { terminal = { code, signal, terminalEvent: 'exit' }; });
  return { child, server, identity, currentIdentity: () => browserProcessIdentity(child.pid), terminal: () => terminal ?? (child.exitCode !== null || child.signalCode !== null ? { code: child.exitCode, signal: child.signalCode, terminalEvent: 'ChildProcess exit state' } : undefined) };
}
export async function closeV2Resources({ dispose, browser, server, ownership, bridge, native, budgets, onFailure }) {
  const cleanup = {}; let browserDisposal;
  const attempt = async (stage, ms, operation) => {
    try { return { ok: true, value: await withV2Deadline(stage, ms, operation) }; }
    catch (error) { onFailure(error); return { ok: false }; }
  };
  if (dispose) { const disposed = await attempt('browser-dispose', budgets.disposeMs, dispose); browserDisposal = disposed.value; }
  if (browser || server || ownership) {
    const graceful = await attempt('browser-close', budgets.browserGraceMs, async () => { await browser?.close(); await (ownership?.server ?? server)?.close(); });
    cleanup.browser = { closed: !browser || !browser.isConnected(), processExited: !!ownership?.terminal(), graceful: graceful.ok, forced: false, ownedIdentity: ownership?.identity };
    if (ownership && (!graceful.ok || !ownership.terminal())) {
      const forced = await attempt('browser-force-kill', budgets.browserForceMs, async () => {
        if (ownership.terminal()) return;
        const current = ownership.currentIdentity();
        requireThat(sameOwnedBrowserIdentity(ownership.identity, current), 'refusing browser group kill: fresh PID/start/executable identity differs');
        cleanup.browser.forceIdentity = current; cleanup.browser.forced = true;
        // Public Playwright BrowserServer.kill owns the detached child tree.
        // Never signal a stored PID or a group without the fresh identity fence.
        await ownership.server.kill(); requireThat(ownership.terminal(), 'owned browser process did not exit after force kill');
      });
      cleanup.browser.forceCompleted = forced.ok;
    }
    cleanup.browser.closed = !browser || !browser.isConnected(); cleanup.browser.processExited = !!ownership?.terminal(); cleanup.browser.exit = ownership?.terminal();
  }
  if (bridge) { const closed = await attempt('http-close', budgets.bridgeCloseMs, () => bridge.close()); cleanup.bridge = { closed: closed.ok }; }
  if (native) { const closed = await attempt('native-close', budgets.nativeCloseMs, () => native.close()); cleanup.native = closed.value; }
  return { cleanup, browserDisposal };
}
export function v2NativeArgv({ video, mic, source }) {
  requireThat([video, mic, source].every(value => typeof value === 'string' && value.startsWith('/')), 'absolute frozen input paths required');
  return ['--allow-test-audio-hold', '--peer0', video, mic, source, '127.0.0.1'];
}

export function validateV2Greeting(pair, greeting, { binarySha256, videoSha256, library }) {
  replayRequest(pair, 0); const p = greeting?.provenance;
  requireThat(greeting?.ready === true && p?.instrument === 'fixed-native-peer0-v2' && p.binary_sha256 === binarySha256 && p.video?.archive_sha256 === videoSha256 && p.comparison_available === false && p.pcm_latency_calibrated === false, 'actual V2 greeting/source/binary binding differs');
  requireThat(p.decoder?.version === 'libopus 1.6.1' && p.decoder.sha256 === pair.provenance.library_sha256 && p.decoder.sha256 === sha256(library) && realpathSync(p.decoder.path) === realpathSync(library), 'actual mapped V2 decoder differs');
  for (const archive of pair.archives) requireThat(p[archive.role]?.import_verified === true && p[archive.role].archive_sha256 === archive.archive_sha256 && canonical(p[archive.role].metadata) === canonical(archive.metadata), 'actual V2 native complete archive import differs');
  return p;
}

export async function startV2Native({ binary, binarySha256, video, mic, source, library, startupTimeoutMs = 120000 }) {
  requireThat(sha256(binary) === binarySha256 && Number.isInteger(startupTimeoutMs) && startupTimeoutMs > 0 && startupTimeoutMs <= 120000, 'frozen native executable/startup deadline differs');
  const rpc = new NativeRpc();
  rpc.child = spawn(binary, v2NativeArgv({ video, mic, source }), { env: { ...process.env, LD_LIBRARY_PATH: dirname(library) }, stdio: ['pipe', 'pipe', 'pipe'] });
  rpc.child.once('error', error => { rpc.fail(error); if (!rpc.child.pid) { rpc.exited = { code: null, signal: null, spawnError: String(error), terminalEvent: 'spawn-error' }; rpc.exit.resolve(rpc.exited); } });
  rpc.child.stdin.on('error', error => rpc.fail(error));
  rpc.child.once('exit', (code, signal) => { rpc.exited = { code, signal, terminalEvent: 'exit' }; rpc.exit.resolve(rpc.exited); if (!rpc.closing || code !== 0) rpc.fail(Error(`native V2 peer exited ${code}/${signal}`)); });
  rpc.child.once('close', (code, signal) => { if (!rpc.exited) { rpc.exited = { code, signal, terminalEvent: 'close' }; rpc.exit.resolve(rpc.exited); } });
  rpc.child.stdout.setEncoding('utf8');
  rpc.child.stdout.on('data', chunk => {
    rpc.buffer += chunk; if (rpc.buffer.length > 1024 * 1024) return rpc.fail(Error('native V2 RPC line exceeds bound'));
    let end;
    while ((end = rpc.buffer.indexOf('\n')) >= 0) {
      const line = rpc.buffer.slice(0, end); rpc.buffer = rpc.buffer.slice(end + 1);
      try { const value = JSON.parse(line); if (!rpc.greeted) { rpc.greeted = true; rpc.greeting.resolve(value); } else if (rpc.pending) { const pending = rpc.pending; rpc.pending = undefined; pending.resolve(value); } else rpc.fail(Error('unsolicited native V2 response')); }
      catch (error) { rpc.fail(error); }
    }
  });
  rpc.child.stderr.setEncoding('utf8'); rpc.child.stderr.on('data', chunk => { rpc.stderr = (rpc.stderr + chunk).slice(-65536); });
  let timer;
  try {
    const greeting = await Promise.race([rpc.greeting.promise, new Promise((_, reject) => { timer = setTimeout(() => reject(Error('native V2 greeting deadline')), startupTimeoutMs); })]);
    requireThat(greeting.ready === true && greeting.provenance?.binary_sha256 === binarySha256 && sha256(`/proc/${rpc.child.pid}/exe`) === binarySha256, 'actual mapped V2 executable differs');
    rpc.actualGreeting = greeting; rpc.provenance = greeting.provenance;
    const paths = [...new Set(readFileSync(`/proc/${rpc.child.pid}/maps`, 'utf8').split('\n').filter(line => line.includes('libopus.so')).map(line => line.trim().split(/\s+/).at(-1)).filter(path => path.startsWith('/')).map(path => realpathSync(path)))];
    requireThat(paths.length === 1 && paths[0] === realpathSync(library) && sha256(paths[0]) === sha256(library), 'actual child Opus mapping differs');
    rpc.actualMappedLibrary = { path: paths[0], sha256: sha256(paths[0]) }; return rpc;
  } catch (error) { await rpc.close(); throw error; }
  finally { clearTimeout(timer); }
}

export function v2BrowserRequest(request) {
  requireThat(request && typeof request === 'object' && !Array.isArray(request), 'native V2 request object required');
  const fields = { create: ['op', 'peer', 'publish'], offer: ['op', 'peer'], remote: ['op', 'peer', 'description'], ice: ['op', 'peer', 'candidate'], status: ['op'] }[request.op];
  requireThat(fields && Object.keys(request).length === fields.length && fields.every(key => Object.hasOwn(request, key)), 'browser native V2 operation/keys forbidden; start belongs to reviewed Node preflight');
  if (request.op === 'status') return request;
  requireThat(request.peer === 'publish', 'only owned native publisher allowed');
  if (request.op === 'create') requireThat(request.publish === true, 'only actual native publication allowed');
  if (request.op === 'remote') requireThat(request.description?.type === 'answer' && typeof request.description.sdp === 'string' && request.description.sdp.length <= 262144, 'bounded actual browser answer required');
  if (request.op === 'ice') requireThat(request.candidate && typeof request.candidate === 'object' && JSON.stringify(request.candidate).length <= 4096, 'bounded actual browser ICE candidate required');
  return request;
}

export async function startV2Bridge(rpc, { staticDirectory, clockDelayMs = 0 }) {
  requireThat(Number.isFinite(clockDelayMs) && clockDelayMs >= 0 && clockDelayMs <= 1000, 'bounded test clock delay required');
  const token = randomBytes(32).toString('hex'); let clockCalls = 0, stage = 'fresh'; const signaling = [];
  const names = new Set(['native-pcm-observer.mjs', 'native-pcm-ring.mjs', 'native-pcm-worklet.mjs', 'pcm-kernel.mjs', 'native-peer-checks.mjs', 'native-pcm-v2-receiver.mjs', 'native-pcm-v2-browser.mjs']);
  const server = createServer(async (req, res) => {
    res.setHeader('cross-origin-opener-policy', 'same-origin'); res.setHeader('cross-origin-embedder-policy', 'require-corp'); res.setHeader('cache-control', 'no-store');
    try {
      if (req.method === 'GET' && req.url === '/') { res.setHeader('content-type', 'text/html'); res.end('<!doctype html><title>Owned finite native PCM V2 instrument</title>'); return; }
      if (req.method === 'GET' && names.has(req.url?.slice(1))) { res.setHeader('content-type', 'text/javascript'); res.end(readFileSync(join(staticDirectory, req.url.slice(1)))); return; }
      if (req.headers.authorization !== 'Bearer ' + token) { res.writeHead(403).end(); return; }
      let value;
      if (req.method === 'GET' && req.url === '/clock') {
        if (clockDelayMs) await sleep(clockDelayMs); value = await rpc.call({ op: 'clock' }); requireThat(value.clock === 'CLOCK_MONOTONIC', 'actual V2 native clock domain differs'); nativeNs(value.monoNs); clockCalls++;
      } else if (req.method === 'POST' && req.url === '/rpc') {
        let body = ''; for await (const chunk of req) { body += chunk; requireThat(Buffer.byteLength(body) <= 300000, 'bounded V2 request body required'); }
        const request = v2BrowserRequest(uniqueJson(body));
        if (request.op === 'create') requireThat(stage === 'fresh', 'one native publisher creation only');
        if (request.op === 'offer') requireThat(stage === 'created', 'one actual native offer only');
        if (request.op === 'remote') requireThat(stage === 'offered', 'one actual answer before replay only');
        if (request.op === 'ice') requireThat(stage === 'answered', 'ICE requires actual answered publisher');
        value = await rpc.call(request);
        if (request.op !== 'status') signaling.push({ request, response: value });
        if (request.op === 'create') stage = 'created'; if (request.op === 'offer') stage = 'offered'; if (request.op === 'remote') stage = 'answered';
      } else { res.writeHead(404).end(); return; }
      res.setHeader('content-type', 'application/json'); res.end(JSON.stringify(value));
    } catch (error) { res.writeHead(500, { 'content-type': 'application/json' }); res.end(JSON.stringify({ error: String(error) })); }
  });
  server.requestTimeout = 35000; server.headersTimeout = 5000;
  await new Promise((resolve, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', resolve); });
  return { token, url: `http://127.0.0.1:${server.address().port}`, signaling, clockCalls: () => clockCalls, close: async () => { server.closeAllConnections(); await new Promise(resolve => server.close(resolve)); } };
}
