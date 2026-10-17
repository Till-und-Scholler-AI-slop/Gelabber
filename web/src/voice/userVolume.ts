// Volume and local mute per participant, on top of the output volume.
// Persisted per browser and per own account, like a preference of that
// account; only users away from the default (100 %, not muted) are stored.
// Affects only what this client plays: nobody else hears a difference.

import { create } from "zustand";
import { clampPlaybackVolume, MAX_PLAYBACK_VOLUME } from "./settings.ts";

const STORAGE_PREFIX = "gelabber.user-volumes:";
/** A slider drag writes once it rests, not on every step. */
const SAVE_DELAY_MS = 400;

export type UserVolume = { volume: number; muted: boolean };
export const DEFAULT_USER_VOLUME: UserVolume = { volume: 1, muted: false };

type UserVolumes = Record<string, UserVolume>;

type UserVolumeState = {
  users: UserVolumes;
  setVolume: (userId: string, volume: number) => void;
  setMuted: (userId: string, muted: boolean) => void;
  reset: (userId: string) => void;
};

function sanitize(saved: unknown): UserVolumes {
  if (!saved || typeof saved !== "object") return {};
  const users: UserVolumes = {};
  for (const [id, value] of Object.entries(saved)) {
    if (!id || !value || typeof value !== "object") continue;
    const entry = value as { volume?: unknown; muted?: unknown };
    const volume = clampPlaybackVolume(
      typeof entry.volume === "number" ? entry.volume : 1,
    );
    const muted = entry.muted === true;
    if (volume !== 1 || muted) users[id] = { volume, muted };
  }
  return users;
}

/** Whose levels the store holds: the signed-in account, or nobody. */
let owner: string | null = null;
let pending: { owner: string; users: UserVolumes } | null = null;
let saveTimer: ReturnType<typeof setTimeout> | null = null;

function load(account: string | null): UserVolumes {
  if (!account) return {};
  try {
    return sanitize(
      JSON.parse(localStorage.getItem(STORAGE_PREFIX + account) ?? "null"),
    );
  } catch {
    return {};
  }
}

/** Writes a change still waiting for its delay. */
export function flushUserVolumes(): void {
  if (saveTimer !== null) clearTimeout(saveTimer);
  saveTimer = null;
  const write = pending;
  pending = null;
  if (!write) return;
  try {
    const key = STORAGE_PREFIX + write.owner;
    if (Object.keys(write.users).length === 0) localStorage.removeItem(key);
    else localStorage.setItem(key, JSON.stringify(write.users));
  } catch {
    // Keep the controls usable when storage is disabled.
  }
}

function scheduleSave(users: UserVolumes): void {
  // Signed out, levels last for the page only.
  if (!owner) return;
  pending = { owner, users };
  if (saveTimer !== null) clearTimeout(saveTimer);
  saveTimer = setTimeout(flushUserVolumes, SAVE_DELAY_MS);
}

function update(
  users: UserVolumes,
  userId: string,
  next: Partial<UserVolume>,
): UserVolumes {
  const entry = { ...(users[userId] ?? DEFAULT_USER_VOLUME), ...next };
  const rest = { ...users };
  delete rest[userId];
  const volume = clampPlaybackVolume(entry.volume);
  if (volume === 1 && !entry.muted) return rest;
  return { ...rest, [userId]: { volume, muted: entry.muted } };
}

export const useUserVolumes = create<UserVolumeState>((set) => {
  const write = (users: UserVolumes) => {
    scheduleSave(users);
    set({ users });
  };
  return {
    users: {},
    setVolume: (userId, volume) =>
      write(update(useUserVolumes.getState().users, userId, { volume })),
    setMuted: (userId, muted) =>
      write(update(useUserVolumes.getState().users, userId, { muted })),
    reset: (userId) =>
      write(
        update(useUserVolumes.getState().users, userId, DEFAULT_USER_VOLUME),
      ),
  };
});

/**
 * Switches to the levels of the account that now owns the tab (null:
 * nobody). Called from `releaseUserScope` on every identity change, like
 * the drafts and themes, so this module needs no session import.
 */
export function bindUserVolumeAccount(account: string | null): void {
  if (account === owner) return;
  flushUserVolumes();
  owner = account;
  useUserVolumes.setState({ users: load(account) });
}

// A drag the page is closed in the middle of still lands.
if (typeof window !== "undefined" && window.addEventListener) {
  window.addEventListener("pagehide", flushUserVolumes);
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "hidden") flushUserVolumes();
  });
}

export function userVolumeOf(
  userId: string | null | undefined,
  users: UserVolumes = useUserVolumes.getState().users,
): UserVolume {
  return (userId && users[userId]) || DEFAULT_USER_VOLUME;
}

/**
 * What a participant's voice plays at: the output volume times their own
 * volume, up to the 200 % the boost and the native core allow. Deafen and a
 * local mute silence it; the per-user level comes back with them.
 */
export function effectiveVoiceVolume(input: {
  output: number;
  user: UserVolume;
  deafened: boolean;
}): number {
  if (input.deafened || input.user.muted) return 0;
  return Math.min(
    MAX_PLAYBACK_VOLUME,
    clampPlaybackVolume(input.output) * clampPlaybackVolume(input.user.volume),
  );
}

/** Tests: forget every participant's level and the account in memory;
 * storage stays. */
export function resetUserVolumesForTests(): void {
  if (saveTimer !== null) clearTimeout(saveTimer);
  saveTimer = null;
  pending = null;
  owner = null;
  useUserVolumes.setState({ users: {} });
}
