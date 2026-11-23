// Runs the web client's own RNNoise binary (web/public/audio/rnnoise.wasm,
// hash-checked like the app's loader) over every clip, one 10 ms frame at a
// time, and times each frame. V8 here is the engine Chromium runs the
// AudioWorklet with; a browser adds its own scheduling on top.
// Output: work/out/rnnoise/<id>.wav (no delay compensation) and a JSON
// summary on stdout.
/* global WebAssembly */
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import fs from "node:fs";
import { performance } from "node:perf_hooks";
import { readWav, timingSummary, writeWav } from "./lib.mjs";

const HERE = new URL(".", import.meta.url).pathname,
  CLIPS = `${HERE}work/clips`,
  OUT = `${HERE}work/out/rnnoise`,
  WASM = new URL("../../web/public/audio/rnnoise.wasm", import.meta.url),
  provenance = JSON.parse(
    fs.readFileSync(new URL("../../web/public/audio/RNNOISE-PROVENANCE.json", import.meta.url)),
  );
fs.mkdirSync(OUT, { recursive: true });
const bytes = fs.readFileSync(WASM);
assert.equal(createHash("sha256").update(bytes).digest("hex"), provenance.wasmSha256);
const module = new WebAssembly.Module(bytes);

const manifest = JSON.parse(fs.readFileSync(`${CLIPS}/manifest.json`, "utf8")),
  all = [],
  allCpu = [],
  files = [];
const cpu0 = process.cpuUsage();
for (const clip of manifest) {
  // A fresh instance per clip, like a new call.
  const api = new WebAssembly.Instance(module).exports;
  api._initialize();
  const size = api.rnnoise_get_frame_size();
  assert.equal(size, 480);
  const state = api.rnnoise_create(0),
    input = api.malloc(size * 4),
    output = api.malloc(size * 4),
    heap = new Float32Array(api.memory.buffer);
  const { rate, samples } = readWav(`${CLIPS}/${clip.id}.wav`);
  assert.equal(rate, 48000);
  const out = new Float32Array(samples.length),
    times = [],
    cpuTimes = [];
  for (let n = 0; n + size <= samples.length; n += size) {
    // RNNoise works on 16-bit-scaled floats; the worklet scales the same way.
    for (let j = 0; j < size; j++) heap[input / 4 + j] = samples[n + j] * 32768;
    // Wall time and this thread's CPU time: on a loaded machine only the
    // second is unaffected by other processes taking the core.
    const c = process.threadCpuUsage(),
      t = performance.now();
    api.rnnoise_process_frame(state, output, input);
    times.push(performance.now() - t);
    const d = process.threadCpuUsage(c);
    cpuTimes.push((d.user + d.system) / 1e3);
    for (let j = 0; j < size; j++) out[n + j] = heap[output / 4 + j] / 32768;
  }
  api.rnnoise_destroy(state);
  writeWav(`${OUT}/${clip.id}.wav`, rate, out);
  all.push(...times);
  allCpu.push(...cpuTimes);
  files.push({ file: `${clip.id}.wav`, compute: timingSummary(times) });
}
const cpu = process.cpuUsage(cpu0);
console.log(
  JSON.stringify({
    engine: `RNNoise ${provenance.release} (web/public/audio/rnnoise.wasm, WebAssembly in Node ${process.version})`,
    wasmSha256: provenance.wasmSha256,
    frameSamples: 480,
    compute: timingSummary(all),
    // Node reads thread CPU time in scheduler ticks (4 ms here), so single
    // frames read 0 or 4 ms; only the mean over all frames is meaningful.
    threadCpuMeanMs: allCpu.reduce((a, b) => a + b, 0) / allCpu.length,
    processCpuSeconds: +((cpu.user + cpu.system) / 1e6).toFixed(2),
    files,
  }),
);
