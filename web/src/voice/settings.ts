// Voice/video + in-app notification prefs. Persisted per browser.
// Defaults: AEC/NS/AGC on, Opus "Normal" 64 kbit/s, full playback volume,
// message toasts on, stream profile "Ausgewogen" (the previous safe caps).
// No product SDK; devices are the browser's own list.

import { create } from "zustand";
import { createJSONStorage, persist } from "zustand/middleware";

/** Opus send caps. Default is Normal — not the previous always-128 kbit/s. */
export const AUDIO_QUALITY = {
  phone: {
    bitrate: 24_000,
    label: "Telefon",
    hint: "24 kbit/s",
  },
  normal: {
    bitrate: 64_000,
    label: "Normal",
    hint: "64 kbit/s — Default",
  },
  high: {
    bitrate: 128_000,
    label: "Hoch",
    hint: "128 kbit/s",
  },
} as const;

export type AudioQuality = keyof typeof AUDIO_QUALITY;

/**
 * Camera and screen/Go Live presets. "balanced" keeps the previous safe
 * capture targets and the 2.5 Mbit/s send budget. Economy and Detail are
 * explicit steps inside a hard ceiling — video bitrate is never unlimited.
 */
export const STREAM_PROFILES = {
  economy: {
    label: "Sparsam",
    width: 854,
    height: 480,
    fps: 15,
    maxBitrate: 800_000,
  },
  balanced: {
    label: "Ausgewogen",
    width: 1280,
    height: 720,
    fps: 30,
    maxBitrate: 2_500_000,
  },
  detail: {
    label: "Detail",
    width: 1920,
    height: 1080,
    fps: 30,
    maxBitrate: 4_000_000,
  },
} as const;

export type StreamProfileId = keyof typeof STREAM_PROFILES;
export type StreamKind = "camera" | "screen";
/** Whether a profile change reached the live track or waits for the next one. */
export type StreamApply = "idle" | "live" | "next";

export type MediaSettings = {
  audioInputId: string;
  audioOutputId: string;
  videoInputId: string;
  echoCancellation: boolean;
  noiseSuppression: boolean;
  autoGainControl: boolean;
  /** Remote mix, 0–1. Mute/Deafen still win. */
  outputVolume: number;
  /** Pre-send gain, 0–2. 1 = identity (no Web Audio insert). */
  inputGain: number;
  quality: AudioQuality;
  /** Camera capture + that sender's share of the video budget. */
  cameraProfile: StreamProfileId;
  /** Screen share and Go Live. Both are display captures. */
  screenProfile: StreamProfileId;
  messageToasts: boolean;
  desktopNotify: boolean;
};

export const DEFAULT_MEDIA_SETTINGS: MediaSettings = {
  audioInputId: "",
  audioOutputId: "",
  videoInputId: "",
  echoCancellation: true,
  noiseSuppression: true,
  autoGainControl: true,
  outputVolume: 1,
  inputGain: 1,
  quality: "normal",
  cameraProfile: "balanced",
  screenProfile: "balanced",
  messageToasts: true,
  desktopNotify: false,
};

type MediaSettingsState = MediaSettings & {
  dialogOpen: boolean;
  /** Not persisted. Set by the live session after a profile change. */
  cameraProfileApply: StreamApply;
  screenProfileApply: StreamApply;
  patch: (partial: Partial<MediaSettings>) => void;
  openDialog: () => void;
  closeDialog: () => void;
};

let listener: ((prev: MediaSettings, next: MediaSettings) => void) | null =
  null;

const memoryStore = new Map<string, string>();

const memoryStorage = {
  getItem: (key: string) => memoryStore.get(key) ?? null,
  setItem: (key: string, value: string) => {
    memoryStore.set(key, value);
  },
  removeItem: (key: string) => {
    memoryStore.delete(key);
  },
};

/** Browser localStorage, or an in-memory stand-in when this module loads in Node. */
function mediaStorage() {
  try {
    if (typeof localStorage !== "undefined") return localStorage;
  } catch {
    // SecurityError in some private-mode browsers.
  }
  return memoryStorage;
}

