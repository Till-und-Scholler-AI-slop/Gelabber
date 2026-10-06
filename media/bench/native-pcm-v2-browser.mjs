// Imported only in the owned browser page. Preparation cannot start any RTP.
import { RING, dimensions } from './native-pcm-ring.mjs';
import { v2Publication, holdTrack, validateHeldObjects, inboundSnapshot, receiverEvidence } from './native-pcm-v2-receiver.mjs';

let owned;
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
const requireThat = (condition, message) => { if (!condition) throw Error(message); };
async function deadline(promise, ms) {
  let timer; try { return await Promise.race([promise, new Promise((_, reject) => { timer = setTimeout(() => reject(Error('owned V2 browser deadline')), ms); })]); }
  finally { clearTimeout(timer); }
}
async function rpc(request) {
  const response = await fetch('/rpc', { method: 'POST', headers: { authorization: 'Bearer ' + owned.options.token, 'content-type': 'application/json' }, body: JSON.stringify(request), signal: AbortSignal.timeout(35000) });
  const value = await response.json(); requireThat(response.ok && !value.error, value.error ?? 'owned V2 native HTTP request failed'); return value;
}
async function snapshotAll(initial = false) {
  validateHeldObjects(owned.pc, owned.held); const snapshots = [];
  for (const role of ['mic', 'source']) {
    const binding = owned.held.get(role), stats = [...(await binding.receiver.getStats()).values()];
    owned.lastStats[role] = stats;
    snapshots.push(inboundSnapshot(binding, stats, { initial, previous: initial ? undefined : owned.previous?.find(value => value.role === role) }));
  }
  const pcStats = [...(await owned.pc.getStats()).values()]; owned.lastPcStats = pcStats;
  const audio = pcStats.filter(row => row.type === 'inbound-rtp' && row.kind === 'audio');
  requireThat(audio.length === 2 && new Set(audio.map(row => row.id)).size === 2 && audio.every(row => snapshots.some(value => value.row.id === row.id && value.row.ssrc === row.ssrc && value.row.mid === row.mid && value.row.codecId === row.codecId)), 'whole actual browser inbound audio inventory differs');
  validateHeldObjects(owned.pc, owned.held); owned.previous = snapshots; return snapshots;
}
async function tapReports() {
  return Promise.all(owned.taps.map(tap => deadline(new Promise(resolve => { tap.port.onmessage = ({ data }) => { if (data.type === 'report') resolve(data); }; tap.port.postMessage({ type: 'report' }); }), 3000)));
}
function actualLastInputEnd(group) {
  const { cells, options } = owned, base = RING.header + group * dimensions(2, options.capacity).stride;
  const sequence = Atomics.load(cells, base); if (sequence <= 0n) return -1;
  const slot = base + RING.groupHeader + (Number(sequence) - 1) % options.capacity * RING.row, before = Atomics.load(cells, slot);
  const frame = Atomics.load(cells, slot + 1), frames = Atomics.load(cells, slot + 2), flags = Atomics.load(cells, slot + 4), input = Atomics.load(cells, slot + 5), after = Atomics.load(cells, slot);
  if (before !== sequence || after !== sequence || flags !== 0n || input !== frames) return -1;
  return Number(frame + frames); // A published real block, never currentTime projection.
}
function requiredEnd(archive, tap) {
  requireThat(tap.peaks.length === archive.markers.length && archive.markers.every(marker => tap.peaks.filter(peak => peak.sequence === marker.sequence).length === 1), 'every genuine PN marker required before whole received tail');
  return Math.max(...archive.markers.map(marker => tap.peaks.find(peak => peak.sequence === marker.sequence).receivedFrame - marker.source_sample_ordinal - archive.lookaheadSamples + 96 + archive.samples));
}

