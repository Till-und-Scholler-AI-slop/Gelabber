#!/usr/bin/env node
// Read-only plan by default. --execute owns only a new local browser/native pair.
import { readFileSync, existsSync } from 'node:fs';
import { mkdir, copyFile, writeFile } from 'node:fs/promises';
import { join, dirname, basename, isAbsolute } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { parseArgs } from 'node:util';
import { sha256 } from './native-pcm-bridge.mjs';
import { V2_RUNTIME, V2StageTimeout, withV2Deadline, v2StageBudgets, captureOwnedBrowser, closeV2Resources, startV2Native, startV2Bridge, validateV2Greeting } from './native-pcm-v2-bridge.mjs';
import { uniqueJson, canonical, readReplayPair, replayRequest, qualifyReplayPair } from './native-pcm-replay-contract.mjs';
import { v2Publication, inboundSnapshot } from './native-pcm-v2-receiver.mjs';
import { NATIVE_PCM_POLICY as POLICY, validateBrowserClock } from './native-pcm-policy.mjs';
import { executedChromium } from './browser-provenance.mjs';

const requireThat = (condition, message) => { if (!condition) throw Error(message); };
const directory = dirname(fileURLToPath(import.meta.url));
const helperFiles = ['native-pcm-v2-control.mjs', 'native-pcm-v2-bridge.mjs', 'native-pcm-v2-browser.mjs', 'native-pcm-v2-receiver.mjs', 'native-pcm-bridge.mjs', 'native-pcm-clock-bounds.mjs', 'native-pcm-replay-contract.mjs', 'native-pcm-evidence.mjs', 'native-pcm-policy.mjs', 'native-pcm-observer.mjs', 'native-pcm-ring.mjs', 'native-pcm-worklet.mjs', 'pcm-kernel.mjs', 'native-peer-checks.mjs', 'browser-provenance.mjs', 'package-lock.json'];
const hashes = folder => Object.fromEntries(helperFiles.map(name => [name, sha256(join(folder, name))]));
export function parseV2Options(args) {
  const { values } = parseArgs({ args, options: {
    execute: { type: 'boolean', default: false }, pair: { type: 'string' }, 'pair-sha256': { type: 'string' }, generation: { type: 'string' }, 'generation-sha256': { type: 'string' }, binary: { type: 'string' }, video: { type: 'string' }, 'video-sha256': { type: 'string' }, library: { type: 'string' }, chromium: { type: 'string' }, output: { type: 'string' },
    'audio-hold-ms': { type: 'string', default: '0' }, capacity: { type: 'string', default: '4096' }, 'observer-pause-ms': { type: 'string', default: '.25' }, 'clock-delay-ms': { type: 'string', default: '0' }, 'suspend-ms': { type: 'string', default: '0' },
  } });
  for (const field of ['pair', 'generation', 'binary', 'video', 'library', 'chromium', 'output']) requireThat(typeof values[field] === 'string' && isAbsolute(values[field]), 'absolute --' + field + ' required');
  for (const field of ['pair-sha256', 'generation-sha256', 'video-sha256']) requireThat(/^[a-f0-9]{64}$/.test(values[field] ?? ''), 'frozen --' + field + ' required');
  const audioHoldMs = Number(values['audio-hold-ms']), capacity = Number(values.capacity), pauseMs = Number(values['observer-pause-ms']), clockDelayMs = Number(values['clock-delay-ms']), suspendMs = Number(values['suspend-ms']);
  requireThat(/^(0|50|200|500)$/.test(values['audio-hold-ms']) && [0, 50, 200, 500].includes(audioHoldMs) && Number.isInteger(capacity) && capacity >= 2 && capacity <= 32768 && [pauseMs, clockDelayMs, suspendMs].every(value => Number.isFinite(value) && value >= 0 && value <= 1000), 'bounded actual V2 hold/ring/failure-control policy required');
  return { values, execute: values.execute, audioHoldMs, capacity, pauseMs, clockDelayMs, suspendMs };
}

