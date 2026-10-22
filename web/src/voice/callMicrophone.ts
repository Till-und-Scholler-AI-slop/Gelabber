// The own microphone: capture with its processor, the switch to another
// device or gain, losing it mid-call and getting it back, and its sender.

import {
  captureMicrophone,
  createProcessor,
  noteAudioProcessing,
  useAudioProcessing,
  type MicProcessor,
} from "./audioProcessing.ts";
import type { MediaConnection } from "./mediasoupConnection.ts";
import { noteDiagnosticEvent } from "./diagnostics.ts";
import { useMediaSettings } from "./settings.ts";
import { useVoice } from "./voiceStore.ts";
import {
  applyLocalAudio,
  applySendBitrate,
  captureEpoch,
  currentUserId,
  defaultGetUserMedia,
  hasLiveTrack,
  hintTrack,
  logVoice,
  type RtpSender,
  setPub,
  stopTracks,
  syncMicState,
  voice,
} from "./sessionState.ts";

export function disposeMicGain(): void {
  const insert = voice.activeMicGain;
  voice.activeMicGain = null;
  insert?.dispose();
  noteAudioProcessing(null);
}

export function enqueueAudioCommit(job: () => Promise<void>): Promise<void> {
  const run = voice.audioCommitChain.then(job, job);
  voice.audioCommitChain = run.then(
    () => undefined,
    () => undefined,
  );
  return run;
}

export function disableAudio(stream: MediaStream): void {
  stream.getAudioTracks().forEach((track) => {
    track.enabled = false;
  });
}

function micCurrent(
  pc: MediaConnection,
  session: number,
  request: number,
): boolean {
  return (
    voice.seat.connection === pc &&
    voice.seat.generation === session &&
    voice.micEpoch === request &&
    useVoice.getState().status === "joined"
  );
}

async function replaceSenderTrack(
  sender: RtpSender | undefined,
  track: MediaStreamTrack | null,
): Promise<void> {
  if (!sender?.replaceTrack) return;
  await sender.replaceTrack(track);
}

export async function commitMicSend(
  pc: MediaConnection,
  send: MediaStream,
  session: number,
  capture: MediaStream = voice.rawMicStream ?? send,
): Promise<"replaced" | "added" | "none"> {
  send
    .getAudioTracks()
    .forEach((track) =>
      hintTrack(
        track,
        useMediaSettings.getState().processingMode === "original"
          ? "music"
          : "speech",
      ),
    );
  const track = send.getAudioTracks()[0];
  if (!track) return "none";
  if (voice.seat.connection !== pc || voice.seat.generation !== session)
    return "none";
  const existed = !!pc.sender("a");
  const state = useVoice.getState();
  await pc.publish({
    kind: "a",
    track,
    streamId: send.id,
    epoch: captureEpoch(capture),
    paused: state.muted || state.deafened,
  });
  return existed ? "replaced" : "added";
}

function failedMicProcessor(owner: MicProcessor): void {
  if (voice.activeMicGain !== owner) return;
  const sender = audioSender(),
    generation = voice.seat.generation,
    failedTrack = owner.stream.getAudioTracks()[0];
  // A capture that ended under the processor is the device gone (unplugged,
  // taken away), not the processor: the next capture keeps its filters.
  const lost =
    voice.rawMicStream !== null && !hasLiveTrack(voice.rawMicStream, "audio")
      ? voice.rawMicStream
      : null;
  if (!lost) voice.micForceBrowser = true;
  disableAudio(owner.stream);
  voice.rawMicStream?.getTracks().forEach((track) => track.stop());
  owner.dispose();
  voice.activeMicGain = null;
  voice.localStream = null;
  voice.rawMicStream = null;
  if (lost) {
    voice.micFailure = "unavailable";
    syncMicState();
    noteAudioProcessing({
      ...owner.info,
      actual: null,
      message: "Mikrofon getrennt; Standardgerät wird aktiviert",
      contextState: "closed",
    });
    noteMicLost(lost);
  } else {
    noteAudioProcessing({
      ...owner.info,
      actual: null,
      message: "Audioprozessor ausgefallen; Ersatzmikrofon wird aktiviert",
      contextState: "closed",
    });
    voice.deps?.onError?.(
      new Error(
        "Audioprozessor ausgefallen; lokale Aufnahme beendet, Browser-Ersatz wird aktiviert",
      ),
    );
  }
  void enqueueAudioCommit(async () => {
    if (voice.seat.generation === generation && sender?.track === failedTrack)
      await replaceSenderTrack(sender, null).catch(() => undefined);
  }).then(() => {
    if (
      voice.seat.generation === generation &&
      useVoice.getState().status === "joined" &&
      !voice.activeMicGain
    )
      void refreshMic();
  });
}

