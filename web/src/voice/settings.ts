// Voice/video + in-app notification prefs. Persisted per browser.
// Capture quality and microphone processing are independent of bandwidth.
// No application bitrate limit unless the user explicitly selects economy/custom.
// Capture devices are the browser's own list (the native engine's in the
// desktop app).

import { create } from "zustand";
import { createJSONStorage, persist } from "zustand/middleware";
import { isDesktopApp } from "./native/bridge.ts";
import { listNativeDevices } from "./native/capture.ts";
import { hasNativeFeature } from "./native/features.ts";

/** Explicit economy presets retained for migration; never applied by default. */
export const AUDIO_QUALITY = {
  phone: {
    bitrate: 24_000,
    label: "Telefon",
    hint: "24 kbit/s",
  },
  normal: {
    bitrate: 64_000,
    label: "Normal",
    hint: "64 kbit/s",
  },
  high: {
    bitrate: 128_000,
    label: "Hoch",
    hint: "128 kbit/s",
  },
} as const;

export type AudioQuality = keyof typeof AUDIO_QUALITY;
export type AudioProcessingMode = "enhanced" | "browser" | "original";
export const AUDIO_PROCESSING = {
  enhanced: {
    label: "Verbesserte Sprache",
    hint: "Lokale Rauschunterdrückung; Browserfilter als Ersatz",
  },
  browser: {
    label: "Browserfilter",
    hint: "Rauschunterdrückung und Auto-Gain deines Browsers",
  },
  original: {
    label: "Original / Musik",
    hint: "Stereo sofern unterstützt, ohne Rauschfilter, Auto-Gain oder Sprechpausen-Erkennung",
  },
} as const;
/** The same modes as the desktop app words them: its native core does the
 * filtering, no browser is involved. */
export const AUDIO_PROCESSING_DESKTOP: Record<
  AudioProcessingMode,
  { label: string; hint: string }
> = {
  enhanced: {
    label: AUDIO_PROCESSING.enhanced.label,
    hint: "Lokale Rauschunterdrückung",
  },
  browser: {
    label: "WebRTC-Filter",
    hint: "Rauschunterdrückung und Auto-Gain von WebRTC",
  },
  original: AUDIO_PROCESSING.original,
};
export function asProcessingMode(value: unknown): AudioProcessingMode {
  return value === "browser" || value === "original" ? value : "enhanced";
}

/** Sound for a share: the user's own choice, or "auto" while he has made
 * none. Up to v0.5 this was a boolean, off by default. */
export type SourceAudioShare = "auto" | "on" | "off";
export function asSourceAudioShare(value: unknown): SourceAudioShare {
  return value === "on" || value === "off" ? value : "auto";
}

/** Display-source music stays separate from the speech encoder. */
export const SOURCE_AUDIO_CONSTRAINTS: MediaTrackConstraints = {
  echoCancellation: false,
  noiseSuppression: false,
  autoGainControl: false,
  channelCount: 2,
};

