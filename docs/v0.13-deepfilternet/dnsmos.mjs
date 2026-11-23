// DNSMOS P.835 with Microsoft's ONNX models, following dnsmos_local.py at
// the pinned DNS-Challenge commit (work/dnsmos/dnsmos_local.py): 16 kHz,
// 9.01 s windows hopped by 1 s, clips shorter than a window repeated, the
// per-window scores averaged. Differences from the script: our own 48->16 kHz
// resampler instead of librosa's, and no P.808 score (it needs librosa's mel
// spectrogram and the ticket asks for P.835).
//
// Usage: node dnsmos.mjs <dir with 48 kHz wavs> <id>... -> JSON on stdout.
import * as ort from "onnxruntime-node";
import { downsample48to16, readWav } from "./lib.mjs";

const HERE = new URL(".", import.meta.url).pathname,
  FS = 16000,
  WINDOW = Math.trunc(9.01 * FS);

// Raw model outputs to MOS, from ComputeScore.get_polyfit_val (np.poly1d
// coefficients, highest power first).
const poly = (c) => (x) => c.reduce((acc, k) => acc * x + k, 0);
const MODELS = {
  dnsmos: {
    path: `${HERE}work/dnsmos/sig_bak_ovr.onnx`,
    sig: poly([-0.08397278, 1.22083953, 0.0052439]),
    bak: poly([-0.13166888, 1.60915514, -0.39604546]),
    ovr: poly([-0.06766283, 1.11546468, 0.04602535]),
  },
  // Personalized DNSMOS: trained to rate an interfering talker as noise.
  pdnsmos: {
    path: `${HERE}work/dnsmos/p_sig_bak_ovr.onnx`,
    sig: poly([-0.01019296, 0.02751166, 1.19576786, -0.24348726]),
    bak: poly([-0.04976499, 0.44276479, -0.1644611, 0.96883132]),
    ovr: poly([-0.00533021, 0.005101, 1.18058466, -0.11236046]),
  },
};

export async function createScorer() {
  const sessions = {};
  for (const [name, m] of Object.entries(MODELS))
    sessions[name] = await ort.InferenceSession.create(m.path, {
      intraOpNumThreads: 1,
    });
  return async function score(samples48) {
    let audio = downsample48to16(samples48);
    while (audio.length < WINDOW) {
      const doubled = new Float32Array(audio.length * 2);
      doubled.set(audio);
      doubled.set(audio, audio.length);
      audio = doubled;
    }
    const hops = Math.trunc(Math.floor(audio.length / FS) - 9.01) + 1,
      result = {};
    for (const [name, m] of Object.entries(MODELS)) {
      const session = sessions[name],
        sums = { sig: 0, bak: 0, ovr: 0 };
      let windows = 0;
      for (let i = 0; i < hops; i++) {
        const seg = audio.subarray(i * FS, i * FS + WINDOW);
        if (seg.length < WINDOW) continue;
        const out = await session.run({
          [session.inputNames[0]]: new ort.Tensor("float32", Float32Array.from(seg), [1, WINDOW]),
        });
        const [sig, bak, ovr] = out[session.outputNames[0]].data;
        sums.sig += m.sig(sig);
        sums.bak += m.bak(bak);
        sums.ovr += m.ovr(ovr);
        windows++;
      }
      result[name] = {
        SIG: sums.sig / windows,
        BAK: sums.bak / windows,
        OVRL: sums.ovr / windows,
        windows,
      };
    }
    return result;
  };
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const [dir, ...ids] = process.argv.slice(2),
    score = await createScorer(),
    out = {};
  for (const id of ids) out[id] = await score(readWav(`${dir}/${id}.wav`).samples);
  console.log(JSON.stringify(out, null, 2));
}
