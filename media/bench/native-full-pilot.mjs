#!/usr/bin/env node
// Full original N-participant fixture: native0, browser1..N-1, N-1 Watchers.
// This pilot exposes actual graph/streams and cleanup, never a migration pass.
import fs from 'node:fs';
import http from 'node:http';
import path from 'node:path';
import { parseArgs } from 'node:util';
import { networkInterfaces } from 'node:os';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';
import { NativePeer } from './native-peer.mjs';
import { nativePeerAdapter } from './native-peer-adapters.mjs';
import { nativeExitEvidence, janusSessionAbsence } from './native-video.mjs';
import { nativeFullTopology, nativeFullGraph } from './native-full-checks.mjs';
import { JanusBroker } from './janus-broker.mjs';
import { proxyTarget } from './proxy-target.mjs';
import { executedChromium } from './browser-provenance.mjs';
const { values } = parseArgs({ options: {
  engine: { type: 'string' }, backend: { type: 'string' }, binary: { type: 'string' }, video: { type: 'string' }, mic: { type: 'string' }, source: { type: 'string' },
  output: { type: 'string' }, phase: { type: 'string' }, provenance: { type: 'string' }, chromium: { type: 'string' }, 'bind-interface': { type: 'string' },
  peers: { type: 'string', default: '2' }, seconds: { type: 'string', default: '20' }, warmup: { type: 'string', default: '10' }, 'source-seconds': { type: 'string', default: '120' }, execute: { type: 'boolean', default: false }
} });
const engine = values.engine, peers = Number(values.peers), seconds = Number(values.seconds), warmup = Number(values.warmup), sourceSeconds = Number(values['source-seconds']);
if (!['current', 'mediasoup', 'janus'].includes(engine)) throw new Error('explicit engine required');
for (const field of ['backend', 'binary', 'video', 'mic', 'source', 'output', 'phase', 'chromium']) if (!values[field]) throw new Error('--' + field + ' required');
const backend = new URL(values.backend);
if (!['http:', 'https:'].includes(backend.protocol) || backend.username || backend.password || backend.pathname !== '/' || backend.search || backend.hash) throw new Error('explicit bare backend origin required');
const topology = nativeFullTopology(peers);
if (!Number.isInteger(seconds) || seconds < 10 || seconds > 120 || !Number.isInteger(warmup) || warmup < 5 || warmup > 60 || !Number.isInteger(sourceSeconds) || sourceSeconds % 10 || sourceSeconds < seconds + warmup + 30 || sourceSeconds > 360) throw new Error('seconds10..120/warmup5..60 and whole120..360s source with setup reserve required');
if (fs.existsSync(values.output) || fs.existsSync(values.phase)) throw new Error('fresh output/phase files required');
const token = process.env.BENCH_TOKEN;
if (values.execute && (token?.length ?? 0) < 32) throw new Error('private BENCH_TOKEN required');
const sha = file => createHash('sha256').update(fs.readFileSync(file)).digest('hex');
if (process.version !== 'v26.8.2' || sha(process.execPath) !== '8a22a371fd85aecf5411636574309f6380fbc42694aaf0651a089a8ef9c44e52' || sha(values.chromium) !== 'ded93a9c9a53a1ae040f08124badcca95c938e9d5015ff340c3b5538c41bf39e') throw new Error('actual pinned Node26.8.2/Chrome153 required');
const interfaces = networkInterfaces(), iface = values['bind-interface'];
const addresses = iface ? interfaces[iface]?.filter(value => value.family === 'IPv4' && !value.internal) : [];
if (!iface || addresses?.length !== 1) throw new Error('one explicit observed nonloopback IPv4 interface required');
const bind = addresses[0].address;
const folder = path.dirname(fileURLToPath(import.meta.url)), bundle = fs.readFileSync(path.join(folder, 'native-full.bundle.js'));
const inspected = spawnSync(values.binary, ['--inspect', values.video, values.mic, values.source], { encoding: 'utf8', timeout: 30000 });
if (inspected.status !== 0) throw new Error('strict native input inspection failed: ' + inspected.stderr?.slice(-1024));
const instrument = JSON.parse(inspected.stdout), provenance = values.provenance ? JSON.parse(fs.readFileSync(values.provenance, 'utf8')) : null;
const helpers = ['native-full-pilot.mjs', 'native-full-browser.mjs', 'native-full.bundle.js', 'native-full-checks.mjs', 'native-peer-adapters.mjs', 'native-peer-current.mjs', 'mediasoup-native-peer-sdp.mjs', 'native-peer-checks.mjs', 'native-peer.mjs', 'native-video.mjs', 'janus-events.mjs', 'janus-broker.mjs', 'browser-provenance.mjs', 'proxy-target.mjs', 'package-lock.json'];
const report = { schema: 1, scope: 'bounded full N-participant native0 SFU instrument pilot; no performance/migration/product/PCM acceptance', engine, topology,
  comparison_available: false, production_feature_acceptance: false, native_pcm_qualified: false,
  plan: { execute: values.execute, peers, seconds, warmup, source_seconds: sourceSeconds, source_policy: 'shared immutable preencoded V1 Opus/VP8 archive periods; no browser video BWE control', bind, backend: backend.origin },
  source_provenance: provenance, instrument, collector: { node: process.version, node_sha256: sha(process.execPath), helpers: Object.fromEntries(helpers.map(name => [name, sha(path.join(folder, name))])) },
  failures: [], samples: [], phase_events: [], cleanup_errors: [] };
