// Runs the app's input gate (GEL-27, web/public/audio/rnnoise-worklet.js) in
// its default automatic mode after both filters, so the scores also cover
// what a listener hears in "Enhanced voice", where the gate silences
// pauses.
//
// The auto gate opens on RNNoise's speech probability. DFN3 gives none, so
// "dfn3-gate" uses RNNoise's probabilities for the same input, the way a
// Strong level would have to if it kept RNNoise for the gate.
// Output: work/out/rnnoise-gate and work/out/dfn3-gate (no delay
// compensation; the gate adds its look-ahead), and on stdout the share of
// blocks the gate silenced.
/* global WebAssembly */
import assert from "node:assert/strict";
import fs from "node:fs";
import vm from "node:vm";
import { readWav, writeWav } from "./lib.mjs";

const HERE = new URL(".", import.meta.url).pathname,
  CLIPS = `${HERE}work/clips`,
  OUT = `${HERE}work/out`,
  RATE = 48000,
  // DFN3's output is 10 ms behind RNNoise's (measured, see score.mjs), so its
  // block k lines up with RNNoise's block k - 1.
  DFN3_BLOCKS_BEHIND_RNNOISE = 1;

// The gate class itself, unchanged, from the worklet file.
let GateProcessor;
vm.runInNewContext(
  fs.readFileSync(new URL("../../web/public/audio/rnnoise-worklet.js", import.meta.url), "utf8"),
  {
    WebAssembly,
    sampleRate: RATE,
    AudioWorkletProcessor: class {
      port = { postMessage() {}, onmessage: null };
    },
    registerProcessor(name, ctor) {
      if (name === "gelabber-gate") GateProcessor = ctor;
    },
  },
);
const InputGate = new GateProcessor({}).gate.constructor;

const rnnoise = new WebAssembly.Module(
  fs.readFileSync(new URL("../../web/public/audio/rnnoise.wasm", import.meta.url)),
);
/** RNNoise's speech probability per 10 ms block of `samples`, as the
 * worklet gets it from `rnnoise_process_frame`. */
function speechProbabilities(samples) {
  const api = new WebAssembly.Instance(rnnoise).exports;
  api._initialize();
  const size = api.rnnoise_get_frame_size(),
    state = api.rnnoise_create(0),
    input = api.malloc(size * 4),
    output = api.malloc(size * 4),
    heap = new Float32Array(api.memory.buffer),
    speech = [];
  for (let n = 0; n + size <= samples.length; n += size) {
    for (let j = 0; j < size; j++) heap[input / 4 + j] = samples[n + j] * 32768;
    speech.push(api.rnnoise_process_frame(state, output, input));
  }
  api.rnnoise_destroy(state);
  return speech;
}

const closed = { "rnnoise-gate": [0, 0], "dfn3-gate": [0, 0] };
function gated(engine, samples, speechAt) {
  const gate = new InputGate(RATE);
  gate.configure({ mode: "auto" });
  const size = gate.blockFrames,
    frame = new Float32Array(size),
    out = new Float32Array(samples.length);
  for (let k = 0; (k + 1) * size <= samples.length; k++) {
    frame.set(samples.subarray(k * size, (k + 1) * size));
    gate.block(frame, speechAt(k));
    out.set(frame, k * size);
    // Fully silenced: closed and faded out for the whole block.
    if (gate.amount === 0 && frame.every((v) => v === 0)) closed[engine][0]++;
    closed[engine][1]++;
  }
  return out;
}

const manifest = JSON.parse(fs.readFileSync(`${CLIPS}/manifest.json`, "utf8"));
for (const dir of ["rnnoise-gate", "dfn3-gate"]) fs.mkdirSync(`${OUT}/${dir}`, { recursive: true });
for (const clip of manifest) {
  const { rate, samples } = readWav(`${CLIPS}/${clip.id}.wav`);
  assert.equal(rate, RATE);
  const speech = speechProbabilities(samples);
  writeWav(
    `${OUT}/rnnoise-gate/${clip.id}.wav`,
    RATE,
    gated("rnnoise-gate", readWav(`${OUT}/rnnoise/${clip.id}.wav`).samples, (k) => speech[k]),
  );
  writeWav(
    `${OUT}/dfn3-gate/${clip.id}.wav`,
    RATE,
    gated("dfn3-gate", readWav(`${OUT}/dfn3/${clip.id}.wav`).samples, (k) =>
      k >= DFN3_BLOCKS_BEHIND_RNNOISE ? speech[k - DFN3_BLOCKS_BEHIND_RNNOISE] : 0,
    ),
  );
}
console.log(
  JSON.stringify(
    Object.fromEntries(
      Object.entries(closed).map(([e, [silenced, blocks]]) => [e, { silenced, blocks, share: +(silenced / blocks).toFixed(4) }]),
    ),
  ),
);
