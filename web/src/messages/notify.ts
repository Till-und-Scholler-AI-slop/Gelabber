// Decide whether a chat create should surface as a toast, and how to
// label / stack it. No DOM here — the hook owns permission + navigation,
// and pwa/notifications.ts the delivery of system notifications.

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

/** Desktop delivery has its own preference and includes a hidden open chat. */
export function messageNotificationDecision(input: {
  toastEnabled: boolean;
  desktopEnabled: boolean;
  hidden: boolean;
  type: "c" | "e" | "d";
  own: boolean;
  channelId: string;
  viewingChannelId: string | undefined;
}): { toast: boolean; desktop: boolean } {
  return {
    toast: shouldToastMessage({ ...input, enabled: input.toastEnabled }),
    desktop:
      input.desktopEnabled && input.hidden && input.type === "c" && !input.own,
  };
}

/** Where a tap on a message notification leads, as the router spells it. */
export function conversationPath(
  dm: boolean,
  serverId: string,
  channelId: string,
): string {
  return dm
    ? `/d/${encodeURIComponent(channelId)}`
    : `/s/${encodeURIComponent(serverId)}/c/${encodeURIComponent(channelId)}`;
}

/**
 * A notification tap comes back through the service worker as a message.
 * Only an address `conversationPath` could have produced is navigated to.
 */
export function isConversationPath(value: unknown): value is string {
  return (
    typeof value === "string" &&
    /^\/(?:d\/[^/?#]+|s\/[^/?#]+\/c\/[^/?#]+)$/.test(value)
  );
}

export type NotificationPermissionState =
  "granted" | "denied" | "default" | "unsupported";

/**
 * What the permission box in the settings says. The reasons a browser has no
 * notifications differ by device, and "blocked" is the wrong advice where the
 * address itself rules them out.
 */
export function notificationPermissionText(
  state: NotificationPermissionState,
  device: { secure: boolean; ios: boolean; standalone: boolean },
): string {
  if (state === "granted") return "Browser-Benachrichtigungen sind erlaubt.";
  if (state === "default")
    return "Der Browser benötigt noch deine Erlaubnis für Benachrichtigungen.";
  if (!device.secure)
    return "Benachrichtigungen gibt es nur, wenn Gelabber über HTTPS geöffnet ist.";
  if (state === "denied")
    return "Browser-Benachrichtigungen sind blockiert. Du kannst sie in den Website-Einstellungen deines Browsers erlauben.";
  if (device.ios && !device.standalone)
    return "Auf iPhone und iPad gibt es Benachrichtigungen nur in der installierten App. Füge Gelabber über „Teilen“ zum Home-Bildschirm hinzu und öffne es von dort.";
  return "Dieser Browser unterstützt hier keine Desktop-Benachrichtigungen.";
}

/** Bound replay suppression to the active account/session listener. */
export function createNotificationDedupe(limit = 2048) {
  const seen = new Set<string>();
  return (channelId: string, messageId: string): boolean => {
    const key = `${channelId}:${messageId}`;
    if (seen.has(key)) return false;
    seen.add(key);
    if (seen.size > limit) seen.delete(seen.values().next().value!);
    return true;
  };
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
