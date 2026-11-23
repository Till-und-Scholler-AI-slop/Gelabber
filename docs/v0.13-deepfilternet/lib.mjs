// Shared DSP helpers for the GEL-28 measurement. Plain Node, no dependencies,
// so every step that shapes the audio is readable here.
import fs from "node:fs";

export function readWav(path) {
  const b = fs.readFileSync(path);
  if (b.toString("ascii", 0, 4) !== "RIFF" || b.toString("ascii", 8, 12) !== "WAVE")
    throw new Error(`${path}: not a RIFF/WAVE file`);
  let off = 12,
    fmt = null,
    data = null;
  while (off + 8 <= b.length) {
    const id = b.toString("ascii", off, off + 4),
      size = b.readUInt32LE(off + 4),
      body = off + 8;
    if (id === "fmt ")
      fmt = {
        format: b.readUInt16LE(body),
        channels: b.readUInt16LE(body + 2),
        rate: b.readUInt32LE(body + 4),
        bits: b.readUInt16LE(body + 14),
      };
    if (id === "data") data = b.subarray(body, body + Math.min(size, b.length - body));
    off = body + size + (size & 1);
  }
  if (!fmt || !data) throw new Error(`${path}: missing fmt or data chunk`);
  const { format, channels, rate, bits } = fmt,
    frames = Math.floor(data.length / (channels * (bits / 8))),
    out = new Float32Array(frames);
  // Mixed down to mono: every source used here is mono except the DFN assets
  // we do not use.
  for (let n = 0; n < frames; n++) {
    let sum = 0;
    for (let c = 0; c < channels; c++) {
      const i = (n * channels + c) * (bits / 8);
      if (format === 1 && bits === 16) sum += data.readInt16LE(i) / 32768;
      else if (format === 3 && bits === 32) sum += data.readFloatLE(i);
      else throw new Error(`${path}: unsupported format ${format}/${bits}`);
    }
    out[n] = sum / channels;
  }
  return { rate, samples: out };
}

// 32-bit float WAV, so no clipping or requantisation between steps.
export function writeWav(path, rate, samples) {
  const b = Buffer.alloc(44 + samples.length * 4);
  b.write("RIFF", 0, "ascii");
  b.writeUInt32LE(36 + samples.length * 4, 4);
  b.write("WAVEfmt ", 8, "ascii");
  b.writeUInt32LE(16, 16);
  b.writeUInt16LE(3, 20);
  b.writeUInt16LE(1, 22);
  b.writeUInt32LE(rate, 24);
  b.writeUInt32LE(rate * 4, 28);
  b.writeUInt16LE(4, 32);
  b.writeUInt16LE(32, 34);
  b.write("data", 36, "ascii");
  b.writeUInt32LE(samples.length * 4, 40);
  for (let n = 0; n < samples.length; n++) b.writeFloatLE(samples[n], 44 + n * 4);
  fs.writeFileSync(path, b);
}

// Deterministic PRNG (same LCG as web/scripts/audio/benchmark-rnnoise.mjs).
export function lcg(seed) {
  let s = seed >>> 0;
  return () => {
    s = (Math.imul(s, 1664525) + 1013904223) >>> 0;
    return s / 0x100000000;
  };
}

function bessel0(x) {
  let sum = 1,
    term = 1;
  for (let k = 1; k < 50; k++) {
    term *= (x / (2 * k)) ** 2;
    sum += term;
  }
  return sum;
}

// Kaiser-windowed (beta 10) sinc low-pass for rational 1:3 / 3:1
// resampling, cutoff 7.6 kHz, 193 taps at 48 kHz.
const FACTOR = 3,
  TAPS_PER_PHASE = 64,
  KERNEL = (() => {
    const len = FACTOR * TAPS_PER_PHASE + 1,
      mid = (len - 1) / 2,
      fc = 7600 / 48000,
      beta = 10,
      h = new Float64Array(len);
    for (let i = 0; i < len; i++) {
      const t = i - mid,
        sinc = t === 0 ? 2 * fc : Math.sin(2 * Math.PI * fc * t) / (Math.PI * t),
        w = bessel0(beta * Math.sqrt(1 - (t / mid) ** 2)) / bessel0(beta);
      h[i] = sinc * w;
    }
    return h;
  })();

export function upsample16to48(x) {
  const mid = (KERNEL.length - 1) / 2,
    y = new Float32Array(x.length * FACTOR);
  for (let n = 0; n < y.length; n++) {
    // y[n] = sum_k x[k] * h[n + mid - 3k] * 3, zero-phase.
    let acc = 0;
    const kMin = Math.ceil((n + mid - (KERNEL.length - 1)) / FACTOR),
      kMax = Math.floor((n + mid) / FACTOR);
    for (let k = Math.max(0, kMin); k <= Math.min(x.length - 1, kMax); k++)
      acc += x[k] * KERNEL[n + mid - FACTOR * k];
    y[n] = acc * FACTOR;
  }
  return y;
}

export function downsample48to16(x) {
  const mid = (KERNEL.length - 1) / 2,
    y = new Float32Array(Math.floor(x.length / FACTOR));
  for (let m = 0; m < y.length; m++) {
    const c = m * FACTOR;
    let acc = 0;
    for (let i = 0; i < KERNEL.length; i++) {
      const k = c + i - mid;
      if (k >= 0 && k < x.length) acc += x[k] * KERNEL[i];
    }
    y[m] = acc;
  }
  return y;
}

