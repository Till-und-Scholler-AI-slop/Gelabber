// Volume and local mute per participant, on top of the output volume.
// Persisted per browser like the media settings; only users away from the
// default (100 %, not muted) are stored. Affects only what this client plays:
// nobody else hears a difference.

import { create } from "zustand";
import { clampPlaybackVolume, MAX_PLAYBACK_VOLUME } from "./settings.ts";

const STORAGE_KEY = "gelabber.user-volumes";

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

function load(): UserVolumes {
  try {
    return sanitize(JSON.parse(localStorage.getItem(STORAGE_KEY) ?? "null"));
  } catch {
    return {};
  }
}

function save(users: UserVolumes): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(users));
  } catch {
    // Keep the controls usable when storage is disabled.
  }
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
    save(users);
    set({ users });
  };
  return {
    users: load(),
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

/** Tests: forget every participant's level, in memory and in storage. */
export function resetUserVolumesForTests(): void {
  try {
    localStorage.removeItem(STORAGE_KEY);
  } catch {
    // No storage in this environment.
  }
  useUserVolumes.setState({ users: {} });
}
