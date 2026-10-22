// What the call plays: one element per received microphone, so each
// participant keeps a volume of their own, and one per watched Live's sound.
// Volumes, the output device and the browser's autoplay block live here.

import type { AudioSource } from "./diagnostics.ts";
import type { ReceivedSource } from "./mediasoupConnection.ts";
import { isDesktopApp } from "./native/bridge.ts";
import {
  createAudioOutput,
  createStream,
  isNativeAudioOutput,
} from "./native/tracks.ts";
import {
  boost,
  boostOutput,
  isBoosted,
  releaseBoost,
  startBoostOutput,
} from "./playbackBoost.ts";
import { useMediaSettings } from "./settings.ts";
import {
  effectiveVoiceVolume,
  userVolumeOf,
  useUserVolumes,
} from "./userVolume.ts";
import { useVoice } from "./voiceStore.ts";

type VoiceAudio = { el: HTMLAudioElement; userId: string | null };
type SourceKind = "s" | "l";

export class ReceivedAudio {
  private readonly voiceAudio = new Map<MediaStreamTrack, VoiceAudio>();
  private readonly receivedAudioSources = new Map<
    MediaStreamTrack,
    AudioSource
  >();
  private readonly receivedSourceAudio = new Map<
    MediaStreamTrack,
    { stream?: MediaStream; userId: string; kind: SourceKind }
  >();
  private readonly sourceAudio = new Map<
    MediaStreamTrack,
    { el: HTMLAudioElement; userId: string; kind: SourceKind }
  >();
  private readonly blockedPlayback = new Set<HTMLAudioElement>();
  /** The browser blocked the boost output: play at 100 % until a click. */
  private boostBlocked = false;

  constructor(
    private readonly hooks: {
      /** Who sent a received track, and as what. */
      identity: (track: MediaStreamTrack) => ReceivedSource | undefined;
      self: () => string | null;
      /** A received microphone started playing. */
      voiceAttached: () => void;
    },
  ) {}

  /** The browser refused to play something: the call bar offers a click. */
  get playbackBlocked(): boolean {
    return this.blockedPlayback.size > 0;
  }

  /** A received audio track: a Live's sound if it is one, else a microphone. */
  attachIncoming(track: MediaStreamTrack, stream?: MediaStream): void {
    if (this.attachSourceAudio(track, stream)) return;
    this.forgetSourceAudioReceiver(track);
    this.receivedAudioSources.set(track, "voice");
    this.attachVoiceAudio(track, stream);
  }

  /** The SDK closed a received track. */
  close(track: MediaStreamTrack, audio: boolean): void {
    this.receivedAudioSources.delete(track);
    this.receivedSourceAudio.delete(track);
    if (!audio) return;
    this.dropVoiceAudio(track);
    const held = this.sourceAudio.get(track);
    if (held) {
      this.silenceAudio(held.el);
      this.sourceAudio.delete(track);
    }
    this.updatePlaybackBlocked();
  }

  /** Everything received ends with the seat's connection. */
  clear(): void {
    this.clearSourcePlayback();
    this.receivedSourceAudio.clear();
    this.receivedAudioSources.clear();
    // Paused and detached: an element left playing keeps the tab's
    // "playing audio" indicator after the call is over.
    this.clearVoiceAudio();
    this.updatePlaybackBlocked();
  }

  /** A publisher's Live sound ended. */
  dropSource(userId: string, kind: SourceKind): void {
    this.clearSourcePlayback(userId, kind);
    for (const [track, source] of this.receivedSourceAudio)
      if (source.userId === userId && source.kind === kind)
        this.receivedSourceAudio.delete(track);
  }

  /** Watching a publisher's Live again plays the sound that still arrives. */
  resumeSource(userId: string, kind: SourceKind): void {
    for (const [track, received] of this.receivedSourceAudio) {
      if (
        received.userId === userId &&
        received.kind === kind &&
        track.readyState !== "ended"
      )
        this.attachSourceAudio(track, received.stream);
    }
  }

  clearSourcePlayback(userId?: string, kind?: SourceKind): void {
    for (const [track, entry] of this.sourceAudio) {
      if ((userId && entry.userId !== userId) || (kind && entry.kind !== kind))
        continue;
      this.silenceAudio(entry.el);
      this.sourceAudio.delete(track);
    }
    this.updatePlaybackBlocked();
  }

