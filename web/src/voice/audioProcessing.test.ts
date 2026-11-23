import { afterEach, describe, expect, it, vi } from "vitest";
import {
  captureMicrophone,
  createProcessor,
  gateConfig,
  resetNoiseModelForTests,
  useInputGate,
} from "./audioProcessing.ts";
import {
  DFN_MODEL_HASH,
  DFN_WASM_HASH,
  forgetStrongFallback,
  setStrongBuilderForTests,
} from "./deepFilterNet.ts";
import { DEFAULT_MEDIA_SETTINGS } from "./settings.ts";
function stream() {
  const track = {
    stop: vi.fn(),
    getSettings: () => ({
      sampleRate: 48000,
      channelCount: 1,
      echoCancellation: true,
      noiseSuppression: true,
      autoGainControl: true,
    }),
  };
  return {
    raw: {
      getTracks: () => [track],
      getAudioTracks: () => [track],
    } as unknown as MediaStream,
    track,
  };
}
afterEach(() => {
  vi.unstubAllGlobals();
  vi.useRealTimers();
  resetNoiseModelForTests();
});
describe("microphone processor ownership", () => {
  it.each(["enhanced", "strong", "browser", "original"] as const)(
    "bypasses a rejected gain context for %s without killing the native fallback",
    async (mode) => {
      const contexts: Array<{ closed: boolean }> = [];
      class Context {
        state = "suspended";
        closed = false;
        constructor() {
          contexts.push(this);
        }
        resume = async () => {
          throw new Error("AudioContext unavailable");
        };
        close = async () => {
          this.closed = true;
          this.state = "closed";
        };
      }
      vi.stubGlobal("AudioContext", Context);
      vi.stubGlobal("AudioWorkletNode", class {});
      const first = stream(),
        second = stream();
      const getMedia = vi
        .fn()
        .mockResolvedValueOnce(first.raw)
        .mockResolvedValueOnce(second.raw);
      const result = await captureMicrophone(getMedia, {
        ...DEFAULT_MEDIA_SETTINGS,
        processingMode: mode,
        inputGain: 1.5,
      });
      expect(getMedia).toHaveBeenCalledTimes(2);
      expect(contexts).toHaveLength(1);
      expect(contexts[0]?.closed).toBe(true);
      expect(first.track.stop).toHaveBeenCalled();
      expect(second.track.stop).not.toHaveBeenCalled();
      expect(result.processor.stream).toBe(second.raw);
      expect(result.processor.nativeFallback).toBe(true);
      expect(result.processor.info.inputGain).toBe(1);
      expect(result.processor.info.message).toContain(
        "Mikrofonverstärkung 100 %",
      );
      expect(result.processor.info.actual).toBe(
        mode === "original" ? "original" : "browser",
      );
      expect(result.processor.usable()).toBe(true);
      result.processor.dispose();
    },
  );
  it("uses visible native fallback when AudioWorklet is unavailable", async () => {
    vi.stubGlobal("AudioWorkletNode", undefined);
    const { raw } = stream();
    const getMedia = vi.fn<
      (constraints: MediaStreamConstraints) => Promise<MediaStream>
    >(async () => raw);
    const result = await captureMicrophone(getMedia, DEFAULT_MEDIA_SETTINGS);
    expect(getMedia).toHaveBeenCalledOnce();
    expect(getMedia.mock.calls[0]?.[0]).toMatchObject({
      audio: { noiseSuppression: true, autoGainControl: true },
    });
    expect(result.processor.info.actual).toBe("browser");
    expect(result.processor.info.requested).toBe("enhanced");
    expect(result.processor.info.message).toContain("Browserfilter aktiv");
  });
  it("stops a corrupt-model candidate and recaptures real browser filters", async () => {
    const contexts: Array<{ closed: boolean }> = [];
    vi.stubGlobal("AudioWorkletNode", class {});
    vi.stubGlobal(
      "AudioContext",
      class {
        state = "running";
        sampleRate = 48_000;
        closed = false;
        audioWorklet = { addModule: async () => {} };
        constructor() {
          contexts.push(this);
        }
        resume = async () => {};
        close = async () => {
          this.closed = true;
        };
        createMediaStreamSource() {
          return { connect() {}, disconnect() {} };
        }
        createGain() {
          return { gain: { value: 1 }, connect() {}, disconnect() {} };
        }
        createMediaStreamDestination() {
          return { stream: stream().raw };
        }
      },
    );
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => new Response(new Uint8Array([1, 2, 3]))),
    );
    const first = stream(),
      second = stream();
    const getMedia = vi
      .fn()
      .mockResolvedValueOnce(first.raw)
      .mockResolvedValueOnce(second.raw);
    const result = await captureMicrophone(getMedia, DEFAULT_MEDIA_SETTINGS);
    expect(fetch).toHaveBeenCalled();
    expect(first.track.stop).toHaveBeenCalled();
    expect(contexts[0]?.closed).toBe(true);
    expect(second.track.stop).not.toHaveBeenCalled();
    expect(getMedia.mock.calls[0]?.[0]).toMatchObject({
      audio: { noiseSuppression: false, autoGainControl: true },
    });
    expect(getMedia.mock.calls[1]?.[0]).toMatchObject({
      audio: { noiseSuppression: true, autoGainControl: true },
    });
    expect(result.processor.info.actual).toBe("browser");
  });
  it("does not start replacement capture after the owning call ends", async () => {
    const { raw, track } = stream();
    const acquired = vi.fn(),
      discarded = vi.fn();
    const getMedia = vi.fn<
      (constraints: MediaStreamConstraints) => Promise<MediaStream>
    >(async () => raw);
    await expect(
      captureMicrophone(getMedia, DEFAULT_MEDIA_SETTINGS, true, undefined, {
        current: () => false,
        acquired,
        discarded,
      }),
    ).rejects.toThrow("abgebrochen");
    expect(track.stop).toHaveBeenCalled();
    expect(getMedia).toHaveBeenCalledOnce();
    expect(discarded).toHaveBeenCalledWith(raw);
  });
});

