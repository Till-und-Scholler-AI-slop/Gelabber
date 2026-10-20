// One switch for the whole account (issue #268): notifications say only who
// wrote, not what or where, so a locked screen shows nothing more. The server
// sends Web Push that way (`hidden_title` in api/src/push/mod.rs); this
// module gives the page's own notifications the same text.
import {
  useQuery,
  useQueryClient,
  type QueryClient,
  type QueryState,
} from "@tanstack/react-query";

import { api, ApiError } from "../api/client.ts";
import { closeMessageNotifications } from "../pwa/notifications.ts";
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
    // A server without the switch stays that way; asking again won't help.
    retry: (failures, error) => !unsupported(error) && failures < 3,
  });
}

/** A server from before the switch (the desktop app may talk to one). */
const unsupported = (error: unknown) =>
  error instanceof ApiError && error.status === 404;

export function useChangeNotificationPrivacy() {
  const client = useQueryClient();
  const userId = useUserId();
  const generation = scopeGeneration();
  return async (change: NotificationPrivacy) => {
    const stamp = { userId, generation };
    const saved = await putNotificationPrivacy(change);
    if (stampHolds(stamp)) client.setQueryData(privacyKey(stamp), saved);
    // What is on screen already still shows the message: take it down.
    if (saved.hide_content) void closeMessageNotifications({ user: userId });
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
 * Whether a notification must leave the message out, from what the query
 * knows. Unknown counts as hidden: not loaded yet (a phone app resuming in the
 * background), or the last fetch failed. Only a server from before the switch
 * shows the message without asking, as it always did. A page notification
 * that showed too much would also replace the server's hidden push for the
 * same message (`showPushed` in public/sw.js).
 */
export function hidesContent(
  state: Pick<QueryState<NotificationPrivacy>, "data" | "error"> | undefined,
): boolean {
  if (state?.error) return !unsupported(state.error);
  return state?.data?.hide_content ?? true;
}

/** Title and text of a notification, without the message when it hides it. */
export function notificationText(
  hide: boolean,
  shown: { title: string; body: string; author: string },
): { title: string; body: string } {
  if (!hide) return { title: shown.title, body: shown.body };
  return { title: `Neue Nachricht von ${shown.author}`, body: "" };
}
