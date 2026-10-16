// Who is speaking in the joined call, from audio levels the media stack
// already keeps: RTP audio levels (RFC 6464) of received microphones in a
// browser, the native core's playout and capture meters in the desktop app,
// and an analyser on the own send track. Levels never leave this page.

import { create } from "zustand";

import { invokeNative, isDesktopApp } from "./native/bridge.ts";
import type { MediaSender, ReceivedSource } from "./mediasoupConnection.ts";
import { useVoice, voiceMicrophones } from "./session.ts";

/** RMS share of full scale (about -34 dBFS) that counts as speech; the
 * readers below all report on that scale. */
export const SPEAKING_LEVEL = 0.02;
/** libwebrtc's stats `audioLevel` is the peak sample of about 100 ms, where
 * speech runs some three times its RMS. */
const PEAK_PER_RMS = 3;
/** Pauses between words shorter than this keep the indicator on. */
export const SPEAKING_HOLD_MS = 400;
/** A received level this old belongs to a source that went quiet (DTX). */
const STALE_MS = 500;
const POLL_MS = 120;
/** Each received level costs the native core a stats walk. */
const NATIVE_POLL_MS = 250;
/** The decoder's level (no audio-level header extension) costs a stats
 * walk per received microphone; it is read at most this often. */
const DECODER_LEVEL_MS = 360;
/** Polls in a row without a send-track level before the analyser takes
 * over for that track. */
const SENDER_MISSES = 3;
/** The native meters: min(100, round(RMS * 350)), see capture_dsp.h. */
const NATIVE_METER_SCALE = 350;

type SpeakingState = {
  /** `serverId/channelId` of the call the levels belong to. */
  call: string | null;
  users: Readonly<Record<string, true>>;
};

const NOBODY: Readonly<Record<string, true>> = Object.freeze({});

export const useSpeakingStore = create<SpeakingState>(() => ({
  call: null,
  users: NOBODY,
}));

const callKey = (serverId: string, channelId: string) =>
  `${serverId}/${channelId}`;

/** Everyone speaking in this channel, if it is the joined call. */
export function useSpeakingUsers(
  serverId: string | null | undefined,
  channelId: string | null | undefined,
): Readonly<Record<string, true>> {
  return useSpeakingStore((s) =>
    serverId && channelId && s.call === callKey(serverId, channelId)
      ? s.users
      : NOBODY,
  );
}

export function useSpeaking(
  serverId: string | null | undefined,
  channelId: string | null | undefined,
  userId: string,
): boolean {
  return useSpeakingStore(
    (s) =>
      Boolean(serverId && channelId) &&
      s.call === callKey(serverId!, channelId!) &&
      s.users[userId] === true,
  );
}

type Receiver = Pick<RTCRtpReceiver, "getStats"> & {
  getSynchronizationSources?: () => RTCRtpSynchronizationSource[];
};

/** A received microphone in the browser. */
export async function receiverLevel(
  receiver: Receiver,
  now: () => { epoch: number; page: number } = clocks,
): Promise<number | null> {
  let latest: RTCRtpSynchronizationSource | undefined;
  for (const source of receiver.getSynchronizationSources?.() ?? [])
    if (
      typeof source.audioLevel === "number" &&
      (!latest || source.timestamp > latest.timestamp)
    )
      latest = source;
  const { epoch, page } = now();
  if (latest) {
    // Browsers have reported this either on the epoch or on the page clock.
    const age = Math.min(
      Math.abs(epoch - latest.timestamp),
      Math.abs(page - latest.timestamp),
    );
    return age > STALE_MS ? 0 : (latest.audioLevel ?? 0);
  }
  // Without the audio-level header extension: the decoder's own level.
  const cached = decoderLevels.get(receiver);
  if (cached && page - cached.at < DECODER_LEVEL_MS) return cached.level;
  const report = await receiver.getStats();
  let level: number | null = null;
  report.forEach((entry: { type?: string; audioLevel?: unknown }) => {
    if (entry.type === "inbound-rtp" && typeof entry.audioLevel === "number")
      level = entry.audioLevel / PEAK_PER_RMS;
  });
  decoderLevels.set(receiver, { at: page, level });
  return level;
}

const decoderLevels = new WeakMap<
  object,
  { at: number; level: number | null }
>();

function clocks() {
  const page = performance.now();
  return { epoch: performance.timeOrigin + page, page };
}

