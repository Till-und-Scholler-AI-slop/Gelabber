/* global AudioWorkletProcessor, sampleRate, registerProcessor, WebAssembly */
// RNNoise v0.2, 48 kHz mono. No allocation/network/memory growth in process().
// AudioWorklet quantum size is independent of the model's 480-sample frames.
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
    this.queue = new Float32Array(4096);
    this.head = 0;
    this.tail = this.size; // bounded 10 ms reblocking, including arbitrary quanta
    this.count = 0;
    this.disposed = false;
    this.port.postMessage({ ready: true });
    this.port.onmessage = (event) => {
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
        this.api.rnnoise_process_frame(this.state, this.output, this.input);
        for (let j = 0; j < this.size; j++) {
          this.queue[this.tail++ % this.queue.length] =
            this.heap[(this.output >> 2) + j] / 32768;
        }
        this.count = 0;
      }
      target[i] =
        this.head < this.tail ? this.queue[this.head++ % this.queue.length] : 0;
    }
    return true;
  }
}
registerProcessor("gelabber-rnnoise", RNNoiseProcessor);
