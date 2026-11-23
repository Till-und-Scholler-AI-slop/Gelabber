// Scores the unprocessed clips and every engine's output, measures each
// engine's input-to-output delay, and writes results/scores.json and
// results/summary.md.
import fs from "node:fs";
import { createScorer } from "./dnsmos.mjs";
import { bestLag, readWav, siSdr } from "./lib.mjs";

const HERE = new URL(".", import.meta.url).pathname,
  CLIPS = `${HERE}work/clips`,
  RESULTS = `${HERE}results`,
  RATE = 48000,
  ENGINES = {
    noisy: { label: "Unprocessed", dir: CLIPS },
    rnnoise: { label: "RNNoise (app wasm)", dir: `${HERE}work/out/rnnoise` },
    dfn3: { label: "DeepFilterNet3", dir: `${HERE}work/out/dfn3` },
    "dfn3-lib": { label: "DFN3, libDF default thresholds", dir: `${HERE}work/out/dfn3-lib` },
    // After the app's default input gate (gate.mjs).
    "rnnoise-gate": { label: "RNNoise + gate", dir: `${HERE}work/out/rnnoise-gate` },
    "dfn3-gate": { label: "DFN3 + gate", dir: `${HERE}work/out/dfn3-gate` },
  };
fs.mkdirSync(RESULTS, { recursive: true });
const manifest = JSON.parse(fs.readFileSync(`${CLIPS}/manifest.json`, "utf8"));
const load = (engine, id) => readWav(`${ENGINES[engine].dir}/${id}.wav`).samples;

// Delay: cross-correlate input and output of the clean clips (speech only),
// 1 s to 9 s, lags 0 to 60 ms. Noise would be suppressed and cannot show it.
const delay = {};
for (const engine of ["rnnoise", "dfn3", "dfn3-lib", "rnnoise-gate", "dfn3-gate"]) {
  delay[engine] = manifest
    .filter((c) => c.condition === "clean")
    .map((c) => {
      const { lag, corr } = bestLag(load("noisy", c.id), load(engine, c.id), RATE, 8 * RATE, 2880);
      return { clip: c.id, samples: lag, ms: (lag / RATE) * 1e3, correlation: +corr.toFixed(4) };
    });
}
const lagOf = (engine) =>
  engine === "noisy"
    ? 0
    : Math.round(delay[engine].reduce((a, d) => a + d.samples, 0) / delay[engine].length);

// Native vs WebAssembly builds of the same source and model: only the
// scored build's output is rated, so show how far the other one is from it.
function maxDiff(engine, otherDir) {
  if (!fs.existsSync(`${HERE}work/out/${otherDir}`)) return null;
  let diff = 0;
  for (const c of manifest) {
    const a = load(engine, c.id),
      b = readWav(`${HERE}work/out/${otherDir}/${c.id}.wav`).samples;
    for (let n = 0; n < a.length; n++) diff = Math.max(diff, Math.abs(a[n] - b[n]));
  }
  return diff;
}
const buildDiffs = {
  rnnoiseWasmVsNativeMaxAbsDiff: maxDiff("rnnoise", "rnnoise-native"),
  dfn3NativeVsWasmMaxAbsDiff: maxDiff("dfn3", "dfn3-wasm"),
};

const score = await createScorer(),
  rows = [];
for (const c of manifest) {
  const ref = readWav(`${CLIPS}/${c.id}.ref.wav`).samples,
    row = { ...c, engines: {} };
  for (const engine of Object.keys(ENGINES)) {
    const out = load(engine, c.id),
      lag = lagOf(engine),
      entry = await score(out);
    // SI-SDR against the dry talker, output shifted back by the engine's
    // delay; 0.5 s settle time skipped. Not for reverb (the dry talker is
    // not what a denoiser should produce) or clean.
    if (c.condition !== "reverb" && c.condition !== "clean") {
      const from = RATE / 2,
        to = ref.length - lag - RATE / 10;
      entry.siSdr = siSdr(ref.subarray(from, to), out.subarray(from + lag, to + lag));
    }
    row.engines[engine] = entry;
  }
  rows.push(row);
  process.stderr.write(".");
}
process.stderr.write("\n");

