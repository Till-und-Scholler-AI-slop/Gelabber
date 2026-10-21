// The server or DM the user was in last, so "/" (app start, PWA start_url,
// desktop window) reopens it instead of always the first server. Persisted
// per browser and per account; the router records it on every navigation.

import { create } from "zustand";
import { persist } from "zustand/middleware";

export type Place = { kind: "server" | "dm"; id: string };

type LastPlaceState = {
  byUser: Record<string, Place>;
  remember: (userId: string, place: Place) => void;
};

export const useLastPlace = create<LastPlaceState>()(
  persist(
    (set, get) => ({
      byUser: {},
      remember: (userId, place) => {
        const current = get().byUser[userId];
        if (current?.kind === place.kind && current.id === place.id) return;
        set((state) => ({ byUser: { ...state.byUser, [userId]: place } }));
      },
    }),
    {
      name: "gelabber.last-place",
      version: 1,
      partialize: (state) => ({ byUser: state.byUser }),
    },
  ),
);

/** The place a workspace path belongs to; `null` for anything else. */
export function placeFromPath(pathname: string): Place | null {
  const server = /^\/s\/([^/]+)(?:\/(?:c\/[^/]+)?)?$/.exec(pathname);
  if (server) return { kind: "server", id: decodeURIComponent(server[1]) };
  const dm = /^\/d\/([^/]+)\/?$/.exec(pathname);
  if (dm) return { kind: "dm", id: decodeURIComponent(dm[1]) };
  return null;
}

/**
 * Where "/" should go: the remembered place while its server or DM is still
 * listed, otherwise the first server. `undefined` while the lists needed to
 * decide are still loading; `null` when there is nowhere to go.
 */
export function startPlace(
  remembered: Place | undefined,
  servers: readonly { id: string }[] | undefined,
  dms: readonly { id: string }[] | undefined,
  dmsSettled: boolean,
): Place | null | undefined {
  if (!servers) return undefined;
  if (remembered?.kind === "server") {
    if (servers.some((server) => server.id === remembered.id))
      return remembered;
  } else if (remembered?.kind === "dm") {
    if (dms?.some((dm) => dm.id === remembered.id)) return remembered;
    if (!dmsSettled) return undefined;
  }
  const first = servers[0];
  return first ? { kind: "server", id: first.id } : null;
}
