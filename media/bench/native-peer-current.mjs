// Native peer0 in the current fixture's actual ticket/WS/publish/Watch protocol.
// Ordinary WebRTC microphone reception stays in its publishing connection.
import { rethrowAfterCleanup } from './native-video.mjs';
import { nativePublicationIdentity } from './native-peer-checks.mjs';
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
const describe = value => value.sdp.split(/\r?\n/).filter(line => /^m=|^c=|^a=(candidate:|end-of-candidates|ice-lite|setup:|fingerprint:|mid:|rtpmap:|fmtp:|rtcp-fb:|extmap:|ssrc:|ssrc-group:|msid:|sendonly|recvonly|sendrecv)/.test(line));
async function until(predicate, failure) {
  const deadline = Date.now() + 30000;
  while (Date.now() < deadline) { if (failure?.()) throw failure(); if (predicate()) return; await sleep(25); }
  throw new Error('native current signaling deadline');
}

export function currentNativeMicrophoneBindings(received, members) {
  if (!received || !Array.isArray(members) || members.length < 1 || members.length > 31) throw new Error('actual current microphone graph required');
  const owners = new Map();
  for (const member of members) {
    if (!member.user || !Number.isInteger(member.index) || member.index < 1 || member.index > 31 || owners.has(member.user) || [...owners.values()].includes(member.index)) throw new Error('unique actual joined user/peer mapping required');
    owners.set(member.user, member.index);
  }
  const bindings = [], sources = new Set(), ssrcs = new Set();
  for (const edge of Object.values(received)) {
    if (!Number.isInteger(edge.ssrc) || edge.ssrc <= 0 || edge.ssrc > 0xffffffff || ssrcs.has(edge.ssrc) || !edge.track_id || edge.stream_ids?.length !== 1) throw new Error('actual current receive track/SSRC/MSID missing or duplicated');
    const match = /^([^:]+):a$/.exec(edge.stream_ids[0]);
    const index = match ? owners.get(match[1]) : undefined;
    if (!index) throw new Error('native receive MSID is not an actual joined foreign microphone');
    const source = `peer-${index}/mic`;
    if (sources.has(source)) throw new Error('duplicate actual native microphone source');
    sources.add(source); ssrcs.add(edge.ssrc);
    bindings.push({ source_name: source, owner_user: match[1], ssrc: edge.ssrc, track_id: edge.track_id, stream_id: edge.stream_ids[0],
      binding_basis: 'current server SDP owner:a MSID and actual native receive SSRC' });
  }
  if (bindings.length !== owners.size) throw new Error('incomplete native microphone receive graph');
  return bindings;
}