const round = (v, d = 2) => (v == null || Number.isNaN(v) ? null : +v.toFixed(d));
fs.writeFileSync(
  `${RESULTS}/scores.json`,
  JSON.stringify(
    {
      delay,
      compensatedLagSamples: Object.fromEntries(Object.keys(delay).map((e) => [e, lagOf(e)])),
      ...buildDiffs,
      clips: rows.map((r) => ({
        ...r,
        engines: Object.fromEntries(
          Object.entries(r.engines).map(([k, v]) => [
            k,
            {
              dnsmos: Object.fromEntries(
                Object.entries(v.dnsmos).map(([m, s]) => [m, m === "windows" ? s : round(s, 3)]),
              ),
              pdnsmos: Object.fromEntries(
                Object.entries(v.pdnsmos).map(([m, s]) => [m, m === "windows" ? s : round(s, 3)]),
              ),
              siSdr: round(v.siSdr),
            },
          ]),
        ),
      })),
    },
    null,
    2,
  ) + "\n",
);

// Means over the three talkers per condition and SNR.
const groups = new Map();
for (const r of rows) {
  const key = r.snrDb == null ? r.condition : `${r.condition} ${r.snrDb} dB`;
  if (!groups.has(key)) groups.set(key, []);
  groups.get(key).push(r);
}
const mean = (list, f) => {
  const v = list.map(f).filter((x) => x != null);
  return v.length ? v.reduce((a, b) => a + b, 0) / v.length : null;
};
const fmt = (v, d = 2) => (v == null ? "–" : v.toFixed(d));
const engines = Object.keys(ENGINES),
  // DFN3 − RNNoise, without and with the gate.
  diffs = [
    ["dfn3", "rnnoise"],
    ["dfn3-gate", "rnnoise-gate"],
  ].map(([a, b]) => [engines.indexOf(a), engines.indexOf(b)]);
const diffCells = (vals, digits, wrap = (x) => x) =>
  diffs
    .map(([a, b]) => wrap(fmt(vals[a] != null && vals[b] != null ? vals[a] - vals[b] : null, digits)))
    .join(" | ");
function table(title, metric, pick, digits = 2) {
  const lines = [
    `### ${title}`,
    "",
    `| Condition | ${engines.map((e) => ENGINES[e].label).join(" | ")} | DFN3 − RNNoise | DFN3 − RNNoise, gated |`,
    `|---|${engines.map(() => "---:").join("|")}|---:|---:|`,
  ];
  const noisyRows = [];
  for (const [key, list] of groups) {
    const vals = engines.map((e) => mean(list, (r) => pick(r.engines[e])));
    if (vals.every((v) => v == null)) continue;
    if (key !== "clean" && key !== "reverb") noisyRows.push(...list);
    lines.push(`| ${key} | ${vals.map((v) => fmt(v, digits)).join(" | ")} | ${diffCells(vals, digits)} |`);
  }
  const all = engines.map((e) => mean(noisyRows, (r) => pick(r.engines[e])));
  lines.push(
    `| **all noise conditions** | ${all.map((v) => `**${fmt(v, digits)}**`).join(" | ")} | ${diffCells(
      all,
      digits,
      (x) => `**${x}**`,
    )} |`,
  );
  return [...lines, "", metric, ""].join("\n");
}

