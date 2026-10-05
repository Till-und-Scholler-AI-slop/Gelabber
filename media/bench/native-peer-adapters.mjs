// Native peer0 is one ordinary participant, with three publishing tracks and
// one incoming microphone from every other participant. No RTP injection.
import { JanusSignal } from './janus-events.mjs';
import { JanusBroker } from './janus-broker.mjs';
import { nativeCurrentPeer0 } from './native-peer-current.mjs';
import { nativePublicationIdentity } from './native-peer-checks.mjs';
import { mediasoupNativePeerPublish, mediasoupNativePeerReceive, mediasoupNativeReceiveDtls } from './mediasoup-native-peer-sdp.mjs';
import { rethrowAfterCleanup } from './native-video.mjs';
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
const filtered = value => value.sdp.split(/\r?\n/).filter(line => /^m=|^c=|^a=(candidate:|setup:|fingerprint:|mid:|rtpmap:|fmtp:|rtcp-fb:|ssrc:|msid:|sendonly|recvonly|sendrecv)/.test(line));

async function bindReceived(native, peer, bindings) {
  const deadline = Date.now() + 30000;
  while (Date.now() < deadline) {
    const status = await native.call({ op: 'status' }), received = status.peers[peer]?.received;
    if (received && Object.keys(received).length === bindings.length && bindings.every(binding => received[String(binding.ssrc)]?.packets_received > 0)) {
      for (const binding of bindings) await native.call({ op: 'bind', peer, ssrc: binding.ssrc, source_name: binding.source_name });
      return bindings;
    }
    await sleep(50);
  }
  throw new Error('native actual receive graph deadline');
}

export function janusReceiveBindings(description, streams, publications) {
  if (description?.type !== 'offer' || !Array.isArray(streams) || !Array.isArray(publications) || publications.length < 1) throw new Error('actual Janus subscriber offer/stream ownership required');
  const bindings = [], owners = new Set(), mids = new Set(), ssrcs = new Set();
  for (const stream of streams) {
    const owner = publications.find(value => value.feed === stream.feed_id && String(value.mid) === String(stream.feed_mid));
    if (!owner || owner.kind !== 'audio' || stream.type !== 'audio' || owners.has(owner.source_name) || mids.has(String(stream.mid))) throw new Error('Janus actual feed/MID microphone ownership missing or duplicate');
    const section = description.sdp.split(/(?=^m=)/m).find(value => value.split(/\r?\n/).includes('a=mid:' + stream.mid));
    const actual = [...new Set((section?.match(/^a=ssrc:(\d+) cname:/gm) ?? []).map(line => Number(line.match(/\d+/)[0])))];
    if (!section?.startsWith('m=audio ') || actual.length !== 1 || !actual[0] || actual[0] > 0xffffffff || ssrcs.has(actual[0])) throw new Error('Janus actual offered audio MID/SSRC missing or duplicate');
    owners.add(owner.source_name); mids.add(String(stream.mid)); ssrcs.add(actual[0]);
    bindings.push({ source_name: owner.source_name, feed_id: stream.feed_id, feed_mid: String(stream.feed_mid), mid: String(stream.mid), ssrc: actual[0],
      binding_basis: 'actual Janus subscriber feed_id/feed_mid→offer MID→actual offered SSRC' });
  }
  if (bindings.length !== publications.length) throw new Error('Janus native receive graph incomplete');
  return bindings;
}

