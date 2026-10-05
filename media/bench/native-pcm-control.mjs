#!/usr/bin/env node
import { mkdir, copyFile, writeFile } from 'node:fs/promises';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';
import { chromium } from 'playwright';
import { NativeRpc, startBridge, sha256 } from './native-pcm-bridge.mjs';
import { NATIVE_PCM_POLICY as POLICY, validateBrowserClock } from './native-pcm-policy.mjs';
import { boundNativeCallback, validateClockSamples } from './native-pcm-clock-bounds.mjs';

const { values } = parseArgs({ options: {
  execute: { type: 'boolean', default: false }, binary: { type: 'string' }, 'binary-sha256': { type: 'string' },
  video: { type: 'string' }, mic: { type: 'string' }, source: { type: 'string' }, library: { type: 'string' },
  chromium: { type: 'string' }, output: { type: 'string' }, seconds: { type: 'string', default: '10' },
  'observer-pause-ms': { type: 'string', default: '.25' }, 'clock-delay-ms': { type: 'string', default: '0' },
  capacity: { type: 'string', default: '4096' }, 'suspend-ms': { type: 'string', default: '0' },
} });
for (const field of ['binary', 'binary-sha256', 'video', 'mic', 'source', 'library', 'chromium', 'output']) if (!values[field]) throw Error('--' + field + ' required');
const seconds = Number(values.seconds), pauseMs = Number(values['observer-pause-ms']), capacity = Number(values.capacity), clockDelayMs = Number(values['clock-delay-ms']), suspendMs = Number(values['suspend-ms']);
if (!Number.isInteger(seconds) || seconds < 5 || seconds > 20 || !Number.isFinite(pauseMs) || pauseMs < 0 || pauseMs > 1000 || !Number.isInteger(capacity) || capacity < 2 || capacity > 32768 || !Number.isFinite(suspendMs) || suspendMs < 0 || suspendMs > 1000) throw Error('invalid bounded clock-only control policy');
const directory = dirname(fileURLToPath(import.meta.url));
const files = ['native-pcm-ring.mjs', 'native-pcm-clock-bounds.mjs', 'native-pcm-observer.mjs', 'native-pcm-worklet.mjs', 'native-pcm-policy.mjs', 'native-pcm-evidence.mjs', 'native-pcm-bridge.mjs', 'native-pcm-control.mjs', 'pcm-kernel.mjs'];
const hashes = () => Object.fromEntries(files.map(name => [name, sha256(join(directory, name))]));
const provenance = { nodeVersion: process.version, nodeSha256: sha256(process.execPath), chromiumSha256: sha256(values.chromium), nativeSha256: sha256(values.binary), librarySha256: sha256(values.library), archives: Object.fromEntries(['video', 'mic', 'source'].map(name => [name, sha256(values[name])])), helpers: hashes(), policy: POLICY };
if (process.version !== POLICY.nodeVersion || provenance.nodeSha256 !== POLICY.nodeSha256 || provenance.chromiumSha256 !== POLICY.chromiumSha256 || provenance.nativeSha256 !== values['binary-sha256']) throw Error('actual instrument binary freeze differs');
if (!values.execute) { console.log(JSON.stringify({ execute: false, mode: 'clock-only', seconds, provenance, scope: 'zero peer/SDP/ICE/RTP; no PCM/SFU/production qualification' }, null, 2)); process.exit(0); }
await mkdir(values.output); await mkdir(join(values.output, 'inputs'));
for (const name of files) await copyFile(join(directory, name), join(values.output, 'inputs', name));
const result = { schema: 1, mode: 'clock-only', scope: 'physical AudioWorklet callback bounded by serialized native CLOCK_MONOTONIC RPC; zero peer/SDP/ICE/RTP', comparison_available: false, pcm_latency_calibrated: false, callback_clock_bracketing_qualified: false, provenance, config: { seconds, pauseMs, capacity, clockDelayMs, suspendMs }, failures: [], cleanup: {} };
let rpc, bridge, browser;
try {
  rpc = await NativeRpc.start({ binary: values.binary, binarySha256: values['binary-sha256'], video: values.video, mic: values.mic, source: values.source, libraryDirectory: dirname(values.library) });
  result.nativeProvenance = rpc.provenance;
  if (rpc.provenance.decoder?.sha256 !== provenance.librarySha256 || rpc.provenance.decoder.version !== 'libopus 1.6.1' || sha256(rpc.provenance.decoder.path) !== provenance.librarySha256) throw Error('actual mapped native Opus decoder differs');
  for (const name of ['video', 'mic', 'source']) if (rpc.provenance[name]?.archive_sha256 !== provenance.archives[name]) throw Error('actual native imported archive differs');
  result.initialStatus = await rpc.call({ op: 'status' });
  if (Object.keys(result.initialStatus.peers ?? {}).length || Object.keys(result.initialStatus.sources ?? {}).length) throw Error('clock-only native unexpectedly owns peers/sources');
  bridge = await startBridge(rpc, { staticDirectory: join(values.output, 'inputs'), clockDelayMs });
  browser = await chromium.launch({ headless: true, executablePath: values.chromium, args: ['--autoplay-policy=no-user-gesture-required', '--no-sandbox'] });
  result.browser = await (await browser.newBrowserCDPSession()).send('Browser.getVersion');
  const page = await browser.newPage(); page.on('pageerror', error => result.failures.push(String(error)));
  await page.goto(bridge.url);
  result.raw = await page.evaluate(async ({ token, seconds, pauseMs, capacity, suspendMs }) => {
    const { dimensions, RING } = await import('/native-pcm-ring.mjs');
    const sab = new SharedArrayBuffer(dimensions(2, capacity).cells * 8), cells = new BigInt64Array(sab);
    const worker = new Worker('/native-pcm-observer.mjs', { type: 'module' });
    let readyResolve, completeResolve, readyReject, completeReject;
    const ready = new Promise((resolve, reject) => { readyResolve = resolve; readyReject = reject; });
    const complete = new Promise((resolve, reject) => { completeResolve = resolve; completeReject = reject; });
    // Both promises are handled even if an early worker error precedes ready.
    complete.catch(() => {});
    worker.onmessage = ({ data }) => { if (data.type === 'ready') readyResolve(data); if (data.type === 'complete') completeResolve(data); };
    worker.onerror = event => { readyReject(Error(event.message)); completeReject(Error(event.message)); };
    const context = new AudioContext({ sampleRate: 48000 }), taps = [], states = [];
    let handler;
    const timeout = async (promise, ms) => {
      let timer; try { return await Promise.race([promise, new Promise((_, reject) => { timer = setTimeout(() => reject(Error('owned browser instrument deadline')), ms); })]); }
      finally { clearTimeout(timer); }
    };
    try {
      worker.postMessage({ sab, groups: 2, capacity, token, pauseMs }); const workerReady = await timeout(ready, 5000);
      const deadline = performance.now() + 5000;
      while (!Atomics.load(cells, RING.nativeLower)) { if (performance.now() > deadline) throw Error('first native clock probe missing'); await new Promise(resolve => setTimeout(resolve, 5)); }
      await context.audioWorklet.addModule('/native-pcm-worklet.mjs');
      for (let group = 0; group < 2; group++) {
        const tap = new AudioWorkletNode(context, 'gelabber-native-pcm-tap', { numberOfInputs: 0, numberOfOutputs: 1, outputChannelCount: [1], processorOptions: { sab, groups: 2, group, capacity, uid: group ? 64 : 0, markerCount: 0, mode: 'clock' } });
        tap.connect(context.destination); taps.push(tap);
      }
      await context.resume(); states.push(context.state); handler = () => states.push(context.state); context.addEventListener('statechange', handler);
      if (suspendMs) { await new Promise(resolve => setTimeout(resolve, seconds * 400)); await context.suspend(); await new Promise(resolve => setTimeout(resolve, suspendMs)); await context.resume(); await new Promise(resolve => setTimeout(resolve, seconds * 600)); }
      else await new Promise(resolve => setTimeout(resolve, seconds * 1000));
      const reports = await Promise.all(taps.map(tap => timeout(new Promise(resolve => { tap.port.onmessage = ({ data }) => { if (data.type === 'report') resolve(data); }; tap.port.postMessage({ type: 'report' }); }), 3000)));
      Atomics.store(cells, RING.stop, 1n); const observed = await timeout(complete, 5000);
      return { workerReady, observer: observed, taps: reports, states, crossOriginIsolated, audio: { sampleRate: context.sampleRate, baseLatency: context.baseLatency, outputLatency: context.outputLatency, outputTimestampDebugOnly: context.getOutputTimestamp() } };
    } finally {
      Atomics.store(cells, RING.stop, 1n); worker.terminate(); taps.forEach(tap => tap.disconnect());
      if (handler) context.removeEventListener('statechange', handler); await context.close();
    }
  }, { token: bridge.token, seconds, pauseMs, capacity, suspendMs });
  validateBrowserClock({ browser: result.browser, crossOriginIsolated: result.raw.crossOriginIsolated, precision: result.raw.workerReady.precision });
  const { observer, states, taps } = result.raw; validateClockSamples(observer.clocks);
  if (observer.failures.length || observer.missing || states.some(state => state !== 'running') || taps.some(tap => tap.gaps || tap.nonfinite || tap.clipped)) throw Error('clock control callback integrity/suspension failed');
  result.brackets = observer.rows.map(rows => {
    const tested = rows.filter(row => row.firstFrame >= 48000 && row.firstFrame < (seconds - 1) * 48000);
    if (!tested.length) throw Error('clock control produced no callbacks');
    const widths = [], failures = [];
    for (const row of tested) {
      try { if (row.flags || row.inputFrames !== row.frames) throw Error('callback input integrity'); widths.push(boundNativeCallback({ lowerMs: row.lowerMs - POLICY.epsilonMs, upperMs: row.upperMs + POLICY.epsilonMs }, observer.clocks, { epsilonMs: POLICY.epsilonMs, maxWidthMs: POLICY.maxIntervalWidthMs }).widthMs); }
      catch (error) { failures.push({ sequence: row.sequence, error: String(error) }); }
    }
    widths.sort((a, b) => a - b);
    return { callbacks: tested.length, accepted: widths.length, failed: failures.length, minWidthMs: widths[0], medianWidthMs: widths[Math.floor(widths.length / 2)], maxWidthMs: widths.at(-1), failures: failures.slice(0, 10) };
  });
  if (result.brackets.some(group => group.failed)) throw Error('one or more callbacks lacks a sufficiently tight causal bracket');
  result.callback_clock_bracketing_qualified = true;
} catch (error) { result.failures.push(String(error)); }
finally {
  await browser?.close();
  if (rpc && !rpc.failure) {
    try { result.finalStatus = await rpc.call({ op: 'status' }); if (Object.keys(result.finalStatus.peers ?? {}).length || Object.keys(result.finalStatus.sources ?? {}).length) result.failures.push('clock-only native unexpectedly owns peers/sources at cleanup'); }
    catch (error) { result.failures.push(String(error)); }
  }
  await bridge?.close(); result.cleanup.native = await rpc?.close();
  result.cleanup.zeroPeerRpcContract = !Object.keys(result.cleanup.native?.calls ?? {}).some(op => !['clock', 'status'].includes(op));
  result.provenance.helpersAfter = hashes();
  if (JSON.stringify(provenance.helpers) !== JSON.stringify(result.provenance.helpersAfter) || sha256(values.binary) !== provenance.nativeSha256 || sha256(values.chromium) !== provenance.chromiumSha256 || sha256(values.library) !== provenance.librarySha256) result.failures.push('executed instrument/source freeze changed during control');
  if (result.cleanup.native?.code !== 0 || !result.cleanup.zeroPeerRpcContract) result.failures.push('owned native cleanup/zero-peer contract failed');
  if (result.failures.length) result.callback_clock_bracketing_qualified = false;
  await writeFile(join(values.output, 'report.json'), JSON.stringify(result));
  const { raw, ...summary } = result; await writeFile(join(values.output, 'summary.json'), JSON.stringify({ ...summary, reportSha256: sha256(join(values.output, 'report.json')) }, null, 2) + '\n');
  console.log(JSON.stringify({ output: values.output, callback_clock_bracketing_qualified: result.callback_clock_bracketing_qualified, pcm_latency_calibrated: false, brackets: result.brackets, failures: result.failures, zeroPeerRpcContract: result.cleanup.zeroPeerRpcContract }, null, 2));
}
if (!result.callback_clock_bracketing_qualified) process.exitCode = 1;
