// Notification settings per server and per channel or DM (issue #242):
// a level and a mute. The unread rows carry the effective result, so badges
// and notifications read those; this module only lists and changes the own
// settings, for the menu that shows them.
import {
  useQuery,
  useQueryClient,
  type QueryClient,
} from "@tanstack/react-query";
import { useEffect, useState } from "react";

import { api, serverNow } from "../api/client.ts";
import {
  scopeGeneration,
  stampHolds,
  useUserId,
  type ScopeStamp,
} from "../auth/scope.ts";
import { getGateway } from "../ws/client.ts";
import { isMuted, type NotifyLevel } from "./notifyLevel.ts";
import { refreshReadState, useReadState } from "./readState.ts";

export type NotificationScope = "server" | "channel";

export type NotificationSetting = {
  scope: NotificationScope;
  id: string;
  /** null: as the server (a channel) or "all" (a server). */
  level: NotifyLevel | null;
  muted: boolean;
  muted_until: string | null;
};

export type SettingChange = {
  level: NotifyLevel | null;
  muted: boolean;
  /** With `muted`: for this long; absent until turned back on. */
  mute_minutes?: number;
};

export const settingsKey = (stamp: ScopeStamp) =>
  ["user", stamp.userId, stamp.generation, "notification-settings"] as const;

export const listNotificationSettings = (signal?: AbortSignal) =>
  api<NotificationSetting[]>("/notification-settings", { signal });

export const putNotificationSetting = (
  scope: NotificationScope,
  id: string,
  change: SettingChange,
) =>
  api<NotificationSetting>(
    `/${scope === "server" ? "servers" : "channels"}/${id}/notification-settings`,
    { method: "PUT", body: change },
  );

export function useNotificationSettings() {
  const userId = useUserId();
  const generation = scopeGeneration();
  return useQuery({
    queryKey: settingsKey({ userId, generation }),
    queryFn: ({ signal }) => listNotificationSettings(signal),
    enabled: Boolean(userId),
    staleTime: 60_000,
  });
}

/** The own setting of one server or conversation; inheriting when absent. */
export function settingOf(
  settings: readonly NotificationSetting[] | undefined,
  scope: NotificationScope,
  id: string,
): NotificationSetting {
  return (
    settings?.find((row) => row.scope === scope && row.id === id) ?? {
      scope,
      id,
      level: null,
      muted: false,
      muted_until: null,
    }
  );
}

async function refreshAll(client: QueryClient, stamp: ScopeStamp) {
  if (!stampHolds(stamp)) return;
  await Promise.all([
    client.invalidateQueries({ queryKey: settingsKey(stamp), exact: true }),
    refreshReadState(client, stamp),
  ]);
}

/** Changes a setting, then the badges that follow from it. */
export function useChangeNotificationSetting() {
  const client = useQueryClient();
  const userId = useUserId();
  const generation = scopeGeneration();
  return async (
    scope: NotificationScope,
    id: string,
    change: SettingChange,
  ) => {
    const stamp = { userId, generation };
    const saved = await putNotificationSetting(scope, id, change);
    if (!stampHolds(stamp)) return saved;
    client.setQueryData<NotificationSetting[]>(settingsKey(stamp), (rows) => [
      ...(rows ?? []).filter((row) => row.scope !== scope || row.id !== id),
      ...(saved.level || saved.muted ? [saved] : []),
    ]);
    await refreshReadState(client, stamp);
    return saved;
  };
}

/** Another device (or tab) changed a setting: the gateway says so. */
export function useNotificationSettingsBridge(): void {
  const client = useQueryClient();
  const userId = useUserId();
  const generation = scopeGeneration();
  useEffect(() => {
    if (!userId) return;
    const stamp = { userId, generation };
    return getGateway().onAccount((frame) => {
      if (frame.op === "ns") void refreshAll(client, stamp);
    });
  }, [client, userId, generation]);
}

/** The server clock for mutes, ticking only while a mute with an end is
 * running: a badge comes back when its mute runs out. */
export function useMuteClock(): number {
  const { data } = useReadState();
  const timed = (data ?? []).some((row) => row.muted && row.muted_until);
  const [, setTick] = useState(0);
  useEffect(() => {
    if (!timed) return;
    const timer = window.setInterval(() => setTick((n) => n + 1), 30_000);
    return () => window.clearInterval(timer);
  }, [timed]);
  return serverNow();
}

/** The conversation is muted (its own mute or its server's) right now. */
export function useChannelMuted(channelId: string): boolean {
  const now = useMuteClock();
  const { data } = useReadState();
  return isMuted(
    data?.find((row) => row.channel_id === channelId),
    now,
  );
}

/** The server itself is muted right now. */
export function useServerMuted(serverId: string): boolean {
  const now = useMuteClock();
  const { data } = useNotificationSettings();
  return isMuted(settingOf(data, "server", serverId), now);
}
