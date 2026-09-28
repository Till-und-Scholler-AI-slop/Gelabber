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
    heldLiveClaims: [],
    rejectedSdp: 0,
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
    set ontrack(handler) {
      super.ontrack =
        handler &&
        ((event) => {
          state.incomingTracks.push({
            pc: this,
            track: event.track,
            publisher: (event.streams[0]?.id ?? event.track.id).split(":")[0],
            sourceKind: (event.streams[0]?.id ?? event.track.id)
              .split(":")[1]
              ?.split("-")[0],
          });
          if (state.holdTracks)
            state.heldTracks.push({
              event,
              deliver: () => handler.call(this, event),
            });
          else handler.call(this, event);
        });
    }
    get ontrack() {
      return super.ontrack;
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
        return super.setRemoteDescription({
          type: "answer",
          sdp: "invalid native SDP rejection control",
        });
      }
      return super.setRemoteDescription(description);
    }
  };
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
      };
      state.sockets.push(item);
      this.addEventListener("message", (event) => {
        try {
          const frame = JSON.parse(event.data);
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
      const send = this.send.bind(this);
      this.send = (data) => {
        try {
          const frame = JSON.parse(data);
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
        if (error.message === "E2E_NATIVE_STATS_DEADLINE") throw error;
        // Firefox rejects getStats on a closed peer, including a close racing
        // this sample. Keep the closed peer visible; never hide a live error.
        if (pc.connectionState !== "closed") throw error;
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
    heldVideoTracks: state.heldTracks.filter(
      (t) => t.event.track.kind === "video",
    ).length,
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