type NativeConsumerLevel = {
  playoutLevel?: () => Promise<{ level: number; samples: number } | null>;
};

/** Samples each native consumer had played at the last poll. */
const playedSamples = new WeakMap<object, number>();

async function receivedLevel(source: ReceivedSource): Promise<number | null> {
  const consumer = source.consumer as unknown as NativeConsumerLevel;
  if (typeof consumer.playoutLevel === "function") {
    const meter = await consumer.playoutLevel();
    if (meter === null) return null;
    // The meter keeps its last window while playout pulls no audio.
    const moved = playedSamples.get(consumer) !== meter.samples;
    playedSamples.set(consumer, meter.samples);
    return moved ? meter.level / NATIVE_METER_SCALE : 0;
  }
  return source.rtpReceiver ? receiverLevel(source.rtpReceiver) : null;
}

/** The browser's level of the own send track, where it reports one. */
export async function senderLevel(
  sender: Pick<RTCRtpSender, "getStats">,
): Promise<number | null> {
  const report = await sender.getStats();
  let level: number | null = null;
  report.forEach(
    (entry: { type?: string; kind?: string; audioLevel?: unknown }) => {
      if (
        entry.type === "media-source" &&
        entry.kind === "audio" &&
        typeof entry.audioLevel === "number"
      )
        level = entry.audioLevel / PEAK_PER_RMS;
    },
  );
  return level;
}

/** RMS of the own send track where the browser reports no level (Firefox). */
class TrackMeter {
  private context: AudioContext | null = null;
  private node: MediaStreamAudioSourceNode | null = null;
  private analyser: AnalyserNode | null = null;
  private track: MediaStreamTrack | null = null;
  private samples: Float32Array<ArrayBuffer> | null = null;
  /** Removes the gesture listeners while they wait to resume the context. */
  private unlisten: (() => void) | null = null;

  level(track: MediaStreamTrack): number | null {
    if (track !== this.track) this.attach(track);
    const analyser = this.analyser;
    const samples = this.samples;
    if (this.context?.state === "suspended") this.resumeOnGesture();
    if (!analyser || !samples || this.context?.state !== "running") return null;
    analyser.getFloatTimeDomainData(samples);
    let sum = 0;
    for (const sample of samples) sum += sample * sample;
    return Math.sqrt(sum / samples.length);
  }

  private attach(track: MediaStreamTrack): void {
    this.node?.disconnect();
    this.node = null;
    this.track = track;
    if (typeof AudioContext === "undefined") return;
    try {
      if (!this.context) {
        this.context = new AudioContext();
        // Runs at once where the page may play (a click to join, or an
        // active microphone in Firefox); otherwise see resumeOnGesture.
        if (this.context.state === "suspended")
          void this.context.resume().catch(() => {});
      }
      this.analyser ??= this.context.createAnalyser();
      this.analyser.fftSize = 1024;
      this.samples ??= new Float32Array(this.analyser.fftSize);
      this.node = this.context.createMediaStreamSource(
        new MediaStream([track]),
      );
      // No destination: nothing is played back or sent from here.
      this.node.connect(this.analyser);
    } catch {
      this.node = null;
    }
  }

  /**
   * A context the autoplay policy holds back starts with the next click or
   * key press on the page, not with a resume() on every poll.
   */
  private resumeOnGesture(): void {
    if (this.unlisten || typeof window === "undefined") return;
    const resume = () => {
      this.unlisten?.();
      if (this.context?.state === "suspended")
        void this.context.resume().catch(() => {});
    };
    const options = { capture: true, passive: true } as const;
    window.addEventListener("pointerdown", resume, options);
    window.addEventListener("keydown", resume, options);
    this.unlisten = () => {
      window.removeEventListener("pointerdown", resume, options);
      window.removeEventListener("keydown", resume, options);
      this.unlisten = null;
    };
  }

  dispose(): void {
    this.unlisten?.();
    this.node?.disconnect();
    this.analyser?.disconnect();
    if (this.context && this.context.state !== "closed")
      void this.context.close().catch(() => {});
    this.context = null;
    this.node = null;
    this.analyser = null;
    this.track = null;
    this.samples = null;
  }
}

