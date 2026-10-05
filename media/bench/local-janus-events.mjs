#!/usr/bin/env node
// A loopback signaling control, not a Janus/SFU resource benchmark.
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { chromium } from 'playwright';
import { JanusBroker } from './janus-broker.mjs';
import { executedChromium } from './browser-provenance.mjs';
const output = process.argv[2];
if (!output) throw new Error('Usage: node local-janus-events.mjs OUTPUT.json');
const folder = path.dirname(fileURLToPath(import.meta.url));
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
async function until(predicate) {
  for (let attempt = 0; attempt < 1000; attempt++) { if (predicate()) return; await sleep(5); }
  throw new Error('Local control deadline');
}
let id = 0, baselineActive = 0, baselineMaximum = 0, backendMaximum = 0;
const polls = new Map(), timers = new Set(), controllers = new Set();
const fake = http.createServer(async (request, response) => {
  if (request.method === 'GET') {
    const session = Number(request.url.split('/')[2].split('?')[0]); polls.set(session, response);
    backendMaximum = Math.max(backendMaximum, polls.size);
    response.on('close', () => { if (polls.get(session) === response) polls.delete(session); }); return;
  }
  let text = ''; for await (const chunk of request) text += chunk;
  const body = JSON.parse(text);
  response.setHeader('content-type', 'application/json');
  if (body.janus === 'create') { response.end(JSON.stringify({ janus: 'success', data: { id: ++id } })); return; }
  const session = Number(request.url.split('/')[2]);
  if (!body.body?.missing) {
    await until(() => polls.has(session));
    polls.get(session).end(JSON.stringify({ transaction: body.transaction, plugindata: { data: { peer: body.body.peer } } }));
  }
  if (body.body?.stalled) return; // Event has arrived, but its HTTP ACK never does.
  await sleep(10); response.end(JSON.stringify({ janus: 'ack' }));
});
await new Promise(resolve => fake.listen(0, '127.0.0.1', resolve));
const backend = `http://127.0.0.1:${fake.address().port}`;
const broker = new JanusBroker(async (session, signal) => {
  const response = await fetch(`${backend}/janus/${session}?maxev=10`, { signal }); return response.json();
});
const proxy = http.createServer(async (request, response) => {
  try {
    if (request.url === '/') { response.end('<!doctype html><script type="module">import {JanusSignal} from "/signal.js";window.JanusSignal=JanusSignal;</script>'); return; }
    if (request.url === '/signal.js') { response.setHeader('content-type', 'text/javascript'); response.end(fs.readFileSync(path.join(folder, 'janus-events.mjs'))); return; }
    if (request.url === '/janus-events') { broker.connect(response); return; }
    if (request.url === '/baseline-post') { response.end('{}'); return; }
    if (request.url.startsWith('/baseline/')) {
      baselineActive++; baselineMaximum = Math.max(baselineMaximum, baselineActive);
      const timer = setTimeout(() => { timers.delete(timer); baselineActive--; response.end('{}'); }, 600); timers.add(timer); return;
    }
    let text = ''; for await (const chunk of request) text += chunk;
    const controller = new AbortController(); controllers.add(controller);
    response.on('close', () => controller.abort());
    try {
      const remote = await fetch(backend + request.url, { method: 'POST', body: text, signal: AbortSignal.any([controller.signal, AbortSignal.timeout(1000)]) });
      const value = await remote.json();
      if (JSON.parse(text).janus === 'create') broker.add(value.data.id);
      response.setHeader('content-type', 'application/json'); response.end(JSON.stringify(value));
    } finally { controllers.delete(controller); }
  } catch { if (!response.destroyed) { response.statusCode = 502; response.end('{}'); } }
});
await new Promise(resolve => proxy.listen(0, '127.0.0.1', resolve));
const evidence = { scope: 'loopback HTTP1 signaling control, fake backend; no ICE/RTP/SFU/WAN', acceptance: false, resource_comparison_available: false,
  source_revision: execFileSync('git', ['rev-parse', 'HEAD'], { cwd: folder, encoding: 'utf8' }).trim(),
  git_dirty: !!execFileSync('git', ['status', '--porcelain'], { cwd: folder, encoding: 'utf8' }).trim(),
  artifact_sha256: Object.fromEntries(['local-janus-events.mjs', 'janus-events.mjs', 'janus-broker.mjs', 'browser-provenance.mjs', 'package-lock.json']
    .map(name => [name, createHash('sha256').update(fs.readFileSync(path.join(folder, name))).digest('hex')])) };
