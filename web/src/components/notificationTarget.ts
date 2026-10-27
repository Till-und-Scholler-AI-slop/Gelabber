// Which server or conversation the notification dialog is open for.
import { create } from "zustand";

import type { NotificationScope } from "../messages/notificationSettings.ts";
import { useHoldMenu } from "./useHoldMenu.ts";

export type NotificationTarget = {
  scope: NotificationScope;
  id: string;
  /** "#allgemein", a server's or a person's name. */
  name: string;
  /** A server channel: inherits from this server. */
  serverId?: string;
  dm?: boolean;
  /** All direct messages (the rail's home tile): only "mark as read". */
  allDms?: boolean;
};

export const useNotificationTarget = create<{
  target: NotificationTarget | null;
}>(() => ({
  target: null,
}));

export function openNotificationSettings(target: NotificationTarget): void {
  useNotificationTarget.setState({ target });
}

/**
 * A right click or a held finger on an entry: the dialog instead of the
 * browser's menu.
 */
export function useNotificationMenu(target: NotificationTarget) {
  const open = () => openNotificationSettings(target);
  const hold = useHoldMenu(open);
  return {
    onContextMenu: (event: { preventDefault: () => void }) => {
      event.preventDefault();
      open();
    },
    ...hold,
  };
}

/** Minutes left of a running mute, so a level change keeps its end. */
export function minutesUntil(until: string, now: number): number {
  return Math.max(1, Math.ceil((Date.parse(until) - now) / 60_000));
}