it("resumes a hidden graph on visibility and disposes listeners without false failure", async () => {
  const windowTarget = new EventTarget(),
    documentTarget = Object.assign(new EventTarget(), {
      visibilityState: "visible",
    });
  vi.stubGlobal("window", windowTarget);
  vi.stubGlobal("document", documentTarget);
  const contexts: TestContext[] = [];
  class TestContext {
    state = "running";
    onstatechange: (() => void) | null = null;
    resume = vi.fn(async () => {
      this.state = "running";
    });
    close = vi.fn(async () => {
      this.state = "closed";
      this.onstatechange?.();
    });
    constructor() {
      contexts.push(this);
    }
    createMediaStreamSource() {
      return { connect() {}, disconnect() {} };
    }
    createGain() {
      return { gain: { value: 1 }, connect() {}, disconnect() {} };
    }
    createMediaStreamDestination() {
      return { stream: stream().raw };
    }
  }
  vi.stubGlobal("AudioContext", TestContext);
  const failed = vi.fn(),
    states = vi.fn();
  const processor = await createProcessor(
    stream().raw,
    { ...DEFAULT_MEDIA_SETTINGS, inputGain: 2 },
    "browser",
    failed,
    states,
  );
  const context = contexts[0]!;
  documentTarget.visibilityState = "hidden";
  context.state = "suspended";
  context.onstatechange?.();
  expect(context.resume).toHaveBeenCalledTimes(1);
  expect(states.mock.lastCall?.[0].contextState).toBe("suspended");
  documentTarget.visibilityState = "visible";
  documentTarget.dispatchEvent(new Event("visibilitychange"));
  await Promise.resolve();
  await Promise.resolve();
  expect(context.state).toBe("running");
  processor.dispose();
  windowTarget.dispatchEvent(new Event("focus"));
  expect(failed).not.toHaveBeenCalled();
  expect(context.resume).toHaveBeenCalledTimes(2);
});
it("reports unrecoverable context failure once without a resume loop", async () => {
  const contexts: TestContext[] = [];
  class TestContext {
    state = "running";
    onstatechange: (() => void) | null = null;
    reject = false;
    resume = vi.fn(async () => {
      if (this.reject) throw new Error("closed device");
    });
    close = async () => {};
    constructor() {
      contexts.push(this);
    }
    createMediaStreamSource() {
      return { connect() {}, disconnect() {} };
    }
    createGain() {
      return { gain: { value: 1 }, connect() {}, disconnect() {} };
    }
    createMediaStreamDestination() {
      return { stream: stream().raw };
    }
  }
  vi.stubGlobal("AudioContext", TestContext);
  const failed = vi.fn();
  const processor = await createProcessor(
    stream().raw,
    { ...DEFAULT_MEDIA_SETTINGS, inputGain: 2 },
    "browser",
    failed,
  );
  const context = contexts[0]!;
  context.reject = true;
  context.state = "suspended";
  context.onstatechange?.();
  await vi.waitFor(() => expect(failed).toHaveBeenCalledOnce());
  context.onstatechange?.();
  processor.setGain(1);
  await Promise.resolve();
  expect(context.resume).toHaveBeenCalledTimes(2);
  processor.dispose();
});