export async function acquireMic(
  getMedia: (constraints: MediaStreamConstraints) => Promise<MediaStream>,
  current: () => boolean,
): Promise<{ raw: MediaStream; processor: MicProcessor }> {
  const owned: { processor?: MicProcessor } = {};
  const result = await captureMicrophone(
    getMedia,
    useMediaSettings.getState(),
    voice.micForceBrowser,
    () => {
      if (owned.processor) failedMicProcessor(owned.processor);
    },
    {
      current,
      acquired(raw) {
        disableAudio(raw);
        voice.pendingMicRaw.add(raw);
      },
      discarded(raw) {
        voice.pendingMicRaw.delete(raw);
      },
    },
    (info) => {
      if (owned.processor && voice.activeMicGain === owned.processor)
        noteAudioProcessing(info);
    },
  );
  owned.processor = result.processor;
  const latest = useMediaSettings.getState();
  const gain =
    voice.micForceBrowser || result.processor.nativeFallback
      ? 1
      : latest.inputGain;
  if (result.processor.stream === result.raw && gain !== 1) {
    const previous = result.processor;
    try {
      result.processor = await createProcessor(
        result.raw,
        { ...latest, inputGain: gain },
        result.processor.info.actual ?? "browser",
        () => {
          if (owned.processor) failedMicProcessor(owned.processor);
        },
        (info) => {
          if (owned.processor && voice.activeMicGain === owned.processor)
            noteAudioProcessing(info);
        },
      );
      previous.dispose();
    } catch (error) {
      result.processor.dispose();
      stopTracks(result.raw);
      voice.pendingMicRaw.delete(result.raw);
      if (current() && !voice.micForceBrowser) {
        voice.micForceBrowser = true;
        return acquireMic(getMedia, current);
      }
      throw error;
    }
  } else result.processor.setGain(gain);
  owned.processor = result.processor;
  if (!current() || !result.processor.usable()) {
    result.processor.dispose();
    stopTracks(result.raw);
    voice.pendingMicRaw.delete(result.raw);
    if (current() && !voice.micForceBrowser) {
      voice.micForceBrowser = true;
      return acquireMic(getMedia, current);
    }
    throw new Error(
      "Mikrofonanfrage abgebrochen oder Audioprozessor ausgefallen",
    );
  }
  voice.captureEpochs.set(result.processor.stream, captureEpoch(result.raw));
  return result;
}

export function discardFailedMicCandidate(
  raw: MediaStream,
  insert: MicProcessor,
  current: boolean,
): void {
  insert.dispose();
  stopTracks(raw);
  voice.pendingMicRaw.delete(raw);
  if (current && !voice.micForceBrowser) {
    voice.micForceBrowser = true;
    voice.deps?.onError?.(
      new Error(
        "Audioprozessor vor Mikrofonwechsel ausgefallen; Browser-Ersatz wird aktiviert",
      ),
    );
    void refreshMic();
  }
}

/** A refused or blocked microphone: the user's own answer, asked again
 * only when they ask for it. */
function micRefused(error: unknown): boolean {
  const name = (error as { name?: unknown } | null)?.name;
  return name === "NotAllowedError" || name === "SecurityError";
}

/** The browser's reason stays in the console; the toast says it in German. */
function logMicFailure(error: unknown): void {
  logVoice("warn", "microphone", {
    detail:
      error instanceof Error
        ? `${error.name}: ${error.message}`
        : String(error),
  });
}

