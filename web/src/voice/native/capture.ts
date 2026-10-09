// Capture through the desktop app's native core: the microphone through the
// engine's audio device module and processing (RNNoise, WebRTC APM, gain),
// the screen through the desktop's own picker (xdg-desktop-portal).
import type { MicProcessor, ProcessingInfo } from "../audioProcessing.ts";
import type { DeviceList, MediaSettings } from "../settings.ts";
import { invokeNative } from "./bridge.ts";
import { hasNativeFeature, SCREEN_CANCEL_FEATURE } from "./features.ts";
import { NativeStream, NativeTrack } from "./tracks.ts";

type NativeAudioDevices = {
  inputs: Array<{ id: string; name: string }>;
  outputs: Array<{ id: string; name: string }>;
  input: string;
  output: string;
};

/** Device lists in the shape of the browser's, from the native engine. The
 * system default (id "") is the selects' own default entry. */
export async function listNativeDevices(): Promise<DeviceList> {
  const [devices, cameras] = await Promise.all([
    invokeNative<NativeAudioDevices>("media_audio_devices"),
    invokeNative<Array<{ id: string; name: string }>>(
      "media_video_devices",
    ).catch(() => []),
  ]);
  const options = (items: Array<{ id: string; name: string }>) =>
    items
      .filter((item) => item.id)
      .map((item) => ({ id: item.id, label: item.name || item.id }));
  return {
    audioinput: options(devices.inputs),
    audiooutput: options(devices.outputs),
    videoinput: options(cameras),
  };
}

/** Unknown ids (a device from another machine, or unplugged) fall back to the
 * system default like the browser's `ideal` constraint. */
export async function selectInput(id: string): Promise<string> {
  try {
    await invokeNative("media_audio_configure", { options: { input: id } });
    return id;
  } catch (error) {
    if (!id) throw error;
    await invokeNative("media_audio_configure", { options: { input: "" } });
    return "";
  }
}

type Range = ConstrainULong | ConstrainDouble | undefined;

function wanted(range: Range, fallback: number): number {
  const value =
    typeof range === "number"
      ? range
      : (range?.ideal ?? range?.max ?? range?.exact ?? range?.min);
  return Math.round(value ?? fallback);
}

function frameRate(video: MediaStreamConstraints["video"]): number {
  if (!video || video === true) return 30;
  return Math.min(120, Math.max(1, wanted(video.frameRate, 30)));
}

function deviceId(constraint: ConstrainDOMString | undefined): string {
  if (typeof constraint === "string") return constraint;
  if (Array.isArray(constraint)) return constraint[0] ?? "";
  const value = constraint?.exact ?? constraint?.ideal;
  return (Array.isArray(value) ? value[0] : value) ?? "";
}

const unsupported = (what: string) =>
  new DOMException(
    `${what} gibt es in der Desktop-App noch nicht`,
    "NotSupportedError",
  );

/** getUserMedia for the desktop app: cameras through the native core. The
 * microphone goes through `captureNativeMicrophone`. A camera id the
 * machine does not have falls back to the first camera, like `ideal`. */
export async function nativeGetUserMedia(
  constraints: MediaStreamConstraints,
): Promise<MediaStream> {
  const video = constraints.video;
  if (!video) throw unsupported("Diese Mikrofonanfrage");
  const shape = video === true ? {} : video;
  const options = {
    width: Math.min(7680, Math.max(16, wanted(shape.width, 1280))),
    height: Math.min(4320, Math.max(16, wanted(shape.height, 720))),
    fps: frameRate(video),
  };
  const id = deviceId(shape.deviceId);
  const strict =
    typeof shape.deviceId === "object" &&
    !Array.isArray(shape.deviceId) &&
    shape.deviceId.exact !== undefined;
  let source: number;
  let used = id;
  try {
    source = await invokeNative<number>("media_source_camera", {
      options: { ...options, device: id },
    });
  } catch (error) {
    if (!id || strict)
      throw new DOMException(
        `Kamera nicht verfügbar: ${String(error)}`,
        "NotReadableError",
      );
    used = "";
    source = await invokeNative<number>("media_source_camera", {
      options: { ...options, device: "" },
    }).catch((fallback: unknown) => {
      throw new DOMException(
        `Kamera nicht verfügbar: ${String(fallback)}`,
        "NotReadableError",
      );
    });
  }
  const state = await invokeNative<ScreenState>("media_source_state", {
    source,
  }).catch(() => ({ state: "live" }) as ScreenState);
  const track = new NativeTrack(
    "video",
    "Kamera",
    { source },
    {
      deviceId: used,
      width: state.width || options.width,
      height: state.height || options.height,
      frameRate: options.fps,
    },
  );
  return new NativeStream([
    track as unknown as MediaStreamTrack,
  ]) as unknown as MediaStream;
}

