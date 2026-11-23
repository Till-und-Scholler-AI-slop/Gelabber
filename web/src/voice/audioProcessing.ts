import { create } from "zustand";
import { translate } from "../i18n/catalog.ts";
import { diagnostics } from "../i18n/catalogs/diagnostics.ts";
import { fetchVerified, onceWithRetry } from "./audioAssets.ts";
import {
  forgetStrongFallback,
  loadStrongFilter,
  strongFellBack,
  strongInfo,
  strongRemembered,
  type StrongFallback,
  type StrongFilter,
  type StrongInfo,
} from "./deepFilterNet.ts";
import {
  isLocalFilter,
  micConstraints,
  type AudioProcessingMode,
  type MediaSettings,
} from "./settings.ts";

export { forgetStrongFallback };

/** The input sensitivity gate (GEL-27) in public/audio/rnnoise-worklet.js:
 * off, automatic, or below a threshold in dB. */
export type GateMode = "off" | "auto" | "manual";
export type GateConfig = { mode: GateMode; thresholdDb: number };
/** What the gate measured last, about every 80 ms. */
export type GateReport = {
  mode: GateMode;
  levelDb: number;
  thresholdDb: number;
  open: boolean;
};

/** Original / music stays unprocessed, and push-to-talk sends what the key
 * opens. Otherwise the gate follows the settings. */
export function gateConfig(
  settings: Pick<
    MediaSettings,
    "gateAuto" | "gateThresholdDb" | "voiceInput" | "processingMode"
  >,
  actual: AudioProcessingMode = settings.processingMode,
): GateConfig {
  return {
    mode:
      actual === "original" || settings.voiceInput === "ptt"
        ? "off"
        : settings.gateAuto
          ? "auto"
          : "manual",
    thresholdDb: settings.gateThresholdDb,
  };
}

/** The latest report of a running gate: the call's microphone, the
 * microphone test or the settings' level meter. */
export const useInputGate = create<{ report: GateReport | null; at: number }>(
  () => ({ report: null, at: 0 }),
);
function noteGate(data: unknown): void {
  const report = data as Partial<GateReport> & { op?: unknown };
  if (
    report?.op !== "gate" ||
    typeof report.levelDb !== "number" ||
    typeof report.thresholdDb !== "number"
  )
    return;
  useInputGate.setState({
    report: {
      mode:
        report.mode === "auto" || report.mode === "manual"
          ? report.mode
          : "off",
      levelDb: report.levelDb,
      thresholdDb: report.thresholdDb,
      open: report.open === true,
    },
    at: Date.now(),
  });
}

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
  /** The gate as configured, or null where this capture has none. */
  gate: GateMode | null;
  /** "Enhanced voice: Strong" (GEL-28): DFN3's time per block and why it
   * runs as Standard. Null unless Strong was asked for. */
  strong: StrongInfo | null;
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
  gate: null,
  strong: null,
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
  /** Follows changed gate settings (or push-to-talk) without a recapture. */
  setGate: (settings: Parameters<typeof gateConfig>[0]) => void;
  dispose: () => void;
};
const ASSET_HASH =
  "e66d0eaef35d3774e86377efa8b9897e5b226284fb53059f0c1444881888b71c";
/** RNNoise, checked and compiled once per page. After a failed load,
 * captures use the browser filters for a while before it is fetched again. */
const rnnoise = onceWithRetry(async () =>
  WebAssembly.compile(await fetchVerified("rnnoise.wasm", ASSET_HASH)),
);
export function resetNoiseModelForTests(): void {
  rnnoise.reset();
}
/** A worklet's added delay: 10 ms reblocking, plus the input gate's 20 ms
 * look-ahead while the gate is on. The filters' own delay comes on top:
 * 20 ms for RNNoise, 30 ms for DeepFilterNet3 (GEL-28 measurement). */
function workletBufferMs(gate: GateMode): number {
  return gate === "off" ? 10 : 30;
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
    addedBufferMs: isLocalFilter(actual)
      ? workletBufferMs(gateConfig(settings, actual).mode)
      : 0,
    inputGain: 1,
    contextState: null,
    gate: actual === "original" ? "off" : null,
    // Only where DFN3 may run: never beside browser filters.
    strong: actual === "strong" ? strongInfo() : null,
  };
}

/** Graph owns only its output/context. The session owns and stops raw capture.
 * `plain` asks for no worklet: the capture after a failed processor. */
