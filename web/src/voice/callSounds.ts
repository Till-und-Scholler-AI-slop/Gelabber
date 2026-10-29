import { renderCue, type CueName } from "./callSoundSynth.ts";
import { isDesktopApp } from "./native/bridge.ts";
import { useMediaSettings, type MediaSettings } from "./settings.ts";

/** A call cue: every call event has its own (callSoundSynth.ts). */
export type CallSound = Exclude<CueName, "message">;
/** Plus a message notification on a desktop (issue #247). */
type Cue = CueName;

/** Arriving, leaving and what the others do are the call's events, also
 * your own arrival: a deafened call hears none of them. Your own controls
 * (microphone, deafen, camera, Live) stay audible. */
const REMOTE_CUES: ReadonlySet<Cue> = new Set<Cue>([
  "joinSelf",
  "joinOther",
  "leaveSelf",
  "leaveOther",
  "muteOther",
  "unmuteOther",
  "viewerJoin",
  "viewerLeave",
]);

/** The tones sit below 3.5 kHz and come out of 24 kHz unchanged, at half the
 * work. The camera's shutter is filtered noise and keeps its top at 48 kHz. */
function rateOf(kind: Cue): number {
  return kind === "cameraOn" || kind === "cameraOff" ? 48000 : 24000;
}

/** 16-bit mono PCM WAV. A WAV keeps playback on the selected speaker,
 * including browsers without AudioContext.setSinkId. No microphone/media
 * graph. "silent" is 120 ms of nothing, to unlock playback. */
export function callSoundWave(kind: Cue | "silent"): Uint8Array {
  const rate = kind === "silent" ? 24000 : rateOf(kind);
  const samples =
    kind === "silent" ? new Float32Array(2880) : renderCue(kind, rate);
  const bytes = new Uint8Array(44 + samples.length * 2);
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
  view.setUint32(40, samples.length * 2, true);
  for (let i = 0; i < samples.length; i++)
    view.setInt16(
      44 + i * 2,
      Math.round(Math.max(-1, Math.min(1, samples[i])) * 32767),
      true,
    );
  return bytes;
}

/** A WAV as a `data:` URI. In chunks: Safari takes at most 65536 arguments
 * to one call, and a cue has more bytes than that. */
export function callSoundSource(bytes: Uint8Array): string {
  let text = "";
  for (let i = 0; i < bytes.length; i += 0x8000)
    text += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  return `data:audio/wav;base64,${btoa(text)}`;
}

// Rendered on first use (a few milliseconds each), then kept.
const sources = new Map<string, string>();
function source(kind: Cue | "silent"): string {
  let value = sources.get(kind);
  if (!value) {
    value = callSoundSource(callSoundWave(kind));
    sources.set(kind, value);
  }
  return value;
}

function wanted(settings: MediaSettings, kind: Cue): boolean {
  return (
    settings.callSounds &&
    settings.callSoundVolume > 0 &&
    settings.outputVolume > 0 &&
    (settings.otherMuteSounds ||
      (kind !== "muteOther" && kind !== "unmuteOther"))
  );
}

type SoundAudio = Pick<
  HTMLAudioElement,
  "src" | "volume" | "play" | "pause"
> & {
  setSinkId?: (id: string) => Promise<void>;
  onended?: ((event: Event) => unknown) | null;
  removeAttribute?: (name: string) => void;
  load?: () => void;
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
    this.release();
  }

  /** A finished or stopped cue gives its source up, so no idle element
   * keeps the tab marked as playing audio. */
  private release(): void {
    const audio = this.audio;
    if (!audio?.src) return;
    audio.removeAttribute?.("src");
    audio.load?.();
  }

  private start(audio: SoundAudio, src: string, volume: number): void {
    const generation = this.generation;
    audio.onended = () => {
      if (generation === this.generation) this.release();
    };
    audio.src = src;
    audio.volume = volume;
    void audio.play().catch(() => {});
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
    this.start(this.audio, source("silent"), 0);
  }

  /** `remote`: an event of the call, not your own control; by default
   * as REMOTE_CUES says. A cue that does not play leaves the current one. */
  play(kind: Cue, { remote = REMOTE_CUES.has(kind) } = {}): void {
    if (!wanted(this.settings(), kind) || (this.deafened && remote)) return;
    this.stop();
    const settings = this.settings();
    this.audio ??= this.createAudio();
    const audio = this.audio;
    if (!audio) return;
    const generation = this.generation;
    const play = () => {
      if (generation !== this.generation) return;
      const latest = this.settings();
      if (!wanted(latest, kind)) return;
      this.start(
        audio,
        source(kind),
        // Playback above 100 % boosts calls only, not the cues.
        latest.callSoundVolume * Math.min(1, latest.outputVolume),
      );
    };
    // The desktop app's output ids belong to its native engine. The webview
    // does not know them and would reject them, so cues stay on its default
    // output there.
    if (audio.setSinkId && !isDesktopApp()) {
      // Serialize routing too: a late old sink promise must not override a new one.
      this.routing = this.routing
        .then(async () => {
          if (generation !== this.generation) return;
          try {
            await audio.setSinkId!(settings.audioOutputId);
          } catch {
            // An unplugged or refused speaker must not cost the cue: it plays
            // where the element already is.
          }
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
export const playCallSound = (
  kind: CallSound,
  options?: { remote?: boolean },
): void => player.play(kind, options);

// Its own element and switch: a message cue neither cuts off a call cue nor
// plays when only call sounds are on. Same volume as the call cues.
const messagePlayer = new CallSoundPlayer(
  () => {
    const settings = useMediaSettings.getState();
    return { ...settings, callSounds: settings.messageSound };
  },
  () => (typeof Audio === "undefined" ? null : new Audio()),
);
useMediaSettings.subscribe((next, prev) => {
  if (
    next.messageSound !== prev.messageSound ||
    next.callSoundVolume !== prev.callSoundVolume ||
    next.outputVolume !== prev.outputVolume ||
    next.audioOutputId !== prev.audioOutputId
  )
    messagePlayer.stop();
});
export const playMessageSound = (): void => messagePlayer.play("message");
/** Someone went live elsewhere (liveNotices.ts): the Live cue, under the
 * message cue's switch. */
export const playLiveSound = (): void => messagePlayer.play("liveStart");
export const unlockCallSounds = (): void => player.unlock();
export const stopCallSounds = (): void => player.stop();
export const setCallSoundsDeafened = (value: boolean): void =>
  player.setDeafened(value);
