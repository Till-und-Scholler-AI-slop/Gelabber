#!/usr/bin/env node
// Run on a separate load-generator host for acceptance. Local runs are probes.
import { chromium, firefox } from 'playwright';
import http from 'node:http';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { proxyTarget } from './proxy-target.mjs';
import { executedChromium } from './browser-provenance.mjs';
import { readPcmCalibration, samePcmBrowser } from './pcm-policy.mjs';
const options = {};
for (let index = 2; index < process.argv.length; index += 2) options[process.argv[index].replace(/^--/, '')] = process.argv[index + 1];
const engine = options.engine, backend = options.backend, peers = Number(options.peers ?? 2);
const videoBitrate = Number(options['video-bitrate'] ?? 6000000);
const protocolLogs = options['protocol-logs'] === 'true';
const fixedVideoFixture = options['fixed-video-fixture'] === 'true';
const pcmLatency = options['pcm-latency'] === 'true';
if (!['current', 'mediasoup', 'janus'].includes(engine) || !backend || !options.output || !process.env.BENCH_TOKEN) throw new Error('Required: --engine current|mediasoup|janus --backend URL --output FILE; BENCH_TOKEN environment');
if (![2, 8, 16, 32].includes(peers)) throw new Error('Supported participant matrix: 2/8/16/32');
if (!Number.isSafeInteger(videoBitrate) || videoBitrate <= 0 || videoBitrate > 100000000) throw new Error('Invalid fixture video bitrate');
if (protocolLogs && options.browser === 'firefox') throw new Error('Chromium RTC event logging required for --protocol-logs');
if (fixedVideoFixture && (options.browser === 'firefox' || options.video !== 'true' || videoBitrate % 1000)) throw new Error('Fixed video fixture requires Chromium video and whole kbit/s');
if (pcmLatency && (options.browser === 'firefox' || Number(options.duration ?? 60000) < 8000)) throw new Error('PCM latency requires Chromium and at least eight seconds');
const folder = path.dirname(fileURLToPath(import.meta.url));
const pcmCalibration = pcmLatency ? readPcmCalibration(options['pcm-calibration'], folder) : undefined;
const browserArgs = ['--enable-automation', '--autoplay-policy=no-user-gesture-required', '--disable-background-timer-throttling', '--disable-renderer-backgrounding', '--disable-features=WebRtcHideLocalIpsWithMdns'];
if (protocolLogs) {
  const directory = path.join(path.dirname(path.resolve(options.output)), 'rtc-events');
  fs.mkdirSync(directory, { recursive: true, mode: 0o700 });
  browserArgs.push('--webrtc-event-logging=' + directory);
}
const proxy = http.createServer(async (request, response) => {
  try {
    if (request.url === '/') { response.setHeader('content-type', 'text/html'); response.end('<!doctype html><title>Gelabber media benchmark</title><script src="/client.js"></script>'); return; }
    if (request.url === '/client.js') { response.setHeader('content-type', 'text/javascript'); response.end(fs.readFileSync(path.join(folder, 'client.bundle.js'))); return; }
    if (request.url === '/pcm.js' && pcmLatency) { response.setHeader('content-type', 'text/javascript'); response.end(fs.readFileSync(path.join(folder, 'pcm.bundle.js'))); return; }
    let body = ''; for await (const part of request) { body += part; if (body.length > 256 * 1024) throw new Error('body limit'); }
    const target = proxyTarget(request.url, backend);
    const headers = { 'content-type': 'application/json', authorization: `Bearer ${process.env.BENCH_TOKEN}` };
    if (request.url.startsWith('/janus')) {
      if (body) body = JSON.stringify({ ...JSON.parse(body), apisecret: process.env.BENCH_TOKEN });
      else target.searchParams.set('apisecret', process.env.BENCH_TOKEN);
    }
    const remote = await fetch(target, { method: request.method, headers, ...(body ? { body } : {}), signal: AbortSignal.timeout(65000) });
    response.statusCode = remote.status; response.setHeader('content-type', 'application/json'); response.end(await remote.text());
  } catch (error) { response.statusCode = 502; response.end(JSON.stringify({ error: String(error) })); }
});
await new Promise(resolve => proxy.listen(0, '127.0.0.1', resolve));
let browser, page, executedBrowser;
const pageErrors = [];
const metadata = () => ({ hostname: os.hostname(), platform: os.platform(), cpu: os.cpus()[0]?.model, logical_cpus: os.cpus().length, browser: options.browser === 'firefox' ? 'firefox' : 'chromium', browser_channel: options.browser === 'firefox' ? 'firefox' : protocolLogs ? 'chromium' : 'headless-shell', protocol_logs: protocolLogs, browser_version: browser?.version(), executed_browser: executedBrowser, browser_args: options.browser === 'firefox' ? [] : browserArgs, remote_claim: options['separate-host'] === 'true', recorded_at: new Date().toISOString() });
const save = result => { fs.mkdirSync(path.dirname(path.resolve(options.output)), { recursive: true }); fs.writeFileSync(options.output, JSON.stringify({ ...result, ...(pcmCalibration ? { pcm_calibration: pcmCalibration } : {}), load_generator: metadata() }, null, 2) + '\n'); };
try {
  const browserType = options.browser === 'firefox' ? firefox : chromium;
  browser = await browserType.launch({ headless: true, ...(browserType === chromium ? { args: browserArgs, ...(protocolLogs ? { channel: 'chromium' } : {}) } : {}) });
  if (browserType === chromium) executedBrowser = await executedChromium(browser);
  if (pcmLatency && !samePcmBrowser(pcmCalibration, executedBrowser)) throw new Error('PCM calibration executed browser differs from this run');
  page = await browser.newPage();
  page.on('console', message => console.error('Browser:', message.text()));
  page.on('pageerror', error => { pageErrors.push(error.message); console.error('Browser error:', error.message); });
  await page.goto(`http://127.0.0.1:${proxy.address().port}`);
  await page.waitForFunction(() => typeof window.startBenchmark === 'function');
  const result = await page.evaluate(config => window.startBenchmark(config), { engine, backend, peers, video: options.video === 'true', videoBitrate, fixedVideoFixture, pcmLatency, warmupMs: Number(options.warmup ?? 10000), durationMs: Number(options.duration ?? 60000) });
  result.failures.push(...pageErrors);
  save(result);
  if (result.failures.length) process.exitCode = 1;
  console.log(`Evidence: ${options.output}`);
} catch (error) {
  let observed = []; try { observed = await page?.evaluate(() => window.benchmarkFailures ?? []); } catch {}
  save({ backend: engine, peers, video: options.video === 'true', failures: [...observed, ...pageErrors, String(error)], samples: [] });
  console.error(error); process.exitCode = 1;
} finally { if (browser) await browser.close(); proxy.closeAllConnections(); await new Promise(resolve => proxy.close(resolve)); }
