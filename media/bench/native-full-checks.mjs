// Each observed edge is bound by real signaling/SSRC/MID evidence. Counts alone
// cannot qualify a graph with duplicate or misplaced receiver sources.
const role = source => source === 'video' || source.endsWith('/video') ? 'video' : 'audio';
export function nativeFullTopology(peers) {
  if (!Number.isInteger(peers) || peers < 2 || peers > 32) throw new Error('N2..32 required');
  const participants = Array.from({ length: peers }, (_, index) => ({ peer: 'peer-' + index, implementation: index ? 'browser' : 'native' }));
  const edges = [];
  for (const receiver of participants) {
    for (const publisher of participants) if (publisher.peer !== receiver.peer) edges.push({ receiver: receiver.peer, source: publisher.peer + '/mic', kind: 'audio' });
    if (receiver.peer !== 'peer-0') for (const name of ['screen-audio', 'video']) edges.push({ receiver: receiver.peer, source: 'peer-0/' + name, kind: role(name) });
  }
  return { participants, edges, audio_edges: peers * peers - 1, video_edges: peers - 1, video_watchers: peers - 1,
    video_watch_policy: 'every browser participant1..N-1 watches peer0; native peer0 does not decode its own video',
    source_publishers: 1, voice_participants: peers, comparison_available: false };
}
const key = (receiver, source) => receiver + '<-' + source;
const finite = number => typeof number === 'number' && Number.isFinite(number);
const quantiles = values => {
  const sorted = [...values].sort((a, b) => a - b);
  return { count: sorted.length, min: sorted[0] ?? null, max: sorted.at(-1) ?? null,
    median: sorted.length ? sorted[Math.floor(sorted.length / 2)] : null, values: sorted };
};
function actualBrowserBinding(row) {
  const binding = row._binding;
  if (!binding || row._source !== binding.source_name || row.kind !== binding.kind || !binding.binding_basis) return false;
  if (binding.producer_id) return row.ssrc === binding.ssrc && !!binding.consumer_id && !!binding.track_id;
  if (binding.feed_id !== undefined) return String(row.mid) === binding.mid && !!binding.track_id && binding.feed_mid !== undefined;
  return row.trackIdentifier === binding.track_id && !!binding.owner_user && !!binding.stream_id;
}
const canonicalNs = value => typeof value === 'string' && /^[1-9][0-9]*$/.test(value);
const nsSeconds = value => canonicalNs(value) ? Number(BigInt(value)) / 1e9 : NaN;
function counterDelta(rows, field, signed = false) {
  if (rows.some(row => !finite(row[field]) || (!signed && row[field] < 0))) return { delta: null, unavailable: 'missing/nonfinite/negative counter' };
  if (!signed && rows.some((row, index) => index && row[field] < rows[index - 1][field])) return { delta: null, unavailable: 'counter reset during measurement' };
  return { delta: rows.at(-1)[field] - rows[0][field], unavailable: null };
}
// Preserve actual quality even when a strict historical pilot fails. Browser
// sample totals include concealment; native decoded_samples has no playout PLC.
function edgeQuality(rows, edge) {
  const native = rows[0]._native, seconds = rows.at(-1)._seconds - rows[0]._seconds, unavailable = [];
  const fields = native ? ['packets_received', 'payload_bytes_received', 'decoded_samples', 'sequence_gaps', 'timestamp_gaps', 'reordered_or_duplicate_packets', 'decode_errors']
    : ['packetsReceived', 'bytesReceived', 'packetsLost', 'packetsDiscarded', 'jitterBufferDelay', 'jitterBufferTargetDelay', 'jitterBufferMinimumDelay', 'jitterBufferEmittedCount',
      ...(edge.kind === 'audio' ? ['totalSamplesReceived', 'concealedSamples', 'silentConcealedSamples', 'concealmentEvents', 'insertedSamplesForDeceleration', 'removedSamplesForAcceleration']
        : ['framesDecoded', 'framesDropped', 'freezeCount', 'pauseCount', 'totalFreezesDuration', 'totalPausesDuration'])];
  const counters = Object.fromEntries(fields.map(field => [field, counterDelta(rows, field, field === 'packetsLost')]));
  for (const [field, value] of Object.entries(counters)) if (value.unavailable) unavailable.push(field + ': ' + value.unavailable);
  if (!(seconds > 0) || rows.some((row, index) => index && !(row._seconds > rows[index - 1]._seconds))) unavailable.push('invalid measurement clock');
  const packets = counters[native ? 'packets_received' : 'packetsReceived'].delta;
  const decoded = counters[native ? 'decoded_samples' : edge.kind === 'audio' ? 'totalSamplesReceived' : 'framesDecoded'].delta;
  const concealed = counters.concealedSamples?.delta;
  if (!native && edge.kind === 'audio' && (concealed > decoded || counters.silentConcealedSamples.delta > concealed)) unavailable.push('concealment subset counters disagree');
  const stalled = field => rows.flatMap((row, index) => index && finite(row[field]) && finite(rows[index - 1][field]) && row[field] === rows[index - 1][field]
    ? [{ interval: index - 1, seconds: row._seconds - rows[index - 1]._seconds }] : []);
  const emitted = counters.jitterBufferEmittedCount?.delta;
  const delay = field => finite(counters[field]?.delta) && emitted > 0 ? counters[field].delta / emitted : null;
  const result = { ...edge, receiver_implementation: native ? 'native' : 'browser', measured_seconds: seconds,
    counters, packet_stalls: stalled(native ? 'packets_received' : 'packetsReceived'),
    decoder_stalls: stalled(native ? 'decoded_samples' : edge.kind === 'audio' ? 'totalSamplesReceived' : 'framesDecoded'),
    sample_rate: edge.kind === 'audio' && finite(decoded) && seconds > 0 ? decoded / seconds : null,
    decoded_fps: edge.kind === 'video' && finite(decoded) && seconds > 0 ? decoded / seconds : null,
    packet_loss_fraction: !native && finite(counters.packetsLost.delta) && packets + counters.packetsLost.delta > 0 ? counters.packetsLost.delta / (packets + counters.packetsLost.delta) : null,
    jitter_buffer_mean_seconds: native ? null : { actual: delay('jitterBufferDelay'), target: delay('jitterBufferTargetDelay'), minimum: delay('jitterBufferMinimumDelay') },
    scope: native ? 'actual packet order and libopus decode; no browser jitter buffer/playout PLC or latency' : 'actual receiver interval deltas; sample_rate includes concealed samples; no end-to-end latency',
    complete: unavailable.length === 0, unavailable };
  if (!native && edge.kind === 'audio') result.nonconcealed_sample_rate = finite(decoded) && finite(concealed) && seconds > 0 ? (decoded - concealed) / seconds : null;
  if (!native && !(emitted > 0)) { result.complete = false; unavailable.push('jitter buffer emitted no samples/frames'); }
  return result;
}
function nativeSenderCodec(sample, row, kind) {
  const peer = sample.native.peers.publish, candidates = peer.negotiated_senders?.filter(sender => sender.mid === row.mid && sender.encodings?.some(encoding => encoding.ssrc === row.ssrc));
  if (candidates?.length !== 1) return null;
  const sender = candidates[0], source = row.ssrc === 0x474d4943 ? ['m', 'fixed-native-mic'] : row.ssrc === 0x47565038 ? ['s', 'fixed-native-video'] : ['s', 'fixed-native-source-audio'];
  if (!sender.basis || sender.stream_id !== source[0] || sender.track_id !== source[1] || sender.encodings.length !== 1
      || sender.encodings[0].active !== true
      || sender.encodings[0].mimeType?.toLowerCase() !== (kind === 'audio' ? 'audio/opus' : 'video/vp8') || sender.encodings[0].clockRate !== (kind === 'audio' ? 48000 : 90000) || sender.codecs?.length !== 1) return null;
  const negotiated = sender.codecs[0];
  // rtc0.20.5's CodecStats matcher compares fmtp strings literally. Janus's
  // compatible shorter Opus fmtp can leave codecId empty. Every engine must
  // instead prove the actual negotiated RtpSender parameters/MID/SSRC. If
  // CodecStats exists, it must agree with that independent parameter evidence.
  if (row.codecId) {
    const observed = peer.codecs?.filter(codec => codec.id === row.codecId);
    if (observed?.length !== 1 || ['mimeType', 'clockRate', 'payloadType'].some(field => observed[0][field] !== negotiated[field])) return null;
  }
  return negotiated;
}
export function nativeFullGraph(samples, topology, nativeBindings, receiverPeer, expectedVideoBitrate, policy) {
  const failures = [], sourceFailures = [], expected = new Map(topology.edges.map(edge => [key(edge.receiver, edge.source), edge])), observed = [];
  const sourceFailure = message => { sourceFailures.push(message); failures.push(message); };
  if (!Array.isArray(samples) || samples.length < 3) return { valid: false, failures: ['multiple measurement samples required'],
    source_graph: { valid: false, failures: ['multiple measurement samples required'] }, quality: { complete: false, edges: [] }, comparison_available: false };
  const connectionRoles = policy?.engine === 'current' ? ['media'] : ['send', 'recv'];
  try {
    const canonical = nativeFullTopology(topology.voice_participants);
    if (JSON.stringify(topology.participants) !== JSON.stringify(canonical.participants) || topology.edges.length !== canonical.edges.length
        || canonical.edges.some(edge => expected.get(key(edge.receiver, edge.source))?.kind !== edge.kind)) sourceFailure('canonical full-N topology differs');
  } catch { sourceFailure('canonical full-N topology missing'); }
  if (!['current', 'mediasoup', 'janus'].includes(policy?.engine) || !finite(policy?.requestedSeconds) || policy.requestedSeconds < 10) sourceFailure('explicit engine and measurement-duration policy required');
  if (!finite(expectedVideoBitrate) || expectedVideoBitrate <= 0) sourceFailure('explicit fixed video source bitrate required');
  const browserPeers = topology.participants.filter(peer => peer.peer !== 'peer-0').map(peer => peer.peer);
  const firstAnchor = samples[0].native?.sources?.mic?.timeline?.startClockNs;
  if (!Array.isArray(nativeBindings) || nativeBindings.length !== topology.voice_participants - 1 || new Set(nativeBindings.map(value => value.ssrc)).size !== nativeBindings.length || new Set(nativeBindings.map(value => value.source_name)).size !== nativeBindings.length) sourceFailure('native actual microphone bindings incomplete or duplicate');
  for (const sample of samples) {
    const rows = new Map(), stats = sample.browser.stats, codecs = new Map(stats.filter(row => row.type === 'codec').map(row => [row._endpoint + '/' + row.id, row]));
    const connections = sample.browser.connections;
    if (sample.browser.failures?.length || !Array.isArray(connections) || connections.length !== browserPeers.length || new Set(connections?.map(value => value.peer)).size !== browserPeers.length
        || connections?.some(peer => !browserPeers.includes(peer.peer) || !Array.isArray(peer.states) || peer.states.length !== connectionRoles.length
          || new Set(peer.states.map(value => value.label)).size !== connectionRoles.length || peer.states.some(connection => !connectionRoles.includes(connection.label) || connection.state !== 'connected'))) sourceFailure('exact browser peer/transport inventory missing or connection changed');
    for (const row of stats.filter(row => row.type === 'inbound-rtp' && row.packetsReceived > 0 && row.mid !== 'probator' && codecs.get(row._endpoint + '/' + row.codecId)?.mimeType?.toLowerCase() !== 'video/rtx')) {
      const id = key(row._peer, row._source), edge = expected.get(id), codec = codecs.get(row._endpoint + '/' + row.codecId);
      if (!edge || rows.has(id) || edge.kind !== row.kind || !actualBrowserBinding(row) || codec?.mimeType?.toLowerCase() !== (row.kind === 'audio' ? 'audio/opus' : 'video/vp8') || codec?.clockRate !== (row.kind === 'audio' ? 48000 : 90000)) sourceFailure('browser actual ownership/codec graph missing, duplicate or unknown: ' + id);
      rows.set(id, { ...row, _seconds: row.timestamp / 1000, _native: false });
    }
    const native = sample.native.peers[receiverPeer];
    if (!native || native.connection !== 'connected' || sample.native.peers.publish?.connection !== 'connected'
        || new Set(Object.keys(sample.native.peers)).size !== (receiverPeer === 'publish' ? 1 : 2)) sourceFailure('native publish/receive connection inventory missing/unstable');
    const activeNativeSenders = sample.native.peers.publish?.outbound.filter(row => row.packetsSent > 0);
    if (activeNativeSenders?.length !== 3 || activeNativeSenders.some(row => ![0x474d4943, 0x47565038, 0x47534130].includes(row.ssrc))) sourceFailure('native actual three-source sender inventory differs');
    for (const row of Object.values(native?.received ?? {})) {
      const id = key('peer-0', row.source_name), binding = nativeBindings?.find(value => value.ssrc === row.ssrc && value.source_name === row.source_name);
      if (!binding || !row.track_id || !row.stream_ids?.length || !expected.has(id) || rows.has(id) || !row.packets_received || row.error
          || row.codec?.mimeType?.toLowerCase() !== 'audio/opus' || row.codec.clockRate !== 48000 || !Number.isInteger(row.codec.payloadType)) sourceFailure('native actual SSRC/source/codec graph missing, duplicate or unknown: ' + id);
      rows.set(id, { ...row, _native: true, _seconds: nsSeconds(sample.native.monoNs) });
    }
    if (rows.size !== expected.size || [...expected.keys()].some(id => !rows.has(id))) sourceFailure('actual graph does not cover every expected receiver/source edge');
    const sources = sample.native.sources;
    if (!sources || Object.keys(sources).sort().join(',') !== 'mic,source,video') sourceFailure('native exact source inventory differs');
    if (sample.native.clock !== 'CLOCK_MONOTONIC' || !canonicalNs(sample.native.monoNs) || !sources || ['mic', 'source', 'video'].some(name => {
      const row = sources[name], timeline = row?.timeline;
      return row?.source_policy_valid !== true || row.error || timeline?.clock !== 'CLOCK_MONOTONIC' || timeline.pcm_latency_calibrated !== false
        || !canonicalNs(timeline.startClockNs) || BigInt(timeline.startClockNs) >= BigInt(sample.native.monoNs)
        || !Number.isInteger(timeline.conversionBracketNs) || timeline.conversionBracketNs < 0 || timeline.conversionBracketNs > 100000
        || timeline.startClockNs !== sources.mic?.timeline?.startClockNs || timeline.startClockNs !== firstAnchor;
    })) sourceFailure('native common-clock source schedule failed');
    observed.push(rows);
  }
  const streams = [], qualityEdges = [];
  for (const [id, edge] of expected) {
    const rows = observed.map(sample => sample.get(id)); if (rows.some(row => !row)) continue;
    const first = rows[0], last = rows.at(-1), seconds = last._seconds - first._seconds;
    const packetField = first._native ? 'packets_received' : 'packetsReceived', bytesField = first._native ? 'payload_bytes_received' : 'bytesReceived';
    const pcmField = first._native ? 'decoded_samples' : 'totalSamplesReceived';
    const bitrate = (last[bytesField] - first[bytesField]) * 8 / seconds;
    const intervals = [];
    // Browser getStats and native status are collected sequentially; allow at
    // most 100 ms collection jitter, never a silently shortened measurement.
    if (!(seconds >= policy?.requestedSeconds - .1) || rows.some(row => (first._native ? row.ssrc !== first.ssrc || row.track_id !== first.track_id : row.id !== first.id || row.ssrc !== first.ssrc || row._endpoint !== first._endpoint || row.codecId !== first.codecId))) sourceFailure(id + ' decoder identity/clock changed or measurement too short');
    for (let index = 1; index < rows.length; index++) {
      const previous = rows[index - 1], current = rows[index], delta = current._seconds - previous._seconds;
      if (!(delta > 0) || [packetField, bytesField, ...(edge.kind === 'audio' ? [pcmField] : ['framesDecoded'])].some(field => !finite(previous[field]) || !finite(current[field]) || previous[field] < 0 || current[field] < previous[field])) sourceFailure(id + ' receiver clock/counters missing or reset');
      if (!(delta > 0) || !(current[packetField] > previous[packetField] && current[bytesField] > previous[bytesField])) failures.push(id + ' RTP stalled/counters changed');
      intervals.push((current[bytesField] - previous[bytesField]) * 8 / delta);
      if (edge.kind === 'audio') {
        if (!(current[pcmField] > previous[pcmField])) failures.push(id + ' PCM decoder stalled');
        const counters = first._native ? ['sequence_gaps', 'reordered_or_duplicate_packets', 'timestamp_gaps', 'decode_errors'] : ['packetsLost', 'concealedSamples'];
        if (counters.some(field => !finite(current[field]) || current[field] - previous[field] !== 0)) failures.push(id + ' decoder loss/conceal/order changed or missing');
      } else if (!(current.framesDecoded > previous.framesDecoded)) failures.push(id + ' video decoder stalled');
    }
    qualityEdges.push(edgeQuality(rows, edge));
    const stream = { ...edge, bitrate_bps: bitrate, measured_seconds: seconds, interval_bitrate_bps: quantiles(intervals), ssrc: first.ssrc,
      decoder_id: first.id ?? first.track_id, binding: first._binding ?? nativeBindings?.find(value => value.ssrc === first.ssrc) };
    if (edge.kind === 'audio') {
      stream.decoded_sample_rate = (last[pcmField] - first[pcmField]) / seconds;
      if (!finite(bitrate) || Math.abs(bitrate - 128000) > 12800 || !finite(stream.decoded_sample_rate) || Math.abs(stream.decoded_sample_rate - 48000) > 960) failures.push(id + ' actual audio rate/PCM source differs');
    } else {
      stream.decoded_fps = (last.framesDecoded - first.framesDecoded) / seconds;
      const lost = last.packetsLost - first.packetsLost, received = last.packetsReceived - first.packetsReceived;
      if (!finite(bitrate) || Math.abs(bitrate - expectedVideoBitrate) > expectedVideoBitrate * .1 || !finite(stream.decoded_fps) || Math.abs(stream.decoded_fps - 60) > 3 || rows.some(row => row.frameWidth !== 1920 || row.frameHeight !== 1080) || !finite(lost) || !(received > 0) || lost !== 0) failures.push(id + ' actual 1080p60/rate/loss differs');
    }
    streams.push(stream);
  }
  const quality = { complete: qualityEdges.length === expected.size && qualityEdges.every(edge => edge.complete), edges: qualityEdges,
    policy: 'observations, not a universal zero-loss/PLC product gate; compare every edge to a complete current-SFU baseline; PCM calibration remains strict', comparison_available: false };
  const senders = [];
  for (const peer of topology.participants) {
    const inputs = peer.peer === 'peer-0' ? [[0x474d4943, 'audio'], [0x47565038, 'video'], [0x47534130, 'audio']] : [[null, 'audio']];
    for (const [ssrc, kind] of inputs) {
      const rows = samples.map(sample => peer.peer === 'peer-0' ? sample.native.peers.publish.outbound.filter(row => row.ssrc === ssrc) : sample.browser.stats.filter(row => row._peer === peer.peer && row.type === 'outbound-rtp' && row.kind === 'audio' && row.packetsSent > 0));
      if (rows.some(rows => rows.length !== 1)) { sourceFailure(peer.peer + ' actual sender graph differs'); continue; }
      const values = rows.map(rows => rows[0]), first = values[0], last = values.at(-1);
      const times = samples.map((sample, index) => peer.peer === 'peer-0' ? nsSeconds(sample.native.monoNs) : values[index].timestamp / 1000);
      const seconds = times.at(-1) - times[0], bitrate = (last.bytesSent - first.bytesSent) * 8 / seconds, intervals = [];
      if (!(seconds >= policy?.requestedSeconds - .1) || values.some((row, index) => {
        const codec = peer.peer === 'peer-0' ? nativeSenderCodec(samples[index], row, kind)
          : samples[index].browser.stats.find(value => value._endpoint === row._endpoint && value.type === 'codec' && value.id === row.codecId);
        return row.kind !== kind || codec?.mimeType?.toLowerCase() !== (kind === 'audio' ? 'audio/opus' : 'video/vp8')
          || codec.clockRate !== (kind === 'audio' ? 48000 : 90000) || !Number.isInteger(codec.payloadType) || codec.payloadType < 0 || codec.payloadType > 127;
      })) sourceFailure(peer.peer + ' actual sender kind/codec differs or measurement too short');
      for (let index = 1; index < values.length; index++) {
        const previous = values[index - 1], value = values[index], delta = times[index] - times[index - 1];
        if (!(delta > 0) || [previous, value].some(row => !finite(row.packetsSent) || !finite(row.bytesSent) || row.packetsSent < 0 || row.bytesSent < 0)
            || value.ssrc !== first.ssrc || value.id !== first.id || !(value.packetsSent > previous.packetsSent && value.bytesSent > previous.bytesSent)) sourceFailure(peer.peer + ' actual sender stalled/changed');
        intervals.push((value.bytesSent - previous.bytesSent) * 8 / delta);
      }
      const target = kind === 'audio' ? 128000 : expectedVideoBitrate;
      if (!finite(bitrate) || Math.abs(bitrate - target) > .1 * target) sourceFailure(peer.peer + ' actual sender bitrate differs');
      const source = peer.peer + '/' + (peer.peer !== 'peer-0' || ssrc === 0x474d4943 ? 'mic' : kind === 'video' ? 'video' : 'screen-audio');
      senders.push({ peer: peer.peer, source, kind, ssrc: first.ssrc, bitrate_bps: bitrate, interval_bitrate_bps: quantiles(intervals) });
    }
  }
  return { valid: failures.length === 0 && streams.length === expected.size, failures: [...new Set(failures)], topology,
    source_graph: { valid: sourceFailures.length === 0 && streams.length === expected.size, failures: [...new Set(sourceFailures)],
      quality_data_complete: quality.complete, measurement_comparable: sourceFailures.length === 0 && streams.length === expected.size && quality.complete,
      source_clock: 'one unchanged native CLOCK_MONOTONIC anchor throughout every source/sample',
      scope: 'owned full graph, actual sender rates and source schedule; receiver quality is reported separately; cross-run archives/environment still require comparison' }, quality,
    measurement_policy: { ...policy, minimum_peer_span_seconds: policy?.requestedSeconds - .1, interval_bitrate_policy: 'per-stream observed interval distributions; mean source rate and complete duration are required; this pilot cannot qualify performance' },
    streams, senders, audio_received_bitrate_distribution: quantiles(streams.filter(row => row.kind === 'audio').map(row => row.bitrate_bps)),
    video_decoder_fps_distribution: quantiles(streams.filter(row => row.kind === 'video').map(row => row.decoded_fps)), comparison_available: false };
}
