import { create } from "zustand";
import { translate } from "../i18n/catalog.ts";
import { diagnostics } from "../i18n/catalogs/diagnostics.ts";
import {
  micConstraints,
  type AudioProcessingMode,
  type MediaSettings,
} from "./settings.ts";

export type ProcessingInfo = {
  requested: AudioProcessingMode;
  actual: AudioProcessingMode | null;
  message: string;
  sampleRate: number | null;
  channels: number | null;
  echoCancellation: boolean | null;
  noiseSuppression: boolean | null;
  autoGainControl: boolean | null;
  addedBufferMs: number | null;
  inputGain: number;
  contextState: string | null;
};
/** Its words in the language of the moment it is noted. */
const inactive = (): ProcessingInfo => ({
  requested: "enhanced",
  actual: null,
  message: translate(diagnostics, "processing.inactive"),
  sampleRate: null,
  channels: null,
  echoCancellation: null,
  noiseSuppression: null,
  autoGainControl: null,
  addedBufferMs: null,
  inputGain: 1,
  contextState: null,
});
export const useAudioProcessing = create<ProcessingInfo>(() => inactive());
export function noteAudioProcessing(info: ProcessingInfo | null): void {
  useAudioProcessing.setState(info ?? inactive());
}
export type MicProcessor = {
  stream: MediaStream;
  info: ProcessingInfo;
  enhanced: boolean;
  /** This capture deliberately bypasses the failed processing/gain graph. */
  nativeFallback: boolean;
  /** Persistent faults remain visible even before the session installs callbacks. */
  usable: () => boolean;
  setGain: (gain: number) => void;
  dispose: () => void;
};
const ASSET_HASH =
  "e66d0eaef35d3774e86377efa8b9897e5b226284fb53059f0c1444881888b71c";
/** A download that stops arriving gives up; a slow one that keeps arriving
 * does not (the model is about 5.6 MB and sent uncompressed). */
const MODEL_STALL_MS = 8_000;
/** After a failed load, captures use the browser filters this long before
 * the model is fetched again, longer after each failure. */
const MODEL_RETRY_MS = [30_000, 120_000, 600_000];
let wasm: Promise<WebAssembly.Module> | null = null;
let modelFailure: { error: unknown; count: number; until: number } | null =
  null;
async function fetchModel(): Promise<ArrayBuffer> {
  const abort = new AbortController();
  let stall: ReturnType<typeof setTimeout> | undefined;
  const arm = () => {
    clearTimeout(stall);
    stall = setTimeout(
      () =>
        abort.abort(
          new Error(translate(diagnostics, "processing.error.modelStalled")),
        ),
      MODEL_STALL_MS,
    );
  };
  arm();
  try {
    const response = await fetch(`/audio/rnnoise.wasm?sha256=${ASSET_HASH}`, {
      cache: "force-cache",
      signal: abort.signal,
    });
    if (!response.ok)
      throw new Error(translate(diagnostics, "processing.error.modelLoad"));
    if (!response.body) return await response.arrayBuffer();
    const reader = response.body.getReader();
    const chunks: Uint8Array[] = [];
    let size = 0;
    for (;;) {
      arm();
      const { done, value } = await reader.read();
      if (done) break;
      chunks.push(value);
      size += value.byteLength;
    }
    const bytes = new Uint8Array(size);
    let offset = 0;
    for (const chunk of chunks) {
      bytes.set(chunk, offset);
      offset += chunk.byteLength;
    }
    return bytes.buffer;
  } finally {
    clearTimeout(stall);
  }
}
async function loadModel(): Promise<WebAssembly.Module> {
  if (modelFailure && Date.now() < modelFailure.until) throw modelFailure.error;
  if (!wasm) {
    wasm = (async () => {
      const bytes = await fetchModel();
      const digest = await crypto.subtle.digest("SHA-256", bytes);
      const hash = Array.from(new Uint8Array(digest), (byte) =>
        byte.toString(16).padStart(2, "0"),
      ).join("");
      if (hash !== ASSET_HASH)
        throw new Error(
          translate(diagnostics, "processing.error.modelChecksum"),
        );
      const model = await WebAssembly.compile(bytes);
      modelFailure = null;
      return model;
    })().catch((error: unknown) => {
      wasm = null;
      const count = (modelFailure?.count ?? 0) + 1;
      modelFailure = {
        error,
        count,
        until:
          Date.now() +
          MODEL_RETRY_MS[Math.min(count, MODEL_RETRY_MS.length) - 1]!,
      };
      throw error;
    });
  }
  return wasm;
}
export function resetNoiseModelForTests(): void {
  wasm = null;
  modelFailure = null;
}
function infoFor(
  raw: MediaStream,
  settings: MediaSettings,
  actual: AudioProcessingMode,
  message: string,
): ProcessingInfo {
  const effective = raw.getAudioTracks()[0]?.getSettings?.() ?? {};
  return {
    requested: settings.processingMode,
    actual,
    message:
      actual === "browser" &&
      effective.noiseSuppression === false &&
      effective.autoGainControl === false
        ? translate(diagnostics, "processing.browserNoFilters")
        : message,
    sampleRate: effective.sampleRate ?? null,
    channels: effective.channelCount ?? null,
    echoCancellation:
      typeof effective.echoCancellation === "boolean"
        ? effective.echoCancellation
        : null,
    noiseSuppression: effective.noiseSuppression ?? null,
    autoGainControl: effective.autoGainControl ?? null,
    addedBufferMs: actual === "enhanced" ? 10 : 0,
    inputGain: 1,
    contextState: null,
  };
}

