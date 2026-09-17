// Voice/video + in-app notification prefs. Persisted per browser.
// Defaults: AEC/NS/AGC on, Opus "Normal" 64 kbit/s, full playback volume,
// message toasts on. No product SDK; devices are the browser's own list.

import { create } from "zustand";
import { persist } from "zustand/middleware";

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
  messageToasts: true,
  desktopNotify: false,
};

type MediaSettingsState = MediaSettings & {
  dialogOpen: boolean;
  patch: (partial: Partial<MediaSettings>) => void;
  openDialog: () => void;
  closeDialog: () => void;
};

let listener: ((prev: MediaSettings, next: MediaSettings) => void) | null =
  null;

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
    messageToasts: state.messageToasts,
    desktopNotify: state.desktopNotify,
  };
}

export const useMediaSettings = create<MediaSettingsState>()(
  persist(
    (set, get) => ({
      ...DEFAULT_MEDIA_SETTINGS,
      dialogOpen: false,
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
      partialize: (state) => snapshot(state),
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
  });
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

export function cameraConstraints(): boolean | MediaTrackConstraints {
  const id = useMediaSettings.getState().videoInputId;
  if (!id) return true;
  return { deviceId: { ideal: id } };
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
