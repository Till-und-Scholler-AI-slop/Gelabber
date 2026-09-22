// Voice diagnostics (issue 86). Numeric getStats() only — no A/V recording.
// Rates come from per-interval counter deltas. A missing browser field stays
// unknown (null), and a counter that goes backwards is a reset, not a loss.
// Export drops SDP, ICE candidates, IPs, tokens, cookies, and message text.

import { create } from "zustand";

import { APP_VERSION } from "../version.ts";
import {
  AUDIO_QUALITY,
  VIDEO_MAX_FPS,
  VIDEO_SEND_BUDGET,
  useMediaSettings,
  type MediaSettings,
} from "./settings.ts";

export const DIAGNOSTIC_LIMITS = {
  intervalMs: 2_000,
  samples: 48,
  events: 64,
  phases: 16,
} as const;

export type ConnectionRole = "voice" | "watch";
export type VideoSource = "camera" | "screen" | "live";
export type FlowSource = "voice" | VideoSource | "video" | "watch";
export type Phase = "voice-only" | "stream-on" | "stream-off" | "watch";
export type DiagnosticEventKind =
  | "stream-start"
  | "stream-stop"
  | "device-change"
  | "sdp-error"
  | "ice-error"
  | "recovery";

export type CounterName =
  | "bytesSent"
  | "bytesReceived"
  | "packetsSent"
  | "packetsReceived"
  | "packetsLost"
  | "concealedSamples"
  | "totalSamplesReceived"
  | "framesDecoded"
  | "framesSent";

const COUNTERS: readonly CounterName[] = [
  "bytesSent",
  "bytesReceived",
  "packetsSent",
  "packetsReceived",
  "packetsLost",
  "concealedSamples",
  "totalSamplesReceived",
  "framesDecoded",
  "framesSent",
];

export type StatsEntry = {
  id: string;
  type: string;
  timestamp?: number;
  kind?: string;
  mimeType?: string;
  codecId?: string;
  bytesSent?: number;
  bytesReceived?: number;
  packetsSent?: number;
  packetsReceived?: number;
  packetsLost?: number;
  jitter?: number;
  roundTripTime?: number;
  currentRoundTripTime?: number;
  concealedSamples?: number;
  totalSamplesReceived?: number;
  frameWidth?: number;
  frameHeight?: number;
  framesPerSecond?: number;
  framesDecoded?: number;
  framesSent?: number;
  qualityLimitationReason?: string;
  selected?: boolean;
  nominated?: boolean;
  state?: string;
  iceState?: string;
  localCandidateId?: string;
  remoteCandidateId?: string;
  selectedCandidatePairId?: string;
  candidateType?: string;
  availableOutgoingBitrate?: number;
  availableIncomingBitrate?: number;
  trackIdentifier?: string;
  trackId?: string;
  mediaSourceId?: string;
  mid?: string;
  localId?: string;
  fractionLost?: number;
};

export type FlowStats = {
  direction: "send" | "recv";
  kind: "audio" | "video";
  source: FlowSource;
  codec: string | null;
  measuredBitrateBps: number | null;
  configuredMaxBitrateBps: number | null;
  configuredMaxFps: number | null;
  packetLoss: number | null;
  packetsLost: number | null;
  jitterMs: number | null;
  rttMs: number | null;
  concealedSamples: number | null;
  totalSamplesReceived: number | null;
  concealedRatio: number | null;
  width: number | null;
  height: number | null;
  fps: number | null;
  qualityLimitationReason: string | null;
};

export type TransportSnapshot = {
  path: "direct" | "turn" | null;
  availableOutgoingBps: number | null;
  availableIncomingBps: number | null;
  rttMs: number | null;
};

export type ConnectionSnapshot = {
  role: ConnectionRole;
  transport: TransportSnapshot;
  flows: FlowStats[];
};

export type Caps = {
  audioMaxBitrate: number;
  videoSendBudget: number;
  videoMaxFps: number;
};

export type DiagnosticSample = {
  at: string;
  phase: Phase;
  caps: Caps;
  voice: ConnectionSnapshot | null;
  watch: ConnectionSnapshot | null;
};

export type DiagnosticEvent = {
  at: string;
  kind: DiagnosticEventKind;
  connection: ConnectionRole;
  detail: string;
};

export type PhaseMark = {
  at: string;
  phase: Phase;
  audioSendBps: number | null;
  audioRecvBps: number | null;
  videoSendBps: number | null;
};

