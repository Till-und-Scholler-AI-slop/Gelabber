// Measured compute/reblocking and PCM delay acceptance; no hardware-quality claim.
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

// Noise is heavily suppressed, so it cannot identify RNNoise's algorithmic delay.
// Use independent voiced harmonics plus an aperiodic chirp, with continuous phase.
// This measures input PCM -> actual worklet output PCM, including model buffering.
const voiced = new Float32Array(samples.length);
let phase = 0,
  chirp = 0;
seed = 42;
for (let n = 0; n < voiced.length; n++) {
  seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0;
  const t = n / 48000;
  phase +=
    (2 * Math.PI * (180 + 70 * Math.sin(t * 1.19) + 35 * Math.sin(t * 3.71))) /
    48000;
  chirp += (2 * Math.PI * (450 + 300 * t + 60 * Math.sin(t * 2.11))) / 48000;
  const envelope = 0.65 + 0.2 * Math.sin(t * 4.17) + 0.1 * Math.sin(t * 7.13);
  voiced[n] =
    envelope *
      (0.24 * Math.sin(phase) +
        0.08 * Math.sin(2 * phase) +
        0.04 * Math.sin(3 * phase) +
        0.06 * Math.sin(chirp)) +
    (seed / 0xffffffff - 0.5) * 0.002;
}
const voiceApi = new WebAssembly.Instance(model).exports;
voiceApi._initialize();
const voiceState = voiceApi.rnnoise_create(0),
  voiceInput = voiceApi.malloc(size * 4),
  voiceOutput = voiceApi.malloc(size * 4),
  voiceHeap = new Float32Array(voiceApi.memory.buffer),
  voiceDirect = new Float32Array(voiced.length),
  voiceActual = new Float32Array(voiced.length),
  voiceProcessor = new Processor({ processorOptions: { model } });
for (let n = 0; n < voiced.length; n += size) {
  for (let j = 0; j < size; j++)
    voiceHeap[voiceInput / 4 + j] = voiced[n + j] * 32768;
  voiceApi.rnnoise_process_frame(voiceState, voiceOutput, voiceInput);
  for (let j = 0; j < size; j++)
    voiceDirect[n + j] = voiceHeap[voiceOutput / 4 + j] / 32768;
}
index = 0;
quantum = 0;
while (index < voiced.length) {
  const count = Math.min(
    quanta[quantum++ % quanta.length],
    voiced.length - index,
  );
  voiceProcessor.process(
    [[voiced.subarray(index, index + count)]],
    [[voiceActual.subarray(index, index + count)]],
  );
  index += count;
}
for (let n = 480; n < voiceActual.length; n++)
  assert.equal(voiceActual[n], voiceDirect[n - 480]);
function sampleDelay(reference, processed, start, length) {
  const mean =
    reference.subarray(start, start + length).reduce((a, b) => a + b, 0) /
    length;
  let energy = 0;
  for (let n = start; n < start + length; n++)
    energy += (reference[n] - mean) ** 2;
  const candidates = [];
  for (let lag = 0; lag <= 2400; lag++) {
    let dot = 0,
      sum = 0,
      outEnergy = 0;
    for (let n = start; n < start + length; n++) {
      const y = processed[n + lag];
      dot += (reference[n] - mean) * y;
      sum += y;
      outEnergy += y * y;
    }
    candidates.push({
      samples: lag,
      correlation: dot / Math.sqrt(energy * (outEnergy - (sum * sum) / length)),
    });
  }
  candidates.sort((a, b) => b.correlation - a.correlation);
  const peak = candidates[0],
    separated = candidates.find(
      (candidate) => Math.abs(candidate.samples - peak.samples) > 48,
    );
  assert.ok(
    peak.correlation >= 0.95,
    "Delay probe must preserve an identifiable signal",
  );
  assert.ok(
    peak.correlation - separated.correlation >= 0.02,
    "Delay peak must be distinct from periodic aliases outside 1 ms",
  );
  return { ...peak, bestSeparatedCorrelation: separated.correlation };
}
const sampleDelayWindows = [48000, 120000, 216000].map((start) => {
  const length = 24000,
    directDelay = sampleDelay(voiced, voiceDirect, start, length),
    workletDelay = sampleDelay(voiced, voiceActual, start, length);
  assert.equal(workletDelay.samples - directDelay.samples, 480);
  assert.ok(
    workletDelay.samples > 0 && workletDelay.samples <= 1440,
    "Synthetic PCM model + worklet delay must stay within 30 ms",
  );
  return {
    startSample: start,
    windowSamples: length,
    directRnnoiseDelaySamples: directDelay.samples,
    workletInputOutputDelaySamples: workletDelay.samples,
    workletInputOutputDelayMs: workletDelay.samples / 48,
    peakCorrelation: workletDelay.correlation,
    bestSeparatedCorrelation: workletDelay.bestSeparatedCorrelation,
  };
});
voiceApi.rnnoise_destroy(voiceState);
voiceApi.free(voiceInput);
voiceApi.free(voiceOutput);
voiceProcessor.port.onmessage({ data: { op: "dispose" } });
times.sort((a, b) => a - b);
console.log(
  JSON.stringify(
    {
      rnnoiseVersion: "0.2",
      frames: 600,
      frameMs: 10,
      additionalReblockingSamples: 480,
      additionalReblockingMs: 10,
      pcmDelayProbe: {
        signal: "deterministic aperiodic voiced harmonics and chirp",
        sampleRate: 48000,
        searchRangeMs: [0, 50],
        periodicAliasExclusionMs: 1,
        windows: sampleDelayWindows,
        includesRnnoiseAlgorithmicDelay: true,
        includesCaptureCodecNetworkPlayback: false,
      },
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
