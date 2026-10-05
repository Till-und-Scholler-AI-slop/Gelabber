// Ordinary three-track native peer0 publication using the pinned official SDP
// helpers. The single-video diagnostic bridge remains independently frozen.
import { createRequire } from 'node:module';
import * as ortc from 'mediasoup-client/ortc';
import { RemoteSdp } from 'mediasoup-client/handlers/sdp/RemoteSdp';
import { extractRtpCapabilities, extractDtlsParameters, getCname } from 'mediasoup-client/handlers/sdp/commonUtils';
import { getRtpEncodings } from 'mediasoup-client/handlers/sdp/unifiedPlanUtils';
import { nativePublicationIdentity } from './native-peer-checks.mjs';
const sdp = createRequire(import.meta.resolve('mediasoup-client'))('sdp-transform');
export const NATIVE_PUBLICATIONS = Object.freeze([
  Object.freeze({ role: 'mic', kind: 'audio', ssrc: 0x474d4943, track: 'fixed-native-mic' }),
  Object.freeze({ role: 'video', kind: 'video', ssrc: 0x47565038, track: 'fixed-native-video' }),
  Object.freeze({ role: 'screen-audio', kind: 'audio', ssrc: 0x47534130, track: 'fixed-native-source-audio' })
]);

function remoteTransport(transport) {
  if (!transport?.iceParameters?.iceLite || !transport.iceCandidates?.length) throw new Error('normal ICE-lite WebRtcTransport required');
  const fingerprint = transport.dtlsParameters?.fingerprints?.find(value => value.algorithm === 'sha-256');
  if (!fingerprint) throw new Error('actual advertised worker SHA-256 fingerprint required');
  return new RemoteSdp(structuredClone({ iceParameters: transport.iceParameters, iceCandidates: transport.iceCandidates,
    dtlsParameters: { ...transport.dtlsParameters, fingerprints: [fingerprint] } }));
}

export function mediasoupNativePeerPublish(offer, routerCapabilities, transport) {
  nativePublicationIdentity(offer);
  const parsed = sdp.parse(offer.sdp), active = parsed.media.filter(media => media.port !== 0);
  if (active.length !== 3 || new Set(active.map(media => String(media.mid))).size !== 3) throw new Error('exactly three distinct native microphone/video/source-audio MIDs required');
  const extended = ortc.getExtendedRtpCapabilities(extractRtpCapabilities({ sdpObject: parsed }), structuredClone(routerCapabilities));
  const dtlsParameters = extractDtlsParameters({ sdpObject: parsed });
  if (dtlsParameters.role !== 'auto') throw new Error('native peer0 must offer setup:actpass');
  dtlsParameters.role = 'server';
  const remote = remoteTransport(transport); remote.updateDtlsRole('client');
  const publications = active.map((media, index) => {
    const expected = NATIVE_PUBLICATIONS[index];
    if (media.type !== expected.kind || !['sendonly', 'sendrecv'].includes(media.direction)) throw new Error('native source kind/order/direction differs');
    const parameters = ortc.getSendingRtpParameters(expected.kind, extended), answerParameters = ortc.getSendingRemoteRtpParameters(expected.kind, extended);
    const codec = parameters.codecs[0];
    if (parameters.codecs.length !== 1 || parameters.headerExtensions.length || codec.mimeType.toLowerCase() !== (expected.kind === 'video' ? 'video/vp8' : 'audio/opus') || codec.clockRate !== (expected.kind === 'video' ? 90000 : 48000) || (expected.kind === 'audio' && codec.channels !== 2)) throw new Error('requires fixed VP8/Opus without extra codecs or extensions');
    parameters.mid = String(media.mid); parameters.encodings = getRtpEncodings({ offerMediaObject: media, codecs: parameters.codecs });
    if (parameters.encodings.length !== 1 || parameters.encodings[0].rtx || parameters.encodings[0].ssrc !== expected.ssrc) throw new Error('native single-encoding source SSRC differs');
    const trackIds = new Set((media.ssrcs ?? []).filter(value => value.attribute === 'msid').map(value => value.value.split(' ')[1]));
    if (trackIds.size !== 1 || !trackIds.has(expected.track)) throw new Error('actual native SDP track identity differs');
    parameters.rtcp.cname = getCname({ offerMediaObject: media }); parameters.rtcp.reducedSize = Boolean(media.rtcpRsize);
    if (!parameters.rtcp.cname) throw new Error('native CNAME missing');
    remote.send({ offerMediaObject: media, offerRtpParameters: parameters, answerRtpParameters: answerParameters });
    return { ...expected, mid: parameters.mid, rtpParameters: parameters };
  });
  return { description: { type: 'answer', sdp: remote.getSdp() }, dtlsParameters, publications,
    receiverCapabilities: ortc.getRecvRtpCapabilities(extended), comparison_available: false };
}

// Each consumer originates from an actually returned browser producer id and
// carries the worker's actual receive SSRC. Never assign an owner from SSRC order.
export function mediasoupNativePeerReceive(consumers, transport) {
  if (!Array.isArray(consumers) || consumers.length < 1 || consumers.length > 31) throw new Error('one microphone consumer per other native peer0 participant required');
  const owners = new Set(), producerIds = new Set(), ssrcs = new Set();
  const remote = remoteTransport(transport), bindings = [];
  consumers.forEach((consumer, index) => {
    const owner = consumer.owner, parameters = consumer.rtpParameters, codec = parameters?.codecs?.[0], encoding = parameters?.encodings?.[0];
    if (!/^peer-(?:[1-9]|[12][0-9]|3[01])\/mic$/.test(owner) || owners.has(owner) || !consumer.producerId || producerIds.has(consumer.producerId)) throw new Error('duplicate or invalid actual microphone producer ownership');
    if (consumer.kind !== 'audio' || !consumer.id || parameters?.codecs?.length !== 1 || codec?.mimeType?.toLowerCase() !== 'audio/opus' || codec.clockRate !== 48000 || codec.channels !== 2 || parameters?.encodings?.length !== 1 || !Number.isInteger(encoding?.ssrc) || encoding.ssrc <= 0 || encoding.ssrc > 0xffffffff || ssrcs.has(encoding.ssrc)) throw new Error('native microphone consumer codec/actual SSRC differs');
    owners.add(owner); producerIds.add(consumer.producerId); ssrcs.add(encoding.ssrc);
    const mid = String(index); bindings.push({ source_name: owner, producer_id: consumer.producerId, consumer_id: consumer.id, ssrc: encoding.ssrc, mid });
    remote.receive({ mid, kind: 'audio', offerRtpParameters: parameters, streamId: owner, trackId: consumer.id });
  });
  return { description: { type: 'offer', sdp: remote.getSdp() }, bindings, comparison_available: false };
}

export function mediasoupNativeReceiveDtls(answer) {
  if (answer?.type !== 'answer') throw new Error('actual native receive answer required');
  const parameters = extractDtlsParameters({ sdpObject: sdp.parse(answer.sdp) });
  if (!['client', 'server'].includes(parameters.role)) throw new Error('native receive DTLS role must be negotiated');
  return parameters;
}
