// Voice/video + in-app notification prefs. Persisted per browser.
// Capture quality and microphone processing are independent of bandwidth.
// No application bitrate limit unless the user explicitly selects economy/custom.
// Capture devices are the browser's own list (the native engine's in the
// desktop app).

import { create } from "zustand";
import {
  createJSONStorage,
  persist,
  type PersistStorage,
} from "zustand/middleware";
import { isDesktopApp } from "./native/bridge.ts";
import { listNativeDevices } from "./native/capture.ts";
import { hasNativeFeature } from "./native/features.ts";
import {
  asVoiceInputMode,
  clampReleaseDelay,
  PTT_RELEASE_DELAY_DEFAULT,
  type VoiceInputMode,
} from "./pushToTalk.ts";

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
    hint: "Lokale Rauschunterdrückung und Auto-Gain; Browserfilter als Ersatz",
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
  /** Remote mix, 0–2. Above 1 is amplified. Mute/Deafen still win. */
  outputVolume: number;
  /** Pre-send gain, 0–2. 1 = identity (no Web Audio insert). */
  inputGain: number;
  /** Whether a share carries sound; `sharesSourceAudio` reads it. */
  sourceAudioShare: SourceAudioShare;
  /** Desktop app: application whose sound is shared ("" = every application
   * but Gelabber). The browser picks the source in its own dialog. */
  sourceAudioApp: string;
  /** Screen/Live audio, 0–2 like `outputVolume`. */
  sourceAudioVolume: number;
  sourceAudioMuted: boolean;
  quality: AudioQuality;
  processingMode: AudioProcessingMode;
  /** Explicit bandwidth-saving option; false means no application audio/video cap. */
  economyMode: boolean;
  /** Camera capture + that sender's share of the video budget. */
  cameraProfile: StreamProfileId;
  /** Live (a display capture). */
  screenProfile: StreamProfileId;
  /** 0 = no custom limit; otherwise explicit shared video payload cap in bit/s. */
  videoUploadLimit: number;
  callSounds: boolean;
  callSoundVolume: number;
  messageToasts: boolean;
  desktopNotify: boolean;
  /** A short cue with a message notification on a desktop (issue #247). */
  messageSound: boolean;
  /** Voice activity, or push-to-talk (pushToTalk.ts). */
  voiceInput: VoiceInputMode;
  /** The push-to-talk key: a `KeyboardEvent.code`, or `Mouse4`/`Mouse5`. */
  pttKey: string;
  /** The key as it was pressed when bound, for the settings. */
  pttKeyLabel: string;
  /** Milliseconds the microphone stays open after the key comes up. */
  pttReleaseDelay: number;
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
  messageSound: true,
  voiceInput: "activity",
  pttKey: "",
  pttKeyLabel: "",
  pttReleaseDelay: PTT_RELEASE_DELAY_DEFAULT,
};

/** The settings dialog's tab it opens on. */
export type MediaDialogSection = "audio" | "video" | "diagnostics";

