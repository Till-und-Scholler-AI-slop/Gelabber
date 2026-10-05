#!/usr/bin/env node
// Two-endpoint diagnostic source → SFU → decoder; never a voice matrix pass.
import fs from 'node:fs';
import http from 'node:http';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';
import { NativeVideo, decodedVideo, nativeExitEvidence, janusSessionAbsence } from './native-video.mjs';
import { nativePublisher } from './native-video-adapters.mjs';
import { JanusBroker } from './janus-broker.mjs';
import { proxyTarget } from './proxy-target.mjs';
import { executedChromium } from './browser-provenance.mjs';
const options = {}, allowed = new Set(['engine', 'backend', 'binary', 'archive', 'output', 'seconds', 'warmup', 'bind', 'execute']);
for (let i = 2; i < process.argv.length; i++) {
  const key = process.argv[i]; if (!key.startsWith('--') || !allowed.has(key.slice(2))) throw new Error('unsupported option');
  options[key.slice(2)] = key === '--execute' ? true : process.argv[++i];
}
const seconds = Number(options.seconds ?? 20), warmup = Number(options.warmup ?? 10), engine = options.engine;
if (!['current', 'mediasoup', 'janus'].includes(engine) || !options.backend || !options.binary || !options.archive || !options.output || fs.existsSync(options.output)) throw new Error('required --engine, --backend URL, --binary FILE, --archive FILE, fresh --output FILE');
const backend = new URL(options.backend);
if (!['http:', 'https:'].includes(backend.protocol) || backend.username || backend.password || backend.pathname !== '/' || backend.search || backend.hash) throw new Error('backend must be an explicit HTTP origin');
if (!Number.isInteger(seconds) || seconds < 8 || seconds > 120 || !Number.isInteger(warmup) || warmup < 0 || warmup > 60) throw new Error('measurement8..120s/warmup0..60s required');
if (options.execute && (process.env.BENCH_TOKEN?.length ?? 0) < 32) throw new Error('execution requires BENCH_TOKEN with32bytes');
const inspected = spawnSync(options.binary, ['--inspect', options.archive], { encoding: 'utf8', timeout: 30000 });
if (inspected.status !== 0) throw new Error('native archive inspection failed');
const provenance = JSON.parse(inspected.stdout);
if ((seconds + warmup) % provenance.source.duration_seconds) throw new Error('duration must span whole frozen periods');
const folder = path.dirname(fileURLToPath(import.meta.url)), token = process.env.BENCH_TOKEN;
const sha = file => createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const bundle = fs.readFileSync(path.join(folder, 'native-video.bundle.js'));
const git = args => spawnSync('git', ['-C', folder, ...args], { encoding: 'utf8' });
const head = git(['rev-parse', 'HEAD']), dirty = git(['status', '--porcelain']);
const evidence = { scope: 'one video-only native publisher and one browser decoder through one media engine; zero voice peers',
  acceptance: false, comparison_available: false, production_feature_acceptance: false,
  source_revision: head.status === 0 ? head.stdout.trim() : null, source_dirty: dirty.status === 0 ? !!dirty.stdout.trim() : null,
  run_plan: { engine, backend: backend.origin, execute: !!options.execute, seconds, warmup, native_bind: options.bind ?? '127.0.0.1',
    topology: { source_publishers: 1, receivers: 1, voice_peers: 0, media_dtls_transports: 2, native_audio_tracks: 0, janus_sessions: engine === 'janus' ? 3 : 0 } },
  instrument: provenance, collector: { node: process.version, executable_sha256: sha(process.execPath),
    artifact_sha256: Object.fromEntries(['native-video-pilot.mjs', 'native-video-adapters.mjs', 'native-video-browser.mjs', 'native-video.bundle.js', 'native-video.mjs', 'mediasoup-native-sdp.mjs', 'janus-broker.mjs', 'janus-events.mjs', 'browser-provenance.mjs', 'proxy-target.mjs', 'package-lock.json'].map(name => [name, sha(path.join(folder, name))])) }, failures: [] };
