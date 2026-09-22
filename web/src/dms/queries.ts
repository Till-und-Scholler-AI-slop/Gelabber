// TanStack Query layer for 1:1 DMs.
//
// Feel rule: opening an already-known DM is a cache hit and navigates in
// the same frame. The first open waits on POST /api/dms (idempotent);
// the list is prefetched with the workspace so that path is rare.
//
// Keys include the signed-in user. A response that belongs to someone else
// is ignored, so logout → login cannot show the previous account's DMs.

import {
  useMutation,
  useQuery,
  useQueryClient,
  type QueryClient,
} from "@tanstack/react-query";

import {
  scopeGeneration,
  stampHolds,
  takeStamp,
  useUserId,
} from "../auth/scope.ts";
import { notifyError } from "../components/toasts.ts";
import * as remote from "./api.ts";
import type { DirectMessage } from "./types.ts";

export const dmKeys = {
  all: (userId: string, generation: number) =>
    ["user", userId, generation, "dms"] as const,
  list: (userId: string, generation: number) =>
    ["user", userId, generation, "dms", "list"] as const,
  detail: (userId: string, generation: number, id: string) =>
    ["user", userId, generation, "dms", "detail", id] as const,
};

const STALE_MS = 60_000;

export function useDms() {
  const userId = useUserId();
  const generation = scopeGeneration();
  return useQuery({
    queryKey: dmKeys.list(userId, generation),
    queryFn: ({ signal }) => remote.listDms(signal),
    enabled: userId.length > 0,
    staleTime: STALE_MS,
  });
}

export function useDm(id: string | undefined, enabled = true) {
  const userId = useUserId();
  const generation = scopeGeneration();
  return useQuery({
    queryKey: dmKeys.detail(userId, generation, id ?? ""),
    queryFn: ({ signal }) => remote.getDm(id ?? "", signal),
    enabled: userId.length > 0 && id !== undefined && enabled,
    staleTime: STALE_MS,
    retry: (count, error) =>
      count < 2 &&
      !(
        error instanceof Error &&
        "code" in error &&
        (error.code === "not_found" || error.code === "forbidden")
      ),
  });
}

export function prefetchDms(client: QueryClient, userId: string): void {
  const generation = scopeGeneration();
  void client.prefetchQuery({
    queryKey: dmKeys.list(userId, generation),
    queryFn: ({ signal }) => remote.listDms(signal),
    staleTime: STALE_MS,
  });
}

function remember(
  client: QueryClient,
  userId: string,
  generation: number,
  dm: DirectMessage,
): void {
  client.setQueryData<DirectMessage>(
    dmKeys.detail(userId, generation, dm.id),
    dm,
  );
  client.setQueryData<DirectMessage[]>(
    dmKeys.list(userId, generation),
    (current) => {
      if (!current) return [dm];
      if (current.some((row) => row.id === dm.id)) {
        return current.map((row) => (row.id === dm.id ? dm : row));
      }
      return [dm, ...current];
    },
  );
}

export function findCachedDm(
  client: QueryClient,
  userId: string,
  peerId: string,
): DirectMessage | undefined {
  const list = client.getQueryData<DirectMessage[]>(
    dmKeys.list(userId, scopeGeneration()),
  );
  return list?.find((dm) => dm.peer.id === peerId);
}

/**
 * Commit an opened DM only while `stamp` is still the signed-in user.
 * A late POST from the previous account must not fill the next cache.
 */
export function cacheOpenedDm(
  client: QueryClient,
  stamp: { userId?: string; generation?: number } | undefined,
  dm: DirectMessage,
): void {
  if (!stampHolds(stamp)) return;
  remember(client, stamp.userId, stamp.generation, dm);
}

export function useOpenDm() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (userId: string) => remote.openDm(userId),
    onMutate: () => takeStamp(),
    onSuccess: (dm, _userId, stamp) => cacheOpenedDm(client, stamp, dm),
    onError: (error, _userId, stamp) => {
      if (!stampHolds(stamp)) return;
      notifyError(error);
    },
  });
}

/**
 * Drop a DM the caller can no longer see. `keepDetail` leaves the (failed)
 * detail query alone while a page still observes it — removing it would
 * refetch and 404 again.
 */
export function forgetDm(
  client: QueryClient,
  userId: string,
  id: string,
  options: { keepDetail?: boolean } = {},
): void {
  const generation = scopeGeneration();
  if (!options.keepDetail) {
    client.removeQueries({ queryKey: dmKeys.detail(userId, generation, id) });
  }
  client.setQueryData<DirectMessage[]>(
    dmKeys.list(userId, generation),
    (current) => current?.filter((dm) => dm.id !== id),
  );
}
