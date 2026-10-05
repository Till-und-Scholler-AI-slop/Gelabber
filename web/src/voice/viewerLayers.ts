import type { StatsEntry } from "./diagnostics.ts";
import type { MediaClientFrame } from "./media.ts";
import type { PeerConnection, RtpSender } from "./session.ts";
export type ViewerSource = {
  trackId: string;
  userId: string;
  kind: "v" | "s" | "l";
};
/** Native VP8 representations. Keep decoder capabilities for reused m-lines. */
export function addLayeredVideo(
  pc: PeerConnection,
  track: MediaStreamTrack,
  stream: MediaStream,
): RtpSender | undefined {
  if (!pc.addTransceiver || typeof RTCRtpSender === "undefined") return;
  const codecs = RTCRtpSender.getCapabilities("video")?.codecs;
  if (!codecs?.some((c) => c.mimeType.toLowerCase() === "video/vp8")) return;
  let transceiver: ReturnType<NonNullable<PeerConnection["addTransceiver"]>> =
    undefined;
  try {
    transceiver = pc.addTransceiver(track, {
      direction: "sendrecv",
      streams: [stream],
      sendEncodings: [
        { rid: "q", scaleResolutionDownBy: 4 },
        { rid: "f", scaleResolutionDownBy: 1 },
      ],
    });
    if (!transceiver?.sender) return;
    const rank = (c: { mimeType: string }) =>
      c.mimeType.toLowerCase() === "video/vp8"
        ? 0
        : c.mimeType.toLowerCase() === "video/rtx"
          ? 1
          : 2;
    transceiver.setCodecPreferences?.(
      [...codecs].sort((a, b) => rank(a) - rank(b)),
    );
    return transceiver.sender;
  } catch {
    if (transceiver?.sender) pc.removeTrack?.(transceiver.sender);
    return;
  }
}
export function renderedVideoHeight(trackId: string): number {
  if (typeof document === "undefined") return 0;
  let height = 0;
  for (const video of document.querySelectorAll("video")) {
    const stream = video.srcObject;
    if (
      !(stream instanceof MediaStream) ||
      !stream.getVideoTracks().some((t) => t.id === trackId)
    )
      continue;
    const rect = video.getBoundingClientRect();
    if (rect.width <= 0 || rect.height <= 0) continue;
    const imageHeight =
      video.videoWidth > 0 && video.videoHeight > 0
        ? Math.min(
            rect.height,
            (rect.width * video.videoHeight) / video.videoWidth,
          )
        : rect.height;
    height = Math.max(height, imageHeight * (globalThis.devicePixelRatio || 1));
  }
  return Math.min(16384, Math.ceil(height));
}
type Previous = {
  received: number;
  lost: number;
  time: number;
  congested: boolean;
  good: number;
};
/** Receiver loss controls its representation. Four healthy intervals recover;
 * unknown/reset counters never masquerade as congestion or bandwidth. */
export class ViewerLayerController {
  private previous = new Map<string, Previous>();
  update(
    report: readonly StatsEntry[],
    sources: readonly ViewerSource[],
    send: (frame: MediaClientFrame) => void,
    height: (id: string) => number = renderedVideoHeight,
  ): void {
    const active = new Set<string>();
    for (const source of sources) {
      const stats = report.find(
        (e) =>
          e.type === "inbound-rtp" &&
          e.kind === "video" &&
          e.trackIdentifier === source.trackId,
      );
      if (!stats) continue;
      const key = `${source.userId}:${source.kind}`;
      active.add(key);
      const received = stats.packetsReceived,
        lost = stats.packetsLost,
        time = stats.timestamp;
      const previous = this.previous.get(key);
      let congested = previous?.congested ?? false,
        good = previous?.good ?? 0;
      if (
        typeof received === "number" &&
        typeof lost === "number" &&
        typeof time === "number"
      ) {
        if(previous&&time<=previous.time){
          // Overlapping native getStats promises may resolve out of order.
          // Keep newer evidence; stale samples are not counter restarts.
          send({op:"q",u:source.userId,k:source.kind,h:height(source.trackId),congested:previous.congested});
          continue;
        }
        if (
          previous &&
          received >= previous.received &&
          lost >= previous.lost
        ) {
          const n = received - previous.received,
            missing = lost - previous.lost;
          if (n + missing >= 20 && missing / (n + missing) > 0.03) {
            congested = true;
            good = 0;
          } else if (n >= 20 && missing === 0) {
            good++;
            if (good >= 4) congested = false;
          } else good = 0;
        } else {
          congested = false;
          good = 0;
        }
        this.previous.set(key, { received, lost, time, congested, good });
      }
      send({
        op: "q",
        u: source.userId,
        k: source.kind,
        h: height(source.trackId),
        congested,
      });
    }
    for (const key of this.previous.keys())
      if (!active.has(key)) this.previous.delete(key);
  }
}
