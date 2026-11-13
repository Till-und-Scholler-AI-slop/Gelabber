// TanStack Query layer for the operator area "Betrieb". Keys live under the
// signed-in user's scope, so signing out drops them with everything else.
// Writes are not optimistic: an operator action changes what other people
// can do, so the page shows it once the server has done it.

import {
  keepPreviousData,
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";

import { scopeGeneration, useUserId } from "../auth/scope.ts";
import { useSession } from "../auth/store.ts";
import * as remote from "./api.ts";
import { ApiError } from "../api/client.ts";
import { mailPendingRefetch } from "./feedbackMail.ts";
import type {
  AccountQuery,
  FeedbackQuery,
  InvitationQuery,
  ReportQuery,
  ServerQuery,
} from "./types.ts";

export const operatorKeys = {
  all: (userId: string, generation: number) =>
    ["user", userId, generation, "operator"] as const,
  overview: (userId: string, generation: number) =>
    ["user", userId, generation, "operator", "overview"] as const,
  accounts: (userId: string, generation: number, query: AccountQuery) =>
    ["user", userId, generation, "operator", "accounts", query] as const,
  servers: (userId: string, generation: number, query: ServerQuery) =>
    ["user", userId, generation, "operator", "servers", query] as const,
  reports: (userId: string, generation: number, query: ReportQuery) =>
    ["user", userId, generation, "operator", "reports", query] as const,
  invitations: (userId: string, generation: number, query: InvitationQuery) =>
    ["user", userId, generation, "operator", "invitations", query] as const,
  usage: (userId: string, generation: number) =>
    ["user", userId, generation, "operator", "usage"] as const,
  feedback: (userId: string, generation: number, query: FeedbackQuery) =>
    ["user", userId, generation, "operator", "feedback", query] as const,
  feedbackEntry: (userId: string, generation: number, id: string) =>
    ["user", userId, generation, "operator", "feedbackEntry", id] as const,
};

/** The reports badge refreshes this often while the area is open. */
export const OVERVIEW_REFETCH_MS = 60_000;

/** Only an operator asks: anyone else would get a 404 for nothing. */
function useOperatorScope() {
  const userId = useUserId();
  const operator = useSession((state) => state.operator);
  return {
    userId,
    generation: scopeGeneration(),
    enabled: userId.length > 0 && operator,
  };
}

/** `GET /api/operator`: registration mode, mail, and the open reports. */
export function useOperatorOverview() {
  const { userId, generation, enabled } = useOperatorScope();
  return useQuery({
    queryKey: operatorKeys.overview(userId, generation),
    queryFn: ({ signal }) => remote.getOverview(signal),
    enabled,
    refetchInterval: OVERVIEW_REFETCH_MS,
  });
}

export function useOperatorAccounts(query: AccountQuery) {
  const { userId, generation, enabled } = useOperatorScope();
  return useQuery({
    queryKey: operatorKeys.accounts(userId, generation, query),
    queryFn: ({ signal }) => remote.listAccounts(query, signal),
    enabled,
    placeholderData: keepPreviousData,
  });
}

export function useOperatorServers(query: ServerQuery) {
  const { userId, generation, enabled } = useOperatorScope();
  return useQuery({
    queryKey: operatorKeys.servers(userId, generation, query),
    queryFn: ({ signal }) => remote.listServerAdmins(query, signal),
    enabled,
    placeholderData: keepPreviousData,
  });
}

export function useOperatorReports(query: ReportQuery) {
  const { userId, generation, enabled } = useOperatorScope();
  return useQuery({
    queryKey: operatorKeys.reports(userId, generation, query),
    queryFn: ({ signal }) => remote.listReports(query, signal),
    enabled,
    placeholderData: keepPreviousData,
  });
}

export function useOperatorInvitations(query: InvitationQuery) {
  const { userId, generation, enabled } = useOperatorScope();
  return useQuery({
    queryKey: operatorKeys.invitations(userId, generation, query),
    queryFn: ({ signal }) => remote.listInvitations(query, signal),
    enabled,
    placeholderData: keepPreviousData,
  });
}

export function useOperatorUsage() {
  const { userId, generation, enabled } = useOperatorScope();
  return useQuery({
    queryKey: operatorKeys.usage(userId, generation),
    queryFn: ({ signal }) => remote.getUsage(signal),
    enabled,
  });
}

export function useOperatorFeedback(query: FeedbackQuery) {
  const { userId, generation, enabled } = useOperatorScope();
  return useQuery({
    queryKey: operatorKeys.feedback(userId, generation, query),
    queryFn: ({ signal }) => remote.listFeedback(query, signal),
    enabled,
    placeholderData: keepPreviousData,
    refetchInterval: ({ state }) =>
      mailPendingRefetch(state.data?.entries ?? []),
  });
}

/** One entry, for the link in an operator mail (`?entry=`). */
export function useOperatorFeedbackEntry(id: string | undefined) {
  const { userId, generation, enabled } = useOperatorScope();
  return useQuery({
    queryKey: operatorKeys.feedbackEntry(userId, generation, id ?? ""),
    queryFn: ({ signal }) => remote.getFeedback(id ?? "", signal),
    enabled: enabled && id !== undefined,
    refetchInterval: ({ state }) =>
      mailPendingRefetch(state.data ? [state.data] : []),
    // A deleted entry stays deleted; say so at once.
    retry: (count, error) =>
      count < 2 && !(error instanceof ApiError && error.code === "not_found"),
  });
}

/**
 * Runs one operator action (block, disable, resolve, …) and then refetches
 * the whole area: one action changes several pages and the reports badge.
 */
export function useOperatorAction<Input, Result>(
  action: (input: Input) => Promise<Result>,
) {
  const client = useQueryClient();
  const userId = useUserId();
  return useMutation({
    mutationFn: action,
    onSettled: () =>
      client.invalidateQueries({
        queryKey: operatorKeys.all(userId, scopeGeneration()),
      }),
  });
}

/**
 * Refetches the whole area, for an action run outside `useOperatorAction`
 * (one that asks first, then calls the route itself).
 */
export function useOperatorRefresh(): () => void {
  const client = useQueryClient();
  const userId = useUserId();
  return () =>
    void client.invalidateQueries({
      queryKey: operatorKeys.all(userId, scopeGeneration()),
    });
}
