import { afterEach, describe, expect, it, vi } from "vitest";
import { captureMicrophone, createProcessor } from "./audioProcessing.ts";
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
});
describe("microphone processor ownership", () => {
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
      audio: { noiseSuppression: false, autoGainControl: false },
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