it("bounds a resume promise that never settles and cancels its timer on dispose", async () => {
  vi.useFakeTimers();
  const contexts: TestContext[] = [];
  class TestContext {
    state = "running";
    onstatechange: (() => void) | null = null;
    blocked = false;
    resume = vi.fn(() =>
      this.blocked ? new Promise<void>(() => {}) : Promise.resolve(),
    );
    close = async () => {
      this.state = "closed";
    };
    constructor() {
      contexts.push(this);
    }
    createMediaStreamSource() {
      return { connect() {}, disconnect() {} };
    }
    createGain() {
      return { gain: { value: 1 }, connect() {}, disconnect() {} };
    }
    createMediaStreamDestination() {
      return { stream: stream().raw };
    }
  }
  vi.stubGlobal("AudioContext", TestContext);
  const failed = vi.fn();
  const processor = await createProcessor(
    stream().raw,
    { ...DEFAULT_MEDIA_SETTINGS, inputGain: 2 },
    "browser",
    failed,
  );
  const context = contexts[0]!;
  context.blocked = true;
  context.state = "suspended";
  context.onstatechange?.();
  await vi.advanceTimersByTimeAsync(1_999);
  expect(failed).not.toHaveBeenCalled();
  await vi.advanceTimersByTimeAsync(1);
  expect(failed).toHaveBeenCalledOnce();
  expect(processor.usable()).toBe(false);
  processor.setGain(1);
  expect(context.resume).toHaveBeenCalledTimes(2);
  processor.dispose();
  await vi.advanceTimersByTimeAsync(5_000);
  expect(failed).toHaveBeenCalledOnce();
});

