// Connect the singleton gateway while a user is signed in, and keep its
// subscribe set aligned with the workspace URL. Issue 5 listens via
// `getGateway().onEvent` — this hook does not render messages.

import { useEffect } from "react";

import { getGateway } from "./client.ts";
import { topicKey, type Topic } from "./protocol.ts";

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
  extras: Topic[] = [],
): Topic[] {
  const topics: Topic[] = [];
  const seen = new Set<string>();
  const add = (topic: Topic) => {
    const key = topicKey(topic);
    if (seen.has(key)) return;
    seen.add(key);
    topics.push(topic);
  };
  if (serverId) {
    add({ s: serverId });
    if (channelId) add({ s: serverId, c: channelId });
  } else if (channelId) {
    add({ s: channelId, c: channelId });
  }
  for (const extra of extras) add(extra);
  return topics;
}

export function useGatewayTopics(
  serverId: string | undefined,
  channelId: string | undefined,
  extras: Topic[] = [],
): void {
  const extraKey = extras
    .map((topic) => topicKey(topic))
    .sort()
    .join("|");
  useEffect(() => {
    const topics = workspaceTopics(serverId, channelId, extras);
    getGateway().setTopics(topics);
    return () => {
      getGateway().setTopics([]);
    };
    // extras is hashed as extraKey so a new [] every render does not resubscribe.
    // eslint-disable-next-line react-hooks/exhaustive-deps -- extras identity is extraKey
  }, [serverId, channelId, extraKey]);
}