/** Session registers this so a slider change hits the live peer without reload. */
export function onMediaSettingsChange(
  fn: ((prev: MediaSettings, next: MediaSettings) => void) | null,
): void {
  listener = fn;
}

export function clampVolume(value: number): number {
  if (!Number.isFinite(value)) return 1;
  return Math.min(1, Math.max(0, value));
}

export function clampGain(value: number): number {
  if (!Number.isFinite(value)) return 1;
  return Math.min(2, Math.max(0, value));
}

export function asQuality(value: unknown): AudioQuality {
  return value === "phone" || value === "high" || value === "normal"
    ? value
    : "normal";
}

export function asStreamProfile(value: unknown): StreamProfileId {
  return value === "economy" || value === "detail" || value === "balanced"
    ? value
    : "balanced";
}

function snapshot(state: MediaSettingsState): MediaSettings {
  return {
    audioInputId: state.audioInputId,
    audioOutputId: state.audioOutputId,
    videoInputId: state.videoInputId,
    echoCancellation: state.echoCancellation,
    noiseSuppression: state.noiseSuppression,
    autoGainControl: state.autoGainControl,
    outputVolume: state.outputVolume,
    inputGain: state.inputGain,
    quality: state.quality,
    cameraProfile: state.cameraProfile,
    screenProfile: state.screenProfile,
    messageToasts: state.messageToasts,
    desktopNotify: state.desktopNotify,
  };
}

export const useMediaSettings = create<MediaSettingsState>()(
  persist(
    (set, get) => ({
      ...DEFAULT_MEDIA_SETTINGS,
      dialogOpen: false,
      cameraProfileApply: "idle",
      screenProfileApply: "idle",
      patch: (partial) => {
        const prev = snapshot(get());
        set({
          audioInputId:
            partial.audioInputId !== undefined
              ? partial.audioInputId
              : prev.audioInputId,
          audioOutputId:
            partial.audioOutputId !== undefined
              ? partial.audioOutputId
              : prev.audioOutputId,
          videoInputId:
            partial.videoInputId !== undefined
              ? partial.videoInputId
              : prev.videoInputId,
          echoCancellation:
            partial.echoCancellation !== undefined
              ? partial.echoCancellation
              : prev.echoCancellation,
          noiseSuppression:
            partial.noiseSuppression !== undefined
              ? partial.noiseSuppression
              : prev.noiseSuppression,
          autoGainControl:
            partial.autoGainControl !== undefined
              ? partial.autoGainControl
              : prev.autoGainControl,
          outputVolume:
            partial.outputVolume !== undefined
              ? clampVolume(partial.outputVolume)
              : prev.outputVolume,
          inputGain:
            partial.inputGain !== undefined
              ? clampGain(partial.inputGain)
              : prev.inputGain,
          quality:
            partial.quality !== undefined
              ? asQuality(partial.quality)
              : prev.quality,
          cameraProfile:
            partial.cameraProfile !== undefined
              ? asStreamProfile(partial.cameraProfile)
              : prev.cameraProfile,
          screenProfile:
            partial.screenProfile !== undefined
              ? asStreamProfile(partial.screenProfile)
              : prev.screenProfile,
          messageToasts:
            partial.messageToasts !== undefined
              ? partial.messageToasts
              : prev.messageToasts,
          desktopNotify:
            partial.desktopNotify !== undefined
              ? partial.desktopNotify
              : prev.desktopNotify,
        });
        listener?.(prev, snapshot(get()));
      },
      openDialog: () => set({ dialogOpen: true }),
      closeDialog: () => set({ dialogOpen: false }),
    }),
    {
      name: "gelabber.media",
      storage: createJSONStorage(mediaStorage),
      partialize: (state) => snapshot(state),
      merge: (persisted, current) => {
        if (!persisted || typeof persisted !== "object") return current;
        const stored = persisted as Partial<MediaSettings>;
        return {
          ...current,
          ...stored,
          cameraProfile: asStreamProfile(
            "cameraProfile" in stored
              ? stored.cameraProfile
              : current.cameraProfile,
          ),
          screenProfile: asStreamProfile(
            "screenProfile" in stored
              ? stored.screenProfile
              : current.screenProfile,
          ),
          cameraProfileApply: current.cameraProfileApply,
          screenProfileApply: current.screenProfileApply,
          dialogOpen: current.dialogOpen,
        };
      },
    },
  ),
);

