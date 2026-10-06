// Browser/Node shared direct-loopback evidence checks. Never fill absent stats.
import { nativePublicationIdentity } from './native-peer-checks.mjs';

export const QUALITY_COUNTERS = Object.freeze(['packetsLost', 'concealedSamples', 'silentConcealedSamples', 'packetsDiscarded', 'insertedSamplesForDeceleration', 'removedSamplesForAcceleration']);
export const RECEIVER_COUNTERS = Object.freeze(['packetsReceived', 'totalSamplesReceived', ...QUALITY_COUNTERS]);
const requireThat = (condition, message) => { if (!condition) throw Error(message); };

export function v2Publication(offer) {
  requireThat(offer?.sdp?.length <= 262144, 'bounded native offer required');
  const bindings = nativePublicationIdentity(offer).map(value => ({ ...value, role: value.role === 'screen-audio' ? 'source' : value.role, uid: value.role === 'mic' ? 0 : value.role === 'screen-audio' ? 64 : null }));
  const sections = offer.sdp.split(/(?=^m=)/m).filter(section => section.startsWith('m='));
  for (const [index, binding] of bindings.entries()) {
    const lines = sections[index].trim().split(/\r?\n/), map = binding.kind === 'audio' ? 'a=rtpmap:111 opus/48000/2' : 'a=rtpmap:96 VP8/90000';
    requireThat(binding.mid.length > 0 && !/\s/.test(binding.mid) && lines.includes(map) && !/^m=\w+ 0 /.test(lines[0]), 'native MID/negotiated source codec differs');
    requireThat(lines.filter(line => /^(?:a=sendrecv|a=sendonly)$/.test(line)).length === 1 && !lines.includes('a=inactive') && !lines.includes('a=recvonly'), 'native publication direction differs');
    const ssrcs = new Set(lines.filter(line => line.startsWith('a=ssrc:')).map(line => line.match(/^a=ssrc:(\d+) /)?.[1]));
    requireThat(ssrcs.size === 1 && ssrcs.has(String(binding.ssrc)), 'native publication has foreign/ambiguous SSRC');
  }
  return bindings;
}

export function holdTrack(event, bindings, held) {
  const binding = bindings.find(value => value.mid === event.transceiver?.mid);
  requireThat(binding && !held.has(binding.role) && event.track?.id === binding.track_id && event.track.kind === binding.kind && event.receiver?.track === event.track && event.transceiver.receiver === event.receiver && event.streams?.length === 1 && event.streams[0].id === binding.stream_id, 'actual held receiver MID/MSID/track binding differs');
  requireThat(event.track.readyState === 'live' && event.track.enabled === true, 'actual receiver track must be live/enabled');
  const value = { ...binding, receiver: event.receiver, transceiver: event.transceiver, track: event.track, stream: event.streams[0] }; held.set(binding.role, value); return value;
}

export function validateHeldObjects(pc, held) {
  const receivers = pc.getReceivers(), transceivers = pc.getTransceivers();
  requireThat(pc.connectionState === 'connected' && held.size === 3 && receivers.length === 3 && new Set(receivers).size === 3 && transceivers.length === 3 && new Set(transceivers).size === 3 && pc.getSenders().every(sender => sender.track === null), 'complete connected native three-track receiver graph without browser publication required before replay');
  for (const binding of held.values()) requireThat(receivers.includes(binding.receiver) && transceivers.includes(binding.transceiver) && ['recvonly', 'sendrecv'].includes(binding.transceiver.currentDirection) && binding.receiver.track === binding.track && binding.transceiver.receiver === binding.receiver && binding.transceiver.mid === binding.mid && binding.track.id === binding.track_id && binding.track.kind === binding.kind && binding.track.readyState === 'live' && binding.track.enabled === true, 'actual held receiver object/track/MID changed or disabled');
}

export function inboundSnapshot(binding, stats, { initial = false, previous } = {}) {
  requireThat(Array.isArray(stats), 'actual receiver stats must be retained');
  const rows = stats.filter(row => row.type === 'inbound-rtp' && row.kind === 'audio');
  requireThat(rows.length === 1, 'genuine pre-replay inbound report unavailable or ambiguous');
  const row = rows[0], codecs = stats.filter(value => value.type === 'codec' && value.id === row.codecId);
  requireThat(typeof row.id === 'string' && row.id.trim().length > 0 && row.ssrc === binding.ssrc && row.mid === binding.mid && row.trackIdentifier === binding.track_id && codecs.length === 1 && codecs[0].mimeType?.toLowerCase() === 'audio/opus' && codecs[0].clockRate === 48000 && codecs[0].channels === 2, 'actual held inbound report MID/SSRC/track/codec differs');
  requireThat(Number.isFinite(row.timestamp) && row.timestamp > 0, 'actual receiver timestamp unavailable');
  for (const field of RECEIVER_COUNTERS) {
    requireThat(Number.isSafeInteger(row[field]) && row[field] >= 0, 'actual receiver counter unavailable/malformed: ' + field);
    requireThat(!initial || row[field] === 0, 'genuine initial receiver counters must be zero: ' + field);
    requireThat(!QUALITY_COUNTERS.includes(field) || row[field] === 0, 'actual receiver loss/PLC/discard/stretch: ' + field);
    if (previous) requireThat(row[field] >= previous.row[field], 'actual receiver counter regressed: ' + field);
  }
  if (previous) requireThat(row.id === previous.row.id && row.codecId === previous.row.codecId && row.timestamp >= previous.row.timestamp, 'actual held report/codec identity or time changed');
  return { role: binding.role, uid: binding.uid, mid: binding.mid, trackId: binding.track_id, streamId: binding.stream_id, row: { ...row }, codec: { ...codecs[0] }, stats };
}

export function receiverEvidence(initial, final) {
  requireThat(initial.role === final.role && initial.uid === final.uid && initial.mid === final.mid && initial.trackId === final.trackId && initial.streamId === final.streamId && initial.row.id === final.row.id && initial.row.codecId === final.row.codecId, 'actual initial/final held receiver identity differs');
  requireThat(initial.row.ssrc === final.row.ssrc && initial.codec.mimeType.toLowerCase() === 'audio/opus' && final.codec.mimeType.toLowerCase() === 'audio/opus' && initial.codec.clockRate === 48000 && final.codec.clockRate === 48000, 'actual initial/final receiver codec/SSRC differs');
  const evidence = { uid: final.uid, role: final.role, ssrc: final.row.ssrc, mid: final.mid, trackId: final.trackId, streamId: final.streamId, codecId: final.row.codecId, identityStable: true, live: true, enabled: true, codec: final.codec.mimeType.toLowerCase(), decodedSamplesProgress: final.row.totalSamplesReceived > initial.row.totalSamplesReceived, packetsProgress: final.row.packetsReceived > initial.row.packetsReceived, initialInboundReportId: initial.row.id, inboundReportId: final.row.id };
  for (const field of RECEIVER_COUNTERS) { evidence[field] = final.row[field]; evidence['initial' + field[0].toUpperCase() + field.slice(1)] = initial.row[field]; }
  return evidence;
}
