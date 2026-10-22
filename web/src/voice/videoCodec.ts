// The codec a video source is sent in (issue #165). mediasoup forwards what
// it gets and transcodes nothing, so every receiver in the room has to decode
// the publisher's codec: the server names the codecs they all decode
// (`MediaResults["capabilities"]`, the `codecs` event). Among those, AV1
// wins where this device encodes it in hardware: the same picture for less
// bitrate at little CPU (issue #318). Then H.264 where it is encoded in
// hardware, which costs the least CPU; otherwise VP8, which every WebRTC
// stack encodes and decodes. AV1 on the CPU costs far more than VP8 and
// comes last.
//
// AV1 goes out as one stream, never in simulcast layers, so every receiver
// decodes the whole picture. A device that does not decode AV1 in hardware
// leaves AV1 out of the codecs it tells the server it receives
// (`receivableCapabilities`): the room then has no AV1, and senders stay on
// H.264 or VP8 with layers.

import type {
  RtpCapabilities,
  RtpCodecCapability,
} from "mediasoup-client/types";
import { isDesktopApp } from "./native/bridge.ts";
import { loadNativeFeatures, type NativeFeature } from "./native/features.ts";

const H264 = "video/h264";
const AV1 = "video/av1";
const VP8 = "video/vp8";
const VP9 = "video/vp9";

/** Which codecs this device encodes in hardware. */
export type HardwareVideo = { av1: boolean; h264: boolean };

export const SOFTWARE_VIDEO: HardwareVideo = { av1: false, h264: false };

/** The H.264 every decoder in a WebRTC stack has and the server counts:
 * packetization mode 1, Constrained Baseline (profile 0x42 with
 * constraint_set1, e.g. 42e01f). */
export function publishableH264(codec: RtpCodecCapability): boolean {
  if (codec.mimeType.toLowerCase() !== H264) return false;
  const parameters = (codec.parameters ?? {}) as Record<string, unknown>;
  const profile = String(parameters["profile-level-id"] ?? "42001f");
  const constraints = Number.parseInt(profile.slice(2, 4), 16);
  return (
    Number(parameters["packetization-mode"]) === 1 &&
    profile.slice(0, 2).toLowerCase() === "42" &&
    (constraints & 0x40) !== 0
  );
}

/** The video codec to send: the best of the device's `codecs` that every
 * receiver decodes. `allowed` is null from a server that does not say;
 * then, and when nothing is allowed, the order is that of servers before. */
export function chooseVideoCodec(
  codecs: readonly RtpCodecCapability[],
  allowed: readonly string[] | null,
  hardware: HardwareVideo,
): RtpCodecCapability | undefined {
  const find = (mime: string, usable: (c: RtpCodecCapability) => boolean) =>
    codecs.find((c) => c.mimeType.toLowerCase() === mime && usable(c));
  if (allowed?.length) {
    const order = [
      ...(hardware.av1 ? [AV1] : []),
      ...(hardware.h264 ? [H264] : []),
      VP8,
      H264,
      VP9,
      AV1,
    ];
    for (const mime of order) {
      if (!allowed.includes(mime)) continue;
      const codec = find(mime, (c) => mime !== H264 || publishableH264(c));
      if (codec) return codec;
    }
  }
  for (const mime of [VP8, H264, VP9, AV1]) {
    const codec = find(mime, () => true);
    if (codec) return codec;
  }
  return undefined;
}

/** The device's codecs to receive with, as the server is told them: without
 * AV1 (and its RTX) unless `av1Decoding`, the device decoding AV1 in
 * hardware. */
export function receivableCapabilities(
  capabilities: RtpCapabilities,
  av1Decoding: boolean,
): RtpCapabilities {
  if (av1Decoding || !capabilities.codecs) return capabilities;
  const av1 = new Set(
    capabilities.codecs
      .filter((c) => c.mimeType.toLowerCase() === AV1)
      .map((c) => c.preferredPayloadType),
  );
  const codecs = capabilities.codecs.filter(
    (c) =>
      c.mimeType.toLowerCase() !== AV1 &&
      !(
        c.mimeType.toLowerCase() === "video/rtx" &&
        av1.has(Number((c.parameters as { apt?: unknown } | undefined)?.apt))
      ),
  );
  return { ...capabilities, codecs };
}

/** The desktop app's features when its core encodes, or decodes, in
 * hardware. */
export const H264_HARDWARE_FEATURE: NativeFeature = "h264-hardware";
export const AV1_HARDWARE_FEATURE: NativeFeature = "av1-hardware";
export const AV1_HARDWARE_DECODE_FEATURE: NativeFeature = "av1-hardware-decode";

/** How long the browser may take to say; past it, software is assumed. */
const PROBE_TIMEOUT_MS = 1000;

type Probed = HardwareVideo & { av1Decoding: boolean };

let probed: Promise<Probed> | null = null;

function probeOnce(): Promise<Probed> {
  probed ??= probe().catch(() => ({ ...SOFTWARE_VIDEO, av1Decoding: false }));
  return probed;
}

/** Which codecs this device encodes in hardware, asked once per page. A
 * browser says so as `powerEfficient`. */
export async function hardwareVideo(): Promise<HardwareVideo> {
  const { av1, h264 } = await probeOnce();
  return { av1, h264 };
}

/** Whether this device decodes AV1 in hardware, asked with the encoders. */
export async function hardwareAv1Decoding(): Promise<boolean> {
  return (await probeOnce()).av1Decoding;
}

async function probe(): Promise<Probed> {
  if (isDesktopApp()) {
    const features = await withTimeout(loadNativeFeatures(), new Set<string>());
    return {
      av1: features.has(AV1_HARDWARE_FEATURE),
      h264: features.has(H264_HARDWARE_FEATURE),
      av1Decoding: features.has(AV1_HARDWARE_DECODE_FEATURE),
    };
  }
  const [av1, h264, av1Decoding] = await Promise.all([
    withTimeout(powerEfficient("encodingInfo", "video/AV1"), false),
    withTimeout(powerEfficient("encodingInfo", "video/H264"), false),
    withTimeout(powerEfficient("decodingInfo", "video/AV1"), false),
  ]);
  return { av1, h264, av1Decoding };
}

async function powerEfficient(
  question: "encodingInfo" | "decodingInfo",
  contentType: string,
): Promise<boolean> {
  const capabilities =
    typeof navigator === "undefined" ? undefined : navigator.mediaCapabilities;
  if (!capabilities?.[question]) return false;
  const info = await capabilities[question]({
    type: "webrtc",
    video: {
      contentType,
      width: 1920,
      height: 1080,
      bitrate: 4_000_000,
      framerate: 30,
    },
  });
  return info.supported && info.powerEfficient;
}

function withTimeout<T>(answer: Promise<T>, fallback: T): Promise<T> {
  return Promise.race([
    answer.catch(() => fallback),
    new Promise<T>((resolve) =>
      setTimeout(() => resolve(fallback), PROBE_TIMEOUT_MS),
    ),
  ]);
}

/** Tests: asks again on the next call. */
export function resetHardwareVideoForTests(): void {
  probed = null;
}
