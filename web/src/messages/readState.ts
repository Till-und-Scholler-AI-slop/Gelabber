import {
  useQuery,
  useQueryClient,
  type QueryClient,
} from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { api } from "../api/client.ts";
import {
  scopeGeneration,
  stampHolds,
  useUserId,
  type ScopeStamp,
} from "../auth/scope.ts";
import { create } from "zustand";
import type { Message } from "./types.ts";
import { getGateway, type Gateway } from "../ws/client.ts";

export type ReadState = {
  channel_id: string;
  server_id: string | null;
  read_message_id: string | null;
  read_at: string | null;
  unread_count: number;
};
export const readKey = (stamp: ScopeStamp) =>
  ["user", stamp.userId, stamp.generation, "unread"] as const;
export const listReadState = (signal?: AbortSignal) =>
  api<ReadState[]>("/messages/unread", { signal });
export const markRead = (channelId: string, messageId: string) =>
  api<ReadState>(`/channels/${channelId}/read`, {
    method: "PUT",
    body: { message_id: messageId },
  });

const readingView = create<{
  channel: string | null;
  stamp: ScopeStamp | null;
}>(() => ({ channel: null, stamp: null }));
export function useReadingChannel(): string | null {
  const view = readingView();
  return view.stamp && stampHolds(view.stamp) ? view.channel : null;
}

export function useReadState() {
  const userId = useUserId();
  const generation = scopeGeneration();
  return useQuery({
    queryKey: readKey({ userId, generation }),
    queryFn: ({ signal }) => listReadState(signal),
    enabled: Boolean(userId),
    staleTime: 15_000,
    retry: false,
  });
}

/** REST snapshots are private. Refresh for deltas, gap recovery and other devices. */
export function attachReadRecovery(
  client: QueryClient,
  stamp: ScopeStamp,
  gateway: Gateway,
): () => void {
  let active = true;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let maxTimer: ReturnType<typeof setTimeout> | undefined;
  const waiting = new Map<string, () => void>();
  const invalidate = (queryKey: readonly unknown[]) => {
    for (const query of client.getQueryCache().findAll({ queryKey })) {
      if (query.state.fetchStatus === "idle" || waiting.has(query.queryHash))
        continue;
      const stop = client.getQueryCache().subscribe((event) => {
        if (
          event.query !== query ||
          (event.type !== "removed" && query.state.fetchStatus !== "idle")
        )
          return;
        stop();
        waiting.delete(query.queryHash);
        if (event.type !== "removed" && active && stampHolds(stamp))
          void client.invalidateQueries(
            { queryKey: query.queryKey, exact: true },
            { cancelRefetch: false },
          );
      });
      waiting.set(query.queryHash, stop);
    }
    void client.invalidateQueries({ queryKey }, { cancelRefetch: false });
  };
  const channels = new Set<string>();
  let all = false;
  const refresh = (channelId?: string) => {
    if (!active || !stampHolds(stamp)) return;
    if (channelId) channels.add(channelId);
    else all = true;
    clearTimeout(timer);
    const flush = () => {
      clearTimeout(timer);
      clearTimeout(maxTimer);
      maxTimer = undefined;
      if (!active || !stampHolds(stamp)) return;
      const pending = all ? [undefined] : [...channels];
      all = false;
      channels.clear();
      // WS frames arrive in separate tasks. A trailing debounce coalesces
      // replay bursts without repeatedly aborting SQL or paginated searches.
      invalidate(readKey(stamp));
      for (const channel of pending) {
        for (const kind of ["message-search", "message-context"]) {
          invalidate([
            "user",
            stamp.userId,
            stamp.generation,
            kind,
            ...(channel ? [channel] : []),
          ]);
        }
      }
    };
    timer = setTimeout(flush, 750);
    maxTimer ??= setTimeout(flush, 5_000);
  };
  const focus = () => {
    if (document.visibilityState === "visible") refresh();
  };
  const cleanup = [
    gateway.onEvent((event) => refresh(event.c)),
    gateway.onReady(() => refresh()),
    gateway.onGap(() => refresh()),
    gateway.onResync(() => refresh()),
    gateway.onDm(() => refresh()),
  ];
  window.addEventListener("focus", focus);
  document.addEventListener("visibilitychange", focus);
  const poll = window.setInterval(focus, 15_000);
  return () => {
    active = false;
    cleanup.forEach((fn) => fn());
    clearTimeout(timer);
    clearTimeout(maxTimer);
    waiting.forEach((stop) => stop());
    window.clearInterval(poll);
    window.removeEventListener("focus", focus);
    document.removeEventListener("visibilitychange", focus);
  };
}

export async function refreshReadState(
  client: QueryClient,
  stamp: ScopeStamp,
): Promise<void> {
  if (!stampHolds(stamp)) return;
  // Cancel a snapshot started before the write/event, including an initial read.
  await client.cancelQueries({ queryKey: readKey(stamp), exact: true });
  if (!stampHolds(stamp)) return;
  await client.invalidateQueries({ queryKey: readKey(stamp), exact: true });
}

