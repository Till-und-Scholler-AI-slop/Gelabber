// Local-only SFU acceptance: actual Chromium encoders/decoders, no API accounts.
// Build gelabber-media first; source the coordinator's pinned test environment.
import assert from 'node:assert/strict';
import { randomUUID, randomBytes } from 'node:crypto';
import { createServer } from 'node:http';
import { createConnection } from 'node:net';
import { spawn } from 'node:child_process';
import { openSync, readFileSync } from 'node:fs';
import { stripTypeScriptTypes } from 'node:module';
import { chromium } from '../../web/node_modules/playwright/index.mjs';

const redis = new URL(process.env.REDIS_URL ?? 'redis://127.0.0.1:56379');
assert.equal(redis.hostname, '127.0.0.1', 'only the isolated local test Redis is supported');
const wsUrl = process.env.MEDIA_WS_URL ?? 'ws://127.0.0.1:18081/media/ws';
assert(wsUrl.startsWith('ws://127.0.0.1:'), 'local-only media test');
const media = process.env.MEDIA_WS_URL ? null : spawn(`${process.env.CARGO_TARGET_DIR ?? new URL('../../target', import.meta.url).pathname}/debug/gelabber-media`, [], {
  env: { ...process.env, MEDIA_ADDR: '127.0.0.1:18081', MEDIA_ICE_BIND: '127.0.0.1:0', MEDIA_ADVERTISED_IP: '' },
  stdio: ['ignore', openSync('/tmp/gelabber-media-browser-sfu.log', 'w'), openSync('/tmp/gelabber-media-browser-sfu-error.log', 'w')],
});
const server = createServer((_req, res) => res.end('<button id="start">Start</button>'));
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const origin = `http://127.0.0.1:${server.address().port}`;
const mediaSource = readFileSync(new URL('../../web/src/voice/media.ts', import.meta.url), 'utf8');
const identitySource = stripTypeScriptTypes(mediaSource.slice(mediaSource.indexOf('export function publishedTrackIds'), mediaSource.indexOf('export type MediaServerFrame')).replace('export function', 'function'));
const browser = await chromium.launch({ headless: true });
const alphabet = 'abcdefghjkmnpqrstuvwxyz23456789';
async function mint(user, channel, owner) {
  const ticket = Array.from(randomBytes(12), n => alphabet[n % alphabet.length]).join('');
  const args = ['SET', `gb:mt:${ticket}`, JSON.stringify({ u: user, s: owner, c: channel, g: true }), 'EX', '60'];
  const payload = `*${args.length}\r\n` + args.map(arg => `$${Buffer.byteLength(arg)}\r\n${arg}\r\n`).join('');
  await new Promise((resolve, reject) => {
    const socket = createConnection({ host: redis.hostname, port: Number(redis.port) }, () => socket.write(payload));
    socket.on('error', reject);
    socket.once('data', data => { socket.end(); data.toString().startsWith('+OK') ? resolve() : reject(new Error('ticket mint failed')); });
  });
  return ticket;
}
async function peer(user, channel, owner, microphone = false) {
  const page = await browser.newPage();
  await page.goto(origin);
  await page.addScriptTag({ content: identitySource });
  // AudioContext is resumed by a real user gesture, without autoplay overrides.
  await page.evaluate(() => { document.querySelector('#start').onclick = () => { window.audio = new AudioContext(); void audio.resume(); }; });
  await page.click('#start');
  const ticket = await mint(user, channel, owner);
  await page.evaluate(async ({ ticket, wsUrl, microphone }) => {
    const pc = new RTCPeerConnection();
    const ws = new WebSocket(wsUrl);
    const received = new Map();
    const captures = new Map();
    const errors = [];
    let chain = Promise.resolve();
    const send = frame => ws.send(JSON.stringify(frame));
    const enqueue = job => chain = chain.then(job).catch(error => errors.push(error.message));
    const ice = [];
    let joined;
    const ready = new Promise(resolve => joined = resolve);
    pc.onicecandidate = event => { if (event.candidate) send({ op: 'i', ice: event.candidate.candidate, mid: event.candidate.sdpMid }); };
    pc.ontrack = event => {
      const stream = event.streams[0];
      received.set(stream.id, event.track);
      if (event.track.kind === 'video') {
        const video = document.createElement('video');
        video.muted = true;
        video.autoplay = true;
        video.srcObject = stream;
        document.body.append(video);
        void video.play().catch(error => errors.push(error.message));
      }
    };
    ws.onopen = () => send({ op: 'j', tk: ticket });
    ws.onmessage = event => enqueue(async () => {
      const frame = JSON.parse(event.data);
      if (frame.op === 'ok') joined();
      else if (frame.op === 'a') await pc.setRemoteDescription({ type: 'answer', sdp: frame.sdp });
      else if (frame.op === 'o') {
        if (pc.signalingState === 'have-local-offer') await pc.setLocalDescription({ type: 'rollback' });
        await pc.setRemoteDescription({ type: 'offer', sdp: frame.sdp });
        for (const candidate of ice.splice(0)) await pc.addIceCandidate(candidate);
        const answer = await pc.createAnswer();
        await pc.setLocalDescription(answer);
        send({ op: 'a', sdp: answer.sdp });
      } else if (frame.op === 'i') {
        const candidate = { candidate: frame.ice, sdpMid: frame.mid ?? null, sdpMLineIndex: frame.mid ? undefined : 0 };
        if (pc.remoteDescription) await pc.addIceCandidate(candidate); else ice.push(candidate);
      } else if (frame.op === 'err') errors.push(frame.e);
    });
    await ready;
    if (microphone) {
      const oscillator = audio.createOscillator();
      const destination = audio.createMediaStreamDestination();
      oscillator.connect(destination);
      oscillator.start();
      pc.addTrack(destination.stream.getAudioTracks()[0], destination.stream);
    } else pc.addTransceiver('audio', { direction: 'recvonly' });
    const offer = async (restart = false) => {
      // Answers are asynchronous. Wait for stable before starting a new browser offer.
      await new Promise((resolve, reject) => {
        const until = performance.now() + 5000;
        const poll = () => pc.signalingState === 'stable' ? resolve() : performance.now() > until ? reject(new Error('signaling stayed unstable')) : setTimeout(poll, 10);
        poll();
      });
      await enqueue(async () => {
        const codecs = RTCRtpSender.getCapabilities('video').codecs;
        const preferred = [...codecs.filter(c => c.mimeType.toLowerCase() === 'video/vp8'), ...codecs.filter(c => c.mimeType.toLowerCase() !== 'video/vp8')];
        for (const t of pc.getTransceivers()) if ((t.sender.track ?? t.receiver.track)?.kind === 'video') t.setCodecPreferences(preferred);
        const description = await pc.createOffer(restart ? { iceRestart: true } : undefined);
        await pc.setLocalDescription(description);
        const bindings = publishedTrackIds(pc.localDescription.sdp);
        for (const [k, source] of captures) {
          const mid = pc.getTransceivers().find(t => t.sender === source.sender).mid;
          source.identity = bindings.get(mid);
          if (!source.identity) throw new Error('missing MSID binding');
          send({ op: 'p', k, t: source.identity });
        }
        send({ op: 'o', sdp: pc.localDescription.sdp });
      });
    };
    window.call = {
      pc, ws, errors, received,
      async start(k) {
        const canvas = document.createElement('canvas');
        canvas.width = 160; canvas.height = 100;
        const context = canvas.getContext('2d');
        let frame = 0;
        const draw = () => { context.fillStyle = `rgb(${frame++ % 255},${k === 's' ? 180 : 30},${k === 'l' ? 180 : 30})`; context.fillRect(0, 0, 160, 100); context.fillStyle = 'white'; context.fillText(`${k}:${frame}`, 10, 20); };
        draw();
        const timer = setInterval(draw, 50);
        const stream = canvas.captureStream(15);
        const track = stream.getVideoTracks()[0];
        const sender = pc.addTrack(track, stream);
        captures.set(k, { track, sender, timer });
        await offer();
      },
      async stop(k) {
        const source = captures.get(k);
        send({ op: 'u', k, t: source.identity });
        pc.removeTrack(source.sender);
        source.track.stop(); clearInterval(source.timer); captures.delete(k);
        await offer();
      },
      async failedPublishAndReuse() {
        const canvas = document.createElement('canvas');
        canvas.width = 160; canvas.height = 100;
        const rejected = canvas.captureStream(15);
        const first = rejected.getVideoTracks()[0];
        const sender = pc.addTrack(first, rejected);
        const description = await pc.createOffer();
        await pc.setLocalDescription(description);
        const oldMid = pc.getTransceivers().find(t => t.sender === sender).mid;
        const oldId = publishedTrackIds(pc.localDescription.sdp).get(oldMid);
        send({ op: 'p', k: 's', t: oldId });
        await pc.setLocalDescription({ type: 'rollback' });
        send({ op: 'u', k: 's', t: oldId });
        pc.removeTrack(sender); first.stop();
        await this.start('s');
        const next = captures.get('s');
        return { sameSender: next.sender === sender, differentCaptureId: next.track.id !== first.id, retainedMsid: next.identity === oldId };
      },
      offer,
      async stats() {
        const stats = await pc.getStats();
        const videos = [];
        let audioBytes = 0;
        const outgoing = [];
        for (const entry of stats.values()) {
          if (entry.type === 'inbound-rtp' && entry.kind === 'video') {
            videos.push({ frames: entry.framesDecoded ?? 0, keyframes: entry.keyFramesDecoded ?? 0, bytes: entry.bytesReceived, mid: entry.mid, pli: entry.pliCount, codec: stats.get(entry.codecId)?.mimeType, pt: stats.get(entry.codecId)?.payloadType });
          }
          if (entry.type === 'inbound-rtp' && entry.kind === 'audio') audioBytes += entry.bytesReceived;
          if (entry.type === 'outbound-rtp') outgoing.push({kind: entry.kind, frames: entry.framesEncoded, keyframes: entry.keyFramesEncoded, plis: entry.pliCount, bytes: entry.bytesSent, codec: stats.get(entry.codecId)?.mimeType, pt: stats.get(entry.codecId)?.payloadType});
        }
        return { videos, audioBytes, outgoing, sdp: pc.remoteDescription?.sdp.split('\r\n').filter(l => /^(m=video|a=mid:|a=rtpmap:|a=rtcp-fb:)/.test(l)), transceivers: pc.getTransceivers().length, streams: [...received.keys()], errors: [...errors] };
      },
    };
    await offer();
  }, { ticket, wsUrl, microphone });
  return page;
}
async function pollStats(page, predicate, budget) {
  const deadline = Date.now() + budget;
  let last;
  do {
    last = await page.evaluate(() => call.stats());
    if (predicate(last)) return last;
    await new Promise(resolve => setTimeout(resolve, 50));
  } while (Date.now() < deadline);
  throw new Error(`decoder budget exceeded: ${JSON.stringify(last)}`);
}
async function decoded(page, count, budget = 10000) {
  const before = await pollStats(page, stats => stats.videos.filter(v => v.frames > 3).length >= count, budget);
  return pollStats(page, stats => stats.videos.filter(v => v.frames > (before.videos.find(old => old.mid === v.mid)?.frames ?? 0)).length >= count, budget);
}

