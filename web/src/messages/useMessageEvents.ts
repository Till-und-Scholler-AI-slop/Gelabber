import { useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";

import { scopeGeneration, stampHolds } from "../auth/scope.ts";
import { useSession } from "../auth/session.ts";
import { getGateway } from "../ws/client.ts";
import { applyChatEvent } from "./events.ts";

/** Subscribe the message cache to chat create/edit/delete events. */
export function useMessageEvents(): void {
  const client = useQueryClient();
  const me = useSession((state) => state.user?.id);
  useEffect(() => {
    const userId = me;
    const generation = scopeGeneration();
    return getGateway().onEvent((event) => {
      if (!userId || !stampHolds({ userId, generation })) return;
      applyChatEvent(client, userId, generation, event);
    });
  }, [client, me]);
}
