// Shared deterministic PCM marker/detector, independent of wall-clock delivery
// of worklet messages. Only 48 kHz is supported by this measurement fixture.
export const PCM = Object.freeze({ sampleRate: 48000, carrierHz: 2000, chipFrames: 96, chips: 63, periodFrames: 96000, amplitude: 0.35, threshold: 0.72, maxDelayMs: 1000, errorBoundMs: 2 });
export function markerCode(source, sequence = 0) {
  let seed = (Math.imul(source + 1, 0x9e3779b1) ^ Math.imul(sequence, 0x85ebca6b)) >>> 0;
  return Array.from({ length: PCM.chips }, () => {
    seed ^= seed << 13; seed ^= seed >>> 17; seed ^= seed << 5;
    return seed & 1 ? 1 : -1;
  });
}
export function markerSample(code, offset) {
  if (offset < 0 || offset >= PCM.chips * PCM.chipFrames) return 0;
  return code[Math.floor(offset / PCM.chipFrames)] * Math.sin(offset * 2 * Math.PI * PCM.carrierHz / PCM.sampleRate) * PCM.amplitude;
}

// Complex demodulation followed by a chip matched filter. Evaluate every 24
// original samples (0.5 ms), rather than convolving a full PCM buffer per frame.
export class MarkerDetector {
  constructor(source, emit, startFrame) {
    this.emit = emit;
    this.source = source; this.startFrame = startFrame; this.sequence = undefined;
    this.setCode(0);
    this.rawI = new Float64Array(24); this.rawQ = new Float64Array(24);
    this.historyI = new Float64Array(256); this.historyQ = new Float64Array(256);
    this.sumI = 0; this.sumQ = 0; this.entries = 0; this.candidate = null; this.refractory = 0;
  }
  setCode(sequence) {
    this.sequence = sequence;
    const code = markerCode(this.source, sequence), mean = code.reduce((a, b) => a + b) / code.length;
    this.weights = code.map(value => value - mean);
    this.weightEnergy = this.weights.reduce((sum, value) => sum + value * value, 0);
  }
  push(value, frame) {
    if (frame % 4) return;
    const downFrame = frame / 4, rawIndex = downFrame % 24;
    const phase = frame * 2 * Math.PI * PCM.carrierHz / PCM.sampleRate;
    this.sumI += value * Math.cos(phase) - this.rawI[rawIndex];
    this.sumQ += value * Math.sin(phase) - this.rawQ[rawIndex];
    this.rawI[rawIndex] = value * Math.cos(phase); this.rawQ[rawIndex] = value * Math.sin(phase);
    if (downFrame % 6) return;
    const index = this.entries++ % 256;
    this.historyI[index] = this.sumI; this.historyQ[index] = this.sumQ;
    if (this.entries < 253 || downFrame < this.refractory) return;
    if (this.startFrame !== undefined) {
      const sequence = Math.floor((frame - PCM.chips * PCM.chipFrames - this.startFrame) / PCM.periodFrames);
      if (sequence < 0) return;
      if (sequence !== this.sequence) this.setCode(sequence);
    }
    let real = 0, imaginary = 0, energy = 0;
    for (let chip = 0; chip < PCM.chips; chip++) {
      const slot = (index - (PCM.chips - 1 - chip) * 4 + 256) % 256;
      const i = this.historyI[slot], q = this.historyQ[slot];
      real += i * this.weights[chip]; imaginary += q * this.weights[chip]; energy += i * i + q * q;
    }
    const score = energy > 0 ? Math.hypot(real, imaginary) / Math.sqrt(energy * this.weightEnergy) : 0;
    const amplitude = Math.sqrt(energy / PCM.chips) / 12;
    if (!this.candidate && score >= PCM.threshold && amplitude >= 0.04) this.candidate = { until: downFrame + 120, score: 0, sequence: this.sequence };
    if (this.candidate) {
      if (score > this.candidate.score) Object.assign(this.candidate, { score, frame: downFrame, amplitude });
      if (downFrame >= this.candidate.until) {
        const best = this.candidate;
        this.emit({ receivedFrame: (best.frame - PCM.chips * 24 + 1) * 4, sequence: best.sequence, score: best.score, amplitude: best.amplitude });
        this.candidate = null; this.refractory = downFrame + PCM.sampleRate / 4 / 2;
      }
    }
  }
}
