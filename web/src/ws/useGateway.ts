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

export function useGatewayTopics(
  serverId: string | undefined,
  channelId: string | undefined,
): void {
  useEffect(() => {
    const topics: Topic[] = [];
    if (serverId) {
      topics.push({ s: serverId });
      if (channelId) {
        topics.push({ s: serverId, c: channelId });
      }
    }
    getGateway().setTopics(topics);
    return () => {
      getGateway().setTopics([]);
    };
  }, [serverId, channelId]);
}
