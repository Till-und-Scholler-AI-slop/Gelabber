import { api } from "../api/client.ts";
import { stampHolds, type ScopeStamp } from "../auth/scope.ts";
import { notifyError } from "../components/toasts.ts";
import type { QueryClient, InfiniteData } from "@tanstack/react-query";
import { applyMessageEdited, messageKeys } from "./queries.ts";
import {
  asReactionList,
  type Message,
  type MessagePage,
  type Reaction,
} from "./types.ts";

export type ReactionIntent = {
  emoji: string;
  add: boolean;
  scope: ScopeStamp;
};

/** Only the current user's vote is optimistic; content and other votes stay live. */
export function displayedReactions(
  reactions: Reaction[] | undefined,
  userId: string,
  intent?: ReactionIntent,
): Reaction[] {
  const result = asReactionList(reactions).map((reaction) => ({
    ...reaction,
    user_ids: [...new Set(reaction.user_ids)],
  }));
  if (!intent || intent.scope.userId !== userId) return result;
  let row = result.find((reaction) => reaction.emoji === intent.emoji);
  if (!row && intent.add) {
    row = { emoji: intent.emoji, user_ids: [] };
    result.push(row);
  }
  if (row) {
    row.user_ids = row.user_ids.filter((id) => id !== userId);
    if (intent.add) row.user_ids.push(userId);
  }
  return result.filter((reaction) => reaction.user_ids.length > 0);
}

export function reactionMutationOptions(
  client: QueryClient,
  channelId: string,
  messageId: string,
) {
  return {
    mutationFn: async (intent: ReactionIntent) => {
      if (!stampHolds(intent.scope) || messageId.startsWith("tmp:"))
        throw new DOMException("Session changed", "AbortError");
      return api<Message>(
        `/messages/${messageId}/reactions/${encodeURIComponent(intent.emoji)}`,
        {
          method: intent.add ? "PUT" : "DELETE",
        },
      );
    },
    onSuccess: (message: Message, intent: ReactionIntent) => {
      if (stampHolds(intent.scope)) {
        applyMessageEdited(
          client,
          intent.scope.userId,
          intent.scope.generation,
          channelId,
          message,
        );
        const prefix = ["user", intent.scope.userId, intent.scope.generation];
        const update = (row: Message) =>
          row.id === message.id &&
          (row.revision ?? 0) <= (message.revision ?? 0)
            ? message
            : row;
        client.setQueriesData<InfiniteData<MessagePage>>(
          { queryKey: [...prefix, "message-search", channelId] },
          (data) =>
            data && {
              ...data,
              pages: data.pages.map((page) => ({
                ...page,
                messages: page.messages.map(update),
              })),
            },
        );
        client.setQueriesData<{ messages: Message[] }>(
          { queryKey: [...prefix, "message-context", channelId] },
          (data) => data && { ...data, messages: data.messages.map(update) },
        );
      }
    },
    onError: (error: Error, intent: ReactionIntent) => {
      if (!stampHolds(intent.scope)) return;
      notifyError(error);
      void client.invalidateQueries({
        queryKey: messageKeys.channel(
          intent.scope.userId,
          intent.scope.generation,
          channelId,
        ),
      });
    },
    retry: false as const,
  };
}
