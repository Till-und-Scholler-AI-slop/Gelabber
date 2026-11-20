// The input sensitivity gate in public/audio/rnnoise-worklet.js (GEL-27),
// run outside a browser the way scripts/audio/benchmark-rnnoise.mjs does.
/* global URL, WebAssembly */
import fs from "node:fs";
import vm from "node:vm";
import { describe, expect, it } from "vitest";

const RATE = 48_000;
const BLOCK = 480;

function load() {
  const processors = {};
  vm.runInNewContext(
    fs.readFileSync(
      new URL("../../public/audio/rnnoise-worklet.js", import.meta.url),
      "utf8",
    ),
    {
      WebAssembly,
      sampleRate: RATE,
      AudioWorkletProcessor: class {
        port = { postMessage() {}, onmessage: null };
      },
      registerProcessor(name, ctor) {
        processors[name] = ctor;
      },
    },
  );
  return processors;
}

const processors = load();

function gate(config, gain = 1) {
  const reports = [];
  const processor = new processors["gelabber-gate"]({
    processorOptions: { gate: config, gain },
  });
  processor.port.postMessage = (data) => {
    if (data.op === "gate") reports.push(data);
  };
  return { processor, reports };
}

/** Runs `input` through in render quanta of 128 and returns the output. */
function run(processor, input) {
  const output = new Float32Array(input.length);
  for (let i = 0; i < input.length; i += 128)
    processor.process(
      [[input.subarray(i, i + 128)]],
      [[output.subarray(i, i + 128)]],
    );
  return output;
}

/** A tone at `db` dBFS RMS for `ms`. */
function tone(db, ms, hz = 220) {
  const amplitude = Math.SQRT2 * 10 ** (db / 20);
  const out = new Float32Array((RATE * ms) / 1000);
  for (let n = 0; n < out.length; n++)
    out[n] = amplitude * Math.sin((2 * Math.PI * hz * n) / RATE);
  return out;
}

/** White noise at about `db` dBFS RMS. */
function noise(db, ms, seed = 7) {
  const amplitude = Math.sqrt(3) * 10 ** (db / 20);
  const out = new Float32Array((RATE * ms) / 1000);
  let state = seed;
  for (let n = 0; n < out.length; n++) {
    state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
    out[n] = amplitude * (2 * (state / 0xffffffff) - 1);
  }
  return out;
}

function join(...parts) {
  const out = new Float32Array(parts.reduce((n, p) => n + p.length, 0));
  let at = 0;
  for (const part of parts) {
    out.set(part, at);
    at += part.length;
  }
  return out;
}

/** RMS in dBFS of `samples` between `fromMs` and `toMs`. */
function levelDb(samples, fromMs, toMs) {
  const from = (RATE * fromMs) / 1000,
    to = (RATE * toMs) / 1000;
  let sum = 0;
  for (let n = from; n < to; n++) sum += samples[n] * samples[n];
  const rms = Math.sqrt(sum / (to - from));
  return rms > 0 ? 20 * Math.log10(rms) : -Infinity;
}

const silent = (samples, fromMs, toMs) =>
  samples
    .subarray((RATE * fromMs) / 1000, (RATE * toMs) / 1000)
    .every((value) => value === 0);

