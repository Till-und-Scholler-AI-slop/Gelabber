// Local-only SFU acceptance: actual Chromium encoders/decoders, no API accounts.
// Build gelabber-media first; source the coordinator's pinned test environment.
import assert from 'node:assert/strict';
import { randomUUID, randomBytes } from 'node:crypto';
import { createServer } from 'node:http';
import { createConnection } from 'node:net';
import { spawn } from 'node:child_process';
import { openSync, readFileSync } from 'node:fs';
import { stripTypeScriptTypes } from 'node:module';
import { networkInterfaces } from 'node:os';
import { chromium, firefox } from '../../web/node_modules/playwright/index.mjs';

const redis = new URL(process.env.REDIS_URL ?? 'redis://127.0.0.1:56379');
assert.equal(redis.hostname, '127.0.0.1', 'only the isolated local test Redis is supported');
const wsUrl = process.env.MEDIA_WS_URL ?? 'ws://127.0.0.1:18081/media/ws';
assert(wsUrl.startsWith('ws://127.0.0.1:'), 'local-only media test');
const iceBind = process.env.MEDIA_TEST_LAN === '1'
  ? `${Object.values(networkInterfaces()).flat().find(address => !address.internal && address.family === 'IPv4')?.address}:0`
  : '127.0.0.1:0';