export function makeV2Plan(options) {
  const { values } = options;
  requireThat(!existsSync(values.output), 'fresh owned V2 output directory required');
  requireThat(process.version === POLICY.nodeVersion && sha256(process.execPath) === POLICY.nodeSha256 && sha256(values.chromium) === POLICY.chromiumSha256 && sha256(values.binary) === V2_RUNTIME.nativeSha256 && sha256(values.library) === V2_RUNTIME.librarySha256, 'actual pinned V2 Node/Chromium/native/libopus freeze differs');
  requireThat(sha256(values.pair) === values['pair-sha256'] && sha256(values.generation) === values['generation-sha256'] && sha256(values.video) === values['video-sha256'], 'actual frozen pair/generation/video SHA differs');
  const manifest = uniqueJson(readFileSync(values.pair, 'utf8')), generation = uniqueJson(readFileSync(values.generation, 'utf8'));
  requireThat(isAbsolute(manifest.mic?.path ?? '') && isAbsolute(manifest.source?.path ?? ''), 'absolute frozen audio paths required');
  const pair = readReplayPair(manifest);
  requireThat(pair.provenance.library_sha256 === V2_RUNTIME.librarySha256 && generation.run_id === pair.run_id && canonical(generation.provenance) === canonical(pair.provenance) && generation.comparison_available === false && generation.pcm_latency_calibrated === false, 'recorded source generation/pair provenance differs');
  for (const [name, path, expected] of [['pair', values.pair, values['pair-sha256']], ['video', values.video, values['video-sha256']], ['mic', manifest.mic.path, manifest.mic.sha256], ['source', manifest.source.path, manifest.source.sha256]]) requireThat(generation.artifacts?.[basename(path)] === expected, 'recorded generation artifact SHA differs: ' + name);
  const plan = { schema: 1, execute: options.execute, mode: 'native-v2-direct-loopback', comparison_available: false, pcm_latency_calibrated: false, scope: 'owned finite native-to-browser decoded-input instrument; no SFU/resource/product/device or migration acceptance', run_id: pair.run_id, codebook_sha256: pair.codebook_sha256, measurement_seconds: pair.measurement_seconds, total_seconds: pair.total_seconds, request: replayRequest(pair, options.audioHoldMs), config: { audioHoldMs: options.audioHoldMs, capacity: options.capacity, pauseMs: options.pauseMs, clockDelayMs: options.clockDelayMs, suspendMs: options.suspendMs },
    provenance: { nodeVersion: process.version, nodeSha256: sha256(process.execPath), chromiumSha256: sha256(values.chromium), nativeSha256: sha256(values.binary), librarySha256: sha256(values.library), pairSha256: values['pair-sha256'], generationSha256: values['generation-sha256'], videoSha256: values['video-sha256'], micSha256: manifest.mic.sha256, sourceSha256: manifest.source.sha256, helpers: hashes(directory) }, source_generation: generation,
    deadline_budgets: v2StageBudgets(pair.total_seconds, options.audioHoldMs), requirements: ['complete connected three-track native receiver graph before finite start', 'genuine same-report zero initial counters; unavailable is failure', 'same held live/enabled receiver MID/MSID/SSRC/codec through complete decoded tail', 'actual whole PN/input/callback/native clock and cleanup evidence'], output: values.output };
  return { plan, pair, manifest };
}