export async function prepareV2Replay(options) {
  requireThat(!owned, 'one owned browser V2 preparation only');
  owned = { options, held: new Map(), taps: [], sources: [], failures: [], states: [], connectionStates: [], lastStats: {}, samples: [], cleanupErrors: [] };
  const result = { ready: false, comparison_available: false, pcm_latency_calibrated: false };
  try {
    requireThat(crossOriginIsolated === true, 'owned V2 page must be cross-origin isolated');
    await rpc({ op: 'create', peer: 'publish', publish: true }); const offered = await rpc({ op: 'offer', peer: 'publish' });
    owned.offer = offered.description; owned.bindings = v2Publication(owned.offer);
    const pc = owned.pc = new RTCPeerConnection({ iceServers: [] });
    pc.addEventListener('track', event => {
      try {
        const binding = holdTrack(event, owned.bindings, owned.held);
        if (binding.kind === 'video') { const video = owned.video = document.createElement('video'); video.muted = true; video.autoplay = true; video.srcObject = new MediaStream([binding.track]); document.body.append(video); video.play().catch(error => owned.failures.push(String(error))); }
      } catch (error) { owned.failures.push(String(error)); }
    });
    await pc.setRemoteDescription(owned.offer); await pc.setLocalDescription(await pc.createAnswer());
    const gatheringDeadline = performance.now() + 15000;
    while (pc.iceGatheringState !== 'complete') { requireThat(performance.now() < gatheringDeadline, 'browser actual ICE gathering deadline'); await sleep(10); }
    owned.answer = { type: pc.localDescription.type, sdp: pc.localDescription.sdp };
    await rpc({ op: 'remote', peer: 'publish', description: owned.answer });
    const graphDeadline = performance.now() + 30000;
    while (pc.connectionState !== 'connected' || owned.held.size !== 3) { requireThat(!owned.failures.length && !['failed', 'closed'].includes(pc.connectionState) && performance.now() < graphDeadline, 'connected actual whole receiver graph deadline: ' + owned.failures.join('; ')); await sleep(10); }
    validateHeldObjects(pc, owned.held);
    owned.connectionStates.push(pc.connectionState); pc.addEventListener('connectionstatechange', owned.connectionHandler = () => owned.connectionStates.push(pc.connectionState));
    const shape = dimensions(2, options.capacity), sab = new SharedArrayBuffer(shape.cells * 8); owned.cells = new BigInt64Array(sab);
    owned.context = new AudioContext({ sampleRate: 48000 }); requireThat(owned.context.sampleRate === 48000, 'actual AudioContext sample rate differs');
    let readyResolve, readyReject, completeResolve, completeReject;
    const ready = new Promise((resolve, reject) => { readyResolve = resolve; readyReject = reject; });
    owned.complete = new Promise((resolve, reject) => { completeResolve = resolve; completeReject = reject; }); owned.complete.catch(() => {});
    const worker = owned.worker = new Worker('/native-pcm-observer.mjs', { type: 'module' });
    worker.onmessage = ({ data }) => { if (data.type === 'ready') readyResolve(data); if (data.type === 'complete') completeResolve(data); };
    worker.onerror = event => { readyReject(Error(event.message)); completeReject(Error(event.message)); };
    worker.postMessage({ sab, groups: 2, capacity: options.capacity, token: options.token, pauseMs: options.pauseMs, maxRows: 2000000 });
    owned.workerReady = await deadline(ready, 5000);
    const clockDeadline = performance.now() + 5000;
    while (!Atomics.load(owned.cells, RING.nativeLower)) { requireThat(performance.now() < clockDeadline, 'actual initial native clock probe missing'); await sleep(5); }
    await owned.context.audioWorklet.addModule('/native-pcm-worklet.mjs');
    for (const [group, role] of ['mic', 'source'].entries()) {
      const binding = owned.held.get(role), archive = options.archives.find(value => value.role === role);
      const tap = new AudioWorkletNode(owned.context, 'gelabber-native-pcm-tap', { numberOfInputs: 1, numberOfOutputs: 1, outputChannelCount: [1], channelCount: 1, channelCountMode: 'explicit', channelInterpretation: 'discrete', processorOptions: { sab, groups: 2, group, capacity: options.capacity, uid: binding.uid, markerCount: archive.markers.length, mode: 'receive' } });
      tap.addEventListener('processorerror', () => owned.failures.push('actual receive AudioWorklet processor failed'));
      const source = owned.context.createMediaStreamSource(new MediaStream([binding.track])); source.connect(tap); tap.connect(owned.context.destination); owned.sources.push(source); owned.taps.push(tap);
    }
    await owned.context.resume(); owned.states.push(owned.context.state); owned.context.addEventListener('statechange', owned.stateHandler = () => owned.states.push(owned.context.state));
    const statsDeadline = performance.now() + 5000;
    while (true) {
      try { owned.initial = await snapshotAll(true); break; }
      catch (error) { if (!String(error).includes('genuine pre-replay inbound report unavailable or ambiguous') || performance.now() >= statsDeadline) throw error; await sleep(20); }
    }
    requireThat(!owned.failures.length, owned.failures.join('; '));
    return { ...result, ready: true, offer: owned.offer, answer: owned.answer, bindings: owned.bindings, initial: owned.initial, workerReady: owned.workerReady, crossOriginIsolated, contextState: owned.context.state, sampleRate: owned.context.sampleRate, connection: pc.connectionState, graph: { receivers: pc.getReceivers().length, held: [...owned.held.values()].map(({ receiver, transceiver, track, stream, ...binding }) => ({ ...binding, live: track.readyState === 'live', enabled: track.enabled })) } };
  } catch (error) { owned.failures.push(String(error)); return { ...result, failures: [...owned.failures], offer: owned.offer, lastStats: owned.lastStats, lastPcStats: owned.lastPcStats, workerReady: owned.workerReady }; }
}