assert(!iceBind.startsWith('undefined'), 'local IPv4 interface required');
const media = process.env.MEDIA_WS_URL ? null : spawn(`${process.env.CARGO_TARGET_DIR ?? new URL('../../target', import.meta.url).pathname}/debug/gelabber-media`, [], {
  env: { ...process.env, MEDIA_ADDR: '127.0.0.1:18081', MEDIA_ICE_BIND: iceBind, MEDIA_ADVERTISED_IP: '' },
  stdio: ['ignore', openSync('/tmp/gelabber-media-browser-sfu.log', 'w'), openSync('/tmp/gelabber-media-browser-sfu-error.log', 'w')],
});
const server = createServer((_req, res) => {
  res.setHeader('Content-Type', 'text/html; charset=utf-8');
  res.end('<button id="start">Start</button>');
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const origin = `http://127.0.0.1:${server.address().port}`;
const mediaSource = readFileSync(new URL('../../web/src/voice/media.ts', import.meta.url), 'utf8');
const identitySource = stripTypeScriptTypes(mediaSource.slice(mediaSource.indexOf('export function publishedTrackIds'), mediaSource.indexOf('export type MediaServerFrame')).replace('export function', 'function'));
const browser = await (process.env.MEDIA_BROWSER === 'firefox' ? firefox : chromium).launch({ headless: true });
let vite = null;
if (process.env.MEDIA_CASE === 'recovery') {
  const { createServer: createVite } = await import('../../web/node_modules/vite/dist/node/index.js');
  vite = await createVite({ root: new URL('../../web', import.meta.url).pathname, server: { host: '127.0.0.1', port: 15173, strictPort: true, cors: true, hmr: false } });
  await vite.listen();
}
const alphabet = 'abcdefghjkmnpqrstuvwxyz23456789';
const authorities = new Map(), members = new Map(), channels = new Map();
const fixtureKeys = new Set(), writes = new Set();
async function redisCommand(args) {
  const payload = `*${args.length}\r\n` + args.map(arg => `$${Buffer.byteLength(String(arg))}\r\n${arg}\r\n`).join('');
  return new Promise((resolve, reject) => {
    const socket = createConnection({ host: redis.hostname, port: Number(redis.port) }, () => socket.write(payload));
    socket.on('error', reject);
    socket.once('data', data => { socket.end(); data.toString().startsWith('-') ? reject(new Error('local fixture Redis command failed')) : resolve(data.toString()); });
  });
}
async function renewLease(authority) {
  await redisCommand(['SET', `gb:auth:session:${authority.session}`, authority.user, 'PX', '3000']);
}
async function mint(user, channel, owner) {
  const id = `${owner}:${user}:${channel}`;
  let authority = authorities.get(id);
  if (!authority) {
    const memberKey = `gb:auth:member:${owner}:${user}`, channelKey = `gb:auth:channel:${channel}`;
    if (!members.has(memberKey)) members.set(memberKey, randomUUID());
    if (!channels.has(channelKey)) channels.set(channelKey, randomUUID());
    authority = { user, owner, channel, session: randomBytes(32).toString('hex'), expires_at: Math.floor(Date.now() / 1000) + 300, member: members.get(memberKey), channelNonce: channels.get(channelKey) };
    await redisCommand(['SET', memberKey, authority.member]);
    await redisCommand(['SET', channelKey, authority.channelNonce]);
    for (const key of [memberKey, channelKey, `gb:auth:session:${authority.session}`, `gb:auth:demand:${authority.session}`]) fixtureKeys.add(key);
    await renewLease(authority);
    // This is the test API lease writer. Production media only refreshes demand.
    authority.timer = setInterval(() => {
      const write = renewLease(authority); writes.add(write);
      write.catch(() => {}).finally(() => writes.delete(write));
    }, 500);
    authorities.set(id, authority);
  }
  const ticket = Array.from(randomBytes(12), n => alphabet[n % alphabet.length]).join('');
  const key = `gb:mt:${ticket}`; fixtureKeys.add(key);
  await redisCommand(['SET', key, JSON.stringify({ u: user, s: owner, c: channel, g: true, auth: { session: authority.session, expires_at: authority.expires_at, member: authority.member, channel: authority.channelNonce } }), 'EX', '60']);
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
    let serverClosed = false;
    ws.onclose = () => { serverClosed = true; };
    let chain = Promise.resolve();
    const send = frame => ws.send(JSON.stringify(frame));
    const enqueue = job => chain = chain.then(job).catch(error => errors.push(error.message));
    const ice = [];
    let joined;
    const ready = new Promise(resolve => joined = resolve);
    pc.onicecandidate = event => { if (event.candidate?.candidate) send({ op: 'i', ice: event.candidate.candidate, mid: event.candidate.sdpMid }); };
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
        return { videos, audioBytes, outgoing, serverClosed, sdp: pc.remoteDescription?.sdp.split('\r\n').filter(l => /^(m=video|a=mid:|a=rtpmap:|a=rtcp-fb:)/.test(l)), transceivers: pc.getTransceivers().length, streams: [...received.keys()], errors: [...errors] };
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

async function productionReceiver(user, channel, owner) {
  const page = await browser.newPage();
  await page.goto(origin);
  await page.exposeFunction('freshTicket', () => mint(user, channel, owner));
  await page.evaluate(async ({ wsUrl, channel, owner, user }) => {
    const session = await import('http://127.0.0.1:15173/src/voice/session.ts');
    const media = await import('http://127.0.0.1:15173/src/voice/media.ts');
    const peers = [];
    let socket, capture, displayCalls = 0, micCalls = 0, emptyCandidates = 0;
    const videos = new Map();
    session.useVoice.subscribe(state => {
      for (const source of Object.values(state.remote)) for (const stream of Object.values(source)) {
        if (!stream || videos.has(stream.id)) continue;
        const element = document.createElement('video');
        element.muted = true; element.autoplay = true; element.srcObject = stream;
        document.body.append(element); videos.set(stream.id, element);
        void element.play();
      }
    });
    session.configureVoice({
      userId: () => user,
      gateway: { send() {}, onSig: () => () => {}, onErr: () => () => {}, onReady: () => () => {} },
      createPeer: iceServers => {
        const pc = new RTCPeerConnection({ iceServers });
        const apply = pc.setRemoteDescription.bind(pc);
        pc.setRemoteDescription = async description => {
          try { await apply(description); }
          catch (error) { window.recoveryErrors.push(`${error.name}: ${error.message}`); throw error; }
        };
        peers.push(pc); return pc;
      },
      getUserMedia: async () => { micCalls++; throw new DOMException('test microphone permission denied', 'NotAllowedError'); },
      getDisplayMedia: async () => {
        displayCalls++;
        const canvas = document.createElement('canvas'); canvas.width = 160; canvas.height = 100;
        const context = canvas.getContext('2d'); let frame = 0;
        setInterval(() => { context.fillStyle = `rgb(${frame++ % 255},140,30)`; context.fillRect(0,0,160,100); }, 50);
        capture = canvas.captureStream(15);
        return capture;
      },
      fetchTicket: async () => {
        if (Date.now() < (window.offlineUntil ?? 0)) throw new Error('local API outage');
        return { ticket: await window.freshTicket(), media_path: '/media/ws', ice_servers: [] };
      },
      openMedia: () => {
        socket = media.openMediaSocket(wsUrl);
        const send = socket.send.bind(socket);
        socket.send = frame => { if (frame.op === 'i' && !frame.ice) emptyCandidates++; send(frame); };
        return socket;
      },
      onError: error => { window.recoveryErrors.push(error.message); },
    });
    window.recoveryErrors = [];
    window.call = {
      join: () => session.joinVoice({ serverId: owner, channelId: channel, channelName: 'Local regression' }),
      share: () => session.toggleShare(),
      breakTransport: () => { window.offlineUntil = Date.now() + 8000; socket.close(); },
      leave: () => session.leaveVoice(),
      async stats() {
        const pc = peers.at(-1); let report = new Map();
        try { if (pc && pc.signalingState !== 'closed') report = await pc.getStats(); }
        catch (error) { if (error.name !== 'InvalidStateError' && pc?.connectionState !== 'closed') throw error; }
        const videos = []; let audioBytes = 0;
        for (const entry of report.values()) {
          if (entry.type === 'inbound-rtp' && entry.kind === 'video') videos.push({ frames: entry.framesDecoded, keyframes: entry.keyFramesDecoded, bytes: entry.bytesReceived, mid: entry.mid });
          if (entry.type === 'inbound-rtp' && entry.kind === 'audio') audioBytes += entry.bytesReceived;
        }
        return { videos, audioBytes, peers: peers.length, connected: pc?.connectionState === 'connected', displayCalls, micCalls, emptyCandidates, sdp: pc?.signalingState === 'closed' ? [] : pc?.remoteDescription?.sdp.split('\r\n').filter(l => /^(m=video|a=mid:|a=rtpmap:)/.test(l)), captureId: capture?.getVideoTracks()[0].id, captureState: capture?.getVideoTracks()[0].readyState, errors: window.recoveryErrors, localScreenId: session.useVoice.getState().localScreen?.getVideoTracks()[0].id };
      },
    };
    document.querySelector('#start').onclick = () => window.call.join();
  }, { wsUrl, channel, owner, user });
  await page.click('#start');
  return page;
}

try {
  for (let i = 0; i < 100; i++) {
    try { if ((await fetch(wsUrl.replace('ws:', 'http:').replace('/media/ws', '/media/ready'))).ok) break; } catch {}
    if (i === 99) throw new Error('isolated media did not start');
    await new Promise(resolve => setTimeout(resolve, 50));
  }
  const channel = randomUUID(), owner = randomUUID(), publisher = randomUUID();
  const a = await peer(publisher, channel, owner, true);
  if (process.env.MEDIA_CASE === 'revoke') {
    await a.waitForFunction(() => call.pc.signalingState === 'stable');
    await a.evaluate(() => call.start('l'));
    const b = await peer(randomUUID(), channel, owner);
    const before = await decoded(b, 1);
    const otherChannel = randomUUID(), otherPublisher = randomUUID(), otherOwner = randomUUID();
    const control = await peer(otherPublisher, otherChannel, otherOwner, true);
    await control.waitForFunction(() => call.pc.signalingState === 'stable');
    await control.evaluate(() => call.start('v'));
    const controlReceiver = await peer(randomUUID(), otherChannel, otherOwner);
    await decoded(controlReceiver, 1);
    const authority = authorities.get(`${owner}:${publisher}:${channel}`);
    clearInterval(authority.timer);
    await Promise.allSettled([...writes]);
    const started = Date.now();
    await redisCommand(['DEL', `gb:auth:session:${authority.session}`]);
    await a.waitForFunction(() => call.errors.includes('unauthorized'), null, { timeout: 2000 });
    await pollStats(a, stats => stats.serverClosed, 2000);
    const revokeMs = Date.now() - started;
    assert(revokeMs < 2000);
    await new Promise(resolve => setTimeout(resolve, 200));
    const stopped = await b.evaluate(() => call.stats());
    await new Promise(resolve => setTimeout(resolve, 500));
    const after = await b.evaluate(() => call.stats());
    assert(after.videos.every(video => video.bytes === stopped.videos.find(old => old.mid === video.mid)?.bytes), 'revoked publisher still forwarded RTP');
    const unaffected = await decoded(controlReceiver, 1);
    const late = await peer(randomUUID(), channel, owner);
    await late.waitForFunction(() => call.pc.connectionState === 'connected');
    await new Promise(resolve => setTimeout(resolve, 500));
    assert.equal((await late.evaluate(() => call.stats())).videos.length, 0, 'late peer saw revoked publication');
    console.log(JSON.stringify({ browser: browser.version(), case: 'authorization-revocation', revokeMs, before, stopped, after, unaffected }, null, 2));
  } else if (process.env.MEDIA_CASE === 'recovery') {
    await a.waitForFunction(() => call.pc.signalingState === 'stable');
    await a.evaluate(() => call.start('v'));
    const b = await productionReceiver(randomUUID(), channel, owner);
    let first;
    try { first = await decoded(b, 1); } catch (error) {
      console.log(JSON.stringify({ phase: 'recovery-setup', publisher: await a.evaluate(() => call.stats()), subscriber: await b.evaluate(() => call.stats()) }));
      throw error;
    }
    assert(first.audioBytes > 0, 'mic-denied receiver must receive voice');
    assert.equal(first.micCalls, 1);
    await b.evaluate(() => { document.querySelector('#start').onclick = () => call.share(); });
    await b.click('#start');
    await decoded(a, 1);
    const before = await b.evaluate(() => call.stats());
    const started = Date.now();
    await b.evaluate(() => call.breakTransport());
    await pollStats(b, stats => stats.peers >= 2 && stats.connected, 20000);
    await decoded(a, 1);
    const after = await decoded(b, 1);
    const recoveryMs = Date.now() - started;
    assert(recoveryMs < 20000);
    assert.equal(after.captureId, before.captureId);
    assert.equal(after.localScreenId, before.captureId);
    assert.equal(after.captureState, 'live');
    assert.equal(after.displayCalls, 1);
    assert.equal(after.micCalls, 1);
    assert.deepEqual(after.errors, []);
    await b.evaluate(() => call.leave());
    assert.equal((await b.evaluate(() => call.stats())).captureState, 'ended');
    console.log(JSON.stringify({ browser: browser.version(), case: 'production-session-recovery', recoveryMs, first, before, after }, null, 2));
  } else {
  const b = await peer(randomUUID(), channel, owner);
  // Deliberately publish in a different order than v/s/l. Each source must remain tagged.
  // Wait for the initial answer before the deliberately rejected offer.
  await a.waitForFunction(() => call.pc.signalingState === 'stable');
  const reuse = process.env.MEDIA_CASE === 'renegotiate' ? null : await a.evaluate(() => call.failedPublishAndReuse());
  if (reuse) {
    assert(reuse.sameSender && reuse.differentCaptureId && reuse.retainedMsid, 'Chromium reuse regression was not exercised');
    await decoded(b, 1);
  }
  const videoCount = process.env.MEDIA_ASSERT_PT_MAPPING === '1' ? 1 : 3;
  for (const k of (videoCount === 1 ? ['l'] : reuse ? ['l', 'v'] : ['l', 's', 'v'])) {
    try { await a.evaluate(k => call.start(k), k); } catch (error) {
      console.log(JSON.stringify({ phase: `publish-${k}`, publisher: await a.evaluate(() => call.stats()), subscriber: await b.evaluate(() => call.stats()) }));
      throw error;
    }
  }
  let first;
  try { first = await decoded(b, videoCount); } catch (error) {
    console.log(JSON.stringify({ publisher: await a.evaluate(() => call.stats()), subscriber: await b.evaluate(() => call.stats()) }));
    throw error;
  }
  if (process.env.MEDIA_CASE !== 'renegotiate') for (const k of ['v', 's', 'l']) assert(first.streams.includes(`${publisher}:${k}`), `missing ${k} identity`);
  assert(first.audioBytes > 0, 'voice audio must continue with video');
  console.log(JSON.stringify({ phase: 'before-renegotiation', first, publisher: await a.evaluate(() => call.stats()) }));
  for (let i = 0; i < Number(process.env.MEDIA_RESTARTS ?? 5); i++) {
    try {
      await b.evaluate(restart => call.offer(restart), process.env.MEDIA_ICE_RESTART !== '0');
      if (process.env.MEDIA_ASSERT_PT_MAPPING === '1') await decoded(b, videoCount);
    } catch (error) {
      console.log(JSON.stringify({ phase: `reoffer-${i}`, publisher: await a.evaluate(() => call.stats()), subscriber: await b.evaluate(() => call.stats()) }));
      throw error;
    }
  }
  let restarted;
  try { restarted = await decoded(b, videoCount); } catch (error) {
    console.log(JSON.stringify({ phase: 'after-renegotiation', publisher: await a.evaluate(() => call.stats()), subscriber: await b.evaluate(() => call.stats()) }));
    throw error;
  }
  if (process.env.MEDIA_ASSERT_PT_MAPPING === '1') {
    const source = (await a.evaluate(() => call.stats())).outgoing.filter(track => track.kind === 'video');
    assert.equal(source.length, videoCount);
    assert(source.every(track => track.codec === 'video/AV1' && track.pt === 99));
    assert(first.videos.every(track => track.codec === 'video/AV1' && track.pt === 41));
    assert(restarted.videos.every(track => track.codec === 'video/AV1' && track.pt === 99));
    assert.equal(restarted.transceivers, videoCount + 1);
    assert.deepEqual(restarted.errors, []);
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
  }
} finally {
  for (const authority of authorities.values()) clearInterval(authority.timer);
  await Promise.allSettled([...writes]);
  await browser.close();
  await vite?.close();
  await new Promise(resolve => server.close(resolve));
  if (media) { media.kill('SIGTERM'); await new Promise(resolve => media.once('exit', resolve)); }
  if (fixtureKeys.size) await redisCommand(['DEL', ...fixtureKeys]);
}
