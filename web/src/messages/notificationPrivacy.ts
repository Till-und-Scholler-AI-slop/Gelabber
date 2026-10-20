// One switch for the whole account (issue #268): notifications say only who
// wrote, not what or where, so a locked screen shows nothing more. The server
// sends Web Push that way (`hidden_title` in api/src/push/mod.rs); this
// module gives the page's own notifications the same text.
import {
  useQuery,
  useQueryClient,
  type QueryClient,
} from "@tanstack/react-query";

import { api } from "../api/client.ts";
import {
  scopeGeneration,
  stampHolds,
  useUserId,
  type ScopeStamp,
} from "../auth/scope.ts";

export type NotificationPrivacy = { hide_content: boolean };

export const privacyKey = (stamp: ScopeStamp) =>
  ["user", stamp.userId, stamp.generation, "notification-privacy"] as const;

export const getNotificationPrivacy = (signal?: AbortSignal) =>
  api<NotificationPrivacy>("/notification-privacy", { signal });

export const putNotificationPrivacy = (change: NotificationPrivacy) =>
  api<NotificationPrivacy>("/notification-privacy", {
    method: "PUT",
    body: change,
  });

export function useNotificationPrivacy() {
  const userId = useUserId();
  const generation = scopeGeneration();
  return useQuery({
    queryKey: privacyKey({ userId, generation }),
    queryFn: ({ signal }) => getNotificationPrivacy(signal),
    enabled: Boolean(userId),
    staleTime: 60_000,
  });
}

export function useChangeNotificationPrivacy() {
  const client = useQueryClient();
  const userId = useUserId();
  const generation = scopeGeneration();
  return async (change: NotificationPrivacy) => {
    const stamp = { userId, generation };
    const saved = await putNotificationPrivacy(change);
    if (stampHolds(stamp)) client.setQueryData(privacyKey(stamp), saved);
    return saved;
  };
}

/** Another device (or tab) changed it: the gateway's `ns` says so. */
export function refreshNotificationPrivacy(
  client: QueryClient,
  stamp: ScopeStamp,
): Promise<void> {
  return client.invalidateQueries({
    queryKey: privacyKey(stamp),
    exact: true,
  });
}

/**
 * Title and text of a notification for an account that hides the content.
 * Unknown (not loaded yet, or a server without the switch) counts as not
 * hidden, as the server's default is.
 */
export function notificationText(
  privacy: NotificationPrivacy | undefined,
  shown: { title: string; body: string; author: string },
): { title: string; body: string } {
  if (!privacy?.hide_content) return { title: shown.title, body: shown.body };
  return { title: `Neue Nachricht von ${shown.author}`, body: "" };
}
