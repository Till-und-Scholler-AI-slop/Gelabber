// The upload budget on the seat's video senders: bitrate and FPS caps per
// stream profile, and the senders' priorities.

import { prioritizeSender } from "./mediaPriority.ts";
import type { MediaPeer } from "./mediaPeer.ts";
import type { MediaConnection } from "./mediasoupConnection.ts";
import {
  allocateVideoBitrates,
  streamProfileFps,
  useMediaSettings,
  type StreamProfileId,
} from "./settings.ts";

/**
 * One video-budget queue for the current peer generation. Profile changes,
 * publish, unpublish and profile changes share it. A newer request supersedes an
 * in-flight snapshot so an older pass cannot write a stale sender cap.
 */
export class VideoLimits {
  private chain: Promise<void> = Promise.resolve();
  private generation = 0;
  private revision = 0;

  constructor(
    /** The seat as it is now: a reset replaces it. */
    private readonly seat: () => MediaPeer,
    private readonly profileFor: (track: MediaStreamTrack) => StreamProfileId,
  ) {}

  reset(): void {
    this.revision += 1;
    this.chain = Promise.resolve();
    this.generation = this.seat().generation;
  }

  private current(
    pc: MediaConnection,
    generation: number,
    revision: number,
  ): boolean {
    const seat = this.seat();
    return (
      this.revision === revision &&
      this.generation === generation &&
      seat.generation === generation &&
      seat.connection === pc
    );
  }

  /** Serialize budget writes. Profiles are read when the pass runs, not when queued. */
  enqueue(pc: MediaConnection): Promise<void> {
    const generation = this.seat().generation;
    if (this.generation !== generation) {
      this.chain = Promise.resolve();
      this.generation = generation;
    }
    const revision = ++this.revision;
    const run = this.chain.then(
      () => this.apply(pc, generation, revision),
      () => this.apply(pc, generation, revision),
    );
    this.chain = run.then(
      () => undefined,
      () => undefined,
    );
    return run;
  }

  private async apply(
    pc: MediaConnection,
    generation: number,
    revision: number,
  ): Promise<void> {
    if (!this.current(pc, generation, revision)) return;
    const senders = (pc.senders() ?? []).filter(
      (s) => s.track?.kind === "video" && s.track.readyState !== "ended",
    );
    const profiles = senders.map((sender) =>
      sender.track ? this.profileFor(sender.track) : "balanced",
    );
    const shares = allocateVideoBitrates(
      profiles,
      useMediaSettings.getState().videoUploadLimit,
      useMediaSettings.getState().economyMode,
    );
    for (let index = 0; index < senders.length; index += 1) {
      if (!this.current(pc, generation, revision)) return;
      const sender = senders[index];
      const params = sender?.getParameters?.();
      if (!sender || !params?.encodings.length) continue;
      const share = shares[index] ?? null;
      const fps = streamProfileFps(profiles[index] ?? "balanced");
      for (let layer = 0; layer < params.encodings.length; layer += 1) {
        const encoding = params.encodings[layer]!;
        if (share === null) delete encoding.maxBitrate;
        else
          encoding.maxBitrate =
            Math.floor(share / params.encodings.length) +
            (layer < share % params.encodings.length ? 1 : 0);
        encoding.maxFramerate = fps;
      }
      try {
        await sender.setParameters?.(params);
      } catch {
        // A rejected hint leaves the last working sender settings intact.
      }
      await prioritizeSender(sender, "video", () =>
        this.current(pc, generation, revision),
      );
      // This snapshot is stale. Stop before the next sender; the newer pass
      // applies the latest budgets across every sender still sending.
      if (!this.current(pc, generation, revision)) return;
    }
  }
}