/** A join that goes on without a microphone. */
export function reportJoinMicFailure(error: unknown): void {
  logMicFailure(error);
  voice.micFailure = micRefused(error) ? "denied" : "unavailable";
  syncMicState();
  voice.deps?.onError?.(microphoneFailure(error));
  noteAudioProcessing(null);
  const info = useAudioProcessing.getState();
  noteAudioProcessing({
    ...info,
    requested: useMediaSettings.getState().processingMode,
    message: "Mikrofon konnte nicht aktiviert werden; du hörst weiter zu",
  });
}

/** Capture again for the connection that replaced the one a microphone
 * prompt was answered for, or have the next connection do it. Only within
 * the same join: a new join asks for its own microphone. A refused prompt is
 * not asked again but reported once. */
export function resumeOvertakenMic(join: number, error?: unknown): void {
  if (
    join !== voice.micJoin ||
    useVoice.getState().status !== "joined" ||
    hasLiveTrack(voice.localStream, "audio")
  )
    return;
  if (micRefused(error)) {
    voice.micOvertaken = false;
    reportJoinMicFailure(error);
    return;
  }
  const pc = voice.seat.connection;
  if (pc && voice.startedConnections.has(pc) && voice.seat.isOpen()) {
    voice.micOvertaken = false;
    void refreshMic();
  } else voice.micOvertaken = true;
}

export function announceMicrophone(): void {
  if (voice.announced.has("a")) return;
  const { serverId, channelId } = useVoice.getState();
  if (!serverId || !channelId) return;
  const self = currentUserId();
  if (self) setPub(self, "a", true);
  voice.announced.add("a");
  voice.deps?.gateway.send({
    op: "sig",
    t: "p",
    s: serverId,
    c: channelId,
    k: "a",
  });
}

/** A capture that now sends: the call has its microphone again, and watches
 * for losing it. */
export function noteMicLive(raw: MediaStream): void {
  voice.micFailure = null;
  syncMicState();
  for (const track of raw.getAudioTracks())
    track.addEventListener("ended", () => onMicEnded(raw));
}

/** The microphone was unplugged or failed mid-call, where no processor
 * watches the capture: capture again, which takes the system default while
 * the chosen device is gone. */
function onMicEnded(raw: MediaStream): void {
  if (voice.rawMicStream !== raw || useVoice.getState().status !== "joined")
    return;
  voice.micFailure = "unavailable";
  syncMicState();
  fallBackToDefaultMic(raw);
}

function fallBackToDefaultMic(raw: MediaStream): void {
  if (voice.micLossHandled.has(raw)) return;
  noteMicLost(raw);
  void refreshMic();
}

/** Says once per capture that it is gone, and remembers to switch back. */
function noteMicLost(raw: MediaStream): void {
  if (voice.micLossHandled.has(raw)) return;
  voice.micLossHandled.add(raw);
  logVoice("warn", "microphone-lost");
  noteDiagnosticEvent({
    kind: "device-change",
    connection: "voice",
    detail: "audio-input-lost",
  });
  voice.micFellBack = useMediaSettings.getState().audioInputId !== "";
  voice.deps?.onError?.(
    new Error("Mikrofon getrennt – Standardgerät wird verwendet"),
  );
}

/** A device came or went. The chosen microphone gone: fall back as if it
 * had ended (some browsers keep the track). There (again, or for the first
 * time since joining without it): use it. */
export async function onDeviceChange(): Promise<void> {
  const chosen = useMediaSettings.getState().audioInputId;
  if (useVoice.getState().status !== "joined" || !chosen || !voice.rawMicStream)
    return;
  const raw = voice.rawMicStream;
  let inputs: string[];
  try {
    inputs = (await navigator.mediaDevices.enumerateDevices())
      .filter((device) => device.kind === "audioinput")
      .map((device) => device.deviceId);
  } catch {
    return;
  }
  // Without permission the list has no ids and says nothing about the chosen one.
  if (
    inputs.every((id) => id === "") ||
    voice.rawMicStream !== raw ||
    useMediaSettings.getState().audioInputId !== chosen
  )
    return;
  const present = inputs.includes(chosen);
  // The capture names its device where the browser says: a chosen one that
  // was missing at joining is no loss when another device comes or goes.
  const using = raw.getAudioTracks()[0]?.getSettings?.().deviceId;
  const onChosen = using ? using === chosen : !voice.micFellBack;
  if (!present && onChosen) fallBackToDefaultMic(raw);
  else if (present && !onChosen) {
    voice.micFellBack = false;
    void refreshMic();
  }
}

