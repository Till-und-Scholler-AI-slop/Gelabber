// "Enhanced voice: Strong" (GEL-28): the model build in
// public/audio/deepfilternet-worker.js and the gelabber-strong processor in
// public/audio/rnnoise-worklet.js, run outside a browser like
// inputGate.test.mjs, with the checked-in WebAssembly and model.
/* global URL, WebAssembly, performance */
import fs from "node:fs";
import vm from "node:vm";
import { beforeAll, describe, expect, it } from "vitest";

const RATE = 48_000;
const BLOCK = 480;
const QUANTUM = 128;
const asset = (name) =>
  fs.readFileSync(new URL(`../../public/audio/${name}`, import.meta.url));
// The thresholds of the GEL-28 measurement: the full model on every block.
const PARAMS = [100, -15, 35, 35];

/** The worklet file in a context of its own. `clock.t` is what
 * `performance.now()` returns there; `currentFrame` follows `run`; `sent`
 * holds what processors post on their ports. */
function loadWorklet() {
  const processors = {};
  const clock = { t: 0 };
  const sent = [];
  const context = vm.createContext({
    WebAssembly,
    sampleRate: RATE,
    currentFrame: 0,
    performance: { now: () => clock.t },
    AudioWorkletProcessor: class {
      port = { postMessage: (data) => sent.push(data), onmessage: null };
    },
    registerProcessor(name, ctor) {
      processors[name] = ctor;
    },
  });
  vm.runInContext(asset("rnnoise-worklet.js").toString("utf8"), context);
  return {
    processors,
    clock,
    context,
    sent,
    StrongLoad: vm.runInContext("StrongLoad", context),
  };
}

/** What the worker answers for `model`. */
function buildInWorker(module, model) {
  let answer;
  const self = {
    postMessage(data) {
      answer = data;
    },
  };
  vm.runInNewContext(asset("deepfilternet-worker.js").toString("utf8"), {
    WebAssembly,
    performance,
    self,
  });
  self.onmessage({ data: { module, model, params: PARAMS } });
  return answer;
}

function tone(ms, hz, amplitude = 0.2) {
  const out = new Float32Array((RATE * ms) / 1000);
  for (let n = 0; n < out.length; n++)
    out[n] = amplitude * Math.sin((2 * Math.PI * hz * n) / RATE);
  return out;
}
/** Speech-like bursts over a little noise. */
function voice(ms, seed = 3) {
  const out = tone(ms, 180);
  let state = seed;
  for (let n = 0; n < out.length; n++) {
    state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
    const on = Math.floor(n / (RATE / 4)) % 2 === 0;
    out[n] = (on ? out[n] : 0) + 0.01 * (2 * (state / 0xffffffff) - 1);
  }
  return out;
}

let dfnModule, rnnoiseModule, built;
beforeAll(() => {
  dfnModule = new WebAssembly.Module(asset("deepfilternet.wasm"));
  rnnoiseModule = new WebAssembly.Module(asset("rnnoise.wasm"));
  const model = asset("deepfilternet3-model.bin");
  built = buildInWorker(
    dfnModule,
    model.buffer.slice(model.byteOffset, model.byteOffset + model.byteLength),
  );
}, 120_000);

describe("model build in the worker", () => {
  it("answers with the module's memory after the build and a warm-up time", () => {
    expect(built.ok).toBe(true);
    expect(built.memory).toBeInstanceOf(ArrayBuffer);
    expect(built.memory.byteLength % 65_536).toBe(0);
    expect(built.state).toBeGreaterThan(0);
    expect(built.blockMs).toBeGreaterThan(0);
  });

  it("reports a model that does not build", () => {
    const answer = buildInWorker(dfnModule, new Uint8Array([1, 2, 3]).buffer);
    expect(answer.ok).toBe(false);
    expect(answer.memory).toBeUndefined();
  });
});