type MediaSettingsState = MediaSettings & {
  dialogOpen: boolean;
  dialogSection: MediaDialogSection | null;
  /** Not persisted. Set by the live session after a profile change. */
  cameraProfileApply: StreamApply;
  screenProfileApply: StreamApply;
  patch: (partial: Partial<MediaSettings>) => void;
  openDialog: (section?: MediaDialogSection) => void;
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

const SETTINGS_KEY = "gelabber.media";
/** The stream-sound choice once more, in an entry of its own. A web client
 * up to v0.5 on the same storage (the server rolled back, and updated again
 * later) rewrites the settings with the keys it knows; this entry it leaves
 * alone. */
const SOURCE_AUDIO_SHARE_KEY = "gelabber.media.source-audio-share";

/** The settings as they are stored: with the stream-sound choice also as the
 * boolean a web client up to v0.5 reads. Only "on" is a choice it knows. */
type StoredMediaSettings = MediaSettings & { shareSourceAudio: boolean };

const backing = mediaStorage();
// Undefined only for a getter that throws.
const settingsJson = createJSONStorage<StoredMediaSettings>(() => backing)!;

/** The choice as this client stored it last, if it ever did. */
function keptSourceAudioShare(): SourceAudioShare | null {
  try {
    const kept = backing.getItem(SOURCE_AUDIO_SHARE_KEY);
    return kept === "auto" || kept === "on" || kept === "off" ? kept : null;
  } catch {
    return null;
  }
}

/** Saves the settings and, in step with them, the choice in its own entry.
 * Settings without a choice, which this client never writes, leave that
 * entry as it is. */
const settingsStorage: PersistStorage<StoredMediaSettings> = {
  getItem: (name) => settingsJson.getItem(name),
  setItem: (name, value) => {
    settingsJson.setItem(name, value);
    const choice: unknown =
      name === SETTINGS_KEY ? value.state.sourceAudioShare : undefined;
    if (choice !== "auto" && choice !== "on" && choice !== "off") return;
    try {
      backing.setItem(SOURCE_AUDIO_SHARE_KEY, choice);
    } catch {
      // The settings themselves are saved; only a rollback loses an "off".
    }
  },
  removeItem: (name) => {
    settingsJson.removeItem(name);
    if (name === SETTINGS_KEY) backing.removeItem(SOURCE_AUDIO_SHARE_KEY);
  },
};

/** The choice where a web client up to v0.5 saved last and dropped ours. Its
 * true is "on". Its false was the default as well as a choice, and its
 * control had moved out of sight, so by itself it is no choice; it stands for
 * the "off" this client had stored before it, which that boolean cannot
 * hold. After an "on" of ours, which that client read as true, a false is
 * its user switching sound off there. */
function restoredSourceAudioShare(sharedBefore: boolean): SourceAudioShare {
  if (sharedBefore) return "on";
  const kept = keptSourceAudioShare();
  return kept === "off" || kept === "on" ? "off" : "auto";
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

/** Highest playback volume: 400 %, amplified above 100 % and limited so
 * that a loud voice cannot clip. */
export const MAX_PLAYBACK_VOLUME = 4;

export function clampPlaybackVolume(value: number): number {
  if (!Number.isFinite(value)) return 1;
  return Math.min(MAX_PLAYBACK_VOLUME, Math.max(0, value));
}

/** Steps of a playback slider. The left half runs from 0 to 100 %, the
 * right half from 100 % to the maximum, so 100 % stays in the middle and
 * quiet levels keep their fine steps. */
export const PLAYBACK_SLIDER_MAX = 200;
const PLAYBACK_SLIDER_HALF = PLAYBACK_SLIDER_MAX / 2;

/** Where `volume` sits on a playback slider. */
export function playbackSliderPosition(volume: number): number {
  const value = clampPlaybackVolume(volume);
  return Math.round(
    value <= 1
      ? value * PLAYBACK_SLIDER_HALF
      : PLAYBACK_SLIDER_HALF +
          ((value - 1) / (MAX_PLAYBACK_VOLUME - 1)) * PLAYBACK_SLIDER_HALF,
  );
}

/** Filled share of a playback slider, for `.lr-range`'s
 * `--lr-range-value` (WebKit has no progress pseudo). */
export function playbackSliderFill(volume: number): string {
  return `${(playbackSliderPosition(volume) * 100) / PLAYBACK_SLIDER_MAX}%`;
}

/** The volume at a playback slider position, in whole percent. */
export function playbackSliderVolume(position: number): number {
  const step = Math.min(PLAYBACK_SLIDER_MAX, Math.max(0, position));
  const value =
    step <= PLAYBACK_SLIDER_HALF
      ? step / PLAYBACK_SLIDER_HALF
      : 1 +
        ((step - PLAYBACK_SLIDER_HALF) / PLAYBACK_SLIDER_HALF) *
          (MAX_PLAYBACK_VOLUME - 1);
  return Math.round(value * 100) / 100;
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
    messageSound: state.messageSound,
    voiceInput: state.voiceInput,
    pttKey: state.pttKey,
    pttKeyLabel: state.pttKeyLabel,
    pttReleaseDelay: state.pttReleaseDelay,
  };
}

export const useMediaSettings = create<MediaSettingsState>()(
  persist(
    (set, get) => ({
      ...DEFAULT_MEDIA_SETTINGS,
      // The desktop app announces messages while its window is not in
      // front from the start, as chat apps do; a browser tab waits to be
      // asked. A choice made before stays as it is.
      desktopNotify: isDesktopApp(),
      dialogOpen: false,
      dialogSection: null,
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
              ? clampPlaybackVolume(partial.outputVolume)
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
              ? clampPlaybackVolume(partial.sourceAudioVolume)
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
          messageSound: partial.messageSound ?? prev.messageSound,
          voiceInput:
            partial.voiceInput !== undefined
              ? asVoiceInputMode(partial.voiceInput)
              : prev.voiceInput,
          pttKey: partial.pttKey ?? prev.pttKey,
          pttKeyLabel: partial.pttKeyLabel ?? prev.pttKeyLabel,
          pttReleaseDelay:
            partial.pttReleaseDelay !== undefined
              ? clampReleaseDelay(partial.pttReleaseDelay)
              : prev.pttReleaseDelay,
        });
        listener?.(prev, snapshot(get()));
      },
      openDialog: (section) =>
        set({ dialogOpen: true, dialogSection: section ?? null }),
      closeDialog: () => set({ dialogOpen: false, dialogSection: null }),
    }),
    {
      name: SETTINGS_KEY,
      storage: settingsStorage,
      partialize: (state): StoredMediaSettings => ({
        ...snapshot(state),
        shareSourceAudio: state.sourceAudioShare === "on",
      }),
      merge: (persisted, current) => {
        if (!persisted || typeof persisted !== "object") return current;
        // `shareSourceAudio` is the boolean of v0.5 and earlier. It is stored
        // for such a client and is not part of the settings here.
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
          messageSound:
            typeof stored.messageSound === "boolean"
              ? stored.messageSound
              : current.messageSound,
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
          sourceAudioShare:
            "sourceAudioShare" in stored
              ? asSourceAudioShare(stored.sourceAudioShare)
              : restoredSourceAudioShare(sharedBefore === true),
          sourceAudioApp:
            typeof stored.sourceAudioApp === "string"
              ? stored.sourceAudioApp
              : "",
          outputVolume: clampPlaybackVolume(stored.outputVolume ?? 1),
          sourceAudioVolume: clampPlaybackVolume(stored.sourceAudioVolume ?? 1),
          sourceAudioMuted: stored.sourceAudioMuted === true,
          voiceInput: asVoiceInputMode(stored.voiceInput),
          pttKey: typeof stored.pttKey === "string" ? stored.pttKey : "",
          pttKeyLabel:
            typeof stored.pttKeyLabel === "string" ? stored.pttKeyLabel : "",
          pttReleaseDelay: clampReleaseDelay(
            stored.pttReleaseDelay ?? PTT_RELEASE_DELAY_DEFAULT,
          ),
          cameraProfileApply: current.cameraProfileApply,
          screenProfileApply: current.screenProfileApply,
          dialogOpen: current.dialogOpen,
          dialogSection: current.dialogSection,
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

/** Whether a Live carries sound. Without a choice of his own
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

/** Whose sound a share carries in the desktop app, for the controls to name:
 * the chosen application's, or every application's. The app captures it by
 * itself, so no picker has told the user; a browser's own picker has, and
 * there is nothing to name. */
export function sourceAudioScope(
  settings: Pick<MediaSettings, "sourceAudioApp"> = useMediaSettings.getState(),
): string | null {
  if (!hasNativeFeature("app-audio")) return null;
  if (settings.sourceAudioApp) return `Ton von „${settings.sourceAudioApp}“`;
  return hasNativeFeature("app-audio-excludes-self")
    ? "Ton aller Anwendungen außer Gelabber"
    : "Ton aller Anwendungen";
}

export function resetMediaSettingsForTests(): void {
  useMediaSettings.setState({
    ...DEFAULT_MEDIA_SETTINGS,
    dialogOpen: false,
    dialogSection: null,
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
    autoGainControl: mode !== "original" && settings.autoGainControl,
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