type ScreenState = {
  state: "pending" | "live" | "cancelled" | "ended" | "failed";
  width?: number;
  height?: number;
};

const POLL_MS = 250;
const WATCH_MS = 1_000;

export type NativeAudioApp = { id: string; name: string; streams: number };

/** Applications playing sound right now, for the source-audio choice. */
export async function listNativeAudioApps(): Promise<NativeAudioApp[]> {
  return invokeNative<NativeAudioApp[]>("media_audio_apps");
}

/** Sound of other applications for a share, captured from the sound server:
 * `app` is an id from `listNativeAudioApps`, "" every application but
 * Gelabber. No picker is involved, so it can join a share that already runs.
 * Rejects with the core's own reason. `onStreams` hears once a second how
 * many of the application's playback streams feed the track; with none there
 * is nothing to hear. */
export async function captureAppAudio(
  app: string,
  onStreams?: (streams: number) => void,
): Promise<MediaStreamTrack> {
  const source = await invokeNative<number>("media_source_app_audio", {
    options: { app },
  });
  const track = new NativeTrack(
    "audio",
    "Quellton",
    { source },
    { sampleRate: 48_000, channelCount: 2 },
  );
  if (onStreams) {
    const timer = setInterval(() => {
      void invokeNative<{ streams?: unknown }>("media_source_state", { source })
        .then((state) => {
          if (track.readyState === "live" && typeof state?.streams === "number")
            onStreams(state.streams);
        })
        .catch(() => undefined);
    }, WATCH_MS);
    track.onStop(() => clearInterval(timer));
  }
  return track as unknown as MediaStreamTrack;
}

/** Whether a screen source that did not go live was the user's own answer.
 * The pinned libwebrtc reports every non-zero portal response as an error,
 * and xdg-desktop-portal-hyprland answers a closed picker with one, so a
 * source that was waiting for the picker and then failed is taken as a
 * cancel: no message for a plain cancel is worth more than one for the rare
 * portal error. A source that failed before the picker came up stays a
 * failure, and an app with `SCREEN_CANCEL_FEATURE` is taken at its word. */
function screenCancelled(
  state: ScreenState["state"],
  pending: boolean,
): boolean {
  if (state === "cancelled") return true;
  if (state !== "failed" || !pending) return false;
  return !hasNativeFeature(SCREEN_CANCEL_FEATURE);
}

/** getDisplayMedia for the desktop app: the desktop's own picker chooses the
 * screen or window. Its sound is not the picker's to give: the session adds
 * `captureAppAudio` to the share. */
export async function nativeGetDisplayMedia(
  constraints: MediaStreamConstraints,
): Promise<MediaStream> {
  const fps = frameRate(constraints.video);
  const source = await invokeNative<number>("media_source_screen", {
    options: { type: "any", fps, cursor: true, contentHint: "detail" },
  });
  let state: ScreenState;
  let pending = false;
  try {
    for (;;) {
      state = await invokeNative<ScreenState>("media_source_state", { source });
      if (state.state !== "pending") break;
      pending = true;
      await new Promise((resolve) => setTimeout(resolve, POLL_MS));
    }
  } catch (error) {
    void invokeNative("media_source_close", { source }).catch(() => undefined);
    throw error;
  }
  if (state.state !== "live") {
    void invokeNative("media_source_close", { source }).catch(() => undefined);
    throw screenCancelled(state.state, pending)
      ? new DOMException("Freigabe abgebrochen", "NotAllowedError")
      : new DOMException("Bildschirmaufnahme fehlgeschlagen", "AbortError");
  }
  const track = new NativeTrack(
    "video",
    "Bildschirm",
    { source },
    {
      width: state.width,
      height: state.height,
      frameRate: fps,
      displaySurface: "monitor",
    },
  );
  // The portal session can end from the desktop (stop button, window gone).
  const timer = setInterval(() => {
    void invokeNative<ScreenState>("media_source_state", { source })
      .then((next) => {
        if (next.state === "live") {
          if (next.width && next.height)
            track.updateSettings({ width: next.width, height: next.height });
          return;
        }
        clearInterval(timer);
        track.end();
      })
      .catch(() => {
        clearInterval(timer);
        track.end();
      });
  }, WATCH_MS);
  track.onStop(() => clearInterval(timer));
  return new NativeStream([
    track as unknown as MediaStreamTrack,
  ]) as unknown as MediaStream;
}