it("does not lose processorerror synchronously after the worklet ready acknowledgement", async () => {
  class Context {
    state = "running";
    sampleRate = 48_000;
    audioWorklet = { addModule: async () => {} };
    resume = async () => {};
    close = async () => {
      this.state = "closed";
    };
    createMediaStreamSource() {
      return { connect() {}, disconnect() {} };
    }
    createGain() {
      return { gain: { value: 1 }, connect() {}, disconnect() {} };
    }
    createMediaStreamDestination() {
      return { stream: stream().raw };
    }
  }
  class Worklet {
    onprocessorerror: (() => void) | null = null;
    port = {
      onmessage: null as ((event: { data: { ready: boolean } }) => void) | null,
      postMessage() {},
      close() {},
    };
    constructor() {
      queueMicrotask(() => {
        this.port.onmessage?.({ data: { ready: true } });
        this.onprocessorerror?.();
      });
    }
    connect() {}
    disconnect() {}
  }
  vi.stubGlobal("AudioContext", Context);
  vi.stubGlobal("AudioWorkletNode", Worklet);
  vi.stubGlobal("crypto", {
    subtle: {
      digest: async () =>
        Uint8Array.from(
          "e66d0eaef35d3774e86377efa8b9897e5b226284fb53059f0c1444881888b71c".match(
            /../g,
          )!,
          (part) => parseInt(part, 16),
        ).buffer,
    },
  });
  vi.stubGlobal(
    "fetch",
    vi.fn(
      async () => new Response(new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0])),
    ),
  );
  const first = stream(),
    second = stream(),
    failed = vi.fn();
  const getMedia = vi
    .fn()
    .mockResolvedValueOnce(first.raw)
    .mockResolvedValueOnce(second.raw);
  const result = await captureMicrophone(
    getMedia,
    DEFAULT_MEDIA_SETTINGS,
    false,
    failed,
  );
  expect(failed).toHaveBeenCalledOnce();
  expect(first.track.stop).toHaveBeenCalled();
  expect(result.raw).toBe(second.raw);
  expect(result.processor.usable()).toBe(true);
  expect(result.processor.info.actual).toBe("browser");
  result.processor.dispose();
});

describe("noise model download", () => {
  const MODEL = [0, 97, 115, 109, 1, 0, 0, 0];
  function enhancedGraph() {
    class Context {
      state = "running";
      sampleRate = 48_000;
      audioWorklet = { addModule: async () => {} };
      resume = async () => {};
      close = async () => {
        this.state = "closed";
      };
      createMediaStreamSource() {
        return { connect() {}, disconnect() {} };
      }
      createGain() {
        return { gain: { value: 1 }, connect() {}, disconnect() {} };
      }
      createMediaStreamDestination() {
        return { stream: stream().raw };
      }
    }
    class Worklet {
      onprocessorerror: (() => void) | null = null;
      port = {
        onmessage: null as
          ((event: { data: { ready: boolean } }) => void) | null,
        postMessage() {},
        close() {},
      };
      constructor() {
        queueMicrotask(() => this.port.onmessage?.({ data: { ready: true } }));
      }
      connect() {}
      disconnect() {}
    }
    vi.stubGlobal("AudioContext", Context);
    vi.stubGlobal("AudioWorkletNode", Worklet);
    vi.stubGlobal("crypto", {
      subtle: {
        digest: async () =>
          Uint8Array.from(
            "e66d0eaef35d3774e86377efa8b9897e5b226284fb53059f0c1444881888b71c".match(
              /../g,
            )!,
            (part) => parseInt(part, 16),
          ).buffer,
      },
    });
  }
  /** A body that sends one byte every `everyMs`, and stops when aborted. */
  function trickle(everyMs: number, bytes: number[], stallAfter = Infinity) {
    return vi.fn(async (_url: string, init?: RequestInit) => {
      let sent = 0;
      let ended = false;
      let wake = () => {};
      const body = new ReadableStream<Uint8Array>({
        start(controller) {
          init?.signal?.addEventListener("abort", () => {
            if (ended) return;
            ended = true;
            controller.error(init.signal?.reason);
            wake();
          });
        },
        pull(controller) {
          return new Promise<void>((resolve) => {
            wake = resolve;
            if (ended || sent >= stallAfter) return;
            setTimeout(() => {
              if (ended) return;
              if (sent >= bytes.length) {
                ended = true;
                controller.close();
              } else controller.enqueue(new Uint8Array([bytes[sent++]!]));
              resolve();
            }, everyMs);
          });
        },
      });
      return new Response(body);
    });
  }

  it("keeps a slow download that keeps arriving", async () => {
    vi.useFakeTimers();
    enhancedGraph();
    // A total deadline would run on the same clock as the download.
    vi.spyOn(AbortSignal, "timeout").mockImplementation((ms) => {
      const abort = new AbortController();
      setTimeout(
        () => abort.abort(new DOMException("timeout", "TimeoutError")),
        ms,
      );
      return abort.signal;
    });
    // Eight bytes, three seconds apart: longer than any total limit of 8 s.
    vi.stubGlobal("fetch", trickle(3_000, MODEL));
    const getMedia = vi.fn(async () => stream().raw);
    const result = captureMicrophone(getMedia, DEFAULT_MEDIA_SETTINGS);
    await vi.advanceTimersByTimeAsync(40_000);
    const { processor } = await result;
    expect(getMedia).toHaveBeenCalledOnce();
    expect(processor.info.actual).toBe("enhanced");
    processor.dispose();
  });

  it("gives up on a stalled download and waits before fetching again", async () => {
    vi.useFakeTimers();
    enhancedGraph();
    const fetchModel = trickle(100, MODEL, 2);
    vi.stubGlobal("fetch", fetchModel);
    const getMedia = vi.fn(async () => stream().raw);
    const first = captureMicrophone(getMedia, DEFAULT_MEDIA_SETTINGS);
    await vi.advanceTimersByTimeAsync(9_000);
    expect((await first).processor.info.actual).toBe("browser");
    expect(fetchModel).toHaveBeenCalledOnce();
    // The next capture right after does not pull the model again.
    const second = await captureMicrophone(getMedia, DEFAULT_MEDIA_SETTINGS);
    expect(second.processor.info.actual).toBe("browser");
    expect(fetchModel).toHaveBeenCalledOnce();
    // Later it tries again.
    await vi.advanceTimersByTimeAsync(31_000);
    vi.stubGlobal("fetch", trickle(1, MODEL));
    const third = captureMicrophone(getMedia, DEFAULT_MEDIA_SETTINGS);
    await vi.advanceTimersByTimeAsync(100);
    expect((await third).processor.info.actual).toBe("enhanced");
  });
});