/** Polls while a call is joined; returns the stop for tests and HMR. */
export function watchSpeaking(): () => void {
  let timer: ReturnType<typeof setInterval> | null = null;
  let polling = false;
  /** Moves on with every leave and on stop: a poll from before is void. */
  let session = 0;
  let call: string | null = null;
  const lastLoud = new Map<string, number>();
  const meter = new TrackMeter();
  let nativeBlocks = -1;
  /** The send track the browser was last asked about, and how many polls
   * in a row it reported no level for it. */
  let senderTrack: MediaStreamTrack | null = null;
  let senderMisses = 0;

  async function selfLevel(sender: MediaSender | undefined, mine: number) {
    if (isDesktopApp()) {
      const levels = await invokeNative<{ processed: number; blocks: number }>(
        "media_audio_levels",
      );
      // A capture that delivers no blocks keeps its last meter reading.
      const moved = levels.blocks !== nativeBlocks;
      nativeBlocks = levels.blocks;
      return moved ? levels.processed / NATIVE_METER_SCALE : 0;
    }
    const track = sender?.track;
    if (!track || track.readyState !== "live") return null;
    if (track !== senderTrack) {
      // A new device: ask the browser again.
      senderTrack = track;
      senderMisses = 0;
    }
    // After SENDER_MISSES polls in a row without a browser level, this track
    // stays on the analyser until the send track changes (device switch).
    if (sender.rtpSender && senderMisses < SENDER_MISSES) {
      const level = await senderLevel(sender.rtpSender).catch(() => null);
      if (level !== null) {
        senderMisses = 0;
        return level;
      }
      senderMisses++;
    }
    return session === mine ? meter.level(track) : null;
  }

  async function poll() {
    const state = useVoice.getState();
    if (state.status !== "joined" || !state.serverId || !state.channelId)
      return;
    const current = callKey(state.serverId, state.channelId);
    const mine = session;
    const { self, sender, received } = voiceMicrophones();
    const levels = new Map<string, number>();
    const note = (user: string, level: number | null) => {
      if (level !== null)
        levels.set(user, Math.max(levels.get(user) ?? 0, level));
    };
    await Promise.all([
      self && !state.muted && !state.deafened
        ? selfLevel(sender, mine).then(
            (level) => note(self, level),
            () => undefined,
          )
        : undefined,
      ...received
        .filter(
          (source) =>
            source.owner !== self &&
            !source.consumer.closed &&
            !source.consumer.paused,
        )
        .map((source) =>
          receivedLevel(source).then(
            (level) => note(source.owner, level),
            () => undefined,
          ),
        ),
    ]);
    const after = useVoice.getState();
    if (
      session !== mine ||
      after.status !== "joined" ||
      after.serverId !== state.serverId ||
      after.channelId !== state.channelId
    )
      return;
    if (current !== call) {
      call = current;
      lastLoud.clear();
    }
    const now = performance.now();
    for (const [user, level] of levels)
      if (level >= SPEAKING_LEVEL) lastLoud.set(user, now);
    if (self && (after.muted || after.deafened)) lastLoud.delete(self);
    const users: Record<string, true> = {};
    for (const [user, at] of lastLoud) {
      if (now - at < SPEAKING_HOLD_MS) users[user] = true;
      else lastLoud.delete(user);
    }
    publish(current, users);
  }

  function tick() {
    if (
      polling ||
      (typeof document !== "undefined" && document.visibilityState === "hidden")
    )
      return;
    polling = true;
    void poll()
      .catch(() => undefined)
      .finally(() => {
        polling = false;
      });
  }

  function sync() {
    const state = useVoice.getState();
    if (state.status === "joined") {
      timer ??= setInterval(tick, isDesktopApp() ? NATIVE_POLL_MS : POLL_MS);
      return;
    }
    if (timer !== null) clearInterval(timer);
    timer = null;
    session++;
    call = null;
    lastLoud.clear();
    nativeBlocks = -1;
    senderTrack = null;
    senderMisses = 0;
    meter.dispose();
    publish(null, NOBODY);
  }

  const unsubscribe = useVoice.subscribe(sync);
  sync();
  return () => {
    session++;
    unsubscribe();
    if (timer !== null) clearInterval(timer);
    timer = null;
    meter.dispose();
    publish(null, NOBODY);
  };
}

function publish(call: string | null, users: Record<string, true>): void {
  const state = useSpeakingStore.getState();
  const same =
    state.call === call &&
    Object.keys(state.users).length === Object.keys(users).length &&
    Object.keys(users).every((user) => state.users[user]);
  if (same) return;
  useSpeakingStore.setState({
    call,
    users: Object.keys(users).length ? users : NOBODY,
  });
}
