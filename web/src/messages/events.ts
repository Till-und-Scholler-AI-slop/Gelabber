// Both message bridges use the same snapshot/delta reconciliation contract.

import type { QueryClient } from "@tanstack/react-query";
import type { ChatEvent } from "../ws/protocol.ts";
import { applyChannelEvent } from "./queries.ts";

export function applyChatEvent(
  client: QueryClient,
  userId: string,
  generation: number,
  event: ChatEvent,
): void {
  applyChannelEvent(client, userId, generation, event);
}