/** Explicit capture targets. Legacy presets remain valid for saved preferences. */
export const VIDEO_RESOLUTIONS = [480, 720, 1080, 1440, 2160] as const;
export const VIDEO_FRAME_RATES = [15, 24, 30, 45, 60] as const;
export type VideoResolution = (typeof VIDEO_RESOLUTIONS)[number];
export type VideoFrameRate = (typeof VIDEO_FRAME_RATES)[number];
type ExplicitProfileId = `${VideoResolution}p${VideoFrameRate}`;
type StreamProfile = {
  label: string;
  width: number;
  height: number;
  fps: number;
};
const resolutionSpecs = {
  480: { width: 854 },
  720: { width: 1280 },
  1080: { width: 1920 },
  1440: { width: 2560 },
  2160: { width: 3840 },
} as const;
export function explicitStreamProfile(
  height: VideoResolution,
  fps: VideoFrameRate,
): ExplicitProfileId {
  return `${height}p${fps}`;
}
const explicitProfiles = Object.fromEntries(
  VIDEO_RESOLUTIONS.flatMap((height) =>
    VIDEO_FRAME_RATES.map((fps) => [
      explicitStreamProfile(height, fps),
      {
        label: `${height === 2160 ? "4K" : `${height}p`} · ${fps} FPS`,
        width: resolutionSpecs[height].width,
        height,
        fps,
      },
    ]),
  ),
) as Record<ExplicitProfileId, StreamProfile>;
export const STREAM_PROFILES = {
  economy: {
    label: "Sparsam",
    width: 854,
    height: 480,
    fps: 15,
  },
  balanced: {
    label: "Automatisch",
    width: 1280,
    height: 720,
    fps: 30,
  },
  detail: {
    label: "Detail",
    width: 1920,
    height: 1080,
    fps: 30,
  },
  ...explicitProfiles,
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
  /** Whether a share carries sound; `sharesSourceAudio` reads it. */
  sourceAudioShare: SourceAudioShare;
  /** Desktop app: application whose sound is shared ("" = every application
   * but Gelabber). The browser picks the source in its own dialog. */
  sourceAudioApp: string;
  sourceAudioVolume: number;
  sourceAudioMuted: boolean;
  quality: AudioQuality;
  processingMode: AudioProcessingMode;
  /** Explicit bandwidth-saving option; false means no application audio/video cap. */
  economyMode: boolean;
  /** Camera capture + that sender's share of the video budget. */
  cameraProfile: StreamProfileId;
  /** Screen share and Go Live. Both are display captures. */
  screenProfile: StreamProfileId;
  /** 0 = no custom limit; otherwise explicit shared video payload cap in bit/s. */
  videoUploadLimit: number;
  callSounds: boolean;
  callSoundVolume: number;
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
  sourceAudioShare: "auto",
  sourceAudioApp: "",
  sourceAudioVolume: 1,
  sourceAudioMuted: false,
  quality: "normal",
  processingMode: "enhanced",
  economyMode: false,
  cameraProfile: "balanced",
  screenProfile: "balanced",
  videoUploadLimit: 0,
  callSounds: true,
  callSoundVolume: 0.35,
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
  return typeof value === "string" && Object.hasOwn(STREAM_PROFILES, value)
    ? (value as StreamProfileId)
    : "balanced";
}

