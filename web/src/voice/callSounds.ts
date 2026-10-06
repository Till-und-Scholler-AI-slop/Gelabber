import { useMediaSettings, type MediaSettings } from "./settings.ts";

export type CallSound =
  "join" | "leave" | "mute" | "unmute" | "deafen" | "undeafen";
const notes: Record<CallSound, readonly number[]> = {
  join: [523.25, 783.99],
  leave: [783.99, 523.25],
  mute: [392],
  unmute: [659.25],
  deafen: [440, 349.23],
  undeafen: [349.23, 440],
};

/** Original, short sine chimes. PCM WAV keeps playback on the selected speaker,
 * including browsers without AudioContext.setSinkId. No microphone/media graph. */
export function callSoundWave(kind: CallSound | "silent"): Uint8Array {
  const frequencies = kind === "silent" ? [0] : notes[kind];
  const rate = 24000;
  const noteSamples = 2880; // 120 ms with silence between notes
  const count = noteSamples * frequencies.length;
  const bytes = new Uint8Array(44 + count * 2);
  const view = new DataView(bytes.buffer);
  const label = (offset: number, text: string) =>
    [...text].forEach((char, index) =>
      view.setUint8(offset + index, char.charCodeAt(0)),
    );
  label(0, "RIFF");
  view.setUint32(4, bytes.length - 8, true);
  label(8, "WAVEfmt ");
  view.setUint32(16, 16, true);
  view.setUint16(20, 1, true);
  view.setUint16(22, 1, true);
  view.setUint32(24, rate, true);
  view.setUint32(28, rate * 2, true);
  view.setUint16(32, 2, true);
  view.setUint16(34, 16, true);
  label(36, "data");
  view.setUint32(40, count * 2, true);
  for (let i = 0; i < count; i++) {
    const n = i % noteSamples;
    const t = n / rate;
    // Smooth attack and release avoid clicks; last 20 ms are silent.
    const envelope = Math.max(0, Math.min(t / 0.012, (0.1 - t) / 0.04, 1));
    const sample = Math.sin(
      2 * Math.PI * frequencies[Math.floor(i / noteSamples)] * t,
    );
    view.setInt16(
      44 + i * 2,
      Math.round(sample * envelope * 0.3 * 32767),
      true,
    );
  }
  return bytes;
}

const sources = new Map<string, string>();
function source(kind: CallSound | "silent"): string {
  let value = sources.get(kind);
  if (!value) {
    value = `data:audio/wav;base64,${btoa(String.fromCharCode(...callSoundWave(kind)))}`;
    sources.set(kind, value);
  }
  return value;
}

type SoundAudio = Pick<
  HTMLAudioElement,
  "src" | "volume" | "play" | "pause"
> & {
  setSinkId?: (id: string) => Promise<void>;
};

/** One reusable element, no backlog. A new cue replaces the previous one.
 * Failed autoplay/output selection must never interfere with the call. */
export class CallSoundPlayer {
  private audio: SoundAudio | null = null;
  private generation = 0;
  private deafened = false;
  private routing: Promise<void> = Promise.resolve();

  constructor(
    private readonly settings: () => MediaSettings,
    private readonly createAudio: () => SoundAudio | null,
  ) {}

  stop(): void {
    this.generation++;
    this.audio?.pause();
  }

  setDeafened(value: boolean): void {
    if (value !== this.deafened) this.stop();
    this.deafened = value;
  }

  unlock(): void {
    if (!this.settings().callSounds) return;
    this.stop();
    this.audio ??= this.createAudio();
    if (!this.audio) return;
    this.audio.src = source("silent");
    this.audio.volume = 0;
    void this.audio.play().catch(() => {});
  }

  play(kind: CallSound): void {
    this.stop();
    const settings = this.settings();
    if (
      !settings.callSounds ||
      !settings.callSoundVolume ||
      !settings.outputVolume
    )
      return;
    // Explicit mute/deafen button feedback remains audible; remote events do not.
    if (this.deafened && (kind === "join" || kind === "leave")) return;
    this.audio ??= this.createAudio();
    const audio = this.audio;
    if (!audio) return;
    const generation = this.generation;
    const play = () => {
      if (generation !== this.generation) return;
      const latest = this.settings();
      if (!latest.callSounds || !latest.callSoundVolume || !latest.outputVolume)
        return;
      audio.src = source(kind);
      audio.volume = latest.callSoundVolume * latest.outputVolume;
      void audio.play().catch(() => {});
    };
    if (audio.setSinkId) {
      // Serialize routing too: a late old sink promise must not override a new one.
      this.routing = this.routing
        .then(async () => {
          if (generation !== this.generation) return;
          await audio.setSinkId!(settings.audioOutputId);
          play();
        })
        .catch(() => {});
    } else {
      play();
    }
  }
}

const player = new CallSoundPlayer(useMediaSettings.getState, () =>
  typeof Audio === "undefined" ? null : new Audio(),
);
useMediaSettings.subscribe((next, prev) => {
  if (
    next.callSounds !== prev.callSounds ||
    next.callSoundVolume !== prev.callSoundVolume ||
    next.outputVolume !== prev.outputVolume ||
    next.audioOutputId !== prev.audioOutputId
  )
    player.stop();
});
export const playCallSound = (kind: CallSound): void => player.play(kind);
export const unlockCallSounds = (): void => player.unlock();
export const stopCallSounds = (): void => player.stop();
export const setCallSoundsDeafened = (value: boolean): void =>
  player.setDeafened(value);
