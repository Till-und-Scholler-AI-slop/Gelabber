import { useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";

import { getGateway } from "../ws/client.ts";
import { applyChatEvent } from "./events.ts";

/** Subscribe the message cache to chat create/edit/delete events. */
export function useMessageEvents(): void {
  const client = useQueryClient();
  useEffect(() => {
    return getGateway().onEvent((event) => applyChatEvent(client, event));
  }, [client]);
}
