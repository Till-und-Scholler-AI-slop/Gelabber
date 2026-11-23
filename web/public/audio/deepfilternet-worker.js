/* global self, performance, WebAssembly */
// Builds the DeepFilterNet3 model for "Enhanced voice: Strong" (GEL-28) off
// the main and the audio thread: about half a second on a server core.
// Answers with a copy of the module's memory right after the build; the
// worklet restores it into a fresh instance of the same module (see
// web/scripts/audio/deepfilternet/src/lib.rs for why that is sound).
//
// Then it runs the model on a fixed test signal, so the browser compiles
// the hot code with its optimizing tier before the call needs it, and
// reports the time per block it measured here: a first estimate, on a
// thread without real-time priority.

const WARM = { blocks: 300, maxMs: 1500, measured: 100 };

self.onmessage = (event) => {
  const { module, model, params } = event.data;
  try {
    const api = new WebAssembly.Instance(module, {}).exports;
    const bytes = new Uint8Array(model);
    const pointer = api.df_alloc(bytes.length);
    if (!pointer) throw new Error("allocation failed");
    new Uint8Array(api.memory.buffer, pointer, bytes.length).set(bytes);
    const started = performance.now();
    const state = api.df_create(pointer, bytes.length, ...params);
    if (!state) throw new Error("model did not build");
    const buildMs = performance.now() - started;
    const hop = api.df_hop_size(state);
    if (hop !== 480) throw new Error(`unexpected hop ${hop}`);
    const memory = api.memory.buffer.slice(0);

    const times = [];
    const warmStart = performance.now();
    for (let k = 0; k < WARM.blocks; k++) {
      const input = new Float32Array(
        api.memory.buffer,
        api.df_input(state),
        hop,
      );
      for (let j = 0; j < hop; j++) {
        const n = k * hop + j;
        input[j] =
          (k % 50 < 25 ? 0.1 * Math.sin(n * 0.0236) : 0) +
          0.003 * Math.sin(n * 1.7) * Math.sin(n * 0.31);
      }
      const t = performance.now();
      if (Number.isNaN(api.df_process(state))) throw new Error("model failed");
      times.push(performance.now() - t);
      if (performance.now() - warmStart > WARM.maxMs) break;
    }
    const last = times.slice(-WARM.measured);
    const blockMs = last.reduce((sum, ms) => sum + ms, 0) / last.length;
    self.postMessage({ ok: true, memory, state, buildMs, blockMs }, [memory]);
  } catch (error) {
    self.postMessage({ ok: false, error: String(error?.message ?? error) });
  }
};
