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
import { rememberJoinedServer } from "../servers/queries.ts";
import * as remote from "./api.ts";
import type {
  DirectoryStatus,
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
export function useListings(q: string, language: string | undefined) {
  const userId = useUserId();
  const generation = scopeGeneration();
  const query: ListingQuery = { q: q || undefined, language };
  return useInfiniteQuery({
    queryKey: directoryKeys.search(userId, generation, query),
    queryFn: ({ pageParam, signal }) =>
      remote.listListings(query, pageParam, signal),
    initialPageParam: 0,
    getNextPageParam: (page: ListingPage) => page.next_offset ?? undefined,
    enabled: userId.length > 0,
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
  const userId = useUserId();
  const generation = scopeGeneration();
  return useQuery({
    queryKey: directoryKeys.requests(userId, generation, serverId),
    queryFn: ({ signal }) => remote.listJoinRequests(serverId, signal),
    enabled: userId.length > 0 && enabled,
    staleTime: STALE_MS,
  });
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
    onSettled: (_answer, _error, _input, stamp) => {
      if (!stampHolds(stamp)) return;
      void client.invalidateQueries({
        queryKey: directoryKeys.requests(
          stamp.userId,
          stamp.generation,
          serverId,
        ),
      });
    },
  });
}