export function clampVideoUploadLimit(value: unknown): number {
  if (typeof value !== "number" || !Number.isFinite(value) || value <= 0)
    return 0;
  return Math.round(Math.min(0xffff_ffff, Math.max(1, value)));
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
    sourceAudioShare: state.sourceAudioShare,
    sourceAudioApp: state.sourceAudioApp,
    sourceAudioVolume: state.sourceAudioVolume,
    sourceAudioMuted: state.sourceAudioMuted,
    quality: state.quality,
    processingMode: state.processingMode,
    economyMode: state.economyMode,
    cameraProfile: state.cameraProfile,
    screenProfile: state.screenProfile,
    videoUploadLimit: state.videoUploadLimit,
    callSounds: state.callSounds,
    callSoundVolume: state.callSoundVolume,
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
          sourceAudioShare:
            partial.sourceAudioShare !== undefined
              ? asSourceAudioShare(partial.sourceAudioShare)
              : prev.sourceAudioShare,
          sourceAudioApp: partial.sourceAudioApp ?? prev.sourceAudioApp,
          sourceAudioVolume:
            partial.sourceAudioVolume !== undefined
              ? clampVolume(partial.sourceAudioVolume)
              : prev.sourceAudioVolume,
          sourceAudioMuted: partial.sourceAudioMuted ?? prev.sourceAudioMuted,
          quality:
            partial.quality !== undefined
              ? asQuality(partial.quality)
              : prev.quality,
          processingMode:
            partial.processingMode !== undefined
              ? asProcessingMode(partial.processingMode)
              : prev.processingMode,
          economyMode: partial.economyMode ?? prev.economyMode,
          cameraProfile:
            partial.cameraProfile !== undefined
              ? asStreamProfile(partial.cameraProfile)
              : prev.cameraProfile,
          screenProfile:
            partial.screenProfile !== undefined
              ? asStreamProfile(partial.screenProfile)
              : prev.screenProfile,
          videoUploadLimit:
            partial.videoUploadLimit !== undefined
              ? clampVideoUploadLimit(partial.videoUploadLimit)
              : prev.videoUploadLimit,
          callSounds: partial.callSounds ?? prev.callSounds,
          callSoundVolume:
            partial.callSoundVolume !== undefined
              ? clampVolume(partial.callSoundVolume)
              : prev.callSoundVolume,
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
        // `shareSourceAudio` is the boolean of v0.5 and earlier.
        const { shareSourceAudio: sharedBefore, ...stored } =
          persisted as Partial<MediaSettings> & { shareSourceAudio?: unknown };
        return {
          ...current,
          ...stored,
          quality: asQuality(stored.quality),
          processingMode:
            "processingMode" in stored
              ? asProcessingMode(stored.processingMode)
              : stored.noiseSuppression === false &&
                  stored.autoGainControl === false
                ? "original"
                : "browser",
          economyMode:
            "economyMode" in stored
              ? stored.economyMode === true
              : stored.quality === "phone",
          callSounds:
            typeof stored.callSounds === "boolean" ? stored.callSounds : true,
          callSoundVolume: clampVolume(stored.callSoundVolume ?? 0.35),
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
          videoUploadLimit: clampVideoUploadLimit(stored.videoUploadLimit),
          // The old false was the default as well as a choice, and its
          // control had moved out of sight: only an old true is a choice.
          sourceAudioShare:
            "sourceAudioShare" in stored
              ? asSourceAudioShare(stored.sourceAudioShare)
              : sharedBefore === true
                ? "on"
                : "auto",
          sourceAudioApp:
            typeof stored.sourceAudioApp === "string"
              ? stored.sourceAudioApp
              : "",
          sourceAudioVolume: clampVolume(stored.sourceAudioVolume ?? 1),
          sourceAudioMuted: stored.sourceAudioMuted === true,
          cameraProfileApply: current.cameraProfileApply,
          screenProfileApply: current.screenProfileApply,
          dialogOpen: current.dialogOpen,
        };
      },
    },
  ),
);

export function audioBitrate(
  settings: Pick<
    MediaSettings,
    "economyMode" | "quality"
  > = useMediaSettings.getState(),
): number | null {
  return settings.economyMode ? AUDIO_QUALITY[settings.quality].bitrate : null;
}
export function sourceAudioBitrate(
  settings: Pick<MediaSettings, "economyMode"> = useMediaSettings.getState(),
): number | null {
  return settings.economyMode ? 128_000 : null;
}

/** Whether screen share and Go Live carry sound. Without a choice of his own
 * the user gets it where the call stays out of it: in a desktop app whose
 * "every application" leaves out Gelabber itself. A browser captures whatever
 * its picker hands over, the call included, so there it stays off. */
export function sharesSourceAudio(
  settings: Pick<
    MediaSettings,
    "sourceAudioShare"
  > = useMediaSettings.getState(),
): boolean {
  if (isDesktopApp() && !hasNativeFeature("app-audio")) return false;
  if (settings.sourceAudioShare !== "auto")
    return settings.sourceAudioShare === "on";
  return hasNativeFeature("app-audio-excludes-self");
}

/** What the switch stores. Off where a share carries no sound unasked is no
 * choice to keep: remembered, it would hold the sound back in a desktop app
 * that leaves the call out, once the user has updated to one. */
export function sourceAudioChoice(on: boolean): SourceAudioShare {
  if (on) return "on";
  return sharesSourceAudio({ sourceAudioShare: "auto" }) ? "off" : "auto";
}

/** A desktop app before v0.6 captures its own playout with "every
 * application": switched on there, a share also carries the call. */
export const SOURCE_AUDIO_CARRIES_CALL =
  "Diese App-Version sendet mit dem Stream-Ton auch den Ton des Anrufs; ein Update der Desktop-App behebt das.";