export function nativeInfo(
  settings: MediaSettings,
  input: string,
): ProcessingInfo {
  const mode = settings.processingMode;
  return {
    requested: mode,
    actual: mode,
    message:
      (mode === "enhanced"
        ? "Desktop-App: RNNoise im nativen Kern"
        : mode === "browser"
          ? "Desktop-App: WebRTC-Rauschfilter im nativen Kern"
          : "Desktop-App: Originalton (Stereo, ohne Filter)") +
      (input !== settings.audioInputId ? "; Standardmikrofon" : ""),
    sampleRate: 48_000,
    channels: mode === "original" ? 2 : 1,
    echoCancellation: settings.echoCancellation,
    noiseSuppression: mode === "browser" && settings.noiseSuppression,
    autoGainControl: mode === "browser" && settings.autoGainControl,
    addedBufferMs: mode === "enhanced" ? 10 : 0,
    inputGain: settings.inputGain,
    contextState: null,
  };
}

/** The desktop app's captureMicrophone: one native source does capture and
 * processing. `raw` is a placeholder stream for the session's bookkeeping
 * (stopping it releases nothing); the processor's stream carries the track
 * that is sent. */
export async function captureNativeMicrophone(
  settings: MediaSettings,
  ownership: {
    current: () => boolean;
    acquired: (raw: MediaStream) => void;
    discarded: (raw: MediaStream) => void;
  },
  onState?: (info: ProcessingInfo) => void,
): Promise<{ raw: MediaStream; processor: MicProcessor }> {
  const input = await selectInput(settings.audioInputId);
  const source = await invokeNative<number>("media_source_microphone", {
    options: {
      processingMode: settings.processingMode,
      echoCancellation: settings.echoCancellation,
      noiseSuppression: settings.noiseSuppression,
      autoGainControl: settings.autoGainControl,
      inputGain: settings.inputGain,
    },
  });
  const trackSettings: MediaTrackSettings = {
    deviceId: input,
    sampleRate: 48_000,
    channelCount: settings.processingMode === "original" ? 2 : 1,
    echoCancellation: settings.echoCancellation,
    noiseSuppression:
      settings.processingMode === "browser" && settings.noiseSuppression,
    autoGainControl:
      settings.processingMode === "browser" && settings.autoGainControl,
  };
  const track = new NativeTrack("audio", "Mikrofon", { source }, trackSettings);
  const raw = new NativeStream([
    new NativeTrack("audio", "Mikrofon", {}, trackSettings),
  ] as unknown as MediaStreamTrack[]) as unknown as MediaStream;
  ownership.acquired(raw);
  if (!ownership.current()) {
    track.stop();
    ownership.discarded(raw);
    throw new Error("Mikrofonanfrage abgebrochen");
  }
  const info = nativeInfo(settings, input);
  let alive = true;
  const processor: MicProcessor = {
    stream: new NativeStream([
      track as unknown as MediaStreamTrack,
    ]) as unknown as MediaStream,
    info,
    enhanced: settings.processingMode === "enhanced",
    nativeFallback: false,
    usable: () => alive && track.readyState === "live",
    setGain(gain) {
      info.inputGain = gain;
      void invokeNative("media_audio_configure", {
        options: { inputGain: gain },
      }).catch((error) => console.warn("[gelabber] native media", error));
    },
    dispose() {
      alive = false;
      track.stop();
    },
  };
  onState?.(info);
  return { raw, processor };
}
