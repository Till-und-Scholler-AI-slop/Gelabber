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
import type { ReadSync } from "../ws/protocol.ts";
import { closeMessageNotifications } from "../pwa/notifications.ts";

export type ReadState = {
  channel_id: string;
  server_id: string | null;
  read_message_id: string | null;
  read_at: string | null;
  /** Read position and the membership it belongs to; the newest message
   * when the counts were taken. Absent from older APIs. */
  read_order?: number | null;
  membership_at?: string;
  latest_order?: number | null;
  unread_count: number;
  /** Unread messages that mention the user. Absent from older APIs. */
  mention_count?: number;
  /** Effective notification level of the conversation (notifyLevel.ts). */
  level?: "all" | "mentions" | "none";
  /** A mute of the conversation or its server; see `muted_until`. */
  muted?: boolean;
  /** When that mute ends; null while muted: until turned back on. */
  muted_until?: string | null;
};
export const readKey = (stamp: ScopeStamp) =>
  ["user", stamp.userId, stamp.generation, "unread"] as const;
export const listReadState = (signal?: AbortSignal) =>
  api<ReadState[]>("/messages/unread", { signal });
/** Every text channel of a server read to its newest message (#245). The
 * answer is the rows that moved. */
export const markServerRead = (serverId: string) =>
  api<ReadState[]>(`/servers/${serverId}/read`, { method: "PUT" });
/** One conversation up to its newest message, wherever the view is. */
export const markChannelRead = (channelId: string) =>
  api<ReadState>(`/channels/${channelId}/read`, { method: "PUT", body: {} });
/** Everything, or only the direct messages. */
export const markAllRead = (scope: "all" | "dms") =>
  api<ReadState[]>("/messages/read-all", { method: "PUT", body: { scope } });
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

/** The newest successful unread snapshot of a scope: its request number and
 * when it was asked for. A snapshot asked for after an event already counts
 * it; no second one is needed. Request numbers instead of wall-clock times:
 * they never tie and do not move when the system clock is set back. */
type Snapshot = { request: number; at: number };
let snapshotRequests = 0;
const snapshots = new Map<string, Snapshot>();
const snapshotSlot = (stamp: ScopeStamp) =>
  `${stamp.userId}:${stamp.generation}`;
const NO_SNAPSHOT: Snapshot = { request: 0, at: -Infinity };
const latestSnapshot = (stamp: ScopeStamp) =>
  snapshots.get(snapshotSlot(stamp)) ?? NO_SNAPSHOT;
/** Focus and the poll refresh only a snapshot at least this old. */
const SNAPSHOT_FRESH_MS = 10_000;

export const readStateOptions = (stamp: ScopeStamp) => ({
  queryKey: readKey(stamp),
  // The request's number and monotonic start time, recorded only once it
  // succeeded: a failed or aborted snapshot covers nothing.
  queryFn: async ({ signal }: { signal: AbortSignal }) => {
    const asked = { request: ++snapshotRequests, at: performance.now() };
    const snapshot = await listReadState(signal);
    const slot = snapshotSlot(stamp);
    const latest = snapshots.get(slot);
    if (!latest || asked.request > latest.request) snapshots.set(slot, asked);
    return snapshot;
  },
  staleTime: 15_000,
  retry: false,
  // attachReadRecovery refreshes on focus, unless a snapshot is still fresh.
  refetchOnWindowFocus: false,
});

