// The "Signatur" call sounds: every event has its own small gesture, built
// from a few synthesized voices. Original and rendered in code, so there are
// no audio files to ship or license. Each render is the same sound: the noise
// is seeded. callSounds.ts turns a render into the WAV it plays.

export type CueName =
  | "joinSelf"
  | "joinOther"
  | "leaveOther"
  | "leaveSelf"
  | "muteSelf"
  | "unmuteSelf"
  | "muteOther"
  | "unmuteOther"
  | "deafen"
  | "undeafen"
  | "liveStart"
  | "liveEnd"
  | "message"
  | "viewerJoin"
  | "viewerLeave"
  | "cameraOn"
  | "cameraOff";

/** Target peak of each cue. Someone else's events sit below your own. */
export const CUE_LEVELS: Readonly<Record<CueName, number>> = {
  joinSelf: 0.34,
  joinOther: 0.26,
  leaveOther: 0.24,
  leaveSelf: 0.32,
  muteSelf: 0.3,
  unmuteSelf: 0.3,
  muteOther: 0.14,
  unmuteOther: 0.14,
  deafen: 0.3,
  undeafen: 0.3,
  liveStart: 0.32,
  liveEnd: 0.26,
  message: 0.18,
  viewerJoin: 0.18,
  viewerLeave: 0.15,
  cameraOn: 0.24,
  cameraOff: 0.22,
};

/** Room in the reverb, as in the audition the set was picked from. */
const ROOM_MIX = 0.12;
/** The canvas every recipe draws on, before the silent tail is cut. */
const CANVAS_SECONDS = 1.6;

