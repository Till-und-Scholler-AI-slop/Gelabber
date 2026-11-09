import { afterEach, describe, expect, it, vi } from "vitest";
import {
  captureMicrophone,
  createProcessor,
  resetNoiseModelForTests,
} from "./audioProcessing.ts";
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
  it.each(["enhanced", "browser", "original"] as const)(
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
      expect(result.processor.info.message).toContain("Mikrofonverstärkung 100 %");
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
