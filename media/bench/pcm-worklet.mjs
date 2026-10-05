import { PCM, markerCode, markerSample, MarkerDetector } from './pcm-kernel.mjs';

class PcmMarkerProcessor extends AudioWorkletProcessor {
  constructor(options) {
    super();
    this.mode = options.processorOptions.mode;
    this.source = options.processorOptions.source; this.startFrame = options.processorOptions.startFrame;
    this.code = markerCode(options.processorOptions.source);
    this.next = options.processorOptions.startFrame;
    this.sequence = 0; this.clipped = 0;
    this.frameGaps = 0; this.lastFrame = undefined;
    this.port.onmessage = () => this.port.postMessage({ type: 'state', frame_gaps: this.frameGaps, clipped_frames: this.clipped });
    this.detector = new MarkerDetector(this.source, event => this.port.postMessage({ type: 'received', ...event }), this.startFrame);
    if (sampleRate !== PCM.sampleRate) throw new Error('PCM fixture requires 48 kHz');
  }
  process(inputs, outputs) {
    const input = inputs[0]?.[0], output = outputs[0][0];
    if (this.lastFrame !== undefined && currentFrame !== this.lastFrame) this.frameGaps += Math.abs(currentFrame - this.lastFrame);
    this.lastFrame = currentFrame + output.length;
    for (let i = 0; i < output.length; i++) {
      const frame = currentFrame + i, value = input?.[i] ?? 0;
      if (this.mode === 'source') {
        while (frame >= this.next + PCM.periodFrames) this.next += PCM.periodFrames;
        if (frame === this.next) {
          this.sequence = (frame - this.startFrame) / PCM.periodFrames;
          this.code = markerCode(this.source, this.sequence);
          this.port.postMessage({ type: 'sent', sequence: this.sequence, sentFrame: frame, clipped_frames: this.clipped });
        }
        const mixed = value + markerSample(this.code, frame - this.next);
        if (Math.abs(mixed) > 1) this.clipped++;
        output[i] = Math.max(-1, Math.min(1, mixed));
      } else {
        this.detector.push(value, frame); output[i] = 0;
      }
    }
    return true;
  }
}
registerProcessor('gelabber-pcm-marker', PcmMarkerProcessor);
