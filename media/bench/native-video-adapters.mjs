// Ordinary DTLS/SRTP endpoints; no PlainTransport, cascade, RTP injection or
// production authorization semantics are substituted in these diagnostic APIs.
import { JanusSignal } from './janus-events.mjs';
import { JanusBroker } from './janus-broker.mjs';
import { mediasoupNativeAnswer } from './mediasoup-native-sdp.mjs';
import { rethrowAfterCleanup } from './native-video.mjs';
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
const description = value => value.sdp.split(/\r?\n/).filter(line => /^m=|^c=|^a=(candidate:|end-of-candidates|ice-lite|setup:|fingerprint:|mid:|rtpmap:|fmtp:|rtcp-fb:|extmap:|sendonly|recvonly|sendrecv)/.test(line));
async function until(predicate, failure) {
  for (let i = 0; i < 600; i++) { if (failure?.()) throw failure(); if (predicate()) return; await sleep(50); }
  throw new Error('native diagnostic signaling deadline');
}

export async function nativePublisher(engine, source, backend, token) {
  const controllers = new Set();
  const post = async (url, body, signal) => {
    const controller = new AbortController(); controllers.add(controller);
    try {
      const response = await fetch(backend + url, { method: 'POST', headers: { 'content-type': 'application/json', authorization: 'Bearer ' + token },
        body: JSON.stringify(body), signal: AbortSignal.any([controller.signal, signal ?? AbortSignal.timeout(30000)]) });
      const value = await response.json();
      if (!response.ok || value.error || value.janus === 'error') throw new Error(`native diagnostic ${body.op ?? body.janus} failed: ${String(value.error ?? response.status).slice(0, 512)}`);
      return value;
    } finally { controllers.delete(controller); }
  };
  const rpc = body => post('/rpc', body);
  const scope = { engine, transport: 'WebRTC DTLS/SRTP', source_publishers: 1, voice_peers: 0 };
  const offered = (await source.call({ op: 'offer' })).description;
  if (engine === 'mediasoup') {
    const close = async () => {
      await rpc({ op: 'leave', peer: 'native-source' }); const stats = await rpc({ op: 'summary' });
      if (['peers', 'transports', 'producers', 'consumers'].some(key => stats[key] !== 0)) throw new Error('mediasoup diagnostic resources remain after leave');
      return { engine_stats: stats };
    };
    try {
      await rpc({ op: 'join', peer: 'native-source' });
      const transport = await rpc({ op: 'transport', peer: 'native-source' }), capabilities = await rpc({ op: 'capabilities' });
      const bridge = mediasoupNativeAnswer(offered, capabilities, transport);
      await rpc({ op: 'connect', peer: 'native-source', transportId: transport.id, dtlsParameters: bridge.dtlsParameters });
      const producer = await rpc({ op: 'produce', peer: 'native-source', transportId: transport.id, kind: 'video', rtpParameters: bridge.rtpParameters });
      await source.call({ op: 'remote', description: bridge.description });
      return { receiver: { engine, producerId: producer.id }, scope,
        diagnostics: () => ({ negotiation: { local: description(offered), remote: description(bridge.description) },
          worker_fingerprints: transport.dtlsParameters.fingerprints }), close };
    } catch (error) { await rethrowAfterCleanup(error, close); }
  }
  if (engine === 'janus') {
    const stream = { close() {} }, failures = [];
    let broker;
    const signal = new JanusSignal({ failures, eventSource: () => stream, fetcher: async (url, options) => {
      const body = { ...JSON.parse(options.body), apisecret: token };
      if (body.janus === 'destroy') await broker.remove(Number(url.split('/')[2]));
      const value = await post(url, body, options.signal);
      if (body.janus === 'create') broker.add(value.data.id);
      return { ok: true, json: async () => value };
    } });
    broker = new JanusBroker(async (id, aborted) => {
      const url = new URL(`${backend}/janus/${id}?rid=${Date.now()}&maxev=10`); url.searchParams.set('apisecret', token);
      const response = await fetch(url, { signal: aborted }); if (!response.ok) throw new Error('native Janus poll failed'); return response.json();
    });
    broker.emit = envelope => stream.onmessage({ data: JSON.stringify(envelope) }); stream.onopen();
    let manager, admin, roomCreated = false; const room = 424243;
    const close = async () => {
      const errors = [], destroyed = []; let rooms;
      try {
        if (roomCreated) {
          try { await manager.request(admin, { janus: 'message', body: { request: 'destroy', room } }); }
          catch (error) { errors.push(String(error)); }
          try { rooms = (await manager.request(admin, { janus: 'message', body: { request: 'list' } })).plugindata.data.list; }
          catch (error) { errors.push(String(error)); }
        }
      }
      finally {
        await Promise.all([...signal.sessions.keys()].map(async id => {
          try {
            const response = await signal.api('/' + id, { janus: 'destroy', transaction: crypto.randomUUID() }, AbortSignal.timeout(3000));
            if (response.janus !== 'success') throw new Error('Janus session destroy not confirmed'); destroyed.push(id);
          } catch (error) { errors.push(String(error)); }
        }));
        signal.close(); await broker.close(); controllers.forEach(controller => controller.abort());
      }
      if (rooms?.some(entry => entry.room === room)) errors.push('Janus diagnostic room remains');
      const evidence = { engine_stats: { rooms, diagnostic_room_present: rooms ? rooms.some(entry => entry.room === room) : null, diagnostic_publishers: rooms && !rooms.some(entry => entry.room === room) ? 0 : null, diagnostic_subscribers: rooms && !rooms.some(entry => entry.room === room) ? 0 : null,
        count_basis: 'confirmed room destroy and absence from room list; sessions confirmed destroyed separately' }, destroyed_sessions: destroyed, backend_events: broker.evidence() };
      if (errors.length) { const error = new Error('Janus publisher cleanup: ' + errors.join('; ')); error.cleanup_evidence = evidence; throw error; }
      return evidence;
    };
    try {
      manager = await signal.session(); admin = await manager.attach();
      await manager.request(admin, { janus: 'message', body: { request: 'create', room, publishers: 4, bitrate: 0, bitrate_cap: false, videocodec: 'vp8' } }); roomCreated = true;
      const owner = await signal.session(), handle = await owner.attach();
      const joined = await owner.request(handle, { janus: 'message', body: { request: 'join', room, ptype: 'publisher' } });
      const published = await owner.request(handle, { janus: 'message', body: { request: 'publish', audio: false, video: true }, jsep: { ...offered, trickle: false } });
      await source.call({ op: 'remote', description: published.jsep });
      const streams = published.plugindata?.data?.streams ?? joined.plugindata?.data?.streams;
      const mid = streams?.find(stream => stream.type === 'video')?.mid ?? offered.sdp.match(/^a=mid:(.+)$/m)?.[1]?.trim();
      if (!mid) throw new Error('native Janus video MID absent');
      return { receiver: { engine, room, feed: joined.plugindata.data.id, mid: String(mid) }, scope,
        diagnostics: () => ({ requests: signal.trace, backend_events: broker.evidence(), negotiation: { local: description(offered), remote: description(published.jsep) } }), close };
    } catch (error) { await rethrowAfterCleanup(error, close); }
  }
  if (engine !== 'current') throw new Error('unsupported native diagnostic engine');
  const identity = await rpc({ op: 'join' }), socket = new WebSocket(backend.replace(/^http/, 'ws') + '/ws');
  let joined = false, answered = false, remote = false, failure, queue = Promise.resolve(); const candidates = [], trace = [], negotiation = { local: description(offered), remote: [] };
  const send = value => socket.send(JSON.stringify(value));
  socket.onerror = () => { failure = new Error('native current websocket failed'); };
  socket.onmessage = message => {
    const frame = JSON.parse(message.data); trace.push({ at: Date.now(), operation: frame.op });
    queue = queue.then(async () => {
      if (frame.op === 'err') throw new Error('native current signaling error: ' + frame.e);
      if (frame.op === 'ok') joined = true;
      if (frame.op === 'a' || frame.op === 'o') {
        negotiation.remote = description({ sdp: frame.sdp });
        const result = await source.call({ op: 'remote', description: { type: frame.op === 'a' ? 'answer' : 'offer', sdp: frame.sdp } }); remote = true;
        for (const candidate of candidates.splice(0)) await source.call({ op: 'ice', candidate });
        if (frame.op === 'o') send({ op: 'a', sdp: result.description.sdp }); else answered = true;
      }
      if (frame.op === 'i') {
        const candidate = { candidate: frame.ice, sdpMid: frame.mid ?? null, ...(frame.mid == null ? { sdpMLineIndex: 0 } : {}) };
        if (remote) await source.call({ op: 'ice', candidate }); else candidates.push(candidate);
      }
    }).catch(error => { failure = error; });
  };
  const close = async () => {
    socket.close(); await queue; let stats;
    for (let attempt = 0; attempt < 60; attempt++) {
      stats = await rpc({ op: 'summary' }); if (stats.peers === 0 && stats.rooms === 0) break; await sleep(50);
    }
    if (stats?.peers !== 0 || stats?.rooms !== 0) throw new Error('current diagnostic resources remain after leave');
    return { engine_stats: stats };
  };
  try {
    await until(() => socket.readyState === WebSocket.OPEN, () => failure);
    send({ op: 'j', tk: identity.tk, v: 2 }); await until(() => joined, () => failure);
    const track = offered.sdp.match(/^a=msid:\S+ (\S+)/m)?.[1]; if (!track) throw new Error('native MSID track identity missing');
    send({ op: 'p', k: 's', t: track }); send({ op: 'o', sdp: offered.sdp }); await until(() => answered, () => failure);
    return { receiver: { engine, backend, owner: identity.user }, scope, diagnostics: () => ({ trace, negotiation, error: failure?.message }), close };
  } catch (error) { controllers.forEach(controller => controller.abort()); await rethrowAfterCleanup(error, close); }
}