export type RelevantSettings = {
  audioQuality: string;
  audioMaxBitrate: number;
  echoCancellation: boolean;
  noiseSuppression: boolean;
  autoGainControl: boolean;
  outputVolume: number;
  inputGain: number;
  videoSendBudget: number;
  videoMaxFps: number;
  customAudioInput: boolean;
  customAudioOutput: boolean;
  customVideoInput: boolean;
};

export type DiagnosticExport = {
  exportedAt: string;
  appVersion: string;
  browser: { userAgent: string; platform: string; language: string };
  settings: RelevantSettings;
  note: string;
  phases: PhaseMark[];
  events: DiagnosticEvent[];
  samples: DiagnosticSample[];
};

type Baseline = {
  timestamp?: number;
  counters: Partial<Record<CounterName, number>>;
};

type DiagnosticsState = {
  samples: DiagnosticSample[];
  events: DiagnosticEvent[];
  phases: PhaseMark[];
  latest: DiagnosticSample | null;
  polling: { voice: boolean; watch: boolean };
};

type Poll = {
  token: number;
  timer: ReturnType<typeof setInterval>;
  getReport: () => Promise<readonly StatsEntry[] | null>;
  videoSources: () => Readonly<Record<string, VideoSource>>;
  streaming: () => boolean;
  caps: () => Caps;
};

const emptyState = (): DiagnosticsState => ({
  samples: [],
  events: [],
  phases: [],
  latest: null,
  polling: { voice: false, watch: false },
});

export const useVoiceDiagnostics = create<DiagnosticsState>(emptyState);

const baselines = new Map<ConnectionRole, Map<string, Baseline>>();
const polls = new Map<ConnectionRole, Poll>();
const faulted = new Set<ConnectionRole>();
let tokenSeq = 0;
let streamSeen = false;
let liveVoice: ConnectionSnapshot | null = null;
let liveWatch: ConnectionSnapshot | null = null;
let logoutInstalled = false;

const NUMERIC_FIELDS = [
  "timestamp",
  "bytesSent",
  "bytesReceived",
  "packetsSent",
  "packetsReceived",
  "packetsLost",
  "jitter",
  "roundTripTime",
  "currentRoundTripTime",
  "concealedSamples",
  "totalSamplesReceived",
  "frameWidth",
  "frameHeight",
  "framesPerSecond",
  "framesDecoded",
  "framesSent",
  "availableOutgoingBitrate",
  "availableIncomingBitrate",
  "fractionLost",
] as const;

const STRING_FIELDS = [
  "type",
  "kind",
  "mimeType",
  "codecId",
  "qualityLimitationReason",
  "state",
  "iceState",
  "localCandidateId",
  "remoteCandidateId",
  "selectedCandidatePairId",
  "candidateType",
  "trackIdentifier",
  "trackId",
  "mediaSourceId",
  "mid",
  "localId",
  "id",
] as const;

const ICE_TOKENS = new Set([
  "new",
  "checking",
  "connected",
  "completed",
  "disconnected",
  "failed",
  "closed",
  "succeeded",
  "in-progress",
  "waiting",
  "frozen",
]);

const BANNED_EXPORT_KEY =
  /^(sdp|candidate|candidates|cookie|cookies|token|tokens|password|authorization|address|relatedaddress|ip|port|url|username|credential|message|content|email|foundation|protocol|ice)$/i;

function finiteNumber(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value)
    ? value
    : undefined;
}

function shortToken(value: unknown, max = 32): string | undefined {
  if (typeof value !== "string") return undefined;
  const trimmed = value.trim();
  if (!trimmed || trimmed.length > max) return undefined;
  // Length is the `max` check above. The pattern is only the allowed charset.
  if (!/^[A-Za-z0-9_.:/+-]+$/.test(trimmed)) return undefined;
  return trimmed;
}

/** Copy only allowlisted stat fields. Addresses, candidates, and SDP never land. */
export function statsEntriesFromReport(report: unknown): StatsEntry[] {
  const raw: { key: string | null; value: unknown }[] = [];
  if (report instanceof Map) {
    for (const [key, value] of report) {
      raw.push({ key: typeof key === "string" ? key : null, value });
    }
  } else if (Array.isArray(report)) {
    for (const value of report) raw.push({ key: null, value });
  } else if (
    report &&
    typeof report === "object" &&
    "forEach" in report &&
    typeof report.forEach === "function"
  ) {
    (
      report as { forEach: (fn: (value: unknown, key: string) => void) => void }
    ).forEach((value, key) => {
      raw.push({ key: typeof key === "string" ? key : null, value });
    });
  }
  const entries: StatsEntry[] = [];
  for (const item of raw) {
    const entry = sanitizeEntry(item.value, item.key);
    if (entry) entries.push(entry);
  }
  return entries;
}

