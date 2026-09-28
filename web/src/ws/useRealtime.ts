// Apply sequenced chat/server events and kick/ban errors to the query
// cache. Message deletes land immediately; the list pin lives in MessagePane.

import { useQueryClient, type QueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { useEffect } from "react";

import { scopeGeneration, stampHolds, type ScopeStamp } from "../auth/scope.ts";
import { useSession } from "../auth/session.ts";
import { notify } from "../components/toasts.ts";
import { applyChannelEvent } from "../messages/queries.ts";
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
          { ...filters, refetchType: "all" },
          { cancelRefetch: true, throwOnError: true },
        );
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
        })
        .finally(() => {
          if (running === request) running = null;
          if (succeeded && alive() && gateway.gapRecoveries().length > 0)
            schedule();
        });
    });
  };
  const offGap = gateway.onGap(() => schedule());
  const offReady = gateway.onReady(() => schedule(true));
  // Navigation can replace the bridge while a recovery is still pending.
  // No new frame is required to resume that work under the new listener.
  if (gateway.gapRecoveries().length > 0) schedule();
  return () => {
    active = false;
    offGap();
    offReady();
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
