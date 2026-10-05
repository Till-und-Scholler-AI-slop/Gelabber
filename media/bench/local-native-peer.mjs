#!/usr/bin/env node
// Bounded two-endpoint loopback instrument control, before any SFU/full-N run.
import { createServer } from 'node:http';
import { createHash } from 'node:crypto';
import { readFile, writeFile } from 'node:fs/promises';
import { parseArgs } from 'node:util';
import { networkInterfaces } from 'node:os';
import { chromium } from 'playwright';
import { NativePeer } from './native-peer.mjs';
import { nativeExitEvidence, decodedVideo } from './native-video.mjs';
import { nativeBrowserAudio, nativePublicationIdentity } from './native-peer-checks.mjs';

const { values } = parseArgs({ options: { execute: { type: 'boolean', default: false },
  binary: { type: 'string' }, video: { type: 'string' }, mic: { type: 'string' }, source: { type: 'string' },
  output: { type: 'string' }, chromium: { type: 'string' }, seconds: { type: 'string', default: '30' }, 'bind-interface': { type: 'string' }, 'runtime-provenance': { type: 'string' } } });
for (const field of ['binary', 'video', 'mic', 'source', 'output', 'chromium']) if (!values[field]) throw new Error('--' + field + ' required');
const seconds = Number(values.seconds);
if (!Number.isInteger(seconds) || seconds < 20 || seconds > 60 || seconds % 10) throw new Error('whole ten-second control duration20..60 required');
const interfaces = networkInterfaces();
let bind = '127.0.0.1';
if (values['bind-interface']) {
  const candidates = interfaces[values['bind-interface']]?.filter(value => value.family === 'IPv4' && !value.internal);
  if (candidates?.length !== 1) throw new Error('selected interface must have one explicit IPv4 address');
  bind = candidates[0].address;
}
const hash = async path => createHash('sha256').update(await readFile(path)).digest('hex');
const provenance = { node_version: process.version, node_sha256: await hash(process.execPath), browser_sha256: await hash(values.chromium), interfaces, bind,
  inputs: Object.fromEntries(await Promise.all(['binary', 'video', 'mic', 'source'].map(async key => [key, await hash(values[key])]))),
  helpers: Object.fromEntries(await Promise.all(['local-native-peer.mjs', 'native-peer.mjs', 'native-video.mjs', 'native-peer-checks.mjs'].map(async name => [name, await hash(new URL(name, import.meta.url))]))) };