export function audioBitrate(): number {
  return AUDIO_QUALITY[useMediaSettings.getState().quality].bitrate;
}

export function resetMediaSettingsForTests(): void {
  useMediaSettings.setState({
    ...DEFAULT_MEDIA_SETTINGS,
    dialogOpen: false,
    cameraProfileApply: "idle",
    screenProfileApply: "idle",
  });
}

export function noteStreamProfileApply(
  kind: StreamKind,
  apply: StreamApply,
): void {
  if (kind === "camera") {
    useMediaSettings.setState({ cameraProfileApply: apply });
    return;
  }
  useMediaSettings.setState({ screenProfileApply: apply });
}

export function micConstraints(): MediaTrackConstraints {
  const settings = useMediaSettings.getState();
  return {
    echoCancellation: settings.echoCancellation,
    noiseSuppression: settings.noiseSuppression,
    autoGainControl: settings.autoGainControl,
    channelCount: 1,
    ...(settings.audioInputId
      ? { deviceId: { ideal: settings.audioInputId } }
      : {}),
  };
}

/**
 * Previous single-source cap. Still the balanced budget, and still the pool
 * when every active source is balanced — concurrent sources share it.
 */
export const VIDEO_SEND_BUDGET = STREAM_PROFILES.balanced.maxBitrate;
/** Hard ceiling for every video sender combined. Detail cannot exceed this. */
export const VIDEO_SEND_CEILING = STREAM_PROFILES.detail.maxBitrate;
export const VIDEO_MAX_FPS = STREAM_PROFILES.detail.fps;

const BALANCED_CAMERA: MediaTrackConstraints = {
  width: { ideal: 1280, max: 1920 },
  height: { ideal: 720, max: 1080 },
  frameRate: { ideal: VIDEO_MAX_FPS, max: VIDEO_MAX_FPS },
};

const BALANCED_DISPLAY: MediaTrackConstraints = {
  width: { max: 1920 },
  height: { max: 1080 },
  frameRate: { ideal: 15, max: VIDEO_MAX_FPS },
};

export function streamProfileFps(profile: StreamProfileId): number {
  return Math.min(VIDEO_MAX_FPS, STREAM_PROFILES[profile].fps);
}

/** Capture target for a profile. Balanced keeps the previous safe constraints. */
export function videoConstraintsFor(
  kind: StreamKind,
  profile: StreamProfileId,
): MediaTrackConstraints {
  if (profile === "balanced") {
    return kind === "camera" ? { ...BALANCED_CAMERA } : { ...BALANCED_DISPLAY };
  }
  const spec = STREAM_PROFILES[profile];
  const fps = streamProfileFps(profile);
  return {
    width: { ideal: spec.width, max: spec.width },
    height: { ideal: spec.height, max: spec.height },
    frameRate: { ideal: fps, max: fps },
  };
}

export type StreamEstimate = {
  resolution: string;
  fps: string;
  maxBitrate: number;
};

/** What the settings UI shows — same numbers the capturer and sender use. */
export function streamEstimate(
  kind: StreamKind,
  profile: StreamProfileId,
): StreamEstimate {
  const spec = STREAM_PROFILES[profile];
  if (profile === "balanced" && kind === "camera") {
    return {
      resolution: "720p ideal, max. 1080p",
      fps: "30 FPS",
      maxBitrate: spec.maxBitrate,
    };
  }
  if (profile === "balanced" && kind === "screen") {
    return {
      resolution: "max. 1080p",
      fps: "15 FPS ideal, max. 30",
      maxBitrate: spec.maxBitrate,
    };
  }
  return {
    resolution: `${spec.height}p`,
    fps: `${spec.fps} FPS`,
    maxBitrate: spec.maxBitrate,
  };
}

