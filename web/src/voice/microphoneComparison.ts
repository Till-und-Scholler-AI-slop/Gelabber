import {
  captureMicrophone,
  type MicProcessor,
  type ProcessingInfo,
} from "./audioProcessing.ts";
import { type MediaSettings } from "./settings.ts";

export type ComparisonState = {
  phase: "idle" | "pending" | "active" | "error";
  message: string;
  processing?: ProcessingInfo;
};
export type MicrophoneLevels = {
  before: number;
  after: number;
  clipping: boolean;
};
export type ComparisonClips = { before: string; after: string };

/** User-initiated local comparison. URLs and capture are owned by this test. */
export function createMicrophoneComparison(
  settings: MediaSettings,
  callbacks: {
    state: (state: ComparisonState) => void;
    levels: (levels: MicrophoneLevels) => void;
    clips: (clips: ComparisonClips | null) => void;
    recording: (recording: boolean) => void;
  },
) {
  let stopped = false;
  let raw: MediaStream | null = null;
  let processor: MicProcessor | null = null;
  let ctx: AudioContext | null = null;
  let frame = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let recorders: MediaRecorder[] = [];
  let clipUrls: string[] = [];
  let recording = false;
  let clipEpoch = 0;
  const nodes: AudioNode[] = [];
  function clearClips() {
    clipEpoch++;
    clipUrls.forEach((url) => URL.revokeObjectURL(url));
    clipUrls = [];
    callbacks.clips(null);
  }
  function endRecording() {
    clearTimeout(timer);
    recorders.forEach((recorder) => {
      if (recorder.state !== "inactive") recorder.stop();
    });
    recording = false;
    callbacks.recording(false);
  }
  function stop() {
    if (stopped) return;
    stopped = true;
    endRecording();
    clearClips();
    cancelAnimationFrame(frame);
    nodes.forEach((node) => node.disconnect());
    processor?.dispose();
    raw?.getTracks().forEach((track) => track.stop());
    if (ctx && ctx.state !== "closed") void ctx.close().catch(() => {});
  }
  async function start() {
    callbacks.state({
      phase: "pending",
      message: "Bitte erlaube den lokalen Mikrofonzugriff.",
    });
    try {
      // Resume before the permission prompt consumes the explicit button gesture.
      ctx = new AudioContext();
      await ctx.resume();
      const result = await captureMicrophone(
        (constraints) => navigator.mediaDevices.getUserMedia(constraints),
        settings,
        false,
        () => {
          callbacks.state({
            phase: "error",
            message:
              "Rauschfilter ausgefallen. Beende den Test und starte ihn erneut.",
          });
          stop();
        },
        {
          current: () => !stopped,
          acquired(stream) {
            raw = stream;
          },
          discarded() {
            raw = null;
          },
        },
      );
      raw = result.raw;
      processor = result.processor;
      if (stopped) {
        processor.dispose();
        raw.getTracks().forEach((track) => track.stop());
        return;
      }
      const analysers = [raw, processor.stream].map((stream) => {
        const source = ctx!.createMediaStreamSource(stream),
          analyser = ctx!.createAnalyser();
        analyser.fftSize = 1024;
        source.connect(analyser);
        nodes.push(source, analyser);
        return analyser;
      });
      const values = analysers.map(
        (analyser) => new Float32Array(analyser.fftSize),
      );
      let last = 0;
      function measure(time: number) {
        if (stopped) return;
        if (time - last >= 80) {
          let clipping = false;
          const levels = analysers.map((analyser, index) => {
            const data = values[index]!;
            analyser.getFloatTimeDomainData(data);
            let energy = 0;
            for (const value of data) {
              energy += value * value;
              clipping ||= Math.abs(value) >= 0.99;
            }
            return Math.min(
              100,
              Math.round(Math.sqrt(energy / data.length) * 350),
            );
          });
          callbacks.levels({ before: levels[0]!, after: levels[1]!, clipping });
          last = time;
        }
        frame = requestAnimationFrame(measure);
      }
      raw.getAudioTracks().forEach((track) =>
        track.addEventListener(
          "ended",
          () => {
            if (!stopped) {
              callbacks.state({
                phase: "error",
                message: "Mikrofonverbindung beendet. Starte den Test erneut.",
              });
              stop();
            }
          },
          { once: true },
        ),
      );
      callbacks.state({
        phase: "active",
        message: result.processor.info.message,
        processing: result.processor.info,
      });
      frame = requestAnimationFrame(measure);
    } catch (error) {
      if (!stopped) {
        const message =
          error instanceof Error && error.name === "NotAllowedError"
            ? "Mikrofonzugriff nicht erlaubt. Prüfe die Browserberechtigung."
            : "Mikrofontest konnte nicht starten. Prüfe Gerät und Berechtigung.";
        callbacks.state({ phase: "error", message });
        stop();
      }
    }
  }
  function record() {
    if (stopped || !raw || !processor || recording) return;
    if (typeof MediaRecorder === "undefined") {
      callbacks.state({
        phase: "active",
        message: "Dieser Browser unterstützt keine lokale Aufnahme.",
      });
      return;
    }
    clearClips();
    const epoch = clipEpoch;
    const mimeType = ["audio/webm;codecs=opus", "audio/ogg;codecs=opus"].find(
      (type) => MediaRecorder.isTypeSupported(type),
    );
    const urls: Partial<ComparisonClips> = {};
    try {
      recorders = [raw, processor.stream].map((stream, index) => {
        const recorder = new MediaRecorder(
          stream,
          mimeType ? { mimeType } : undefined,
        );
        const chunks: Blob[] = [];
        recorder.ondataavailable = (event) => {
          if (event.data.size) chunks.push(event.data);
        };
        recorder.onstop = () => {
          if (stopped || epoch !== clipEpoch) return;
          const url = URL.createObjectURL(
            new Blob(chunks, { type: recorder.mimeType }),
          );
          clipUrls.push(url);
          urls[index === 0 ? "before" : "after"] = url;
          if (urls.before && urls.after)
            callbacks.clips({ before: urls.before, after: urls.after });
        };
        recorder.onerror = () => {
          if (stopped || epoch !== clipEpoch) return;
          endRecording();
          clearClips();
          callbacks.state({
            phase: "active",
            message: "Aufnahme fehlgeschlagen; Mikrofontest läuft weiter.",
          });
        };
        return recorder;
      });
      recorders.forEach((recorder) => recorder.start());
      recording = true;
      callbacks.recording(true);
      timer = setTimeout(endRecording, 8_000);
    } catch {
      endRecording();
      clearClips();
      callbacks.state({
        phase: "active",
        message: "Lokale Aufnahme wird hier nicht unterstützt.",
      });
    }
  }
  return { start, stop, record, endRecording, clearClips };
}