describe("input sensitivity gate", () => {
  function gateGraph() {
    const nodes: Array<{
      name: string;
      options: { processorOptions: Record<string, unknown> };
      sent: unknown[];
      port: { onmessage: ((event: { data: unknown }) => void) | null };
    }> = [];
    class Context {
      state = "running";
      sampleRate = 44_100;
      audioWorklet = { addModule: vi.fn(async () => {}) };
      resume = async () => {};
      close = async () => {
        this.state = "closed";
      };
      createMediaStreamSource() {
        return { connect() {}, disconnect() {} };
      }
      createGain() {
        return { gain: { value: 1 }, connect() {}, disconnect() {} };
      }
      createMediaStreamDestination() {
        return { stream: stream().raw };
      }
    }
    class Worklet {
      onprocessorerror: (() => void) | null = null;
      sent: unknown[] = [];
      port = {
        onmessage: null as ((event: { data: unknown }) => void) | null,
        postMessage: (data: unknown) => this.sent.push(data),
        close() {},
      };
      constructor(
        _context: unknown,
        public name: string,
        public options: { processorOptions: Record<string, unknown> },
      ) {
        nodes.push(this);
        queueMicrotask(() => this.port.onmessage?.({ data: { ready: true } }));
      }
      connect() {}
      disconnect() {}
    }
    vi.stubGlobal("AudioContext", Context);
    vi.stubGlobal("AudioWorkletNode", Worklet);
    return nodes;
  }

  it("runs a gate of its own with browser filters, also at 100 % gain", async () => {
    const nodes = gateGraph();
    const processor = await createProcessor(
      stream().raw,
      { ...DEFAULT_MEDIA_SETTINGS, processingMode: "browser" },
      "browser",
    );
    expect(nodes.map((node) => node.name)).toEqual(["gelabber-gate"]);
    expect(nodes[0]!.options.processorOptions).toEqual({
      gate: { mode: "auto", thresholdDb: -50 },
      gain: 1,
    });
    expect(processor.info.gate).toBe("auto");
    expect(processor.info.addedBufferMs).toBe(30);
    processor.dispose();
  });

  it("follows the settings and the gain, and reports what it measures", async () => {
    const nodes = gateGraph();
    useInputGate.setState({ report: null, at: 0 });
    const states = vi.fn();
    const processor = await createProcessor(
      stream().raw,
      { ...DEFAULT_MEDIA_SETTINGS, processingMode: "browser" },
      "browser",
      undefined,
      states,
    );
    const node = nodes[0]!;
    processor.setGate({
      ...DEFAULT_MEDIA_SETTINGS,
      gateAuto: false,
      gateThresholdDb: -42,
    });
    processor.setGate({ ...DEFAULT_MEDIA_SETTINGS, voiceInput: "ptt" });
    processor.setGain(1.5);
    expect(node.sent).toEqual([
      { op: "gateConfig", config: { mode: "manual", thresholdDb: -42 } },
      { op: "gateConfig", config: { mode: "off", thresholdDb: -50 } },
      { op: "gain", value: 1.5 },
    ]);
    expect(states.mock.lastCall?.[0].gate).toBe("off");
    // Off, the gate adds no look-ahead: only the 10 ms reblocking is left.
    expect(states.mock.lastCall?.[0].addedBufferMs).toBe(10);
    node.port.onmessage?.({
      data: {
        op: "gate",
        mode: "auto",
        levelDb: -31,
        thresholdDb: -44,
        open: true,
      },
    });
    expect(useInputGate.getState().report).toEqual({
      mode: "auto",
      levelDb: -31,
      thresholdDb: -44,
      open: true,
    });
    processor.dispose();
    expect(node.sent.at(-1)).toEqual({ op: "dispose" });
  });

  it("keeps original sound and the fallback capture without a gate", async () => {
    const nodes = gateGraph();
    const original = await createProcessor(
      stream().raw,
      { ...DEFAULT_MEDIA_SETTINGS, processingMode: "original", inputGain: 1.5 },
      "original",
    );
    const { raw } = stream();
    const fallback = await createProcessor(
      raw,
      { ...DEFAULT_MEDIA_SETTINGS, processingMode: "browser" },
      "browser",
      undefined,
      undefined,
      true,
    );
    expect(nodes).toHaveLength(0);
    expect(original.info.gate).toBe("off");
    expect(fallback.stream).toBe(raw);
    expect(fallback.info.gate).toBeNull();
    original.dispose();
    fallback.dispose();
  });

  it("is off for original sound and push-to-talk", () => {
    expect(gateConfig(DEFAULT_MEDIA_SETTINGS).mode).toBe("auto");
    expect(
      gateConfig({ ...DEFAULT_MEDIA_SETTINGS, processingMode: "original" })
        .mode,
    ).toBe("off");
    expect(
      gateConfig({ ...DEFAULT_MEDIA_SETTINGS, voiceInput: "ptt" }).mode,
    ).toBe("off");
    expect(gateConfig({ ...DEFAULT_MEDIA_SETTINGS, gateAuto: false })).toEqual({
      mode: "manual",
      thresholdDb: -50,
    });
  });
});