  /** What each received audio track is, for the diagnostics. */
  diagnosticSources(sources: Record<string, AudioSource>): void {
    for (const [track, source] of this.receivedAudioSources)
      sources[track.id] = source;
    for (const [track, source] of this.sourceAudio)
      sources[track.id] = source.kind === "s" ? "screen-audio" : "live-audio";
  }

  /** Call synchronously from a click so the browser grants playback activation. */
  retry(): void {
    // The click also allows the boost again; applyPlayback restarts it.
    this.boostBlocked = false;
    this.applyPlayback();
    for (const { el } of [
      ...this.voiceAudio.values(),
      ...this.sourceAudio.values(),
    ]) {
      if (el?.srcObject) this.playAudio(el);
    }
  }

  applyPlayback(): void {
    const deafened = useVoice.getState().deafened;
    const settings = useMediaSettings.getState();
    this.applyVoiceVolumes();
    for (const { el } of this.voiceAudio.values()) void this.applySink(el);
    for (const { el } of this.sourceAudio.values()) {
      this.setOutputVolume(
        el,
        deafened || settings.sourceAudioMuted,
        deafened ? 0 : settings.sourceAudioVolume,
      );
      void this.applySink(el);
    }
    const boosted = boostOutput();
    if (boosted) void this.applySink(boosted);
  }

  /** A per-user slider moves volumes only: setting the same output device
   * again is not free in every browser (a short dropout). Only a boost output
   * the slider just created still needs it. */
  applyUserVolumes(): void {
    const before = boostOutput();
    this.applyVoiceVolumes();
    const boosted = boostOutput();
    if (boosted && boosted !== before) void this.applySink(boosted);
  }

  /** Plays a received microphone on an element of its own. */
  private attachVoiceAudio(
    track: MediaStreamTrack,
    stream?: MediaStream,
  ): void {
    const held = this.voiceAudio;
    if (held.has(track)) return;
    const el = createAudioOutput();
    if (!el) return;
    el.autoplay = true;
    el.setAttribute("playsinline", "true");
    // One element per track: a MediaStream's multiple audio tracks must not
    // compete for an HTML media element's single selected audio track.
    el.srcObject = createStream([track]) ?? stream ?? null;
    const userId = this.hooks.identity(track)?.owner ?? null;
    if (userId) el.setAttribute("data-voice", userId);
    held.set(track, { el, userId });
    track.addEventListener("ended", () => {
      if (held.get(track)?.el !== el) return;
      held.delete(track);
      this.silenceAudio(el);
      this.updatePlaybackBlocked();
    });
    this.hooks.voiceAttached();
    this.applyPlayback();
    this.playAudio(el);
  }

  private dropVoiceAudio(track: MediaStreamTrack): void {
    const voice = this.voiceAudio.get(track);
    if (!voice) return;
    this.silenceAudio(voice.el);
    this.voiceAudio.delete(track);
  }

  private clearVoiceAudio(): void {
    for (const { el } of this.voiceAudio.values()) this.silenceAudio(el);
    this.voiceAudio.clear();
  }

  private attachSourceAudio(
    track: MediaStreamTrack,
    stream: MediaStream | undefined,
  ): boolean {
    const identity = this.hooks.identity(track);
    const parsed = identity ? { userId: identity.owner, k: identity.k } : null;
    if (!parsed || (parsed.k !== "sa" && parsed.k !== "la")) return false;
    this.dropVoiceAudio(track);
    const state = useVoice.getState();
    // An older client's Go Live sound (`la`) plays like a Live's.
    const kind = parsed.k === "sa" ? "s" : "l";
    this.receivedAudioSources.set(
      track,
      kind === "s" ? "screen-audio" : "live-audio",
    );
    this.receivedSourceAudio.set(track, {
      stream,
      userId: parsed.userId,
      kind,
    });
    const held = this.sourceAudio.get(track);
    const sameSource =
      held?.userId === parsed.userId &&
      held.kind === kind &&
      held.el.srcObject === stream;
    if (held && !sameSource) this.clearSourcePlayback(held.userId, held.kind);
    const allowed = state.sourceSubscriptions[parsed.userId]?.[kind];
    if (
      !allowed ||
      parsed.userId === this.hooks.self() ||
      (typeof Audio === "undefined" && !isDesktopApp())
    )
      return true;
    if (sameSource) return true;
    this.clearSourcePlayback(parsed.userId, kind);
    for (const [previous, source] of this.receivedSourceAudio) {
      if (
        previous !== track &&
        source.userId === parsed.userId &&
        source.kind === kind
      )
        this.receivedSourceAudio.delete(previous);
    }
    const el = createAudioOutput();
    if (!el) return true;
    el.autoplay = true;
    el.setAttribute("playsinline", "true");
    el.srcObject = stream ?? createStream([track]);
    el.setAttribute("data-source-audio", kind);
    el.setAttribute("data-publisher", parsed.userId);
    this.sourceAudio.set(track, { el, userId: parsed.userId, kind });
    track.addEventListener("ended", () => {
      if (this.sourceAudio.get(track)?.el !== el) return;
      this.receivedSourceAudio.delete(track);
      this.receivedAudioSources.delete(track);
      this.sourceAudio.delete(track);
      this.silenceAudio(el);
      this.updatePlaybackBlocked();
    });
    this.applyPlayback();
    this.playAudio(el);
    return true;
  }