function sanitizeEntry(
  value: unknown,
  mapKey: string | null,
): StatsEntry | null {
  if (!value || typeof value !== "object") return null;
  const source = value as Record<string, unknown>;
  const id = shortToken(source.id, 64) ?? (mapKey && shortToken(mapKey, 64));
  const type = shortToken(source.type, 40);
  if (!id || !type) return null;
  const entry: StatsEntry = { id, type };
  for (const field of NUMERIC_FIELDS) {
    const num = finiteNumber(source[field]);
    if (num !== undefined) entry[field] = num;
  }
  for (const field of STRING_FIELDS) {
    if (field === "id" || field === "type") continue;
    const token = shortToken(source[field], field === "mimeType" ? 40 : 64);
    if (token !== undefined) entry[field] = token;
  }
  if (typeof source.selected === "boolean") entry.selected = source.selected;
  if (typeof source.nominated === "boolean") entry.nominated = source.nominated;
  if (entry.state && !ICE_TOKENS.has(entry.state)) delete entry.state;
  if (entry.iceState && !ICE_TOKENS.has(entry.iceState)) delete entry.iceState;
  if (entry.kind !== "audio" && entry.kind !== "video") delete entry.kind;
  return entry;
}

export function sanitizeDetail(input: string): string {
  const trimmed = input.trim();
  if (!trimmed) return "unbekannt";
  if (/bearer\s+\S+|cookie|csrf|token=/i.test(trimmed)) return "unbekannt";
  if (
    trimmed.length > 180 ||
    /v=0/.test(trimmed) ||
    /a=candidate/i.test(trimmed) ||
    /a=ice-/i.test(trimmed)
  ) {
    return "redacted";
  }
  const cleaned = trimmed
    .replace(/candidate:\S+/gi, "")
    .replace(/\b(?:\d{1,3}\.){3}\d{1,3}\b/g, "")
    .replace(/\b(?:[0-9a-f]{0,4}:){2,}[0-9a-f]{0,4}\b/gi, "")
    .replace(/\s+/g, " ")
    .trim();
  if (!cleaned) return "unbekannt";
  return cleaned.slice(0, 80);
}

/**
 * Delta of one cumulative counter.
 * The first sample and a missing field are unknown, not zero.
 * A decrease is a reset: this interval has no delta.
 */
export function diffCounter(
  previous: number | undefined,
  next: number | undefined,
): { delta: number | null; reset: boolean } {
  if (next === undefined || !Number.isFinite(next) || next < 0) {
    return { delta: null, reset: false };
  }
  if (previous === undefined || !Number.isFinite(previous)) {
    return { delta: null, reset: false };
  }
  if (next < previous) return { delta: null, reset: true };
  return { delta: next - previous, reset: false };
}

function counterValue(entry: StatsEntry, key: CounterName): number | undefined {
  const value = entry[key];
  if (value === undefined || !Number.isFinite(value) || value < 0)
    return undefined;
  return value;
}

function gauge(value: number | undefined): number | null {
  if (value === undefined || !Number.isFinite(value)) return null;
  return value;
}

function secondsToMs(value: number | undefined): number | null {
  const seconds = gauge(value);
  if (seconds === null) return null;
  return seconds * 1000;
}

function bitrateBps(bytes: number | null, dtMs: number | null): number | null {
  if (bytes === null || dtMs === null || dtMs <= 0) return null;
  return (bytes * 8 * 1000) / dtMs;
}

function lossRatio(lost: number | null, base: number | null): number | null {
  if (lost === null || base === null || base <= 0 || lost < 0) return null;
  return lost / base;
}

function codecName(mime: string | undefined): string | null {
  if (!mime) return null;
  const part = mime.split("/")[1];
  return part || null;
}

function mediaKind(
  entry: StatsEntry,
  codecs: Map<string, StatsEntry>,
): "audio" | "video" | null {
  if (entry.kind === "audio" || entry.kind === "video") return entry.kind;
  const mime = entry.codecId ? codecs.get(entry.codecId)?.mimeType : undefined;
  if (mime?.startsWith("audio/")) return "audio";
  if (mime?.startsWith("video/")) return "video";
  return null;
}

