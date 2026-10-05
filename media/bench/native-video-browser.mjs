// Diagnostic topology: one browser video decoder, zero voice publications.
import { Device } from 'mediasoup-client';
import { JanusSignal } from './janus-events.mjs';
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
const until = async predicate => {
  for (let i = 0; i < 600; i++) { if (predicate()) return; await sleep(50); }
  throw new Error('diagnostic receiver deadline');
};
const failures = []; window.nativePilotFailures = failures;
const rpc = async body => {
  const response = await fetch('/rpc', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body), signal: AbortSignal.timeout(30000) });
  const value = await response.json(); if (!response.ok || value.error) throw new Error('diagnostic RPC failed'); return value;
};
const retain = track => {
  const element = document.createElement('video'); element.autoplay = true; element.muted = true;
  element.srcObject = new MediaStream([track]); document.body.append(element); element.play().catch(error => failures.push(String(error)));
};
const pc = () => { const connection = new RTCPeerConnection({ iceServers: [] }); connection.ontrack = event => retain(event.track); return connection; };
window.nativePilotSetup = async config => {
  if (config.engine === 'mediasoup') {
    const device = new Device(); await device.load({ routerRtpCapabilities: await rpc({ op: 'capabilities' }) });
    await rpc({ op: 'join', peer: 'decoder' });
    let recv, closing;
    window.nativePilotClose = () => closing ??= (async () => { recv?.close(); await rpc({ op: 'leave', peer: 'decoder' }); return { peer_left: true }; })();
    const params = await rpc({ op: 'transport', peer: 'decoder' });
    recv = device.createRecvTransport(params);
    recv.on('connect', ({ dtlsParameters }, ok, fail) => rpc({ op: 'connect', peer: 'decoder', transportId: recv.id, dtlsParameters }).then(ok, fail));
    const consumer = await recv.consume(await rpc({ op: 'consume', peer: 'decoder', transportId: recv.id, producerId: config.producerId, rtpCapabilities: device.rtpCapabilities }));
    retain(consumer.track); await rpc({ op: 'resume', peer: 'decoder', consumerId: consumer.id });
    window.nativePilotCollect = async () => [...await recv.getStats()].map(([, value]) => value);
    return;
  }
  const connection = pc();
  window.nativePilotCollect = async () => [...await connection.getStats()].map(([, value]) => value);
  if (config.engine === 'janus') {
    const signal = new JanusSignal({ failures });
    let closing;
    window.nativePilotClose = () => closing ??= (async () => {
      const destroyed = [], errors = [];
      try {
        for (const session of signal.sessions.values()) {
          try {
            const response = await signal.api(`/${session.id}`, { janus: 'destroy', transaction: crypto.randomUUID() }, AbortSignal.timeout(3000));
            if (response.janus !== 'success') throw new Error('Janus destroy not confirmed'); destroyed.push(session.id);
          } catch (error) { errors.push(String(error)); }
        }
      } finally { signal.close(); connection.close(); }
      if (errors.length) throw new Error('Janus receiver cleanup: ' + errors.join('; '));
      return { destroyed_sessions: destroyed, remaining_pending: [...signal.sessions.values()].reduce((n, session) => n + session.pending.size, 0), connection_state: connection.connectionState };
    })();
    await signal.ready; const session = await signal.session(), handle = await session.attach();
    const received = await session.request(handle, { janus: 'message', body: { request: 'join', room: config.room, ptype: 'subscriber', streams: [{ feed: config.feed, mid: config.mid }] } });
    await connection.setRemoteDescription(received.jsep); await connection.setLocalDescription(await connection.createAnswer());
    await until(() => connection.iceGatheringState === 'complete');
    await session.request(handle, { janus: 'message', body: { request: 'start', room: config.room }, jsep: { ...connection.localDescription.toJSON(), trickle: false } });
  } else if (config.engine === 'current') {
    const identity = await rpc({ op: 'join' }), socket = new WebSocket(config.backend.replace(/^http/, 'ws') + '/ws');
    let joined = false, answered = false, queue = Promise.resolve(), remote = false; const candidates = [];
    const send = value => socket.send(JSON.stringify(value));
    window.nativePilotClose = async () => {
      socket.close(); await queue; connection.close(); await until(() => socket.readyState === WebSocket.CLOSED);
      return { connection_state: connection.connectionState, websocket_state: socket.readyState };
    };
    socket.onmessage = message => {
      const frame = JSON.parse(message.data);
      queue = queue.then(async () => {
        if (frame.op === 'err') throw new Error('current diagnostic signaling error: ' + frame.e);
        if (frame.op === 'ok') joined = true;
        if (frame.op === 'a' || frame.op === 'o') {
          await connection.setRemoteDescription({ type: frame.op === 'a' ? 'answer' : 'offer', sdp: frame.sdp }); remote = true;
          for (const candidate of candidates.splice(0)) await connection.addIceCandidate(candidate);
          if (frame.op === 'o') { await connection.setLocalDescription(await connection.createAnswer()); send({ op: 'a', sdp: connection.localDescription.sdp }); }
          else answered = true;
        }
        if (frame.op === 'i') {
          const candidate = { candidate: frame.ice, sdpMid: frame.mid ?? null, ...(frame.mid == null ? { sdpMLineIndex: 0 } : {}) };
          if (remote) await connection.addIceCandidate(candidate); else candidates.push(candidate);
        }
      }).catch(error => failures.push(String(error)));
    };
    await until(() => socket.readyState === WebSocket.OPEN); send({ op: 'j', tk: identity.tk, v: 2 }); await until(() => joined);
    send({ op: 'w', u: config.owner, k: 's', on: true });
    connection.addTransceiver('video', { direction: 'recvonly' });
    await connection.setLocalDescription(await connection.createOffer()); await until(() => connection.iceGatheringState === 'complete');
    send({ op: 'o', sdp: connection.localDescription.sdp }); await until(() => answered);
  } else throw new Error('unsupported diagnostic engine');
};
window.nativePilotSamples = async seconds => {
  const samples = [], start = Date.now();
  while (Date.now() - start < seconds * 1000) {
    if (failures.length) throw new Error(failures.join('; '));
    samples.push({ at: Date.now(), stats: await window.nativePilotCollect() }); await sleep(1000);
  }
  return samples;
};
