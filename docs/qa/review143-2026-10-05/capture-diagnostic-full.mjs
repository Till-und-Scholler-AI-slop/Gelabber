/* global window, document, setInterval, setTimeout, clearTimeout */
import { nativeEvaluate } from "./native-evaluate.mjs";

// External, bounded observation only. No sender, SDP, track, playback or PLI
// mutation. Keep native source/receiver/canvas progression separate.
export async function installCaptureDiagnostic(actor) {
  return nativeEvaluate(actor, () => {
    const state = window.__e2e;
    if (state.captureDiagnostic) return;
    const trace = state.captureDiagnostic = { rows: [], busy: false };
    const finite = value => Number.isFinite(value) ? value : null;
    const stats = async subject => {
      let timer;
      try {
        return await Promise.race([subject.getStats(), new Promise((_, reject) => {
          timer = setTimeout(() => reject(new Error("probe-stats-timeout")), 400);
        })]);
      } finally { clearTimeout(timer); }
    };
    setInterval(async () => {
      if (trace.busy) return;
      trace.busy = true;
      const row = { at: Date.now(), visibility: document.visibilityState,
        rejectPlayback: !!state.rejectPlayback, playRejected: state.playRejected,
        layerHints: state.sockets.flatMap(socket => socket.layerHints ?? []),
        captures: state.captures.map((capture, index) => ({ index, kind: capture.kind,
          slot: capture.slot, ready: capture.track.readyState, muted: capture.track.muted,
          draws: capture.drawCount?.() ?? null })), peers: [] };
      try {
        for (const [index, peer] of state.peers.entries()) {
          if (peer.connectionState === "closed") continue;
          const pc = { index, connection: peer.connectionState, signaling: peer.signalingState,
            sources: [], receivers: [], paths: [] };
          row.peers.push(pc);
          const report = await stats(peer);
          for (const entry of report.values()) {
            if (entry.type !== "transport" || !entry.selectedCandidatePairId) continue;
            const pair = report.get(entry.selectedCandidatePairId);
            pc.paths.push({ outgoing: finite(pair?.availableOutgoingBitrate), rtt: finite(pair?.currentRoundTripTime) });
          }
          for (const transceiver of peer.getTransceivers()) {
            const sender = transceiver.sender, track = sender.track;
            if (track?.kind === "video") {
              const capture = state.captures.findIndex(value => value.track === track);
              const native = await stats(sender);
              const description = peer.localDescription?.sdp.split(/(?=m=)/)
                .find(section => section.includes(`a=mid:${transceiver.mid}\r\n`));
              const msid = description?.split(/\r?\n/).find(line => line.startsWith("a=msid:"))?.slice(7).split(" ");
              const source = { mid: transceiver.mid, direction: transceiver.currentDirection,
                capture, ready: track.readyState, muted: track.muted,
                msidTrackMatches: msid?.[1] === track.id,
                msidKind: msid?.[0]?.split(":")[1] ?? null,
                contract: sender.getParameters().encodings.map(encoding => ({ rid: encoding.rid ?? null,
                  active: encoding.active ?? null, maxBitrate: encoding.maxBitrate ?? null,
                  maxFramerate: encoding.maxFramerate ?? null, scale: encoding.scaleResolutionDownBy ?? null })), encodings: [] };
              pc.sources.push(source);
              for (const entry of native.values()) {
                if (entry.type !== "outbound-rtp" || (entry.kind ?? entry.mediaType) !== "video") continue;
                const identifier = entry.trackIdentifier ?? native.get(entry.mediaSourceId)?.trackIdentifier;
                if (identifier !== undefined && identifier !== track.id) continue;
                const codec = native.get(entry.codecId), remote = native.get(entry.remoteId);
                source.encodings.push({ current: sender.track === track && track.readyState === "live",
                  rid: entry.rid ?? null, ssrc: entry.ssrc, pt: codec?.payloadType ?? null, codec: codec?.mimeType ?? null,
                  packets: finite(entry.packetsSent), bytes: finite(entry.bytesSent), headerBytes: finite(entry.headerBytesSent),
                  frames: finite(entry.framesEncoded), keyframes: finite(entry.keyFramesEncoded),
                  width: finite(entry.frameWidth), height: finite(entry.frameHeight), fps: finite(entry.framesPerSecond),
                  targetBitrate: finite(entry.targetBitrate), limitation: entry.qualityLimitationReason ?? null,
                  lost: finite(remote?.packetsLost), fractionLost: finite(remote?.fractionLost), rtt: finite(remote?.roundTripTime) });
              }
            }
            const receiver = transceiver.receiver;
            if (receiver.track.kind !== "video") continue;
            const item = state.incomingTracks.findLast(value => value.track === receiver.track && value.pc === peer);
            if (item?.sourceKind === "l" && !trace.heldTrack) trace.heldTrack = receiver.track;
            const native = await stats(receiver);
            const incoming = { mid: transceiver.mid, kind: item?.sourceKind ?? null,
              ready: receiver.track.readyState, muted: receiver.track.muted,
              held: receiver.track === trace.heldTrack, rtp: [] };
            pc.receivers.push(incoming);
            for (const entry of native.values()) {
              if (entry.type !== "inbound-rtp" || (entry.kind ?? entry.mediaType) !== "video") continue;
              const codec = native.get(entry.codecId);
              incoming.rtp.push({ ssrc: entry.ssrc, pt: codec?.payloadType ?? null, codec: codec?.mimeType ?? null,
                packets: finite(entry.packetsReceived), bytes: finite(entry.bytesReceived), frames: finite(entry.framesDecoded),
                keyframes: finite(entry.keyFramesDecoded), pli: finite(entry.pliCount), lost: finite(entry.packetsLost),
                width: finite(entry.frameWidth), height: finite(entry.frameHeight) });
            }
          }
        }
        const video = [...document.querySelectorAll("figure")].find(figure => figure.textContent.includes("— Live"))?.querySelector("video");
        if (video && !trace.heldElement) trace.heldElement = video;
        row.video = video ? { currentIsHeldElement: video === trace.heldElement,
          hasHeldTrack: video.srcObject?.getTracks().includes(trace.heldTrack) ?? false,
          paused: video.paused, ready: video.readyState, width: video.videoWidth, height: video.videoHeight,
          presented: video.getVideoPlaybackQuality?.().totalVideoFrames ?? null,
          rect: { width: video.getBoundingClientRect().width, height: video.getBoundingClientRect().height },
          dpr: window.devicePixelRatio } : null;
      } catch { row.probeUnavailable = true; }
      finally {
        if (trace.rows.length >= 400) trace.rows.shift();
        trace.rows.push(row); trace.busy = false;
      }
    }, 100);
  });
}

export async function captureDiagnostics(f) {
  const result = {};
  for (const [name, actor] of Object.entries({ owner: f.owner, watcher: f.watcher })) {
    try {
      result[name] = await nativeEvaluate(actor, () => window.__e2e.captureDiagnostic?.rows ?? []);
    } catch { result[name] = { probeUnavailable: true }; }
  }
  return result;
}