if (values['runtime-provenance']) {
  const runtime = JSON.parse(await readFile(values['runtime-provenance'], 'utf8'));
  if (!runtime.offline_controls_valid || !runtime.cleanup_valid || runtime.image_labels?.['gelabber.bench.native.sha256'] !== provenance.inputs.binary) throw new Error('runtime control is missing/invalid or native binary differs');
  provenance.runtime = { image_id: runtime.image_id, base_image_id: runtime.base_image_id, labels: runtime.image_labels,
    glibc: runtime.executed_glibc, native_ldd: runtime.executed_native_ldd, library_ldd: runtime.executed_library_ldd,
    control_report_sha256: await hash(values['runtime-provenance']) };
}
if (process.version !== 'v26.8.2' || provenance.node_sha256 !== '8a22a371fd85aecf5411636574309f6380fbc42694aaf0651a089a8ef9c44e52') throw new Error('executed pinned Node26.8.2 required');
if (provenance.browser_sha256 !== 'ded93a9c9a53a1ae040f08124badcca95c938e9d5015ff340c3b5538c41bf39e') throw new Error('executed pinned Chrome153 binary required');
if (!values.execute) { console.log(JSON.stringify({ execute: false, seconds, provenance, scope: 'local loopback; no SFU/WAN/latency acceptance' }, null, 2)); process.exit(0); }
const result = { schema: 1, scope: 'one native peer0 and one browser peer1 on the same local network namespace; 2 microphones+1VP8+1separateSourceAudio; no SFU or WAN',
  comparison_available: false, production_feature_acceptance: false, pcm_latency_calibrated: false, provenance, samples: [], failures: [], cleanup_errors: [] };
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
let source, browser, page, server;
try {
  source = new NativePeer(values.binary, values.video, values.mic, values.source, bind);
  result.native_provenance = (await source.ready).provenance;
  await source.call({ op: 'create', peer: 'publish', publish: true });
  const offer = (await source.call({ op: 'offer' })).description;
  result.publication_identity = nativePublicationIdentity(offer);
  server = createServer((request, response) => { response.writeHead(200, { 'content-type': 'text/html' }); response.end('<!doctype html><title>Private native peer0 loopback control</title>'); });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  browser = await chromium.launch({ headless: true, executablePath: values.chromium, args: ['--autoplay-policy=no-user-gesture-required', '--no-sandbox'] });
  const cdp = await browser.newBrowserCDPSession(); result.executed_browser = await cdp.send('Browser.getVersion');
  page = await browser.newPage(); page.on('pageerror', error => result.failures.push(String(error)));
  await page.goto('http://127.0.0.1:' + server.address().port);
  const answer = await page.evaluate(async offer => {
    const pc = window.pc = new RTCPeerConnection({ iceServers: [] }); window.tracks = []; window.media = []; window.errors = [];
    pc.ontrack = ({ track }) => {
      window.tracks.push(track); const element = document.createElement(track.kind === 'video' ? 'video' : 'audio');
      element.autoplay = true; element.muted = true; element.srcObject = new MediaStream([track]); document.body.append(element);
      window.media.push(element); element.play().catch(error => window.errors.push(String(error)));
    };
    const context = window.audioContext = new AudioContext({ sampleRate: 48000 });
    const destination = context.createMediaStreamDestination();
    for (const frequency of [317, 719, 1249, 2027]) {
      const tone = context.createOscillator(), gain = context.createGain(); tone.frequency.value = frequency + 13; gain.gain.value = .07;
      tone.connect(gain).connect(destination); tone.start();
    }
    await context.resume(); await pc.setRemoteDescription(offer);
    pc.addTrack(destination.stream.getAudioTracks()[0], destination.stream);
    await pc.setLocalDescription(await pc.createAnswer());
    for (const sender of pc.getSenders()) if (sender.track) {
      const parameters = sender.getParameters();
      if (!parameters.encodings?.length) throw new Error('browser microphone RTP encoding missing after answer');
      parameters.encodings[0].maxBitrate = 128000; await sender.setParameters(parameters);
    }
    const deadline = performance.now() + 15000;
    while (pc.iceGatheringState !== 'complete') { if (performance.now() > deadline) throw new Error('loopback ICE gathering deadline'); await new Promise(resolve => setTimeout(resolve, 25)); }
    window.collect = async () => ({ stats: [...await pc.getStats()].map(([, value]) => value), errors: [...window.errors],
      connection: pc.connectionState, tracks: window.tracks.map(track => ({ id: track.id, kind: track.kind, state: track.readyState })),
      elements: window.media.map(element => ({ ready: element.readyState, time: element.currentTime })) });
    return pc.localDescription;
  }, offer);
  await source.call({ op: 'remote', peer: 'publish', description: answer });
  result.timeline = (await source.call({ op: 'start', peer: 'publish', seconds })).timeline;
  const started = performance.now();
  for (let index = 0; index <= seconds; index++) {
    await sleep(Math.max(0, started + index * 1000 - performance.now()));
    const native = await source.call({ op: 'status' }), receiver = await page.evaluate(() => window.collect());
    result.samples.push({ at_seconds: (performance.now() - started) / 1000, native, receiver });
    for (const state of Object.values(native.sources)) if (state.error) throw new Error(state.error);
  }
  const measurements = result.samples.filter(sample => sample.at_seconds >= 5 && sample.at_seconds <= seconds - 1);
  result.video = decodedVideo(measurements.map(sample => sample.receiver), result.native_provenance.video.metadata.rtp_payload_bitrate_bps);
  if (!result.video.valid) result.failures.push(...result.video.failures);
  const first = measurements[0], last = measurements.at(-1), windowSeconds = last.at_seconds - first.at_seconds;
  const received = Object.values(last.native.peers.publish.received);
  if (received.length !== 1 || received[0].decode_errors || received[0].sequence_gaps || received[0].timestamp_gaps || received[0].reordered_or_duplicate_packets) result.failures.push('native reverse microphone decoder/packet graph differs');
  if (received.length === 1) {
    const edge = received[0], previous = first.native.peers.publish.received[String(edge.ssrc)];
    const sent = last.receiver.stats.find(value => value.type === 'outbound-rtp' && value.kind === 'audio');
    if (!previous || sent?.ssrc !== edge.ssrc) result.failures.push('native microphone source cannot bind to actual browser sender SSRC');
    else {
      await source.call({ op: 'bind', peer: 'publish', ssrc: edge.ssrc, source_name: 'peer-1/mic' });
      result.native_reverse_audio = { decoded_sample_rate: (edge.decoded_samples - previous.decoded_samples) / windowSeconds,
        payload_bitrate_bps: (edge.payload_bytes_received - previous.payload_bytes_received) * 8 / windowSeconds, binding: 'actual browser outbound SSRC→native receive SSRC' };
      if (Math.abs(result.native_reverse_audio.decoded_sample_rate - 48000) > 960) result.failures.push('native decoded microphone sample rate differs');
      if (Math.abs(result.native_reverse_audio.payload_bitrate_bps - 128000) > 12800) result.failures.push('reverse microphone actual bitrate differs');
    }
  }
  const audio = nativeBrowserAudio(measurements.map(sample => sample.receiver));
  result.browser_audio = audio.roles;
  if (!audio.valid) result.failures.push(...audio.failures);
  // start() schedules all tracks100ms in the future; observe their completion
  // after that final boundary rather than racing it from the RPC reply time.
  await sleep(200);
  result.final_native_status = await source.call({ op: 'status' });
  for (const name of ['mic', 'source', 'video']) if (!result.final_native_status.sources[name]?.source_policy_valid) result.failures.push(name + ' replay did not complete its fixed schedule');
} catch (error) { result.failures.push(String(error)); }
finally {
  if (page) try { await page.evaluate(async () => { window.pc?.close(); await window.audioContext?.close(); return true; }); } catch (error) { result.cleanup_errors.push(String(error)); }
  if (browser) try { await browser.close(); } catch (error) { result.cleanup_errors.push(String(error)); }
  if (source) {
    try { await source.close(); } catch (error) { result.cleanup_errors.push(String(error)); }
    result.native_exit = nativeExitEvidence(source.child); if (!result.native_exit.clean) result.cleanup_errors.push('native peer did not exit normally');
  }
  if (server) try { await new Promise((resolve, reject) => server.close(error => error ? reject(error) : resolve())); } catch (error) { result.cleanup_errors.push(String(error)); }
}
result.cleanup_valid = result.cleanup_errors.length === 0;
result.instrument_loopback_valid = result.failures.length === 0 && result.cleanup_valid;
await writeFile(values.output, JSON.stringify(result, null, 2), { flag: 'wx' });
console.log(JSON.stringify({ output: values.output, valid: result.instrument_loopback_valid, failures: result.failures, cleanup_errors: result.cleanup_errors }, null, 2));
process.exitCode = result.instrument_loopback_valid ? 0 : 1;
