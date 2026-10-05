// Strict same-namespace two-endpoint control. An SFU's rewritten SSRC/MID needs
// its actual signaling map; these native source SSRCs are only valid directly.
export const NATIVE_AUDIO_SOURCES = new Map([[0x474d4943, 'peer-0/mic'], [0x47534130, 'peer-0/screen-audio']]);

export function nativePublicationIdentity(offer) {
  if (offer?.type !== 'offer' || typeof offer.sdp !== 'string') throw new Error('actual native publication offer required');
  const sections = offer.sdp.split(/(?=^m=)/m).filter(section => section.startsWith('m='));
  const expected = [
    { role: 'mic', kind: 'audio', stream_id: 'm', track_id: 'fixed-native-mic', ssrc: 0x474d4943 },
    { role: 'video', kind: 'video', stream_id: 's', track_id: 'fixed-native-video', ssrc: 0x47565038 },
    { role: 'screen-audio', kind: 'audio', stream_id: 's', track_id: 'fixed-native-source-audio', ssrc: 0x47534130 }
  ];
  if (sections.length !== expected.length) throw new Error('requires exactly native microphone/video/source-audio publication');
  const mids = new Set();
  return sections.map((section, index) => {
    const value = expected[index], lines = section.trim().split(/\r?\n/);
    const mid = lines.filter(line => line.startsWith('a=mid:'));
    const msid = lines.filter(line => line.startsWith('a=msid:'));
    if (!lines[0].startsWith('m=' + value.kind + ' ') || mid.length !== 1 || mids.has(mid[0].slice(6)) ||
        msid.length !== 1 || msid[0] !== `a=msid:${value.stream_id} ${value.track_id}` ||
        !lines.includes(`a=ssrc:${value.ssrc} msid:${value.stream_id} ${value.track_id}`)) throw new Error('native actual SDP track/stream/MID/SSRC identity differs');
    mids.add(mid[0].slice(6)); return { ...value, mid: mid[0].slice(6), binding_basis: 'actual offer media MSID and matching SSRC MSID' };
  });
}

export function nativeBrowserAudio(samples) {
  const failures = [], roles = [], edges = [];
  if (!Array.isArray(samples) || samples.length < 2) return { valid: false, failures: ['audio requires multiple samples'], roles };
  for (const sample of samples) {
    if (sample.connection !== 'connected' || !Array.isArray(sample.errors) || sample.errors.length) failures.push('browser connection or collected stats errors during measurement');
    const codecs = new Map(sample.stats.filter(value => value.type === 'codec').map(value => [value.id, value]));
    const audio = sample.stats.filter(value => value.type === 'inbound-rtp' && value.kind === 'audio' && value.packetsReceived > 0);
    const sourceSet = new Set(audio.map(value => value.ssrc));
    if (audio.length !== 2 || sourceSet.size !== 2 || ![...NATIVE_AUDIO_SOURCES.keys()].every(ssrc => sourceSet.has(ssrc))) failures.push('requires exactly distinct microphone and screen-audio SSRCs in every sample');
    if (audio.some(value => codecs.get(value.codecId)?.mimeType?.toLowerCase() !== 'audio/opus' || codecs.get(value.codecId)?.clockRate !== 48000)) failures.push('actual browser audio decoder codec differs');
    edges.push(new Map(audio.map(value => [value.ssrc, value])));
  }
  for (const [ssrc, role] of NATIVE_AUDIO_SOURCES) {
    const rows = edges.map(edge => edge.get(ssrc));
    if (rows.some(row => !row)) continue;
    const first = rows[0], last = rows.at(-1), seconds = (last.timestamp - first.timestamp) / 1000;
    const bitrate = (last.bytesReceived - first.bytesReceived) * 8 / seconds;
    const sampleRate = (last.totalSamplesReceived - first.totalSamplesReceived) / seconds;
    const lost = last.packetsLost - first.packetsLost, concealed = last.concealedSamples - first.concealedSamples;
    if (!(seconds > 0) || !Number.isFinite(bitrate) || Math.abs(bitrate - 128000) > 12800 || !Number.isFinite(sampleRate) || Math.abs(sampleRate - 48000) > 960 || lost !== 0 || concealed !== 0) failures.push(role + ' measured decoder bitrate/samples/loss/concealment differs');
    if (rows.some(row => row.id !== first.id || row.codecId !== first.codecId)) failures.push(role + ' decoder identity changed');
    for (let index = 1; index < rows.length; index++) {
      const previous = rows[index - 1], current = rows[index];
      if (!(current.packetsReceived > previous.packetsReceived && current.bytesReceived > previous.bytesReceived && current.totalSamplesReceived > previous.totalSamplesReceived)) failures.push(role + ' decoder/RTP stalled');
      if (!(current.timestamp > previous.timestamp) || current.packetsLost - previous.packetsLost !== 0 || current.concealedSamples - previous.concealedSamples !== 0) failures.push(role + ' sample clock/loss/concealment differs within measurement');
    }
    roles.push({ role, decoder_id: first.id, codec_id: first.codecId, ssrc, bitrate_bps: bitrate,
      decoded_sample_rate: sampleRate, lost_packets: lost, concealed_samples: concealed, measured_seconds: seconds });
  }
  return { valid: failures.length === 0 && roles.length === 2, failures, roles };
}