/** The user asks for the microphone again after it was missing or refused. */
export function retryMicrophone(): void {
  if (useVoice.getState().status !== "joined") return;
  void refreshMic();
}

export async function refreshMic(): Promise<void> {
  const pc = voice.seat.connection;
  if (!pc || useVoice.getState().status !== "joined") return;
  const session = voice.seat.generation;
  const join = voice.micJoin;
  const request = ++voice.micEpoch;
  const getUserMedia = voice.deps?.getUserMedia ?? defaultGetUserMedia;
  let candidate: Awaited<ReturnType<typeof acquireMic>>;
  try {
    candidate = await acquireMic(getUserMedia, () =>
      micCurrent(pc, session, request),
    );
  } catch (error) {
    // A track still sending stays until a new one replaces it. Without one
    // (a failed processor took it off the sender, or a rebuild asks again)
    // the call goes on without a microphone.
    if (micCurrent(pc, session, request)) {
      if (hasLiveTrack(voice.localStream, "audio")) {
        logMicFailure(error);
        voice.deps?.onError?.(
          microphoneFailure(error, "der Wechsel hat nicht geklappt"),
        );
      } else reportJoinMicFailure(error);
    } else if (voice.seat.generation !== session)
      resumeOvertakenMic(join, error);
    return;
  }
  const { raw, processor: insert } = candidate;
  const send = insert.stream;
  if (!micCurrent(pc, session, request)) {
    insert.dispose();
    stopTracks(raw);
    if (voice.seat.generation !== session) resumeOvertakenMic(join);
    return;
  }
  disableAudio(raw);
  voice.pendingMicRaw.add(raw);
  disableAudio(send);
  await enqueueAudioCommit(async () => {
    if (!micCurrent(pc, session, request)) {
      insert?.dispose();
      stopTracks(raw);
      voice.pendingMicRaw.delete(raw);
      if (voice.seat.generation !== session) resumeOvertakenMic(join);
      return;
    }
    if (!insert.usable()) {
      discardFailedMicCandidate(raw, insert, true);
      return;
    }
    const sender = audioSender();
    const previous = sender?.track ?? null;
    let outcome: "replaced" | "added" | "none";
    try {
      outcome = await commitMicSend(pc, send, session, raw);
    } catch (error) {
      insert?.dispose();
      stopTracks(raw);
      voice.pendingMicRaw.delete(raw);
      if (voice.seat.generation !== session) {
        // A rebuild closed this connection while the publish was in flight
        // (socket still open, e.g. an "unavailable" answer or ICE recovery).
        resumeOvertakenMic(join);
      } else if (
        micCurrent(pc, session, request) &&
        !hasLiveTrack(voice.localStream, "audio") &&
        !voice.seat.isOpen()
      ) {
        // The media socket just closed and its rebuild has not started yet:
        // that one asks for the microphone again.
        voice.micOvertaken = true;
      } else if (micCurrent(pc, session, request)) {
        voice.deps?.onError?.(
          error instanceof Error
            ? error
            : new Error(
                "Mikrofonwechsel fehlgeschlagen; bisheriges Mikrofon bleibt aktiv",
              ),
        );
      }
      return;
    }
    if (!micCurrent(pc, session, request) || !insert.usable()) {
      if (voice.seat.generation === session && voice.seat.connection === pc) {
        try {
          if (outcome === "replaced" && previous && voice.localStream) {
            await commitMicSend(pc, voice.localStream, session);
            applyLocalAudio();
          } else if (outcome === "added") await pc.closeSource("a");
        } catch {
          // seat.connection may already be tearing down
        }
      }
      discardFailedMicCandidate(raw, insert, micCurrent(pc, session, request));
      return;
    }
    const oldRaw = voice.rawMicStream;
    const oldSend = voice.localStream;
    const oldInsert = voice.activeMicGain;
    voice.rawMicStream = raw;
    voice.localStream = send;
    voice.activeMicGain = insert;
    noteMicLive(raw);
    noteAudioProcessing(insert.info);
    voice.pendingMicRaw.delete(raw);
    applyLocalAudio();
    announceMicrophone();
    try {
      await applySendBitrate();
    } catch {
      // bitrate is best-effort
    }
    oldInsert?.dispose();
    if (oldSend && oldSend !== oldRaw && oldSend !== send) {
      stopTracks(oldSend);
    }
    if (oldRaw && oldRaw !== raw) {
      stopTracks(oldRaw);
    }
  });
}