function trackIdentifier(
  entry: StatsEntry,
  byId: Map<string, StatsEntry>,
): string | null {
  if (entry.trackIdentifier) return entry.trackIdentifier;
  if (entry.trackId) {
    const track = byId.get(entry.trackId);
    if (track?.trackIdentifier) return track.trackIdentifier;
  }
  if (entry.mediaSourceId) {
    const source = byId.get(entry.mediaSourceId);
    if (source?.trackIdentifier) return source.trackIdentifier;
  }
  return null;
}

function transportPath(
  localType: string | undefined,
  remoteType: string | undefined,
): "direct" | "turn" | null {
  const known = [localType, remoteType].filter((type): type is string =>
    Boolean(type),
  );
  if (known.length === 0) return null;
  if (known.some((type) => type === "relay")) return "turn";
  if (known.length < 2) return null;
  if (
    known.every(
      (type) => type === "host" || type === "srflx" || type === "prflx",
    )
  ) {
    return "direct";
  }
  return null;
}

export function reduceConnection(input: {
  role: ConnectionRole;
  entries: readonly StatsEntry[];
  previous: ReadonlyMap<string, Baseline>;
  caps: Caps;
  videoSources: Readonly<Record<string, VideoSource>>;
}): { snapshot: ConnectionSnapshot; next: Map<string, Baseline> } {
  const byId = new Map(input.entries.map((entry) => [entry.id, entry]));
  const codecs = new Map(
    input.entries
      .filter((entry) => entry.type === "codec")
      .map((entry) => [entry.id, entry]),
  );
  const remoteInbound = input.entries.filter(
    (entry) => entry.type === "remote-inbound-rtp",
  );
  const next = new Map<string, Baseline>();
  const rtp = input.entries.filter(
    (entry) => entry.type === "inbound-rtp" || entry.type === "outbound-rtp",
  );
  const videoSenders = rtp.filter(
    (entry) =>
      entry.type === "outbound-rtp" && mediaKind(entry, codecs) === "video",
  ).length;
  const perVideo =
    videoSenders > 0
      ? Math.floor(input.caps.videoSendBudget / videoSenders)
      : null;

  const flows: FlowStats[] = [];
  for (const entry of rtp) {
    const kind = mediaKind(entry, codecs);
    if (!kind) continue;
    const direction = entry.type === "outbound-rtp" ? "send" : "recv";
    const prev = input.previous.get(entry.id);
    const timestamp = finiteNumber(entry.timestamp);
    const timeReset =
      prev?.timestamp !== undefined &&
      timestamp !== undefined &&
      timestamp <= prev.timestamp;
    const dt =
      !timeReset && prev?.timestamp !== undefined && timestamp !== undefined
        ? timestamp - prev.timestamp
        : null;
    const deltas = {} as Record<
      CounterName,
      { delta: number | null; reset: boolean }
    >;
    for (const key of COUNTERS) {
      deltas[key] = timeReset
        ? { delta: null, reset: true }
        : diffCounter(prev?.counters[key], counterValue(entry, key));
    }
    const counters: Baseline["counters"] = {};
    for (const key of COUNTERS) {
      const value = counterValue(entry, key);
      if (value !== undefined) counters[key] = value;
    }
    next.set(entry.id, { timestamp, counters });

    const remote = remoteInbound.find((item) => item.localId === entry.id);
    const remotePrev = remote ? input.previous.get(remote.id) : undefined;
    const remoteLost = remote
      ? diffCounter(
          timeReset ? undefined : remotePrev?.counters.packetsLost,
          timeReset ? undefined : counterValue(remote, "packetsLost"),
        )
      : { delta: null, reset: false };
    if (remote) {
      const remoteCounters: Baseline["counters"] = {};
      for (const key of COUNTERS) {
        const value = counterValue(remote, key);
        if (value !== undefined) remoteCounters[key] = value;
      }
      next.set(remote.id, {
        timestamp: finiteNumber(remote.timestamp) ?? timestamp,
        counters: remoteCounters,
      });
    }

    const bytes =
      direction === "send"
        ? deltas.bytesSent.delta
        : deltas.bytesReceived.delta;
    const lostDelta =
      direction === "recv" ? deltas.packetsLost.delta : remoteLost.delta;
    const lossBase =
      direction === "recv"
        ? deltas.packetsReceived.delta !== null && lostDelta !== null
          ? deltas.packetsReceived.delta + lostDelta
          : null
        : deltas.packetsSent.delta;
    const fraction =
      lostDelta === null &&
      remote?.fractionLost !== undefined &&
      remote.fractionLost >= 0 &&
      remote.fractionLost <= 1
        ? remote.fractionLost
        : null;
    const concealed = kind === "audio" ? deltas.concealedSamples.delta : null;
    const samples = kind === "audio" ? deltas.totalSamplesReceived.delta : null;
    const track = trackIdentifier(entry, byId);
    const source = sourceOf(
      input.role,
      direction,
      kind,
      track,
      input.videoSources,
    );
    const fpsGauge = gauge(entry.framesPerSecond);
    const frameDelta =
      direction === "send"
        ? (deltas.framesSent.delta ?? deltas.framesDecoded.delta)
        : (deltas.framesDecoded.delta ?? deltas.framesSent.delta);
    const fps =
      kind === "video"
        ? fpsGauge !== null
          ? fpsGauge
          : bitrateLikeFps(frameDelta, dt)
        : null;
    flows.push({
      direction,
      kind,
      source,
      codec: codecName(
        entry.codecId ? codecs.get(entry.codecId)?.mimeType : undefined,
      ),
      measuredBitrateBps: bitrateBps(bytes, dt),
      configuredMaxBitrateBps:
        direction === "send"
          ? kind === "audio"
            ? input.caps.audioMaxBitrate
            : perVideo
          : null,
      configuredMaxFps:
        direction === "send" && kind === "video"
          ? input.caps.videoMaxFps
          : null,
      packetLoss: lossRatio(lostDelta, lossBase) ?? fraction,
      packetsLost: lostDelta,
      jitterMs: secondsToMs(
        direction === "send" ? (remote?.jitter ?? entry.jitter) : entry.jitter,
      ),
      rttMs: secondsToMs(
        direction === "send" ? remote?.roundTripTime : entry.roundTripTime,
      ),
      concealedSamples: concealed,
      totalSamplesReceived: samples,
      concealedRatio:
        concealed !== null && samples !== null && samples > 0
          ? concealed / samples
          : null,
      width: kind === "video" ? gauge(entry.frameWidth) : null,
      height: kind === "video" ? gauge(entry.frameHeight) : null,
      fps,
      qualityLimitationReason:
        kind === "video" && direction === "send"
          ? qualityLimit(entry.qualityLimitationReason)
          : null,
    });
  }

  return {
    snapshot: {
      role: input.role,
      transport: reduceTransport(input.entries, byId),
      flows,
    },
    next,
  };
}

