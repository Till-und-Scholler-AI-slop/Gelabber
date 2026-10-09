// System notifications for messages. Phones do not let a page create one
// itself (`new Notification()` throws there), so with an active service worker
// the notification goes through its registration and public/sw.js handles the
// tap. Where no worker runs (vite dev, the desktop app, plain HTTP) the page
// shows it directly, as it always did. Either way this only works while the
// page is alive; delivery to a closed app would need Web Push.
import { isConversationPath } from "../messages/notify.ts";
import { activeServiceWorker } from "./register.ts";

/** Posted by sw.js to the window it brought forward after a tap. */
export const NOTIFICATION_OPEN = "gelabber:open-conversation";

export type MessageNotification = {
  title: string;
  body: string;
  /** Conversation it is about. A newer one replaces the older. */
  channelId: string;
  /** Where a tap leads; see `conversationPath` in messages/notify.ts. */
  path: string;
  /** Account it is shown for. A tap after an account change goes nowhere. */
  user: string;
};

type PageNotification = { onclick: (() => void) | null; close: () => void };
type Options = {
  body: string;
  silent: boolean;
  tag: string;
  renotify: boolean;
  icon?: string;
  data?: { path: string; user: string };
};
type NotificationApi = {
  permission: string;
  new (title: string, options: Options): PageNotification;
};
// The part of ServiceWorkerRegistration used here, with the options above
// (lib.dom no longer lists `renotify`).
type Registration = {
  showNotification(title: string, options: Options): Promise<void>;
  getNotifications(): Promise<{ tag: string; close(): void }[]>;
};

// One notification per account and conversation. Without a conversation this
// is what all tags of the account start with.
const tagOf = (user: string, channelId = "") => `gelabber:${user}:${channelId}`;
const shownByPage = new Map<string, PageNotification>();

/**
 * Show one notification. The caller has already decided that it is wanted;
 * permission is only ever requested from a button, never from here.
 * `onClick` runs for a notification the page created itself. A tap on one the
 * worker showed arrives through `followNotificationTaps`.
 */
export async function showMessageNotification(
  message: MessageNotification,
  onClick: () => void,
): Promise<"worker" | "page" | "none"> {
  const api = (globalThis as { Notification?: NotificationApi }).Notification;
  if (!api || api.permission !== "granted") return "none";
  const tag = tagOf(message.user, message.channelId);
  const options: Options = {
    body: message.body,
    silent: true,
    tag,
    renotify: false,
  };
  const registration: Registration | undefined = await activeServiceWorker();
  if (registration) {
    try {
      await registration.showNotification(message.title, {
        ...options,
        icon: "/icons/icon-192.png",
        data: { path: message.path, user: message.user },
      });
      return "worker";
    } catch {
      // A desktop browser can still show it from the page.
    }
  }
  try {
    shownByPage.get(tag)?.close();
    const notification = new api(message.title, options);
    shownByPage.set(tag, notification);
    notification.onclick = () => {
      notification.close();
      onClick();
    };
    return "page";
  } catch {
    // Permission revoked mid-flight, or a phone without an active worker.
    return "none";
  }
}

/**
 * Take down what is on screen for an account that leaves, or for one of its
 * conversations once that is being read. Never more than that: what the worker
 * shows belongs to every window of the app, and a window that merely starts
 * (as after a tap, when the phone had discarded the app) must leave the
 * notifications of the other conversations alone.
 */
export function closeMessageNotifications(of: {
  user: string;
  channelId?: string;
}): void {
  const tag = tagOf(of.user, of.channelId);
  const meant = (shown: string) =>
    of.channelId === undefined ? shown.startsWith(tag) : shown === tag;
  for (const [shown, notification] of shownByPage) {
    if (!meant(shown)) continue;
    notification.close();
    shownByPage.delete(shown);
  }
  void activeServiceWorker()
    .then(async (registration: Registration | undefined) => {
      for (const shown of (await registration?.getNotifications()) ?? [])
        if (meant(shown.tag)) shown.close();
    })
    .catch(() => {
      // Nothing to clean up where the worker cannot list its notifications.
    });
}

/**
 * A conversation that is on screen no longer needs its notification: it was
 * only shown because the page was hidden. Withdraws it now if the page is
 * visible, and whenever the page becomes visible with that conversation open
 * (a phone user coming back through the app switcher instead of the tap).
 */
export function withdrawWhileViewing(
  user: string,
  channelId: string,
  page: Pick<
    Document,
    "hidden" | "addEventListener" | "removeEventListener"
  > = document,
): () => void {
  const seen = () => {
    if (!page.hidden) closeMessageNotifications({ user, channelId });
  };
  seen();
  page.addEventListener("visibilitychange", seen);
  return () => page.removeEventListener("visibilitychange", seen);
}

/**
 * Follow a tap on a worker-shown notification inside the running app: sw.js
 * brings a window forward and names the conversation, the app changes route
 * itself. Nothing reloads, so a call survives.
 */
export function followNotificationTaps(app: {
  user: () => string | undefined;
  open: (path: string) => void;
}): () => void {
  if (typeof navigator === "undefined" || !navigator.serviceWorker)
    return () => {};
  const worker = navigator.serviceWorker;
  const follow = (event: MessageEvent) => {
    const data: unknown = event.data;
    if (data === null || typeof data !== "object") return;
    const { type, path, user } = data as Record<string, unknown>;
    if (type !== NOTIFICATION_OPEN || !isConversationPath(path)) return;
    if (typeof user !== "string" || user !== app.user()) return;
    app.open(path);
  };
  worker.addEventListener("message", follow);
  return () => worker.removeEventListener("message", follow);
}