async function applyInputGain(): Promise<void> {
  await enqueueAudioCommit(async () => {
    if (
      !voice.rawMicStream ||
      useVoice.getState().status !== "joined" ||
      !voice.seat.connection
    )
      return;
    const settings = useMediaSettings.getState();
    if (
      voice.activeMicGain &&
      voice.activeMicGain.stream !== voice.rawMicStream
    ) {
      voice.activeMicGain.setGain(settings.inputGain);
      return;
    }
    if (settings.inputGain === 1) return;
    const pc = voice.seat.connection,
      raw = voice.rawMicStream,
      session = voice.seat.generation,
      request = voice.micEpoch;
    const actual = voice.activeMicGain?.info?.actual ?? "browser";
    let insert: MicProcessor | undefined;
    try {
      insert = await createProcessor(
        raw,
        settings,
        actual,
        () => {
          if (insert) failedMicProcessor(insert);
        },
        (info) => {
          if (voice.activeMicGain === insert) noteAudioProcessing(info);
        },
      );
    } catch (error) {
      voice.deps?.onError?.(error);
      return;
    }
    if (!micCurrent(pc, session, request) || !insert.usable()) {
      insert.dispose();
      if (micCurrent(pc, session, request))
        voice.deps?.onError?.(
          new Error(
            "Mic-Gain konnte nicht aktiviert werden; bisheriges Mikrofon bleibt aktiv",
          ),
        );
      return;
    }
    disableAudio(insert.stream);
    const sender = audioSender(),
      previous = sender?.track ?? null;
    try {
      await commitMicSend(pc, insert.stream, session);
    } catch (error) {
      insert.dispose();
      voice.deps?.onError?.(error);
      return;
    }
    if (!micCurrent(pc, session, request) || !insert.usable()) {
      if (voice.seat.connection === pc && voice.seat.generation === session)
        await replaceSenderTrack(sender, previous).catch(() => undefined);
      insert.dispose();
      if (micCurrent(pc, session, request))
        voice.deps?.onError?.(
          new Error("Mic-Gain ausgefallen; bisheriges Mikrofon bleibt aktiv"),
        );
      return;
    }
    voice.activeMicGain?.dispose();
    voice.activeMicGain = insert;
    voice.localStream = insert.stream;
    noteAudioProcessing(insert.info);
    applyLocalAudio();
  });
}

export function queueInputGain(): void {
  void applyInputGain();
}

export function audioSender(): RtpSender | undefined {
  return voice.seat.connection?.sender("a");
}

/** The browser's own getUserMedia reasons are English ("Requested device not
 * found"); say them in German with what follows for the call: without a
 * sending microphone it goes on without one, a failed switch keeps the
 * previous one. */
function microphoneFailure(
  error: unknown,
  consequence = "du hörst nur zu",
): unknown {
  switch ((error as { name?: unknown } | null)?.name) {
    case "NotFoundError":
    case "OverconstrainedError":
      return new Error(`Kein Mikrofon gefunden, ${consequence}.`);
    case "NotAllowedError":
    case "SecurityError":
      return new Error(`Mikrofonzugriff nicht erlaubt, ${consequence}.`);
    case "NotReadableError":
    case "AbortError":
      return new Error(
        `Das Mikrofon ist gerade nicht verfügbar, ${consequence}.`,
      );
    default:
      return error;
  }
}
