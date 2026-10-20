// The codec a video source is sent in (issue #165). mediasoup forwards what
// it gets and transcodes nothing, so every receiver in the room has to decode
// the publisher's codec: the server names the codecs they all decode
// (`MediaResults["capabilities"]`, the `codecs` event). Among those, H.264
// wins where this device encodes it in hardware, which costs the least CPU;
// otherwise VP8, which every WebRTC stack encodes and decodes.

import type { RtpCodecCapability } from "mediasoup-client/types";
import { isDesktopApp } from "./native/bridge.ts";
import { loadNativeFeatures, type NativeFeature } from "./native/features.ts";

const H264 = "video/h264";

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
  hardwareH264: boolean,
): RtpCodecCapability | undefined {
  const find = (mime: string, usable: (c: RtpCodecCapability) => boolean) =>
    codecs.find((c) => c.mimeType.toLowerCase() === mime && usable(c));
  if (allowed?.length) {
    const order = hardwareH264
      ? [H264, "video/vp8", "video/vp9", "video/av1"]
      : ["video/vp8", H264, "video/vp9", "video/av1"];
    for (const mime of order) {
      if (!allowed.includes(mime)) continue;
      const codec = find(mime, (c) => mime !== H264 || publishableH264(c));
      if (codec) return codec;
    }
  }
  for (const mime of ["video/vp8", H264, "video/vp9", "video/av1"]) {
    const codec = find(mime, () => true);
    if (codec) return codec;
  }
  return undefined;
}

/** The desktop app's feature when its core encodes H.264 in hardware. */
export const H264_HARDWARE_FEATURE: NativeFeature = "h264-hardware";

/** How long the browser may take to say; past it, software is assumed. */
const PROBE_TIMEOUT_MS = 1000;

let probed: Promise<boolean> | null = null;

/** Whether this device encodes H.264 in hardware, asked once per page. A
 * browser says so as `powerEfficient`. */
export function hardwareH264(): Promise<boolean> {
  probed ??= Promise.race([
    probe().catch(() => false),
    new Promise<boolean>((resolve) =>
      setTimeout(() => resolve(false), PROBE_TIMEOUT_MS),
    ),
  ]);
  return probed;
}

async function probe(): Promise<boolean> {
  if (isDesktopApp())
    return (await loadNativeFeatures()).has(H264_HARDWARE_FEATURE);
  const capabilities =
    typeof navigator === "undefined" ? undefined : navigator.mediaCapabilities;
  if (!capabilities?.encodingInfo) return false;
  const info = await capabilities.encodingInfo({
    type: "webrtc",
    video: {
      contentType: "video/H264",
      width: 1920,
      height: 1080,
      bitrate: 4_000_000,
      framerate: 30,
    },
  });
  return info.supported && info.powerEfficient;
}

/** Tests: asks again on the next call. */
export function resetHardwareH264ForTests(): void {
  probed = null;
}