function bitrateLikeFps(
  frames: number | null,
  dtMs: number | null,
): number | null {
  if (frames === null || dtMs === null || dtMs <= 0) return null;
  return (frames * 1000) / dtMs;
}

function qualityLimit(value: string | undefined): string | null {
  if (!value) return null;
  if (
    value === "none" ||
    value === "cpu" ||
    value === "bandwidth" ||
    value === "other"
  ) {
    return value;
  }
  return null;
}

function sourceOf(
  role: ConnectionRole,
  direction: "send" | "recv",
  kind: "audio" | "video",
  track: string | null,
  videoSources: Readonly<Record<string, VideoSource>>,
): FlowSource {
  if (role === "watch") return "watch";
  if (kind === "video" && direction === "send") {
    if (track && videoSources[track]) return videoSources[track];
    return "video";
  }
  return "voice";
}

function reduceTransport(
  entries: readonly StatsEntry[],
  byId: Map<string, StatsEntry>,
): TransportSnapshot {
  const unknown: TransportSnapshot = {
    path: null,
    availableOutgoingBps: null,
    availableIncomingBps: null,
    rttMs: null,
  };
  const transports = entries.filter((entry) => entry.type === "transport");
  const pairs = entries.filter((entry) => entry.type === "candidate-pair");
  const selectedId = transports.find(
    (entry) => entry.selectedCandidatePairId,
  )?.selectedCandidatePairId;
  const pair =
    (selectedId ? pairs.find((entry) => entry.id === selectedId) : undefined) ??
    pairs.find((entry) => entry.selected) ??
    pairs.find((entry) => entry.nominated && entry.state === "succeeded") ??
    (pairs.filter((entry) => entry.state === "succeeded").length === 1
      ? pairs.find((entry) => entry.state === "succeeded")
      : undefined);
  if (!pair) return unknown;
  const local = pair.localCandidateId
    ? byId.get(pair.localCandidateId)
    : undefined;
  const remote = pair.remoteCandidateId
    ? byId.get(pair.remoteCandidateId)
    : undefined;
  return {
    path: transportPath(local?.candidateType, remote?.candidateType),
    availableOutgoingBps: gauge(pair.availableOutgoingBitrate),
    availableIncomingBps: gauge(pair.availableIncomingBitrate),
    rttMs: secondsToMs(pair.currentRoundTripTime),
  };
}

