import { spawn } from 'node:child_process';
import { createHash, randomBytes } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { createServer } from 'node:http';
import { join } from 'node:path';
import { nativeNs } from './native-pcm-clock-bounds.mjs';

export function sha256(path) { return createHash('sha256').update(readFileSync(path)).digest('hex'); }
function deferred() { let resolve, reject; const promise = new Promise((a, b) => { resolve = a; reject = b; }); return { promise, resolve, reject }; }
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
async function boundedWait(promise, ms) {
  let timer;
  try { return await Promise.race([promise, new Promise(resolve => { timer = setTimeout(resolve, ms); })]); }
  finally { clearTimeout(timer); }
}

// Exactly one owned child, serialized line RPCs, bounded responses and cleanup.
// Clock-only mode never submits create/SDP/ICE/start to the native executable.
export class NativeRpc {
  static async start({ binary, binarySha256, video, mic, source, libraryDirectory, allowTestHold = false, startupTimeoutMs = 120000 }) {
    if (sha256(binary) !== binarySha256) throw Error('native executable hash differs');
    const rpc = new NativeRpc();
    rpc.child = spawn(binary, ['--peer0', video, mic, source, '127.0.0.1', ...(allowTestHold ? ['--allow-test-audio-hold'] : [])], { env: { ...process.env, LD_LIBRARY_PATH: libraryDirectory }, stdio: ['pipe', 'pipe', 'pipe'] });
    rpc.child.once('error', error => {
      rpc.fail(error);
      // Failed spawn has no PID and emits error/close, not exit. Treat it as
      // terminal immediately, rather than waiting forever after a false kill.
      if (!rpc.child.pid) { rpc.exited = { code: null, signal: null, spawnError: String(error), terminalEvent: 'spawn-error' }; rpc.exit.resolve(rpc.exited); }
    });
    rpc.child.stdin.on('error', error => rpc.fail(error));
    rpc.child.once('exit', (code, signal) => { rpc.exited = { code, signal, terminalEvent: 'exit' }; rpc.exit.resolve(rpc.exited); if (!rpc.closing || code !== 0) rpc.fail(Error(`native peer exited ${code}/${signal}`)); });
    rpc.child.once('close', (code, signal) => {
      if (!rpc.exited) { rpc.exited = { code, signal, terminalEvent: 'close' }; rpc.exit.resolve(rpc.exited); }
    });
    rpc.child.stdout.setEncoding('utf8');
    rpc.child.stdout.on('data', chunk => {
      rpc.buffer += chunk;
      if (rpc.buffer.length > 1024 * 1024) return rpc.fail(Error('native RPC line exceeds bound'));
      let end;
      while ((end = rpc.buffer.indexOf('\n')) >= 0) {
        const line = rpc.buffer.slice(0, end); rpc.buffer = rpc.buffer.slice(end + 1);
        try {
          const value = JSON.parse(line);
          if (!rpc.greeted) { rpc.greeted = true; rpc.greeting.resolve(value); }
          else if (rpc.pending) { const pending = rpc.pending; rpc.pending = undefined; pending.resolve(value); }
          else rpc.fail(Error('unsolicited native RPC response'));
        } catch (error) { rpc.fail(error); }
      }
    });
    rpc.child.stderr.setEncoding('utf8');
    rpc.child.stderr.on('data', chunk => { rpc.stderr = (rpc.stderr + chunk).slice(-65536); });
    let timer;
    try {
      const greeting = await Promise.race([rpc.greeting.promise, new Promise((_, reject) => { timer = setTimeout(() => reject(Error('native greeting timed out')), startupTimeoutMs); })]);
      if (greeting.ready !== true || greeting.provenance?.binary_sha256 !== binarySha256 || sha256(`/proc/${rpc.child.pid}/exe`) !== binarySha256) throw Error('mapped native executable provenance differs');
      rpc.provenance = greeting.provenance; return rpc;
    } catch (error) { await rpc.close(); throw error; }
    finally { clearTimeout(timer); }
  }
  constructor() {
    this.buffer = ''; this.stderr = ''; this.greeting = deferred(); this.exit = deferred(); this.tail = Promise.resolve(); this.queued = 0; this.calls = [];
    // A startup failure may arrive before start() begins awaiting the greeting.
    this.greeting.promise.catch(() => {});
  }
  fail(error) {
    if (!this.failure) this.failure = String(error);
    this.greeting.reject(error); this.pending?.reject(error); this.pending = undefined;
  }
  call(request, timeoutMs = 30000) {
    if (this.closing || this.failure || this.queued >= 32 || request.op === 'close') return Promise.reject(Error(this.failure ?? 'native RPC unavailable/bounded queue exceeded'));
    this.queued++;
    const operation = this.tail.then(async () => {
      if (this.closing || this.failure) throw Error(this.failure ?? 'native RPC closed');
      const pending = deferred(); this.pending = pending;
      let timer;
      try {
        this.child.stdin.write(JSON.stringify(request) + '\n'); this.calls.push(request.op);
        if (this.calls.length > 2000000) throw Error('native RPC call evidence bound exceeded');
        const result = await Promise.race([pending.promise, new Promise((_, reject) => { timer = setTimeout(() => reject(Error('native RPC timed out')), timeoutMs); })]);
        if (result.error) throw Error('native RPC: ' + result.error);
        return result;
      } catch (error) { this.fail(error); throw error; }
      finally { clearTimeout(timer); this.pending = undefined; }
    });
    this.tail = operation.catch(() => {});
    return operation.finally(() => this.queued--);
  }
  async close({ queueTimeoutMs = 2500, signalTimeoutMs = 5000, killTimeoutMs = 2000 } = {}) {
    if (this.closePromise) return this.closePromise;
    if (![queueTimeoutMs, signalTimeoutMs, killTimeoutMs].every(value => Number.isFinite(value) && value > 0 && value <= 5000)) throw Error('invalid bounded native cleanup policy');
    this.closePromise = (async () => {
      this.closing = true;
      await boundedWait(this.tail, queueTimeoutMs);
      this.pending?.reject(Error('owned RPC cancelled during cleanup')); this.pending = undefined;
      if (!this.exited) { if (!this.child.stdin.destroyed) this.child.stdin.end('{"op":"close"}\n'); await boundedWait(this.exit.promise, signalTimeoutMs); }
      let killSignalAccepted;
      if (!this.exited) { killSignalAccepted = this.child.kill('SIGKILL'); await boundedWait(this.exit.promise, killTimeoutMs); }
      if (!this.exited) {
        this.failure ??= 'native cleanup terminal event timed out';
        this.exited = { code: null, signal: null, cleanupTimedOut: true, killSignalAccepted };
        for (const stream of [this.child.stdin, this.child.stdout, this.child.stderr]) stream?.destroy();
        this.child.unref();
      }
      return { ...this.exited, failure: this.failure, stderr: this.stderr, calls: this.calls.reduce((out, op) => ({ ...out, [op]: (out[op] ?? 0) + 1 }), {}) };
    })();
    return this.closePromise;
  }
}

