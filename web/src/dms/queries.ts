// TanStack Query layer for 1:1 DMs.
//
// Feel rule: opening an already-known DM is a cache hit and navigates in
// the same frame. The first open waits on POST /api/dms (idempotent);
// the list is prefetched with the workspace so that path is rare.

import {
  useMutation,
  useQuery,
  useQueryClient,
  type QueryClient,
} from "@tanstack/react-query";

import { notifyError } from "../components/toasts.ts";
import * as remote from "./api.ts";
import type { DirectMessage } from "./types.ts";

export const dmKeys = {
  all: ["dms"] as const,
  list: () => ["dms", "list"] as const,
  detail: (id: string) => ["dms", "detail", id] as const,
};

const STALE_MS = 60_000;

export function useDms() {
  return useQuery({
    queryKey: dmKeys.list(),
    queryFn: ({ signal }) => remote.listDms(signal),
    staleTime: STALE_MS,
  });
}

export function useDm(id: string | undefined) {
  return useQuery({
    queryKey: dmKeys.detail(id ?? ""),
    queryFn: ({ signal }) => remote.getDm(id ?? "", signal),
    enabled: id !== undefined,
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

export function prefetchDms(client: QueryClient): void {
  void client.prefetchQuery({
    queryKey: dmKeys.list(),
    queryFn: ({ signal }) => remote.listDms(signal),
    staleTime: STALE_MS,
  });
}

function remember(client: QueryClient, dm: DirectMessage): void {
  client.setQueryData<DirectMessage>(dmKeys.detail(dm.id), dm);
  client.setQueryData<DirectMessage[]>(dmKeys.list(), (current) => {
    if (!current) return [dm];
    if (current.some((row) => row.id === dm.id)) {
      return current.map((row) => (row.id === dm.id ? dm : row));
    }
    return [dm, ...current];
  });
}

export function findCachedDm(
  client: QueryClient,
  peerId: string,
): DirectMessage | undefined {
  const list = client.getQueryData<DirectMessage[]>(dmKeys.list());
  return list?.find((dm) => dm.peer.id === peerId);
}

export function useOpenDm() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (userId: string) => remote.openDm(userId),
    onSuccess: (dm) => remember(client, dm),
    onError: (error) => notifyError(error),
  });
}

export function forgetDm(client: QueryClient, id: string): void {
  client.removeQueries({ queryKey: dmKeys.detail(id) });
  client.setQueryData<DirectMessage[]>(dmKeys.list(), (current) =>
    current?.filter((dm) => dm.id !== id),
  );
}
