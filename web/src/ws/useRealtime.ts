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

/** Reconcile cached history and workspace lists after gaps and every reconnect.
 * Coalesce a burst, retain visible data, and leave failed recovery invalidated. */
export function attachRealtimeRecovery(
  client: QueryClient,
  stamp: ScopeStamp,
  gateway: Gateway,
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
      const gaps = gateway.gapRecoveries();
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
        // Cancel initial reads too: invalidateQueries alone shares a pending
        // request without data, which may have started before the gap.
        await client.cancelQueries(filters);
        if (!alive() || attempt !== version) return;
        // Cached inactive histories also need a canonical snapshot before a
        // resume head can advance. Unopened channels still have no cache.
        await client.invalidateQueries(
          {
            ...filters,
            predicate: (query) =>
              filters.predicate(query) &&
              !(
                discover &&
                ["servers", "dms"].includes(String(query.queryKey[3])) &&
                query.queryKey[4] === "list"
              ),
            refetchType: "all",
          },
          { cancelRefetch: true, throwOnError: false },
        );
        if (!alive() || attempt !== version) return;
        // Removed channels/DMs are authoritative absence, not an outage that
        // should prevent every remaining topic from recovering forever.
        for (const query of client.getQueryCache().findAll(filters)) {
          if (
            discover &&
            ["servers", "dms"].includes(String(query.queryKey[3])) &&
            query.queryKey[4] === "list"
          )
            continue;
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
        const known = Array.isArray(dms) ? dms : [];
        for (const dm of known) {
          if (typeof dm.id !== "string" || !dm.id) continue;
          if (
            !requested.has(dm.id) &&
            client.getQueryData(
              messageQueryOptions(client, stamp.userId, stamp.generation, dm.id)
                .queryKey,
            )
          )
            continue;
          gateway.addTopic({ s: dm.id, c: dm.id });
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
