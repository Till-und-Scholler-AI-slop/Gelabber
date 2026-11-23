// Builds the fixed GEL-28 clip set from the files fetch.sh pinned. Every step
// is deterministic (fixed offsets, seeded noise), so the clips are the same on
// every machine: work/clips/<id>.wav (48 kHz float, what the filters get),
// work/clips/<id>.ref.wav (the dry target speech in that mix) and
// work/clips/manifest.json.
import fs from "node:fs";
import {
  activeRms,
  biquad,
  convolve,
  dbfs,
  lcg,
  readWav,
  rms,
  scale,
  upsample16to48,
  writeWav,
} from "./lib.mjs";

const HERE = new URL(".", import.meta.url).pathname,
  WORK = `${HERE}work`,
  OUT = `${WORK}/clips`,
  RATE = 48000;
fs.mkdirSync(OUT, { recursive: true });

const load16 = (name) => {
  const { rate, samples } = readWav(`${WORK}/snsd/${name}.wav`);
  if (rate !== 16000) throw new Error(`${name}: expected 16 kHz`);
  return upsample16to48(samples);
};

// Repeats or cuts `x` to `length`, starting at `offset` seconds.
const segment = (x, offsetS, length) => {
  const out = new Float32Array(length),
    start = Math.round(offsetS * RATE);
  for (let n = 0; n < length; n++) out[n] = x[(start + n) % x.length];
  return out;
};

const add = (...xs) => {
  const out = new Float32Array(xs[0].length);
  for (const x of xs) for (let n = 0; n < out.length; n++) out[n] += x[n];
  return out;
};

// Speech at -26 dBFS active level, the usual level for speech-quality tests.
const SPEECH_LEVEL = 10 ** (-26 / 20);
// Every real microphone has a full-band self-noise floor; the 16 kHz sources
// have none above 8 kHz, which neither filter ever sees in practice.
const FLOOR_DBFS = -75;

// Target talkers and the second voice for each, picked from MS-SNSD
// clean_test by estimated pitch so the second voice is the other sex.
const TARGETS = [
  { id: "m1", file: "clnsp0", second: "clnsp13", typingAt: 20, fanAt: 0 },
  { id: "f1", file: "clnsp2", second: "clnsp8", typingAt: 60, fanAt: 5 },
  { id: "m2", file: "clnsp6", second: "clnsp13", typingAt: 100, fanAt: 10 },
];
const TV_SOURCE = "clnsp12";
const SNRS = [0, 10];

const rir = (() => {
  const { rate, samples } = readWav(
    `${WORK}/src/deepfilternet/assets/rir_sim_1001_w11.7_l2.6_h2.5_rt60_0.7919.wav`,
  );
  if (rate !== RATE) throw new Error("RIR: expected 48 kHz");
  return samples;
})();

// Mouse clicks: no openly licensed recording was obtainable here (see the
// doc), so this is a synthetic stand-in: press and release transients, 3 ms
// decaying noise bursts high-passed at 1.5 kHz, every 0.25 to 1.25 s.
function mouseClicks(length, seed) {
  const rnd = lcg(seed),
    out = new Float32Array(length);
  let t = Math.round(0.2 * RATE);
  while (t < length) {
    for (const at of [t, t + Math.round((0.06 + 0.06 * rnd()) * RATE)]) {
      const amp = 0.6 + 0.4 * rnd();
      for (let i = 0; i < 0.003 * RATE && at + i < length; i++)
        out[at + i] += amp * (rnd() * 2 - 1) * Math.exp(-i / (0.0008 * RATE));
    }
    t += Math.round((0.25 + rnd()) * RATE);
  }
  return biquad(out, "highpass", 1500, RATE);
}

// TV across the room: other speech through a small loudspeaker (200 Hz to
// 5 kHz) and the simulated room.
function tv(length) {
  let x = segment(load16(TV_SOURCE), 0, length);
  x = biquad(biquad(x, "highpass", 200, RATE), "lowpass", 5000, RATE);
  return convolve(x, rir);
}

const manifest = [];
function emit(id, target, condition, snrDb, mix, ref, seed) {
  const floor = new Float32Array(mix.length),
    rnd = lcg(seed),
    floorGain = 10 ** (FLOOR_DBFS / 20) * Math.sqrt(3);
  for (let n = 0; n < floor.length; n++) floor[n] = (rnd() * 2 - 1) * floorGain;
  let out = add(mix, floor);
  let peak = 0;
  for (const v of out) peak = Math.max(peak, Math.abs(v));
  // Keep headroom; scale mix and reference together so SI-SDR is unaffected.
  const gain = peak > 0.89 ? 0.89 / peak : 1;
  out = scale(out, gain);
  writeWav(`${OUT}/${id}.wav`, RATE, out);
  writeWav(`${OUT}/${id}.ref.wav`, RATE, scale(ref, gain));
  manifest.push({
    id,
    target,
    condition,
    snrDb,
    seconds: +(out.length / RATE).toFixed(2),
    peakGainDb: +dbfs(gain).toFixed(2),
  });
}

TARGETS.forEach((t, ti) => {
  let speech = load16(t.file);
  speech = scale(speech, SPEECH_LEVEL / activeRms(speech, RATE));
  const len = speech.length,
    noises = {
      keyboard: segment(load16("Typing_1"), t.typingAt, len),
      mouse: mouseClicks(len, 1000 + ti),
      fan: segment(load16("AirConditioner_1"), t.fanAt, len),
      tv: tv(len),
      "second-voice": segment(load16(t.second), 0, len),
    };
  emit(`${t.id}-clean`, t.id, "clean", null, speech, speech, 1 + ti);
  for (const [name, noise] of Object.entries(noises)) {
    // Speech-like interferers are levelled like speech, the rest by RMS.
    const level =
      name === "tv" || name === "second-voice" ? activeRms(noise, RATE) : rms(noise);
    for (const snr of SNRS) {
      const n = scale(noise, (SPEECH_LEVEL * 10 ** (-snr / 20)) / level);
      emit(`${t.id}-${name}-${snr}db`, t.id, name, snr, add(speech, n), speech, 100 * ti + snr + 7);
    }
  }
  // Reverberant room: the talker through the simulated room (RT60 0.79 s),
  // no added noise. The dry speech stays the reference.
  let wet = convolve(speech, rir);
  wet = scale(wet, SPEECH_LEVEL / activeRms(wet, RATE));
  emit(`${t.id}-reverb`, t.id, "reverb", null, wet, speech, 50 + ti);
});

fs.writeFileSync(`${OUT}/manifest.json`, JSON.stringify(manifest, null, 2) + "\n");
console.log(`${manifest.length} clips in ${OUT}`);
