// The codec a video source is sent in (issue #165). mediasoup forwards what
// it gets and transcodes nothing, so every receiver in the room has to decode
// the publisher's codec: the server names the codecs they all decode
// (`MediaResults["capabilities"]`, the `codecs` event). Among those, AV1
// wins where this device encodes it in hardware: the same picture for less
// bitrate at little CPU (issue #318). Then H.264 where it is encoded in
// hardware, which costs the least CPU. Otherwise VP8 in three simulcast
// layers, which every WebRTC stack encodes and decodes. VP9 SVC does not
// come before it: libwebrtc caps each VP9 spatial layer at about
// 1.6 x pixels + 50 kbit/s whatever the frame rate, far below the ceilings
// VP8 layers get. VP9 follows H.264 and goes out in three layers
// (`VP9_SVC_MODE`) where it is the best codec the room has and this device
// can send SVC. AV1 on the CPU costs far more than VP8 and comes last.
//
// AV1 goes out as one stream, never in simulcast layers, so every receiver
// decodes the whole picture. A device that cannot decode 4K AV1 smoothly
// (hardware or not; the desktop app says so with `av1-hardware-decode`)
// leaves AV1 out of the codecs it tells the server it receives
// (`receivableCapabilities`): the room then has no AV1, and senders stay on
// H.264 or VP8 with layers. Sending AV1 still needs a hardware encoder.

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

/** Which codecs this device encodes in hardware, and whether it encodes VP9
 * in three spatial layers (in software, which is fine). */
export type HardwareVideo = {
  av1: boolean;
  h264: boolean;
  vp9Svc: boolean;
};

export const SOFTWARE_VIDEO: HardwareVideo = {
  av1: false,
  h264: false,
  vp9Svc: false,
};

/** The scalability mode VP9 is sent in: three spatial and three temporal
 * layers, with inter-layer prediction on key frames only. */
export const VP9_SVC_MODE = "L3T3_KEY";

/** The VP9 every receiver decodes: profile 0 (8-bit 4:2:0). Profile 2
 * (10-bit) is not decodable everywhere. */
export function publishableVp9(codec: RtpCodecCapability): boolean {
  if (codec.mimeType.toLowerCase() !== VP9) return false;
  const parameters = (codec.parameters ?? {}) as Record<string, unknown>;
  return Number(parameters["profile-id"] ?? 0) === 0;
}

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
      const codec = find(
        mime,
        (c) =>
          (mime !== H264 || publishableH264(c)) &&
          (mime !== VP9 || publishableVp9(c)),
      );
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
 * AV1 (and its RTX) unless `av1Decoding`, the device decoding AV1
 * smoothly at 4K. */
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
/** The desktop app's feature when its core encodes VP9 in spatial layers. */
export const VP9_SVC_FEATURE: NativeFeature = "vp9-svc";

/** How long the browser may take to say; past it, software is assumed. */
const PROBE_TIMEOUT_MS = 1000;

/** The desktop app may be busy starting its media core. */
const DESKTOP_PROBE_TIMEOUT_MS = 10_000;

type Probed = HardwareVideo & { av1Decoding: boolean };

let probed: Promise<Probed> | null = null;

function probeOnce(): Promise<Probed> {
  if (!probed) {
    const mine = probe()
      .catch(() => ({
        ...SOFTWARE_VIDEO,
        av1Decoding: false,
        timedOut: false,
      }))
      .then(({ timedOut, ...result }) => {
        // A guess made for lack of an answer is not kept: ask again later.
        if (timedOut && probed === mine) probed = null;
        return result;
      });
    probed = mine;
  }
  return probed;
}

/** Which codecs this device encodes in hardware, asked once per page. A
 * browser says so as `powerEfficient`. */
export async function hardwareVideo(): Promise<HardwareVideo> {
  const { av1, h264, vp9Svc } = await probeOnce();
  return { av1, h264, vp9Svc };
}

/** Whether this device decodes AV1 smoothly at 4K, in hardware or not,
 * asked with the encoders. */
export async function decodesAv1(): Promise<boolean> {
  return (await probeOnce()).av1Decoding;
}

async function probe(): Promise<Probed & { timedOut: boolean }> {
  if (isDesktopApp()) {
    const [features, timedOut] = await withTimeout(
      loadNativeFeatures(),
      new Set<string>(),
      DESKTOP_PROBE_TIMEOUT_MS,
    );
    return {
      av1: features.has(AV1_HARDWARE_FEATURE),
      h264: features.has(H264_HARDWARE_FEATURE),
      vp9Svc: features.has(VP9_SVC_FEATURE),
      av1Decoding: features.has(AV1_HARDWARE_DECODE_FEATURE),
      timedOut,
    };
  }
  const answers = await Promise.all([
    withTimeout(powerEfficient("video/AV1"), false),
    withTimeout(powerEfficient("video/H264"), false),
    withTimeout(layeredVp9(), false),
    withTimeout(smoothDecoding("video/AV1"), false),
  ]);
  const [av1, h264, vp9Svc, av1Decoding] = answers.map(([answer]) => answer);
  return {
    av1,
    h264,
    vp9Svc,
    av1Decoding,
    timedOut: answers.some(([, timedOut]) => timedOut),
  };
}

async function powerEfficient(contentType: string): Promise<boolean> {
  const capabilities =
    typeof navigator === "undefined" ? undefined : navigator.mediaCapabilities;
  if (!capabilities?.encodingInfo) return false;
  const info = await capabilities.encodingInfo({
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

/** Whether the browser encodes VP9 in `VP9_SVC_MODE`. Chromium does so on
 * the CPU, so `powerEfficient` is not asked. */
async function layeredVp9(): Promise<boolean> {
  const capabilities =
    typeof navigator === "undefined" ? undefined : navigator.mediaCapabilities;
  if (!capabilities?.encodingInfo) return false;
  const info = await capabilities.encodingInfo({
    type: "webrtc",
    video: {
      contentType: "video/VP9",
      width: 1920,
      height: 1080,
      bitrate: 4_000_000,
      framerate: 30,
      scalabilityMode: VP9_SVC_MODE,
    },
  });
  return info.supported;
}

/** Whether the browser decodes `contentType` at 4K without dropping frames;
 * a CPU decoder that keeps up is as good as a hardware one. */
async function smoothDecoding(contentType: string): Promise<boolean> {
  const capabilities =
    typeof navigator === "undefined" ? undefined : navigator.mediaCapabilities;
  if (!capabilities?.decodingInfo) return false;
  const info = await capabilities.decodingInfo({
    type: "webrtc",
    video: {
      contentType,
      width: 3840,
      height: 2160,
      bitrate: 20_000_000,
      framerate: 30,
    },
  });
  return info.supported && info.smooth;
}

/** The answer, or `fallback` after `ms` or a failure; the flag says the time
 * ran out. */
function withTimeout<T>(
  answer: Promise<T>,
  fallback: T,
  ms = PROBE_TIMEOUT_MS,
): Promise<[T, boolean]> {
  let timer: ReturnType<typeof setTimeout>;
  return Promise.race([
    answer.then(
      (value): [T, boolean] => [value, false],
      (): [T, boolean] => [fallback, false],
    ),
    new Promise<[T, boolean]>((resolve) => {
      timer = setTimeout(() => resolve([fallback, true]), ms);
    }),
  ]).finally(() => clearTimeout(timer));
}

/** Tests: asks again on the next call. */
export function resetHardwareVideoForTests(): void {
  probed = null;
}
