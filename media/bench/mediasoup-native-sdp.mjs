// Offline ORTC/SDP bridge for the bounded one-track native diagnostic source.
// Uses the already pinned official client helpers; no Plain/Direct transport.
import { createRequire } from 'node:module';
import * as ortc from 'mediasoup-client/ortc';
import { RemoteSdp } from 'mediasoup-client/handlers/sdp/RemoteSdp';
import { extractRtpCapabilities, extractDtlsParameters, getCname } from 'mediasoup-client/handlers/sdp/commonUtils';
import { getRtpEncodings } from 'mediasoup-client/handlers/sdp/unifiedPlanUtils';

const requireClient = createRequire(import.meta.resolve('mediasoup-client'));
const sdp = requireClient('sdp-transform');

export function mediasoupNativeAnswer(offer, routerCapabilities, transport) {
  if (offer?.type !== 'offer' || typeof offer.sdp !== 'string') throw new Error('native SDP offer required');
  const parsed = sdp.parse(offer.sdp);
  const active = parsed.media.filter(media => media.port !== 0);
  if (active.length !== 1 || active[0].type !== 'video' || !['sendonly', 'sendrecv'].includes(active[0].direction)) throw new Error('diagnostic bridge supports one video publisher, no extra tracks');
  if (!transport?.iceParameters?.iceLite || !transport.iceCandidates?.length || !transport.dtlsParameters?.fingerprints?.length) throw new Error('a normal mediasoup WebRtcTransport is required');
  const media = active[0];
  const extended = ortc.getExtendedRtpCapabilities(extractRtpCapabilities({ sdpObject: parsed }), structuredClone(routerCapabilities));
  const parameters = ortc.getSendingRtpParameters('video', extended);
  const remoteParameters = ortc.getSendingRemoteRtpParameters('video', extended);
  if (parameters.codecs.length !== 1 || parameters.codecs[0].mimeType.toLowerCase() !== 'video/vp8' || parameters.codecs[0].clockRate !== 90000 || parameters.headerExtensions.length) throw new Error('requires one VP8 encoding without RTP header extensions');
  parameters.mid = String(media.mid);
  parameters.encodings = getRtpEncodings({ offerMediaObject: media, codecs: parameters.codecs });
  if (parameters.encodings.length !== 1 || parameters.encodings[0].rtx || parameters.encodings[0].ssrc !== 0x47565038) throw new Error('native SSRC/single-encoding policy differs');
  parameters.rtcp.cname = getCname({ offerMediaObject: media });
  // The Rust API requires reducedSize explicitly; JS ORTC leaves it absent.
  // Match the actual native SDP rather than inventing browser defaults.
  parameters.rtcp.reducedSize = Boolean(media.rtcpRsize);
  if (!parameters.rtcp.cname) throw new Error('native RTP CNAME is missing');
  const dtlsParameters = extractDtlsParameters({ sdpObject: parsed });
  if (dtlsParameters.role !== 'auto') throw new Error('native source must offer setup:actpass');
  // rtc 0.20.5 verifies only SHA-256. Select a real advertised fingerprint
  // of the worker certificate; never rewrite or bypass certificate checks.
  const fingerprint = transport.dtlsParameters.fingerprints.find(value => value.algorithm === 'sha-256');
  if (!fingerprint) throw new Error('worker did not advertise a SHA-256 certificate fingerprint');
  dtlsParameters.role = 'server';
  const remote = new RemoteSdp(structuredClone({ iceParameters: transport.iceParameters, iceCandidates: transport.iceCandidates,
    dtlsParameters: { ...transport.dtlsParameters, fingerprints: [fingerprint] } }));
  remote.updateDtlsRole('client');
  remote.send({ offerMediaObject: media, offerRtpParameters: parameters, answerRtpParameters: remoteParameters });
  return { description: { type: 'answer', sdp: remote.getSdp() }, dtlsParameters, rtpParameters: parameters,
    policy: { client: 'mediasoup-client 3.24.1', transport: 'WebRtcTransport DTLS/SRTP', source_encodings: 1, kind: 'video', comparison_available: false } };
}
