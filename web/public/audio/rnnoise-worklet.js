/* global AudioWorkletProcessor, sampleRate, registerProcessor, WebAssembly */
// RNNoise v0.2, 48 kHz mono. No allocation/network/memory growth in process().
// AudioWorklet quantum size is independent of the model's 480-sample frames.
//
// Both processors here end in the input sensitivity gate (GEL-27). The
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
  /** One 10 ms block in, the one `lookaheadBlocks` before it out, in place.
   * `speech` is RNNoise's probability for it, or null without RNNoise.
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
      this.open = true;
    } else {
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

class RNNoiseProcessor extends AudioWorkletProcessor {
  constructor(options) {
    super();
    this.instance = new WebAssembly.Instance(options.processorOptions.model, {
      wasi_snapshot_preview1: {
        fd_write: () => 0,
        proc_exit: () => {
          throw new Error("RNNoise aborted");
        },
      },
      env: { emscripten_notify_memory_growth: () => {} },
    });
    this.api = this.instance.exports;
    this.api._initialize?.();
    this.size = this.api.rnnoise_get_frame_size();
    if (sampleRate !== 48000 || this.size !== 480)
      throw new Error("Unsupported RNNoise format");
    this.state = this.api.rnnoise_create(0);
    this.input = this.api.malloc(this.size * 4);
    this.output = this.api.malloc(this.size * 4);
    if (!this.state || !this.input || !this.output)
      throw new Error("RNNoise allocation failed");
    this.heap = new Float32Array(this.api.memory.buffer);
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
