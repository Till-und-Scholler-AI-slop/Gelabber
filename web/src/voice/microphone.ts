import { translate } from "../i18n/catalog.ts";
import { voiceSettings } from "../i18n/catalogs/voiceSettings.ts";

export type MicrophoneTestState = {
  phase: "idle" | "pending" | "active" | "error";
  message: string;
};

type MicrophoneEnvironment = {
  mediaDevices?: Pick<MediaDevices, "getUserMedia">;
  AudioContextClass?: typeof AudioContext;
  requestFrame: typeof requestAnimationFrame;
  cancelFrame: typeof cancelAnimationFrame;
};

const errorMessages: Record<string, keyof typeof voiceSettings.de> = {
  NotAllowedError: "micTest.error.notAllowed",
  NotFoundError: "micTest.error.notFound",
  NotReadableError: "micTest.error.notReadable",
  SecurityError: "micTest.error.security",
};

/** Local, disposable meter. A late permission result never revives a closed test. */
export function createMicrophoneSession(
  {
    onState,
    onLevel,
  }: {
    onState: (state: MicrophoneTestState) => void;
    onLevel: (level: number) => void;
  },
  environment: MicrophoneEnvironment = {
    mediaDevices: navigator.mediaDevices,
    AudioContextClass: window.AudioContext,
    requestFrame: window.requestAnimationFrame.bind(window),
    cancelFrame: window.cancelAnimationFrame.bind(window),
  },
  audio: MediaTrackConstraints | boolean = true,
) {
  let disposed = false;
  let started = false;
  let stream: MediaStream | undefined;
  let context: AudioContext | undefined;
  let source: MediaStreamAudioSourceNode | undefined;
  let analyser: AnalyserNode | undefined;
  let frame: number | undefined;
  let lastUpdate = 0;
  const endedListeners: Array<[MediaStreamTrack, () => void]> = [];

  function stop() {
    if (disposed) return;
    disposed = true;
    if (frame !== undefined) environment.cancelFrame(frame);
    for (const [track, listener] of endedListeners)
      track.removeEventListener("ended", listener);
    source?.disconnect();
    analyser?.disconnect();
    stream?.getTracks().forEach((track) => track.stop());
    if (context && context.state !== "closed")
      void context.close().catch(() => {});
  }

  async function start() {
    if (started || disposed) return;
    started = true;
    if (!environment.mediaDevices?.getUserMedia) {
      onState({
        phase: "error",
        message: translate(voiceSettings, "micTest.error.unsupported"),
      });
      return;
    }
    onState({
      phase: "pending",
      message: translate(voiceSettings, "micTest.allowBrowser"),
    });
    try {
      // Resume in the explicit click gesture, before waiting for permission.
      if (environment.AudioContextClass) {
        context = new environment.AudioContextClass();
        void context.resume().catch(() => {});
      }
      stream = await environment.mediaDevices.getUserMedia({
        audio,
        video: false,
      });
      if (disposed) {
        stream.getTracks().forEach((track) => track.stop());
        return;
      }
      for (const track of stream.getTracks()) {
        const onEnded = () => {
          if (!disposed)
            onState({
              phase: "error",
              message: translate(voiceSettings, "micTest.error.ended"),
            });
          stop();
        };
        track.addEventListener("ended", onEnded);
        endedListeners.push([track, onEnded]);
      }
      if (!context) throw new Error("AudioContext unavailable");
      if (context.state === "suspended") await context.resume();
      if (disposed) return;
      if (context.state !== "running")
        throw new Error("AudioContext not running");
      analyser = context.createAnalyser();
      analyser.fftSize = 1024;
      source = context.createMediaStreamSource(stream);
      source.connect(analyser);
      // No destination connection: no playback, recording or transmission.
      const meter = analyser;
      const values = new Uint8Array(meter.fftSize);
      onState({
        phase: "active",
        message: translate(voiceSettings, "micTest.speak"),
      });
      function measure(time: number) {
        if (disposed) return;
        if (time - lastUpdate >= 80) {
          meter.getByteTimeDomainData(values);
          const energy = values.reduce(
            (sum, sample) => sum + ((sample - 128) / 128) ** 2,
            0,
          );
          onLevel(
            Math.min(100, Math.round(Math.sqrt(energy / values.length) * 350)),
          );
          lastUpdate = time;
        }
        frame = environment.requestFrame(measure);
      }
      frame = environment.requestFrame(measure);
    } catch (error) {
      if (!disposed) {
        const name = error instanceof Error ? error.name : "";
        onState({
          phase: "error",
          message: translate(
            voiceSettings,
            Object.hasOwn(errorMessages, name)
              ? errorMessages[name]
              : "micTest.error.failed",
          ),
        });
        stop();
      }
    }
  }
  return { start, stop };
}

/** A copy of the call's microphone track to measure, and the level the
 * others hear from it. Mute and push-to-talk switch the call's track, and a
 * copy does not follow them: it stays on, and the level reads the original. */
export function sentLevelMeter(track: MediaStreamTrack): {
  copy: MediaStreamTrack;
  level: (value: number) => number;
} {
  const copy = track.clone();
  copy.enabled = true;
  return { copy, level: (value) => (track.enabled ? value : 0) };
}