describe("Enhanced voice: Strong (GEL-28)", () => {
  const RNNOISE = [0, 97, 115, 109, 1, 0, 0, 0];
  // An empty module with a custom section: valid, and told apart by length.
  const DFN = [0, 97, 115, 109, 1, 0, 0, 0, 0, 2, 1, 120];
  const MODEL = [9, 9, 9];
  const HASHES: Record<number, string> = {
    8: "e66d0eaef35d3774e86377efa8b9897e5b226284fb53059f0c1444881888b71c",
    12: DFN_WASM_HASH,
    3: DFN_MODEL_HASH,
  };
  type Node = {
    name: string;
    options: { processorOptions: Record<string, unknown> };
    port: { onmessage: ((event: { data: unknown }) => void) | null };
  };
  function strongGraph(clock = "performance") {
    const nodes: Node[] = [];
    class Context {
      state = "running";
      sampleRate = 48_000;
      audioWorklet = { addModule: vi.fn(async () => {}) };
      resume = async () => {};
      close = async () => {
        this.state = "closed";
      };
      createMediaStreamSource() {
        return { connect() {}, disconnect() {} };
      }
      createGain() {
        return { gain: { value: 1 }, connect() {}, disconnect() {} };
      }
      createMediaStreamDestination() {
        return { stream: stream().raw };
      }
    }
    class Worklet {
      onprocessorerror: (() => void) | null = null;
      port = {
        onmessage: null as ((event: { data: unknown }) => void) | null,
        postMessage() {},
        close() {},
      };
      constructor(
        _context: unknown,
        public name: string,
        public options: { processorOptions: Record<string, unknown> },
      ) {
        nodes.push(this);
        queueMicrotask(() =>
          this.port.onmessage?.({ data: { ready: true, clock } }),
        );
      }
      connect() {}
      disconnect() {}
    }
    vi.stubGlobal("AudioContext", Context);
    vi.stubGlobal("AudioWorkletNode", Worklet);
    vi.stubGlobal("crypto", {
      subtle: {
        digest: async (_algorithm: string, bytes: ArrayBuffer) =>
          Uint8Array.from(HASHES[bytes.byteLength]!.match(/../g)!, (part) =>
            parseInt(part, 16),
          ).buffer,
      },
    });
    const fetchAsset = vi.fn(
      async (url: string) =>
        new Response(
          new Uint8Array(
            url.startsWith("/audio/deepfilternet.wasm")
              ? DFN
              : url.startsWith("/audio/deepfilternet3-model.bin")
                ? MODEL
                : RNNOISE,
          ),
        ),
    );
    vi.stubGlobal("fetch", fetchAsset);
    return { nodes, fetchAsset };
  }
  const memory = new ArrayBuffer(65_536);
  const built = { memory, state: 8, buildMs: 500, blockMs: 2 };
  const strong = {
    ...DEFAULT_MEDIA_SETTINGS,
    processingMode: "strong" as const,
  };
  afterEach(() => setStrongBuilderForTests());

  it("runs DeepFilterNet3 beside RNNoise with the built model", async () => {
    const { nodes, fetchAsset } = strongGraph();
    const build = vi.fn(async () => built);
    setStrongBuilderForTests(build);
    const processor = await createProcessor(stream().raw, strong, "strong");
    expect(fetchAsset.mock.calls.map(([url]) => url)).toEqual(
      expect.arrayContaining([
        `/audio/deepfilternet.wasm?sha256=${DFN_WASM_HASH}`,
        `/audio/deepfilternet3-model.bin?sha256=${DFN_MODEL_HASH}`,
      ]),
    );
    const [module, model] = build.mock.calls[0]! as unknown as [
      WebAssembly.Module,
      ArrayBuffer,
    ];
    expect(module).toBeInstanceOf(WebAssembly.Module);
    expect(new Uint8Array(model)).toEqual(new Uint8Array(MODEL));
    expect(nodes.map((node) => node.name)).toEqual(["gelabber-strong"]);
    expect(nodes[0]!.options.processorOptions).toMatchObject({
      dfn: { memory, state: 8 },
      gate: { mode: "auto", thresholdDb: -50 },
      gain: 1,
    });
    expect(processor.enhanced).toBe(true);
    expect(processor.info).toMatchObject({
      requested: "strong",
      actual: "strong",
      message: "Starke Rauschunterdrückung aktiv (DeepFilterNet3)",
      addedBufferMs: 30,
      gate: "auto",
      strong: { dfnMs: null, fallback: null, clock: "performance" },
    });
    // Built once per page.
    const again = await createProcessor(stream().raw, strong, "strong");
    expect(build).toHaveBeenCalledOnce();
    processor.dispose();
    again.dispose();
  });

  it("shows DFN3's time per block as the worklet reports it", async () => {
    const { nodes } = strongGraph("date");
    setStrongBuilderForTests(async () => built);
    const states = vi.fn();
    const processor = await createProcessor(
      stream().raw,
      strong,
      "strong",
      undefined,
      states,
    );
    nodes[0]!.port.onmessage?.({
      data: { op: "strong", dfnMs: 2.5, rnnoiseMs: 0.4, maxMs: 3.1 },
    });
    expect(states.mock.lastCall?.[0].strong).toEqual({
      dfnMs: 2.5,
      rnnoiseMs: 0.4,
      maxMs: 3.1,
      clock: "date",
      fallback: null,
    });
    processor.dispose();
  });

  it("switches to Standard in place, says why and stays there", async () => {
    const { nodes } = strongGraph();
    setStrongBuilderForTests(async () => built);
    const states = vi.fn();
    const processor = await createProcessor(
      stream().raw,
      strong,
      "strong",
      undefined,
      states,
    );
    nodes[0]!.port.onmessage?.({
      data: { op: "strong", fallback: "slow", dfnMs: 8.1, totalMs: 8.5 },
    });
    const info = states.mock.lastCall?.[0];
    expect(info.actual).toBe("enhanced");
    expect(info.requested).toBe("strong");
    expect(info.message).toBe(
      "Starker Filter auf Standard umgestellt: Gerät zu langsam",
    );
    expect(info.strong).toMatchObject({ fallback: "slow", dfnMs: 8.1 });
    expect(processor.info.actual).toBe("enhanced");
    // The next capture starts with Standard, until Strong is chosen again.
    const next = await createProcessor(stream().raw, strong, "strong");
    expect(nodes[1]!.name).toBe("gelabber-rnnoise");
    expect(next.info.actual).toBe("enhanced");
    expect(next.info.strong?.fallback).toBe("slow");
    forgetStrongFallback();
    const retry = await createProcessor(stream().raw, strong, "strong");
    expect(nodes[2]!.name).toBe("gelabber-strong");
    [processor, next, retry].forEach((p) => p.dispose());
  });

  it("uses Standard when DeepFilterNet3 does not load", async () => {
    const { nodes } = strongGraph();
    setStrongBuilderForTests(async () => {
      throw new Error("Starker Filter startet nicht");
    });
    const processor = await createProcessor(stream().raw, strong, "strong");
    expect(nodes.map((node) => node.name)).toEqual(["gelabber-rnnoise"]);
    expect(processor.info.actual).toBe("enhanced");
    expect(processor.info.message).toBe(
      "Starker Filter nicht verfügbar (Starker Filter startet nicht); Standard aktiv",
    );
    expect(processor.info.strong?.fallback).toBe("load");
    processor.dispose();
  });

  it("uses Standard when the worker already measured a slow device", async () => {
    const { nodes } = strongGraph();
    setStrongBuilderForTests(async () => ({ ...built, blockMs: 12 }));
    const processor = await createProcessor(stream().raw, strong, "strong");
    expect(nodes.map((node) => node.name)).toEqual(["gelabber-rnnoise"]);
    expect(processor.info.strong).toMatchObject({
      fallback: "slow",
      dfnMs: 12,
    });
    processor.dispose();
  });

  it("falls back to browser filters where no worklet runs", async () => {
    vi.stubGlobal("AudioWorkletNode", undefined);
    const { raw } = stream();
    const result = await captureMicrophone(async () => raw, strong);
    expect(result.processor.info.requested).toBe("strong");
    expect(result.processor.info.actual).toBe("browser");
    expect(result.processor.nativeFallback).toBe(true);
  });
});