export async function createProcessor(
  raw: MediaStream,
  settings: MediaSettings,
  actual: AudioProcessingMode,
  onFailure?: () => void,
  onState?: (info: ProcessingInfo) => void,
  plain = false,
): Promise<MicProcessor> {
  const info = infoFor(
    raw,
    settings,
    actual,
    translate(
      diagnostics,
      actual === "strong"
        ? "processing.strong"
        : actual === "enhanced"
          ? "processing.enhanced"
          : actual === "browser"
            ? "processing.browser"
            : "processing.original",
    ),
  );
  // "Browser filters" runs the gate in a worklet of its own.
  const gated =
    actual === "browser" && !plain && typeof AudioWorkletNode !== "undefined";
  if (!isLocalFilter(actual) && !gated && settings.inputGain === 1) {
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
      setGate() {},
      dispose() {
        disposed = true;
        raw
          .getAudioTracks()
          .forEach((track) => track.removeEventListener?.("ended", ended));
      },
    };
  }
  const ctx = isLocalFilter(actual)
    ? new AudioContext({ sampleRate: 48_000 })
    : new AudioContext();
  let source: MediaStreamAudioSourceNode | undefined;
  let gain: GainNode | undefined;
  let node: AudioWorkletNode | undefined;
  let destination: MediaStreamAudioDestinationNode | undefined;
  let disposed = false;
  let faulted = false;
  let resuming: Promise<void> | null = null;
  /** Strong running as Standard from here on. */
  const fallBack = (reason: StrongFallback, detail: string) => {
    info.actual = "enhanced";
    info.strong = { ...(info.strong ?? strongInfo()), fallback: reason };
    info.message = translate(
      diagnostics,
      reason === "load"
        ? "processing.strongUnavailable"
        : "processing.strongFellBack",
      {
        reason:
          reason === "load"
            ? detail
            : translate(diagnostics, `strong.reason.${reason}`),
      },
    );
    message = info.message;
  };
  /** What the Strong worklet says about DFN3. */
  const noteStrong = (data: unknown) => {
    const report = data as {
      op?: unknown;
      fallback?: unknown;
      dfnMs?: unknown;
      rnnoiseMs?: unknown;
      maxMs?: unknown;
    };
    if (report?.op !== "strong" || !info.strong) return;
    const ms = (value: unknown) =>
      typeof value === "number" && Number.isFinite(value) ? value : null;
    if (
      report.fallback === "slow" ||
      report.fallback === "underrun" ||
      report.fallback === "error"
    ) {
      info.strong.dfnMs = ms(report.dfnMs) ?? info.strong.dfnMs;
      strongFellBack(report.fallback, info.strong.dfnMs);
      fallBack(report.fallback, "");
    } else {
      info.strong.dfnMs = ms(report.dfnMs);
      info.strong.rnnoiseMs = ms(report.rnnoiseMs);
      info.strong.maxMs = ms(report.maxMs);
    }
    info.strong = { ...info.strong };
    onState?.({ ...info });
  };
  let resumeTimer: ReturnType<typeof setTimeout> | undefined;
  let cancelResume: (() => void) | undefined;
  let message = info.message;
  // A graph rebuilt with Standard after Strong fell back still says why.
  const fellBack =
    settings.processingMode === "strong" && actual === "enhanced"
      ? strongRemembered()
      : null;
  if (fellBack) {
    info.strong = { ...strongInfo(), dfnMs: fellBack.dfnMs };
    fallBack(fellBack.reason, "");
  }
  const fault = () => {
    if (!disposed && !faulted) {
      faulted = true;
      onFailure?.();
    }
  };
  /** Waits for the worklet's hello; then its port carries gate reports. */
  const workletReady = (worklet: AudioWorkletNode) =>
    new Promise<void>((resolve, reject) => {
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
          if (info.strong)
            info.strong.clock =
              event.data.clock === "date" ? "date" : "performance";
          resolve();
        } else if (!disposed) {
          noteGate(event.data);
          noteStrong(event.data);
        }
      };
    });
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
    if (isLocalFilter(actual)) {
      if (
        ctx.sampleRate !== 48_000 ||
        !ctx.audioWorklet ||
        typeof AudioWorkletNode === "undefined"
      )
        throw new Error(
          translate(diagnostics, "processing.error.noiseFilterUnsupported"),
        );
      const remembered = actual === "strong" ? strongRemembered() : null;
      const strongLoad: Promise<StrongFilter | Error> | null =
        actual === "strong" && !remembered
          ? loadStrongFilter().catch((error: unknown) =>
              error instanceof Error ? error : new Error(String(error)),
            )
          : null;
      const model = await rnnoise.get();
      let strong: StrongFilter | null = null;
      if (remembered) {
        info.strong = { ...strongInfo(), dfnMs: remembered.dfnMs };
        fallBack(remembered.reason, "");
      } else if (strongLoad) {
        // The worker's warm-up time is no verdict: the worklet measures.
        const loaded = await strongLoad;
        if (loaded instanceof Error) fallBack("load", loaded.message);
        else strong = loaded;
      }
      await ctx.audioWorklet.addModule(
        `/audio/rnnoise-worklet.js?v=${ASSET_HASH}`,
      );
      const filterNode = (name: string, extra: Record<string, unknown>) =>
        new AudioWorkletNode(ctx, name, {
          numberOfInputs: 1,
          numberOfOutputs: 1,
          outputChannelCount: [1],
          processorOptions: {
            model,
            ...extra,
            gate: gateConfig(settings, actual),
            gain: settings.inputGain,
          },
        });
      if (strong) {
        // Strong that does not start (no memory for the model copy, or too
        // slow to say hello) is Standard, not browser filters. A processor
        // error is RNNoise's, which Standard would hit as well.
        try {
          node = filterNode("gelabber-strong", {
            dfn: {
              module: strong.module,
              memory: strong.memory,
              state: strong.state,
            },
          });
          await workletReady(node);
        } catch (error) {
          if (faulted || disposed) throw error;
          if (node) {
            node.onprocessorerror = null;
            node.port.onmessage = null;
            node.port.postMessage({ op: "dispose" });
            node.port.close();
            node.disconnect();
            node = undefined;
          }
          strongFellBack("error", null);
          fallBack("error", "");
        }
      }
      if (!node) {
        node = filterNode("gelabber-rnnoise", {});
        await workletReady(node);
      }
      node.onprocessorerror = fault;
      source.connect(node);
      node.connect(gain);
      info.sampleRate = ctx.sampleRate;
      info.channels = 1;
    } else if (gated && ctx.audioWorklet) {
      await ctx.audioWorklet.addModule(
        `/audio/rnnoise-worklet.js?v=${ASSET_HASH}`,
      );
      node = new AudioWorkletNode(ctx, "gelabber-gate", {
        numberOfInputs: 1,
        numberOfOutputs: 1,
        outputChannelCount: [1],
        processorOptions: {
          gate: gateConfig(settings, actual),
          gain: settings.inputGain,
        },
      });
      await workletReady(node);
      node.onprocessorerror = fault;
      source.connect(node);
      node.connect(gain);
      info.addedBufferMs = workletBufferMs(gateConfig(settings, actual).mode);
    } else source.connect(gain);
    if (faulted || ctx.state !== "running")
      throw new Error(translate(diagnostics, "processing.error.pausedAtStart"));
    gain.connect(destination);
    info.sampleRate = ctx.sampleRate ?? info.sampleRate;
    info.channels = destination.channelCount;
    info.inputGain = settings.inputGain;
    info.contextState = ctx.state;
    if (node) info.gate = gateConfig(settings, actual).mode;
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
    const currentNode = node;
    return {
      stream: destination.stream,
      info,
      enhanced: isLocalFilter(actual),
      nativeFallback: false,
      usable: () =>
        !disposed &&
        !faulted &&
        ctx.state === "running" &&
        raw.getAudioTracks().some((track) => track.readyState !== "ended"),
      setGain(next) {
        if (!disposed) {
          currentGain.gain.value = next;
          currentNode?.port.postMessage({ op: "gain", value: next });
          info.inputGain = next;
          onState?.({ ...info });
          resume();
        }
      },
      setGate(next) {
        if (disposed || !currentNode) return;
        const config = gateConfig(next, actual);
        currentNode.port.postMessage({ op: "gateConfig", config });
        if (info.gate !== config.mode) {
          info.gate = config.mode;
          info.addedBufferMs = workletBufferMs(config.mode);
          onState?.({ ...info });
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
  const local = isLocalFilter(settings.processingMode);
  const actual =
    (forceBrowser || unavailable) && local
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
      forceBrowser || unavailable,
    );
    if (!ownership.current() || !processor.usable()) {
      processor.dispose();
      throw new Error(
        translate(diagnostics, "processing.error.cancelledOrFailed"),
      );
    }
    if (forceBrowser || (unavailable && local)) processor.nativeFallback = true;
    if ((forceBrowser || unavailable) && local)
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
        true,
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
        isLocalFilter(actual)
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