function sumMeasured(
  flows: FlowStats[] | undefined,
  kind: "audio" | "video",
  direction: "send" | "recv",
): number | null {
  const matched = (flows ?? []).filter(
    (flow) => flow.kind === kind && flow.direction === direction,
  );
  if (matched.length === 0) return null;
  if (matched.some((flow) => flow.measuredBitrateBps === null)) return null;
  return matched.reduce(
    (total, flow) => total + (flow.measuredBitrateBps ?? 0),
    0,
  );
}

function classifyVoice(streaming: boolean): Phase {
  if (streaming) {
    streamSeen = true;
    return "stream-on";
  }
  return streamSeen ? "stream-off" : "voice-only";
}

/** Shared phase uses the voice poll's streaming state when that poll is up. */
function phaseForReport(reportedStreaming: boolean): Phase {
  const voice = polls.get("voice");
  if (voice) return classifyVoice(voice.streaming());
  if (polls.has("watch")) return "watch";
  return classifyVoice(reportedStreaming);
}

function pushBounded<T>(items: readonly T[], item: T, max: number): T[] {
  const next =
    items.length >= max ? items.slice(items.length - max + 1) : items.slice();
  next.push(item);
  return next;
}

function recordPhase(
  phase: Phase,
  at: string,
  sample: DiagnosticSample | null,
): void {
  const state = useVoiceDiagnostics.getState();
  const last = state.phases[state.phases.length - 1];
  if (last?.phase === phase) return;
  const mark: PhaseMark = {
    at,
    phase,
    audioSendBps: sumMeasured(sample?.voice?.flows, "audio", "send"),
    audioRecvBps: sumMeasured(sample?.voice?.flows, "audio", "recv"),
    videoSendBps: sumMeasured(sample?.voice?.flows, "video", "send"),
  };
  useVoiceDiagnostics.setState({
    phases: pushBounded(state.phases, mark, DIAGNOSTIC_LIMITS.phases),
  });
}

export function noteDiagnosticEvent(event: {
  kind: DiagnosticEventKind;
  connection: ConnectionRole;
  detail: string;
  streaming?: boolean;
  at?: number;
}): void {
  if (event.kind === "ice-error" || event.kind === "sdp-error") {
    faulted.add(event.connection);
  }
  if (event.kind === "recovery") {
    if (!faulted.has(event.connection)) return;
    faulted.delete(event.connection);
  }
  const at = new Date(event.at ?? Date.now()).toISOString();
  const stored: DiagnosticEvent = {
    at,
    kind: event.kind,
    connection: event.connection,
    detail: sanitizeDetail(event.detail),
  };
  const state = useVoiceDiagnostics.getState();
  useVoiceDiagnostics.setState({
    events: pushBounded(state.events, stored, DIAGNOSTIC_LIMITS.events),
  });
  if (
    (event.kind === "stream-start" || event.kind === "stream-stop") &&
    event.streaming !== undefined
  ) {
    recordPhase(
      classifyVoice(event.streaming),
      at,
      useVoiceDiagnostics.getState().latest,
    );
  }
}

export function applyStatsReport(
  role: ConnectionRole,
  entries: readonly StatsEntry[],
  opts: {
    streaming: boolean;
    caps: Caps;
    videoSources: Readonly<Record<string, VideoSource>>;
    now?: number;
  },
): DiagnosticSample {
  const previous = baselines.get(role) ?? new Map<string, Baseline>();
  const reduced = reduceConnection({
    role,
    entries,
    previous,
    caps: opts.caps,
    videoSources: opts.videoSources,
  });
  baselines.set(role, reduced.next);
  if (role === "voice") liveVoice = reduced.snapshot;
  else liveWatch = reduced.snapshot;
  const phase = phaseForReport(opts.streaming);
  const sample: DiagnosticSample = {
    at: new Date(opts.now ?? Date.now()).toISOString(),
    phase,
    caps: opts.caps,
    voice: polls.has("voice") || role === "voice" ? liveVoice : null,
    watch: polls.has("watch") || role === "watch" ? liveWatch : null,
  };
  const state = useVoiceDiagnostics.getState();
  useVoiceDiagnostics.setState({
    samples: pushBounded(state.samples, sample, DIAGNOSTIC_LIMITS.samples),
    latest: sample,
  });
  recordPhase(phase, sample.at, sample);
  return sample;
}

