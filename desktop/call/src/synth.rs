//! The call cues, synthesized like web/src/voice/callSoundSynth.ts: every
//! event has its own small gesture from a few voices, rendered in code with
//! seeded noise so each render is the same sound.

use crate::Sound;

/// One cue of the web's set (`CueName`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Cue {
    JoinSelf,
    JoinOther,
    LeaveOther,
    LeaveSelf,
    MuteSelf,
    UnmuteSelf,
    MuteOther,
    UnmuteOther,
    Deafen,
    Undeafen,
    LiveStart,
    LiveEnd,
    Message,
    ViewerJoin,
    ViewerLeave,
    CameraOn,
    CameraOff,
}

impl Cue {
    pub(crate) fn of(sound: Sound) -> Self {
        match sound {
            Sound::JoinSelf => Self::JoinSelf,
            Sound::LeaveSelf => Self::LeaveSelf,
            Sound::JoinOther => Self::JoinOther,
            Sound::LeaveOther => Self::LeaveOther,
            Sound::Mute => Self::MuteSelf,
            Sound::Unmute => Self::UnmuteSelf,
            Sound::Deafen => Self::Deafen,
            Sound::Undeafen => Self::Undeafen,
            Sound::OtherMute => Self::MuteOther,
            Sound::OtherUnmute => Self::UnmuteOther,
            Sound::LiveStart => Self::LiveStart,
            Sound::LiveEnd => Self::LiveEnd,
            Sound::CameraOn => Self::CameraOn,
            Sound::CameraOff => Self::CameraOff,
            Sound::Message => Self::Message,
            Sound::ViewerJoin => Self::ViewerJoin,
            Sound::ViewerLeave => Self::ViewerLeave,
        }
    }

    /// `CUE_LEVELS`: target peak; someone else's events sit below your own.
    pub(crate) fn level(self) -> f64 {
        match self {
            Self::JoinSelf => 0.34,
            Self::JoinOther => 0.26,
            Self::LeaveOther => 0.24,
            Self::LeaveSelf => 0.32,
            Self::MuteSelf | Self::UnmuteSelf => 0.3,
            Self::MuteOther | Self::UnmuteOther => 0.14,
            Self::Deafen | Self::Undeafen => 0.3,
            Self::LiveStart => 0.32,
            Self::LiveEnd => 0.26,
            Self::Message => 0.18,
            Self::ViewerJoin => 0.18,
            Self::ViewerLeave => 0.15,
            Self::CameraOn => 0.24,
            Self::CameraOff => 0.22,
        }
    }

    /// The tones sit below 3.5 kHz and come out of 24 kHz unchanged; the
    /// camera's shutter is filtered noise and keeps its top at 48 kHz.
    pub(crate) fn rate(self) -> u32 {
        match self {
            Self::CameraOn | Self::CameraOff => 48_000,
            _ => 24_000,
        }
    }

    /// Events of the call rather than your own controls: a deafened call
    /// hears none of them (`REMOTE_CUES`).
    pub(crate) fn remote(self) -> bool {
        matches!(
            self,
            Self::JoinSelf
                | Self::JoinOther
                | Self::LeaveSelf
                | Self::LeaveOther
                | Self::MuteOther
                | Self::UnmuteOther
                | Self::ViewerJoin
                | Self::ViewerLeave
        )
    }
}

const ROOM_MIX: f64 = 0.12;
const CANVAS_SECONDS: f64 = 1.6;

/// mulberry32, as the web seeds its noise (32-bit integer arithmetic).
fn rng(seed: i32) -> impl FnMut() -> f64 {
    let mut seed = seed;
    move || {
        seed = seed.wrapping_add(0x6d2b_79f5);
        let s = seed as u32;
        let mut t = (s ^ (s >> 15)).wrapping_mul(1 | s);
        t = t.wrapping_add((t ^ (t >> 7)).wrapping_mul(61 | t)) ^ t;
        f64::from(t ^ (t >> 14)) / 4_294_967_296.0
    }
}

fn hz(midi: f64) -> f64 {
    440.0 * 2f64.powf((midi - 69.0) / 12.0)
}

const D3: f64 = 50.0;
const F3: f64 = 54.0;
const A3: f64 = 57.0;
const D4: f64 = 62.0;
const F4: f64 = 66.0;
const A4: f64 = 69.0;
const D5: f64 = 74.0;
const F5: f64 = 78.0;