export async function nativePeerAdapter(engine, native, backend, token) {
  const origin = new URL(backend);
  if (!['http:', 'https:'].includes(origin.protocol) || origin.username || origin.password || origin.pathname !== '/' || origin.search || origin.hash || typeof token !== 'string' || token.length < 32) throw new Error('explicit backend origin/private token required');
  if (engine === 'current') {
    const adapter = await nativeCurrentPeer0(native, origin.origin, token);
    return { ...adapter, receive: members => adapter.bind(members), receiver_peer: 'publish' };
  }
  const controllers = new Set();
  const post = async (path, body, signal) => {
    const controller = new AbortController(); controllers.add(controller);
    try {
      const response = await fetch(origin.origin + path, { method: 'POST', headers: { 'content-type': 'application/json', authorization: 'Bearer ' + token },
        body: JSON.stringify(body), signal: AbortSignal.any([controller.signal, signal ?? AbortSignal.timeout(30000)]) });
      const value = await response.json();
      if (!response.ok || value.error || value.janus === 'error') throw new Error('native peer0 signaling failed: ' + JSON.stringify(value).slice(0, 512));
      return value;
    } finally { controllers.delete(controller); }
  };
  const rpc = body => post('/rpc', body);
  await native.call({ op: 'create', peer: 'publish', publish: true });
  const offer = (await native.call({ op: 'offer', peer: 'publish' })).description;
  const sourceIds = nativePublicationIdentity(offer);
  if (engine === 'mediasoup') {
    let joined = false, publication, receiveBridge;
    const close = async () => {
      controllers.forEach(controller => controller.abort());
      if (joined) await rpc({ op: 'leave', peer: 'peer-0' });
      return { peer_left: joined, engine_stats: await rpc({ op: 'summary' }) };
    };
    try {
      await rpc({ op: 'join', peer: 'peer-0' }); joined = true;
      const transport = await rpc({ op: 'transport', peer: 'peer-0' });
      const bridge = mediasoupNativePeerPublish(offer, await rpc({ op: 'capabilities' }), transport);
      await rpc({ op: 'connect', peer: 'peer-0', transportId: transport.id, dtlsParameters: bridge.dtlsParameters });
      const tracks = [];
      for (const source of bridge.publications) {
        const producer = await rpc({ op: 'produce', peer: 'peer-0', transportId: transport.id, kind: source.kind, rtpParameters: source.rtpParameters });
        tracks.push({ source_name: 'peer-0/' + source.role, kind: source.kind, producer_id: producer.id, track_id: source.track, ssrc: source.ssrc });
      }
      await native.call({ op: 'remote', peer: 'publish', description: bridge.description });
      publication = { index: 0, peer: 'peer-0', tracks };
      return { engine, publication, receiver_peer: 'receive', check() {},
        diagnostics: () => ({ publication_offer: filtered(offer), publication_answer: filtered(bridge.description), receive_offer: receiveBridge ? filtered(receiveBridge.description) : null,
          worker_fingerprints: transport.dtlsParameters.fingerprints }),
        receive: async members => {
          const peers = new Set();
          if (!Array.isArray(members) || !members.length || members.some(member => !/^peer-(?:[1-9]|[12][0-9]|3[01])$/.test(member.peer) || peers.has(member.peer) || (peers.add(member.peer), member.tracks?.length !== 1) || member.tracks[0].source_name !== member.peer + '/mic' || !member.tracks[0].producer_id)) throw new Error('actual foreign microphone producer manifest required');
          await native.call({ op: 'create', peer: 'receive', publish: false });
          const receiveTransport = await rpc({ op: 'transport', peer: 'peer-0' }), consumers = [];
          for (const member of members) {
            const track = member.tracks[0];
            const consumer = await rpc({ op: 'consume', peer: 'peer-0', transportId: receiveTransport.id, producerId: track.producer_id, rtpCapabilities: bridge.receiverCapabilities });
            if (consumer.producerId !== track.producer_id) throw new Error('actual worker consumer producer binding differs');
            consumers.push({ ...consumer, owner: track.source_name });
          }
          receiveBridge = mediasoupNativePeerReceive(consumers, receiveTransport);
          const answer = (await native.call({ op: 'remote', peer: 'receive', description: receiveBridge.description })).description;
          await rpc({ op: 'connect', peer: 'peer-0', transportId: receiveTransport.id, dtlsParameters: mediasoupNativeReceiveDtls(answer) });
          for (const consumer of consumers) await rpc({ op: 'resume', peer: 'peer-0', consumerId: consumer.id });
          return bindReceived(native, 'receive', receiveBridge.bindings);
        }, close };
    } catch (error) { await rethrowAfterCleanup(error, close); }
  }
  if (engine !== 'janus') throw new Error('unsupported native peer0 engine');
  const stream = { close() {} }, failures = []; let broker;
  const signal = new JanusSignal({ failures, eventSource: () => stream, fetcher: async (url, options) => {
    const body = { ...JSON.parse(options.body), apisecret: token };
    if (body.janus === 'destroy') await broker.remove(Number(url.split('/')[2]));
    const value = await post(url, body, options.signal); if (body.janus === 'create') broker.add(value.data.id);
    return { ok: true, json: async () => value };
  } });
  broker = new JanusBroker(async (id, aborted) => {
    const url = new URL(`${origin.origin}/janus/${id}?rid=${Date.now()}&maxev=10`); url.searchParams.set('apisecret', token);
    const response = await fetch(url, { signal: aborted }); if (!response.ok) throw new Error('native Janus poll failed'); return response.json();
  });
  broker.emit = envelope => stream.onmessage({ data: JSON.stringify(envelope) }); stream.onopen();
  let manager, admin, owner, roomCreated = false, receiveDescription; const room = 424244;
  const close = async () => {
    const errors = [], destroyed = []; let rooms;
    try {
      if (roomCreated) {
        try { await manager.request(admin, { janus: 'message', body: { request: 'destroy', room } }); } catch (error) { errors.push(String(error)); }
        try { rooms = (await manager.request(admin, { janus: 'message', body: { request: 'list' } })).plugindata.data.list; } catch (error) { errors.push(String(error)); }
      }
    } finally {
      await Promise.all([...signal.sessions.keys()].map(async id => {
        try {
          const response = await signal.api('/' + id, { janus: 'destroy', transaction: crypto.randomUUID() }, AbortSignal.timeout(3000));
          if (response.janus !== 'success') throw new Error('Janus session destroy not confirmed'); destroyed.push(id);
        } catch (error) { errors.push(String(error)); }
      }));
      signal.close(); await broker.close(); controllers.forEach(controller => controller.abort());
    }
    if (!Array.isArray(rooms) || rooms.some(value => value.room === room)) errors.push('owned Janus room absence not confirmed');
    const evidence = { engine_stats: { rooms, room_present: rooms?.some(value => value.room === room),
      count_basis: 'owned room destroyed/list absence; sessions destroyed separately' }, destroyed_sessions: destroyed, backend_events: broker.evidence() };
    if (errors.length) { const error = new Error('native Janus cleanup: ' + errors.join('; ')); error.cleanup_evidence = evidence; throw error; }
    return evidence;
  };
  try {
    manager = await signal.session(); admin = await manager.attach();
    await manager.request(admin, { janus: 'message', body: { request: 'create', room, publishers: 64, bitrate: 0, bitrate_cap: false, audiocodec: 'opus', videocodec: 'vp8', opus_fec: true, opus_dtx: false } }); roomCreated = true;
    owner = await signal.session(); const handle = await owner.attach();
    const joined = await owner.request(handle, { janus: 'message', body: { request: 'join', room, ptype: 'publisher' } });
    const published = await owner.request(handle, { janus: 'message', body: { request: 'publish', audio: true, video: true }, jsep: { ...offer, trickle: false } });
    await native.call({ op: 'remote', peer: 'publish', description: published.jsep });
    const streams = published.plugindata?.data?.streams;
    if (!Array.isArray(streams) || streams.length !== 3 || sourceIds.some(source => streams.filter(value => String(value.mid) === source.mid && value.type === source.kind).length !== 1)) throw new Error('actual Janus native publication stream manifest differs');
    const publication = { index: 0, peer: 'peer-0', feed: joined.plugindata.data.id, room,
      tracks: sourceIds.map(source => ({ source_name: 'peer-0/' + source.role, kind: source.kind, feed: joined.plugindata.data.id, mid: source.mid, track_id: source.track_id, ssrc: source.ssrc })) };
    return { engine, publication, receiver_peer: 'receive', check: () => { if (failures.length) throw new Error(failures.join('; ')); },
      diagnostics: () => ({ requests: signal.trace, backend_events: broker.evidence(), publication_offer: filtered(offer), publication_answer: filtered(published.jsep), receive_offer: receiveDescription ? filtered(receiveDescription) : null }),
      receive: async members => {
        const tracks = members.flatMap(member => member.tracks);
        if (members.length !== tracks.length || tracks.some(track => track.kind !== 'audio' || !/^peer-(?:[1-9]|[12][0-9]|3[01])\/mic$/.test(track.source_name))) throw new Error('actual Janus foreign microphones required');
        await native.call({ op: 'create', peer: 'receive', publish: false });
        const subscriber = await owner.attach();
        const received = await owner.request(subscriber, { janus: 'message', body: { request: 'join', room, ptype: 'subscriber', streams: tracks.map(track => ({ feed: track.feed, mid: track.mid })) } });
        receiveDescription = received.jsep;
        const bindings = janusReceiveBindings(received.jsep, received.plugindata?.data?.streams, tracks);
        const answer = (await native.call({ op: 'remote', peer: 'receive', description: received.jsep })).description;
        await owner.request(subscriber, { janus: 'message', body: { request: 'start', room }, jsep: { ...answer, trickle: false } });
        return bindReceived(native, 'receive', bindings);
      }, close };
  } catch (error) { await rethrowAfterCleanup(error, close); }
}