/** Graph owns only its output/context. The session owns and stops raw capture. */
export async function createProcessor(
  raw: MediaStream,
  settings: MediaSettings,
  actual: AudioProcessingMode,
  onFailure?: () => void,
  onState?: (info: ProcessingInfo) => void,
): Promise<MicProcessor> {
  const info = infoFor(
    raw,
    settings,
    actual,
    translate(
      diagnostics,
      actual === "enhanced"
        ? "processing.enhanced"
        : actual === "browser"
          ? "processing.browser"
          : "processing.original",
    ),
  );
  if (actual !== "enhanced" && settings.inputGain === 1) {
    let disposed = false;
    const ended = () => {
      if (!disposed) onFailure?.();
    };
    raw
      .getAudioTracks()
      .forEach((track) => track.addEventListener?.("ended", ended));
    return {
      stream: raw,
      info,
      enhanced: false,
      nativeFallback: false,
      usable: () =>
        !disposed &&
        raw.getAudioTracks().some((track) => track.readyState !== "ended"),
      setGain() {},
      dispose() {
        disposed = true;
        raw
          .getAudioTracks()
          .forEach((track) => track.removeEventListener?.("ended", ended));
      },
    };
  }
  const ctx =
    actual === "enhanced"
      ? new AudioContext({ sampleRate: 48_000 })
      : new AudioContext();
  let source: MediaStreamAudioSourceNode | undefined;
  let gain: GainNode | undefined;
  let node: AudioWorkletNode | undefined;
  let destination: MediaStreamAudioDestinationNode | undefined;
  let disposed = false;
  let faulted = false;
  let resuming: Promise<void> | null = null;
  let resumeTimer: ReturnType<typeof setTimeout> | undefined;
  let cancelResume: (() => void) | undefined;
  const message = info.message;
  const fault = () => {
    if (!disposed && !faulted) {
      faulted = true;
      onFailure?.();
    }
  };
  const resume = () => {
    if (
      disposed ||
      faulted ||
      ctx.state === "running" ||
      resuming ||
      (typeof document !== "undefined" && document.visibilityState === "hidden")
    )
      return;
    resuming = resumeContext()
      .then(() => {
        if (!disposed && ctx.state !== "running") fault();
      })
      .catch(fault)
      .finally(() => {
        resuming = null;
      });
  };
  function resumeContext(): Promise<void> {
    return new Promise<void>((resolve, reject) => {
      cancelResume = () =>
        reject(new Error(translate(diagnostics, "processing.error.ended")));
      resumeTimer = setTimeout(
        () =>
          reject(new Error(translate(diagnostics, "processing.error.resume"))),
        2_000,
      );
      void ctx.resume().then(resolve, reject);
    }).finally(() => {
      clearTimeout(resumeTimer);
      resumeTimer = undefined;
      cancelResume = undefined;
    });
  }
  const dispose = () => {
    if (disposed) return;
    disposed = true;
    clearTimeout(resumeTimer);
    cancelResume?.();
    raw
      .getAudioTracks()
      .forEach((track) => track.removeEventListener?.("ended", fault));
    ctx.onstatechange = null;
    if (typeof window !== "undefined") {
      window.removeEventListener("focus", resume);
      window.removeEventListener("pointerdown", resume, true);
      window.removeEventListener("keydown", resume, true);
    }
    if (typeof document !== "undefined")
      document.removeEventListener("visibilitychange", resume);
    if (node) {
      node.onprocessorerror = null;
      node.port.postMessage({ op: "dispose" });
      node.port.close();
      node.disconnect();
    }
    source?.disconnect?.();
    gain?.disconnect?.();
    destination?.stream.getTracks().forEach((track) => track.stop());
    if (ctx.state !== "closed") void ctx.close().catch(() => {});
  };
  try {
    await resumeContext();
    if (ctx.state !== "running")
      throw new Error(translate(diagnostics, "processing.error.paused"));
    source = ctx.createMediaStreamSource(raw);
    gain = ctx.createGain();
    gain.gain.value = settings.inputGain;
    destination = ctx.createMediaStreamDestination();
    destination.channelCount =
      actual === "original" ? Math.min(2, info.channels ?? 2) : 1;
    destination.channelCountMode = "explicit";
    if (actual === "enhanced") {
      if (
        ctx.sampleRate !== 48_000 ||
        !ctx.audioWorklet ||
        typeof AudioWorkletNode === "undefined"
      )
        throw new Error(
          translate(diagnostics, "processing.error.noiseFilterUnsupported"),
        );
      const model = await loadModel();
      await ctx.audioWorklet.addModule(
        `/audio/rnnoise-worklet.js?v=${ASSET_HASH}`,
      );
      node = new AudioWorkletNode(ctx, "gelabber-rnnoise", {
        numberOfInputs: 1,
        numberOfOutputs: 1,
        outputChannelCount: [1],
        processorOptions: { model },
      });
      const worklet = node;
      await new Promise<void>((resolve, reject) => {
        const timeout = setTimeout(
          () =>
            reject(
              new Error(
                translate(diagnostics, "processing.error.workletTimeout"),
              ),
            ),
          2_000,
        );
        worklet.onprocessorerror = () => {
          fault();
          clearTimeout(timeout);
          reject(
            new Error(translate(diagnostics, "processing.error.workletFailed")),
          );
        };
        worklet.port.onmessage = (event: MessageEvent) => {
          if (event.data?.ready) {
            clearTimeout(timeout);
            resolve();
          }
        };
      });
      node.onprocessorerror = fault;
      source.connect(node);
      node.connect(gain);
      info.sampleRate = ctx.sampleRate;
      info.channels = 1;
    } else source.connect(gain);
    if (faulted || ctx.state !== "running")
      throw new Error(translate(diagnostics, "processing.error.pausedAtStart"));
    gain.connect(destination);
    info.sampleRate = ctx.sampleRate ?? info.sampleRate;
    info.channels = destination.channelCount;
    info.inputGain = settings.inputGain;
    info.contextState = ctx.state;
    raw
      .getAudioTracks()
      .forEach((track) => track.addEventListener?.("ended", fault));
    ctx.onstatechange = () => {
      if (disposed || faulted) return;
      info.contextState = String(ctx.state);
      info.message =
        ctx.state === "running"
          ? message
          : translate(diagnostics, "processing.resuming");
      onState?.({ ...info });
      if (ctx.state === "closed") fault();
      else resume();
    };
    if (typeof window !== "undefined") {
      window.addEventListener("focus", resume);
      window.addEventListener("pointerdown", resume, true);
      window.addEventListener("keydown", resume, true);
    }
    if (typeof document !== "undefined")
      document.addEventListener("visibilitychange", resume);
    const currentGain = gain;
    return {
      stream: destination.stream,
      info,
      enhanced: actual === "enhanced",
      nativeFallback: false,
      usable: () =>
        !disposed &&
        !faulted &&
        ctx.state === "running" &&
        raw.getAudioTracks().some((track) => track.readyState !== "ended"),
      setGain(next) {
        if (!disposed) {
          currentGain.gain.value = next;
          info.inputGain = next;
          onState?.({ ...info });
          resume();
        }
      },
      dispose,
    };
  } catch (error) {
    dispose();
    throw error;
  }
}