/// Topology-preserving state-variable filter; the cutoff may change per sample.
struct Svf {
    ic1: f64,
    ic2: f64,
    sr: f64,
}

impl Svf {
    fn new(sr: f64) -> Self {
        Self {
            ic1: 0.0,
            ic2: 0.0,
            sr,
        }
    }

    fn run(&mut self, x: f64, fc: f64, q: f64, band: bool) -> f64 {
        let g = (std::f64::consts::PI * fc.min(self.sr * 0.45) / self.sr).tan();
        let k = 1.0 / q;
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;
        let v3 = x - self.ic2;
        let v1 = a1 * self.ic1 + a2 * v3;
        let v2 = self.ic2 + a2 * self.ic1 + a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        if band { v1 } else { v2 }
    }
}

struct Glide {
    glide: f64,
    hold: f64,
    decay: f64,
    attack: f64,
    vel: f64,
    parts: &'static [(f64, f64)],
}

impl Default for Glide {
    fn default() -> Self {
        Self {
            glide: 0.15,
            hold: 0.0,
            decay: 0.15,
            attack: 0.008,
            vel: 1.0,
            parts: &[(1.0, 1.0), (2.0, 0.25), (3.0, 0.08)],
        }
    }
}

struct Bell {
    decay: f64,
    ratio: f64,
    index: f64,
}

impl Default for Bell {
    fn default() -> Self {
        Self {
            decay: 0.5,
            ratio: 3.5,
            index: 1.0,
        }
    }
}

/// The voices at one sample rate, drawing into a Float32Array-like canvas.
struct Voices {
    sr: f64,
}

impl Voices {
    fn start(&self, out: &[f32], at: f64) -> usize {
        ((at * self.sr).round() as usize).min(out.len())
    }

    fn span(&self, out: &[f32], start: usize, seconds: f64) -> usize {
        (out.len() - start).min((seconds * self.sr).round() as usize)
    }

    fn partials(
        &self,
        out: &mut [f32],
        at: f64,
        f: f64,
        vel: f64,
        list: &[(f64, f64, f64)],
        attack: f64,
    ) {
        let start = self.start(out, at);
        let longest = list.iter().map(|p| p.2).fold(f64::MIN, f64::max);
        let len = self.span(out, start, longest * 7.0);
        for &(ratio, amp, decay) in list {
            let w = 2.0 * std::f64::consts::PI * f * ratio / self.sr;
            for i in 0..len {
                let i_f = i as f64;
                let rise = (i_f / (attack * self.sr)).min(1.0);
                let env = rise * rise * (3.0 - 2.0 * rise) * (-i_f / (decay * self.sr)).exp();
                add(out, start + i, vel * amp * env * (w * i_f).sin());
            }
        }
    }

    fn holz(&self, out: &mut [f32], at: f64, midi: f64, vel: f64) {
        self.partials(
            out,
            at,
            hz(midi),
            vel,
            &[(1.0, 1.0, 0.3), (3.93, 0.09, 0.04)],
            0.006,
        );
    }

    fn filz(&self, out: &mut [f32], at: f64, midi: f64, vel: f64) {
        self.partials(
            out,
            at,
            hz(midi),
            vel,
            &[
                (1.0, 1.0, 0.45),
                (1.003, 0.45, 0.4),
                (2.0, 0.22, 0.18),
                (3.0, 0.06, 0.08),
            ],
            0.012,
        );
    }

    fn whoosh(&self, out: &mut [f32], at: f64, dur: f64, from: f64, to: f64, gain: f64) {
        let start = self.start(out, at);
        let len = self.span(out, start, dur);
        let mut noise = rng((from + to).round() as i32);
        let mut bp = Svf::new(self.sr);
        for i in 0..len {
            let p = i as f64 / len as f64;
            let fc = from * (to / from).powf(p);
            let env = (std::f64::consts::PI * p).sin().powi(2);
            let x = noise() * 2.0 - 1.0;
            add(out, start + i, gain * env * bp.run(x, fc, 0.9, true));
        }
    }

