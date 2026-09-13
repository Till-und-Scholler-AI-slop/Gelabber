// Connect the singleton gateway while a user is signed in, and keep its
// subscribe set aligned with the workspace URL. Issue 5 listens via
// `getGateway().onEvent` — this hook does not render messages.

import { useEffect } from "react";

import { getGateway } from "./client.ts";
import type { Topic } from "./protocol.ts";

export function useGatewaySession(authenticated: boolean): void {
  useEffect(() => {
    const gateway = getGateway();
    if (authenticated) {
      gateway.start();
      return () => {
        gateway.stop();
      };
    }
    gateway.stop();
    return undefined;
  }, [authenticated]);
}

/** Server channel, or a DM whose protocol `s` is the channel id. */
export function workspaceTopics(
  serverId: string | undefined,
  channelId: string | undefined,
): Topic[] {
  if (serverId) {
    const topics: Topic[] = [{ s: serverId }];
    if (channelId) topics.push({ s: serverId, c: channelId });
    return topics;
  }
  if (channelId) return [{ s: channelId, c: channelId }];
  return [];
}

export function useGatewayTopics(
  serverId: string | undefined,
  channelId: string | undefined,
): void {
  useEffect(() => {
    const topics = workspaceTopics(serverId, channelId);
    getGateway().setTopics(topics);
    return () => {
      getGateway().setTopics([]);
    };
  }, [serverId, channelId]);
}