export async function finishV2Replay({ start, suspendMs = 0 }) {
  requireThat(owned?.initial && !owned.started, 'actual zero preflight must precede one finite start'); owned.started = true; owned.start = start;
  const result = { completed: false, comparison_available: false, pcm_latency_calibrated: false };
  try {
    requireThat(start.started === true && typeof start.timeline?.startClockNs === 'string' && start.total_seconds === owned.options.totalSeconds, 'actual whole native V2 start required');
    owned.taps.forEach(tap => tap.port.postMessage({ type: 'anchor', startClockNs: start.timeline.startClockNs }));
    const finishDeadline = performance.now() + (owned.options.totalSeconds * 1000 + owned.options.audioHoldMs + 5000), began = performance.now();
    let suspended = false, nativePoll = 0;
    while (true) {
      requireThat(performance.now() < finishDeadline && !owned.failures.length && owned.pc.connectionState === 'connected', 'actual V2 whole receive deadline/connection failure: ' + owned.failures.join('; '));
      if (suspendMs && !suspended && performance.now() - began >= 2000) { suspended = true; await owned.context.suspend(); await sleep(suspendMs); await owned.context.resume(); }
      const snapshots = await snapshotAll(); owned.samples.push({ atPerformanceMs: performance.now(), snapshots });
      if (performance.now() >= nativePoll) { owned.status = await rpc({ op: 'status' }); nativePoll = performance.now() + 100; }
      requireThat(Object.keys(owned.status.peers ?? {}).join(',') === 'publish' && owned.status.peers.publish.connection === 'connected' && ['mic', 'source', 'video'].every(role => owned.status.sources?.[role] && !owned.status.sources[role].error && !owned.status.sources[role].schedule_failure), 'actual native V2 graph/source schedule failed');
      requireThat(snapshots.every(value => { const archive = owned.options.archives.find(archive => archive.role === value.role); return value.row.packetsReceived <= archive.packets && value.row.totalSamplesReceived <= archive.samples; }), 'actual whole receiver packet/sample totals exceed finite archive');
      const whole = snapshots.every(value => { const archive = owned.options.archives.find(archive => archive.role === value.role); return value.row.packetsReceived === archive.packets && value.row.totalSamplesReceived === archive.samples; });
      const complete = ['mic', 'source', 'video'].every(role => owned.status?.sources?.[role]?.completed === true && owned.status.sources[role].source_policy_valid === true && owned.status.sources[role].running === false);
      if (whole && complete) {
        const taps = owned.tapReports = await tapReports();
        if (taps.every((tap, group) => actualLastInputEnd(group) >= requiredEnd(owned.options.archives[group], tap))) break;
      }
      await sleep(20);
    }
    // Retain actual subsequent clock probes so every required callback's
    // observer drain precedes an actual native after-bracket. Never project it.
    await sleep(40); owned.final = await snapshotAll(); owned.status = await rpc({ op: 'status' }); owned.tapReports = await tapReports();
    Atomics.store(owned.cells, RING.stop, 1n); owned.observer = await deadline(owned.complete, 5000);
    requireThat(!owned.failures.length && owned.states.every(state => state === 'running') && owned.connectionStates.every(state => state === 'connected'), 'actual context/receiver connection interrupted during V2 replay');
    result.completed = true;
    result.groups = owned.options.archives.map((archive, group) => ({ role: archive.role, uid: archive.uid, run_id: owned.options.runId, codebook_sha256: owned.options.codebookSha256, archive_sha256: archive.archiveSha256, receiver: receiverEvidence(owned.initial[group], owned.final[group]), tap: owned.tapReports[group] }));
  } catch (error) { owned.failures.push(String(error)); }
  return { ...result, start, status: owned.status, initial: owned.initial, final: owned.final, samples: owned.samples, lastStats: owned.lastStats, lastPcStats: owned.lastPcStats, observer: owned.observer, taps: owned.tapReports, contextStates: [...owned.states], connectionStates: [...owned.connectionStates], failures: [...owned.failures] };
}

export async function disposeV2Replay() {
  if (!owned) return { noOwnedSession: true };
  const cleanup = {}, errors = owned.cleanupErrors;
  if (owned.taps.length) { try { owned.tapReports = await tapReports(); } catch (error) { errors.push(String(error)); } }
  if (owned.cells) Atomics.store(owned.cells, RING.stop, 1n);
  if (owned.worker) {
    try { owned.observer ??= await deadline(owned.complete, 5000); } catch (error) { errors.push(String(error)); }
    owned.worker.terminate(); cleanup.workerTerminated = true;
  }
  for (const node of [...owned.sources, ...owned.taps]) { try { node.disconnect(); } catch (error) { errors.push(String(error)); } }
  cleanup.nodesDisconnected = true;
  if (owned.context) { owned.context.removeEventListener('statechange', owned.stateHandler); try { await deadline(owned.context.close(), 3000); } catch (error) { errors.push(String(error)); } cleanup.audioContext = owned.context.state; }
  if (owned.pc) { if (owned.connectionHandler) owned.pc.removeEventListener('connectionstatechange', owned.connectionHandler); owned.pc.close(); cleanup.peerConnection = owned.pc.connectionState; }
  if (owned.video) { owned.video.srcObject = null; owned.video.remove(); cleanup.videoDetached = true; }
  return { cleanup, cleanupErrors: [...errors], observer: owned.observer, taps: owned.tapReports, failures: [...owned.failures], lastStats: owned.lastStats, lastPcStats: owned.lastPcStats };
}