    fn glide(&self, out: &mut [f32], at: f64, f0: f64, f1: f64, o: Glide) {
        let start = self.start(out, at);
        let len = self.span(out, start, o.hold + o.decay * 7.0);
        let mut phase = 0.0;
        for i in 0..len {
            let t = i as f64 / self.sr;
            let p = (t / o.glide).min(1.0);
            let f = f0 * (f1 / f0).powf(1.0 - (1.0 - p) * (1.0 - p));
            let env = (t / o.attack).min(1.0)
                * if t < o.hold {
                    1.0
                } else {
                    (-(t - o.hold) / o.decay).exp()
                };
            let s: f64 = o
                .parts
                .iter()
                .map(|(ratio, amp)| amp * (phase * ratio).sin())
                .sum();
            add(out, start + i, o.vel * env * s);
            phase += 2.0 * std::f64::consts::PI * f / self.sr;
        }
    }

    fn tick(&self, out: &mut [f32], at: f64, fc: f64, gain: f64, seed: i32) {
        let start = self.start(out, at);
        let len = self.span(out, start, 0.02);
        let mut noise = rng(seed);
        let mut bp = Svf::new(self.sr);
        for i in 0..len {
            let x = noise() * 2.0 - 1.0;
            let env = (-(i as f64) / (0.0025 * self.sr)).exp();
            add(out, start + i, gain * env * bp.run(x, fc, 1.2, true));
        }
    }

    fn knock(&self, out: &mut [f32], at: f64, f0: f64, f1: f64, gain: f64, decay: f64) {
        let start = self.start(out, at);
        let len = self.span(out, start, decay * 7.0);
        let mut phase: f64 = 0.0;
        for i in 0..len {
            let t = i as f64 / self.sr;
            let f = f1 + (f0 - f1) * (-t / 0.025).exp();
            add(
                out,
                start + i,
                gain * (t / 0.002).min(1.0) * (-t / decay).exp() * phase.sin(),
            );
            phase += 2.0 * std::f64::consts::PI * f / self.sr;
        }
    }

    fn bubble(&self, out: &mut [f32], at: f64, f0: f64, f1: f64, dur: f64, gain: f64) {
        let start = self.start(out, at);
        let len = self.span(out, start, dur * 5.0);
        let mut phase: f64 = 0.0;
        for i in 0..len {
            let t = i as f64 / self.sr;
            let f = f0 * (f1 / f0).powf((t / dur).min(1.0));
            add(
                out,
                start + i,
                gain * (t / 0.004).min(1.0)
                    * (-t / (dur * 0.9)).exp()
                    * (phase.sin() + 0.1 * (2.0 * phase).sin()),
            );
            phase += 2.0 * std::f64::consts::PI * f / self.sr;
        }
    }

