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
import {
  applyChannelEvent,
  messageKeys,
  messageQueryOptions,
} from "../messages/queries.ts";
import {
  applyMemberRemoved,
  forgetServer,
  markServerBlocked,
  refetchOnAccessChange,
} from "../servers/queries.ts";
import type { Server } from "../servers/types.ts";
import { leaveVoice, useVoice } from "../voice/session.ts";
import { getGateway, type GapRecovery, type Gateway } from "./client.ts";
import type { ChatEvent } from "./protocol.ts";
import { shouldLeaveView } from "./leaveView.ts";
import { translate } from "../i18n/catalog.ts";
import { shell } from "../i18n/catalogs/shell.ts";
import { auth } from "../i18n/catalogs/auth.ts";

/** Kick/ban closes both planes before the sidebar that hosts voice unmounts. */
function dropVoice(serverId: string): void {
  const voice = useVoice.getState();
  if (voice.status === "joined" && voice.serverId === serverId) {
    leaveVoice();
  }
}

/**
 * `{"op":"err","e":"server_blocked","s":…}`: an operator blocked the server.
 * Like a kick it ends voice and leaves the server's view, but the server
 * stays in the list, marked, so its tile opens the blocked notice. Every
 * later subscribe to it meets the same error; once the list says blocked,
 * only voice is dropped again, without another toast or navigation.
 * Returns whether the block is news.
 */
export function applyServerBlocked(
  client: QueryClient,
  userId: string,
  generation: number,
  serverId: string,
): boolean {
  dropVoice(serverId);
  const listed = client
    .getQueryData<Server[]>(serverKeys.list(userId, generation))
    ?.find((server) => server.id === serverId);
  if (listed?.blocked) return false;
  markServerBlocked(client, userId, generation, serverId);
  return true;
}

/** The server-topic event "access changed" (`t: "e"`, no `c`,
 * `d: {"k": "a"}`): roles, rights or a channel's access changed. */
export function isAccessChange(event: ChatEvent): boolean {
  return (
    event.t === "e" &&
    !event.c &&
    event.d !== null &&
    typeof event.d === "object" &&
    "k" in event.d &&
    event.d.k === "a"
  );
}

/** Reads the server's detail and the server list again, after the caller's
 * own role or server writes of it have settled: the caller's rights and the
 * channels they see may have changed. An open channel that is gone sends
 * `ChannelPage` back to the server. */
export function applyAccessChange(
  client: QueryClient,
  userId: string,
  generation: number,
  serverId: string,
): void {
  refetchOnAccessChange(client, userId, generation, serverId);
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
  /** The running round is still in its spread wait and has read nothing. */
  let spreading = false;
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
      // A reconnect or resync during the wait joins the waiting round, which
      // takes its snapshot afterwards: a flapping link must not keep
      // pushing the refetch back.
      if (running && spreading) return;
      const version = ++attempt;
      let gaps: GapRecovery[] = [];
      let discover = false;
      let discovery = discoveryVersion;
      let resync: number | undefined;
      let announced: string[] = [];
      const snapshot = () => {
        gaps = gateway.gapRecoveries();
        discover = rediscover || pendingDms.size > 0;
        discovery = discoveryVersion;
        resync = gateway.pendingResync;
        rediscover = false;
        announced = [...pendingDms];
      };
      const filters = {
        queryKey: ["user", stamp.userId, stamp.generation],
        predicate: (query: { queryKey: readonly unknown[] }) =>
          ["messages", "servers", "dms"].includes(String(query.queryKey[3])),
      };
      const request = (async () => {
        // Every client of a restarted server reconnects within about a
        // second; a random wait spreads their refetches over another one.
        // Gap notices, resyncs and DM announcements that arrive meanwhile
        // join this round instead of starting a second one.
        if (restart && readySpreadMs > 0) {
          spreading = true;
          try {
            await new Promise((done) =>
              setTimeout(done, Math.random() * readySpreadMs),
            );
          } finally {
            spreading = false;
          }
          if (!alive() || attempt !== version) return;
        }
        snapshot();
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
          // It stays invalidated (patchPages keeps the flag through live
          // events) and is read again when next opened.
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
  // A first subscribe races the history read that left with it: read again.
  const offFresh = gateway.onFresh((topic) => {
    if (!alive() || !topic.c) return;
    void client.invalidateQueries(
      {
        queryKey: messageKeys.channel(stamp.userId, stamp.generation, topic.c),
        refetchType: "active",
      },
      { cancelRefetch: true, throwOnError: false },
    );
  });
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
    offFresh();
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
        translate(
          shell,
          reason === "banned" ? "removed.banned" : "removed.kicked",
        ),
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
      if (isAccessChange(event)) {
        applyAccessChange(client, userId, generation, event.s);
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
      if (err.e === "server_blocked" && err.s && userId && alive()) {
        const serverId = err.s;
        if (!applyServerBlocked(client, userId, generation, serverId)) return;
        onSelfRemoved?.(serverId);
        notify(translate(auth, "error.serverBlocked"));
        if (shouldLeaveView(viewingServerId, serverId))
          void navigate({ to: "/", replace: true });
      }
    });
    return () => {
      offEvent();
      offErr();
      offRecovery();
    };
  }, [client, me, generation, navigate, onSelfRemoved, viewingServerId]);
}
