// TanStack Query layer for the server directory. Keys live under the
// signed-in user's scope like `serverKeys`, so signing out drops them.
// Joins are not optimistic: whether the door is open is the server's call.

import {
  keepPreviousData,
  useInfiniteQuery,
  useMutation,
  useQuery,
  useQueryClient,
  type InfiniteData,
  type QueryClient,
} from "@tanstack/react-query";

import {
  scopeGeneration,
  stampHolds,
  takeStamp,
  useUserId,
} from "../auth/scope.ts";
import { useEffect } from "react";

import { notify } from "../components/toasts.ts";
import { translate } from "../i18n/catalog.ts";
import { discovery } from "../i18n/catalogs/discovery.ts";
import { listServers } from "../servers/api.ts";
import { rememberJoinedServer, serverKeys } from "../servers/queries.ts";
import type { Server } from "../servers/types.ts";
import { getGateway } from "../ws/client.ts";
import type { AccountFrame } from "../ws/protocol.ts";
import * as remote from "./api.ts";
import type {
  DirectoryStatus,
  JoinRequest,
  Listing,
  ListingPage,
  ListingQuery,
  ListingSettingsInput,
  ReportInput,
} from "./types.ts";

export const directoryKeys = {
  all: (userId: string, generation: number) =>
    ["user", userId, generation, "directory"] as const,
  status: (userId: string, generation: number) =>
    ["user", userId, generation, "directory", "status"] as const,
  listings: (userId: string, generation: number) =>
    ["user", userId, generation, "directory", "listings"] as const,
  search: (userId: string, generation: number, query: ListingQuery) =>
    [
      "user",
      userId,
      generation,
      "directory",
      "listings",
      query.q ?? "",
      query.language ?? "",
    ] as const,
  settings: (userId: string, generation: number, serverId: string) =>
    ["user", userId, generation, "directory", "settings", serverId] as const,
  requests: (userId: string, generation: number, serverId: string) =>
    ["user", userId, generation, "directory", "requests", serverId] as const,
  requestCount: (userId: string, generation: number, serverId: string) =>
    [
      "user",
      userId,
      generation,
      "directory",
      "requestCount",
      serverId,
    ] as const,
};

/** Listings and their counts move slowly; a minute is fresh enough. */
const STALE_MS = 60_000;

const statusQuery = (userId: string, generation: number) => ({
  queryKey: directoryKeys.status(userId, generation),
  queryFn: ({ signal }: { signal?: AbortSignal }) =>
    remote.directoryStatus(signal),
  staleTime: STALE_MS,
});

/** `GET /api/directory`: on or off, and the languages with listings. */
export function useDirectoryStatus() {
  const userId = useUserId();
  const generation = scopeGeneration();
  return useQuery({
    ...statusQuery(userId, generation),
    enabled: userId.length > 0,
    // Off, a failed check hides the tile rather than retrying.
    retry: false,
  });
}

/** The status from the cache, else fetched: the `/discover` guard. */
export function ensureDirectoryStatus(
  client: QueryClient,
  stamp: { userId: string; generation: number },
): Promise<DirectoryStatus> {
  return client.ensureQueryData(statusQuery(stamp.userId, stamp.generation));
}

/** The directory, 30 rows a page, for one search and language. */
export function useListings(
  q: string,
  language: string | undefined,
  enabled = true,
) {
  const userId = useUserId();
  const generation = scopeGeneration();
  const query: ListingQuery = { q: q || undefined, language };
  return useInfiniteQuery({
    queryKey: directoryKeys.search(userId, generation, query),
    queryFn: ({ pageParam, signal }) =>
      remote.listListings(query, pageParam, signal),
    initialPageParam: 0,
    getNextPageParam: (page: ListingPage) => page.next_offset ?? undefined,
    enabled: userId.length > 0 && enabled,
    staleTime: STALE_MS,
    // A new search keeps the old rows until its own arrive.
    placeholderData: keepPreviousData,
  });
}

/** Rewrites one server's row in every cached search. */
function patchListing(
  client: QueryClient,
  stamp: { userId: string; generation: number },
  serverId: string,
  update: (listing: Listing) => Listing,
): void {
  client.setQueriesData<InfiniteData<ListingPage, number>>(
    { queryKey: directoryKeys.listings(stamp.userId, stamp.generation) },
    (data) =>
      data && {
        ...data,
        pages: data.pages.map((page) => ({
          ...page,
          listings: page.listings.map((listing) =>
            listing.server_id === serverId ? update(listing) : listing,
          ),
        })),
      },
  );
}