function syncPolling(): void {
  useVoiceDiagnostics.setState({
    polling: {
      voice: polls.has("voice"),
      watch: polls.has("watch"),
    },
  });
}

async function tick(role: ConnectionRole, token: number): Promise<void> {
  const poll = polls.get(role);
  if (!poll || poll.token !== token) return;
  let entries: readonly StatsEntry[] | null;
  try {
    entries = await poll.getReport();
  } catch {
    entries = null;
  }
  const current = polls.get(role);
  if (!current || current.token !== token) return;
  if (!entries) return;
  applyStatsReport(role, entries, {
    streaming: current.streaming(),
    caps: current.caps(),
    videoSources: current.videoSources(),
  });
}

export function attachDiagnostics(input: {
  role: ConnectionRole;
  getReport: () => Promise<readonly StatsEntry[] | null>;
  videoSources?: () => Readonly<Record<string, VideoSource>>;
  streaming?: () => boolean;
  caps?: () => Caps;
}): void {
  detachDiagnostics(input.role);
  const token = ++tokenSeq;
  const poll: Poll = {
    token,
    timer: 0 as ReturnType<typeof setInterval>,
    getReport: input.getReport,
    videoSources: input.videoSources ?? (() => ({})),
    streaming: input.streaming ?? (() => false),
    caps: input.caps ?? defaultCaps,
  };
  const run = () => {
    void tick(input.role, token);
  };
  poll.timer = setInterval(run, DIAGNOSTIC_LIMITS.intervalMs);
  const unref = poll.timer as { unref?: () => void };
  unref.unref?.();
  polls.set(input.role, poll);
  if (input.role === "voice") {
    recordPhase(
      classifyVoice(poll.streaming()),
      new Date().toISOString(),
      liveSample(),
    );
  } else if (!polls.has("voice")) {
    recordPhase("watch", new Date().toISOString(), liveSample());
  }
  syncPolling();
  run();
}

function liveSample(): DiagnosticSample | null {
  return useVoiceDiagnostics.getState().latest;
}

export function detachDiagnostics(role: ConnectionRole): void {
  const poll = polls.get(role);
  if (poll) {
    clearInterval(poll.timer);
    polls.delete(role);
  }
  baselines.delete(role);
  if (role === "voice") liveVoice = null;
  else liveWatch = null;
  syncPolling();
}

export function diagnosticsPolling(): { voice: boolean; watch: boolean } {
  return {
    voice: polls.has("voice"),
    watch: polls.has("watch"),
  };
}

export function resetDiagnostics(): void {
  for (const role of ["voice", "watch"] as const) detachDiagnostics(role);
  baselines.clear();
  faulted.clear();
  streamSeen = false;
  liveVoice = null;
  liveWatch = null;
  useVoiceDiagnostics.setState(emptyState());
}

type LogoutListener = (
  state: { user: unknown },
  previous: { user: unknown },
) => void;

/**
 * Register a logout reset. The caller passes `subscribe` so this module
 * does not import the session store while that store is still initializing.
 */
export function installDiagnosticsLogoutReset(
  subscribe: (listener: LogoutListener) => void,
): void {
  if (logoutInstalled) return;
  logoutInstalled = true;
  subscribe((state, previous) => {
    if (previous.user && !state.user) resetDiagnostics();
  });
}

export function defaultCaps(): Caps {
  return {
    audioMaxBitrate: AUDIO_QUALITY[useMediaSettings.getState().quality].bitrate,
    videoSendBudget: VIDEO_SEND_BUDGET,
    videoMaxFps: VIDEO_MAX_FPS,
  };
}

