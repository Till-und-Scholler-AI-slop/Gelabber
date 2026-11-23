/* global AudioWorkletProcessor, sampleRate, currentFrame, registerProcessor, WebAssembly */
// RNNoise v0.2, 48 kHz mono. No allocation/network/memory growth in process().
// AudioWorklet quantum size is independent of the model's 480-sample frames.
// "Enhanced voice: Strong" (GEL-28, at the end) adds DeepFilterNet3, whose
// tract runtime may still allocate and grow its own memory while it runs.
//
// All processors here end in the input sensitivity gate (GEL-27). The
// desktop app runs the same gate in desktop/native/src/capture_dsp.cc: keep
// the numbers and the steps in `InputGate.block` the same in both.

/** Gate timing and levels. All levels are dB of the 10 ms block's RMS
 * relative to full scale, measured after the input gain.
 *
 * The gate opens only after `openBlocks` blocks in a row speak (or are
 * above the threshold): a key click lasts one or two. So that the first of
 * those blocks is not lost, the output runs `lookaheadBlocks` behind. */
const GATE = {
  blockMs: 10,
  openBlocks: 3,
  lookaheadBlocks: 2,
  holdMs: 250,
  hysteresisDb: 3,
  attackMs: 5,
  releaseMs: 40,
  silenceDb: -100,
  floorMinDb: -90,
  floorMaxDb: -30,
  floorRiseDbPerS: 1.5,
  // Automatic, from the level alone ("Browser filters").
  levelMarginDb: 14,
  levelMinDb: -60,
  levelMaxDb: -30,
  // Automatic with RNNoise's speech probability ("Enhanced voice").
  speechMarginDb: 6,
  speechMinDb: -70,
  speechMaxDb: -40,
  speechOpen: 0.6,
  manualMinDb: -80,
  manualMaxDb: 0,
  reportBlocks: 8,
};
const clamp = (value, low, high) => Math.min(high, Math.max(low, value));

class InputGate {
  constructor(rate) {
    this.blockFrames = Math.round((rate * GATE.blockMs) / 1000);
    this.attackStep = 1000 / (GATE.attackMs * rate);
    this.releaseStep = 1000 / (GATE.releaseMs * rate);
    this.mode = "off"; // "off" | "auto" | "manual"
    this.manualDb = -50;
    this.gain = 1; // the input gain after this node: levels are measured as sent
    // Starts high: the first quiet blocks pull it down to the room at once.
    this.floorDb = GATE.floorMaxDb;
    this.thresholdDb = GATE.levelMinDb;
    this.levelDb = GATE.silenceDb;
    this.open = true;
    this.holdMs = 0;
    this.amount = 1;
    this.blocks = 0;
    this.run = 0; // blocks in a row that open the gate
    this.delay = Array.from(
      { length: GATE.lookaheadBlocks + 1 },
      () => new Float32Array(this.blockFrames),
    );
    this.slot = 0;
    this.bypass = false; // off: no delay
  }
  configure(config) {
    const mode = config?.mode;
    this.mode = mode === "auto" || mode === "manual" ? mode : "off";
    const db = Number(config?.thresholdDb);
    this.manualDb = Number.isFinite(db)
      ? clamp(db, GATE.manualMinDb, GATE.manualMaxDb)
      : -50;
    if (this.mode === "off") {
      this.open = true;
      this.amount = 1;
    }
  }
  setGain(value) {
    this.gain = Number.isFinite(value) && value > 0 ? value : 0;
  }
  /** One 10 ms block in, the one `lookaheadBlocks` before it out, in place
   * (off: the same block, unchanged). `speech` is RNNoise's probability for
   * it, or null without RNNoise.
   * Returns true when a report is due. */
  block(frame, speech) {
    let sum = 0;
    for (let i = 0; i < frame.length; i++) sum += frame[i] * frame[i];
    const rms = Math.sqrt(sum / Math.max(1, frame.length)) * this.gain;
    const level =
      rms > 0 ? Math.max(GATE.silenceDb, 20 * Math.log10(rms)) : GATE.silenceDb;
    this.levelDb = level;
    // The floor falls to any quieter block at once and rises slowly, so
    // pauses between words keep it at the room's noise.
    this.floorDb =
      level < this.floorDb
        ? Math.max(level, GATE.floorMinDb)
        : Math.min(
            this.floorDb + (GATE.floorRiseDbPerS * GATE.blockMs) / 1000,
            GATE.floorMaxDb,
          );
    const useSpeech = this.mode === "auto" && typeof speech === "number";
    this.thresholdDb =
      this.mode === "manual"
        ? this.manualDb
        : useSpeech
          ? clamp(
              this.floorDb + GATE.speechMarginDb,
              GATE.speechMinDb,
              GATE.speechMaxDb,
            )
          : clamp(
              this.floorDb + GATE.levelMarginDb,
              GATE.levelMinDb,
              GATE.levelMaxDb,
            );
    if (this.mode === "off") {
      // Off (Original, push-to-talk) costs no look-ahead: the block leaves
      // as it came. Switched on, the gate starts from an empty delay.
      this.open = true;
      this.run = 0;
      this.amount = 1;
      if (!this.bypass) {
        this.bypass = true;
        for (const slot of this.delay) slot.fill(0);
      }
      this.blocks = (this.blocks + 1) % GATE.reportBlocks;
      return this.blocks === 0;
    }
    this.bypass = false;
    const opens =
      level >= this.thresholdDb && (!useSpeech || speech >= GATE.speechOpen);
    // With speech probabilities, only speech keeps it open: a key click
    // in a pause must not stretch the hold.
    const stays = useSpeech
      ? opens
      : level >= this.thresholdDb - GATE.hysteresisDb;
    this.run = opens ? this.run + 1 : 0;
    if (this.run >= GATE.openBlocks || (this.open && stays)) {
      this.open = true;
      this.holdMs = GATE.holdMs;
    } else if (this.open) {
      this.holdMs -= GATE.blockMs;
      if (this.holdMs <= 0) this.open = false;
    }
    // The newest block waits; the oldest one leaves, gated.
    const newest = this.delay[this.slot];
    newest.set(frame);
    this.slot = (this.slot + 1) % this.delay.length;
    frame.set(this.delay[this.slot]);
    const target = this.open ? 1 : 0;
    if (target !== 1 || this.amount !== 1)
      for (let i = 0; i < frame.length; i++) {
        this.amount =
          target > this.amount
            ? Math.min(1, this.amount + this.attackStep)
            : Math.max(0, this.amount - this.releaseStep);
        frame[i] = this.amount === 0 ? 0 : frame[i] * this.amount;
      }
    this.blocks = (this.blocks + 1) % GATE.reportBlocks;
    return this.blocks === 0;
  }
  report() {
    return {
      op: "gate",
      mode: this.mode,
      levelDb: this.levelDb,
      thresholdDb: this.thresholdDb,
      open: this.open,
    };
  }
}