/** After a refusal the row is out of date (a new mode, a ban, a delist). */
function refreshDirectory(
  client: QueryClient,
  stamp: { userId: string; generation: number },
): void {
  void client.invalidateQueries({
    queryKey: directoryKeys.all(stamp.userId, stamp.generation),
  });
}

/** "Beitreten": the server joins the list, the row turns into "Öffnen". */
export function useJoinListing() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (serverId: string) => remote.joinListing(serverId),
    onMutate: () => takeStamp(),
    onSuccess: (server, serverId, stamp) => {
      if (!stampHolds(stamp)) return;
      rememberJoinedServer(client, stamp, server);
      patchListing(client, stamp, serverId, (listing) => ({
        ...listing,
        member: true,
        request: null,
        retry_at: null,
        member_count: listing.member
          ? listing.member_count
          : listing.member_count + 1,
      }));
    },
    onError: (_error, _serverId, stamp) => {
      if (stampHolds(stamp)) refreshDirectory(client, stamp);
    },
  });
}

/** "Beitritt anfragen", with an optional message. */
export function useRequestJoin() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: ({
      serverId,
      message,
    }: {
      serverId: string;
      message: string;
    }) => remote.requestJoin(serverId, message.trim()),
    onMutate: () => takeStamp(),
    onSuccess: (answer, { serverId }, stamp) => {
      if (!stampHolds(stamp)) return;
      patchListing(client, stamp, serverId, (listing) => ({
        ...listing,
        request: answer.request,
        retry_at: null,
      }));
    },
    onError: (_error, _input, stamp) => {
      if (stampHolds(stamp)) refreshDirectory(client, stamp);
    },
  });
}

/** "Anfrage zurückziehen". */
export function useWithdrawRequest() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (serverId: string) => remote.withdrawRequest(serverId),
    onMutate: () => takeStamp(),
    onSuccess: (_answer, serverId, stamp) => {
      if (!stampHolds(stamp)) return;
      patchListing(client, stamp, serverId, (listing) => ({
        ...listing,
        request: null,
        retry_at: null,
      }));
    },
    onError: (_error, _serverId, stamp) => {
      if (stampHolds(stamp)) refreshDirectory(client, stamp);
    },
  });
}

/** "Melden": goes to the operator; nothing in the directory changes. */
export function useReportListing() {
  return useMutation({
    mutationFn: ({
      serverId,
      input,
    }: {
      serverId: string;
      input: ReportInput;
    }) => remote.reportListing(serverId, input),
  });
}

/** A server's own listing form (any member may read it). */
export function useListingSettings(serverId: string, enabled: boolean) {
  const userId = useUserId();
  const generation = scopeGeneration();
  return useQuery({
    queryKey: directoryKeys.settings(userId, generation, serverId),
    queryFn: ({ signal }) => remote.getListingSettings(serverId, signal),
    enabled: userId.length > 0 && enabled,
    staleTime: STALE_MS,
  });
}

export function useSaveListingSettings(serverId: string) {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (input: ListingSettingsInput) =>
      remote.putListingSettings(serverId, input),
    onMutate: () => takeStamp(),
    onSuccess: (settings, _input, stamp) => {
      if (!stampHolds(stamp)) return;
      client.setQueryData(
        directoryKeys.settings(stamp.userId, stamp.generation, serverId),
        settings,
      );
      refreshDirectory(client, stamp);
    },
  });
}

/** The pending requests of a server (owner and `moderate_members`). */
export function useJoinRequests(serverId: string, enabled: boolean) {
  const client = useQueryClient();
  const userId = useUserId();
  const generation = scopeGeneration();
  return useQuery({
    queryKey: directoryKeys.requests(userId, generation, serverId),
    queryFn: async ({ signal }) => {
      const requests = await remote.listJoinRequests(serverId, signal);
      // The queue just read is the newest count, too.
      client.setQueryData(
        directoryKeys.requestCount(userId, generation, serverId),
        requests.length,
      );
      return requests;
    },
    enabled: userId.length > 0 && enabled,
    staleTime: STALE_MS,
  });
}