try {
  for (let i = 0; i < 100; i++) {
    try { if ((await fetch(wsUrl.replace('ws:', 'http:').replace('/media/ws', '/media/ready'))).ok) break; } catch {}
    if (i === 99) throw new Error('isolated media did not start');
    await new Promise(resolve => setTimeout(resolve, 50));
  }
  const channel = randomUUID(), owner = randomUUID(), publisher = randomUUID();
  const a = await peer(publisher, channel, owner, true);
  const b = await peer(randomUUID(), channel, owner);
  // Deliberately publish in a different order than v/s/l. Each source must remain tagged.
  // Wait for the initial answer before the deliberately rejected offer.
  await a.waitForFunction(() => call.pc.signalingState === 'stable');
  const reuse = process.env.MEDIA_CASE === 'renegotiate' ? null : await a.evaluate(() => call.failedPublishAndReuse());
  if (reuse) {
    assert(reuse.sameSender && reuse.differentCaptureId && reuse.retainedMsid, 'Chromium reuse regression was not exercised');
    await decoded(b, 1);
  }
  for (const k of (reuse ? ['l', 'v'] : ['l', 's', 'v'])) await a.evaluate(k => call.start(k), k);
  let first;
  try { first = await decoded(b, 3); } catch (error) {
    console.log(JSON.stringify({ publisher: await a.evaluate(() => call.stats()), subscriber: await b.evaluate(() => call.stats()) }));
    throw error;
  }
  if (process.env.MEDIA_CASE !== 'renegotiate') for (const k of ['v', 's', 'l']) assert(first.streams.includes(`${publisher}:${k}`), `missing ${k} identity`);
  assert(first.audioBytes > 0, 'voice audio must continue with video');
  console.log(JSON.stringify({ phase: 'before-renegotiation', first, publisher: await a.evaluate(() => call.stats()) }));
  for (let i = 0; i < Number(process.env.MEDIA_RESTARTS ?? 5); i++) await b.evaluate(restart => call.offer(restart), process.env.MEDIA_ICE_RESTART !== '0');
  let restarted;
  try { restarted = await decoded(b, 3); } catch (error) {
    console.log(JSON.stringify({ phase: 'after-renegotiation', publisher: await a.evaluate(() => call.stats()), subscriber: await b.evaluate(() => call.stats()) }));
    throw error;
  }
  if (process.env.MEDIA_CASE === 'renegotiate') {
    console.log(JSON.stringify({ phase: 'after-renegotiation', restarted }));
    process.exitCode = 0;
  } else {
  assert.equal(restarted.transceivers, first.transceivers, 'repeated offers/ICE restarts added receivers');
  // Preserve a live source long enough that late join requires a fresh keyframe.
  await new Promise(resolve => setTimeout(resolve, Number(process.env.MEDIA_LATE_JOIN_MS ?? 30000)));
  const start = Date.now();
  const c = await peer(randomUUID(), channel, owner);
  const late = await decoded(c, 3);
  const lateMs = Date.now() - start;
  assert(lateMs < 10000, 'late join exceeded local budget');
  for (let i = 0; i < 20; i++) {
    await a.evaluate(() => call.stop('s'));
    await a.evaluate(() => call.start('s'));
    const cycle = await decoded(b, 3);
    assert.equal(cycle.transceivers, first.transceivers, `cycle ${i + 1} grew transceivers`);
  }
  const after = await b.evaluate(() => call.stats());
  assert.equal(after.transceivers, first.transceivers, '20 cycles grew transceivers');
  await b.evaluate(() => call.ws.send(' '.repeat(1023) + '€' + 'x'.repeat(256 * 1024)));
  await b.waitForFunction(() => call.errors.includes('negotiation_failed'));
  const unicode = await decoded(c, 3);
  await a.evaluate(() => call.stop('l'));
  const d = await peer(randomUUID(), channel, owner);
  const stopped = await decoded(d, 2);
  assert(!stopped.streams.includes(`${publisher}:l`), 'late join saw stopped live publication');
  for (const page of [a,b,c,d]) {
    const errors = await page.evaluate(() => call.errors);
    assert.deepEqual(errors.filter(e => e !== 'negotiation_failed'), []);
  }
  console.log(JSON.stringify({ browser: browser.version(), reuse, first, restarted, late, lateMs, after, unicode, stopped }, null, 2));
  }
} finally {
  await browser.close();
  await new Promise(resolve => server.close(resolve));
  if (media) { media.kill('SIGTERM'); await new Promise(resolve => media.once('exit', resolve)); }
}
