import type { StatsEntry } from "./diagnostics.ts";
import type { MediaRequests } from "./media.ts";
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
export type ViewerSource = {
  trackId: string;
  consumerId: string;
  generation: string;
  ssrc?: number;
};
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
  private sent = new Map<
    string,
    { h: number; congested: boolean; at: number }
  >();
  async update(
    report: readonly StatsEntry[],
    sources: readonly ViewerSource[],
    send: (frame: MediaRequests["q"]) => unknown,
    height: (id: string) => number = renderedVideoHeight,
  ): Promise<void> {
    const pending: MediaRequests["q"][] = [];
    const active = new Set<string>();
    const now = Date.now();
    const emit = (frame: MediaRequests["q"]) => {
      const key = `${frame.generation}:${frame.consumerId}`;
      const last = this.sent.get(key);
      if (
        last &&
        last.h === frame.h &&
        last.congested === frame.congested &&
        now - last.at < 30_000
      )
        return;
      pending.push(frame);
    };
    for (const source of sources) {
      const key = `${source.generation}:${source.consumerId}`;
      active.add(key);
      const previous = this.previous.get(key);
      const stats = report.find(
        (e) =>
          e.type === "inbound-rtp" &&
          e.kind === "video" &&
          (source.ssrc !== undefined
            ? e.ssrc === source.ssrc
            : e.trackIdentifier === source.trackId),
      );
      if (!stats) {
        emit({
          consumerId: source.consumerId,
          generation: source.generation,
          h: height(source.trackId),
          congested: previous?.congested ?? false,
        });
        continue;
      }
      const received = stats.packetsReceived,
        lost = stats.packetsLost,
        time = stats.timestamp;
      let congested = previous?.congested ?? false,
        good = previous?.good ?? 0;
      if (
        typeof received === "number" &&
        typeof lost === "number" &&
        typeof time === "number" &&
        Number.isFinite(time) &&
        Number.isFinite(received) &&
        Number.isFinite(lost) &&
        received >= 0 &&
        lost >= 0
      ) {
        if (previous && time <= previous.time) {
          // Overlapping native getStats promises may resolve out of order.
          // Keep newer evidence; stale samples are not counter restarts.
          emit({
            consumerId: source.consumerId,
            generation: source.generation,
            h: height(source.trackId),
            congested: previous.congested,
          });
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
          congested = previous?.congested ?? false;
          good = 0;
        }
        this.previous.set(key, { received, lost, time, congested, good });
      }
      emit({
        consumerId: source.consumerId,
        generation: source.generation,
        h: height(source.trackId),
        congested,
      });
    }
    for (const key of this.sent.keys())
      if (!active.has(key)) this.sent.delete(key);
    for (const key of this.previous.keys())
      if (!active.has(key)) this.previous.delete(key);
    // Leave room in MediaPeer's 64-request budget for control operations.
    // Failed hints stay eligible for the next sample instead of being suppressed.
    for (let offset = 0; offset < pending.length; offset += 8) {
      await Promise.all(
        pending.slice(offset, offset + 8).map(async (frame) => {
          try {
            await send(frame);
            this.sent.set(`${frame.generation}:${frame.consumerId}`, {
              h: frame.h,
              congested: frame.congested,
              at: Date.now(),
            });
          } catch {
            // A source can retire or the request budget can fill during sampling.
          }
        }),
      );
    }
  }
}