const phase = name => {
  const value = { phase: name, at: new Date().toISOString(), monotonic_ms: performance.now() }; report.phase_events.push(value);
  fs.appendFileSync(values.phase, JSON.stringify(value) + '\n', { mode: 0o600 });
};
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
let native, adapter, browser, page, proxy;
const active = new Set(), browserSessions = new Set();
const broker = engine === 'janus' ? new JanusBroker(async (id, signal) => {
  const target = proxyTarget(`/janus/${id}?rid=${Date.now()}&maxev=10`, backend.origin); target.searchParams.set('apisecret', token);
  const response = await fetch(target, { signal }); if (!response.ok) throw new Error('browser Janus poll failed'); return response.json();
}) : undefined;
try {
  if (values.execute) {
    phase('setup'); report.native_join = { setup_started_at: Date.now(), poll_precision_ms: 50 };
    native = new NativePeer(values.binary, values.video, values.mic, values.source, bind);
    const executed = (await native.ready).provenance;
    if (['binary_sha256'].some(key => executed[key] !== instrument[key]) || ['video', 'mic', 'source'].some(key => executed[key].archive_sha256 !== instrument[key].archive_sha256)) throw new Error('native input changed after inspection');
    adapter = await nativePeerAdapter(engine, native, backend.origin, token);
    report.publication = adapter.publication;
    report.timeline = (await native.call({ op: 'start', seconds: sourceSeconds })).timeline;
    report.native_join.dtls_ready_at = Date.now();
    proxy = http.createServer(async (request, response) => {
      try {
        if (request.url === '/') { response.end('<!doctype html><title>Private full-native fixture</title><script src="/native-full.js"></script>'); return; }
        if (request.url === '/native-full.js') { response.setHeader('content-type', 'text/javascript'); response.end(bundle); return; }
        if (request.url === '/janus-events' && broker && request.method === 'GET') { broker.connect(response); return; }
        let body = ''; for await (const part of request) { body += part; if (body.length > 256 * 1024) throw new Error('oversized fixture body'); }
        const target = proxyTarget(request.url, backend.origin), command = body ? JSON.parse(body) : undefined;
        if (request.url.startsWith('/janus')) {
          if (command) body = JSON.stringify({ ...command, apisecret: token }); else target.searchParams.set('apisecret', token);
          if (command?.janus === 'destroy') await broker.remove(Number(target.pathname.split('/')[2]));
        }
        const controller = new AbortController(); active.add(controller); response.on('close', () => controller.abort());
        try {
          const remote = await fetch(target, { method: request.method, headers: { 'content-type': 'application/json', authorization: 'Bearer ' + token }, ...(body ? { body } : {}), signal: AbortSignal.any([controller.signal, AbortSignal.timeout(30000)]) });
          const value = await remote.json(); if (command?.janus === 'create' && value.janus === 'success') { browserSessions.add(value.data.id); broker.add(value.data.id); }
          response.statusCode = remote.status; response.setHeader('content-type', 'application/json'); response.end(JSON.stringify(value));
        } finally { active.delete(controller); }
      } catch (error) { report.failures.push('proxy: ' + String(error)); if (!response.destroyed) { response.statusCode = 502; response.end('{"error":"fixture proxy failed"}'); } }
    });
    await new Promise(resolve => proxy.listen(0, '127.0.0.1', resolve));
    browser = await chromium.launch({ headless: true, executablePath: values.chromium, args: ['--enable-automation', '--autoplay-policy=no-user-gesture-required', '--no-sandbox', '--disable-features=WebRtcHideLocalIpsWithMdns'] });
    report.executed_browser = await executedChromium(browser);
    page = await browser.newPage(); page.on('pageerror', error => report.failures.push('pageerror: ' + String(error)));
    await page.goto(`http://127.0.0.1:${proxy.address().port}`); await page.waitForFunction(() => window.nativeFullPrepare);
    const members = await page.evaluate(config => window.nativeFullPrepare(config), { engine, peers, backend: backend.origin, native: adapter.publication });
    report.browser_publications = members;
    await page.evaluate(publication => window.nativeFullSubscribe(publication), adapter.publication);
    report.native_bindings = await adapter.receive(members);
    report.ready_graph = await page.evaluate(() => window.nativeFullReady());
    report.native_join.full_graph_rtp_ready_at = Date.now();
    const ready = await native.call({ op: 'status' });
    if (ready.peers.publish.outbound.some(row => row.packetsSent > 0)) report.native_join.first_send_rtp_at = Date.now();
    phase('warmup'); await sleep(warmup * 1000);
    phase('measurement'); const start = performance.now();
    for (let index = 0; index <= seconds; index++) {
      await sleep(Math.max(0, start + index * 1000 - performance.now())); adapter.check();
      report.samples.push({ at_monotonic_ms: performance.now(), native: await native.call({ op: 'status' }), browser: await page.evaluate(() => window.nativeFullCollect()) });
      if (report.failures.length) throw new Error(report.failures.join('; '));
    }
    report.graph = nativeFullGraph(report.samples, topology, report.native_bindings, adapter.receiver_peer, instrument.video.metadata.rtp_payload_bitrate_bps, { engine, requestedSeconds: seconds });
    if (!report.graph.valid) report.failures.push(...report.graph.failures);
    report.source_window_valid = ['mic', 'source', 'video'].every(name => report.samples.every(sample => sample.native.sources[name]?.source_policy_valid === true && !sample.native.sources[name].error));
    report.full_graph_streams_valid = report.graph.valid && report.source_window_valid && report.failures.length === 0;
  }
} catch (error) {
  report.failures.push(String(error));
  if (error.cleanup_errors) { report.cleanup_errors.push(...error.cleanup_errors); report.setup_cleanup = { setup_error: error.setup_error, evidence: error.cleanup_evidence }; }
  try { if (native) report.failed_native_status = await native.call({ op: 'status' }); } catch (statusError) { report.failures.push('native failure status: ' + String(statusError)); }
  try { if (page) report.failed_browser_status = await page.evaluate(() => window.nativeFullCollect()); } catch (statusError) { report.failures.push('browser failure status: ' + String(statusError)); }
} finally {
  const failure = error => { report.cleanup_errors.push(String(error)); report.failures.push('cleanup: ' + String(error)); };
  if (values.execute) phase('cleanup'); report.post_leave = {};
  if (adapter?.diagnostics) report.signaling = adapter.diagnostics();
  if (page) { try { report.post_leave.browser = await page.evaluate(() => window.nativeFullClose()); } catch (error) { failure(error); } }
  if (browser) { try { await browser.close(); } catch (error) { failure(error); } }
  if (adapter) { try { report.post_leave.native_adapter = await adapter.close(); } catch (error) { report.post_leave.native_adapter = error.cleanup_evidence; failure(error); } }
  if (native) { try { await native.close(); report.native_exit = nativeExitEvidence(native.child); if (!report.native_exit.clean) throw new Error('native exit was not normal: ' + JSON.stringify(report.native_exit)); } catch (error) { failure(error); } }
  try { await broker?.close(); } catch (error) { failure(error); }
  for (const id of browserSessions) {
    try { const target = new URL(`${backend.origin}/janus/${id}`); target.searchParams.set('apisecret', token);
      const absence = await janusSessionAbsence(await fetch(target, { signal: AbortSignal.timeout(3000) }));
      (report.post_leave.browser_sessions ??= []).push({ id, ...absence });
      if (!absence.absent) { failure(new Error('owned Janus browser session remains')); await fetch(target, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ janus: 'destroy', transaction: crypto.randomUUID(), apisecret: token }), signal: AbortSignal.timeout(3000) }); }
    } catch (error) { failure(error); }
  }
  active.forEach(controller => controller.abort());
  if (proxy) { proxy.closeAllConnections(); await new Promise(resolve => proxy.close(resolve)); }
  if (broker) report.browser_janus_broker = broker.evidence();
  if (adapter && engine !== 'janus') {
    try { const response = await fetch(backend.origin + '/rpc', { method: 'POST', headers: { 'content-type': 'application/json', authorization: 'Bearer ' + token }, body: '{"op":"summary"}', signal: AbortSignal.timeout(3000) });
      const stats = await response.json(); report.post_leave.engine_stats = stats;
      if (!response.ok || (engine === 'current' ? stats.peers !== 0 || stats.rooms !== 0 : ['peers', 'transports', 'producers', 'consumers'].some(name => stats[name] !== 0))) throw new Error('owned whole-engine resources remain after every peer left');
    } catch (error) { failure(error); }
  }
  report.cleanup_valid = report.cleanup_errors.length === 0;
  report.instrument_pilot_valid = !!report.full_graph_streams_valid && report.cleanup_valid && report.failures.length === 0;
  report.finished_at = new Date().toISOString(); if (values.execute) phase('finished');
  fs.mkdirSync(path.dirname(path.resolve(values.output)), { recursive: true }); fs.writeFileSync(values.output, JSON.stringify(report, null, 2) + '\n', { flag: 'wx', mode: 0o600 });
}
console.log(JSON.stringify({ output: values.output, execute: values.execute, instrument_pilot_valid: report.instrument_pilot_valid, failures: report.failures, cleanup_errors: report.cleanup_errors }));
if (report.failures.length) process.exitCode = 1;