/** The gate's messages, shared by both processors. */
function gateMessage(processor, data) {
  if (data?.op === "gateConfig") processor.gate.configure(data.config);
  else if (data?.op === "gain") processor.gate.setGain(Number(data.value));
}

/** An RNNoise instance with its frame buffers: `api`, `size`, `state`,
 * `input` and `output` (byte offsets) and `heap` over its memory. */
function createRNNoise(model) {
  const instance = new WebAssembly.Instance(model, {
    wasi_snapshot_preview1: {
      fd_write: () => 0,
      proc_exit: () => {
        throw new Error("RNNoise aborted");
      },
    },
    env: { emscripten_notify_memory_growth: () => {} },
  });
  const api = instance.exports;
  api._initialize?.();
  const size = api.rnnoise_get_frame_size();
  if (sampleRate !== 48000 || size !== 480)
    throw new Error("Unsupported RNNoise format");
  const state = api.rnnoise_create(0);
  const input = api.malloc(size * 4);
  const output = api.malloc(size * 4);
  if (!state || !input || !output) throw new Error("RNNoise allocation failed");
  return {
    api,
    size,
    state,
    input,
    output,
    heap: new Float32Array(api.memory.buffer),
  };
}

class RNNoiseProcessor extends AudioWorkletProcessor {
  constructor(options) {
    super();
    Object.assign(this, createRNNoise(options.processorOptions.model));
    this.frame = new Float32Array(this.size);
    this.queue = new Float32Array(4096);
    this.head = 0;
    this.tail = this.size; // bounded 10 ms reblocking, including arbitrary quanta
    this.count = 0;
    this.disposed = false;
    this.gate = new InputGate(sampleRate);
    this.gate.configure(options.processorOptions.gate);
    this.gate.setGain(options.processorOptions.gain ?? 1);
    this.port.postMessage({ ready: true });
    this.port.onmessage = (event) => {
      gateMessage(this, event.data);
      if (event.data?.op !== "dispose" || this.disposed) return;
      this.disposed = true;
      this.api.rnnoise_destroy(this.state);
      this.api.free(this.input);
      this.api.free(this.output);
    };
  }
  process(inputs, outputs) {
    if (this.disposed) return false;
    const channels = inputs[0] ?? [];
    const target = outputs[0]?.[0];
    if (!target) return true;
    for (let i = 0; i < target.length; i++) {
      let value = 0;
      for (let c = 0; c < channels.length; c++) value += channels[c][i] || 0;
      if (channels.length) value /= channels.length;
      this.heap[(this.input >> 2) + this.count++] = value * 32768;
      if (this.count === this.size) {
        const speech = this.api.rnnoise_process_frame(
          this.state,
          this.output,
          this.input,
        );
        for (let j = 0; j < this.size; j++)
          this.frame[j] = this.heap[(this.output >> 2) + j] / 32768;
        if (this.gate.block(this.frame, speech))
          this.port.postMessage(this.gate.report());
        for (let j = 0; j < this.size; j++)
          this.queue[this.tail++ % this.queue.length] = this.frame[j];
        this.count = 0;
      }
      target[i] =
        this.head < this.tail ? this.queue[this.head++ % this.queue.length] : 0;
    }
    return true;
  }
}
registerProcessor("gelabber-rnnoise", RNNoiseProcessor);

