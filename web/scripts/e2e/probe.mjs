/* global window, navigator, document, URL, DOMException, setInterval, clearInterval, setTimeout, clearTimeout */
// Observe native peers/sockets. Never retain tickets, cookies, SDP, ICE addresses,
// message bodies, captions, track IDs or DOM text in durable evidence.
export function instrument({ relay }) {
  const state = (window.__e2e = {
    peers: [],
    sockets: [],
    captures: [],
    micCalls: 0,
    displayCalls: 0,
    cameraCalls: 0,
    playRejected: 0,
    mediaElements: new Set(),
    heldTracks: [],
    incomingTracks: [],
    outgoingSources: new Map(),
    heldLiveClaims: [],
    rejectedSdp: 0,
    rejectedSdpErrors: [],
    renderedVideos: new WeakMap(),
    voiceRoster: [],
    voiceRosterSnapshots: 0,
    cancelNextCapture: false,
  });
  const Peer = window.RTCPeerConnection;
  window.RTCPeerConnection = class extends Peer {
    constructor(config) {
      super(relay ? { ...config, iceTransportPolicy: "relay" } : config);
      state.peers.push(this);
    }
    setRemoteDescription(description) {
      if (
        state.rejectNextVideoAnswer &&
        description?.type === "answer" &&
        this.getSenders().some(
          (s) => s.track?.kind === "video" && s.track.readyState === "live",
        )
      ) {
        state.rejectNextVideoAnswer = false;
        state.rejectedSdp++;
        // Let the real browser reject malformed SDP; never fake negotiated stats.
        return super
          .setRemoteDescription({
            type: "answer",
            sdp: "invalid native SDP rejection control",
          })
          .catch((error) => {
            state.rejectedSdpErrors.push({
              name:
                typeof error.name === "string" ? error.name.slice(0, 64) : null,
              errorDetail:
                typeof error.errorDetail === "string"
                  ? error.errorDetail.slice(0, 64)
                  : null,
              isError: error instanceof Error,
              isDOMException:
                typeof DOMException !== "undefined" &&
                error instanceof DOMException,
            });
            if (state.rejectedSdpErrors.length > 4)
              state.rejectedSdpErrors.shift();
            throw error;
          });
      }
      return super.setRemoteDescription(description);
    }
  };
  // Source identity comes from the authenticated Consumer, not browser MSID.
  window.addEventListener?.("gelabber:media-consumer", ({ detail }) => {
    const {
      receiver,
      track,
      owner,
      k,
      consumerId,
      generation,
      producerId,
      epoch,
      rtpParameters,
    } = detail;
    const pc = state.peers.find((peer) =>
      peer.getReceivers().includes(receiver),
    );
    if (!pc || receiver.track !== track)
      throw new Error("E2E_CONSUMER_BINDING_UNAVAILABLE");
    state.incomingTracks.push({
      pc,
      track,
      receiver,
      publisher: owner,
      sourceKind: k,
      consumerId,
      generation,
      producerId,
      epoch,
      rtpParameters,
    });
  });
  window.addEventListener?.("gelabber:media-layers", ({ detail }) => {
    state.layers ??= new Map();
    state.layers.set(detail.consumerId, { ...detail });
  });
  window.addEventListener?.("gelabber:media-producer", ({ detail }) => {
    const pc = state.peers.find((peer) =>
      peer.getSenders().includes(detail.sender),
    );
    state.outgoingSources.set(detail.k, { ...detail, pc });
  });
  const Socket = window.WebSocket;
  state.NativeSocket = Socket;
  window.WebSocket = class extends Socket {
    constructor(url, protocols) {
      super(url, protocols);
      const path = new URL(url, window.location.href).pathname;
      const item = {
        ws: this,
        // Vite HMR and any other socket must never be a media fault target.
        plane:
          path === "/ws" ? "gateway" : path === "/media/ws" ? "media" : "other",
        offers: 0,
        maxSdp: 0,
        receivedEvents: 0,
        gaps: 0,
        resyncs: 0,
        dmDiscoveries: 0,
        topics: new Map(),
        liveOn: 0,
        liveOff: 0,
        pendingMedia: new Map(),
        mediaRpc: [],
      };
      state.sockets.push(item);
      super.addEventListener("message", (event) => {
        try {
          const frame = JSON.parse(event.data);
          if (item.plane === "media" && ["result", "err"].includes(frame.op)) {
            const request = item.pendingMedia.get(frame.id);
            if (request) {
              request.status = frame.op === "result" ? "PASS" : "FAIL";
              request.error =
                frame.op === "err" && typeof frame.e === "string"
                  ? /^[a-z_]{1,64}$/.test(frame.e)
                    ? frame.e
                    : "unclassified"
                  : null;
              item.pendingMedia.delete(frame.id);
            }
          }
          if (
            frame.op === "sig" &&
            frame.t === "r" &&
            Array.isArray(frame.snap)
          ) {
            state.voiceRoster = frame.snap;
            state.voiceRosterSnapshots++;
          }
          if (frame.op === "gap") item.gaps++;
          if (frame.op === "resync") item.resyncs++;
          if (frame.op === "dm") item.dmDiscoveries++;
          if (frame.op === "e") {
            item.receivedEvents++;
          }
          if (["ok", "e", "gap"].includes(frame.op)) {
            if (typeof frame.c === "string" && Number.isSafeInteger(frame.n)) {
              const previous = item.topics.get(frame.c);
              item.topics.set(frame.c, {
                n: frame.n,
                ep: frame.ep,
                epochChanges:
                  (previous?.epochChanges ?? 0) +
                  (previous && previous.ep !== frame.ep ? 1 : 0),
              });
            }
          }
        } catch {
          /* Non-JSON is not evidence. */
        }
      });
      // Delay authoritative Consumer announcements to exercise out-of-order
      // arrival. Suppress local cleanup only for independently held receivers.
      this.addEventListener = (type, listener, options) => {
        if (type !== "message" || typeof listener !== "function")
          return super.addEventListener(type, listener, options);
        return super.addEventListener(
          type,
          (event) => {
            let frame;
            try {
              frame = JSON.parse(event.data);
            } catch {
              /* delegate below */
            }
            if (
              item.plane === "media" &&
              state.holdMediaCleanup &&
              [
                "consumerClosed",
                "consumerState",
                "producerClosed",
                "err",
              ].includes(frame?.op)
            )
              return;
            if (
              item.plane === "media" &&
              state.holdTracks &&
              frame?.op === "consumer"
            ) {
              state.heldTracks.push({
                event,
                kind: frame.kind,
                deliver: () => listener.call(this, event),
              });
              return;
            }
            listener.call(this, event);
          },
          options,
        );
      };
      const send = this.send.bind(this);
      this.send = (data) => {
        try {
          const frame = JSON.parse(data);
          if (item.plane === "media" && Number.isSafeInteger(frame.id)) {
            const consumer = state.incomingTracks.find(
              (source) =>
                source.consumerId === frame.consumerId &&
                source.generation === frame.generation,
            );
            const request = {
              method: frame.op,
              kind: frame.k ?? consumer?.sourceKind ?? null,
              on: typeof frame.on === "boolean" ? frame.on : null,
              status: "pending",
              error: null,
            };
            item.pendingMedia.set(frame.id, request);
            if (item.pendingMedia.size > 64)
              item.pendingMedia.delete(item.pendingMedia.keys().next().value);
            item.mediaRpc.push(request);
            if (item.mediaRpc.length > 128) item.mediaRpc.shift();
          }
          if (typeof frame.sdp === "string") {
            item.offers++;
            item.maxSdp = Math.max(item.maxSdp, frame.sdp.length);
          }
          if (
            item.plane === "gateway" &&
            frame.op === "sig" &&
            frame.k === "l" &&
            frame.t === "p" &&
            state.holdLiveClaims
          ) {
            state.heldLiveClaims.push(() => send(data));
            return;
          }
          if (frame.op === "sig" && frame.k === "l") {
            if (frame.t === "p") item.liveOn++;
            else if (frame.t === "u") item.liveOff++;
          }
        } catch {
          /* Observe only known scalar metadata. */
        }
        return send(data);
      };
    }
  };
  const play = window.HTMLMediaElement.prototype.play;
  window.HTMLMediaElement.prototype.play = function (...args) {
    state.mediaElements.add(this);
    const result =
      state.rejectPlayback && this.srcObject
        ? Promise.reject(
            new DOMException("synthetic blocked play", "NotAllowedError"),
          )
        : play.apply(this, args);
    result?.catch(() => state.playRejected++);
    return result;
  };
  function canvasSource(kind, slot) {
    const canvas = document.createElement("canvas");
    canvas.width = 640;
    canvas.height = 360;
    const ctx = canvas.getContext("2d");
    let frame = 0;
    const colors =
      kind === "camera"
        ? [30, 60, 220]
        : slot % 2
          ? [220, 30, 30]
          : [30, 220, 30];
    const draw = () => {
      ctx.fillStyle = `rgb(${colors.join(",")})`;
      ctx.fillRect(0, 0, 640, 180);
      const shade = 15 + ((frame++ * 23) % 220);
      ctx.fillStyle = `rgb(${shade},${shade},${shade})`;
      ctx.fillRect(0, 180, 640, 180);
    };
    draw();
    const timer = setInterval(draw, 100);
    const stream = canvas.captureStream(10);
    const track = stream.getVideoTracks()[0];
    // MediaStreamTrack.stop() does not fire ended; release the synthetic timer too.
    const stop = track.stop.bind(track);
    track.stop = () => {
      clearInterval(timer);
      stop();
    };
    state.captures.push({ kind, slot, track, stream, colors });
    return stream;
  }
  const gum = navigator.mediaDevices.getUserMedia.bind(navigator.mediaDevices);
  navigator.mediaDevices.getUserMedia = async (constraints) => {
    if (constraints.audio) state.micCalls++;
    if (!constraints.video) return gum(constraints);
    state.cameraCalls++;
    const stream = canvasSource("camera", state.cameraCalls);
    if (constraints.audio) {
      const audio = await gum({ audio: constraints.audio });
      for (const track of audio.getTracks()) stream.addTrack(track);
    }
    return stream;
  };
  navigator.mediaDevices.getDisplayMedia = async () => {
    state.displayCalls++;
    if (state.cancelNextCapture) {
      state.cancelNextCapture = false;
      throw new DOMException(
        "synthetic picker cancellation",
        "NotAllowedError",
      );
    }
    return canvasSource("display", state.displayCalls);
  };
}
export async function samplePublicationFlow({ kind }) {
  const state = window.__e2e;
  const producer = state.outgoingSources?.get(kind);
  const counters = async (native, type, track, ssrcs) => {
    if (!native)
      return { available: false, rows: null, packets: null, frames: null };
    let report;
    try {
      report = await native.getStats();
    } catch {
      return { available: false, rows: null, packets: null, frames: null };
    }
    const rows = [...report.values()].filter((row) => {
      if (row.type !== type || (row.kind ?? row.mediaType) !== "video")
        return false;
      if (ssrcs) return ssrcs.has(row.ssrc);
      const identifier =
        row.trackIdentifier ?? report.get(row.mediaSourceId)?.trackIdentifier;
      return identifier === track.id;
    });
    return {
      available: rows.length > 0,
      rows: rows.length,
      packets:
        rows.length &&
        rows.every((r) =>
          Number.isFinite(
            type === "outbound-rtp" ? r.packetsSent : r.packetsReceived,
          ),
        )
          ? rows.reduce(
              (n, r) =>
                n +
                (type === "outbound-rtp" ? r.packetsSent : r.packetsReceived),
              0,
            )
          : null,
      frames:
        rows.length &&
        rows.every((r) =>
          Number.isFinite(
            type === "outbound-rtp" ? r.framesEncoded : r.framesDecoded,
          ),
        )
          ? rows.reduce(
              (n, r) =>
                n +
                (type === "outbound-rtp" ? r.framesEncoded : r.framesDecoded),
              0,
            )
          : null,
    };
  };
  return {
    producer: {
      present: !!producer,
      bound: !!(
        producer &&
        producer.pc?.getSenders().includes(producer.sender) &&
        producer.sender.track === producer.track
      ),
      connection: producer?.pc?.connectionState ?? null,
      trackLive: producer?.track?.readyState === "live",
      trackEnabled: producer?.track?.enabled ?? null,
      native: await counters(producer?.sender, "outbound-rtp", producer?.track),
    },
    consumers: await Promise.all(
      state.incomingTracks
        .filter(
          (source) =>
            source.sourceKind === kind &&
            source.pc.connectionState !== "closed",
        )
        .map(async (source) => ({
          bound:
            source.pc.getReceivers().includes(source.receiver) &&
            source.receiver.track === source.track,
          connection: source.pc.connectionState,
          trackLive: source.track.readyState === "live",
          identitiesBound: [
            source.producerId,
            source.consumerId,
            source.epoch,
            source.generation,
          ].every((value) => typeof value === "string" && value.length > 0),
          native: await counters(
            source.receiver,
            "inbound-rtp",
            source.track,
            new Set(
              source.rtpParameters?.encodings?.map((encoding) => encoding.ssrc),
            ),
          ),
        })),
    ),
    rpc: state.sockets
      .filter((socket) => socket.plane === "media")
      .flatMap((socket) => socket.mediaRpc ?? []),
  };
}
// Count actual authenticated Consumers, keeping SSRCs and native track identities
// in browser memory. SDK 3.24.1 also creates its own video probation receiver
// (trackId "probator", SSRC 1234); it is never a Gelabber publication.
export async function sampleVideoConsumers({ publisher, deadlineEpochMs }) {
  const state = window.__e2e;
  const result = {
    sources: 0,
    selectedLiveSources: 0,
    foreignSources: 0,
    sourceRtpRows: 0,
    probatorRtpRows: 0,
    unexpectedRtpRows: 0,
    invalidBindings: 0,
    identitiesBound: 0,
  };
  const readStats = async (native) => {
    if (Date.now() >= deadlineEpochMs)
      throw new Error("E2E_NATIVE_STATS_DEADLINE");
    let timer;
    try {
      const report = await Promise.race([
        native.getStats(),
        new Promise((_, reject) => {
          timer = setTimeout(
            () => reject(new Error("E2E_NATIVE_STATS_DEADLINE")),
            Math.max(1, deadlineEpochMs - Date.now()),
          );
        }),
      ]);
      if (Date.now() >= deadlineEpochMs)
        throw new Error("E2E_NATIVE_STATS_DEADLINE");
      return [...report.values()].filter(
        (row) =>
          row.type === "inbound-rtp" && (row.kind ?? row.mediaType) === "video",
      );
    } finally {
      clearTimeout(timer);
    }
  };
  for (const pc of state.peers) {
    if (pc.connectionState !== "connected") continue;
    const receivers = pc.getReceivers();
    const sources = [
      ...new Map(
        state.incomingTracks
          .filter(
            (s) =>
              s.pc === pc &&
              s.track.kind === "video" &&
              s.track.readyState === "live",
          )
          .map((s) => [s.receiver, s]),
      ).values(),
    ];
    const sourceSsrcs = new Set();
    for (const source of sources) {
      result.sources++;
      if (source.publisher === publisher && source.sourceKind === "l")
        result.selectedLiveSources++;
      else result.foreignSources++;
      const encodings = source.rtpParameters?.encodings;
      if (
        ![
          source.consumerId,
          source.producerId,
          source.epoch,
          source.generation,
        ].every(
          (value) =>
            typeof value === "string" &&
            value.length > 0 &&
            value.length <= 128,
        ) ||
        !receivers.includes(source.receiver) ||
        source.receiver.track !== source.track ||
        !Array.isArray(encodings) ||
        encodings.length !== 1 ||
        !Number.isInteger(encodings[0].ssrc) ||
        encodings[0].ssrc <= 0 ||
        encodings[0].ssrc === 1234 ||
        sourceSsrcs.has(encodings[0].ssrc)
      ) {
        result.invalidBindings++;
        continue;
      }
      result.identitiesBound++;
      const ssrc = encodings[0].ssrc;
      const rows = (await readStats(source.receiver)).filter(
        (row) => row.ssrc === ssrc,
      );
      if (
        rows.length !== 1 ||
        !Number.isFinite(rows[0].packetsReceived) ||
        rows[0].packetsReceived <= 0 ||
        !Number.isFinite(rows[0].framesDecoded) ||
        rows[0].framesDecoded <= 0
      )
        result.invalidBindings++;
      else sourceSsrcs.add(ssrc);
    }
    const probators = receivers.filter(
      (receiver) =>
        receiver.track?.kind === "video" &&
        receiver.track.readyState === "live" &&
        receiver.track.id === "probator" &&
        !sources.some((source) => source.receiver === receiver),
    );
    let nativeProbator = false;
    if (probators.length === 1)
      nativeProbator = (await readStats(probators[0])).some(
        (row) => row.ssrc === 1234,
      );
    for (const row of await readStats(pc)) {
      if (sourceSsrcs.has(row.ssrc)) result.sourceRtpRows++;
      else if (nativeProbator && row.ssrc === 1234) result.probatorRtpRows++;
      else result.unexpectedRtpRows++;
    }
  }
  return result;
}
// Bind native sender stats to actual fixture capture objects in browser memory.
// Persist fixture ordinals and RID labels, never native track identifiers or SDP.
export async function sampleVideoSenders({ deadlineEpochMs } = {}) {
  const state = window.__e2e;
  const captures = state.captures
    .map((capture, index) => ({ capture, index }))
    .filter(({ capture }) => capture.track.readyState === "live");
  const senders = [];
  for (const [peerIndex, peer] of state.peers.entries()) {
    if (peer.connectionState !== "connected") continue;
    for (const [senderIndex, sender] of peer.getSenders().entries()) {
      const track = sender.track;
      if (track?.kind !== "video" || track.readyState !== "live") continue;
      const capture = captures.find((item) => item.capture.track === track);
      if (deadlineEpochMs !== undefined && Date.now() >= deadlineEpochMs)
        throw new Error("E2E_NATIVE_STATS_DEADLINE");
      let timer;
      let report;
      try {
        report =
          deadlineEpochMs === undefined
            ? await sender.getStats()
            : await Promise.race([
                sender.getStats(),
                new Promise((_, reject) => {
                  timer = setTimeout(
                    () => reject(new Error("E2E_NATIVE_STATS_DEADLINE")),
                    Math.max(0, deadlineEpochMs - Date.now() - 10),
                  );
                }),
              ]);
      } finally {
        if (timer !== undefined) clearTimeout(timer);
      }
      const current =
        peer.connectionState === "connected" &&
        sender.track === track &&
        track.readyState === "live";
      const encodings = [...report.values()]
        .filter((entry) => {
          if (
            entry.type !== "outbound-rtp" ||
            (entry.kind ?? entry.mediaType) !== "video"
          )
            return false;
          const identifier =
            entry.trackIdentifier ??
            report.get(entry.mediaSourceId)?.trackIdentifier;
          return identifier === undefined || identifier === track.id;
        })
        .map((entry) => ({
          rid: typeof entry.rid === "string" ? entry.rid : null,
          frames: entry.framesEncoded ?? null,
          packets: entry.packetsSent ?? 0,
        }));
      senders.push({
        peer: peerIndex,
        sender: senderIndex,
        capture: capture?.index ?? null,
        current,
        encodings,
      });
    }
  }
  return {
    captures: captures.map(({ capture, index }) => ({
      capture: index,
      kind: capture.kind,
      slot: capture.slot,
    })),
    senders,
  };
}
export async function sample({ deadlineEpochMs } = {}) {
  const state = window.__e2e;
  const peers = [];
  const audioRtpTracks = new Map();
  const videoRtpTracks = new Map();
  const closedPeer = () => ({
    connection: "closed",
    ice: "closed",
    transceivers: null,
    senders: null,
    receivers: null,
    localSdpBytes: null,
    selected: [],
    inbound: [],
    outbound: [],
    nativeSnapshotAvailable: false,
  });
  for (const pc of state.peers) {
    if (pc.connectionState === "closed") {
      peers.push(closedPeer());
      continue;
    }
    let report = new Map();
    if (pc.connectionState !== "closed") {
      try {
        state.samplePhase = "native-getStats";
        if (deadlineEpochMs !== undefined && Date.now() >= deadlineEpochMs)
          throw new Error("E2E_NATIVE_STATS_DEADLINE");
        let statsTimer;
        try {
          report =
            deadlineEpochMs === undefined
              ? await pc.getStats()
              : await Promise.race([
                  pc.getStats(),
                  new Promise((_, reject) => {
                    statsTimer = setTimeout(
                      () => reject(new Error("E2E_NATIVE_STATS_DEADLINE")),
                      Math.max(
                        0,
                        deadlineEpochMs -
                          Date.now() -
                          Math.min(
                            100,
                            Math.max(1, (deadlineEpochMs - Date.now()) / 10),
                          ),
                      ),
                    );
                  }),
                ]);
        } finally {
          if (statsTimer !== undefined) clearTimeout(statsTimer);
        }
        state.samplePhase = "native-stats-resolved";
      } catch (error) {
        // Firefox can reject or leave getStats pending when close races this
        // sample. Only a confirmed closed peer has unavailable native stats.
        if (pc.connectionState !== "closed") {
          if (error.message === "E2E_NATIVE_STATS_DEADLINE")
            throw new Error(
              `${error.message} pc=${state.peers.indexOf(pc)} connection=${pc.connectionState} ice=${pc.iceConnectionState}`,
              { cause: error },
            );
          throw error;
        }
      }
    }
    if (pc.connectionState === "closed") {
      peers.push(closedPeer());
      continue;
    }
    const entries = [...report.values()];
    audioRtpTracks.set(
      pc,
      new Set(
        entries
          .filter(
            (s) =>
              s.type === "inbound-rtp" &&
              (s.kind ?? s.mediaType) === "audio" &&
              s.packetsReceived > 0,
          )
          .map(
            (s) =>
              s.trackIdentifier ?? report.get(s.receiverId)?.trackIdentifier,
          )
          .filter((id) => typeof id === "string"),
      ),
    );
    videoRtpTracks.set(
      pc,
      new Set(
        entries
          .filter(
            (s) =>
              s.type === "inbound-rtp" &&
              (s.kind ?? s.mediaType) === "video" &&
              s.packetsReceived > 0,
          )
          .map(
            (s) =>
              s.trackIdentifier ?? report.get(s.receiverId)?.trackIdentifier,
          )
          .filter((id) => typeof id === "string"),
      ),
    );
    const selected = [];
    for (const transport of entries.filter(
      (s) => s.type === "transport" && s.selectedCandidatePairId,
    )) {
      const pair = report.get(transport.selectedCandidatePairId);
      if (pair)
        selected.push({
          state: pair.state,
          local: report.get(pair.localCandidateId)?.candidateType,
          remote: report.get(pair.remoteCandidateId)?.candidateType,
        });
    }
    const rtp = (type, outbound) =>
      entries
        .filter((s) => s.type === type)
        .map((s) => ({
          kind: s.kind ?? s.mediaType,
          bytes: (outbound ? s.bytesSent : s.bytesReceived) ?? 0,
          packets: (outbound ? s.packetsSent : s.packetsReceived) ?? 0,
          frames: (outbound ? s.framesEncoded : s.framesDecoded) ?? null,
          keyframes:
            (outbound ? s.keyFramesEncoded : s.keyFramesDecoded) ?? null,
          lost: s.packetsLost ?? 0,
          jitter: s.jitter ?? 0,
          pli: s.pliCount ?? 0,
        }));
    state.samplePhase = "native-peer-getters";
    peers.push({
      connection: pc.connectionState,
      ice: pc.iceConnectionState,
      transceivers: pc.getTransceivers().length,
      senders: pc.getSenders().filter((s) => s.track?.readyState === "live")
        .length,
      receivers: pc.getReceivers().filter((r) => r.track?.readyState === "live")
        .length,
      localSdpBytes: pc.localDescription?.sdp.length ?? 0,
      iceEndpointCategories: (pc.getConfiguration?.().iceServers ?? []).flatMap(
        (server) =>
          (Array.isArray(server.urls) ? server.urls : [server.urls]).map(
            (raw) => {
              try {
                const endpoint = new URL(
                  raw.replace(/^(turns?|stuns?):/, "http://"),
                );
                return {
                  scheme: raw.split(":")[0],
                  category: ["localhost", "127.0.0.1", "[::1]"].includes(
                    endpoint.hostname,
                  )
                    ? "loopback"
                    : "non-loopback",
                };
              } catch {
                return { category: "unparseable" };
              }
            },
          ),
      ),
      audioSenders: pc
        .getSenders()
        .filter((s) => s.track?.kind === "audio")
        .map((s) => ({
          live: s.track.readyState === "live",
          enabled: s.track.enabled,
        })),
      selected,
      inbound: rtp("inbound-rtp", false),
      outbound: rtp("outbound-rtp", true),
    });
  }
  state.samplePhase = "native-video-observation";
  const videos = [...document.querySelectorAll("figure video")].map((video) => {
    const caption =
      video.closest("figure").querySelector("figcaption")?.textContent ?? "";
    let pixels = null;
    if (video.readyState >= 2 && video.videoWidth) {
      const canvas = document.createElement("canvas");
      canvas.width = 2;
      canvas.height = 2;
      const ctx = canvas.getContext("2d");
      ctx.drawImage(video, 0, 0, 2, 2);
      const data = ctx.getImageData(0, 0, 2, 2).data;
      pixels = {
        source: [...data.slice(0, 3)],
        motion: [...data.slice(8, 11)],
      };
    }
    let rendered = state.renderedVideos.get(video);
    if (!rendered) {
      rendered = { callbacks: 0, presented: 0 };
      state.renderedVideos.set(video, rendered);
      if (typeof video.requestVideoFrameCallback === "function") {
        const onFrame = (_now, metadata) => {
          rendered.callbacks++;
          if (
            Number.isSafeInteger(metadata.presentedFrames) &&
            metadata.presentedFrames > 0
          )
            rendered.presented = Math.max(
              rendered.presented,
              metadata.presentedFrames,
            );
          video.requestVideoFrameCallback(onFrame);
        };
        video.requestVideoFrameCallback(onFrame);
      }
    }
    const qualityFrames = video.getVideoPlaybackQuality?.().totalVideoFrames;
    const callbackFrames = rendered.presented || rendered.callbacks;
    const hasQuality = Number.isSafeInteger(qualityFrames) && qualityFrames > 0;
    return {
      kind: caption.includes("— Live")
        ? "live"
        : caption.includes("— Bildschirm")
          ? "screen"
          : "camera",
      width: video.videoWidth,
      height: video.videoHeight,
      paused: video.paused,
      ready: video.readyState,
      renderedFrames: hasQuality ? qualityFrames : callbackFrames,
      renderedFramesSource: hasQuality
        ? "native-playback-quality"
        : typeof video.requestVideoFrameCallback === "function"
          ? "native-video-frame-callback"
          : "unavailable",
      playbackQualityFrames: Number.isSafeInteger(qualityFrames)
        ? qualityFrames
        : null,
      videoFrameCallbacks: rendered.callbacks,
      videoPresentedFrames: rendered.presented,
      pixels,
    };
  });
  state.samplePhase = "native-snapshot-complete";
  return {
    micCalls: state.micCalls,
    displayCalls: state.displayCalls,
    cameraCalls: state.cameraCalls,
    playRejected: state.playRejected,
    voiceRosterSnapshots: state.voiceRosterSnapshots,
    voiceOccupancy: state.voiceRoster.filter(
      (entry) => entry.c === state.expectedVoiceChannel,
    ).length,
    liveOccupancy: state.voiceRoster.filter(
      (entry) => entry.c === state.expectedVoiceChannel && entry.l,
    ).length,
    rejectedSdp: state.rejectedSdp,
    rejectedSdpErrors: state.rejectedSdpErrors?.slice() ?? [],
    roomAudio: (() => {
      const incoming = [
        ...new Map(state.incomingTracks.map((t) => [t.track, t])).values(),
      ].filter(
        (t) =>
          t.pc === state.expectedAudioPeer &&
          t.pc.connectionState !== "closed" &&
          t.track.kind === "audio" &&
          t.track.readyState === "live" &&
          audioRtpTracks.get(t.pc)?.has(t.track.id),
      );
      return {
        perSource: (state.expectedAudioPublishers ?? []).map(
          (publisher) =>
            incoming.filter((t) => t.publisher === publisher).length,
        ),
        foreign: incoming.filter(
          (t) => !(state.expectedAudioPublishers ?? []).includes(t.publisher),
        ).length,
      };
    })(),
    watchVideoSources: (() => {
      const incoming = [
        ...new Map(state.incomingTracks.map((t) => [t.track, t])).values(),
      ].filter(
        (t) =>
          t.pc === state.expectedAudioPeer &&
          t.pc.connectionState !== "closed" &&
          t.track.kind === "video" &&
          t.track.readyState === "live" &&
          videoRtpTracks.get(t.pc)?.has(t.track.id),
      );
      return {
        selectedLive: incoming.filter(
          (t) =>
            t.publisher === state.expectedLivePublisher && t.sourceKind === "l",
        ).length,
        foreign: incoming.filter(
          (t) =>
            t.publisher !== state.expectedLivePublisher || t.sourceKind !== "l",
        ).length,
      };
    })(),
    heldTrackCount: state.heldTracks.length,
    heldVideoTracks: state.heldTracks.filter((t) => t.kind === "video").length,
    heldLiveClaimCount: state.heldLiveClaims.length,
    playback: [...state.mediaElements].map((el) => ({
      kind: el.tagName === "AUDIO" ? "audio" : "video",
      paused: el.paused,
      muted: el.muted,
      volume: el.volume,
      audioTracks:
        el.srcObject?.getAudioTracks().filter((t) => t.readyState === "live")
          .length ?? 0,
      videoTracks:
        el.srcObject?.getVideoTracks().filter((t) => t.readyState === "live")
          .length ?? 0,
    })),
    duplicateAudioPlaybackTracks: (() => {
      const tracks = [...state.mediaElements]
        .filter((el) => !el.paused && el.srcObject)
        .flatMap((el) =>
          el.srcObject.getAudioTracks().filter((t) => t.readyState === "live"),
        );
      return tracks.length - new Set(tracks).size;
    })(),
    peers,
    videos,
    captures: state.captures.map((c) => ({
      kind: c.kind,
      slot: c.slot,
      state: c.track.readyState,
      muted: c.track.muted,
      colors: c.colors,
    })),
    sockets: state.sockets.map((s) => ({
      plane: s.plane,
      ready: s.ws.readyState,
      offers: s.offers,
      maxSdp: s.maxSdp,
      receivedEvents: s.receivedEvents,
      gaps: s.gaps,
      resyncs: s.resyncs,
      dmDiscoveries: s.dmDiscoveries,
      epochChanges: [...s.topics.values()].reduce(
        (n, t) => n + t.epochChanges,
        0,
      ),
      liveOn: s.liveOn,
      liveOff: s.liveOff,
    })),
  };
}