// Seeded noise, so every render of a cue is the same sound.
function rng(seed: number): () => number {
  return () => {
    seed = (seed + 0x6d2b79f5) | 0;
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const hz = (midi: number) => 440 * Math.pow(2, (midi - 69) / 12);

// D major, one octave around D4. F3, F4 and F5 are the F sharps.
const N = { D3: 50, F3: 54, A3: 57, D4: 62, F4: 66, A4: 69, D5: 74, F5: 78 };

type GlideOptions = {
  glide?: number;
  hold?: number;
  decay?: number;
  attack?: number;
  vel?: number;
  parts?: readonly (readonly [number, number])[];
};
type BellOptions = { decay?: number; ratio?: number; index?: number };

/** The voices at one sample rate. Times are in seconds. */
function voices(sr: number) {
  // Topology-preserving state-variable filter; cutoff may change per sample.
  const svf = () => {
    let ic1 = 0;
    let ic2 = 0;
    return (x: number, fc: number, q: number, mode?: "band") => {
      const g = Math.tan((Math.PI * Math.min(fc, sr * 0.45)) / sr);
      const k = 1 / q;
      const a1 = 1 / (1 + g * (g + k));
      const a2 = g * a1;
      const a3 = g * a2;
      const v3 = x - ic2;
      const v1 = a1 * ic1 + a2 * v3;
      const v2 = ic2 + a2 * ic1 + a3 * v3;
      ic1 = 2 * v1 - ic1;
      ic2 = 2 * v2 - ic2;
      return mode === "band" ? v1 : v2;
    };
  };

  // Sum of decaying partials: [ratio, amplitude, decay seconds].
  const partials = (
    out: Float32Array,
    at: number,
    f: number,
    vel: number,
    list: readonly (readonly [number, number, number])[],
    attack: number,
  ) => {
    const start = Math.round(at * sr);
    const longest = Math.max(...list.map((p) => p[2]));
    const len = Math.min(out.length - start, Math.round(longest * 7 * sr));
    for (const [ratio, amp, decay] of list) {
      const w = (2 * Math.PI * f * ratio) / sr;
      for (let i = 0; i < len; i++) {
        const rise = Math.min(1, i / (attack * sr));
        const env = rise * rise * (3 - 2 * rise) * Math.exp(-i / (decay * sr));
        out[start + i] += vel * amp * env * Math.sin(w * i);
      }
    }
  };

  return {
    // Soft wood: marimba body, the bright overtones mostly gone, no mallet click.
    holz(out: Float32Array, at: number, midi: number, vel: number) {
      partials(
        out,
        at,
        hz(midi),
        vel,
        [
          [1, 1, 0.3],
          [3.93, 0.09, 0.04],
        ],
        0.006,
      );
    },

    // Felt: a damped piano string pair, slightly detuned, warm.
    filz(out: Float32Array, at: number, midi: number, vel: number) {
      partials(
        out,
        at,
        hz(midi),
        vel,
        [
          [1, 1, 0.45],
          [1.003, 0.45, 0.4],
          [2, 0.22, 0.18],
          [3, 0.06, 0.08],
        ],
        0.012,
      );
    },

    // Soft air sweep under going live (up) and a stream ending (down).
    whoosh(
      out: Float32Array,
      at: number,
      dur: number,
      from: number,
      to: number,
      gain: number,
    ) {
      const start = Math.round(at * sr);
      const len = Math.min(out.length - start, Math.round(dur * sr));
      const noise = rng(Math.round(from + to));
      const bp = svf();
      for (let i = 0; i < len; i++) {
        const p = i / len;
        const fc = from * Math.pow(to / from, p);
        const env = Math.pow(Math.sin(Math.PI * p), 2);
        out[start + i] += gain * env * bp(noise() * 2 - 1, fc, 0.9, "band");
      }
    },

    // A tone that glides from f0 to f1 (easing out), holds, then fades.
    glide(
      out: Float32Array,
      at: number,
      f0: number,
      f1: number,
      o: GlideOptions = {},
    ) {
      const time = o.glide ?? 0.15;
      const hold = o.hold ?? 0;
      const decay = o.decay ?? 0.15;
      const attack = o.attack ?? 0.008;
      const vel = o.vel ?? 1;
      const parts = o.parts ?? [
        [1, 1],
        [2, 0.25],
        [3, 0.08],
      ];
      const start = Math.round(at * sr);
      const len = Math.min(
        out.length - start,
        Math.round((hold + decay * 7) * sr),
      );
      let phase = 0;
      for (let i = 0; i < len; i++) {
        const t = i / sr;
        const p = Math.min(1, t / time);
        const f = f0 * Math.pow(f1 / f0, 1 - (1 - p) * (1 - p));
        const env =
          Math.min(1, t / attack) *
          (t < hold ? 1 : Math.exp(-(t - hold) / decay));
        let s = 0;
        for (const [ratio, amp] of parts) s += amp * Math.sin(phase * ratio);
        out[start + i] += vel * env * s;
        phase += (2 * Math.PI * f) / sr;
      }
    },

    // A soft tick, like a key or a switch.
    tick(
      out: Float32Array,
      at: number,
      fc: number,
      gain: number,
      seed: number,
    ) {
      const start = Math.round(at * sr);
      const len = Math.min(out.length - start, Math.round(0.02 * sr));
      const noise = rng(seed);
      const bp = svf();
      for (let i = 0; i < len; i++)
        out[start + i] +=
          gain *
          Math.exp(-i / (0.0025 * sr)) *
          bp(noise() * 2 - 1, fc, 1.2, "band");
    },

    // A low, round knock whose pitch drops fast.
    knock(
      out: Float32Array,
      at: number,
      f0: number,
      f1: number,
      gain: number,
      decay: number,
    ) {
      const start = Math.round(at * sr);
      const len = Math.min(out.length - start, Math.round(decay * 7 * sr));
      let phase = 0;
      for (let i = 0; i < len; i++) {
        const t = i / sr;
        const f = f1 + (f0 - f1) * Math.exp(-t / 0.025);
        out[start + i] +=
          gain *
          Math.min(1, t / 0.002) *
          Math.exp(-t / decay) *
          Math.sin(phase);
        phase += (2 * Math.PI * f) / sr;
      }
    },

    // A bubble: a sine sliding from f0 to f1.
    bubble(
      out: Float32Array,
      at: number,
      f0: number,
      f1: number,
      dur: number,
      gain: number,
    ) {
      const start = Math.round(at * sr);
      const len = Math.min(out.length - start, Math.round(dur * 5 * sr));
      let phase = 0;
      for (let i = 0; i < len; i++) {
        const t = i / sr;
        const f = f0 * Math.pow(f1 / f0, Math.min(1, t / dur));
        out[start + i] +=
          gain *
          Math.min(1, t / 0.004) *
          Math.exp(-t / (dur * 0.9)) *
          (Math.sin(phase) + 0.1 * Math.sin(2 * phase));
        phase += (2 * Math.PI * f) / sr;
      }
    },

    // FM bell at a frequency.
    bell(
      out: Float32Array,
      at: number,
      f: number,
      vel: number,
      o: BellOptions = {},
    ) {
      const decay = o.decay ?? 0.5;
      const ratio = o.ratio ?? 3.5;
      const index = o.index ?? 1;
      const start = Math.round(at * sr);
      const len = Math.min(out.length - start, Math.round(decay * 6 * sr));
      const wc = (2 * Math.PI * f) / sr;
      const wm = (2 * Math.PI * f * ratio) / sr;
      for (let i = 0; i < len; i++) {
        const env = Math.min(1, i / (0.004 * sr)) * Math.exp(-i / (decay * sr));
        out[start + i] +=
          vel *
          env *
          Math.sin(
            wc * i + index * Math.exp(-i / (0.08 * sr)) * Math.sin(wm * i),
          );
      }
    },

    // A chord behind a low-pass that closes or opens: hands over the ears and off again.
    muffle(
      out: Float32Array,
      at: number,
      dur: number,
      from: number,
      to: number,
      gain: number,
      freqs: readonly number[],
    ) {
      const start = Math.round(at * sr);
      const len = Math.min(out.length - start, Math.round((dur + 0.3) * sr));
      const lp = svf();
      const lp2 = svf();
      for (let i = 0; i < len; i++) {
        const t = i / sr;
        const fc = from * Math.pow(to / from, Math.min(1, t / dur));
        let s = 0;
        for (const f of freqs)
          for (let h = 1; h <= 8; h++)
            s += Math.sin(2 * Math.PI * f * h * t) / h;
        const env =
          Math.min(1, t / 0.03) * (t < dur ? 1 : Math.exp(-(t - dur) / 0.06));
        out[start + i] += gain * env * lp2(lp(s, fc, 0.9), fc, 0.9);
      }
    },

    // A thumb-piano tine whose pitch settles from slightly flat.
    tine(out: Float32Array, at: number, f: number, vel: number) {
      const start = Math.round(at * sr);
      const len = Math.min(out.length - start, Math.round(0.35 * 6 * sr));
      let phase = 0;
      for (let i = 0; i < len; i++) {
        const t = i / sr;
        const env = Math.min(1, t / 0.003) * Math.exp(-t / 0.35);
        out[start + i] +=
          vel *
          env *
          (Math.sin(phase) + 0.3 * Math.exp(-t / 0.02) * Math.sin(phase * 5.4));
        phase += (2 * Math.PI * f * (1 - 0.025 * Math.exp(-t / 0.03))) / sr;
      }
    },

    // Small Freeverb-style room, mono.
    room(out: Float32Array, mix: number) {
      const s = sr / 44100;
      const combs = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617].map(
        (d) => ({ buf: new Float32Array(Math.round(d * s)), i: 0, store: 0 }),
      );
      const passes = [556, 441, 341, 225].map((d) => ({
        buf: new Float32Array(Math.round(d * s)),
        i: 0,
      }));
      const wet = new Float32Array(out.length);
      for (let n = 0; n < out.length; n++) {
        let acc = 0;
        for (const c of combs) {
          const y = c.buf[c.i];
          c.store = y * 0.6 + c.store * 0.4;
          c.buf[c.i] = out[n] * 0.015 + c.store * 0.72;
          c.i = (c.i + 1) % c.buf.length;
          acc += y;
        }
        for (const a of passes) {
          const y = a.buf[a.i];
          a.buf[a.i] = acc + y * 0.5;
          a.i = (a.i + 1) % a.buf.length;
          acc = y - acc;
        }
        wet[n] = acc;
      }
      for (let n = 0; n < out.length; n++) out[n] += wet[n] * mix;
    },
  };
}

type Voices = ReturnType<typeof voices>;

// Contours carry the meaning: up = arrives or switches on, down = leaves or
// switches off; your own events are fuller than someone else's.
const RECIPES: Record<CueName, (out: Float32Array, v: Voices) => void> = {
  // A warm tone opens upward, then a chord blooms.
  joinSelf(out, { glide, filz, bell }) {
    glide(out, 0, hz(N.D3), hz(N.A3), {
      glide: 0.16,
      hold: 0.12,
      decay: 0.12,
      vel: 0.7,
    });
    filz(out, 0.15, N.F3, 0.45);
    filz(out, 0.17, N.A3, 0.45);
    filz(out, 0.19, N.D4, 0.5);
    bell(out, 0.21, hz(N.A4), 0.08, { decay: 0.6, index: 0.6 });
  },
  // Two bubbles rise.
  joinOther(out, { bubble }) {
    bubble(out, 0, 170, 260, 0.08, 1);
    bubble(out, 0.1, 230, 360, 0.09, 0.9);
  },
  // One bubble sinks, a soft wooden step after it.
  leaveOther(out, { bubble, holz }) {
    bubble(out, 0, 330, 190, 0.13, 1);
    holz(out, 0.12, N.D3, 0.35);
  },
  // The tone closes downward and a door falls shut.
  leaveSelf(out, { glide, muffle, knock }) {
    glide(out, 0, hz(N.A3), hz(N.D3), {
      glide: 0.22,
      hold: 0.14,
      decay: 0.1,
      vel: 0.8,
    });
    muffle(out, 0, 0.3, 1800, 160, 0.12, [hz(N.D3), hz(N.A3)]);
    knock(out, 0.24, 110, 55, 0.9, 0.09);
  },
  // A switch flips and the sound drops into felt.
  muteSelf(out, { tick, knock }) {
    tick(out, 0, 2600, 0.6, 11);
    knock(out, 0.004, 210, 140, 0.8, 0.05);
  },
  // The switch flips back and the tone lifts.
  unmuteSelf(out, { tick, glide }) {
    tick(out, 0, 3200, 0.6, 12);
    glide(out, 0.004, 150, 260, {
      glide: 0.05,
      decay: 0.06,
      vel: 0.8,
      attack: 0.003,
    });
  },
  // Someone else: only a light tap.
  muteOther(out, { tick, knock }) {
    tick(out, 0, 2200, 0.5, 13);
    knock(out, 0.003, 280, 210, 0.5, 0.025);
  },
  unmuteOther(out, { tick, glide }) {
    tick(out, 0, 2800, 0.5, 14);
    glide(out, 0.003, 210, 320, {
      glide: 0.04,
      decay: 0.035,
      vel: 0.5,
      attack: 0.003,
    });
  },
  // Hands over the ears: a chord closes into a low hum.
  deafen(out, { muffle, knock }) {
    muffle(out, 0, 0.3, 2600, 170, 0.5, [hz(N.D3), hz(N.A3), hz(N.D4)]);
    knock(out, 0.27, 90, 60, 0.5, 0.08);
  },
  // Hands off: the chord opens up and pops free.
  undeafen(out, { muffle, tick }) {
    muffle(out, 0, 0.26, 170, 2600, 0.5, [hz(N.D3), hz(N.A3), hz(N.D4)]);
    tick(out, 0.25, 3000, 0.4, 15);
  },
  // On air: air rises, three bells climb, one spark on top.
  liveStart(out, { whoosh, bell }) {
    whoosh(out, 0, 0.5, 140, 2200, 0.12);
    bell(out, 0.16, hz(N.D4), 0.5);
    bell(out, 0.25, hz(N.A4), 0.6);
    bell(out, 0.34, hz(N.D5), 0.75);
    bell(out, 0.42, hz(N.F5), 0.18, { decay: 0.4, index: 0.4 });
  },
  // Off air: the air falls, two bells step down, the power winds down.
  liveEnd(out, { whoosh, bell, glide }) {
    whoosh(out, 0, 0.45, 2200, 140, 0.1);
    bell(out, 0, hz(N.A4), 0.55);
    bell(out, 0.12, hz(N.D4), 0.5);
    glide(out, 0.06, 300, 100, {
      glide: 0.32,
      hold: 0.25,
      decay: 0.08,
      vel: 0.35,
      parts: [[1, 1]],
    });
  },
  // A tine that settles, answered a fifth up.
  message(out, { tine }) {
    tine(out, 0, hz(N.D4), 0.8);
    tine(out, 0.09, hz(N.A4), 0.7);
  },
  // Someone starts watching your Live: a small glint.
  viewerJoin(out, { bubble, bell }) {
    bubble(out, 0, 300, 420, 0.05, 0.5);
    bell(out, 0.04, hz(N.A4), 0.5, { decay: 0.35, ratio: 5, index: 0.5 });
  },
  viewerLeave(out, { bell, bubble }) {
    bell(out, 0, hz(N.F4), 0.45, { decay: 0.3, ratio: 5, index: 0.4 });
    bubble(out, 0.05, 380, 260, 0.06, 0.4);
  },
  // Shutter: two soft clicks around a short rising whir.
  cameraOn(out, { tick, whoosh }) {
    tick(out, 0, 1800, 0.6, 21);
    whoosh(out, 0.02, 0.12, 500, 1800, 0.25);
    tick(out, 0.06, 2600, 0.5, 22);
  },
  cameraOff(out, { whoosh, tick }) {
    whoosh(out, 0, 0.12, 1800, 500, 0.22);
    tick(out, 0.1, 1600, 0.6, 23);
  },
};

/**
 * One cue as mono samples at `sampleRate`, peaking at its level. The silent
 * tail is cut and the last 20 ms fade out, so no cue ends in a click.
 */
export function renderCue(name: CueName, sampleRate: number): Float32Array {
  const v = voices(sampleRate);
  const out = new Float32Array(Math.round(CANVAS_SECONDS * sampleRate));
  RECIPES[name](out, v);
  v.room(out, ROOM_MIX);
  let peak = 0;
  for (const value of out) peak = Math.max(peak, Math.abs(value));
  const scale = CUE_LEVELS[name] / (peak || 1);
  let end = out.length;
  while (end > 0 && Math.abs(out[end - 1] * scale) < 0.002) end--;
  const cut = out.subarray(
    0,
    Math.min(out.length, end + Math.round(0.01 * sampleRate)),
  );
  const fade = Math.round(0.02 * sampleRate);
  for (let i = 0; i < cut.length; i++)
    cut[i] *= scale * Math.min(1, (cut.length - i) / fade);
  return cut;
}