// In-place iterative radix-2 FFT on separate real/imaginary arrays.
function fft(re, im, inverse) {
  const n = re.length;
  for (let i = 1, j = 0; i < n; i++) {
    let bit = n >> 1;
    for (; j & bit; bit >>= 1) j ^= bit;
    j ^= bit;
    if (i < j) {
      [re[i], re[j]] = [re[j], re[i]];
      [im[i], im[j]] = [im[j], im[i]];
    }
  }
  for (let len = 2; len <= n; len <<= 1) {
    const ang = ((inverse ? 2 : -2) * Math.PI) / len,
      wr = Math.cos(ang),
      wi = Math.sin(ang);
    for (let i = 0; i < n; i += len) {
      let cr = 1,
        ci = 0;
      for (let j = 0; j < len / 2; j++) {
        const a = i + j,
          b = a + len / 2,
          tr = re[b] * cr - im[b] * ci,
          ti = re[b] * ci + im[b] * cr;
        re[b] = re[a] - tr;
        im[b] = im[a] - ti;
        re[a] += tr;
        im[a] += ti;
        const t = cr * wr - ci * wi;
        ci = cr * wi + ci * wr;
        cr = t;
      }
    }
  }
  if (inverse)
    for (let i = 0; i < n; i++) {
      re[i] /= n;
      im[i] /= n;
    }
}

// Linear convolution, truncated to the length of x.
export function convolve(x, h) {
  let n = 1;
  while (n < x.length + h.length) n <<= 1;
  const ar = new Float64Array(n),
    ai = new Float64Array(n),
    br = new Float64Array(n),
    bi = new Float64Array(n);
  ar.set(x);
  br.set(h);
  fft(ar, ai, false);
  fft(br, bi, false);
  for (let i = 0; i < n; i++) {
    const r = ar[i] * br[i] - ai[i] * bi[i];
    ai[i] = ar[i] * bi[i] + ai[i] * br[i];
    ar[i] = r;
  }
  fft(ar, ai, true);
  return Float32Array.from(ar.subarray(0, x.length));
}

// RBJ biquad, used to make the TV source sound like a small loudspeaker.
export function biquad(x, type, freq, rate, q = Math.SQRT1_2) {
  const w = (2 * Math.PI * freq) / rate,
    cw = Math.cos(w),
    alpha = Math.sin(w) / (2 * q),
    a0 = 1 + alpha;
  const b =
    type === "lowpass"
      ? [(1 - cw) / 2, 1 - cw, (1 - cw) / 2]
      : [(1 + cw) / 2, -(1 + cw), (1 + cw) / 2];
  const a1 = -2 * cw,
    a2 = 1 - alpha,
    y = new Float32Array(x.length);
  let x1 = 0,
    x2 = 0,
    y1 = 0,
    y2 = 0;
  for (let n = 0; n < x.length; n++) {
    const v = (b[0] * x[n] + b[1] * x1 + b[2] * x2 - a1 * y1 - a2 * y2) / a0;
    x2 = x1;
    x1 = x[n];
    y2 = y1;
    y1 = v;
    y[n] = v;
  }
  return y;
}

export function rms(x) {
  let s = 0;
  for (const v of x) s += v * v;
  return Math.sqrt(s / Math.max(1, x.length));
}

// Speech level over 20 ms frames within 40 dB of the loudest frame, a simple
// stand-in for an ITU-T P.56 active level so pauses do not lower the SNR base.
export function activeRms(x, rate) {
  const frame = Math.round(rate * 0.02),
    energies = [];
  for (let n = 0; n + frame <= x.length; n += frame) {
    let s = 0;
    for (let i = n; i < n + frame; i++) s += x[i] * x[i];
    energies.push(s / frame);
  }
  const max = Math.max(...energies),
    active = energies.filter((e) => e > max * 1e-4);
  return Math.sqrt(active.reduce((a, b) => a + b, 0) / active.length);
}

export function scale(x, gain) {
  return x.map((v) => v * gain);
}

export function dbfs(level) {
  return 20 * Math.log10(level);
}

// Scale-invariant SDR (Le Roux et al. 2019) of `est` against `ref`.
export function siSdr(ref, est) {
  const n = Math.min(ref.length, est.length);
  let dot = 0,
    rr = 0;
  for (let i = 0; i < n; i++) {
    dot += ref[i] * est[i];
    rr += ref[i] * ref[i];
  }
  const a = dot / rr;
  let s = 0,
    e = 0;
  for (let i = 0; i < n; i++) {
    const t = a * ref[i];
    s += t * t;
    e += (est[i] - t) ** 2;
  }
  return 10 * Math.log10(s / e);
}

// Lag (in samples, 0..maxLag) at which `processed` best matches `reference`
// by normalised cross-correlation over [start, start + length).
export function bestLag(reference, processed, start, length, maxLag) {
  let best = { lag: -1, corr: -Infinity };
  let rr = 0;
  for (let n = start; n < start + length; n++) rr += reference[n] * reference[n];
  for (let lag = 0; lag <= maxLag; lag++) {
    let dot = 0,
      pp = 0;
    for (let n = start; n < start + length; n++) {
      const p = processed[n + lag] ?? 0;
      dot += reference[n] * p;
      pp += p * p;
    }
    const corr = dot / Math.sqrt(rr * pp || 1);
    if (corr > best.corr) best = { lag, corr };
  }
  return best;
}

export function percentile(sorted, p) {
  if (!sorted.length) return NaN;
  return sorted[Math.min(sorted.length - 1, Math.floor((p / 100) * sorted.length))];
}

export function timingSummary(times) {
  const s = Float64Array.from(times).sort();
  return {
    frames: s.length,
    medianMs: percentile(s, 50),
    p99Ms: percentile(s, 99),
    maxMs: s[s.length - 1],
    meanMs: s.reduce((a, b) => a + b, 0) / s.length,
  };
}
