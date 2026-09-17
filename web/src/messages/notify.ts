// Decide whether a chat create should surface as a toast, and how to
// label / stack it. No DOM here — the hook owns permission + navigation.

export type ToastDecision = {
  show: boolean;
  dm: boolean;
};

export function isDmTopic(serverId: string, channelId: string): boolean {
  return serverId === channelId;
}

export function shouldToastMessage(input: {
  enabled: boolean;
  type: "c" | "e" | "d";
  own: boolean;
  channelId: string;
  viewingChannelId: string | undefined;
}): boolean {
  if (!input.enabled) return false;
  if (input.type !== "c") return false;
  if (input.own) return false;
  if (input.viewingChannelId && input.viewingChannelId === input.channelId) {
    return false;
  }
  return true;
}

export function previewText(content: string, hasAttachment: boolean): string {
  const trimmed = content.trim().replace(/\s+/g, " ");
  if (trimmed) {
    return trimmed.length > 140 ? `${trimmed.slice(0, 137)}…` : trimmed;
  }
  return hasAttachment ? "Datei" : "Neue Nachricht";
}

export const TOAST_STACK_MS = 1_600;
export const TOAST_HOLD_MS = 6_000;
export const TOAST_MAX = 4;

export type StackableToast = {
  id: number;
  channelId: string;
  at: number;
  count: number;
};

/** Same channel within the stack window → bump count instead of a new row. */
export function stackOnto(
  open: StackableToast[],
  channelId: string,
  now: number,
  windowMs = TOAST_STACK_MS,
): StackableToast | null {
  const current = open.find((row) => row.channelId === channelId);
  if (!current) return null;
  if (now - current.at > windowMs) return null;
  return current;
}