export function useReadState() {
  const userId = useUserId();
  const generation = scopeGeneration();
  return useQuery({
    ...readStateOptions({ userId, generation }),
    enabled: Boolean(userId),
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
  const invalidateWorkflows = (channel?: string) => {
    for (const kind of ["message-search", "message-context"])
      invalidate([
        "user",
        stamp.userId,
        stamp.generation,
        kind,
        ...(channel ? [channel] : []),
      ]);
  };
  const channels = new Set<string>();
  let all = false;
  // The newest request number when the latest event of the pending batch
  // arrived. Only a snapshot asked for after that has every event of it.
  let since: number | undefined;
  const refresh = (channelId?: string) => {
    if (!active || !stampHolds(stamp)) return;
    if (channelId) channels.add(channelId);
    else all = true;
    since = snapshotRequests;
    clearTimeout(timer);
    const flush = () => {
      clearTimeout(timer);
      clearTimeout(maxTimer);
      maxTimer = undefined;
      if (!active || !stampHolds(stamp)) return;
      const pending = all ? [undefined] : [...channels];
      all = false;
      channels.clear();
      const covered =
        since !== undefined && latestSnapshot(stamp).request > since;
      since = undefined;
      // WS frames arrive in separate tasks. A trailing debounce coalesces
      // replay bursts without repeatedly aborting SQL or paginated searches.
      // A snapshot asked for after the latest of them already has them all.
      if (!covered) invalidate(readKey(stamp));
      pending.forEach(invalidateWorkflows);
    };
    timer = setTimeout(flush, 750);
    maxTimer ??= setTimeout(flush, 5_000);
  };
  // Focus, visibility and the poll catch what other devices did. A snapshot
  // younger than that already has it: clicking back into the window must
  // not ask again right away, only once that snapshot has aged.
  let deferred: ReturnType<typeof setTimeout> | undefined;
  const focus = () => {
    if (!active || document.visibilityState !== "visible") return;
    const age = performance.now() - latestSnapshot(stamp).at;
    if (age >= SNAPSHOT_FRESH_MS) {
      clearTimeout(deferred);
      deferred = undefined;
      refresh();
    } else if (deferred === undefined) {
      const after = snapshotRequests;
      deferred = setTimeout(() => {
        deferred = undefined;
        if (!active || !stampHolds(stamp)) return;
        if (document.visibilityState !== "visible") return;
        // A snapshot asked for after the focus already has the unread
        // state; searches and contexts still need their refresh. Without
        // this, snapshots arriving less than 10 s apart re-defer forever.
        if (latestSnapshot(stamp).request > after) invalidateWorkflows();
        else focus();
      }, SNAPSHOT_FRESH_MS - age);
    }
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
    clearTimeout(deferred);
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
    const stamp = { userId, generation };
    const gateway = getGateway();
    const stopRecovery = attachReadRecovery(client, stamp, gateway);
    // Read on another device or tab (#244): the badge and the notification
    // go here too, without waiting for the next snapshot.
    const stopSync = gateway.onAccount((frame) => {
      if (frame.op === "rd") applyReadSync(client, stamp, frame.d);
    });
    return () => {
      stopRecovery();
      stopSync();
    };
  }, [client, userId, generation]);
}

type Position = Pick<
  ReadState,
  "read_at" | "read_order" | "membership_at" | "latest_order"
>;
const order = (value: number | null | undefined) => value ?? -Infinity;
const readAt = (row: Position) =>
  row.read_at ? Date.parse(row.read_at) : -Infinity;

/** How `next` relates to `current`, ordered as the server orders cursors:
 * a newer membership wins, then the further position, then the later count.
 * Older APIs carry no orders; their read time decides. */
function compare(next: Position, current: Position): number {
  if (next.membership_at && current.membership_at) {
    const joined =
      Date.parse(next.membership_at) - Date.parse(current.membership_at);
    if (joined) return joined;
    if (order(next.read_order) !== order(current.read_order))
      return order(next.read_order) > order(current.read_order) ? 1 : -1;
    return order(next.latest_order) >= order(current.latest_order) ? 0 : -1;
  }
  return readAt(next) >= readAt(current) ? 0 : -1;
}

/** Read positions from elsewhere into the cached rows. Forward only: a row
 * this window already has further along stays; settings stay as cached.
 * When the cached row counted messages the frame did not know yet, the
 * position is taken and the counts are asked for again. */
export function applyReadSync(
  client: QueryClient,
  stamp: ScopeStamp,
  rows: readonly (Omit<ReadSync, "mention_count"> & {
    mention_count?: number;
  })[],
): void {
  if (!stampHolds(stamp) || rows.length === 0) return;
  const byChannel = new Map(rows.map((row) => [row.channel_id, row]));
  let recount = false;
  // A stale row, or one counted before messages this window knows, closes
  // no notifications: one of those newer messages may still be unread.
  const keepOpen = new Set<string>();
  client.setQueryData<ReadState[]>(readKey(stamp), (cached) =>
    cached?.map((current) => {
      const next = byChannel.get(current.channel_id);
      if (!next) return current;
      if (compare(next, current) < 0) {
        keepOpen.add(next.channel_id);
        return current;
      }
      if (order(current.latest_order) > order(next.latest_order)) {
        recount = true;
        keepOpen.add(next.channel_id);
      }
      return {
        ...current,
        read_message_id: next.read_message_id,
        read_at: next.read_at,
        read_order: next.read_order,
        membership_at: next.membership_at,
        latest_order: next.latest_order,
        unread_count: next.unread_count,
        mention_count: next.mention_count ?? 0,
      };
    }),
  );
  if (recount) void refreshReadState(client, stamp);
  for (const row of rows)
    if (row.unread_count === 0 && !keepOpen.has(row.channel_id))
      void closeMessageNotifications({
        user: stamp.userId,
        channelId: row.channel_id,
      });
}

/** "Als gelesen markieren" for a server, or everything / all DMs. */
export function useMarkManyRead() {
  const client = useQueryClient();
  const userId = useUserId();
  const generation = scopeGeneration();
  return async (
    what: { server: string } | { channel: string } | { all: "all" | "dms" },
  ) => {
    const stamp = { userId, generation };
    const rows =
      "server" in what
        ? await markServerRead(what.server)
        : "channel" in what
          ? [await markChannelRead(what.channel)]
          : await markAllRead(what.all);
    // A snapshot asked for before the write may still count these rows;
    // asked again once the write is in.
    const inFlight =
      client.getQueryState(readKey(stamp))?.fetchStatus === "fetching";
    await client.cancelQueries({ queryKey: readKey(stamp), exact: true });
    applyReadSync(client, stamp, rows);
    if (inFlight)
      void client.invalidateQueries(
        { queryKey: readKey(stamp), exact: true },
        { cancelRefetch: false },
      );
    return rows.length;
  };
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
