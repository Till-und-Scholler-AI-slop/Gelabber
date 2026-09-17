// Connect the singleton gateway while a user is signed in, and keep its
// subscribe set owned by one authenticated coordinator (survives /settings).

import { useEffect, useMemo, useState } from "react";
import { useParams } from "@tanstack/react-router";
import { useShallow } from "zustand/react/shallow";

import { ApiError } from "../api/client.ts";
import { useDms } from "../dms/queries.ts";
import type { DirectMessage } from "../dms/types.ts";
import { useServer } from "../servers/queries.ts";
import type { ServerDetail } from "../servers/types.ts";
import { useVoice } from "../voice/session.ts";
import { getGateway } from "./client.ts";
import { topicKey, type Topic } from "./protocol.ts";
import { useLiveBridge } from "./useLive.ts";
import { useRealtimeBridge } from "./useRealtime.ts";

export function useGatewaySession(authenticated: boolean): void {
  useEffect(() => {
    const gateway = getGateway();
    if (authenticated) {
      gateway.start();
      return () => {
        gateway.setTopics([]);
        gateway.stop();
      };
    }
    gateway.setTopics([]);
    gateway.stop();
    return undefined;
  }, [authenticated]);
}

/** Deduped topic list; first entry for a key wins. */
export function mergeTopics(...groups: Topic[][]): Topic[] {
  const topics: Topic[] = [];
  const seen = new Set<string>();
  const add = (topic: Topic) => {
    const key = topicKey(topic);
    if (seen.has(key)) return;
    seen.add(key);
    topics.push(topic);
  };
  for (const group of groups) {
    for (const topic of group) add(topic);
  }
  return topics;
}

/** Server channel, or a DM whose protocol `s` is the channel id. */
export function workspaceTopics(
  serverId: string | undefined,
  channelId: string | undefined,
  extras: Topic[] = [],
): Topic[] {
  const base: Topic[] = [];
  if (serverId) {
    base.push({ s: serverId });
    if (channelId) base.push({ s: serverId, c: channelId });
  } else if (channelId) {
    base.push({ s: channelId, c: channelId });
  }
  return mergeTopics(base, extras);
}

/** Text channels of the remembered server + every known DM. */
export function notificationTopics(
  server: ServerDetail | undefined,
  dms: DirectMessage[] | undefined,
): Topic[] {
  const topics: Topic[] = [];
  if (server) {
    topics.push({ s: server.id });
    for (const channel of server.channels) {
      if (channel.kind === "text") {
        topics.push({ s: server.id, c: channel.id });
      }
    }
  }
  for (const dm of dms ?? []) {
    topics.push({ s: dm.id, c: dm.id });
  }
  return topics;
}

/** Keep voice / watch signaling reachable outside the workspace layout. */
export function mediaContextTopics(input: {
  status: string;
  serverId: string | null;
  channelId: string | null;
  watching: boolean;
  watchChannelId: string | null;
  /** Fallback server when watching without an active voice seat. */
  contextServerId?: string;
}): Topic[] {
  const topics: Topic[] = [];
  if (input.status === "joined" && input.serverId && input.channelId) {
    topics.push({ s: input.serverId });
    topics.push({ s: input.serverId, c: input.channelId });
  }
  const watchServer = input.serverId ?? input.contextServerId;
  if (input.watching && input.watchChannelId && watchServer) {
    topics.push({ s: watchServer });
    topics.push({ s: watchServer, c: input.watchChannelId });
  }
  return mergeTopics(topics);
}

/**
 * Single owner of `setTopics` while signed in. Survives /settings by keeping
 * the last workspace server id in session memory. Mount only when authenticated.
 */
export function useAuthenticatedSubscriptions(): void {
  const params = useParams({ strict: false });
  const routeServerId = params.serverId;
  const routeChannelId = params.channelId;
  const [rememberedServerId, setRememberedServerId] = useState<
    string | undefined
  >();

  // Remember the last workspace server across /settings (no serverId in URL).
  // Adjust during render when the route changes — avoids an extra effect pass.
  if (routeServerId && routeServerId !== rememberedServerId) {
    setRememberedServerId(routeServerId);
  }

  const effectiveServerId = routeServerId ?? rememberedServerId;
  const { data: dms } = useDms();
  const { data: server, error: serverError } = useServer(effectiveServerId);

  const serverGone =
    serverError instanceof ApiError &&
    (serverError.code === "not_found" || serverError.code === "forbidden");
  if (serverGone && rememberedServerId && !routeServerId) {
    setRememberedServerId(undefined);
  }

  const voice = useVoice(
    useShallow((state) => ({
      status: state.status,
      serverId: state.serverId,
      channelId: state.channelId,
      watching: state.watching,
      watchChannelId: state.watchChannelId,
    })),
  );

  const topics = useMemo(() => {
    const notifications = notificationTopics(server, dms);
    const view = workspaceTopics(routeServerId, routeChannelId);
    const media = mediaContextTopics({
      ...voice,
      contextServerId: effectiveServerId,
    });
    return mergeTopics(view, notifications, media);
  }, [
    routeServerId,
    routeChannelId,
    server,
    dms,
    voice,
    effectiveServerId,
  ]);

  const topicKeyStr = topics
    .map((topic) => topicKey(topic))
    .sort()
    .join("|");

  useEffect(() => {
    getGateway().setTopics(topics);
    // No setTopics([]) on unmount — /settings must keep the union. Logout
    // clears via useGatewaySession.
    // eslint-disable-next-line react-hooks/exhaustive-deps -- topicKeyStr stands in for topics
  }, [topicKeyStr]);

  useLiveBridge();
  useRealtimeBridge(routeServerId ?? rememberedServerId);
}
