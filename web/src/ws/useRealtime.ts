// Apply sequenced chat/server events and kick/ban errors to the query
// cache. Message deletes land immediately; the list pin lives in MessagePane.

import { useQueryClient, type QueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { useEffect } from "react";

import { ApiError } from "../api/client.ts";
import { scopeGeneration, stampHolds, type ScopeStamp } from "../auth/scope.ts";
import { useSession } from "../auth/session.ts";
import { notify } from "../components/toasts.ts";
import { listDms } from "../dms/api.ts";
import { dmKeys } from "../dms/queries.ts";
import type { DirectMessage } from "../dms/types.ts";
import { listServers } from "../servers/api.ts";
import { serverKeys } from "../servers/queries.ts";
import { applyChannelEvent, messageQueryOptions } from "../messages/queries.ts";
import { applyMemberRemoved, forgetServer } from "../servers/queries.ts";
import { leaveVoice, stopWatching, useVoice } from "../voice/session.ts";
import { getGateway, type Gateway } from "./client.ts";
import { shouldLeaveView } from "./leaveView.ts";

/** Kick/ban closes both planes before the sidebar that hosts voice unmounts. */
function dropVoice(serverId: string): void {
  const voice = useVoice.getState();
  if (voice.status === "joined" && voice.serverId === serverId) {
    leaveVoice();
  }
  if (voice.watching && voice.watchServerId === serverId) {
    stopWatching();
  }
}

/** Upper bound of the random wait before a reconnect refetches. */
export const READY_SPREAD_MS = 1_000;

const isHistory = (query: { queryKey: readonly unknown[] }) =>
  query.queryKey[3] === "messages";

/** Reconcile cached history and workspace lists after gaps and every reconnect.
 * Coalesce a burst, retain visible data, and leave failed recovery invalidated. */
export function attachRealtimeRecovery(
  client: QueryClient,
  stamp: ScopeStamp,
  gateway: Gateway,
  { readySpreadMs = READY_SPREAD_MS }: { readySpreadMs?: number } = {},
): () => void {
  let active = true;
  let queued = false;
  let force = false;
  let running: Promise<void> | null = null;
  let attempt = 0;
  let rediscover = gateway.pendingResync !== undefined;
  let discoveryVersion = 0;
  const pendingDms = new Set(gateway.dmDiscoveries());
  const alive = () => active && stampHolds(stamp);
  const schedule = (newConnection = false) => {
    if (!alive()) return;
    force ||= newConnection;
    if (queued) return;
    queued = true;
    queueMicrotask(() => {
      queued = false;
      if (!alive()) return;
      const restart = force;
      force = false;
      if (running && !restart) return;
      const version = ++attempt;
      let gaps = gateway.gapRecoveries();
      const discover = rediscover || pendingDms.size > 0;
      const discovery = discoveryVersion;
      const resync = gateway.pendingResync;
      rediscover = false;
      const announced = [...pendingDms];
      const filters = {
        queryKey: ["user", stamp.userId, stamp.generation],
        predicate: (query: { queryKey: readonly unknown[] }) =>
          ["messages", "servers", "dms"].includes(String(query.queryKey[3])),
      };
      const request = (async () => {
        // Every client of a restarted server reconnects within about a
        // second; a random wait spreads their refetches over another one.
        // Gap notices that answer the resume frames meanwhile join this
        // round instead of starting a second one.
        if (restart && readySpreadMs > 0) {
          await new Promise((done) =>
            setTimeout(done, Math.random() * readySpreadMs),
          );
          if (!alive() || attempt !== version) return;
          gaps = gateway.gapRecoveries();
        }
        // Cancel initial reads too: invalidateQueries alone shares a pending
        // request without data, which may have started before the gap.
        await client.cancelQueries(filters);
        if (!alive() || attempt !== version) return;
        const skipped = (query: { queryKey: readonly unknown[] }) =>
          discover &&
          ["servers", "dms"].includes(String(query.queryKey[3])) &&
          query.queryKey[4] === "list";
        const options = { cancelRefetch: true, throwOnError: false };
        // Only histories on screen are read again. An inactive one stays
        // invalidated (live patches keep that flag), so it refetches when it
        // is next opened and never shows a hole behind an advanced cursor.
        // Workspace lists and details are small and always read again: their
        // live patches do not keep the flag.
        await Promise.all([
          client.invalidateQueries(
            {
              ...filters,
              predicate: (query) =>
                filters.predicate(query) && isHistory(query),
              refetchType: "active",
            },
            options,
          ),
          client.invalidateQueries(
            {
              ...filters,
              predicate: (query) =>
                filters.predicate(query) &&
                !isHistory(query) &&
                !skipped(query),
              refetchType: "all",
            },
            options,
          ),
        ]);
        if (!alive() || attempt !== version) return;
        // Removed channels/DMs are authoritative absence, not an outage that
        // should prevent every remaining topic from recovering forever.
        for (const query of client.getQueryCache().findAll(filters)) {
          if (skipped(query)) continue;
          // Not read in this round: an old error is not this round's outcome.
          if (isHistory(query) && !query.isActive()) continue;
          if (query.state.status !== "error") continue;
          const error = query.state.error;
          if (
            !(error instanceof ApiError) ||
            !["not_found", "forbidden"].includes(error.code)
          )
            throw error;
          const key = query.queryKey;
          if (key[3] === "messages" && typeof key[4] === "string")
            gateway.forgetTopic(`c:${key[4]}`);
          if (
            key[3] === "dms" &&
            key[4] === "detail" &&
            typeof key[5] === "string"
          )
            gateway.forgetTopic(`c:${key[5]}`);
          if (query.getObserversCount() === 0)
            client.removeQueries({ queryKey: key, exact: true });
          else query.setState({ data: undefined });
        }
        if (!discover) return;
        // DMs already in the cached list are subscribed through the
        // notification topics. The snapshot need not predate the resync: a
        // replaced attempt or a list refetch may already have written it.
        const cachedDms = client.getQueryData<DirectMessage[]>(
          dmKeys.list(stamp.userId, stamp.generation),
        );
        const alreadyListed = new Set(
          (Array.isArray(cachedDms) ? cachedDms : []).flatMap((dm) =>
            typeof dm?.id === "string" && dm.id ? [dm.id] : [],
          ),
        );
        // Explicit queries also work when this socket has no known topics or
        // cached DM list. Cancelled older reads cannot swallow discovery.
        const dms = await client.fetchQuery({
          queryKey: dmKeys.list(stamp.userId, stamp.generation),
          queryFn: ({ signal }) => listDms(signal),
          staleTime: 0,
          retry: false,
        });
        await client.fetchQuery({
          queryKey: serverKeys.list(stamp.userId, stamp.generation),
          queryFn: ({ signal }) => listServers(signal),
          staleTime: 0,
          retry: false,
        });
        if (!alive() || attempt !== version) return;
        const requested = new Set(announced);
        // Resync discovers DMs missed before notification delivery resumed.
        // A DM that was already listed is subscribed through the notification
        // topics: its cached history was invalidated above and is read again
        // once on screen, an uncached one loads when opened, and unread state
        // comes from read state. Only announced and newly listed DMs need a
        // first history.
        const known = Array.isArray(dms) ? dms : [];
        for (const dm of known) {
          if (typeof dm.id !== "string" || !dm.id) continue;
          // Idempotent: a topic that is already desired sends no frame.
          gateway.addTopic({ s: dm.id, c: dm.id });
          if (
            !requested.has(dm.id) &&
            (alreadyListed.has(dm.id) ||
              client.getQueryData(
                messageQueryOptions(
                  client,
                  stamp.userId,
                  stamp.generation,
                  dm.id,
                ).queryKey,
              ))
          )
            continue;
          await client.fetchInfiniteQuery({
            ...messageQueryOptions(
              client,
              stamp.userId,
              stamp.generation,
              dm.id,
            ),
            staleTime: 0,
          });
          if (!alive() || attempt !== version) return;
          pendingDms.delete(dm.id);
          gateway.completeDm(dm.id);
        }
        // Unconfirmed/unauthorized IDs never become subscriptions. A later
        // ready/resync notification can retry transient list/history failures.
        for (const id of announced) {
          pendingDms.delete(id);
          gateway.completeDm(id);
        }
        gateway.completeResync(resync);
      })();
      running = request;
      let succeeded = false;
      void request
        .then(() => {
          if (!alive() || running !== request) return;
          for (const gap of gaps) gateway.completeGap(gap);
          succeeded = true;
        })
        .catch(() => {
          // Query errors remain visible; no cursor acknowledgement or retry loop.
          if (discover && alive()) rediscover = true;
        })
        .finally(() => {
          if (running === request) running = null;
          if (
            alive() &&
            ((succeeded && gateway.gapRecoveries().length > 0) ||
              discoveryVersion !== discovery)
          )
            schedule();
        });
    });
  };
  const offGap = gateway.onGap(() => schedule());
  const offReady = gateway.onReady(() => schedule(true));
  const offResync = gateway.onResync(() => {
    rediscover = true;
    discoveryVersion++;
    schedule(true);
  });
  const offDm = gateway.onDm((channelId) => {
    pendingDms.add(channelId);
    discoveryVersion++;
    schedule();
  });
  // Navigation can replace the bridge while a recovery is still pending.
  // No new frame is required to resume that work under the new listener.
  if (gateway.gapRecoveries().length > 0 || pendingDms.size > 0 || rediscover)
    schedule();
  return () => {
    active = false;
    offGap();
    offReady();
    offResync();
    offDm();
  };
}

export function useRealtimeBridge(
  viewingServerId?: string,
  onSelfRemoved?: (serverId: string) => void,
): void {
  const client = useQueryClient();
  const navigate = useNavigate();
  const me = useSession((s) => s.user)?.id;
  const generation = scopeGeneration();

  useEffect(() => {
    const userId = me;
    const alive = () => userId != null && stampHolds({ userId, generation });

    const leave = (reason: "kicked" | "banned", serverId: string) => {
      if (!userId || !alive()) return;
      dropVoice(serverId);
      onSelfRemoved?.(serverId);
      notify(
        reason === "banned"
          ? "Du wurdest vom Server gesperrt."
          : "Du wurdest vom Server entfernt.",
      );
      if (shouldLeaveView(viewingServerId, serverId)) {
        forgetServer(client, userId, serverId, { keepDetail: true });
        void navigate({ to: "/", replace: true }).then(() => {
          if (!alive()) return;
          forgetServer(client, userId, serverId);
        });
        return;
      }
      forgetServer(client, userId, serverId);
    };

    const gateway = getGateway();
    const offRecovery = userId
      ? attachRealtimeRecovery(client, { userId, generation }, gateway)
      : () => undefined;
    const offEvent = gateway.onEvent((event) => {
      if (!userId || !alive()) return;
      if (event.c) {
        applyChannelEvent(client, userId, generation, event);
        return;
      }
      if (event.t === "d" && event.i) {
        const result = applyMemberRemoved(
          client,
          userId,
          generation,
          event.s,
          event.i,
        );
        if (result === "self") {
          const banned =
            event.d !== null &&
            typeof event.d === "object" &&
            "k" in event.d &&
            event.d.k === "b";
          leave(banned ? "banned" : "kicked", event.s);
        }
      }
    });
    const offErr = gateway.onErr((err) => {
      if ((err.e === "kicked" || err.e === "banned") && err.s) {
        leave(err.e, err.s);
      }
    });
    return () => {
      offEvent();
      offErr();
      offRecovery();
    };
  }, [client, me, generation, navigate, onSelfRemoved, viewingServerId]);
}
