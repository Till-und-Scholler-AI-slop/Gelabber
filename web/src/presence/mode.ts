// Manual presence (#590): Automatic, Away, Do not disturb or Invisible, for
// the whole account. The server shows others the mode on top of what the
// sockets report (`d` for Do not disturb, offline for Invisible) and tells
// the account's own sockets the mode itself (`pm`), which this store holds.
//
// While Do not disturb lasts this page plays no message sounds and shows no
// toasts or system notifications; unread badges stay. While Invisible it
// sends no typing (the server drops it anyway).

import { useEffect } from "react";
import { create } from "zustand";

import { api } from "../api/client.ts";
import { formatDate, formatTime } from "../i18n/format.ts";
import { translate } from "../i18n/catalog.ts";
import { shell } from "../i18n/catalogs/shell.ts";
import { getGateway } from "../ws/client.ts";
import type { PresenceStatus } from "../ws/protocol.ts";

export type PresenceMode = "auto" | "idle" | "dnd" | "invisible";
export const PRESENCE_MODES: readonly PresenceMode[] = [
  "auto",
  "idle",
  "dnd",
  "invisible",
];

export type ModeDuration = "1h" | "8h" | "24h" | "forever";
export const MODE_DURATIONS: readonly ModeDuration[] = [
  "1h",
  "8h",
  "24h",
  "forever",
];

export type ModeState = { mode: PresenceMode; until: string | null };

/** What your own avatar shows: the presence others see, or `v` while you
 * are invisible (they see you offline, you see that you are hidden). */
export type OwnPresence = PresenceStatus | "v";

type Store = ModeState & { userId: string | null };

const AUTO: ModeState = { mode: "auto", until: null };

export const usePresenceMode = create<Store>(() => ({
  ...AUTO,
  userId: null,
}));

export function parseMode(raw: unknown): PresenceMode {
  return typeof raw === "string" &&
    (PRESENCE_MODES as readonly string[]).includes(raw)
    ? (raw as PresenceMode)
    : "auto";
}

/** A mode from the server, as stored here. An end that passed already is
 * automatic. */
export function normalizeMode(
  raw: { mode?: unknown; m?: unknown; until?: unknown; u?: unknown },
  now = Date.now(),
): ModeState {
  const mode = parseMode(raw.mode ?? raw.m);
  const until = raw.until ?? raw.u;
  if (mode === "auto") return AUTO;
  if (typeof until !== "string") return { mode, until: null };
  const end = Date.parse(until);
  if (Number.isNaN(end)) return { mode, until: null };
  return end <= now ? AUTO : { mode, until };
}

/** The mode in effect at `now`, an end that passed counting as automatic. */
export function modeAt(state: ModeState, now = Date.now()): PresenceMode {
  if (state.until !== null && Date.parse(state.until) <= now) return "auto";
  return state.mode;
}

export function isDoNotDisturb(now = Date.now()): boolean {
  return modeAt(usePresenceMode.getState(), now) === "dnd";
}

export function isInvisible(now = Date.now()): boolean {
  return modeAt(usePresenceMode.getState(), now) === "invisible";
}

/** Your own presence dot: the mode wins over what a server reports, which
 * is offline while you are invisible. */
export function ownShown(
  mode: PresenceMode,
  reported: PresenceStatus,
): OwnPresence {
  switch (mode) {
    case "idle":
      return "i";
    case "dnd":
      return "d";
    case "invisible":
      return "v";
    default:
      return reported === "x" ? "o" : reported;
  }
}

/** "Nicht stören bis 18:30", or "bis morgen, 09:00" for an end on another
 * day; just the mode's name without an end. */
export function modeLabel(state: ModeState, now = Date.now()): string {
  const name = translate(shell, `presenceMode.${state.mode}`);
  if (state.mode === "auto" || state.until === null) return name;
  const end = new Date(state.until);
  const sameDay = new Date(now).toDateString() === end.toDateString();
  const time = formatTime(end);
  const when = sameDay
    ? time
    : `${formatDate(end, { weekday: "short" })}, ${time}`;
  return translate(shell, "presenceMode.until", { mode: name, when });
}

function apply(userId: string | null, state: ModeState): void {
  const current = usePresenceMode.getState();
  if (
    current.userId === userId &&
    current.mode === state.mode &&
    current.until === state.until
  )
    return;
  usePresenceMode.setState({ ...state, userId });
}

export const getPresenceMode = (signal?: AbortSignal) =>
  api<ModeState>("/me/presence", { signal });

// Only the newest choice speaks: an older answer arriving late must not
// undo a newer one.
let latest = 0;

/** Sets the mode for the whole account; the store follows at once and the
 * server's answer (or the account frame) confirms it. */
export async function setPresenceMode(
  mode: PresenceMode,
  duration: ModeDuration = "forever",
): Promise<ModeState> {
  const userId = usePresenceMode.getState().userId;
  const before = usePresenceMode.getState();
  const call = ++latest;
  const until =
    mode === "auto" || duration === "forever"
      ? null
      : new Date(Date.now() + hoursOf(duration) * 3_600_000).toISOString();
  apply(userId, { mode, until });
  try {
    const saved = await api<ModeState>("/me/presence", {
      method: "PUT",
      body: mode === "auto" ? { mode } : { mode, duration },
    });
    const state = normalizeMode(saved);
    if (call === latest) apply(userId, state);
    return state;
  } catch (error) {
    if (call === latest)
      apply(userId, { mode: before.mode, until: before.until });
    throw error;
  }
}

const hoursOf = (duration: Exclude<ModeDuration, "forever">) =>
  duration === "1h" ? 1 : duration === "8h" ? 8 : 24;

export function resetPresenceMode(): void {
  latest += 1;
  usePresenceMode.setState({ ...AUTO, userId: null });
}

/** Loads the mode for the signed-in account, follows `pm` from the other
 * devices, reads it again after a reconnect (a frame may have gone by),
 * and turns automatic when the end passes. */
export function usePresenceModeBridge(userId: string | null): void {
  useEffect(() => {
    if (!userId) {
      resetPresenceMode();
      return undefined;
    }
    if (usePresenceMode.getState().userId !== userId) resetPresenceMode();
    usePresenceMode.setState({ userId });
    let alive = true;
    const load = () => {
      const call = latest;
      getPresenceMode()
        .then((saved) => {
          if (alive && call === latest) apply(userId, normalizeMode(saved));
        })
        // Offline: automatic until the next load or frame says otherwise.
        .catch(() => {});
    };
    load();
    const gateway = getGateway();
    const offAccount = gateway.onAccount((frame) => {
      if (frame.op !== "pm") return;
      latest += 1;
      apply(userId, normalizeMode(frame));
    });
    const offReady = gateway.onReady(load);
    let timer: ReturnType<typeof setTimeout> | undefined;
    const schedule = (state: ModeState) => {
      if (timer) clearTimeout(timer);
      timer = undefined;
      if (state.until === null) return;
      // setTimeout holds at most ~24.8 days; a mode lasts a day at most.
      const wait = Math.max(0, Date.parse(state.until) - Date.now());
      timer = setTimeout(() => {
        if (alive && modeAt(usePresenceMode.getState()) === "auto")
          apply(userId, AUTO);
      }, wait + 50);
    };
    schedule(usePresenceMode.getState());
    const offStore = usePresenceMode.subscribe(schedule);
    return () => {
      alive = false;
      if (timer) clearTimeout(timer);
      offAccount();
      offReady();
      offStore();
    };
  }, [userId]);
}
