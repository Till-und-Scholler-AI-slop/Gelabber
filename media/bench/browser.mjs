// Real WebRTC endpoints on the load-generator host. Synthetic inputs are shared
// across engines; this does not substitute for product permission/browser tests.
import { Device } from 'mediasoup-client';
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
let videoBitrate = 6000000;
async function until(predicate, ms = 30000) {
  const end = Date.now() + ms;
  while (Date.now() < end) { if (predicate()) return; await sleep(50); }
  throw new Error('WebRTC deadline exceeded');
}
const rpc = async body => {
  const response = await fetch('/rpc', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) });
  const value = await response.json();
  if (!response.ok || value.error) throw new Error(JSON.stringify(value));
  return value;
};
const pcs = [], retained = [], failures = []; window.benchmarkFailures = failures;
const peerEvidence = [], timers = [];
function beginPeer(index, peers, video) {
  const evidence = { peer: `peer-${index}`, setup_started_at: Date.now(), connections: {},
    expected_audio: peers - 1 + (video && index !== 0 ? 1 : 0), expected_video: video && index !== 0 ? 1 : 0 };
  peerEvidence.push(evidence); return evidence;
}
function connectionTimer(connection, label, evidence) {
  const update = () => { if (connection.connectionState === 'connected') evidence.connections[label] ??= Date.now(); };
  if (connection.addEventListener) connection.addEventListener('connectionstatechange', update);
  else connection.on('connectionstatechange', update);
  update();
}
function observePeer(evidence, collect, connections) {
  timers.push((async () => {
    while (!evidence.stop) {
      try {
        const stats = await collect();
        if (Object.keys(evidence.connections).length === connections) evidence.dtls_ready_at ??= Date.now();
        if (stats.some(s => s.type === 'outbound-rtp' && s.packetsSent > 0)) evidence.first_send_rtp_at ??= Date.now();
        const incoming = stats.filter(s => s.type === 'inbound-rtp' && s.packetsReceived > 0 && s.mid !== 'probator');
        if (incoming.filter(s => s.kind === 'audio').length === evidence.expected_audio && incoming.filter(s => s.kind === 'video').length === evidence.expected_video) evidence.full_graph_rtp_ready_at ??= Date.now();
      } catch {} // A transport may not have created its PC yet during setup.
      await sleep(100);
    }
  })());
}
async function collectNative(connections, peer) {
  return (await Promise.all(connections.map(async (pc, index) => [...await pc.getStats()].map(([, value]) => ({ ...value, _endpoint: `${peer}/${index}`, _peer: peer }))))).flat();
}
function retainTrack(track) {
  retained.push(track);
  const element = document.createElement(track.kind === 'video' ? 'video' : 'audio');
  element.autoplay = true; element.muted = true; element.srcObject = new MediaStream([track]);
  document.body.append(element); element.play().catch(error => failures.push(String(error)));
}
function nativePc() {
  const pc = new RTCPeerConnection({ iceServers: [] });
  pcs.push(pc); pc.ontrack = event => retainTrack(event.track); return pc;
}
async function offer(pc) {
  const video = pc.getTransceivers().filter(t => t.sender.track?.kind === 'video');
  const codecs = RTCRtpSender.getCapabilities('video').codecs.filter(c => c.mimeType.toLowerCase() === 'video/vp8');
  for (const transceiver of video) transceiver.setCodecPreferences(codecs);
  const description = await pc.createOffer();
  await pc.setLocalDescription(description);
  await until(() => pc.iceGatheringState === 'complete');
  for (const sender of pc.getSenders()) {
    if (!sender.track) continue;
    const params = sender.getParameters();
    if (params.encodings.length) {
      params.encodings[0].maxBitrate = sender.track.kind === 'video' ? videoBitrate : 128000;
      if (sender.track.kind === 'video') { params.encodings[0].maxFramerate = 60; params.encodings[0].scaleResolutionDownBy = 1; params.degradationPreference = 'maintain-resolution'; }
      await sender.setParameters(params);
    }
  }
  return pc.localDescription;
}
function inputs(index, video) {
  const context = new AudioContext({ sampleRate: 48000 });
  const destination = context.createMediaStreamDestination();
  for (const frequency of [317, 719, 1249, 2027]) {
    const tone = context.createOscillator(), gain = context.createGain();
    tone.frequency.value = frequency + index * 13; gain.gain.value = 0.07;
    tone.connect(gain).connect(destination); tone.start();
  }
  retained.push(context); context.resume();
  const tracks = [destination.stream.getAudioTracks()[0]];
  if (video) {
    const canvas = document.createElement('canvas'); canvas.width = 1920; canvas.height = 1080;
    const draw = canvas.getContext('2d'); let frame = 0;
    const paint = () => {
      for (let bar = 0; bar < 32; bar++) {
        draw.fillStyle = `hsl(${(bar * 11 + frame) % 360} 80% 50%)`;
        draw.fillRect(bar * 60, 0, 60, 1080);
      }
      draw.fillStyle = 'black'; draw.fillRect((frame * 11) % 1800, 100, 120, 600);
      frame++; requestAnimationFrame(paint);
    };
    paint(); tracks.push(canvas.captureStream(60).getVideoTracks()[0]);
    // Independent screen-source audio, separate from the microphone.
    const source = context.createMediaStreamDestination(), tone = context.createOscillator();
    tone.frequency.value = 440; tone.connect(source); tone.start();
    tracks.push(source.stream.getAudioTracks()[0]);
  }
  retained.push(...tracks); return tracks;
}
async function mediasoup(peers, withVideo) {
  const device = new Device(); await device.load({ routerRtpCapabilities: await rpc({ op: 'capabilities' }) });
  const members = [];
  for (let index = 0; index < peers; index++) {
    const evidence = beginPeer(index, peers, withVideo);
    const peer = `peer-${index}`; await rpc({ op: 'join', peer });
    const make = async direction => {
      const params = await rpc({ op: 'transport', peer });
      const transport = direction === 'send' ? device.createSendTransport(params) : device.createRecvTransport(params);
      connectionTimer(transport, direction, evidence);
      transport.on('connect', ({ dtlsParameters }, ok, fail) => rpc({ op: 'connect', peer, transportId: transport.id, dtlsParameters }).then(ok, fail));
      if (direction === 'send') transport.on('produce', ({ kind, rtpParameters }, ok, fail) => rpc({ op: 'produce', peer, transportId: transport.id, kind, rtpParameters }).then(ok, fail));
      retained.push(transport); return transport;
    };
    const send = await make('send'), recv = await make('recv'), published = [];
    const collect = async () => (await Promise.all([send, recv].map(async transport => [...await transport.getStats()].map(([, value]) => ({ ...value, _endpoint: peer + '/' + transport.direction, _peer: peer }))))).flat();
    observePeer(evidence, collect, 2);
    for (const track of inputs(index, withVideo && index === 0)) {
      const producer = await send.produce({ track, codec: track.kind === 'video' ? device.rtpCapabilities.codecs.find(c => c.mimeType.toLowerCase() === 'video/vp8') : undefined,
        encodings: [{ maxBitrate: track.kind === 'video' ? videoBitrate : 128000, ...(track.kind === 'video' ? { maxFramerate: 60, scaleResolutionDownBy: 1 } : {}) }],
        codecOptions: { opusStereo: false, opusFec: true, opusDtx: false, opusMaxAverageBitrate: 128000 } });
      if (track.kind === 'video') { const settings = producer.rtpSender.getParameters(); settings.degradationPreference = 'maintain-resolution'; await producer.rtpSender.setParameters(settings); }
      published.push(producer.id);
    }
    members.push({ peer, send, recv, published, collect });
  }
  for (const member of members) for (const publisher of members) {
    if (member === publisher) continue;
    for (const producerId of publisher.published) {
      const params = await rpc({ op: 'consume', peer: member.peer, transportId: member.recv.id, producerId, rtpCapabilities: device.rtpCapabilities });
      const consumer = await member.recv.consume(params); retainTrack(consumer.track);
      await rpc({ op: 'resume', peer: member.peer, consumerId: consumer.id });
    }
  }
  window.collect = async () => {
    const reports = [];
    for (const member of members) reports.push(await member.collect());
    return reports.flat();
  };
  window.closePeers = async () => { members.forEach(p => { p.send.close(); p.recv.close(); }); await rpc({ op: 'reset' }); return { engine_stats: await rpc({ op: 'summary' }) }; };
}
async function current(peers, withVideo) {
  const members = [];
  for (let index = 0; index < peers; index++) {
    const evidence = beginPeer(index, peers, withVideo);
    const identity = await rpc({ op: 'join' });
    const pc = nativePc(), ws = new WebSocket(window.backend.replace(/^http/, 'ws') + '/ws');
    connectionTimer(pc, 'media', evidence);
    observePeer(evidence, () => collectNative([pc], evidence.peer), 1);
    const member = { ...identity, pc, ws, index, evidence, queue: Promise.resolve(), answer: null, pendingIce: [], joined: false };
    const send = body => ws.send(JSON.stringify(body));
    // Initial offer includes fully gathered candidates, matching Janus non-trickle mode.
    ws.onmessage = event => {
      const frame = JSON.parse(event.data); console.log('current signal', index, frame.op, frame.e ?? '');
      member.queue = member.queue.then(async () => {
        if (frame.op === 'ok') member.joined = true;
        if (frame.op === 'err') throw new Error(JSON.stringify(frame));
        if (frame.op === 'a' || frame.op === 'o') {
          if (frame.op === 'o' && pc.signalingState !== 'stable') await pc.setLocalDescription({ type: 'rollback' });
          await pc.setRemoteDescription({ type: frame.op === 'a' ? 'answer' : 'offer', sdp: frame.sdp });
          for (const ice of member.pendingIce.splice(0)) await pc.addIceCandidate(ice);
          if (frame.op === 'o') { await pc.setLocalDescription(await pc.createAnswer()); send({ op: 'a', sdp: pc.localDescription.sdp }); }
          else member.answer?.();
        }
        if (frame.op === 'i') {
          const candidate = { candidate: frame.ice, sdpMid: frame.mid ?? null, ...(frame.mid == null ? { sdpMLineIndex: 0 } : {}) };
          if (pc.remoteDescription) await pc.addIceCandidate(candidate); else member.pendingIce.push(candidate);
        }
      }).catch(error => { console.error('current failure', String(error)); failures.push(String(error)); });
    };
    await until(() => ws.readyState === WebSocket.OPEN);
    send({ op: 'j', tk: identity.tk, v: 2 }); await until(() => member.joined); members.push(member);
  }
  // Set Watch before screen publication; voice remains room-scoped.
  if (withVideo) for (const member of members.slice(1)) member.ws.send(JSON.stringify({ op: 'w', u: members[0].user, k: 's', on: true }));
  for (const member of members) {
    await until(() => members.every(p => p.pc.signalingState === 'stable'));
    const tracks = inputs(member.index, withVideo && member.index === 0);
    for (let index = 0; index < tracks.length; index++) {
      const track = tracks[index]; member.pc.addTrack(track, new MediaStream([track]));
      if (index > 0) member.ws.send(JSON.stringify({ op: 'p', k: index === 1 ? 's' : 'sa', t: track.id }));
    }
    const answered = new Promise(resolve => { member.answer = resolve; });
    const sdp = await offer(member.pc); member.ws.send(JSON.stringify({ op: 'o', sdp: sdp.sdp }));
    await Promise.race([answered, sleep(30000).then(() => { throw new Error('current publisher answer timeout'); })]);
    await until(() => member.pc.connectionState === 'connected'); await sleep(300);
  }
  window.collect = async () => (await Promise.all(members.map(p => collectNative([p.pc], p.evidence.peer)))).flat();
  window.closePeers = async () => {
    members.forEach(p => { p.ws.close(); p.pc.close(); });
    let stats; const deadline = Date.now() + 3000;
    do { stats = await rpc({ op: 'summary' }); if (stats.peers === 0 && stats.rooms === 0) break; await sleep(100); } while (Date.now() < deadline);
    return { engine_stats: stats };
  };
}
async function janus(peers, withVideo) {
  const members = [], sessions = [];
  const api = async (path, body) => {
    const value = await (await fetch('/janus' + path, body ? { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) } : undefined)).json();
    if (value.janus === 'error') throw new Error(JSON.stringify(value)); return value;
  };
  const session = async () => {
    const response = await api('', { janus: 'create', transaction: crypto.randomUUID() });
    const entry = { id: response.data.id, pending: new Map(), stopped: false }; sessions.push(entry);
    const poll = async () => {
      while (!entry.stopped) {
        try {
          const events = await api(`/${entry.id}?rid=${Date.now()}&maxev=10`);
          for (const event of Array.isArray(events) ? events : [events]) {
            if (event.transaction && entry.pending.has(event.transaction)) { entry.pending.get(event.transaction)(event); entry.pending.delete(event.transaction); }
          }
        } catch (error) { if (!entry.stopped) failures.push(String(error)); await sleep(100); }
      }
    }; poll();
    entry.request = async (handle, body) => {
      const transaction = crypto.randomUUID(); let resolve;
      const waiting = new Promise(ok => { resolve = ok; }); entry.pending.set(transaction, resolve);
      const result = await api(`/${entry.id}${handle ? '/' + handle : ''}`, { ...body, transaction });
      if (result.janus !== 'ack') { entry.pending.delete(transaction); return result; }
      const event = await Promise.race([waiting, sleep(30000).then(() => { throw new Error('Janus event timeout'); })]);
      if (event.plugindata?.data?.error) throw new Error(JSON.stringify(event)); return event;
    };
    entry.attach = async () => (await entry.request(null, { janus: 'attach', plugin: 'janus.plugin.videoroom' })).data.id;
    return entry;
  };
  const manager = await session(), admin = await manager.attach(), room = 424242;
  await manager.request(admin, { janus: 'message', body: { request: 'create', room, publishers: 64, bitrate: 0, bitrate_cap: false, audiocodec: 'opus', videocodec: 'vp8', opus_fec: true, opus_dtx: false } });
  for (let index = 0; index < peers; index++) {
    const evidence = beginPeer(index, peers, withVideo), connections = [];
    const owner = await session(), handle = await owner.attach(), pc = nativePc();
    connections.push(pc); connectionTimer(pc, 'send', evidence);
    observePeer(evidence, () => collectNative(connections, evidence.peer), 2);
    const joined = await owner.request(handle, { janus: 'message', body: { request: 'join', room, ptype: 'publisher' } });
    const tracks = inputs(index, withVideo && index === 0); tracks.forEach(track => pc.addTrack(track, new MediaStream([track])));
    const jsep = await offer(pc);
    const answer = await owner.request(handle, { janus: 'message', body: { request: 'publish', audio: true, video: withVideo && index === 0 }, jsep: { type: jsep.type, sdp: jsep.sdp } });
    await pc.setRemoteDescription(answer.jsep); await until(() => pc.connectionState === 'connected');
    members.push({ owner, id: joined.plugindata.data.id, tracks, evidence, connections });
  }
  for (const member of members) {
    const handle = await member.owner.attach(), pc = nativePc();
    member.connections.push(pc); connectionTimer(pc, 'recv', member.evidence);
    const streams = members.filter(p => p !== member).flatMap(p => p.tracks.map((_, index) => ({ feed: p.id, mid: String(index) })));
    const received = await member.owner.request(handle, { janus: 'message', body: { request: 'join', room, ptype: 'subscriber', streams } });
    await pc.setRemoteDescription(received.jsep); await pc.setLocalDescription(await pc.createAnswer());
    await until(() => pc.iceGatheringState === 'complete');
    await member.owner.request(handle, { janus: 'message', body: { request: 'start', room }, jsep: { type: pc.localDescription.type, sdp: pc.localDescription.sdp } });
    await until(() => pc.connectionState === 'connected');
  }
  window.collect = async () => (await Promise.all(members.map(p => collectNative(p.connections, p.evidence.peer)))).flat();
  window.closePeers = async () => {
    for (const owner of sessions.filter(s => s !== manager)) { owner.stopped = true; await owner.request(null, { janus: 'destroy' }); }
    await manager.request(admin, { janus: 'message', body: { request: 'destroy', room } });
    const rooms = await manager.request(admin, { janus: 'message', body: { request: 'list' } });
    manager.stopped = true; await manager.request(null, { janus: 'destroy' }); pcs.forEach(pc => pc.close());
    return { engine_stats: { rooms: rooms.plugindata.data.list } };
  };
}
window.startBenchmark = async config => {
  window.backend = config.backend;
  videoBitrate = config.videoBitrate;
  await ({ current, mediasoup, janus })[config.engine](config.peers, config.video);
  await until(() => peerEvidence.every(p => p.dtls_ready_at && p.first_send_rtp_at && p.full_graph_rtp_ready_at));
  peerEvidence.forEach(p => { p.stop = true; }); await Promise.all(timers);
  await sleep(config.warmupMs);
  const samples = [], started = Date.now();
  while (Date.now() - started < config.durationMs) {
    samples.push({ at: Date.now(), stats: await window.collect() }); await sleep(1000);
  }
  const lastStats = samples.at(-1).stats;
  const codecs = new Map(lastStats.filter(s => s.type === 'codec').map(s => [s._endpoint + '/' + s.id, s.mimeType]));
  const incoming = lastStats.filter(s => s.type === 'inbound-rtp' && s.packetsReceived > 0 && s.mid !== 'probator' && !String(codecs.get(s._endpoint + '/' + s.codecId)).endsWith('/rtx'));
  const expectedAudio = config.peers * (config.peers - 1) + (config.video ? config.peers - 1 : 0);
  const expectedVideo = config.video ? config.peers - 1 : 0;
  if (incoming.filter(s => s.kind === 'audio').length !== expectedAudio || incoming.filter(s => s.kind === 'video').length !== expectedVideo) failures.push('Incomplete forwarding graph: expected ' + expectedAudio + ' audio and ' + expectedVideo + ' video inbound streams, got ' + incoming.length);
  const result = { backend: config.engine, peers: config.peers, video: config.video, input: { clip: 'moving-colorbars-v1', audioBitrate: 128000, videoBitrate, width: 1920, height: 1080, requestedFps: 60 }, failures: [...failures], samples, join_timing: peerEvidence.map(({ stop, ...p }) => p) };
  result.post_leave = await window.closePeers(); result.post_leave.at = Date.now();
  retained.forEach(item => { if (item instanceof MediaStreamTrack) item.stop(); if (item instanceof AudioContext) item.close(); });
  return result;
};
