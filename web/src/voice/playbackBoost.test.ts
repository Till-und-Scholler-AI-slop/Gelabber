import { afterEach, describe, expect, it, vi } from "vitest";
import {
  boost,
  boostOutput,
  isBoosted,
  releaseBoost,
  resetPlaybackBoostForTests,
  startBoostOutput,
} from "./playbackBoost.ts";

class FakeParam {
  value = 1;
  setTargetAtTime(value: number) {
    this.value = value;
  }
}
class FakeNode {
  connected: unknown[] = [];
  disconnected = false;
  connect(node: unknown) {
    this.connected.push(node);
    return node;
  }
  disconnect() {
    this.connected = [];
    this.disconnected = true;
  }
}
class FakeGainNode extends FakeNode {
  gain = new FakeParam();
}
class FakeSourceNode extends FakeNode {
  constructor(readonly stream: FakeMediaStream) {
    super();
  }
}
class FakeContext {
  static instances: FakeContext[] = [];
  state: AudioContextState = "running";
  currentTime = 0;
  gains: FakeGainNode[] = [];
  sources: FakeSourceNode[] = [];
  limiter = Object.assign(new FakeNode(), {
    threshold: new FakeParam(),
    knee: new FakeParam(),
    ratio: new FakeParam(),
    attack: new FakeParam(),
    release: new FakeParam(),
  });
  destination = Object.assign(new FakeNode(), { stream: { id: "boost" } });
  resume = vi.fn(async () => {});
  close = vi.fn(async () => {
    this.state = "closed";
  });
  constructor() {
    FakeContext.instances.push(this);
  }
  createDynamicsCompressor() {
    return this.limiter;
  }
  createMediaStreamDestination() {
    return this.destination;
  }
  createGain() {
    const gain = new FakeGainNode();
    this.gains.push(gain);
    return gain;
  }
  createMediaStreamSource(stream: FakeMediaStream) {
    const source = new FakeSourceNode(stream);
    this.sources.push(source);
    return source;
  }
}
class FakeMediaStream {
  constructor(private tracks: MediaStreamTrack[] = []) {}
  getTracks() {
    return [...this.tracks];
  }
  getAudioTracks() {
    return this.tracks.filter((track) => track.kind === "audio");
  }
  addTrack(track: MediaStreamTrack) {
    this.tracks.push(track);
  }
  removeTrack(track: MediaStreamTrack) {
    this.tracks = this.tracks.filter((item) => item !== track);
  }
}
class FakeAudio {
  autoplay = false;
  srcObject: unknown = null;
  play = vi.fn(async () => {});
  pause = vi.fn();
  setAttribute() {}
}

function track(id: string): MediaStreamTrack {
  return { id, kind: "audio", readyState: "live" } as MediaStreamTrack;
}
function element(tracks: MediaStreamTrack[]): HTMLAudioElement {
  const el = new FakeAudio();
  el.srcObject = new FakeMediaStream(tracks);
  return el as unknown as HTMLAudioElement;
}
function install() {
  FakeContext.instances = [];
  vi.stubGlobal("AudioContext", FakeContext);
  vi.stubGlobal("MediaStream", FakeMediaStream);
  vi.stubGlobal("Audio", FakeAudio);
}

describe("playback boost", () => {
  afterEach(() => {
    resetPlaybackBoostForTests();
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });

  it("routes every track of an element through one gain into the limiter", () => {
    install();
    const a = track("a"),
      b = track("b");
    const el = element([a, b]);
    expect(boost(el, 1.5)).toBe(true);
    const ctx = FakeContext.instances[0]!;
    expect(ctx.gains).toHaveLength(1);
    expect(ctx.gains[0]!.gain.value).toBe(1.5);
    expect(ctx.gains[0]!.connected).toEqual([ctx.limiter]);
    expect(ctx.limiter.connected).toEqual([ctx.destination]);
    expect(ctx.limiter.threshold.value).toBeLessThan(0);
    expect(ctx.sources.map((source) => source.stream.getTracks())).toEqual([
      [a],
      [b],
    ]);
    expect(boostOutput()?.srcObject).toBe(ctx.destination.stream);

    // Later track changes follow the element's stream.
    const stream = el.srcObject as unknown as FakeMediaStream;
    stream.removeTrack(a);
    const c = track("c");
    stream.addTrack(c);
    expect(boost(el, 2)).toBe(true);
    expect(ctx.gains).toHaveLength(1);
    expect(ctx.gains[0]!.gain.value).toBe(2);
    expect(ctx.sources[0]!.disconnected).toBe(true);
    expect(ctx.sources[2]!.stream.getTracks()).toEqual([c]);
  });

  it("shares one context and closes it with the last boost", () => {
    install();
    const first = element([track("a")]),
      second = element([track("b")]);
    boost(first, 1.2);
    boost(second, 1.8);
    expect(FakeContext.instances).toHaveLength(1);
    const ctx = FakeContext.instances[0]!;
    expect(ctx.gains.map((gain) => gain.gain.value)).toEqual([1.2, 1.8]);
    expect(releaseBoost(first)).toBe(true);
    expect(ctx.close).not.toHaveBeenCalled();
    expect(isBoosted(first)).toBe(false);
    expect(releaseBoost(second)).toBe(true);
    expect(ctx.close).toHaveBeenCalled();
    expect(boostOutput()).toBeNull();
    expect(releaseBoost(second)).toBe(false);
  });

  it("stays on the element where Web Audio or a stream is missing", () => {
    expect(boost(element([track("a")]), 1.5)).toBe(false);
    install();
    const empty = new FakeAudio() as unknown as HTMLAudioElement;
    expect(boost(empty, 1.5)).toBe(false);
    expect(FakeContext.instances).toHaveLength(0);
  });

  it("reports blocked output while the context cannot resume", async () => {
    install();
    vi.useFakeTimers();
    const el = element([track("a")]);
    boost(el, 1.5);
    const ctx = FakeContext.instances[0]!;
    ctx.state = "suspended";
    ctx.resume.mockImplementation(() => new Promise(() => {}));
    const started = startBoostOutput();
    const outcome = expect(started).rejects.toMatchObject({
      name: "NotAllowedError",
    });
    await vi.advanceTimersByTimeAsync(1000);
    await outcome;
    ctx.resume.mockImplementation(async () => {
      ctx.state = "running";
    });
    await startBoostOutput();
    expect((boostOutput() as unknown as FakeAudio).play).toHaveBeenCalled();
  });
});