export function relevantSettings(
  settings: MediaSettings = useMediaSettings.getState(),
): RelevantSettings {
  return {
    audioQuality: settings.quality,
    audioMaxBitrate: AUDIO_QUALITY[settings.quality].bitrate,
    echoCancellation: settings.echoCancellation,
    noiseSuppression: settings.noiseSuppression,
    autoGainControl: settings.autoGainControl,
    outputVolume: settings.outputVolume,
    inputGain: settings.inputGain,
    videoSendBudget: VIDEO_SEND_BUDGET,
    videoMaxFps: VIDEO_MAX_FPS,
    customAudioInput: settings.audioInputId !== "",
    customAudioOutput: settings.audioOutputId !== "",
    customVideoInput: settings.videoInputId !== "",
  };
}

function browserInfo(): DiagnosticExport["browser"] {
  if (typeof navigator === "undefined") {
    return { userAgent: "unknown", platform: "unknown", language: "unknown" };
  }
  return {
    userAgent: navigator.userAgent || "unknown",
    platform: navigator.platform || "unknown",
    language: navigator.language || "unknown",
  };
}

function scrubString(value: string): string {
  if (/bearer\s+\S+|cookie|csrf|token=/i.test(value)) return "unbekannt";
  if (
    /v=0/.test(value) ||
    /a=candidate/i.test(value) ||
    /candidate:\S+/i.test(value)
  ) {
    return "redacted";
  }
  if (/\b(?:\d{1,3}\.){3}\d{1,3}\b/.test(value)) return "unbekannt";
  return value;
}

function scrub(value: unknown): unknown {
  if (typeof value === "string") return scrubString(value);
  if (Array.isArray(value)) return value.map(scrub);
  if (!value || typeof value !== "object") return value;
  const out: Record<string, unknown> = {};
  for (const [key, inner] of Object.entries(value)) {
    if (BANNED_EXPORT_KEY.test(key)) continue;
    out[key] = scrub(inner);
  }
  return out;
}

export function buildDiagnosticExport(now = new Date()): DiagnosticExport {
  const state = useVoiceDiagnostics.getState();
  const payload: DiagnosticExport = {
    exportedAt: now.toISOString(),
    appVersion: APP_VERSION,
    browser: browserInfo(),
    settings: relevantSettings(),
    note: "Messwerte sind Anhaltspunkte und keine bewiesene Fehlerursache.",
    phases: state.phases,
    events: state.events,
    samples: state.samples,
  };
  return scrub(payload) as DiagnosticExport;
}

export function formatBps(bps: number | null): string {
  if (bps === null || !Number.isFinite(bps)) return "unbekannt";
  const abs = Math.abs(bps);
  const scaled = abs >= 1_000_000 ? abs / 1_000_000 : abs / 1_000;
  const unit = abs >= 1_000_000 ? "Mbit/s" : "kbit/s";
  const digits = Number.isInteger(scaled) || scaled >= 100 ? 0 : 1;
  const text = scaled.toLocaleString("de-DE", {
    minimumFractionDigits: 0,
    maximumFractionDigits: digits,
  });
  return `${text} ${unit}`;
}

export function formatRatio(ratio: number | null): string {
  if (ratio === null || !Number.isFinite(ratio)) return "unbekannt";
  return `${(ratio * 100).toLocaleString("de-DE", {
    minimumFractionDigits: 1,
    maximumFractionDigits: 1,
  })} %`;
}

export function formatMs(value: number | null): string {
  if (value === null || !Number.isFinite(value)) return "unbekannt";
  return `${value.toLocaleString("de-DE", { maximumFractionDigits: 1 })} ms`;
}

export function formatCount(value: number | null): string {
  if (value === null || !Number.isFinite(value)) return "unbekannt";
  return value.toLocaleString("de-DE", { maximumFractionDigits: 1 });
}

export const PHASE_LABEL: Record<Phase, string> = {
  "voice-only": "nur Sprache",
  "stream-on": "Stream an",
  "stream-off": "Stream aus",
  watch: "Watch",
};

export const SOURCE_LABEL: Record<FlowSource, string> = {
  voice: "Sprache",
  camera: "Kamera",
  screen: "Bildschirm",
  live: "Live",
  video: "Video",
  watch: "Watch",
};

export const QUALITY_LIMIT_LABEL: Record<string, string> = {
  none: "keine",
  cpu: "CPU",
  bandwidth: "Bandbreite",
  other: "sonstige",
};

export function formatQualityLimit(value: string | null): string {
  if (!value) return "unbekannt";
  return QUALITY_LIMIT_LABEL[value] ?? "unbekannt";
}

export function formatClock(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  return date.toLocaleTimeString("de-DE", {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}
