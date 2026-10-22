import type { PresenceStatus } from "../ws/protocol.ts";

export const PRESENCE_LABEL: Record<PresenceStatus, string> = {
  o: "Online",
  i: "Abwesend",
  x: "Offline",
};

/**
 * Your own presence: as the server in view sees it, else as any server
 * does. Someone using the app is online until a server says otherwise.
 */
export function ownPresence(
  byServer: Record<string, Record<string, PresenceStatus>>,
  userId: string,
  contextId: string | undefined,
): PresenceStatus {
  const inContext = contextId ? byServer[contextId]?.[userId] : undefined;
  if (inContext) return inContext;
  for (const members of Object.values(byServer))
    if (members[userId]) return members[userId];
  return "o";
}

/** The line under your name: presence, then the status text if there is one. */
export function panelSubtitle(
  presence: PresenceStatus,
  statusText: string | null | undefined,
): string {
  return statusText
    ? `${PRESENCE_LABEL[presence]} · ${statusText}`
    : PRESENCE_LABEL[presence];
}
