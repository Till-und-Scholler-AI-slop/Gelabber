#!/usr/bin/env node
// Local sample-clock calibration, including Opus encode/decode. No SFU or WAN.
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';
import { executedChromium } from './browser-provenance.mjs';
const output = process.argv[2];
if (!output) throw new Error('Usage: node pcm-calibrate.mjs OUTPUT.json');
const folder = path.dirname(fileURLToPath(import.meta.url));
const assets = new Map([['/pcm.js', 'pcm.bundle.js'], ['/pcm-marker.mjs', 'pcm-marker.mjs'], ['/pcm-kernel.mjs', 'pcm-kernel.mjs']]);
const proxy = http.createServer((request, response) => {
  if (request.url === '/') { response.setHeader('content-type', 'text/html'); response.end('<!doctype html><title>PCM calibration</title><script type="module">import { PcmMarkers } from "/pcm-marker.mjs"; window.PcmMarkers=PcmMarkers;</script>'); return; }
  const asset = assets.get(request.url); if (!asset) { response.statusCode = 404; response.end(); return; }
  response.setHeader('content-type', 'text/javascript'); response.end(fs.readFileSync(path.join(folder, asset)));
});
await new Promise(resolve => proxy.listen(0, '127.0.0.1', resolve));
let browser;
const result = { schema: 1, recorded_at: new Date().toISOString(), scope: 'local synthetic PCM sample-clock calibration; no acoustic devices, SFU or WAN', failures: [],
  artifact_sha256: Object.fromEntries(['pcm-kernel.mjs', 'pcm-marker.mjs', 'pcm.bundle.js'].map(name => [name, createHash('sha256').update(fs.readFileSync(path.join(folder, name))).digest('hex')])) };
