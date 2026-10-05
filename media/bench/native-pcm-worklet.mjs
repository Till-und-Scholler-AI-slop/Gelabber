import { PCM, MarkerDetector } from './pcm-kernel.mjs';
import { RING, dimensions, publishCallback } from './native-pcm-ring.mjs';

class NativePcmTap extends AudioWorkletProcessor {
  constructor({ processorOptions: options }) {
    super(); this.o = options; this.cells = new BigInt64Array(options.sab);
    if (this.cells.length !== dimensions(options.groups, options.capacity).cells) throw Error('callback ring size differs');
    if (!Number.isInteger(options.group) || options.group < 0 || options.group >= options.groups || !['clock', 'receive'].includes(options.mode) || ![0, 64].includes(options.uid) || !Number.isInteger(options.markerCount) || options.markerCount < 0 || options.markerCount > 180 || sampleRate !== PCM.sampleRate) throw Error('invalid native tap policy');
    this.seq = 0; this.nextFrame = undefined; this.gaps = 0; this.clipped = 0; this.missingInput = 0; this.nonfinite = 0;
    this.peaks = []; this.detectors = new Map(); this.anchor = undefined; this.excessPeaks = 0;
    this.port.onmessage = ({ data }) => {
      if (data.type === 'anchor') {
        if (this.anchor !== undefined || typeof data.startClockNs !== 'string' || !/^[1-9]\d*$/.test(data.startClockNs)) throw Error('invalid/repeated native anchor');
        this.anchor = BigInt(data.startClockNs);
      }
      if (data.type === 'report') this.port.postMessage({ type: 'report', uid: options.uid, peaks: this.peaks, excessPeaks: this.excessPeaks, gaps: this.gaps, clipped: this.clipped, missingInput: this.missingInput, nonfinite: this.nonfinite, currentFrame, sampleRate, blocks: this.seq, workletPerformance: typeof performance });
    };
  }
  process(inputs, outputs) {
    const lower = Atomics.load(this.cells, RING.heartbeat), frame = currentFrame;
    const output = outputs[0][0], input = inputs[0]?.[0];
    let flags = 0;
    if (this.nextFrame !== undefined && this.nextFrame !== frame) { flags |= 1; this.gaps++; }
    this.nextFrame = frame + output.length;
    if (this.o.mode === 'receive' && input?.length !== output.length) { flags |= 2; this.missingInput++; }
    if (this.anchor !== undefined) {
      const now = Atomics.load(this.cells, RING.nativeLower);
      const position = Number(now - this.anchor - 1000000000n) / 2000000000;
      const sequence = Math.floor(position);
      const candidates = [sequence - 1, sequence, sequence + 1].filter(seq => seq >= 0 && seq < this.o.markerCount);
      for (const seq of candidates) if (!this.detectors.has(seq)) {
        const detector = new MarkerDetector(this.o.uid, peak => {
          if (this.peaks.length >= Math.max(4, this.o.markerCount * 4)) this.excessPeaks++;
          else this.peaks.push(peak);
        });
        detector.setCode(seq); this.detectors.set(seq, detector);
      }
      for (const seq of this.detectors.keys()) if (!candidates.includes(seq)) this.detectors.delete(seq);
    }
    for (let i = 0; i < output.length; i++) {
      const value = input?.[i] ?? 0;
      if (!Number.isFinite(value)) { flags |= 4; this.nonfinite++; }
      if (Math.abs(value) >= .999) { flags |= 8; this.clipped++; }
      for (const detector of this.detectors.values()) detector.push(value, frame + i);
      output[i] = 0;
    }
    publishCallback(this.cells, this.o.group, this.o.capacity, ++this.seq, frame, output.length, lower, flags, this.o.mode === 'receive' ? input?.length ?? 0 : output.length);
    return true;
  }
}
registerProcessor('gelabber-native-pcm-tap', NativePcmTap);
