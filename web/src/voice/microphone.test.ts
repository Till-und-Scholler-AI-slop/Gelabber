import { describe, expect, it, vi } from "vitest";

import {
  createMicrophoneSession,
  type MicrophoneTestState,
} from "./microphone.ts";

function setup(
  options: {
    getUserMedia?: (stream: MediaStream) => Promise<MediaStream>;
    noContext?: boolean;
    noDevices?: boolean;
    resume?: () => Promise<void>;
  } = {},
) {
  const states: MicrophoneTestState[] = [];
  const levels: number[] = [];
  const cancelled: number[] = [];
  const contexts: FakeAudioContext[] = [];
  let measure: FrameRequestCallback | undefined;
  const track = Object.assign(new EventTarget(), { stop: vi.fn() });
  const stream = { getTracks: () => [track] } as unknown as MediaStream;
  const source = { connect: vi.fn(), disconnect: vi.fn() };
  const analyser = {
    fftSize: 0,
    getByteTimeDomainData: (values: Uint8Array) => values.fill(160),
    disconnect: vi.fn(),
  };
  class FakeAudioContext {
    state = options.resume ? "suspended" : "running";
    constructor() {
      contexts.push(this);
    }
    resume = options.resume ?? (() => Promise.resolve());
    close = vi.fn(() => {
      this.state = "closed";
      return Promise.resolve();
    });
    createAnalyser = () => analyser;
    createMediaStreamSource = () => source;
  }
  const getUserMedia = vi.fn(() =>
    options.getUserMedia
      ? options.getUserMedia(stream)
      : Promise.resolve(stream),
  );
  const session = createMicrophoneSession(
    {
      onState: (state) => states.push(state),
      onLevel: (level) => levels.push(level),
    },
    {
      mediaDevices: options.noDevices ? undefined : { getUserMedia },
      AudioContextClass: options.noContext
        ? undefined
        : (FakeAudioContext as unknown as typeof AudioContext),
      requestFrame: (callback) => {
        measure = callback;
        return 42;
      },
      cancelFrame: (id) => cancelled.push(id),
    },
    { deviceId: { ideal: "chosen-microphone" } },
  );
  return {
    session,
    states,
    levels,
    track,
    contexts,
    cancelled,
    source,
    analyser,
    getUserMedia,
    measure: (time: number) => measure?.(time),
  };
}

describe("local microphone meter", () => {
  it("does not request capture before explicit start; meters the selected device without playback", async () => {
    const f = setup();
    expect(f.getUserMedia).not.toHaveBeenCalled();
    await f.session.start();
    expect(f.getUserMedia).toHaveBeenCalledWith({
      audio: { deviceId: { ideal: "chosen-microphone" } },
      video: false,
    });
    expect(f.states.map((state) => state.phase)).toEqual(["pending", "active"]);
    expect(f.source.connect).toHaveBeenCalledExactlyOnceWith(f.analyser);
    f.measure(100);
    expect(f.levels).toEqual([88]);
    f.session.stop();
    expect(f.track.stop).toHaveBeenCalledOnce();
    expect(f.contexts[0].close).toHaveBeenCalledOnce();
    expect(f.source.disconnect).toHaveBeenCalledOnce();
    expect(f.analyser.disconnect).toHaveBeenCalledOnce();
    expect(f.cancelled).toEqual([42]);
    f.measure(200);
    expect(f.levels).toEqual([88]);
  });

  it("disposes late permission success without reopening the UI or analysing", async () => {
    let grant!: () => void;
    const f = setup({
      getUserMedia: (stream) =>
        new Promise((resolve) => {
          grant = () => resolve(stream);
        }),
    });
    const pending = f.session.start();
    f.session.stop();
    grant();
    await pending;
    expect(f.track.stop).toHaveBeenCalledOnce();
    expect(f.contexts[0].state).toBe("closed");
    expect(f.states.map((state) => state.phase)).toEqual(["pending"]);
    expect(f.source.connect).not.toHaveBeenCalled();
  });

  it("suppresses late permission rejection after closing", async () => {
    let deny!: () => void;
    const f = setup({
      getUserMedia: () =>
        new Promise((_, reject) => {
          deny = () => reject(new DOMException("Denied", "NotAllowedError"));
        }),
    });
    const pending = f.session.start();
    f.session.stop();
    deny();
    await pending;
    expect(f.states.map((state) => state.phase)).toEqual(["pending"]);
    expect(f.contexts[0].state).toBe("closed");
  });

  it.each([
    "NotAllowedError",
    "NotFoundError",
    "NotReadableError",
    "SecurityError",
  ])("reports %s and closes the audio context", async (name) => {
    const f = setup({
      getUserMedia: () => Promise.reject(new DOMException("Denied", name)),
    });
    await f.session.start();
    expect(f.states.at(-1)?.phase).toBe("error");
    expect(f.states.at(-1)?.message).not.toContain("konnte nicht gestartet");
    expect(f.contexts[0].state).toBe("closed");
  });

  it("stops analysis when the microphone is disconnected", async () => {
    const f = setup();
    await f.session.start();
    f.track.dispatchEvent(new Event("ended"));
    expect(f.states.at(-1)?.phase).toBe("error");
    expect(f.cancelled).toEqual([42]);
    expect(f.track.stop).toHaveBeenCalledOnce();
    expect(f.contexts[0].state).toBe("closed");
  });

  it("releases capture if the browser cannot supply an analyser", async () => {
    const f = setup({ noContext: true });
    await f.session.start();
    expect(f.states.at(-1)?.phase).toBe("error");
    expect(f.track.stop).toHaveBeenCalledOnce();
  });

  it("does not duplicate capture on repeated start or after stop", async () => {
    const f = setup();
    await Promise.all([f.session.start(), f.session.start()]);
    f.session.stop();
    f.session.stop();
    await f.session.start();
    expect(f.getUserMedia).toHaveBeenCalledOnce();
    expect(f.track.stop).toHaveBeenCalledOnce();
  });

  it("reports unavailable browser capture without allocating resources", async () => {
    const f = setup({ noDevices: true });
    await f.session.start();
    expect(f.states.at(-1)?.phase).toBe("error");
    expect(f.getUserMedia).not.toHaveBeenCalled();
    expect(f.contexts).toHaveLength(0);
  });

  it("releases capture while a suspended context resumes, without a late active state", async () => {
    let resume!: () => void;
    const f = setup({
      resume: () =>
        new Promise((resolve) => {
          resume = resolve;
        }),
    });
    const pending = f.session.start();
    // Let getUserMedia settle and enter the awaited context resume.
    await Promise.resolve();
    f.session.stop();
    resume();
    await pending;
    expect(f.track.stop).toHaveBeenCalledOnce();
    expect(f.contexts[0].state).toBe("closed");
    expect(f.states.some((state) => state.phase === "active")).toBe(false);
  });
});
