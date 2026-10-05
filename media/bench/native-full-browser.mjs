// Browser participants 1..N-1. Native peer0 replaces participant0; each browser
// receives N-1 microphones plus peer0's separate source audio and one video.
import { Device } from 'mediasoup-client';
import { JanusSignal } from './janus-events.mjs';
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
const failures = []; window.nativeFullFailures = failures;
const members = [], contexts = [], retained = [], identities = new Map();
let config, device, signal, closing;
const until = async (predicate, cleanup = false) => {
  const end = Date.now() + 30000;
  while (Date.now() < end) { if (!cleanup && failures.length) throw new Error(failures.join('; ')); if (predicate()) return; await sleep(25); }
  throw new Error('full-native browser signaling deadline');
};
const rpc = async body => {
  const response = await fetch('/rpc', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body), signal: AbortSignal.timeout(30000) });
  const value = await response.json(); if (!response.ok || value.error) throw new Error('browser fixture RPC: ' + JSON.stringify(value).slice(0, 512)); return value;
};
function microphone(index) {
  const context = new AudioContext({ sampleRate: 48000 }), destination = context.createMediaStreamDestination();
  contexts.push(context);
  for (const frequency of [317, 719, 1249, 2027]) {
    const tone = context.createOscillator(), gain = context.createGain(); tone.frequency.value = frequency + index * 13; gain.gain.value = .07;
    tone.connect(gain).connect(destination); tone.start();
  }
  context.resume().catch(error => failures.push(String(error)));
  const track = destination.stream.getAudioTracks()[0]; retained.push(track); return track;
}
function retain(track) {
  retained.push(track); const element = document.createElement(track.kind === 'video' ? 'video' : 'audio');
  element.autoplay = true; element.muted = true; element.srcObject = new MediaStream([track]); document.body.append(element);
  element.play().catch(error => failures.push(String(error)));
}
function actualPc(member, label, ontrack) {
  const pc = new RTCPeerConnection({ iceServers: [] }); member.connections.push({ connection: pc, label });
  pc.ontrack = event => { retain(event.track); ontrack?.(event); };
  pc.onconnectionstatechange = () => {
    if (pc.connectionState === 'connected') member.timing.connections[label] ??= Date.now();
    if (['failed', 'closed'].includes(pc.connectionState) && !closing) failures.push(member.peer + '/' + label + ' ' + pc.connectionState);
  };
  return pc;
}
async function sendOffer(pc) {
  await pc.setLocalDescription(await pc.createOffer()); await until(() => pc.iceGatheringState === 'complete'); return pc.localDescription;
}
async function capMic(pc) {
  for (const sender of pc.getSenders()) if (sender.track) {
    const parameters = sender.getParameters(); if (parameters.encodings.length !== 1 || sender.track.kind !== 'audio') throw new Error('one actual microphone sender encoding required');
    parameters.encodings[0].maxBitrate = 128000; await sender.setParameters(parameters);
  }
}
function member(index) {
  const value = { index, peer: 'peer-' + index, tracks: [], connections: [], bindings: new Map(), timing: { peer: 'peer-' + index, setup_started_at: Date.now(), connections: {} } };
  members.push(value); return value;
}
function binding(member, key, source, actual) {
  if (!source || member.bindings.has(key) || [...member.bindings.values()].some(value => value.source_name === source)) throw new Error('missing or duplicate actual receive ownership');
  member.bindings.set(key, { source_name: source, ...actual });
}
async function collectMember(member) {
  const stats = [];
  for (const { connection, label } of member.connections) {
    const entries = [...await connection.getStats()].map(([, value]) => value), endpoint = member.peer + '/' + label;
    for (const entry of entries) {
      const value = { ...entry, _peer: member.peer, _endpoint: endpoint };
      if (value.type === 'inbound-rtp' && value.packetsReceived > 0 && value.mid !== 'probator') {
        const owned = member.bindings.get(config.engine === 'mediasoup' ? String(value.ssrc) : config.engine === 'janus' ? String(value.mid) : value.trackIdentifier);
        if (owned) { value._source = owned.source_name; value._binding = owned; }
      }
      if (value.type === 'outbound-rtp' && value.kind === 'audio') value._source = member.peer + '/mic';
      stats.push(value);
    }
  }
  if (stats.some(value => value.type === 'outbound-rtp' && value.packetsSent > 0)) member.timing.first_send_rtp_at ??= Date.now();
  return stats;
}
async function prepareCurrent() {
  identities.set(config.native.user, 0);
  for (let index = 1; index < config.peers; index++) {
    const value = member(index), identity = await rpc({ op: 'join' }); Object.assign(value, identity); identities.set(identity.user, index);
    const pc = actualPc(value, 'media', event => {
      const stream = event.streams[0]?.id, match = /^([^:]+):(a|s)$/.exec(stream ?? ''), owner = match ? identities.get(match[1]) : undefined;
      if (owner === undefined || event.streams.length !== 1) { failures.push('current actual receive MSID has unknown owner'); return; }
      const source = `peer-${owner}/${event.track.kind === 'video' ? 'video' : match[2] === 'a' ? 'mic' : 'screen-audio'}`;
      try { binding(value, event.track.id, source, { track_id: event.track.id, stream_id: stream, owner_user: match[1], kind: event.track.kind, mid: event.transceiver.mid,
        binding_basis: 'actual current receiver track MSID owner/tag and kind' }); } catch (error) { failures.push(String(error)); }
    });
    const ws = value.ws = new WebSocket(config.backend.replace(/^http/, 'ws') + '/ws'); value.queue = Promise.resolve(); value.candidates = []; value.remote = false;
    ws.onerror = () => failures.push('current participant websocket error');
    const send = body => ws.send(JSON.stringify(body));
    ws.onmessage = event => {
      let frame; try { frame = JSON.parse(event.data); } catch (error) { failures.push(String(error)); return; }
      value.queue = value.queue.then(async () => {
        if (frame.op === 'err') throw new Error('current signaling: ' + String(frame.e));
        if (frame.op === 'ok') value.joined = true;
        if (frame.op === 'a' || frame.op === 'o') {
          if (frame.op === 'o' && pc.signalingState !== 'stable') await pc.setLocalDescription({ type: 'rollback' });
          await pc.setRemoteDescription({ type: frame.op === 'a' ? 'answer' : 'offer', sdp: frame.sdp }); value.remote = true;
          for (const candidate of value.candidates.splice(0)) await pc.addIceCandidate(candidate);
          if (frame.op === 'o') { await pc.setLocalDescription(await pc.createAnswer()); send({ op: 'a', sdp: pc.localDescription.sdp }); }
          else value.answered = true;
        }
        if (frame.op === 'i') {
          const candidate = { candidate: frame.ice, sdpMid: frame.mid ?? null, ...(frame.mid == null ? { sdpMLineIndex: 0 } : {}) };
          if (value.remote) await pc.addIceCandidate(candidate); else value.candidates.push(candidate);
        }
      }).catch(error => failures.push(String(error)));
    };
    await until(() => ws.readyState === WebSocket.OPEN); send({ op: 'j', tk: identity.tk, v: 2 }); await until(() => value.joined);
    send({ op: 'w', u: config.native.user, k: 's', on: true });
    value.publish = async () => {
      await until(() => members.every(entry => entry.connections[0].connection.signalingState === 'stable'));
      const track = microphone(index); pc.addTrack(track, new MediaStream([track]));
      const offer = await sendOffer(pc); send({ op: 'o', sdp: offer.sdp }); await until(() => value.answered);
      await capMic(pc); await until(() => pc.connectionState === 'connected');
      value.tracks.push({ source_name: value.peer + '/mic', kind: 'audio', track_id: track.id, owner_user: identity.user });
    };
  }
  for (const value of members) { await value.publish(); await sleep(200); }
}
async function prepareMediasoup() {
  device = new Device(); await device.load({ routerRtpCapabilities: await rpc({ op: 'capabilities' }) });
  for (let index = 1; index < config.peers; index++) {
    const value = member(index); await rpc({ op: 'join', peer: value.peer }); value.joined = true;
    const make = async direction => {
      const parameters = await rpc({ op: 'transport', peer: value.peer });
      const transport = direction === 'send' ? device.createSendTransport(parameters) : device.createRecvTransport(parameters);
      value.connections.push({ connection: transport, label: direction });
      transport.on('connectionstatechange', state => { if (state === 'connected') value.timing.connections[direction] ??= Date.now(); if (['failed', 'closed'].includes(state) && !closing) failures.push(value.peer + '/' + direction + ' ' + state); });
      transport.on('connect', ({ dtlsParameters }, ok, fail) => rpc({ op: 'connect', peer: value.peer, transportId: transport.id, dtlsParameters }).then(ok, fail));
      if (direction === 'send') transport.on('produce', ({ kind, rtpParameters }, ok, fail) => rpc({ op: 'produce', peer: value.peer, transportId: transport.id, kind, rtpParameters }).then(ok, fail));
      return transport;
    };
    value.send = await make('send'); value.recv = await make('recv');
    const producer = await value.send.produce({ track: microphone(index), encodings: [{ maxBitrate: 128000 }], codecOptions: { opusStereo: false, opusFec: true, opusDtx: false, opusMaxAverageBitrate: 128000 } });
    value.tracks.push({ source_name: value.peer + '/mic', kind: 'audio', producer_id: producer.id });
  }
}
async function prepareJanus() {
  signal = new JanusSignal({ failures }); await signal.ready;
  for (let index = 1; index < config.peers; index++) {
    const value = member(index), owner = value.owner = await signal.session(), handle = await owner.attach();
    const joined = await owner.request(handle, { janus: 'message', body: { request: 'join', room: config.native.room, ptype: 'publisher' } });
    value.feed = joined.plugindata.data.id;
    const pc = actualPc(value, 'send'), track = microphone(index); pc.addTrack(track, new MediaStream([track]));
    const offer = await sendOffer(pc);
    const answer = await owner.request(handle, { janus: 'message', body: { request: 'publish', audio: true, video: false }, jsep: { ...offer.toJSON(), trickle: false } });
    const streams = answer.plugindata?.data?.streams;
    if (!Array.isArray(streams) || streams.length !== 1 || streams[0].type !== 'audio' || streams[0].mid === undefined) throw new Error('actual Janus microphone publication MID missing');
    value.tracks.push({ source_name: value.peer + '/mic', kind: 'audio', feed: value.feed, mid: String(streams[0].mid), track_id: track.id });
    await pc.setRemoteDescription(answer.jsep); await capMic(pc); await until(() => pc.connectionState === 'connected');
  }
}
window.nativeFullClose = () => closing ??= (async () => {
  const errors = [], destroyed = [];
  for (const value of members) {
    for (const { connection } of value.connections) connection.close();
    try {
      if (config.engine === 'current') { value.ws?.close(); if (value.ws) await until(() => value.ws.readyState === WebSocket.CLOSED, true); await value.queue; }
      if (config.engine === 'mediasoup' && value.joined) await rpc({ op: 'leave', peer: value.peer });
    } catch (error) { errors.push(String(error)); }
  }
  if (signal) {
    for (const owner of signal.sessions.values()) {
      try { const value = await signal.api('/' + owner.id, { janus: 'destroy', transaction: crypto.randomUUID() }, AbortSignal.timeout(3000));
        if (value.janus !== 'success') throw new Error('Janus participant session destroy not confirmed'); destroyed.push(owner.id);
      } catch (error) { errors.push(String(error)); }
    }
    signal.close();
  }
  retained.forEach(track => track.stop());
  for (const context of contexts) { try { await context.close(); } catch (error) { errors.push(String(error)); } }
  const evidence = { destroyed_sessions: destroyed, connections_closed: members.every(value => value.connections.every(({ connection }) => connection.closed || connection.connectionState === 'closed')),
    pending_requests: signal ? [...signal.sessions.values()].reduce((sum, owner) => sum + owner.pending.size, 0) : 0, errors };
  if (!evidence.connections_closed || evidence.pending_requests) errors.push('browser participant cleanup incomplete');
  if (errors.length) throw new Error('browser full-native cleanup: ' + errors.join('; ')); return evidence;
})();
window.nativeFullPrepare = async value => {
  config = value;
  if (!Number.isInteger(config.peers) || config.peers < 2 || config.peers > 32 || config.native?.index !== 0) throw new Error('native0 plus1..31 browser peers required');
  await ({ current: prepareCurrent, mediasoup: prepareMediasoup, janus: prepareJanus })[config.engine]();
  return members.map(value => ({ index: value.index, peer: value.peer, user: value.user, feed: value.feed, tracks: value.tracks }));
};
window.nativeFullSubscribe = async native => {
  const publications = [native, ...members];
  if (config.engine === 'mediasoup') {
    for (const value of members) for (const publisher of publications.filter(entry => entry.index !== value.index)) for (const track of publisher.tracks) {
      const params = await rpc({ op: 'consume', peer: value.peer, transportId: value.recv.id, producerId: track.producer_id, rtpCapabilities: device.rtpCapabilities });
      if (params.producerId !== track.producer_id || params.kind !== track.kind || params.rtpParameters.encodings.length !== 1) throw new Error('actual mediasoup consumer producer/encoding identity differs');
      const consumer = await value.recv.consume(params); retain(consumer.track);
      binding(value, String(params.rtpParameters.encodings[0].ssrc), track.source_name, { producer_id: params.producerId, consumer_id: params.id, track_id: consumer.track.id, ssrc: params.rtpParameters.encodings[0].ssrc, kind: params.kind, binding_basis: 'actual worker consumer producerId→SSRC' });
      await rpc({ op: 'resume', peer: value.peer, consumerId: params.id });
    }
  } else if (config.engine === 'janus') {
    for (const value of members) {
      const tracks = publications.filter(entry => entry.index !== value.index).flatMap(entry => entry.tracks), handle = await value.owner.attach();
      const pc = actualPc(value, 'recv', event => {
        const owned = value.bindings.get(String(event.transceiver.mid));
        if (!owned || owned.kind !== event.track.kind) { failures.push('actual Janus receiver MID/kind cannot bind source'); return; }
        owned.track_id = event.track.id;
      });
      const received = await value.owner.request(handle, { janus: 'message', body: { request: 'join', room: native.room, ptype: 'subscriber', streams: tracks.map(track => ({ feed: track.feed, mid: track.mid })) } });
      const streams = received.plugindata?.data?.streams;
      if (!Array.isArray(streams) || streams.length !== tracks.length) throw new Error('actual Janus subscriber graph incomplete');
      for (const stream of streams) {
        const track = tracks.find(track => track.feed === stream.feed_id && String(track.mid) === String(stream.feed_mid));
        if (!track || track.kind !== stream.type) throw new Error('actual Janus subscriber feed/MID/kind differs');
        binding(value, String(stream.mid), track.source_name, { feed_id: stream.feed_id, feed_mid: String(stream.feed_mid), mid: String(stream.mid), kind: stream.type, binding_basis: 'actual Janus subscriber feed_id/feed_mid→receiver MID' });
      }
      await pc.setRemoteDescription(received.jsep); await pc.setLocalDescription(await pc.createAnswer()); await until(() => pc.iceGatheringState === 'complete');
      await value.owner.request(handle, { janus: 'message', body: { request: 'start', room: native.room }, jsep: { ...pc.localDescription.toJSON(), trickle: false } });
    }
  }
};
window.nativeFullCollect = async () => {
  if (failures.length) throw new Error(failures.join('; '));
  const stats = (await Promise.all(members.map(collectMember))).flat();
  return { stats, failures: [...failures], connections: members.map(value => ({ peer: value.peer, states: value.connections.map(({ connection, label }) => ({ label, state: connection.connectionState })) })),
    bindings: members.map(value => ({ peer: value.peer, tracks: [...value.bindings.values()] })), join_timing: members.map(value => value.timing) };
};
window.nativeFullReady = async () => {
  await until(() => members.every(value => value.connections.every(({ connection }) => connection.connectionState === 'connected')));
  const end = Date.now() + 30000;
  while (Date.now() < end) {
    const value = await window.nativeFullCollect();
    if (members.every(member => {
      const rows = value.stats.filter(row => row._peer === member.peer && row.type === 'inbound-rtp' && row.packetsReceived > 0 && row.mid !== 'probator');
      const sources = new Set(rows.map(row => row._source));
      const complete = rows.filter(row => row.kind === 'audio').length === config.peers && rows.filter(row => row.kind === 'video').length === 1 && sources.size === config.peers + 1 && !sources.has(undefined);
      if (complete) { member.timing.full_graph_rtp_ready_at ??= Date.now(); member.timing.dtls_ready_at ??= Math.max(...Object.values(member.timing.connections)); } return complete;
    })) return value;
    await sleep(50);
  }
  throw new Error('full-native actual browser receive graph deadline');
};