let source, publisher, browser, proxy;
const active = new Set(), browserJanusSessions = new Set();
const broker = engine === 'janus' ? new JanusBroker(async (id, signal) => {
  const target = proxyTarget(`/janus/${id}?rid=${Date.now()}&maxev=10`, backend.origin); target.searchParams.set('apisecret', token);
  const response = await fetch(target, { signal }); if (!response.ok) throw new Error('browser Janus poll failed'); return response.json();
}) : undefined;
try {
  if (options.execute) {
    source = new NativeVideo(options.binary, options.archive, options.bind);
    const executed = (await source.ready).provenance;
    if (executed.archive_sha256 !== provenance.archive_sha256 || executed.binary_sha256 !== provenance.binary_sha256) throw new Error('source artifact changed after inspection');
    evidence.executed_source = executed;
    publisher = await nativePublisher(engine, source, backend.origin, token);
    evidence.receiver_binding = publisher.receiver;
    proxy = http.createServer(async (request, response) => {
      try {
        if (request.url === '/') { response.end('<!doctype html><script src="/native-video.js"></script>'); return; }
        if (request.url === '/native-video.js') { response.setHeader('content-type', 'text/javascript'); response.end(bundle); return; }
        if (request.url === '/janus-events' && broker && request.method === 'GET') { broker.connect(response); return; }
        let body = ''; for await (const part of request) { body += part; if (body.length > 256 * 1024) throw new Error('request body too large'); }
        const target = proxyTarget(request.url, backend.origin), command = body ? JSON.parse(body) : undefined;
        if (request.url.startsWith('/janus')) {
          if (command) body = JSON.stringify({ ...command, apisecret: token }); else target.searchParams.set('apisecret', token);
          if (command?.janus === 'destroy') await broker.remove(Number(target.pathname.split('/')[2]));
        }
        const controller = new AbortController(); active.add(controller); response.on('close', () => controller.abort());
        try {
          const remote = await fetch(target, { method: request.method, headers: { 'content-type': 'application/json', authorization: 'Bearer ' + token }, ...(body ? { body } : {}), signal: AbortSignal.any([controller.signal, AbortSignal.timeout(30000)]) });
          const value = await remote.json(); if (command?.janus === 'create' && value.janus === 'success') { browserJanusSessions.add(value.data.id); broker.add(value.data.id); }
          response.statusCode = remote.status; response.setHeader('content-type', 'application/json'); response.end(JSON.stringify(value));
        } finally { active.delete(controller); }
      } catch { if (!response.destroyed) { response.statusCode = 502; response.end('{"error":"diagnostic proxy failed"}'); } }
    });
    await new Promise(resolve => proxy.listen(0, '127.0.0.1', resolve));
    browser = await chromium.launch({ headless: true, args: ['--enable-automation', '--autoplay-policy=no-user-gesture-required', '--disable-features=WebRtcHideLocalIpsWithMdns'] });
    evidence.executed_browser = await executedChromium(browser);
    const page = await browser.newPage(); page.on('pageerror', error => evidence.failures.push(error.message));
    await page.goto(`http://127.0.0.1:${proxy.address().port}`); await page.waitForFunction(() => window.nativePilotSetup);
    // Janus only exposes an active feed after publisher WebRTC starts. All
    // engines start this same immutable source before decoder setup; reserve
    // one whole clip period for setup, without adding unscheduled keyframes.
    const replaySeconds = seconds + warmup + provenance.source.duration_seconds;
    evidence.run_plan.source_replay_seconds = replaySeconds;
    await source.call({ op: 'start', seconds: replaySeconds }); evidence.started_at = new Date().toISOString();
    await page.evaluate(config => window.nativePilotSetup(config), publisher.receiver);
    await page.waitForTimeout(warmup * 1000);
    evidence.samples = await page.evaluate(duration => window.nativePilotSamples(duration), seconds);
    const completedBy = Date.now() + provenance.source.duration_seconds * 1000;
    do {
      evidence.native = await source.call({ op: 'status' });
      if (evidence.native.source.completed || evidence.native.source.error) break;
      await page.waitForTimeout(100);
    } while (Date.now() < completedBy);
    evidence.decoder = decodedVideo(evidence.samples, provenance.source.rtp_payload_bitrate_bps);
    evidence.failures.push(...evidence.decoder.failures);
    if (!evidence.native.source.completed || !evidence.native.source.source_policy_valid || evidence.native.source.error || evidence.native.connection !== 'connected') evidence.failures.push('source did not finish its fixed schedule over connected WebRTC');
    const diagnostics = publisher.diagnostics?.(); if (diagnostics) evidence.signaling = diagnostics;
    if (diagnostics?.error) evidence.failures.push(diagnostics.error);
    evidence.source_decoder_valid = evidence.failures.length === 0;
  }
} catch (error) {
  evidence.failures.push(String(error)); evidence.source_decoder_valid = false;
  if (error.cleanup_errors) evidence.setup_cleanup = { valid: false, errors: error.cleanup_errors,
    setup_error: error.setup_error, evidence: error.cleanup_evidence };
  try { if (source) evidence.native = await source.call({ op: 'status' }); } catch (statusError) { evidence.failures.push('failed source status: ' + String(statusError)); }
}
finally {
  const cleanupErrors = [...(evidence.setup_cleanup?.errors ?? [])]; evidence.post_leave = { browser: [], sessions: [] };
  const recordCleanup = error => { const text = String(error); cleanupErrors.push(text); evidence.failures.push('cleanup: ' + text); };
  if (publisher?.diagnostics) evidence.signaling = publisher.diagnostics();
  if (browser) {
    for (const page of browser.contexts().flatMap(context => context.pages())) {
      try { evidence.post_leave.browser.push(await page.evaluate(() => window.nativePilotClose?.())); }
      catch (error) { recordCleanup(error); }
    }
    try { await browser.close(); } catch (error) { recordCleanup(error); }
  }
  try { if (publisher) evidence.post_leave.publisher = await publisher.close(); }
  catch (error) { evidence.post_leave.publisher = error.cleanup_evidence; recordCleanup(error); }
  try {
    if (source) {
      await source.close(); evidence.post_leave.native_exit = nativeExitEvidence(source.child);
      if (!evidence.post_leave.native_exit.clean) recordCleanup(new Error('native child did not exit cleanly: ' + JSON.stringify(evidence.post_leave.native_exit)));
    }
  } catch (error) { recordCleanup(error); }
  try { await broker?.close(); } catch (error) { recordCleanup(error); }
  for (const id of browserJanusSessions) {
    try {
      const target = new URL(`${backend.origin}/janus/${id}`); target.searchParams.set('apisecret', token);
      const response = await fetch(target, { signal: AbortSignal.timeout(3000) });
      const absence = await janusSessionAbsence(response); evidence.post_leave.sessions.push({ id, ...absence });
      if (!absence.absent) {
        recordCleanup(new Error('owned browser Janus session remains after leave'));
        await fetch(target, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ janus: 'destroy', transaction: crypto.randomUUID(), apisecret: token }), signal: AbortSignal.timeout(3000) });
      }
    } catch (error) { recordCleanup(error); }
  }
  active.forEach(controller => controller.abort());
  if (proxy) { proxy.closeAllConnections(); await new Promise(resolve => proxy.close(resolve)); }
  if (broker) evidence.browser_janus_broker = broker.evidence();
  evidence.cleanup_errors = cleanupErrors; evidence.cleanup_valid = cleanupErrors.length === 0;
  evidence.instrument_diagnostic_valid = !!evidence.source_decoder_valid && evidence.cleanup_valid;
  if (evidence.failures.length) evidence.instrument_diagnostic_valid = false;
  evidence.finished_at = new Date().toISOString();
  fs.mkdirSync(path.dirname(path.resolve(options.output)), { recursive: true }); fs.writeFileSync(options.output, JSON.stringify(evidence, null, 2) + '\n', { flag: 'wx' });
}
console.log(`Evidence: ${options.output}`); if (evidence.failures.length) process.exitCode = 1;