describe("Strong load monitor", () => {
  const { StrongLoad } = loadWorklet();
  /** Feeds `blocks` blocks of `ms` each and returns the first verdict. */
  function feed(load, blocks, dfnMs, rnnoiseMs = 0.4) {
    for (let k = 0; k < blocks; k++) {
      const verdict = load.block(
        typeof dfnMs === "function" ? dfnMs(k) : dfnMs,
        rnnoiseMs,
      );
      if (verdict) return { verdict, at: k };
    }
    return null;
  }

  it("keeps a device that has time left", () => {
    expect(feed(new StrongLoad(), 2000, 2)).toBeNull();
  });

  it("gives up when DFN3 and RNNoise take most of each 10 ms block", () => {
    // 7.6 ms of 10: judged after a second of warm-up and three seconds.
    expect(feed(new StrongLoad(), 2000, 7.2)).toEqual({
      verdict: "slow",
      at: 399,
    });
  });

  it("does not judge the first second, while the code warms up", () => {
    expect(feed(new StrongLoad(), 2000, (k) => (k < 100 ? 30 : 2))).toBeNull();
  });

  it("gives up when one block in twenty takes longer than its 10 ms", () => {
    const late = (k) => (k % 16 === 0 ? 12 : 2);
    expect(feed(new StrongLoad(), 2000, late)?.verdict).toBe("slow");
    const rare = (k) => (k % 25 === 0 ? 12 : 2);
    expect(feed(new StrongLoad(), 2000, rare)).toBeNull();
  });

  /** Render quanta whose wall-clock time runs `stretch` times their
   * length, after a second of blocks; at quantum `gapAt` the thread stalls
   * for `gapMs`. */
  function quanta(load, seconds, stretch, gapAt = -1, gapMs = 5000) {
    feed(load, 101, 2);
    let wall = 1000,
      frame = 0;
    for (let q = 0; q < (seconds * RATE) / QUANTUM; q++) {
      wall += ((QUANTUM * 1000) / RATE) * stretch + (q === gapAt ? gapMs : 0);
      frame += QUANTUM;
      const verdict = load.quantum(wall, frame);
      if (verdict) return verdict;
    }
    return null;
  }

  it("gives up when the audio falls behind the clock", () => {
    expect(quanta(new StrongLoad(), 20, 1.15)).toBe("underrun");
    expect(quanta(new StrongLoad(), 20, 1.03)).toBeNull();
  });

  it("starts over after a pause such as a suspended context", () => {
    expect(quanta(new StrongLoad(), 20, 1, 400)).toBeNull();
  });

  it("takes one stall for an interruption, not for lag", () => {
    // An output device switch or an OS audio interruption: 0.6 s at once.
    expect(quanta(new StrongLoad(), 20, 1, 400, 600)).toBeNull();
    expect(quanta(new StrongLoad(), 20, 1, 400, 120)).toBeNull();
  });

  it("needs two windows in a row that fall behind", () => {
    const load = new StrongLoad();
    feed(load, 101, 2);
    let wall = 1000,
      frame = 0;
    const step = (QUANTUM * 1000) / RATE;
    const verdicts = [];
    // Behind by 15 % for 6 s, then on time, then behind again for 12 s.
    for (const [seconds, stretch] of [
      [6, 1.15],
      [6, 1],
      [12, 1.15],
    ])
      for (let q = 0; q < (seconds * RATE) / QUANTUM; q++) {
        wall += step * stretch;
        frame += QUANTUM;
        const verdict = load.quantum(wall, frame);
        if (verdict) verdicts.push(Math.round(wall / 1000));
      }
    expect(verdicts).toHaveLength(1);
    expect(verdicts[0]).toBeGreaterThan(18);
  });

  it("reports the mean time per block since the last report", () => {
    const load = new StrongLoad();
    feed(load, 50, 2, 0.5);
    feed(load, 50, 4, 0.5);
    expect(load.report()).toEqual({ dfnMs: 3, rnnoiseMs: 0.5, maxMs: 4.5 });
    feed(load, 10, 1, 0.5);
    expect(load.report().dfnMs).toBe(1);
  });
});

