// One function per directory route. No caching here; `queries.ts` owns that.

import { api } from "../api/client.ts";
import type { Server } from "../servers/types.ts";
import type {
  DirectoryStatus,
  JoinRequest,
  ListingPage,
  ListingQuery,
  ListingSettings,
  ListingSettingsInput,
  ReportInput,
  RequestState,
} from "./types.ts";

export const directoryStatus = (signal?: AbortSignal) =>
  api<DirectoryStatus>("/directory", { signal });

export function listingsPath(query: ListingQuery, offset: number): string {
  const search = new URLSearchParams();
  if (query.q) search.set("q", query.q);
  if (query.language) search.set("language", query.language);
  if (offset > 0) search.set("offset", String(offset));
  const text = search.toString();
  return `/directory/listings${text ? `?${text}` : ""}`;
}

export const listListings = (
  query: ListingQuery,
  offset: number,
  signal?: AbortSignal,
) => api<ListingPage>(listingsPath(query, offset), { signal });

/** An open server: the caller becomes a member; the answer is the list row. */
export const joinListing = (serverId: string) =>
  api<Server>(`/directory/listings/${serverId}/join`, { method: "POST" });

export const requestJoin = (serverId: string, message: string) =>
  api<{ request: RequestState }>(`/directory/listings/${serverId}/request`, {
    method: "POST",
    body: { message },
  });

export const withdrawRequest = (serverId: string) =>
  api<null>(`/directory/listings/${serverId}/request`, { method: "DELETE" });

/** Operator's report route, open to members and to whoever sees the listing. */
export const reportListing = (serverId: string, input: ReportInput) =>
  api<unknown>(`/servers/${serverId}/reports`, {
    method: "POST",
    body: input,
  });

export const getListingSettings = (serverId: string, signal?: AbortSignal) =>
  api<ListingSettings>(`/servers/${serverId}/listing`, { signal });

export const putListingSettings = (
  serverId: string,
  input: ListingSettingsInput,
) =>
  api<ListingSettings>(`/servers/${serverId}/listing`, {
    method: "PUT",
    body: input,
  });

export const listJoinRequests = (serverId: string, signal?: AbortSignal) =>
  api<JoinRequest[]>(`/servers/${serverId}/join-requests`, { signal });

export const acceptJoinRequest = (serverId: string, userId: string) =>
  api<null>(`/servers/${serverId}/join-requests/${userId}/accept`, {
    method: "POST",
  });

export const declineJoinRequest = (serverId: string, userId: string) =>
  api<null>(`/servers/${serverId}/join-requests/${userId}/decline`, {
    method: "POST",
  });