/** The gate alone, for "Browser filters": the same 10 ms reblocking. */
class GateProcessor extends AudioWorkletProcessor {
  constructor(options) {
    super();
    this.gate = new InputGate(sampleRate);
    this.gate.configure(options?.processorOptions?.gate);
    this.gate.setGain(options?.processorOptions?.gain ?? 1);
    this.size = this.gate.blockFrames;
    this.frame = new Float32Array(this.size);
    this.queue = new Float32Array(Math.max(4096, this.size * 4));
    this.head = 0;
    this.tail = this.size;
    this.count = 0;
    this.disposed = false;
    this.port.postMessage({ ready: true });
    this.port.onmessage = (event) => {
      gateMessage(this, event.data);
      if (event.data?.op === "dispose") this.disposed = true;
    };
  }
  process(inputs, outputs) {
    if (this.disposed) return false;
    const channels = inputs[0] ?? [];
    const target = outputs[0]?.[0];
    if (!target) return true;
    for (let i = 0; i < target.length; i++) {
      let value = 0;
      for (let c = 0; c < channels.length; c++) value += channels[c][i] || 0;
      if (channels.length) value /= channels.length;
      this.frame[this.count++] = value;
      if (this.count === this.size) {
        if (this.gate.block(this.frame, null))
          this.port.postMessage(this.gate.report());
        for (let j = 0; j < this.size; j++)
          this.queue[this.tail++ % this.queue.length] = this.frame[j];
        this.count = 0;
      }
      target[i] =
        this.head < this.tail ? this.queue[this.head++ % this.queue.length] : 0;
    }
    return true;
  }
}
registerProcessor("gelabber-gate", GateProcessor);

/** "Enhanced voice: Strong" (GEL-28): when DeepFilterNet3 cannot keep up.
 *
 * Each 10 ms block has to fit DFN3, RNNoise (for the gate), the gate and
 * the browser's own work on this thread. Times come from
 * `performance.now()` where the worklet has it, otherwise from `Date.now()`,
 * whose 1 ms steps average out over the hundreds of blocks judged here.
 * They are wall-clock times on the audio thread, not CPU times: a browser
 * offers no CPU clock. */
const STRONG = {
  blockMs: 10,
  // The first second is not judged: the code may still be warming up.
  warmupBlocks: 100,
  // Judged over the last 3 s: DFN3 and RNNoise together may take 70 % of a
  // block on average, and no more than one block in twenty may take longer
  // than the block itself.
  windowBlocks: 300,
  meanShare: 0.7,
  lateShare: 0.05,
  // Underruns: over 5 s of the wall clock, the audio clock may fall behind
  // by 10 % at most, judged on two windows in a row. A single pause between
  // two render quanta longer than gapMs is an interruption (a suspended
  // context, an output device switch, a clock step), not lag: it starts
  // the measurement over.
  lagWindowMs: 5000,
  lagShare: 0.1,
  lagWindows: 2,
  gapMs: 150,
  reportBlocks: 100,
};
const clock =
  typeof globalThis.performance?.now === "function"
    ? { name: "performance", now: () => globalThis.performance.now() }
    : { name: "date", now: () => Date.now() };