  private forgetSourceAudioReceiver(track: MediaStreamTrack): void {
    const held = this.sourceAudio.get(track);
    if (held) this.clearSourcePlayback(held.userId, held.kind);
    this.receivedSourceAudio.delete(track);
  }

  private updatePlaybackBlocked(): void {
    useVoice.setState({ playbackBlocked: this.blockedPlayback.size > 0 });
  }

  private playAudio(el: HTMLAudioElement): void {
    const stream = el.srcObject;
    if (!stream) return;
    const boosted = isBoosted(el);
    const failed = (error: unknown) => {
      if (
        el.srcObject !== stream ||
        (error as { name?: string })?.name === "AbortError"
      )
        return;
      this.blockedPlayback.add(el);
      if (boosted && isBoosted(el)) {
        // The muted element would stay silent: play it at 100 % meanwhile.
        this.boostBlocked = true;
        this.applyPlayback();
      }
      this.updatePlaybackBlocked();
    };
    try {
      const started = el.play();
      // A boosted element is heard through the shared boost output instead.
      void (boosted ? Promise.all([started, startBoostOutput()]) : started)
        ?.then(() => {
          if (el.srcObject !== stream) return;
          this.blockedPlayback.delete(el);
          this.updatePlaybackBlocked();
        })
        .catch(failed);
    } catch (error) {
      failed(error);
    }
  }

  /** Each microphone at the output volume times its participant's own. */
  private applyVoiceVolumes(): void {
    const deafened = useVoice.getState().deafened;
    const output = useMediaSettings.getState().outputVolume;
    const users = useUserVolumes.getState().users;
    for (const { el, userId } of this.voiceAudio.values()) {
      const user = userVolumeOf(userId, users);
      this.setOutputVolume(
        el,
        deafened || user.muted,
        effectiveVoiceVolume({ output, user, deafened }),
      );
    }
  }

  /** Up to 100 % on the element itself, above that through the Web Audio
   * boost. Native outputs scale up to 400 % on their own. */
  private setOutputVolume(
    el: HTMLAudioElement,
    muted: boolean,
    volume: number,
  ): void {
    if (
      !muted &&
      volume > 1 &&
      !this.boostBlocked &&
      !isNativeAudioOutput(el)
    ) {
      const wasBoosted = isBoosted(el);
      if (boost(el, volume)) {
        // Still playing, so Chromium keeps feeding the boost; heard through it.
        el.muted = true;
        el.volume = 1;
        if (!wasBoosted) this.playAudio(el);
        return;
      }
    }
    releaseBoost(el);
    el.muted = muted;
    el.volume = isNativeAudioOutput(el) ? volume : Math.min(1, volume);
  }

  /** Stops an element the session no longer plays. */
  private silenceAudio(el: HTMLAudioElement): void {
    el.pause?.();
    el.srcObject = null;
    releaseBoost(el);
    this.blockedPlayback.delete(el);
  }

  private async applySink(el: HTMLAudioElement): Promise<void> {
    const id = useMediaSettings.getState().audioOutputId;
    const sink = el as HTMLAudioElement & {
      setSinkId?: (deviceId: string) => Promise<void>;
    };
    if (!sink.setSinkId) return;
    try {
      await sink.setSinkId(id);
    } catch {
      // unplugged / permission
    }
  }
}