/** How many requests wait in a server's queue: read with the queue, then
 * kept current by the `jq` frame. */
export function useJoinRequestCount(serverId: string, enabled: boolean) {
  const userId = useUserId();
  const generation = scopeGeneration();
  return useQuery({
    queryKey: directoryKeys.requestCount(userId, generation, serverId),
    queryFn: async ({ signal }) =>
      (await remote.listJoinRequests(serverId, signal)).length,
    enabled: userId.length > 0 && enabled,
    staleTime: STALE_MS,
  });
}

/**
 * The directory's account frames: `jn` (a request was accepted) adds the
 * server to the list and says so, `jq` (a queue changed) sets its count
 * and reloads the queue. Both are best effort; the next read catches up.
 */
export function useDirectoryBridge(): void {
  const client = useQueryClient();
  const userId = useUserId();
  const generation = scopeGeneration();
  useEffect(() => {
    if (!userId) return;
    const stamp = { userId, generation };
    return getGateway().onAccount(
      (frame) => void applyDirectoryFrame(client, stamp, frame),
    );
  }, [client, userId, generation]);
}

/** What `jn` and `jq` change in the cache; other frames pass by. */
export async function applyDirectoryFrame(
  client: QueryClient,
  stamp: { userId: string; generation: number },
  frame: AccountFrame,
): Promise<void> {
  if (frame.op === "jn" && typeof frame.s === "string") {
    await announceJoin(client, stamp, frame.s);
    return;
  }
  if (
    frame.op === "jq" &&
    typeof frame.s === "string" &&
    typeof frame.n === "number"
  ) {
    const { userId, generation } = stamp;
    client.setQueryData(
      directoryKeys.requestCount(userId, generation, frame.s),
      frame.n,
    );
    await client.invalidateQueries({
      queryKey: directoryKeys.requests(userId, generation, frame.s),
    });
  }
}

/** `jn`: the server list is read again, then names the new server. */
async function announceJoin(
  client: QueryClient,
  stamp: { userId: string; generation: number },
  serverId: string,
): Promise<void> {
  // The directory row becomes "Öffnen".
  refreshDirectory(client, stamp);
  let servers: Server[];
  try {
    servers = await client.fetchQuery({
      queryKey: serverKeys.list(stamp.userId, stamp.generation),
      queryFn: ({ signal }) => listServers(signal),
      staleTime: 0,
    });
  } catch {
    return;
  }
  if (!stampHolds(stamp)) return;
  const server = servers.find((entry) => entry.id === serverId);
  if (server)
    notify(translate(discovery, "toasts.joined", { name: server.name }));
}

/** "Annehmen" or "Ablehnen": the row leaves the queue either way. */
export function useAnswerJoinRequest(serverId: string) {
  const client = useQueryClient();
  return useMutation({
    mutationFn: ({ userId, accept }: { userId: string; accept: boolean }) =>
      accept
        ? remote.acceptJoinRequest(serverId, userId)
        : remote.declineJoinRequest(serverId, userId),
    onMutate: () => takeStamp(),
    onSuccess: (_answer, { userId }, stamp) => {
      if (!stampHolds(stamp)) return;
      // Off the list before the buttons come back, so it is not answered twice.
      client.setQueryData<JoinRequest[]>(
        directoryKeys.requests(stamp.userId, stamp.generation, serverId),
        (requests) => requests?.filter((request) => request.user_id !== userId),
      );
    },
    onSettled: (_answer, error, { accept }, stamp) => {
      if (!stampHolds(stamp)) return;
      settleJoinRequest(client, stamp, serverId, accept && !error);
    },
  });
}

/** After an answer the queue is read again; an accepted request also adds a
 * member, so the server's member list is read again too. */
export function settleJoinRequest(
  client: QueryClient,
  stamp: { userId: string; generation: number },
  serverId: string,
  accepted: boolean,
): void {
  const { userId, generation } = stamp;
  void client.invalidateQueries({
    queryKey: directoryKeys.requests(userId, generation, serverId),
  });
  if (accepted)
    void client.invalidateQueries({
      queryKey: serverKeys.detail(userId, generation, serverId),
    });
}