class StrongLoad {
  constructor() {
    this.totals = new Float64Array(STRONG.windowBlocks);
    this.dfns = new Float64Array(STRONG.windowBlocks);
    this.blocks = 0;
    this.sum = 0;
    this.dfnSum = 0;
    this.late = 0;
    this.lagStart = null;
    this.lagBehind = 0; // windows in a row that fell behind
    this.lagFrame = 0;
    this.lastWall = null;
    this.count = 0;
    this.reportDfn = 0;
    this.reportRnnoise = 0;
    this.reportMax = 0;
  }
  /** One block's DFN3 and RNNoise times in ms. Returns "slow" when DFN3
   * cannot keep up, otherwise null. */
  block(dfnMs, rnnoiseMs) {
    const total = dfnMs + rnnoiseMs;
    this.count++;
    this.reportDfn += dfnMs;
    this.reportRnnoise += rnnoiseMs;
    this.reportMax = Math.max(this.reportMax, total);
    this.blocks++;
    const judged = this.blocks - STRONG.warmupBlocks;
    if (judged <= 0) return null;
    const slot = (judged - 1) % STRONG.windowBlocks;
    if (judged > STRONG.windowBlocks) {
      this.sum -= this.totals[slot];
      this.dfnSum -= this.dfns[slot];
      if (this.totals[slot] > STRONG.blockMs) this.late--;
    }
    this.totals[slot] = total;
    this.dfns[slot] = dfnMs;
    this.sum += total;
    this.dfnSum += dfnMs;
    if (total > STRONG.blockMs) this.late++;
    if (judged < STRONG.windowBlocks) return null;
    return this.sum / STRONG.windowBlocks > STRONG.meanShare * STRONG.blockMs ||
      this.late > STRONG.lateShare * STRONG.windowBlocks
      ? "slow"
      : null;
  }
  /** Each render quantum's wall-clock time in ms and audio frame. Returns
   * "underrun" when the audio falls behind the wall clock, otherwise null. */
  quantum(nowMs, frame) {
    if (this.blocks <= STRONG.warmupBlocks) return null;
    const last = this.lastWall;
    this.lastWall = nowMs;
    if (this.lagStart === null || nowMs - last > STRONG.gapMs) {
      this.lagBehind = 0;
      this.lagStart = nowMs;
      this.lagFrame = frame;
      return null;
    }
    const wallMs = nowMs - this.lagStart;
    if (wallMs < STRONG.lagWindowMs) return null;
    const audioMs = ((frame - this.lagFrame) * 1000) / sampleRate;
    this.lagStart = nowMs;
    this.lagFrame = frame;
    this.lagBehind =
      wallMs - audioMs > STRONG.lagShare * wallMs ? this.lagBehind + 1 : 0;
    return this.lagBehind >= STRONG.lagWindows ? "underrun" : null;
  }
  /** Mean DFN3 and DFN3 + RNNoise times of the judged blocks. */
  window() {
    const filled = Math.min(
      STRONG.windowBlocks,
      Math.max(0, this.blocks - STRONG.warmupBlocks),
    );
    return filled
      ? { dfnMs: this.dfnSum / filled, totalMs: this.sum / filled }
      : { dfnMs: this.reportDfn / Math.max(1, this.count), totalMs: null };
  }
  /** Means and the slowest block since the last report. */
  report() {
    const report = {
      dfnMs: this.reportDfn / Math.max(1, this.count),
      rnnoiseMs: this.reportRnnoise / Math.max(1, this.count),
      maxMs: this.reportMax,
    };
    this.count = 0;
    this.reportDfn = 0;
    this.reportRnnoise = 0;
    this.reportMax = 0;
    return report;
  }
}

/** DeepFilterNet3 from public/audio/deepfilternet.wasm, restored from the
 * memory the worker built the model in (deepfilternet-worker.js). */
class DeepFilter {
  constructor(options) {
    this.api = new WebAssembly.Instance(options.module, {}).exports;
    const memory = this.api.memory;
    const pages =
      (options.memory.byteLength - memory.buffer.byteLength) / 65536;
    if (pages > 0) memory.grow(pages);
    new Uint8Array(memory.buffer).set(new Uint8Array(options.memory));
    this.state = options.state;
    this.hop = this.api.df_hop_size(this.state);
    if (this.hop !== 480) throw new Error("Unsupported DeepFilterNet format");
    this.views();
  }
  /** Views into the module's memory, again after it grew: growing
   * detaches the old ones. */
  views() {
    this.buffer = this.api.memory.buffer;
    this.input = new Float32Array(
      this.buffer,
      this.api.df_input(this.state),
      this.hop,
    );
    this.output = new Float32Array(
      this.buffer,
      this.api.df_output(this.state),
      this.hop,
    );
  }
  /** The hop in `input` into `output`. False when the model failed. */
  process() {
    const snr = this.api.df_process(this.state);
    if (this.api.memory.buffer !== this.buffer) this.views();
    return !Number.isNaN(snr);
  }
}

