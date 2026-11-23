// DeepFilterNet3 for "Enhanced voice: Strong" (GEL-28): the pinned module
// and model from public/audio (web/scripts/audio/build-deepfilternet.sh),
// the model built in a worker, and what the worklet reports about it.
import { translate } from "../i18n/catalog.ts";
import { diagnostics } from "../i18n/catalogs/diagnostics.ts";
import { fetchVerified, onceWithRetry } from "./audioAssets.ts";

export const DFN_WASM_HASH =
  "18929d4fed09a64173cd26e958a712ac6466d821b3e327035d8eaa7d2e5fbb33";
export const DFN_MODEL_HASH =
  "c94d91f70911001c946e0fabb4aa9adc37045f45a03b56008cb0c8244cb63616";
/** libDF's runtime parameters as measured in GEL-28 step 1: attenuation
 * limit 100 dB (none), local-SNR thresholds -15 / 35 / 35 dB, so the full
 * model runs on every block (libDF's defaults skip it on many and rate
 * worse than RNNoise). No post filter. */
export const DFN_PARAMS = [100, -15, 35, 35] as const;
/** One 10 ms block at 48 kHz. A device whose worker needs longer than
 * this per block cannot run Strong in real time. */
export const STRONG_BLOCK_MS = 10;
/** How much later DFN3's output is than RNNoise's (measured: 30 against
 * 20 ms). */
export const STRONG_EXTRA_DELAY_MS = 10;

export type StrongFilter = {
  module: WebAssembly.Module;
  /** The module's memory right after the model was built. */
  memory: ArrayBuffer;
  /** The model's state pointer in that memory. */
  state: number;
  buildMs: number;
  /** Time per block while the worker warmed the code up. */
  blockMs: number;
};
type Built = Omit<StrongFilter, "module">;
type Builder = (
  module: WebAssembly.Module,
  model: ArrayBuffer,
) => Promise<Built>;

/** Builds the model in public/audio/deepfilternet-worker.js. */
function buildInWorker(
  module: WebAssembly.Module,
  model: ArrayBuffer,
): Promise<Built> {
  return new Promise((resolve, reject) => {
    const failed = () =>
      new Error(translate(diagnostics, "processing.error.strongBuild"));
    if (typeof Worker === "undefined") return reject(failed());
    const worker = new Worker(
      `/audio/deepfilternet-worker.js?v=${DFN_WASM_HASH}`,
    );
    const end = (error: Error | null, built?: Built) => {
      clearTimeout(timeout);
      worker.terminate();
      if (error) reject(error);
      else resolve(built!);
    };
    // A phone needs a few seconds; a worker that never answers is stuck.
    const timeout = setTimeout(() => end(failed()), 60_000);
    worker.onerror = () => end(failed());
    worker.onmessage = (event: MessageEvent) => {
      const data = event.data as Partial<Built> & { ok?: boolean };
      if (
        data?.ok === true &&
        data.memory instanceof ArrayBuffer &&
        typeof data.state === "number"
      )
        end(null, {
          memory: data.memory,
          state: data.state,
          buildMs: Number(data.buildMs),
          blockMs: Number(data.blockMs),
        });
      else end(failed());
    };
    worker.postMessage({ module, model, params: DFN_PARAMS }, [model]);
  });
}

let builder: Builder = buildInWorker;
export function setStrongBuilderForTests(next?: Builder): void {
  builder = next ?? buildInWorker;
  strongFilter.reset();
  slow = null;
}

/** Downloads, checks and builds once per page; about 31 MB of memory
 * stay in this page for the next capture. */
const strongFilter = onceWithRetry(async (): Promise<StrongFilter> => {
  const [wasm, model] = await Promise.all([
    fetchVerified("deepfilternet.wasm", DFN_WASM_HASH),
    fetchVerified("deepfilternet3-model.bin", DFN_MODEL_HASH),
  ]);
  const module = await WebAssembly.compile(wasm);
  return { module, ...(await builder(module, model)) };
});
export const loadStrongFilter = strongFilter.get;

/** Why Strong runs as Standard. "load": the module or model did not load,
 * check or build. The others come from the worklet. */
export type StrongFallback = "slow" | "underrun" | "error" | "load";
export type StrongInfo = {
  /** Mean wall-clock time DFN3 took per 10 ms block on the audio thread,
   * over about the last second; null before the first report. */
  dfnMs: number | null;
  /** The same for RNNoise, which runs beside DFN3 for the input gate. */
  rnnoiseMs: number | null;
  /** The slowest block (DFN3 and RNNoise) of that second. */
  maxMs: number | null;
  /** "date": the worklet has only `Date.now()`, in whole milliseconds. */
  clock: "performance" | "date" | null;
  fallback: StrongFallback | null;
};
export function strongInfo(fallback: StrongFallback | null = null): StrongInfo {
  return { dfnMs: null, rnnoiseMs: null, maxMs: null, clock: null, fallback };
}

/** A device that could not keep up stays on Standard until Strong is
 * chosen again or the page reloads: each new try would cost seconds of
 * broken audio. */
let slow: { reason: StrongFallback; dfnMs: number | null } | null = null;
export function strongFellBack(
  reason: StrongFallback,
  dfnMs: number | null,
): void {
  if (reason !== "load") slow = { reason, dfnMs };
}
export function strongRemembered(): {
  reason: StrongFallback;
  dfnMs: number | null;
} | null {
  return slow;
}
/** The user chose the mode again: Strong gets another try. */
export function forgetStrongFallback(): void {
  slow = null;
}