describe("Strong processor", { timeout: 60_000 }, () => {
  function strong(
    gate = { mode: "off" },
    dfn = { module: dfnModule, memory: built.memory, state: built.state },
  ) {
    const worklet = loadWorklet();
    const processor = new worklet.processors["gelabber-strong"]({
      processorOptions: { model: rnnoiseModule, dfn, gate, gain: 1 },
    });
    return { ...worklet, processor, messages: worklet.sent };
  }

  /** Runs `input` in render quanta. Unless `stretch` says otherwise, the
   * wall clock keeps pace with the audio, plus whatever `cost` adds. */
  function run(setup, input, { stretch = 1 } = {}) {
    const { processor, clock, context } = setup;
    const output = new Float32Array(input.length);
    for (let i = 0; i < input.length; i += QUANTUM) {
      clock.t = Math.max(
        clock.t,
        ((context.currentFrame * 1000) / RATE) * stretch,
      );
      processor.process(
        [[input.subarray(i, i + QUANTUM)]],
        [[output.subarray(i, i + QUANTUM)]],
      );
      context.currentFrame += QUANTUM;
    }
    return output;
  }

  /** Makes each DFN3 block take `ms` on the worklet's clock. */
  function cost(setup, ms) {
    const dfn = setup.processor.dfn;
    const process = dfn.process.bind(dfn);
    dfn.process = () => {
      setup.clock.t += ms;
      return process();
    };
  }

  /** DFN3 alone on `input`, one block at a time, from the same build. */
  function direct(input) {
    const api = new WebAssembly.Instance(dfnModule, {}).exports;
    api.memory.grow(
      (built.memory.byteLength - api.memory.buffer.byteLength) / 65_536,
    );
    new Uint8Array(api.memory.buffer).set(new Uint8Array(built.memory));
    const out = new Float32Array(input.length);
    for (let n = 0; n + BLOCK <= input.length; n += BLOCK) {
      new Float32Array(api.memory.buffer, api.df_input(built.state), BLOCK).set(
        input.subarray(n, n + BLOCK),
      );
      api.df_process(built.state);
      out.set(
        new Float32Array(api.memory.buffer, api.df_output(built.state), BLOCK),
        n,
      );
    }
    return out;
  }

  it("sends DeepFilterNet3's output, 10 ms reblocking later", () => {
    const setup = strong();
    const input = voice(1000);
    const output = run(setup, input);
    const expected = direct(input);
    expect(Array.from(output.subarray(0, BLOCK))).toEqual(
      new Array(BLOCK).fill(0),
    );
    for (let n = BLOCK; n < output.length; n++)
      if (output[n] !== expected[n - BLOCK])
        throw new Error(`sample ${n}: ${output[n]} != ${expected[n - BLOCK]}`);
    expect(setup.messages.some((m) => m.fallback)).toBe(false);
  });

  it("gives the gate RNNoise's speech probability one block later", () => {
    const setup = strong({ mode: "auto" });
    const seen = [];
    const block = setup.processor.gate.block.bind(setup.processor.gate);
    setup.processor.gate.block = (frame, speech) => {
      seen.push(speech);
      return block(frame, speech);
    };
    const input = voice(500);
    run(setup, input);
    // RNNoise alone on the same input.
    const rnnoise = loadWorklet().processors["gelabber-rnnoise"];
    const reference = new rnnoise({
      processorOptions: { model: rnnoiseModule, gate: { mode: "off" } },
    });
    const probabilities = [];
    const original = reference.api.rnnoise_process_frame;
    for (let n = 0; n + BLOCK <= input.length; n += BLOCK) {
      for (let j = 0; j < BLOCK; j++)
        reference.heap[(reference.input >> 2) + j] = input[n + j] * 32768;
      probabilities.push(
        original(reference.state, reference.output, reference.input),
      );
    }
    expect(seen[0]).toBe(0);
    expect(seen.slice(1)).toEqual(probabilities.slice(0, seen.length - 1));
  });

  it("reports DFN3's time per block about once a second", () => {
    const setup = strong();
    cost(setup, 2);
    run(setup, voice(3000));
    const reports = setup.messages.filter((m) => m.op === "strong");
    expect(reports.length).toBeGreaterThanOrEqual(2);
    expect(reports.at(-1).dfnMs).toBeCloseTo(2, 5);
    expect(reports.every((m) => !m.fallback)).toBe(true);
  });

  it("switches to RNNoise's output in place when DFN3 is too slow", () => {
    const setup = strong();
    cost(setup, 8);
    const input = voice(6000);
    const output = run(setup, input);
    const fallback = setup.messages.find((m) => m.fallback);
    expect(fallback).toMatchObject({ op: "strong", fallback: "slow" });
    expect(fallback.dfnMs).toBeCloseTo(8, 5);
    expect(setup.processor.dfn).toBeNull();
    // From then on the output is Standard's, sample for sample.
    const standard = loadWorklet().processors["gelabber-rnnoise"];
    const reference = run(
      {
        ...loadWorklet(),
        processor: new standard({
          processorOptions: { model: rnnoiseModule, gate: { mode: "off" } },
        }),
      },
      input,
    );
    const tail = 5 * RATE;
    expect(Array.from(output.subarray(tail))).toEqual(
      Array.from(reference.subarray(tail)),
    );
  });

  it("switches to RNNoise when the audio falls behind the clock", () => {
    const setup = strong();
    // Two windows of 5 s on the wall clock after the first second.
    run(setup, voice(10_000), { stretch: 1.2 });
    expect(setup.messages.find((m) => m.fallback)?.fallback).toBe("underrun");
  });

  it("starts as Standard when DFN3 cannot be set up", () => {
    // Memory that cannot hold the model, as on a phone short of memory.
    const setup = strong(
      { mode: "off" },
      { module: dfnModule, memory: new ArrayBuffer(65_536), state: 8 },
    );
    expect(setup.processor.dfn).toBeNull();
    expect(setup.messages[0]).toMatchObject({ ready: true });
    expect(setup.messages[1]).toMatchObject({
      op: "strong",
      fallback: "error",
    });
    const input = voice(300);
    const output = run(setup, input);
    const standard = loadWorklet().processors["gelabber-rnnoise"];
    const reference = run(
      {
        ...loadWorklet(),
        processor: new standard({
          processorOptions: { model: rnnoiseModule, gate: { mode: "off" } },
        }),
      },
      input,
    );
    expect(Array.from(output)).toEqual(Array.from(reference));
  });

  it("switches to RNNoise when DFN3 fails", () => {
    const setup = strong();
    setup.processor.dfn.process = () => false;
    const output = run(setup, voice(100));
    expect(setup.messages.find((m) => m.fallback)?.fallback).toBe("error");
    expect(output.some((value) => value !== 0)).toBe(true);
  });
});
