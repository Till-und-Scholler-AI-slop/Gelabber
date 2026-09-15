// Issue #53 — opt-in capture diagnostics (constraints dump + optional A/B MediaRecorder).
// No gain/bitrate changes. Playtest: save downloads under output/issue-53/.

export const CAPTURE_COMPARE_KEYS = [
  "echoCancellation",
  "noiseSuppression",
  "autoGainControl",
  "channelCount",
  "sampleRate",
] as const;

export type CaptureCompareKey = (typeof CAPTURE_COMPARE_KEYS)[number];

export type CaptureSettingComparison = Record<
  string,
  { requested: unknown; applied: unknown; match: boolean | null }
>;

export type CaptureSettingsDiag = {
  at: string;
  userAgent: string | null;
  track: {
    id: string;
    label: string;
    kind: string;
    readyState: string;
    contentHint: string | null;
  };
  requested: MediaTrackConstraints;
  applied: MediaTrackSettings;
  comparison: CaptureSettingComparison;
  /** Playtest should save this JSON under output/issue-53/ */
  playtestSaveHint: string;
};

export type AbRecordHandle = {
  side: "local" | "remote";
  stop: () => Promise<void>;
};

type AbState = {
  local: AbRecordHandle | null;
  remote: AbRecordHandle | null;
  remoteStarted: boolean;
  autoStopTimer: ReturnType<typeof setTimeout> | null;
  lastLocalTrack: MediaStreamTrack | null;
  lastRemoteTrack: MediaStreamTrack | null;
};

const AB_MS = 20_000;
const PLAYTEST_HINT =
  "Playtest: save capture-settings-*.json and local-*/remote-*.webm under output/issue-53/";

const ab: AbState = {
  local: null,
  remote: null,
  remoteStarted: false,
  autoStopTimer: null,
  lastLocalTrack: null,
  lastRemoteTrack: null,
};

function readViteDiagFlag(): boolean {
  try {
    if (typeof import.meta === "undefined") return false;
    const meta = import.meta as ImportMeta & {
      env?: Record<string, string | undefined>;
    };
    return meta.env?.VITE_GELABBER_DIAG === "1";
  } catch {
    return false;
  }
}

function queryDiagEnabled(): boolean {
  if (typeof window === "undefined" || typeof window.location === "undefined") {
    return false;
  }
  try {
    return new URLSearchParams(window.location.search).get("gelabberDiag") === "1";
  } catch {
    return false;
  }
}

function storageDiagEnabled(): boolean {
  if (typeof localStorage === "undefined") return false;
  try {
    return localStorage.getItem("gelabberDiag") === "1";
  } catch {
    return false;
  }
}

/** True when ?gelabberDiag=1, localStorage gelabberDiag=1, or VITE_GELABBER_DIAG=1. */
export function isCaptureDiagEnabled(): boolean {
  return queryDiagEnabled() || storageDiagEnabled() || readViteDiagFlag();
}

/** Pure compare of requested constraint keys vs getSettings() values. */
export function compareCaptureSettings(
  requested: MediaTrackConstraints,
  applied: MediaTrackSettings,
): CaptureSettingComparison {
  const out: CaptureSettingComparison = {};
  for (const key of CAPTURE_COMPARE_KEYS) {
    if (!(key in requested)) continue;
    const req = (requested as Record<string, unknown>)[key];
    const app = (applied as Record<string, unknown>)[key];
    const match =
      app === undefined ? null : Object.is(normalizeConstraint(req), app);
    out[key] = { requested: req, applied: app, match };
  }
  return out;
}

function normalizeConstraint(value: unknown): unknown {
  if (value && typeof value === "object" && "exact" in (value as object)) {
    return (value as { exact: unknown }).exact;
  }
  if (value && typeof value === "object" && "ideal" in (value as object)) {
    return (value as { ideal: unknown }).ideal;
  }
  return value;
}