export function sourceAudioCarriesCall(
  settings: Pick<
    MediaSettings,
    "sourceAudioShare" | "sourceAudioApp"
  > = useMediaSettings.getState(),
): boolean {
  return (
    hasNativeFeature("app-audio") &&
    !hasNativeFeature("app-audio-excludes-self") &&
    settings.sourceAudioShare === "on" &&
    settings.sourceAudioApp === ""
  );
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

export function micConstraints(
  settings: MediaSettings = useMediaSettings.getState(),
  mode = settings.processingMode,
): MediaTrackConstraints {
  return {
    echoCancellation: settings.echoCancellation,
    noiseSuppression: mode === "browser" && settings.noiseSuppression,
    autoGainControl: mode === "browser" && settings.autoGainControl,
    channelCount: mode === "original" ? { ideal: 2 } : 1,
    sampleRate: { ideal: 48_000 },
    ...(settings.audioInputId
      ? { deviceId: { ideal: settings.audioInputId } }
      : {}),
  };
}

/** Shared video pool only when the user explicitly enables economy. */
export const VIDEO_SEND_BUDGET = 2_500_000;
export const VIDEO_MAX_FPS = 60;

const BALANCED_CAMERA: MediaTrackConstraints = {
  width: { ideal: 1280, max: 1920 },
  height: { ideal: 720, max: 1080 },
  frameRate: { ideal: 30, max: 30 },
};

const BALANCED_DISPLAY: MediaTrackConstraints = {
  width: { max: 1920 },
  height: { max: 1080 },
  frameRate: { ideal: 15, max: 30 },
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
  maxBitrate: number | null;
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
      maxBitrate: null,
    };
  }
  if (profile === "balanced" && kind === "screen") {
    return {
      resolution: "max. 1080p",
      fps: "15 FPS ideal, max. 30",
      maxBitrate: null,
    };
  }
  return {
    resolution: spec.height === 2160 ? "4K (2160p)" : `${spec.height}p`,
    fps: `${spec.fps} FPS`,
    maxBitrate: null,
  };
}

export function formatVideoBitrate(bitsPerSecond: number): string {
  const rounded = Math.round((bitsPerSecond / 1_000_000) * 10) / 10;
  const text = Number.isInteger(rounded)
    ? String(rounded)
    : rounded.toFixed(1).replace(".", ",");
  return `${text} Mbit/s`;
}

/** Explicit limits only. Capture profiles determine resolution/FPS, not bitrate. */
export function allocateVideoBitrates(
  profiles: readonly StreamProfileId[],
  uploadLimit = 0,
  economyMode = false,
): Array<number | null> {
  if (profiles.length === 0) return [];
  const pool =
    clampVideoUploadLimit(uploadLimit) || (economyMode ? VIDEO_SEND_BUDGET : 0);
  if (!pool) return profiles.map(() => null);
  const weights = profiles.map(
    (id) =>
      STREAM_PROFILES[asStreamProfile(id)].width *
      STREAM_PROFILES[asStreamProfile(id)].height *
      STREAM_PROFILES[asStreamProfile(id)].fps,
  );
  const sum = weights.reduce((total, value) => total + value, 0);
  return weights.map((weight) =>
    Math.max(1, Math.floor((pool * weight) / sum)),
  );
}

export function videoSendBudget(
  settings: Pick<
    MediaSettings,
    "cameraProfile" | "screenProfile" | "videoUploadLimit"
  > &
    Partial<Pick<MediaSettings, "economyMode">>,
): number | null {
  return (
    clampVideoUploadLimit(settings.videoUploadLimit) ||
    (settings.economyMode ? VIDEO_SEND_BUDGET : null)
  );
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
      ? profile === "economy"
        ? ["economy"]
        : profile === "balanced"
          ? ["balanced", "economy"]
          : [profile, "balanced", "economy"]
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
  if (isDesktopApp()) return listNativeDevices();
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
