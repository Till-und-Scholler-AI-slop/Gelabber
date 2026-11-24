// Times DeepFilterNet3 built for wasm32 (dfn-wasm/, +simd128) in V8, one
// 10 ms hop per call, over every clip: the cost the web client would pay in
// an AudioWorklet before any browser scheduling. Output: work/out/dfn3-wasm
// and a JSON summary on stdout.
//
// Usage: node dfn-wasm.mjs <dfn_wasm.wasm> <DeepFilterNet3_onnx.tar.gz>
/* global WebAssembly */
import fs from "node:fs";
import { performance } from "node:perf_hooks";
import { readWav, timingSummary, writeWav } from "./lib.mjs";

const HERE = new URL(".", import.meta.url).pathname,
  CLIPS = `${HERE}work/clips`,
  OUT = `${HERE}work/out/dfn3-wasm`,
  [wasmPath, modelPath] = process.argv.slice(2);
fs.mkdirSync(OUT, { recursive: true });
const module = new WebAssembly.Module(fs.readFileSync(wasmPath)),
  model = fs.readFileSync(modelPath),
  manifest = JSON.parse(fs.readFileSync(`${CLIPS}/manifest.json`, "utf8")),
  all = [],
  allCpu = [],
  files = [];
for (const clip of manifest) {
  // A fresh instance per clip, like a new call.
  const api = new WebAssembly.Instance(module, {}).exports,
    t0 = performance.now(),
    ptr = api.df_alloc(model.length);
  new Uint8Array(api.memory.buffer, ptr, model.length).set(model);
  // Same thresholds as the native "dfn3" run (deep-filter command line).
  const state = api.df_create(ptr, model.length, 100, -15, 35, 35);
  if (!state) throw new Error("df_create failed");
  const initMs = performance.now() - t0,
    hop = api.df_hop_size(state),
    { samples } = readWav(`${CLIPS}/${clip.id}.wav`),
    out = new Float32Array(samples.length),
    times = [];
  for (let n = 0; n + hop <= samples.length; n += hop) {
    // Views are re-created per hop: memory can grow and detach old buffers.
    new Float32Array(api.memory.buffer, api.df_input(state), hop).set(samples.subarray(n, n + hop));
    const c = process.threadCpuUsage(),
      t = performance.now();
    const lsnr = api.df_process(state);
    times.push(performance.now() - t);
    const d = process.threadCpuUsage(c);
    allCpu.push((d.user + d.system) / 1e3);
    if (Number.isNaN(lsnr)) throw new Error(`${clip.id}: df_process failed`);
    out.set(new Float32Array(api.memory.buffer, api.df_output(state), hop), n);
  }
  writeWav(`${OUT}/${clip.id}.wav`, 48000, out);
  all.push(...times);
  files.push({ file: `${clip.id}.wav`, initMs, compute: timingSummary(times) });
}
console.log(
  JSON.stringify({
    engine: `DeepFilterNet3 (libDF + tract, wasm32 +simd128 in Node ${process.version})`,
    compute: timingSummary(all),
    // Thread CPU time comes in scheduler ticks; only the mean is meaningful.
    threadCpuMeanMs: allCpu.reduce((a, b) => a + b, 0) / allCpu.length,
    files,
  }),
);