export async function refreshChatWorkflows(
  client: QueryClient,
  stamp: ScopeStamp,
  channelId?: string,
): Promise<void> {
  if (!stampHolds(stamp)) return;
  const filter = {
    queryKey: [
      "user",
      stamp.userId,
      stamp.generation,
      "message-search",
      ...(channelId ? [channelId] : []),
    ],
  };
  const contextFilter = {
    queryKey: [
      "user",
      stamp.userId,
      stamp.generation,
      "message-context",
      ...(channelId ? [channelId] : []),
    ],
  };
  await Promise.all([
    client.cancelQueries(filter),
    client.cancelQueries(contextFilter),
  ]);
  if (!stampHolds(stamp)) return;
  await Promise.all([
    refreshReadState(client, stamp),
    client.invalidateQueries(filter),
    client.invalidateQueries(contextFilter),
  ]);
}

export function useReadBridge() {
  const client = useQueryClient();
  const userId = useUserId();
  const generation = scopeGeneration();
  useReadState();
  useEffect(() => {
    if (!userId) return;
    return attachReadRecovery(client, { userId, generation }, getGateway());
  }, [client, userId, generation]);
}

/** Apply an acknowledged read row without losing a refresh other channels need. */
export async function applyMarkedRead(
  client: QueryClient,
  stamp: ScopeStamp,
  channelId: string,
  row: ReadState,
): Promise<void> {
  const queryKey = readKey(stamp);
  if (!client.getQueryData(queryKey)) {
    // Let the initial unread snapshot finish instead of repeatedly
    // cancelling it as new visible messages are marked read.
    void client.invalidateQueries(
      { queryKey, exact: true },
      { cancelRefetch: false },
    );
    return;
  }
  // A snapshot started before the write may still report this channel as
  // unread. Cancel it, but refetch afterwards: it may carry other channels'
  // unread deltas that a debounced refresh asked for.
  const inFlight = client.getQueryState(queryKey)?.fetchStatus === "fetching";
  await client.cancelQueries({ queryKey, exact: true });
  if (!stampHolds(stamp)) return;
  client.setQueryData<ReadState[]>(queryKey, (rows) =>
    rows?.map((current) => (current.channel_id === channelId ? row : current)),
  );
  if (inFlight)
    void client.invalidateQueries(
      { queryKey, exact: true },
      { cancelRefetch: false },
    );
}

export function mayMarkRead(
  atLatest: boolean,
  ready: boolean,
  visible: boolean,
  focused: boolean,
): boolean {
  return atLatest && ready && visible && focused;
}

export function useMarkRead(
  channelId: string,
  messageId: string | undefined,
  atLatest: boolean,
  ready: boolean,
) {
  const client = useQueryClient();
  const userId = useUserId();
  const generation = scopeGeneration();
  const [error, setError] = useState(false);
  const [retry, setRetry] = useState(0);
  useEffect(() => {
    const stamp = { userId, generation };
    const update = () =>
      readingView.setState({
        channel:
          userId &&
          !error &&
          mayMarkRead(
            atLatest,
            ready,
            document.visibilityState === "visible",
            document.hasFocus(),
          )
            ? channelId
            : null,
        stamp,
      });
    update();
    window.addEventListener("focus", update);
    window.addEventListener("blur", update);
    document.addEventListener("visibilitychange", update);
    return () => {
      window.removeEventListener("focus", update);
      window.removeEventListener("blur", update);
      document.removeEventListener("visibilitychange", update);
      if (readingView.getState().stamp === stamp)
        readingView.setState({ channel: null, stamp: null });
    };
  }, [userId, generation, channelId, atLatest, ready, error]);
  useEffect(() => {
    if (!userId || !messageId) return;
    let active = true,
      running = false,
      done = false;
    const stamp = { userId, generation };
    const attempt = () => {
      if (
        !active ||
        running ||
        done ||
        !stampHolds(stamp) ||
        !mayMarkRead(
          atLatest,
          ready,
          document.visibilityState === "visible",
          document.hasFocus(),
        )
      )
        return;
      running = true;
      void markRead(channelId, messageId)
        .then(async (row) => {
          if (!active || !stampHolds(stamp)) return;
          done = true;
          setError(false);
          await applyMarkedRead(client, stamp, channelId, row);
        })
        .catch(() => {
          if (active && stampHolds(stamp)) setError(true);
        })
        .finally(() => {
          running = false;
        });
    };
    const timer = window.setTimeout(attempt, 600);
    window.addEventListener("focus", attempt);
    document.addEventListener("visibilitychange", attempt);
    return () => {
      active = false;
      window.clearTimeout(timer);
      window.removeEventListener("focus", attempt);
      document.removeEventListener("visibilitychange", attempt);
    };
  }, [
    client,
    userId,
    generation,
    channelId,
    messageId,
    atLatest,
    ready,
    retry,
  ]);
  return { error, retry: () => setRetry((value) => value + 1) };
}

/** Only rows actually delivered to the view can advance a read boundary. */
export function readBoundary(items: Message[]): Message | undefined {
  return items.reduce<Message | undefined>(
    (latest, row) =>
      row.id.startsWith("tmp:")
        ? latest
        : !latest || (row.created_order ?? 0) >= (latest.created_order ?? 0)
          ? row
          : latest,
    undefined,
  );
}