// CPU per 10 ms block from the engine runs (run.sh).
const cpuRows = [
  ["cpu-rnnoise-wasm.json", "RNNoise, app wasm (V8)"],
  ["cpu-rnnoise-native.json", "RNNoise, native (desktop build)"],
  ["cpu-dfn3-wasm.json", "DeepFilterNet3, wasm32 +simd128 (V8)"],
  ["cpu-dfn3.json", "DeepFilterNet3, native"],
  ["cpu-dfn3-lib.json", "DeepFilterNet3, native, libDF default thresholds"],
]
  .filter(([f]) => fs.existsSync(`${RESULTS}/${f}`))
  .map(([f, label]) => {
    const j = JSON.parse(fs.readFileSync(`${RESULTS}/${f}`, "utf8")),
      cpuMean = j.threadCpu?.meanMs ?? j.threadCpuMeanMs;
    return `| ${label} | ${j.compute.medianMs.toFixed(3)} | ${j.compute.p99Ms.toFixed(3)} | ${
      j.threadCpu ? j.threadCpu.p99Ms.toFixed(3) : "–"
    } | ${cpuMean.toFixed(3)} | ${(cpuMean * 10).toFixed(1)} % |`;
  });
const cpuTable = [
  "### CPU per 10 ms block (480 samples at 48 kHz), one thread",
  "",
  "| Engine | Wall median (ms) | Wall p99 (ms) | Thread CPU p99 (ms) | Thread CPU mean (ms) | Share of one core |",
  "|---|---:|---:|---:|---:|---:|",
  ...cpuRows,
  "",
  "Wall time includes time the thread was preempted by other processes. Thread CPU p99 is only available natively; Node reads thread CPU in scheduler ticks, so only its mean over all blocks is given.",
  "",
].join("\n");

// Share of 10 ms blocks the gate silenced completely (gate.mjs).
const gateLines = fs.existsSync(`${RESULTS}/gate.json`)
  ? [
      "### Input gate (default automatic mode, after the filter)",
      "",
      ...Object.entries(JSON.parse(fs.readFileSync(`${RESULTS}/gate.json`, "utf8"))).map(
        ([e, g]) => `- ${ENGINES[e].label}: ${g.silenced} of ${g.blocks} blocks silenced (${(g.share * 100).toFixed(1)} %)`,
      ),
      "",
      "The gate holds 250 ms after speech, so it silences only the longer pauses. The DFN3 column uses RNNoise's speech probability for the same input, which DFN3 does not produce.",
      "",
    ]
  : [];

const md = [
  "<!-- Generated by score.mjs; do not edit by hand. -->",
  "",
  cpuTable,
  "### Input-to-output delay (cross-correlation on the clean clips)",
  "",
  "| Engine | Delay per clip (samples at 48 kHz) | Correlation |",
  "|---|---|---|",
  ...Object.entries(delay).map(
    ([e, list]) =>
      `| ${ENGINES[e].label} | ${list.map((d) => `${d.samples} (${d.ms.toFixed(1)} ms)`).join(", ")} | ${list
        .map((d) => d.correlation)
        .join(", ")} |`,
  ),
  "",
  "The gated rows include the gate's 20 ms look-ahead. All rows exclude the web worklet's 10 ms reblocking and all capture, codec, network and playout buffering.",
  "",
  ...gateLines,
  "Quality tables: means over the three talkers (m1, f1, m2).",
  "",
  table("DNSMOS P.835 OVRL (overall)", "1 to 5, higher is better.", (e) => e.dnsmos.OVRL),
  table("DNSMOS P.835 SIG (speech distortion)", "1 to 5, higher is better.", (e) => e.dnsmos.SIG),
  table("DNSMOS P.835 BAK (background intrusiveness)", "1 to 5, higher is better.", (e) => e.dnsmos.BAK),
  table(
    "Personalized DNSMOS OVRL",
    "1 to 5, higher is better. This model also rates a second talker as noise.",
    (e) => e.pdnsmos.OVRL,
  ),
  table(
    "Personalized DNSMOS BAK",
    "1 to 5, higher is better.",
    (e) => e.pdnsmos.BAK,
  ),
  table(
    "SI-SDR against the dry talker (dB)",
    "Higher is better. Measures how much of the output is the target talker; noise and second voices both count against it.",
    (e) => e.siSdr ?? null,
    1,
  ),
].join("\n");
fs.writeFileSync(`${RESULTS}/summary.md`, md);
console.log(md);