/** No unfiltered silent fallback: acquire browser constraints again on DSP failure. */
export async function captureMicrophone(
  getMedia: (constraints: MediaStreamConstraints) => Promise<MediaStream>,
  settings: MediaSettings,
  forceBrowser = false,
  onFailure?: () => void,
  ownership: {
    current: () => boolean;
    acquired: (raw: MediaStream) => void;
    discarded: (raw: MediaStream) => void;
  } = { current: () => true, acquired() {}, discarded() {} },
  onState?: (info: ProcessingInfo) => void,
): Promise<{ raw: MediaStream; processor: MicProcessor }> {
  const unavailable =
    typeof AudioContext === "undefined" ||
    typeof AudioWorkletNode === "undefined" ||
    typeof globalThis.crypto?.subtle === "undefined";
  const actual =
    (forceBrowser || unavailable) && settings.processingMode === "enhanced"
      ? "browser"
      : settings.processingMode;
  let raw = await getMedia({
    audio: micConstraints(settings, actual),
    video: false,
  });
  ownership.acquired(raw);
  try {
    if (!ownership.current())
      throw new Error(translate(diagnostics, "processing.error.cancelled"));
    const processor = await createProcessor(
      raw,
      forceBrowser ? { ...settings, inputGain: 1 } : settings,
      actual,
      onFailure,
      onState,
    );
    if (!ownership.current() || !processor.usable()) {
      processor.dispose();
      throw new Error(
        translate(diagnostics, "processing.error.cancelledOrFailed"),
      );
    }
    if (forceBrowser || (unavailable && settings.processingMode === "enhanced"))
      processor.nativeFallback = true;
    if ((forceBrowser || unavailable) && settings.processingMode === "enhanced")
      processor.info.message = translate(
        diagnostics,
        forceBrowser
          ? "processing.processorFailed"
          : "processing.noiseFilterUnsupported",
        { state: processor.info.message },
      );
    return { raw, processor };
  } catch (error) {
    raw.getTracks().forEach((track) => track.stop());
    ownership.discarded(raw);
    if (!ownership.current()) throw error;
    const fallbackMode = actual === "original" ? "original" : "browser";
    raw = await getMedia({
      audio: micConstraints(settings, fallbackMode),
      video: false,
    });
    ownership.acquired(raw);
    try {
      if (!ownership.current())
        throw new Error(translate(diagnostics, "processing.error.cancelled"), {
          cause: error,
        });
      const processor = await createProcessor(
        raw,
        { ...settings, inputGain: 1 },
        fallbackMode,
        onFailure,
        onState,
      );
      if (!ownership.current() || !processor.usable()) {
        processor.dispose();
        throw new Error(translate(diagnostics, "processing.error.cancelled"), {
          cause: error,
        });
      }
      processor.nativeFallback = true;
      processor.info.message = translate(
        diagnostics,
        actual === "enhanced"
          ? "processing.noiseFilterUnavailable"
          : "processing.unavailable",
        { state: processor.info.message },
      );
      return { raw, processor };
    } catch (fallbackError) {
      raw.getTracks().forEach((track) => track.stop());
      ownership.discarded(raw);
      throw fallbackError;
    }
  }
}
