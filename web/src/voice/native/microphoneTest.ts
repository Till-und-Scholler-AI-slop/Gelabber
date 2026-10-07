// The microphone test in the desktop app: the native core captures and
// processes without a call and reports its meters. There is nothing to
// record locally, so the A/B clips stay a browser feature.
import type {
  ComparisonClips,
  ComparisonState,
  MicrophoneLevels,
} from "../microphoneComparison.ts";
import type { MediaSettings } from "../settings.ts";
import { invokeNative } from "./bridge.ts";
import { nativeInfo, selectInput } from "./capture.ts";

type NativeLevels = {
  input: number;
  processed: number;
  clipping: boolean;
  blocks: number;
};

const POLL_MS = 80;
/** Capture that delivers no 10 ms block for this long has failed. */
const STALL_MS = 2_000;

export function createNativeMicrophoneTest(
  settings: MediaSettings,
  callbacks: {
    state: (state: ComparisonState) => void;
    levels: (levels: MicrophoneLevels) => void;
    clips: (clips: ComparisonClips | null) => void;
    recording: (recording: boolean) => void;
  },
) {
  let stopped = false;
  // The monitor's start and end run in order.
  let monitor: Promise<unknown> | null = null;
  let timer: ReturnType<typeof setInterval> | undefined;

  function fail(message: string) {
    if (stopped) return;
    callbacks.state({ phase: "error", message });
    stop();
  }
  function stop() {
    if (stopped) return;
    stopped = true;
    clearInterval(timer);
    if (monitor)
      void monitor
        .catch(() => undefined)
        .then(() => invokeNative("media_audio_monitor", { options: null }))
        .catch(() => undefined);
  }
  async function start() {
    if (stopped) return;
    callbacks.state({ phase: "pending", message: "Mikrofon wird gestartet." });
    try {
      const input = await selectInput(settings.audioInputId);
      if (stopped) return;
      monitor = invokeNative("media_audio_monitor", {
        options: {
          processingMode: settings.processingMode,
          inputGain: settings.inputGain,
        },
      });
      await monitor;
      if (stopped) return;
      const info = nativeInfo(settings, input);
      callbacks.state({
        phase: "active",
        message: info.message,
        processing: info,
      });
      let blocks = -1;
      let moved = performance.now();
      let polling = false;
      timer = setInterval(() => {
        if (polling) return;
        polling = true;
        void invokeNative<NativeLevels>("media_audio_levels")
          .then((levels) => {
            if (stopped) return;
            const now = performance.now();
            if (levels.blocks !== blocks) {
              blocks = levels.blocks;
              moved = now;
              callbacks.levels({
                before: levels.input,
                after: levels.processed,
                clipping: levels.clipping,
              });
            } else if (now - moved >= STALL_MS) {
              fail(
                "Das Mikrofon liefert keine Daten. Prüfe das Eingabegerät und starte den Test erneut.",
              );
            }
          })
          .catch(() => fail("Mikrofonpegel nicht verfügbar."))
          .finally(() => {
            polling = false;
          });
      }, POLL_MS);
    } catch {
      fail(
        "Mikrofontest konnte nicht starten. Prüfe das Eingabegerät in den Systemeinstellungen.",
      );
    }
  }
  return {
    start,
    stop,
    record() {},
    endRecording() {},
    clearClips() {
      callbacks.clips(null);
    },
  };
}