describe("input gate", () => {
  it("passes everything unchanged, 10 ms later, when off", () => {
    // 10 ms reblocking only: off, the gate adds no look-ahead.
    const late = BLOCK;
    const input = join(noise(-60, 200), tone(-20, 200));
    const { processor } = gate({ mode: "off" });
    const output = run(processor, input);
    expect(Array.from(output.subarray(0, late))).toEqual(
      new Array(late).fill(0),
    );
    for (let n = late; n < output.length; n++)
      expect(output[n]).toBe(input[n - late]);
  });

  it("runs 20 ms behind only while it is on", () => {
    const { processor } = gate({ mode: "off" });
    run(processor, tone(-20, 200));
    processor.port.onmessage?.({
      data: { op: "gateConfig", config: { mode: "manual", thresholdDb: -80 } },
    });
    const input = tone(-20, 300, 330);
    const output = run(processor, input);
    // The last 10 ms from before, then the empty look-ahead, then the input.
    expect(silent(output, 10, 30)).toBe(true);
    for (let n = 3 * BLOCK; n < output.length; n++)
      expect(output[n]).toBe(input[n - 3 * BLOCK]);
  });

  it("keeps a 20 ms burst out and sends the start of speech whole", () => {
    const quiet = () => new Float32Array(RATE / 2);
    const input = join(quiet(), tone(-20, 20), quiet(), tone(-20, 300));
    const output = run(
      gate({ mode: "manual", thresholdDb: -40 }).processor,
      input,
    );
    // Speech starts at 1020 ms in, 1050 ms out.
    expect(silent(output, 0, 1050)).toBe(true);
    expect(levelDb(output, 1050, 1060)).toBeGreaterThan(-23);
    expect(levelDb(output, 1060, 1310)).toBeCloseTo(-20, 0);
  });

  it("sends silence below a manual threshold and the voice above it", () => {
    const input = join(tone(-20, 400), noise(-60, 800), tone(-20, 400));
    const output = run(
      gate({ mode: "manual", thresholdDb: -40 }).processor,
      input,
    );
    // 10 ms reblocking: the input at t is the output at t + 10 ms.
    expect(levelDb(output, 60, 400)).toBeCloseTo(-20, 0);
    // Hold 250 ms and a 40 ms fade, then nothing at all.
    expect(levelDb(output, 420, 640)).toBeGreaterThan(-61);
    expect(silent(output, 720, 1200)).toBe(true);
    expect(levelDb(output, 1260, 1600)).toBeCloseTo(-20, 0);
  });

  it("keeps a pause shorter than the hold, and closes after a longer one", () => {
    const input = join(
      tone(-25, 300),
      new Float32Array((RATE * 150) / 1000),
      tone(-25, 300),
      new Float32Array((RATE * 600) / 1000),
    );
    const { processor, reports } = gate({ mode: "manual", thresholdDb: -45 });
    const output = run(processor, input);
    // The 150 ms pause is sent as it is: the gate never closed.
    expect(reports.slice(0, 12).every((report) => report.open)).toBe(true);
    expect(levelDb(output, 470, 760)).toBeCloseTo(-25, 0);
    expect(reports.at(-1)?.open).toBe(false);
  });

  it("measures after the input gain", () => {
    // -50 dBFS at 200 % gain is about -44 dBFS: above a -46 dB threshold.
    const input = tone(-50, 600);
    const below = run(
      gate({ mode: "manual", thresholdDb: -46 }, 1).processor,
      input,
    );
    const above = run(
      gate({ mode: "manual", thresholdDb: -46 }, 2).processor,
      input,
    );
    expect(silent(below, 400, 600)).toBe(true);
    expect(levelDb(above, 100, 600)).toBeCloseTo(-50, 0);
  });

  it("finds the room's noise by itself and opens for a louder voice", () => {
    // A fan at -55 dBFS for 3 s, a voice at -25 dBFS, the fan again.
    const input = join(noise(-55, 3000), tone(-25, 500), noise(-55, 1500, 9));
    const { processor, reports } = gate({ mode: "auto" });
    const output = run(processor, input);
    expect(silent(output, 2000, 3000)).toBe(true);
    expect(levelDb(output, 3050, 3500)).toBeCloseTo(-25, 0);
    expect(silent(output, 3900, 5000)).toBe(true);
    const settled = reports.find((report) => report.levelDb < -50);
    expect(settled.mode).toBe("auto");
    expect(settled.thresholdDb).toBeGreaterThan(-55);
    expect(settled.thresholdDb).toBeLessThanOrEqual(-30);
  });

  it("follows a change of settings and of the gain while it runs", () => {
    const { processor } = gate({ mode: "manual", thresholdDb: -10 });
    const first = run(processor, tone(-30, 500));
    expect(silent(first, 300, 500)).toBe(true);
    processor.port.onmessage?.({
      data: { op: "gateConfig", config: { mode: "off" } },
    });
    const second = run(processor, tone(-30, 300));
    expect(levelDb(second, 20, 300)).toBeCloseTo(-30, 0);
    processor.port.onmessage?.({
      data: { op: "gateConfig", config: { mode: "manual", thresholdDb: -40 } },
    });
    processor.port.onmessage?.({ data: { op: "gain", value: 0.1 } });
    // -30 dBFS at 10 % gain is -50 dB as sent: below -40.
    const third = run(processor, tone(-30, 600));
    expect(silent(third, 400, 600)).toBe(true);
  });

  it("reports the level, the threshold and the state every 80 ms", () => {
    const { processor, reports } = gate({ mode: "manual", thresholdDb: -35 });
    run(processor, tone(-20, 800));
    expect(reports).toHaveLength(10);
    expect(reports[0]).toMatchObject({
      op: "gate",
      mode: "manual",
      open: true,
    });
    expect(reports[0].levelDb).toBeCloseTo(-20, 0);
    expect(reports[0].thresholdDb).toBe(-35);
  });

  it("keeps the threshold slider within -80 and 0 dB", () => {
    const { processor, reports } = gate({ mode: "manual", thresholdDb: 25 });
    run(processor, tone(-20, 80));
    expect(reports[0].thresholdDb).toBe(0);
  });
});

describe("input gate with RNNoise", () => {
  const model = new WebAssembly.Module(
    fs.readFileSync(
      new URL("../../public/audio/rnnoise.wasm", import.meta.url),
    ),
  );

  it("stays closed for key clicks in automatic mode", () => {
    // Short loud clicks every 180 ms over a quiet room: louder than the
    // automatic threshold, but no speech.
    const input = noise(-70, 3000, 3);
    for (let at = 1000; at < 3000; at += 180) {
      const start = (RATE * at) / 1000;
      for (let n = 0; n < 96; n++)
        input[start + n] =
          (input[start + n] ?? 0) + 0.3 * Math.exp(-n / 18) * (n % 2 ? 1 : -1);
    }
    const reports = [];
    const processor = new processors["gelabber-rnnoise"]({
      processorOptions: { model, gate: { mode: "auto" }, gain: 1 },
    });
    processor.port.postMessage = (data) => {
      if (data.op === "gate") reports.push(data);
    };
    const output = run(processor, input);
    expect(levelDb(output, 1500, 3000)).toBe(-Infinity);
    expect(reports.slice(15).every((report) => !report.open)).toBe(true);
  });

  it("passes RNNoise's output unchanged when off", () => {
    const input = noise(-40, 500);
    const off = new processors["gelabber-rnnoise"]({
      processorOptions: { model, gate: { mode: "off" } },
    });
    const none = new processors["gelabber-rnnoise"]({
      processorOptions: { model },
    });
    expect(Array.from(run(off, input))).toEqual(Array.from(run(none, input)));
  });
});
