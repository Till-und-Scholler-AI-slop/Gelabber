// Measured compute/reblocking acceptance; synthetic input, no hardware-quality claim.
/* global console, WebAssembly, performance, URL */
import assert from "node:assert/strict";
import fs from "node:fs";
import vm from "node:vm";
import { createHash } from "node:crypto";
const bytes = fs.readFileSync(
  new URL("../../public/audio/rnnoise.wasm", import.meta.url),
);
assert.equal(
  createHash("sha256").update(bytes).digest("hex"),
  "e66d0eaef35d3774e86377efa8b9897e5b226284fb53059f0c1444881888b71c",
);
const model = new WebAssembly.Module(bytes),
  api = new WebAssembly.Instance(model).exports;
api._initialize();
const size = api.rnnoise_get_frame_size();
assert.equal(size, 480);
const state = api.rnnoise_create(0),
  input = api.malloc(size * 4),
  output = api.malloc(size * 4),
  heap = new Float32Array(api.memory.buffer);
const direct = [],
  samples = new Float32Array(480 * 600),
  times = [];
let seed = 42;
for (let n = 0; n < samples.length; n++) {
  seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0;
  samples[n] = (seed / 0xffffffff - 0.5) * 0.08;
}
for (let n = 0; n < 600; n++) {
  for (let j = 0; j < size; j++)
    heap[input / 4 + j] = samples[n * size + j] * 32768;
  const start = performance.now();
  api.rnnoise_process_frame(state, output, input);
  if (n >= 100) times.push(performance.now() - start);
  for (let j = 0; j < size; j++) direct.push(heap[output / 4 + j] / 32768);
}
let Processor;
vm.runInNewContext(
  fs.readFileSync(
    new URL("../../public/audio/rnnoise-worklet.js", import.meta.url),
    "utf8",
  ),
  {
    WebAssembly,
    sampleRate: 48000,
    AudioWorkletProcessor: class {
      port = { postMessage() {}, onmessage: null };
    },
    registerProcessor(name, ctor) {
      Processor = ctor;
    },
  },
);
const processor = new Processor({ processorOptions: { model } }),
  actual = [];
let index = 0,
  quantum = 0;
const quanta = [128, 64, 256, 127, 1, 96];
while (index < samples.length) {
  const count = Math.min(
      quanta[quantum++ % quanta.length],
      samples.length - index,
    ),
    block = new Float32Array(count);
  processor.process([[samples.subarray(index, index + count)]], [[block]]);
  actual.push(...block);
  index += count;
}
assert.equal(
  actual.slice(0, 480).every((value) => value === 0),
  true,
);
for (let n = 480; n < actual.length; n++)
  assert.equal(actual[n], direct[n - 480]);
const inputEnergy = samples
    .subarray(48000)
    .reduce((sum, value) => sum + value * value, 0),
  outputEnergy = actual
    .slice(48480)
    .reduce((sum, value) => sum + value * value, 0);
assert.ok(outputEnergy > 0 && outputEnergy < inputEnergy);
times.sort((a, b) => a - b);
console.log(
  JSON.stringify(
    {
      rnnoiseVersion: "0.2",
      frames: 600,
      frameMs: 10,
      additionalReblockingSamples: 480,
      additionalReblockingMs: 10,
      arbitraryRenderQuanta: quanta,
      medianComputeMs: times[250],
      p99ComputeMs: times[495],
      maxComputeMs: times.at(-1),
      syntheticNoiseAttenuationDb: 10 * Math.log10(inputEnergy / outputEnergy),
      memoryBytes: api.memory.buffer.byteLength,
      physicalDevice: false,
    },
    null,
    2,
  ),
);
api.rnnoise_destroy(state);
api.free(input);
api.free(output);
processor.port.onmessage({ data: { op: "dispose" } });