    fn bell(&self, out: &mut [f32], at: f64, f: f64, vel: f64, o: Bell) {
        let start = self.start(out, at);
        let len = self.span(out, start, o.decay * 6.0);
        let wc = 2.0 * std::f64::consts::PI * f / self.sr;
        let wm = 2.0 * std::f64::consts::PI * f * o.ratio / self.sr;
        for i in 0..len {
            let i_f = i as f64;
            let env = (i_f / (0.004 * self.sr)).min(1.0) * (-i_f / (o.decay * self.sr)).exp();
            let modulation = o.index * (-i_f / (0.08 * self.sr)).exp() * (wm * i_f).sin();
            add(out, start + i, vel * env * (wc * i_f + modulation).sin());
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn muffle(
        &self,
        out: &mut [f32],
        at: f64,
        dur: f64,
        from: f64,
        to: f64,
        gain: f64,
        freqs: &[f64],
    ) {
        let start = self.start(out, at);
        let len = self.span(out, start, dur + 0.3);
        let mut lp = Svf::new(self.sr);
        let mut lp2 = Svf::new(self.sr);
        for i in 0..len {
            let t = i as f64 / self.sr;
            let fc = from * (to / from).powf((t / dur).min(1.0));
            let mut s = 0.0;
            for f in freqs {
                for h in 1..=8 {
                    let h = f64::from(h);
                    s += (2.0 * std::f64::consts::PI * f * h * t).sin() / h;
                }
            }
            let env = (t / 0.03).min(1.0)
                * if t < dur {
                    1.0
                } else {
                    (-(t - dur) / 0.06).exp()
                };
            let first = lp.run(s, fc, 0.9, false);
            add(out, start + i, gain * env * lp2.run(first, fc, 0.9, false));
        }
    }

    fn tine(&self, out: &mut [f32], at: f64, f: f64, vel: f64) {
        let start = self.start(out, at);
        let len = self.span(out, start, 0.35 * 6.0);
        let mut phase: f64 = 0.0;
        for i in 0..len {
            let t = i as f64 / self.sr;
            let env = (t / 0.003).min(1.0) * (-t / 0.35).exp();
            add(
                out,
                start + i,
                vel * env * (phase.sin() + 0.3 * (-t / 0.02).exp() * (phase * 5.4).sin()),
            );
            phase += 2.0 * std::f64::consts::PI * f * (1.0 - 0.025 * (-t / 0.03).exp()) / self.sr;
        }
    }

    /// Small Freeverb-style room, mono.
    fn room(&self, out: &mut [f32], mix: f64) {
        let s = self.sr / 44_100.0;
        let mut combs: Vec<(Vec<f32>, usize, f64)> = [
            1116.0, 1188.0, 1277.0, 1356.0, 1422.0, 1491.0, 1557.0, 1617.0,
        ]
        .iter()
        .map(|d: &f64| (vec![0.0f32; (d * s).round() as usize], 0, 0.0))
        .collect();
        let mut passes: Vec<(Vec<f32>, usize)> = [556.0, 441.0, 341.0, 225.0]
            .iter()
            .map(|d: &f64| (vec![0.0f32; (d * s).round() as usize], 0))
            .collect();
        let mut wet = vec![0.0f32; out.len()];
        for n in 0..out.len() {
            let mut acc = 0.0f64;
            for (buf, i, store) in combs.iter_mut() {
                let y = f64::from(buf[*i]);
                // `store` is a JS number (double), the buffers Float32Array.
                *store = y * 0.6 + *store * 0.4;
                buf[*i] = (f64::from(out[n]) * 0.015 + *store * 0.72) as f32;
                *i = (*i + 1) % buf.len();
                acc += y;
            }
            for (buf, i) in passes.iter_mut() {
                let y = f64::from(buf[*i]);
                buf[*i] = (acc + y * 0.5) as f32;
                *i = (*i + 1) % buf.len();
                acc = y - acc;
            }
            wet[n] = acc as f32;
        }
        for (sample, wet) in out.iter_mut().zip(wet) {
            *sample = (f64::from(*sample) + f64::from(wet) * mix) as f32;
        }
    }
}

fn add(out: &mut [f32], index: usize, value: f64) {
    if let Some(sample) = out.get_mut(index) {
        *sample = (f64::from(*sample) + value) as f32;
    }
}

fn recipe(cue: Cue, out: &mut [f32], v: &Voices) {
    match cue {
        Cue::JoinSelf => {
            let o = Glide {
                glide: 0.16,
                hold: 0.12,
                decay: 0.12,
                vel: 0.7,
                ..Glide::default()
            };
            v.glide(out, 0.0, hz(D3), hz(A3), o);
            v.filz(out, 0.15, F3, 0.45);
            v.filz(out, 0.17, A3, 0.45);
            v.filz(out, 0.19, D4, 0.5);
            let bell = Bell {
                decay: 0.6,
                index: 0.6,
                ..Bell::default()
            };
            v.bell(out, 0.21, hz(A4), 0.08, bell);
        }
        Cue::JoinOther => {
            v.bubble(out, 0.0, 170.0, 260.0, 0.08, 1.0);
            v.bubble(out, 0.1, 230.0, 360.0, 0.09, 0.9);
        }
        Cue::LeaveOther => {
            v.bubble(out, 0.0, 330.0, 190.0, 0.13, 1.0);
            v.holz(out, 0.12, D3, 0.35);
        }
        Cue::LeaveSelf => {
            let o = Glide {
                glide: 0.22,
                hold: 0.14,
                decay: 0.1,
                vel: 0.8,
                ..Glide::default()
            };
            v.glide(out, 0.0, hz(A3), hz(D3), o);
            v.muffle(out, 0.0, 0.3, 1800.0, 160.0, 0.12, &[hz(D3), hz(A3)]);
            v.knock(out, 0.24, 110.0, 55.0, 0.9, 0.09);
        }
        Cue::MuteSelf => {
            v.tick(out, 0.0, 2600.0, 0.6, 11);
            v.knock(out, 0.004, 210.0, 140.0, 0.8, 0.05);
        }
        Cue::UnmuteSelf => {
            v.tick(out, 0.0, 3200.0, 0.6, 12);
            let o = Glide {
                glide: 0.05,
                decay: 0.06,
                vel: 0.8,
                attack: 0.003,
                ..Glide::default()
            };
            v.glide(out, 0.004, 150.0, 260.0, o);
        }
        Cue::MuteOther => {
            v.tick(out, 0.0, 2200.0, 0.5, 13);
            v.knock(out, 0.003, 280.0, 210.0, 0.5, 0.025);
        }
        Cue::UnmuteOther => {
            v.tick(out, 0.0, 2800.0, 0.5, 14);
            let o = Glide {
                glide: 0.04,
                decay: 0.035,
                vel: 0.5,
                attack: 0.003,
                ..Glide::default()
            };
            v.glide(out, 0.003, 210.0, 320.0, o);
        }
        Cue::Deafen => {
            v.muffle(out, 0.0, 0.3, 2600.0, 170.0, 0.5, &[hz(D3), hz(A3), hz(D4)]);
            v.knock(out, 0.27, 90.0, 60.0, 0.5, 0.08);
        }
        Cue::Undeafen => {
            v.muffle(
                out,
                0.0,
                0.26,
                170.0,
                2600.0,
                0.5,
                &[hz(D3), hz(A3), hz(D4)],
            );
            v.tick(out, 0.25, 3000.0, 0.4, 15);
        }
        Cue::LiveStart => {
            v.whoosh(out, 0.0, 0.5, 140.0, 2200.0, 0.12);
            v.bell(out, 0.16, hz(D4), 0.5, Bell::default());
            v.bell(out, 0.25, hz(A4), 0.6, Bell::default());
            v.bell(out, 0.34, hz(D5), 0.75, Bell::default());
            let spark = Bell {
                decay: 0.4,
                index: 0.4,
                ..Bell::default()
            };
            v.bell(out, 0.42, hz(F5), 0.18, spark);
        }
        Cue::LiveEnd => {
            v.whoosh(out, 0.0, 0.45, 2200.0, 140.0, 0.1);
            v.bell(out, 0.0, hz(A4), 0.55, Bell::default());
            v.bell(out, 0.12, hz(D4), 0.5, Bell::default());
            let o = Glide {
                glide: 0.32,
                hold: 0.25,
                decay: 0.08,
                vel: 0.35,
                parts: &[(1.0, 1.0)],
                ..Glide::default()
            };
            v.glide(out, 0.06, 300.0, 100.0, o);
        }
        Cue::Message => {
            v.tine(out, 0.0, hz(D4), 0.8);
            v.tine(out, 0.09, hz(A4), 0.7);
        }
        Cue::ViewerJoin => {
            v.bubble(out, 0.0, 300.0, 420.0, 0.05, 0.5);
            let glint = Bell {
                decay: 0.35,
                ratio: 5.0,
                index: 0.5,
            };
            v.bell(out, 0.04, hz(A4), 0.5, glint);
        }
        Cue::ViewerLeave => {
            let glint = Bell {
                decay: 0.3,
                ratio: 5.0,
                index: 0.4,
            };
            v.bell(out, 0.0, hz(F4), 0.45, glint);
            v.bubble(out, 0.05, 380.0, 260.0, 0.06, 0.4);
        }
        Cue::CameraOn => {
            v.tick(out, 0.0, 1800.0, 0.6, 21);
            v.whoosh(out, 0.02, 0.12, 500.0, 1800.0, 0.25);
            v.tick(out, 0.06, 2600.0, 0.5, 22);
        }
        Cue::CameraOff => {
            v.whoosh(out, 0.0, 0.12, 1800.0, 500.0, 0.22);
            v.tick(out, 0.1, 1600.0, 0.6, 23);
        }
    }
}

/// One cue as mono samples at `rate`, peaking at its level, the silent tail
/// cut and the last 20 ms faded (`renderCue`).
pub(crate) fn render(cue: Cue, rate: u32) -> Vec<f32> {
    let voices = Voices {
        sr: f64::from(rate),
    };
    let mut out = vec![0.0f32; (CANVAS_SECONDS * f64::from(rate)).round() as usize];
    recipe(cue, &mut out, &voices);
    voices.room(&mut out, ROOM_MIX);
    let peak = out.iter().fold(0.0f64, |p, v| p.max(f64::from(v.abs())));
    let scale = cue.level() / if peak == 0.0 { 1.0 } else { peak };
    let mut end = out.len();
    while end > 0 && (f64::from(out[end - 1]) * scale).abs() < 0.002 {
        end -= 1;
    }
    let cut = out
        .len()
        .min(end + (0.01 * f64::from(rate)).round() as usize);
    out.truncate(cut);
    let fade = (0.02 * f64::from(rate)).round();
    let len = out.len();
    for (i, sample) in out.iter_mut().enumerate() {
        *sample = (f64::from(*sample) * scale * (((len - i) as f64) / fade).min(1.0)) as f32;
    }
    out
}

/// 16-bit mono PCM WAV of `samples` at `rate`, scaled by `volume`
/// (`callSoundWave`; the element volume of the web is applied here).
pub(crate) fn wave(samples: &[f32], rate: u32, volume: f64) -> Vec<u8> {
    let data = (samples.len() * 2) as u32;
    let mut bytes = Vec::with_capacity(44 + data as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&rate.to_le_bytes());
    bytes.extend_from_slice(&(rate * 2).to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data.to_le_bytes());
    for sample in samples {
        let value = (f64::from(*sample) * volume).clamp(-1.0, 1.0);
        bytes.extend_from_slice(&((value * 32767.0).round() as i16).to_le_bytes());
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Cue; 17] = [
        Cue::JoinSelf,
        Cue::JoinOther,
        Cue::LeaveOther,
        Cue::LeaveSelf,
        Cue::MuteSelf,
        Cue::UnmuteSelf,
        Cue::MuteOther,
        Cue::UnmuteOther,
        Cue::Deafen,
        Cue::Undeafen,
        Cue::LiveStart,
        Cue::LiveEnd,
        Cue::Message,
        Cue::ViewerJoin,
        Cue::ViewerLeave,
        Cue::CameraOn,
        Cue::CameraOff,
    ];

    #[test]
    fn rng_matches_mulberry32() {
        // First values of the web's rng, computed with callSoundSynth.ts.
        let mut noise = rng(11);
        assert_eq!(noise(), 0.5115870486479253);
        assert_eq!(noise(), 0.5299464082345366);
        assert_eq!(noise(), 0.6081185641232878);
        assert_eq!(rng(2340)(), 0.2975308415479958);
    }

    #[test]
    fn every_cue_peaks_at_its_level_and_ends_quietly() {
        for cue in ALL {
            let samples = render(cue, cue.rate());
            let peak = samples.iter().fold(0.0f32, |p, v| p.max(v.abs()));
            assert!(
                (f64::from(peak) - cue.level()).abs() < 0.01,
                "{cue:?} peaks at {peak}"
            );
            assert!(samples.len() < (1.6 * f64::from(cue.rate())) as usize + 1);
            assert!(
                samples.last().unwrap().abs() < 0.01,
                "{cue:?} ends in a click"
            );
        }
    }

    /// Length, a weighted sum and one sample of the web's `renderCue`
    /// (node --experimental-strip-types on callSoundSynth.ts).
    #[test]
    fn renders_match_the_web() {
        let cases = [
            (Cue::JoinSelf, 24_000, 38_400, 476.192166, -0.11680336),
            (Cue::CameraOn, 48_000, 10_459, -8.140002, 0.00000521),
            (Cue::LiveEnd, 24_000, 38_400, -12.112664, 0.05399635),
            (Cue::Deafen, 24_000, 13_766, 131.360782, 0.17615585),
        ];
        for (cue, rate, len, sum, at_1000) in cases {
            let samples = render(cue, rate);
            assert_eq!(samples.len(), len, "{cue:?} length");
            let weighted: f64 = samples
                .iter()
                .enumerate()
                .map(|(i, s)| f64::from(*s) * ((i % 97) + 1) as f64)
                .sum();
            assert!(
                (weighted - sum).abs() < 1e-3,
                "{cue:?}: {weighted} vs {sum}"
            );
            assert!(
                (f64::from(samples[1000]) - at_1000).abs() < 1e-7,
                "{cue:?} sample"
            );
        }
    }

    #[test]
    fn renders_are_the_same_sound() {
        assert_eq!(render(Cue::CameraOn, 48_000), render(Cue::CameraOn, 48_000));
    }

    #[test]
    fn wave_has_a_riff_header() {
        let bytes = wave(&[0.0, 0.5, -1.0], 24_000, 1.0);
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..16], b"WAVEfmt ");
        assert_eq!(bytes.len(), 44 + 6);
        assert_eq!(i16::from_le_bytes([bytes[46], bytes[47]]), 16384);
        assert_eq!(i16::from_le_bytes([bytes[48], bytes[49]]), -32767);
    }
}