/** "Enhanced voice: Strong": DeepFilterNet3, with RNNoise beside it for
 * the gate's speech probability. DFN3's output is 10 ms behind RNNoise's,
 * so the gate gets the probability of the block before, as measured in
 * docs/v0.13-deepfilternet/gate.mjs. When DFN3 cannot keep up or fails,
 * the processor sends RNNoise's output from then on (Standard) and says so
 * on its port. */
class StrongProcessor extends AudioWorkletProcessor {
  constructor(options) {
    super();
    const config = options.processorOptions;
    Object.assign(this, createRNNoise(config.model));
    // Without memory for the model (or a broken copy) this is Standard,
    // said on the port right after the hello.
    let failed = false;
    try {
      this.dfn = new DeepFilter(config.dfn);
    } catch {
      this.dfn = null;
      failed = true;
    }
    this.load = new StrongLoad();
    this.speech = 0; // RNNoise's probability for the block before
    this.frame = new Float32Array(this.size);
    this.queue = new Float32Array(4096);
    this.head = 0;
    this.tail = this.size;
    this.count = 0;
    this.disposed = false;
    this.gate = new InputGate(sampleRate);
    this.gate.configure(config.gate);
    this.gate.setGain(config.gain ?? 1);
    this.port.postMessage({ ready: true, clock: clock.name });
    if (failed) this.fallBack("error");
    this.port.onmessage = (event) => {
      gateMessage(this, event.data);
      if (event.data?.op !== "dispose" || this.disposed) return;
      this.disposed = true;
      this.dfn = null;
      this.api.rnnoise_destroy(this.state);
      this.api.free(this.input);
      this.api.free(this.output);
    };
  }
  /** Standard from now on. */
  fallBack(reason) {
    const { dfnMs, totalMs } = this.load.window();
    this.dfn = null;
    this.port.postMessage({ op: "strong", fallback: reason, dfnMs, totalMs });
  }
  process(inputs, outputs) {
    if (this.disposed) return false;
    const channels = inputs[0] ?? [];
    const target = outputs[0]?.[0];
    if (!target) return true;
    if (this.dfn) {
      const verdict = this.load.quantum(clock.now(), currentFrame);
      if (verdict) this.fallBack(verdict);
    }
    for (let i = 0; i < target.length; i++) {
      let value = 0;
      for (let c = 0; c < channels.length; c++) value += channels[c][i] || 0;
      if (channels.length) value /= channels.length;
      this.heap[(this.input >> 2) + this.count] = value * 32768;
      if (this.dfn) this.dfn.input[this.count] = value;
      this.count++;
      if (this.count === this.size) {
        const started = clock.now();
        const speech = this.api.rnnoise_process_frame(
          this.state,
          this.output,
          this.input,
        );
        let gateSpeech = speech;
        let filtered = false;
        if (this.dfn) {
          const between = clock.now();
          let ok;
          try {
            ok = this.dfn.process();
          } catch {
            ok = false; // a trap: the instance is lost
          }
          const done = clock.now();
          if (ok) {
            this.frame.set(this.dfn.output);
            gateSpeech = this.speech;
            filtered = true;
            const verdict = this.load.block(done - between, between - started);
            if (this.load.count >= STRONG.reportBlocks)
              this.port.postMessage({ op: "strong", ...this.load.report() });
            if (verdict) this.fallBack(verdict);
          } else this.fallBack("error");
        }
        if (!filtered)
          for (let j = 0; j < this.size; j++)
            this.frame[j] = this.heap[(this.output >> 2) + j] / 32768;
        this.speech = speech;
        if (this.gate.block(this.frame, gateSpeech))
          this.port.postMessage(this.gate.report());
        for (let j = 0; j < this.size; j++)
          this.queue[this.tail++ % this.queue.length] = this.frame[j];
        this.count = 0;
      }
      target[i] =
        this.head < this.tail ? this.queue[this.head++ % this.queue.length] : 0;
    }
    return true;
  }
}
registerProcessor("gelabber-strong", StrongProcessor);
