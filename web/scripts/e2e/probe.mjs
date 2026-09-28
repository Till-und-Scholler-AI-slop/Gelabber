/* global window, navigator, document, URL, DOMException, setInterval, clearInterval */
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
    cancelNextCapture: false,
  });
  const Peer = window.RTCPeerConnection;
  window.RTCPeerConnection = class extends Peer {
    constructor(config) {
      super(relay ? { ...config, iceTransportPolicy: "relay" } : config);
      state.peers.push(this);
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
          if (frame.op === "sig" && frame.k === "l") {
            if (frame.on) item.liveOn++;
            else item.liveOff++;
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
    const result =
      state.rejectPlayback && this.tagName === "VIDEO" && this.srcObject
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
export async function sample() {
  const state = window.__e2e;
  const peers = [];
  for (const pc of state.peers) {
    const report = await pc.getStats();
    const entries = [...report.values()];
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
    peers.push({
      connection: pc.connectionState,
      ice: pc.iceConnectionState,
      transceivers: pc.getTransceivers().length,
      senders: pc.getSenders().filter((s) => s.track?.readyState === "live")
        .length,
      receivers: pc.getReceivers().filter((r) => r.track?.readyState === "live")
        .length,
      localSdpBytes: pc.localDescription?.sdp.length ?? 0,
      selected,
      inbound: rtp("inbound-rtp", false),
      outbound: rtp("outbound-rtp", true),
    });
  }
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
      renderedFrames:
        video.getVideoPlaybackQuality?.().totalVideoFrames ?? null,
      pixels,
    };
  });
  return {
    micCalls: state.micCalls,
    displayCalls: state.displayCalls,
    cameraCalls: state.cameraCalls,
    playRejected: state.playRejected,
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