export function formatVideoBitrate(bitsPerSecond: number): string {
  const rounded = Math.round((bitsPerSecond / 1_000_000) * 10) / 10;
  const text = Number.isInteger(rounded)
    ? String(rounded)
    : rounded.toFixed(1).replace(".", ",");
  return `${text} Mbit/s`;
}

/**
 * Share one pool across active video sources. The pool is the largest
 * single-source budget in the set, never above {@link VIDEO_SEND_CEILING}.
 * Two balanced sources therefore still split 2.5 Mbit/s in half.
 */
export function allocateVideoBitrates(
  profiles: readonly StreamProfileId[],
): number[] {
  if (profiles.length === 0) return [];
  const desires = profiles.map(
    (id) => STREAM_PROFILES[asStreamProfile(id)].maxBitrate,
  );
  const pool = Math.min(VIDEO_SEND_CEILING, Math.max(...desires));
  const sum = desires.reduce((total, value) => total + value, 0);
  const scale = sum > pool ? pool / sum : 1;
  return desires.map((desire) => Math.floor(desire * scale));
}

export function isOverconstrainedError(error: unknown): boolean {
  if (!error || typeof error !== "object") return false;
  const name = (error as { name?: string }).name;
  return (
    name === "OverconstrainedError" || name === "ConstraintNotSatisfiedError"
  );
}

/**
 * Steps to try when the browser rejects a capture constraint. Camera can
 * walk all the way to an unconstrained track (permission is already granted).
 * Screen share retries at most once so a rejected profile does not stack
 * display pickers.
 */
export function videoConstraintLadder(
  kind: StreamKind,
  profile: StreamProfileId,
  deviceId = "",
): MediaTrackConstraints[] {
  const order: StreamProfileId[] =
    kind === "camera"
      ? profile === "detail"
        ? ["detail", "balanced", "economy"]
        : profile === "balanced"
          ? ["balanced", "economy"]
          : ["economy"]
      : profile === "balanced"
        ? ["balanced"]
        : [profile, "balanced"];
  const steps = order.map((id) =>
    withDevice(
      videoConstraintsFor(kind, id),
      kind === "camera" ? deviceId : "",
    ),
  );
  if (kind === "camera") {
    steps.push(withDevice({}, deviceId));
  }
  return steps;
}

function withDevice(
  constraints: MediaTrackConstraints,
  deviceId: string,
): MediaTrackConstraints {
  if (!deviceId) return constraints;
  return { ...constraints, deviceId: { ideal: deviceId } };
}

export function displayConstraints(): MediaTrackConstraints {
  return videoConstraintsFor(
    "screen",
    useMediaSettings.getState().screenProfile,
  );
}

export function cameraConstraints(): MediaTrackConstraints {
  const settings = useMediaSettings.getState();
  return withDevice(
    videoConstraintsFor("camera", settings.cameraProfile),
    settings.videoInputId,
  );
}

export type DeviceOption = { id: string; label: string };

export type DeviceList = {
  audioinput: DeviceOption[];
  audiooutput: DeviceOption[];
  videoinput: DeviceOption[];
};

export async function listMediaDevices(): Promise<DeviceList> {
  const empty: DeviceList = { audioinput: [], audiooutput: [], videoinput: [] };
  if (
    typeof navigator === "undefined" ||
    !navigator.mediaDevices?.enumerateDevices
  ) {
    return empty;
  }
  const devices = await navigator.mediaDevices.enumerateDevices();
  const out: DeviceList = { audioinput: [], audiooutput: [], videoinput: [] };
  for (const device of devices) {
    if (
      device.kind !== "audioinput" &&
      device.kind !== "audiooutput" &&
      device.kind !== "videoinput"
    ) {
      continue;
    }
    if (!device.deviceId) continue;
    out[device.kind].push({
      id: device.deviceId,
      label: device.label || device.deviceId,
    });
  }
  return out;
}