function isoStamp(d = new Date()): string {
  return d.toISOString().replace(/[:.]/g, "-");
}

function downloadBlob(blob: Blob, filename: string): void {
  if (typeof document === "undefined" || typeof URL === "undefined") return;
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  a.rel = "noopener";
  document.body.appendChild(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 2_000);
}

function stashDiag(diag: CaptureSettingsDiag): void {
  if (typeof window === "undefined") return;
  const prev = window.__gelabberCaptureDiag;
  if (Array.isArray(prev)) {
    prev.push(diag);
    window.__gelabberCaptureDiag = prev;
  } else if (prev) {
    window.__gelabberCaptureDiag = [prev, diag];
  } else {
    window.__gelabberCaptureDiag = diag;
  }
}

/**
 * Compare requested mic constraints vs track.getSettings(), log + stash,
 * and when diag is enabled download capture-settings-<iso>.json.
 */
export function logCaptureSettings(
  track: MediaStreamTrack,
  requested: MediaTrackConstraints,
): CaptureSettingsDiag {
  const applied =
    typeof track.getSettings === "function" ? track.getSettings() : {};
  const comparison = compareCaptureSettings(requested, applied);
  const contentHint =
    "contentHint" in track
      ? String(
          (track as MediaStreamTrack & { contentHint?: string }).contentHint ??
            "",
        )
      : null;
  const diag: CaptureSettingsDiag = {
    at: new Date().toISOString(),
    userAgent:
      typeof navigator !== "undefined" ? navigator.userAgent : null,
    track: {
      id: track.id,
      label: track.label,
      kind: track.kind,
      readyState: track.readyState,
      contentHint,
    },
    requested: { ...requested },
    applied,
    comparison,
    playtestSaveHint: PLAYTEST_HINT,
  };
  console.info("[gelabberDiag] capture settings", diag);
  stashDiag(diag);
  if (isCaptureDiagEnabled()) {
    const body = JSON.stringify(diag, null, 2);
    downloadBlob(
      new Blob([body], { type: "application/json" }),
      `capture-settings-${isoStamp()}.json`,
    );
  }
  return diag;
}

function pickRecorderMime(): string | undefined {
  if (typeof MediaRecorder === "undefined") return undefined;
  const candidates = [
    "audio/webm;codecs=opus",
    "audio/webm",
    "audio/ogg;codecs=opus",
  ];
  for (const mime of candidates) {
    if (MediaRecorder.isTypeSupported?.(mime)) return mime;
  }
  return undefined;
}

function startAbRecord(
  track: MediaStreamTrack,
  side: "local" | "remote",
): AbRecordHandle | null {
  if (
    typeof MediaRecorder === "undefined" ||
    typeof MediaStream === "undefined"
  ) {
    console.info(`[gelabberDiag] MediaRecorder unavailable for ${side} A/B`);
    return null;
  }
  let cloned: MediaStreamTrack;
  try {
    cloned = track.clone();
  } catch (error) {
    console.info(`[gelabberDiag] track.clone failed for ${side}`, error);
    return null;
  }
  const stream = new MediaStream([cloned]);
  const mime = pickRecorderMime();
  const chunks: BlobPart[] = [];
  let recorder: MediaRecorder;
  try {
    recorder = mime
      ? new MediaRecorder(stream, { mimeType: mime })
      : new MediaRecorder(stream);
  } catch (error) {
    cloned.stop();
    console.info(
      `[gelabberDiag] MediaRecorder start failed for ${side}`,
      error,
    );
    return null;
  }
  recorder.ondataavailable = (ev) => {
    if (ev.data && ev.data.size > 0) chunks.push(ev.data);
  };
  const stop = (): Promise<void> =>
    new Promise((resolve) => {
      if (recorder.state === "inactive") {
        cloned.stop();
        resolve();
        return;
      }
      recorder.onstop = () => {
        const type = recorder.mimeType || mime || "audio/webm";
        const blob = new Blob(chunks, { type });
        const ext = type.includes("ogg") ? "ogg" : "webm";
        downloadBlob(blob, `${side}-${isoStamp()}.${ext}`);
        cloned.stop();
        resolve();
      };
      try {
        recorder.stop();
      } catch {
        cloned.stop();
        resolve();
      }
    });
  recorder.start(1_000);
  console.info(
    `[gelabberDiag] ${side} A/B MediaRecorder started (~${AB_MS / 1000}s)`,
  );
  return { side, stop };
}