export function validateReplayRequest(request, { allowReplay = false, totalSeconds, audioHoldMs = 0 } = {}) {
  if (!request || typeof request !== 'object' || Array.isArray(request)) throw Error('invalid native request');
  if (!allowReplay || !['create', 'offer', 'remote', 'ice', 'start', 'status'].includes(request.op)) throw Error('replay RPC forbidden in clock-only mode');
  if (request.op === 'status') return { op: 'status' };
  if (request.peer !== 'publish') throw Error('only owned publish peer is allowed');
  if (request.op === 'create') { if (request.publish !== true) throw Error('native publication role differs'); return { op: 'create', peer: 'publish', publish: true }; }
  if (request.op === 'start') {
    if (!Number.isInteger(totalSeconds) || totalSeconds < 21 || totalSeconds > 361 || request.total_seconds !== totalSeconds || request.audio_hold_ms !== audioHoldMs || ![0, 50, 200, 500].includes(audioHoldMs)) throw Error('native V2 duration/hold policy differs');
    return { op: 'start', peer: 'publish', total_seconds: totalSeconds, audio_hold_ms: audioHoldMs };
  }
  if (request.op === 'remote') {
    if (request.description?.type !== 'answer' || typeof request.description.sdp !== 'string' || request.description.sdp.length > 262144) throw Error('invalid owned SDP answer');
    return { op: 'remote', peer: 'publish', description: request.description };
  }
  if (request.op === 'ice') {
    if (!request.candidate || typeof request.candidate !== 'object' || JSON.stringify(request.candidate).length > 4096) throw Error('invalid owned ICE candidate');
    return { op: 'ice', peer: 'publish', candidate: request.candidate };
  }
  return { op: 'offer', peer: 'publish' };
}

export async function startBridge(rpc, { staticDirectory, clockDelayMs = 0, replay = {} }) {
  if (!Number.isFinite(clockDelayMs) || clockDelayMs < 0 || clockDelayMs > 1000) throw Error('invalid test clock delay');
  const token = randomBytes(32).toString('hex'); let clockCalls = 0;
  const staticNames = new Set(['native-pcm-observer.mjs', 'native-pcm-ring.mjs', 'native-pcm-worklet.mjs', 'pcm-kernel.mjs']);
  const server = createServer(async (req, res) => {
    res.setHeader('cross-origin-opener-policy', 'same-origin'); res.setHeader('cross-origin-embedder-policy', 'require-corp'); res.setHeader('cache-control', 'no-store');
    try {
      if (req.url === '/' && req.method === 'GET') { res.setHeader('content-type', 'text/html'); res.end('<!doctype html><title>Local native PCM instrument</title>'); return; }
      const name = req.url?.slice(1);
      if (req.method === 'GET' && staticNames.has(name)) { res.setHeader('content-type', 'text/javascript'); res.end(readFileSync(join(staticDirectory, name))); return; }
      if (req.headers.authorization !== 'Bearer ' + token) { res.writeHead(403).end(); return; }
      let result;
      if (req.method === 'GET' && req.url === '/clock') {
        if (clockDelayMs) await sleep(clockDelayMs);
        result = await rpc.call({ op: 'clock' });
        if (result.clock !== 'CLOCK_MONOTONIC') throw Error('native clock domain differs'); nativeNs(result.monoNs); clockCalls++;
      } else if (req.method === 'POST' && req.url === '/rpc') {
        let body = ''; for await (const chunk of req) { body += chunk; if (body.length > 300000) throw Error('owned RPC body too large'); }
        result = await rpc.call(validateReplayRequest(JSON.parse(body), replay));
      } else { res.writeHead(404).end(); return; }
      res.setHeader('content-type', 'application/json'); res.end(JSON.stringify(result));
    } catch (error) { res.writeHead(500, { 'content-type': 'application/json' }); res.end(JSON.stringify({ error: String(error) })); }
  });
  server.requestTimeout = 35000; server.headersTimeout = 5000;
  await new Promise((resolve, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', resolve); });
  return { token, url: `http://127.0.0.1:${server.address().port}`, clockCalls: () => clockCalls, close: async () => { server.closeAllConnections(); await new Promise(resolve => server.close(resolve)); } };
}