try {
  browser = await chromium.launch({ headless: true, ...(process.argv.includes('--full-chromium') ? { channel: 'chromium' } : {}), args: ['--enable-automation', '--autoplay-policy=no-user-gesture-required', '--disable-background-timer-throttling', '--disable-renderer-backgrounding'] });
  result.executed_browser = await executedChromium(browser);
  const page = await browser.newPage();
  page.on('pageerror', error => result.failures.push(String(error)));
  await page.goto(`http://127.0.0.1:${proxy.address().port}`);
  await page.waitForFunction(() => !!window.PcmMarkers);
  result.calibration = await page.evaluate(async () => {
    const markers = await window.PcmMarkers.create(), context = markers.context;
    const publisher = new RTCPeerConnection({ iceServers: [] }), receiver = new RTCPeerConnection({ iceServers: [] });
    const timeout = ms => new Promise(resolve => setTimeout(resolve, ms));
    const wait = async predicate => { for (let i = 0; i < 200; i++) { if (predicate()) return; await timeout(50); } throw new Error('local PCM calibration connection timeout'); };
    try {
      const tone = context.createOscillator(), gain = context.createGain(), source = markers.source('calibration', 0);
      tone.frequency.value = 719; gain.gain.value = .15; tone.connect(gain).connect(source); tone.start();
      for (const delayMs of [0, 137, 300]) {
        const delay = context.createDelay(1); delay.delayTime.value = delayMs / 1000;
        source.connect(delay); markers.receiverNode(delay, 'direct-' + delayMs, 'calibration', 0);
        markers.nodes.push(delay);
      }
      // Silence must not produce a marker even when every active edge sees one.
      const silent = context.createConstantSource(); silent.offset.value = 0; silent.start();
      markers.receiverNode(silent, 'silent', 'calibration', 0); markers.nodes.push(silent);
      const destination = context.createMediaStreamDestination(); source.connect(destination);
      publisher.addTrack(destination.stream.getAudioTracks()[0]);
      const candidates = [[], []];
      publisher.onicecandidate = ({ candidate }) => { if (candidate) { if (receiver.remoteDescription) receiver.addIceCandidate(candidate); else candidates[0].push(candidate); } };
      receiver.onicecandidate = ({ candidate }) => { if (candidate) { if (publisher.remoteDescription) publisher.addIceCandidate(candidate); else candidates[1].push(candidate); } };
      receiver.ontrack = ({ track }) => {
        const playback = document.createElement('audio'); playback.autoplay = true; playback.muted = true;
        playback.srcObject = new MediaStream([track]); document.body.append(playback); playback.play();
        const input = context.createMediaStreamSource(new MediaStream([track]));
        markers.receiverNode(input, 'opus-0', 'calibration', 0);
        const delay = context.createDelay(1); delay.delayTime.value = .2; input.connect(delay);
        markers.receiverNode(delay, 'opus-200', 'calibration', 0); markers.nodes.push(input, delay);
      };
      await publisher.setLocalDescription(await publisher.createOffer()); await receiver.setRemoteDescription(publisher.localDescription);
      for (const candidate of candidates[0]) await receiver.addIceCandidate(candidate);
      await receiver.setLocalDescription(await receiver.createAnswer()); await publisher.setRemoteDescription(receiver.localDescription);
      for (const candidate of candidates[1]) await publisher.addIceCandidate(candidate);
      const sender = publisher.getSenders()[0], parameters = sender.getParameters(); parameters.encodings[0].maxBitrate = 128000; await sender.setParameters(parameters);
      await wait(() => publisher.connectionState === 'connected' && receiver.connectionState === 'connected');
      await timeout(2000); markers.begin(); await timeout(9000); markers.end();
      return { ...await markers.evidence(), receiver_stats: [...await receiver.getStats()].map(([, value]) => value) };
    } finally { publisher.close(); receiver.close(); await markers.close(); }
  });
  const calibration = result.calibration, bound = calibration.parameters.errorBoundMs;
  result.failures.push(...calibration.failures);
  result.delay_checks = [];
  for (const delay of [0, 137, 300]) {
    const edge = calibration.edges.find(edge => edge.peer === 'direct-' + delay);
    const matches = edge.matches.filter(match => !match.problem);
    const errors = matches.map(match => match.latency_ms - delay);
    const valid = matches.length >= 3 && matches.length === edge.expected_markers && errors.every(error => Math.abs(error) <= bound);
    result.delay_checks.push({ path: edge.peer, expected_ms: delay, matched_markers: matches.length, errors_ms: errors, valid });
    if (!valid) result.failures.push('known PCM delay calibration failed: ' + edge.peer);
  }
  const plain = calibration.edges.find(edge => edge.peer === 'opus-0'), delayed = calibration.edges.find(edge => edge.peer === 'opus-200');
  const differences = plain.matches.flatMap(match => {
    const other = delayed.matches.find(value => value.sequence === match.sequence);
    return match.problem || !other || other.problem ? [] : [other.latency_ms - match.latency_ms];
  });
  const valid = differences.length >= 3 && differences.length === plain.expected_markers && differences.every(value => Math.abs(value - 200) <= bound);
  result.delay_checks.push({ path: 'same decoded Opus track plus 200-ms delay', expected_ms: 200, differences_ms: differences, valid });
  if (!valid) result.failures.push('decoded Opus PCM differential calibration failed');
  if (calibration.edges.find(edge => edge.peer === 'silent').received.length) result.failures.push('silence false positive');
  if (calibration.clipped_frames) result.failures.push('source PCM clipping');
  if (Math.abs(calibration.wall_clock_seconds - calibration.sample_clock_seconds) > .1) result.failures.push('PCM sampleclock and wallclock differ');
  result.valid = result.failures.length === 0;
} catch (error) { result.failures.push(String(error)); result.valid = false; }
finally { if (browser) await browser.close(); proxy.closeAllConnections(); await new Promise(resolve => proxy.close(resolve)); }
fs.mkdirSync(path.dirname(path.resolve(output)), { recursive: true }); fs.writeFileSync(output, JSON.stringify(result, null, 2) + '\n');
console.log(JSON.stringify({ output, valid: result.valid, checks: result.delay_checks, failures: result.failures }));
if (!result.valid) process.exitCode = 1;