let browser;
try {
  browser = await chromium.launch({ headless: true, args: ['--enable-automation'] }); evidence.executed_browser = await executedChromium(browser);
  const page = await browser.newPage(); await page.goto(`http://127.0.0.1:${proxy.address().port}`);
  await page.waitForFunction(() => window.JanusSignal);
  evidence.original = await page.evaluate(async () => {
    const polls = Array.from({ length: 9 }, (_, index) => fetch('/baseline/' + index));
    await new Promise(resolve => setTimeout(resolve, 50)); const start = performance.now();
    await fetch('/baseline-post', { method: 'POST' }); const postDelayMs = performance.now() - start;
    await Promise.all(polls); return { post_delay_ms: postDelayMs };
  }); evidence.original.maximum_active_browser_longpolls = baselineMaximum;
  // Nine actual backend polls: the room manager plus eight participant sessions.
  await page.evaluate(async () => { window.signal = new window.JanusSignal({ deadlineMs: 250 }); await window.signal.ready; window.sessions = []; for (let i = 0; i < 9; i++) window.sessions.push(await window.signal.session()); });
  await until(() => polls.size === 9);
  evidence.fixed = await page.evaluate(async () => {
    const start = performance.now();
    const result = await Promise.all(window.sessions.slice(1).map((session, peer) => session.request(99, { janus: 'message', body: { request: 'join', peer } })));
    return { completed_peer_requests: result.map(r => r.plugindata.data.peer), elapsed_ms: performance.now() - start };
  });
  evidence.negative = await page.evaluate(async () => {
    const rejected = [];
    for (const mode of ['missing', 'stalled']) {
      try { await window.sessions[1].request(99, { janus: 'message', body: { request: 'join', peer: 1, [mode]: true } }); }
      catch (error) { rejected.push({ mode, error: error.message }); }
    }
    const remainingPending = window.sessions.reduce((n, session) => n + session.pending.size, 0);
    window.signal.close(); return { rejected, remaining_pending: remainingPending, remaining_requests: window.signal.controllers.size };
  });
  await broker.close(); await until(() => polls.size === 0 && controllers.size === 0);
  evidence.fixed.maximum_active_backend_longpolls = backendMaximum; evidence.broker = broker.evidence();
  evidence.valid = baselineMaximum === 6 && evidence.original.post_delay_ms >= 400 && backendMaximum === 9
    && evidence.fixed.completed_peer_requests.join(',') === '0,1,2,3,4,5,6,7'
    && evidence.fixed.elapsed_ms < 250 && evidence.negative.rejected.length === 2
    && evidence.negative.rejected.every(r => /deadline/.test(r.error)) && evidence.negative.remaining_pending === 0
    && evidence.negative.remaining_requests === 0 && polls.size === 0 && controllers.size === 0;
} catch (error) { evidence.error = String(error); evidence.valid = false; }
finally {
  await broker.close(); if (browser) await browser.close(); controllers.forEach(controller => controller.abort());
  timers.forEach(timer => clearTimeout(timer));
  for (const server of [proxy, fake]) { server.closeAllConnections(); await new Promise(resolve => server.close(resolve)); }
  fs.writeFileSync(output, JSON.stringify(evidence, null, 2) + '\n');
}
console.log(JSON.stringify({ valid: evidence.valid, original: evidence.original, fixed: evidence.fixed, negative: evidence.negative, error: evidence.error }));
if (!evidence.valid) process.exitCode = 1;