export function validateV2Preflight(prepared, nativeStatus, capturedOffer, browserClock) {
  requireThat(prepared?.ready === true && prepared.contextState === 'running' && prepared.sampleRate === 48000 && prepared.connection === 'connected' && prepared.graph?.receivers === 3 && prepared.graph.held?.length === 3 && prepared.initial?.length === 2, 'actual complete V2 browser preparation required');
  requireThat(canonical(prepared.offer) === canonical(capturedOffer), 'browser native offer differs from actual captured RPC');
  const bindings = v2Publication(capturedOffer);
  for (const binding of bindings) {
    const graph = prepared.graph.held.filter(value => value.role === binding.role);
    requireThat(graph.length === 1 && graph[0].mid === binding.mid && graph[0].track_id === binding.track_id && graph[0].stream_id === binding.stream_id && graph[0].ssrc === binding.ssrc && graph[0].live === true && graph[0].enabled === true, 'actual prepared held receiver identity differs');
    if (binding.kind === 'audio') {
      const snapshots = prepared.initial.filter(value => value.role === binding.role); requireThat(snapshots.length === 1, 'actual zero initial receiver inventory differs');
      const verified = inboundSnapshot(binding, snapshots[0].stats, { initial: true }); requireThat(canonical(verified) === canonical(snapshots[0]), 'actual zero initial stats cannot be replaced by summaries');
    }
  }
  requireThat(new Set(prepared.initial.map(value => value.row.id)).size === 2, 'distinct actual initial inbound reports required');
  requireThat(Object.keys(nativeStatus.peers ?? {}).join(',') === 'publish' && Object.keys(nativeStatus.sources ?? {}).length === 0 && nativeStatus.peers.publish.connection === 'connected', 'native whole connected publisher must have no started source before preflight');
  const senders = nativeStatus.peers.publish.negotiated_senders; requireThat(senders?.length === 3, 'native negotiated three-source sender inventory required');
  for (const binding of bindings) requireThat(senders.filter(sender => sender.mid === binding.mid && sender.track_id === binding.track_id && sender.stream_id === binding.stream_id && sender.encodings?.length === 1 && sender.encodings[0].ssrc === binding.ssrc && sender.encodings[0].active === true).length === 1, 'actual native negotiated MID/MSID/SSRC sender differs');
  validateBrowserClock(browserClock); return bindings;
}