function clearAutoStop(): void {
  if (ab.autoStopTimer) {
    clearTimeout(ab.autoStopTimer);
    ab.autoStopTimer = null;
  }
}

function scheduleAutoStop(): void {
  if (ab.autoStopTimer) return;
  ab.autoStopTimer = setTimeout(() => {
    ab.autoStopTimer = null;
    void stopAbRecords();
  }, AB_MS);
}

/** Record a clone of the local mic track (pre-SFU). */
export function startLocalAbRecord(
  track: MediaStreamTrack,
): AbRecordHandle | null {
  ab.lastLocalTrack = track;
  installCaptureDiagWindowHooks();
  if (!isCaptureDiagEnabled()) return null;
  if (ab.local) return ab.local;
  const handle = startAbRecord(track, "local");
  ab.local = handle;
  if (handle) scheduleAutoStop();
  return handle;
}

/** Record remoteMix / incoming audio once (post-SFU path). */
export function startRemoteAbRecord(
  track: MediaStreamTrack,
): AbRecordHandle | null {
  ab.lastRemoteTrack = track;
  installCaptureDiagWindowHooks();
  if (!isCaptureDiagEnabled()) return null;
  if (ab.remoteStarted || ab.remote) return ab.remote;
  ab.remoteStarted = true;
  const handle = startAbRecord(track, "remote");
  ab.remote = handle;
  if (handle) scheduleAutoStop();
  return handle;
}

export async function stopAbRecords(): Promise<void> {
  clearAutoStop();
  const local = ab.local;
  const remote = ab.remote;
  ab.local = null;
  ab.remote = null;
  await Promise.all(
    [local?.stop(), remote?.stop()].filter(
      (p): p is Promise<void> => p !== undefined,
    ),
  );
  console.info("[gelabberDiag] A/B records stopped (downloads if any)");
}

/** Manual start from console using last seen tracks. */
export function startAbRecordsFromWindow(): void {
  if (!isCaptureDiagEnabled()) {
    console.info(
      "[gelabberDiag] enable ?gelabberDiag=1 or localStorage gelabberDiag=1 first",
    );
    return;
  }
  if (ab.lastLocalTrack && !ab.local) {
    ab.local = startAbRecord(ab.lastLocalTrack, "local");
  }
  if (ab.lastRemoteTrack && !ab.remote) {
    ab.remoteStarted = true;
    ab.remote = startAbRecord(ab.lastRemoteTrack, "remote");
  }
  if (ab.local || ab.remote) scheduleAutoStop();
}

/** Reset one-shot remote flag (e.g. on leave). Does not stop active recorders. */
export function resetCaptureDiagSession(): void {
  void stopAbRecords();
  ab.remoteStarted = false;
  ab.lastLocalTrack = null;
  ab.lastRemoteTrack = null;
}

export function installCaptureDiagWindowHooks(): void {
  if (typeof window === "undefined") return;
  window.__gelabberStartAbRecord = () => {
    startAbRecordsFromWindow();
  };
  window.__gelabberStopAbRecord = () => {
    void stopAbRecords();
  };
}

declare global {
  interface Window {
    __gelabberCaptureDiag?: CaptureSettingsDiag | CaptureSettingsDiag[];
    __gelabberStartAbRecord?: () => void;
    __gelabberStopAbRecord?: () => void;
  }
}
