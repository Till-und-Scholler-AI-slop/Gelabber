#!/usr/bin/env node
// Explicit local native-source → Chromium decoder control; no SFU comparison.
import fs from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';
import { executedChromium } from './browser-provenance.mjs';
import { NativeVideo, decodedVideo } from './native-video.mjs';

const options = {};
const allowed = new Set(['binary', 'archive', 'output', 'seconds', 'warmup', 'execute']);
for (let i = 2; i < process.argv.length; i++) {
  const key = process.argv[i];
  if (!key.startsWith('--')) throw new Error('expected named options');
  if (!allowed.has(key.slice(2))) throw new Error('unsupported option: ' + key);
  options[key.slice(2)] = key === '--execute' ? true : process.argv[++i];
}
const seconds = Number(options.seconds ?? 20), warmup = Number(options.warmup ?? 10);
if (!options.binary || !options.archive || !options.output || fs.existsSync(options.output)) throw new Error('required fresh --output FILE, --binary FILE, --archive FILE');
if (!Number.isInteger(seconds) || seconds < 8 || seconds > 120 || !Number.isInteger(warmup) || warmup < 0 || warmup > 60) throw new Error('measurement 8..120s and warmup 0..60s required');
const inspected = spawnSync(options.binary, ['--inspect', options.archive], { encoding: 'utf8', timeout: 30000 });
if (inspected.status !== 0) throw new Error('archive validation failed: ' + inspected.stderr);
const provenance = JSON.parse(inspected.stdout);
if ((seconds + warmup) % provenance.source.duration_seconds) throw new Error('warmup+measurement must span whole frozen source periods');
const folder = path.dirname(fileURLToPath(import.meta.url));
const sha = file => createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const git = args => spawnSync('git', ['-C', folder, ...args], { encoding: 'utf8' });
const head = git(['rev-parse', 'HEAD']), dirty = git(['status', '--porcelain']);
const evidence = { instrument: provenance, scope: 'one native video publisher to one Chromium decoder over loopback; no SFU',
  source_revision: head.status === 0 ? head.stdout.trim() : null, source_dirty: dirty.status === 0 ? Boolean(dirty.stdout.trim()) : null,
  collector: { node: process.version, executable: process.execPath, executable_sha256: sha(process.execPath),
    helper_sha256: Object.fromEntries(['local-fixed-video.mjs', 'native-video.mjs', 'browser-provenance.mjs'].map(file => [file, sha(path.join(folder, file))])) },
  run_plan: { execute: Boolean(options.execute), seconds, warmup, topology: { source_publishers: 1, receivers: 1, voice_peers: 0, dtls_transports: 1 }, video_source_bitrate_bps: provenance.source.rtp_payload_bitrate_bps },
  comparison_available: false, production_feature_acceptance: false, failures: [] };
let source, browser;
const save = () => { fs.mkdirSync(path.dirname(path.resolve(options.output)), { recursive: true }); fs.writeFileSync(options.output, JSON.stringify(evidence, null, 2) + '\n', { flag: 'wx' }); };
try {
  if (options.execute) {
    source = new NativeVideo(options.binary, options.archive);
    evidence.executed_source = (await source.ready).provenance;
    if (evidence.executed_source.archive_sha256 !== provenance.archive_sha256 || evidence.executed_source.binary_sha256 !== provenance.binary_sha256) throw new Error('source artifacts changed after inspection');
    browser = await chromium.launch({ headless: true, args: ['--enable-automation', '--autoplay-policy=no-user-gesture-required', '--disable-features=WebRtcHideLocalIpsWithMdns'] });
    evidence.executed_browser = await executedChromium(browser);
    const page = await browser.newPage();
    page.on('pageerror', error => evidence.failures.push(error.message));
    const offer = await source.call({ op: 'offer' });
    const answer = await page.evaluate(async description => {
      const pc = window.fixedSourcePc = new RTCPeerConnection({ iceServers: [] });
      pc.ontrack = event => {
        const video = document.createElement('video'); video.autoplay = true; video.muted = true;
        video.srcObject = new MediaStream([event.track]); document.body.append(video);
        video.play().catch(error => { window.fixedSourceError = String(error); });
      };
      await pc.setRemoteDescription(description); await pc.setLocalDescription(await pc.createAnswer());
      if (pc.iceGatheringState !== 'complete') await new Promise((resolve, reject) => {
        const timer = setTimeout(() => reject(new Error('receiver ICE gathering deadline')), 15000);
        pc.onicegatheringstatechange = () => { if (pc.iceGatheringState === 'complete') { clearTimeout(timer); resolve(); } };
      });
      return pc.localDescription.toJSON();
    }, offer.description);
    await source.call({ op: 'remote', description: answer });
    await source.call({ op: 'start', seconds: seconds + warmup });
    evidence.started_at = new Date().toISOString();
    await page.waitForTimeout(warmup * 1000);
    evidence.samples = await page.evaluate(async duration => {
      const samples = [], start = Date.now();
      while (Date.now() - start < duration * 1000) {
        if (window.fixedSourceError) throw new Error(window.fixedSourceError);
        samples.push({ at: Date.now(), stats: [...await window.fixedSourcePc.getStats()].map(([, value]) => value) });
        await new Promise(resolve => setTimeout(resolve, 1000));
      }
      return samples;
    }, seconds);
    evidence.native = await source.call({ op: 'status' });
    evidence.decoder = decodedVideo(evidence.samples, provenance.source.rtp_payload_bitrate_bps);
    evidence.failures.push(...evidence.decoder.failures);
    if (!evidence.native.source.completed || !evidence.native.source.source_policy_valid || evidence.native.source.error || evidence.native.connection !== 'connected') evidence.failures.push('native source did not complete its fixed schedule on a connected WebRTC transport');
    evidence.instrument_local_valid = evidence.failures.length === 0;
    evidence.finished_at = new Date().toISOString();
  }
} catch (error) { evidence.failures.push(String(error)); evidence.instrument_local_valid = false; }
finally {
  try { if (browser) await browser.close(); } catch (error) { evidence.failures.push('browser cleanup: ' + String(error)); }
  try { if (source) await source.close(); } catch (error) { evidence.failures.push('source cleanup: ' + String(error)); }
  if (evidence.failures.length) evidence.instrument_local_valid = false;
  save();
}
if (evidence.failures.length) process.exitCode = 1;
console.log(`Evidence: ${options.output}`);