export async function runV2Control(options) {
  const { plan, pair, manifest } = makeV2Plan(options), { values } = options;
  if (!options.execute) { console.log(JSON.stringify(plan, null, 2)); return plan; }
  await mkdir(values.output, { mode: 0o700 }); await mkdir(join(values.output, 'inputs'), { mode: 0o700 });
  const inputs = join(values.output, 'inputs');
  for (const name of helperFiles) await copyFile(join(directory, name), join(inputs, name));
  for (const [name, path] of [['pair.json', values.pair], ['generation.json', values.generation], ['video.rtpbin', values.video], ['mic.opusbin', manifest.mic.path], ['source.opusbin', manifest.source.path]]) await copyFile(path, join(inputs, name));
  requireThat(canonical(hashes(inputs)) === canonical(plan.provenance.helpers) && sha256(join(inputs, 'pair.json')) === plan.provenance.pairSha256 && sha256(join(inputs, 'generation.json')) === plan.provenance.generationSha256 && ['video', 'mic', 'source'].every(name => sha256(join(inputs, name === 'video' ? name + '.rtpbin' : name + '.opusbin')) === plan.provenance[name + 'Sha256']), 'owned frozen V2 copies differ');
  const result = { ...plan, execute: true, qualified: false, replay_evidence_qualified: false, failures: [], stageTimeouts: [], cleanup: {} }, budgets = plan.deadline_budgets;
  const fail = error => { result.failures.push(String(error)); if (error instanceof V2StageTimeout) result.stageTimeouts.push(error.evidence); };
  let rpc, bridge, browser, browserServer, ownership, page;
  try {
    rpc = await startV2Native({ binary: values.binary, binarySha256: plan.provenance.nativeSha256, video: join(inputs, 'video.rtpbin'), mic: join(inputs, 'mic.opusbin'), source: join(inputs, 'source.opusbin'), library: values.library });
    result.greeting = rpc.actualGreeting; result.actualMappedLibrary = rpc.actualMappedLibrary;
    validateV2Greeting(pair, result.greeting, { binarySha256: plan.provenance.nativeSha256, videoSha256: plan.provenance.videoSha256, library: values.library });
    result.initialNativeStatus = await rpc.call({ op: 'status' }); requireThat(Object.keys(result.initialNativeStatus.peers ?? {}).length === 0 && Object.keys(result.initialNativeStatus.sources ?? {}).length === 0, 'fresh V2 native must own zero initial peers/sources');
    bridge = await startV2Bridge(rpc, { staticDirectory: inputs, clockDelayMs: options.clockDelayMs });
    const { chromium } = await withV2Deadline('playwright-import', 10000, () => import('playwright'));
    browserServer = await withV2Deadline('browser-launch', budgets.launchMs, () => chromium.launchServer({ host: '127.0.0.1', timeout: 30000, headless: true, executablePath: values.chromium, args: ['--autoplay-policy=no-user-gesture-required', '--no-sandbox', '--allow-loopback-in-peer-connection', '--disable-features=WebRtcHideLocalIpsWithMdns'] }));
    ownership = captureOwnedBrowser(browserServer); result.ownedBrowserIdentity = ownership.identity;
    requireThat(ownership.identity.executableSha256 === POLICY.chromiumSha256, 'fresh actual owned browser executable differs');
    browser = await withV2Deadline('browser-connect', budgets.connectMs, () => chromium.connect(browserServer.wsEndpoint(), { timeout: budgets.connectMs }));
    result.executedBrowser = await withV2Deadline('browser-provenance', budgets.provenanceMs, () => executedChromium(browser));
    requireThat(result.executedBrowser.sha256 === POLICY.chromiumSha256 && result.executedBrowser.revision === POLICY.chromiumRevision && result.executedBrowser.product.endsWith(POLICY.chromiumVersion), 'actual executed V2 Chromium hash/revision differs');
    page = await withV2Deadline('browser-page', budgets.pageMs, () => browser.newPage()); page.on('pageerror', fail); await withV2Deadline('browser-navigation', budgets.navigateMs, () => page.goto(bridge.url, { timeout: budgets.navigateMs }));
    const archives = pair.archives.map(archive => ({ role: archive.role, uid: archive.uid, archiveSha256: archive.archive_sha256, packets: archive.metadata.packets, samples: archive.metadata.decode_control.samples, lookaheadSamples: archive.metadata.encoder.lookahead_samples, markers: archive.metadata.pn.markers }));
    result.preflight = await withV2Deadline('browser-prepare', budgets.prepareMs, () => page.evaluate(async options => (await import('/native-pcm-v2-browser.mjs')).prepareV2Replay(options), { token: bridge.token, archives, totalSeconds: pair.total_seconds, audioHoldMs: options.audioHoldMs, capacity: options.capacity, pauseMs: options.pauseMs, runId: pair.run_id, codebookSha256: pair.codebook_sha256 }));
    result.preStartNativeStatus = await rpc.call({ op: 'status' });
    const browserClock = { browser: { product: result.executedBrowser.product, revision: result.executedBrowser.revision }, crossOriginIsolated: result.preflight.crossOriginIsolated, precision: result.preflight.workerReady?.precision, chromium_sha256: result.executedBrowser.sha256, node_sha256: plan.provenance.nodeSha256 };
    validateV2Preflight(result.preflight, result.preStartNativeStatus, bridge.signaling.find(value => value.request.op === 'offer')?.response.description, browserClock);
    result.nativeStartRpcBeforeNs = process.hrtime.bigint().toString(); result.start = await rpc.call(replayRequest(pair, options.audioHoldMs)); result.nativeStartRpcAfterNs = process.hrtime.bigint().toString();
    result.raw = await withV2Deadline('browser-finish', budgets.finishMs, () => page.evaluate(async options => (await import('/native-pcm-v2-browser.mjs')).finishV2Replay(options), { start: result.start, suspendMs: options.suspendMs }));
    requireThat(result.raw.completed === true && result.raw.failures.length === 0, 'actual V2 receive control failed: ' + result.raw.failures.join('; '));
    const runtime = { greeting: result.greeting, start: result.start, status: result.raw.status, nativeBinarySha256: plan.provenance.nativeSha256, audioHoldMs: options.audioHoldMs };
    result.qualification = qualifyReplayPair({ pair, runtime, groups: result.raw.groups, observer: result.raw.observer, contextStates: result.raw.contextStates, browserClock });
    requireThat(result.qualification.qualified === true, result.qualification.failures.join('; ')); result.replay_evidence_qualified = true;
  } catch (error) { fail(error); }
  finally {
    const closed = await closeV2Resources({ dispose: page ? () => page.evaluate(async () => (await import('/native-pcm-v2-browser.mjs')).disposeV2Replay()) : undefined, browser, server: browserServer, ownership, bridge, native: rpc, budgets, onFailure: fail });
    result.browserDisposal = closed.browserDisposal; result.cleanup = closed.cleanup;
    if (result.browserDisposal?.cleanupErrors?.length) result.failures.push(...result.browserDisposal.cleanupErrors);
    if (bridge) { result.signaling = bridge.signaling; result.clockCalls = bridge.clockCalls(); }
    try {
      result.helpersAfter = hashes(directory);
      requireThat(canonical(result.helpersAfter) === canonical(plan.provenance.helpers) && canonical(hashes(inputs)) === canonical(plan.provenance.helpers) && sha256(values.binary) === plan.provenance.nativeSha256 && sha256(values.chromium) === plan.provenance.chromiumSha256 && sha256(values.library) === plan.provenance.librarySha256 && sha256(values.pair) === plan.provenance.pairSha256 && sha256(values.generation) === plan.provenance.generationSha256 && sha256(values.video) === plan.provenance.videoSha256 && sha256(manifest.mic.path) === plan.provenance.micSha256 && sha256(manifest.source.path) === plan.provenance.sourceSha256 && sha256(join(inputs, 'pair.json')) === plan.provenance.pairSha256 && sha256(join(inputs, 'generation.json')) === plan.provenance.generationSha256 && ['video', 'mic', 'source'].every(name => sha256(join(inputs, name === 'video' ? name + '.rtpbin' : name + '.opusbin')) === plan.provenance[name + 'Sha256']), 'actual V2 runtime/source/helper freeze changed during control');
    } catch (error) { result.failures.push(String(error)); }
    const disposal = result.browserDisposal?.cleanup;
    const calls = result.cleanup.native?.calls ?? {};
    if (Object.keys(calls).some(op => !['clock', 'status', 'create', 'offer', 'remote', 'ice', 'start'].includes(op)) || ['create', 'offer', 'remote', 'start'].some(op => (calls[op] ?? 0) > 1) || (calls.start ?? 0) !== (result.start ? 1 : 0)) result.failures.push('owned V2 RPC operation inventory failed');
    if (result.cleanup.native?.code !== 0 || result.cleanup.native.signal !== null || result.cleanup.native.failure || result.cleanup.native.cleanupTimedOut || result.cleanup.browser?.closed !== true || result.cleanup.browser.processExited !== true || result.cleanup.browser.exit?.code !== 0 || result.cleanup.browser.exit.signal !== null || !result.cleanup.browser.graceful || result.cleanup.browser.forced || disposal?.audioContext !== 'closed' || disposal.peerConnection !== 'closed' || disposal.workerTerminated !== true || disposal.nodesDisconnected !== true || result.cleanup.bridge?.closed !== true) result.failures.push('owned V2 complete normal browser/native/HTTP/audio/worker cleanup unqualified');
    result.qualified = result.replay_evidence_qualified && result.failures.length === 0;
    await writeFile(join(values.output, 'report.json'), JSON.stringify(result) + '\n', { mode: 0o600 });
    const { raw, browserDisposal, ...summary } = result; await writeFile(join(values.output, 'summary.json'), JSON.stringify({ ...summary, reportSha256: sha256(join(values.output, 'report.json')) }, null, 2) + '\n', { mode: 0o600 });
    console.log(JSON.stringify({ output: values.output, qualified: result.qualified, comparison_available: false, pcm_latency_calibrated: false, failures: result.failures, reportSha256: sha256(join(values.output, 'report.json')) }, null, 2));
  }
  if (!result.qualified) process.exitCode = 1; return result;
}

if (process.argv[1] && pathToFileURL(process.argv[1]).href === import.meta.url) {
  const result = await runV2Control(parseV2Options(process.argv.slice(2)));
  // A failed owned API must not keep the CLI alive through pending IPC handles
  // after bounded cleanup and durable report writes. Playwright also registers
  // exit cleanup for its freshly launched browser process, not handoff PIDs.
  if (result.execute && (result.stageTimeouts.length || result.cleanup.browser?.processExited !== true || result.cleanup.browser?.closed !== true)) process.exit(1);
}
