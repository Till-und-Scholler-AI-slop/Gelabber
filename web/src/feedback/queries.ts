// TanStack Query layer for feedback. Keys live under the signed-in user's
// scope like `directoryKeys`, so signing out drops them. There is no gateway
// frame: the unread dot comes from the status route, read on start, on
// window focus, every five minutes and after the person's own changes.

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
import * as remote from "./api.ts";
import type { FeedbackContext, FeedbackKind, FeedbackStatus } from "./types.ts";

export const feedbackKeys = {
  all: (userId: string, generation: number) =>
    ["user", userId, generation, "feedback"] as const,
  status: (userId: string, generation: number) =>
    ["user", userId, generation, "feedback", "status"] as const,
  list: (userId: string, generation: number) =>
    ["user", userId, generation, "feedback", "list"] as const,
};

/** Fresh for a minute, so the panels of every page share one read. */
const STALE_MS = 60_000;
/** A reply hours later needs no push: the mail tells the person anyway. */
export const STATUS_REFETCH_MS = 300_000;

const statusQuery = (userId: string, generation: number) => ({
  queryKey: feedbackKeys.status(userId, generation),
  queryFn: ({ signal }: { signal?: AbortSignal }) =>
    remote.feedbackStatus(signal),
  staleTime: STALE_MS,
});

/** `GET /api/feedback/status`: on or off, and the unread count. */
export function useFeedbackStatus() {
  const userId = useUserId();
  const generation = scopeGeneration();
  return useQuery({
    ...statusQuery(userId, generation),
    enabled: userId.length > 0,
    refetchInterval: STATUS_REFETCH_MS,
    refetchOnWindowFocus: true,
    // A failed check shows no item rather than retrying.
    retry: false,
  });
}

/** The status from the cache, else fetched: the `/settings/feedback` guard. */
export function ensureFeedbackStatus(
  client: QueryClient,
  stamp: { userId: string; generation: number },
): Promise<FeedbackStatus> {
  return client.ensureQueryData(statusQuery(stamp.userId, stamp.generation));
}

/** The status and the list are read again after a change of the person's. */
export function refreshFeedback(
  client: QueryClient,
  stamp: { userId: string; generation: number },
): void {
  void client.invalidateQueries({
    queryKey: feedbackKeys.all(stamp.userId, stamp.generation),
  });
}

export type FeedbackSend = {
  kind: FeedbackKind;
  text: string;
  image: File | null;
  context: FeedbackContext;
};

/** "Senden": the image goes up first, then the entry is created. */
export function useSendFeedback() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: ({ kind, text, image, context }: FeedbackSend) =>
      remote.sendFeedback({ kind, text: text.trim(), context }, image),
    onMutate: () => takeStamp(),
    onSuccess: (_entry, _input, stamp) => {
      if (stampHolds(stamp)) refreshFeedback(client, stamp);
    },
  });
}
