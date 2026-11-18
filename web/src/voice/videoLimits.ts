// The upload budget on the seat's video senders: bitrate and FPS caps per
// stream profile, and the senders' priorities.
//
// A Live in sharp mode (GEL-26) keeps text readable on a tight connection.
// libwebrtc only switches on the full-size simulcast layer once the smaller
// ones have their bitrate, so a sharp Live makes them cheap (15 FPS, half
// the usual ceiling) and the full-size layer stays on at a lower upload.
// The sender then keeps the resolution and drops frames when its uplink or
// CPU is short ("maintain-resolution"). The smaller layers stay as the
// fallback for a weak viewer: mediasoup gives one the full-size layer at
// its lowest frame rate step before it falls back to a smaller layer.

import { prioritizeSender } from "./mediaPriority.ts";
import type { MediaPeer } from "./mediaPeer.ts";
import type { MediaConnection } from "./mediasoupConnection.ts";
import {
  allocateVideoBitrates,
  STREAM_PROFILES,
  streamProfileFps,
  useMediaSettings,
  type ScreenMode,
  type StreamProfileId,
} from "./settings.ts";

/** How often a sent capture's size is compared with the one its caps were
 * computed for: a shared window or tab can change size at any time. */
const RESIZE_CHECK_MS = 2_000;

/** Bits per pixel and frame: generous for screen content, still a hard ceiling. */
const BITS_PER_PIXEL = 0.1;

export function videoBitrateCeiling(
  width: number,
  height: number,
  fps: number,
): number {
  return Math.round(width * height * fps * BITS_PER_PIXEL);
}

/**
 * Per-encoding `maxBitrate`: the ceiling of each layer's own resolution, or a
 * binding upload share split in proportion to those ceilings.
 */
export function encodingBitrates(
  scales: readonly number[],
  width: number,
  height: number,
  fps: number | readonly number[],
  share: number | null,
  weights?: readonly number[],
): number[] {
  const ceilings = scales.map((scale, layer) =>
    Math.round(
      videoBitrateCeiling(
        width / scale,
        height / scale,
        typeof fps === "number" ? fps : (fps[layer] ?? 0),
      ) * (weights?.[layer] ?? 1),
    ),
  );
  if (share === null) return ceilings;
  const total = ceilings.reduce((sum, ceiling) => sum + ceiling, 0);
  return ceilings.map((ceiling) =>
    Math.min(ceiling, Math.floor((share * ceiling) / total)),
  );
}

/** Frame rate of a sharp Live's smaller layers: a fallback for viewers whose
 * connection cannot carry the full-size layer at its lowest step. */
export const SHARP_FALLBACK_FPS = 15;

/** Share of its usual bitrate ceiling a sharp Live's smaller layer gets. */
export const SHARP_FALLBACK_WEIGHT = 0.5;

/** Per-layer frame rates: every layer at the profile's rate, or for a sharp
 * Live with layers, the full-size one only. */
export function layerFrameRates(
  scales: readonly number[],
  fps: number,
  mode: ScreenMode | null,
): number[] {
  if (mode !== "sharp" || scales.length < 2) return scales.map(() => fps);
  const top = Math.min(...scales);
  return scales.map((scale) =>
    scale === top ? fps : Math.min(fps, SHARP_FALLBACK_FPS),
  );
}

/** Per-layer weights on the bitrate ceilings: 1 everywhere, or for a sharp
 * Live with layers, a cheaper fallback for every layer below full size. */
export function layerBitrateWeights(
  scales: readonly number[],
  mode: ScreenMode | null,
): number[] {
  if (mode !== "sharp" || scales.length < 2) return scales.map(() => 1);
  const top = Math.min(...scales);
  return scales.map((scale) => (scale === top ? 1 : SHARP_FALLBACK_WEIGHT));
}

/** What a sender gives up first when it is short of bits or CPU. */
export function degradationFor(
  mode: ScreenMode | null,
): RTCDegradationPreference | undefined {
  if (mode === "sharp") return "maintain-resolution";
  if (mode === "smooth") return "maintain-framerate";
  return undefined;
}

function sizeOf(track: MediaStreamTrack): string {
  const settings = track.getSettings?.();
  return `${settings?.width ?? 0}x${settings?.height ?? 0}`;
}

/**
 * One video-budget queue for the current peer generation. Profile changes,
 * publish, unpublish and profile changes share it. A newer request supersedes an
 * in-flight snapshot so an older pass cannot write a stale sender cap.
 */
export class VideoLimits {
  private chain: Promise<void> = Promise.resolve();
  private generation = 0;
  private revision = 0;
  /** The size each track's caps were computed for. */
  private sizes = new WeakMap<MediaStreamTrack, string>();
  private resizeTimer: ReturnType<typeof setInterval> | null = null;

  constructor(
    /** The seat as it is now: a reset replaces it. */
    private readonly seat: () => MediaPeer,
    private readonly profileFor: (track: MediaStreamTrack) => StreamProfileId,
  ) {}

  reset(): void {
    this.stopResizeCheck();
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
    this.watchResize(pc, generation);
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

  private stopResizeCheck(): void {
    if (this.resizeTimer !== null) clearInterval(this.resizeTimer);
    this.resizeTimer = null;
  }

  /** Runs the budget again when a sent video track's size changed, while
   * this connection is the seat's and sends video. */
  private watchResize(pc: MediaConnection, generation: number): void {
    if (this.resizeTimer !== null) return;
    this.resizeTimer = setInterval(() => {
      const seat = this.seat();
      const tracks = (pc.senders() ?? [])
        .map((sender) => sender.track)
        .filter(
          (track): track is MediaStreamTrack =>
            track?.kind === "video" && track.readyState !== "ended",
        );
      if (
        seat.generation !== generation ||
        seat.connection !== pc ||
        tracks.length === 0
      ) {
        this.stopResizeCheck();
        return;
      }
      if (tracks.some((track) => this.sizes.get(track) !== sizeOf(track)))
        void this.enqueue(pc).catch(() => {});
    }, RESIZE_CHECK_MS);
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
      const profile = profiles[index] ?? "balanced";
      const fps = streamProfileFps(profile);
      // Only a Live has a mode; cameras keep the browser's choice.
      const mode =
        sender.sourceKind === "s"
          ? useMediaSettings.getState().screenMode
          : null;
      const settings = sender.track?.getSettings?.();
      if (sender.track) this.sizes.set(sender.track, sizeOf(sender.track));
      const fallback = STREAM_PROFILES[profile];
      // libwebrtc's own defaults cap a 1080p+ layer at 5 Mbit/s, so every
      // layer gets an explicit ceiling from its own pixels and frame rate.
      const scales = params.encodings.map((e) => e.scaleResolutionDownBy ?? 1);
      const rates = layerFrameRates(scales, fps, mode);
      const bitrates = encodingBitrates(
        scales,
        settings?.width || fallback.width,
        settings?.height || fallback.height,
        rates,
        shares[index] ?? null,
        layerBitrateWeights(scales, mode),
      );
      for (let layer = 0; layer < params.encodings.length; layer += 1) {
        const encoding = params.encodings[layer]!;
        encoding.maxBitrate = bitrates[layer]!;
        encoding.maxFramerate = rates[layer]!;
      }
      const degradation = degradationFor(mode);
      if (degradation) params.degradationPreference = degradation;
      try {
        await sender.setParameters?.(params);
      } catch {
        // A browser that refuses the degradation preference still takes the
        // caps; any other refusal leaves the last working settings intact.
        if (degradation) {
          delete params.degradationPreference;
          try {
            await sender.setParameters?.(params);
          } catch {
            // As above.
          }
        }
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