export async function nativeCurrentPeer0(native, backend, token) {
  const origin = new URL(backend);
  if (!['http:', 'https:'].includes(origin.protocol) || origin.username || origin.password || origin.pathname !== '/' || origin.search || origin.hash || typeof token !== 'string' || token.length < 32) throw new Error('explicit benchmark origin and private token required');
  const controllers = new Set(), post = async body => {
    const controller = new AbortController(); controllers.add(controller);
    try {
      const response = await fetch(origin.origin + '/rpc', { method: 'POST', headers: { 'content-type': 'application/json', authorization: 'Bearer ' + token },
        body: JSON.stringify(body), signal: AbortSignal.any([controller.signal, AbortSignal.timeout(30000)]) });
      const value = await response.json(); if (!response.ok || value.error) throw new Error('current fixture RPC failed: ' + String(value.error ?? response.status).slice(0, 512)); return value;
    } finally { controllers.delete(controller); }
  };
  let socket, identity, queue = Promise.resolve(), joined = false, answered = false, remote = false, failure;
  const pendingIce = [], trace = [], negotiation = [], started = Date.now();
  const send = value => socket.send(JSON.stringify(value));
  const close = async () => {
    controllers.forEach(controller => controller.abort());
    if (socket && socket.readyState < WebSocket.CLOSING) socket.close();
    if (socket) await until(() => socket.readyState === WebSocket.CLOSED);
    await queue;
    return { socket_closed: !socket || socket.readyState === WebSocket.CLOSED, pending_candidates: pendingIce.length,
      count_basis: 'participant websocket close; whole-engine counters collected after every participant leaves' };
  };
  try {
    await native.call({ op: 'create', peer: 'publish', publish: true });
    identity = await post({ op: 'join' });
    socket = new WebSocket(origin.origin.replace(/^http/, 'ws') + '/ws');
    socket.onerror = () => { failure = new Error('native peer0 current websocket error'); };
    socket.onmessage = event => {
      let frame;
      try { frame = JSON.parse(event.data); } catch (error) { failure = error; return; }
      trace.push({ at: Date.now(), operation: frame.op });
      queue = queue.then(async () => {
        if (frame.op === 'err') throw new Error('native current signal error: ' + String(frame.e).slice(0, 512));
        if (frame.op === 'ok') joined = true;
        if (frame.op === 'a' || frame.op === 'o') {
          const description = { type: frame.op === 'a' ? 'answer' : 'offer', sdp: frame.sdp };
          negotiation.push({ at: Date.now(), direction: 'remote', description: describe(description) });
          const result = await native.call({ op: 'remote', peer: 'publish', description }); remote = true;
          for (const candidate of pendingIce.splice(0)) await native.call({ op: 'ice', peer: 'publish', candidate });
          if (frame.op === 'o') {
            negotiation.push({ at: Date.now(), direction: 'local-answer', description: describe(result.description) });
            send({ op: 'a', sdp: result.description.sdp });
          } else answered = true;
        }
        if (frame.op === 'i') {
          const candidate = { candidate: frame.ice, sdpMid: frame.mid ?? null, ...(frame.mid == null ? { sdpMLineIndex: 0 } : {}) };
          if (remote) await native.call({ op: 'ice', peer: 'publish', candidate }); else pendingIce.push(candidate);
        }
      }).catch(error => { failure = error; });
    };
    await until(() => socket.readyState === WebSocket.OPEN, () => failure);
    send({ op: 'j', tk: identity.tk, v: 2 }); await until(() => joined, () => failure);
    // Normal source tags; microphone stays voice-scoped, video and its separate
    // audio are screen-scoped and require each recipient's actual Watch.
    send({ op: 'p', k: 's', t: 'fixed-native-video' });
    send({ op: 'p', k: 'sa', t: 'fixed-native-source-audio' });
    const offer = (await native.call({ op: 'offer', peer: 'publish' })).description;
    const publications = nativePublicationIdentity(offer);
    negotiation.push({ at: Date.now(), direction: 'local-offer', description: describe(offer) });
    send({ op: 'o', sdp: offer.sdp }); await until(() => answered, () => failure);
    return { engine: 'current', user: identity.user, started_at: started,
      publication: { index: 0, peer: 'peer-0', user: identity.user,
        tracks: publications.map(source => ({ ...source, source_name: 'peer-0/' + source.role })) },
      check: () => { if (failure) throw failure; }, diagnostics: () => ({ trace, negotiation, pending_candidates: pendingIce.length }),
      bind: async members => {
        let status;
        const deadline = Date.now() + 30000;
        do {
          if (failure) throw failure;
          status = await native.call({ op: 'status' });
          const received = Object.values(status.peers.publish.received);
          if (received.some(edge => edge.error)) throw new Error('native current decoder failed');
          if (received.length === members.length && received.every(edge => edge.packets_received > 0)) break;
          await sleep(50);
        } while (Date.now() < deadline);
        const bindings = currentNativeMicrophoneBindings(status.peers.publish.received, members);
        for (const binding of bindings) await native.call({ op: 'bind', peer: 'publish', ssrc: binding.ssrc, source_name: binding.source_name });
        return bindings;
      }, close };
  } catch (error) { await rethrowAfterCleanup(error, close); }
}
