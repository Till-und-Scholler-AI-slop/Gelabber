import { useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";

import { stampHolds, takeStamp } from "../auth/scope.ts";
import { useSession } from "../auth/session.ts";
import { reactionMutationOptions } from "../messages/reactions.ts";
import { asReactionList, type Message } from "../messages/types.ts";

/** The viewer's reaction to one message, shared by its chips, the toolbar
 *  and the action sheet, so each shows the same pending vote. */
export function useReaction(message: Message, canSend: boolean) {
  const client = useQueryClient();
  const userId = useSession((state) => state.user?.id ?? "");
  const mutation = useMutation(
    reactionMutationOptions(client, message.channel_id, message.id),
  );
  const intent =
    mutation.isPending &&
    mutation.variables &&
    stampHolds(mutation.variables.scope)
      ? mutation.variables
      : undefined;
  const busy = Boolean(intent);
  const react = (emoji: string, add: boolean) => {
    const scope = takeStamp();
    if (!scope || scope.userId !== userId || busy || (add && !canSend)) return;
    mutation.mutate({ emoji, add, scope });
    return scope;
  };
  /** Adds `emoji`, or takes the viewer's own one back. */
  const toggle = (emoji: string) =>
    react(
      emoji,
      !(
        asReactionList(message.reactions)
          .find((reaction) => reaction.emoji === emoji)
          ?.user_ids.includes(userId) ?? false
      ),
    );
  // The picker is the row's, so the action sheet can open it too.
  const [picking, setPicking] = useState(false);
  return { userId, intent, busy, react, toggle, picking, setPicking };
}

export type Reacting = ReturnType<typeof useReaction>;
