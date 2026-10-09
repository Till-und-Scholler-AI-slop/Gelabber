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
  /** Replaces an older notification of the same conversation. */
  tag: string;
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

const TAG_PREFIX = "gelabber:";
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
  const options: Options = {
    body: message.body,
    silent: true,
    tag: message.tag,
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
    shownByPage.get(message.tag)?.close();
    const notification = new api(message.title, options);
    shownByPage.set(message.tag, notification);
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

/** Take down what is still on screen, for example when the account changes. */
export function closeMessageNotifications(): void {
  for (const notification of shownByPage.values()) notification.close();
  shownByPage.clear();
  void activeServiceWorker()
    .then(async (registration: Registration | undefined) => {
      for (const shown of (await registration?.getNotifications()) ?? [])
        if (shown.tag.startsWith(TAG_PREFIX)) shown.close();
    })
    .catch(() => {
      // Nothing to clean up where the worker cannot list its notifications.
    });
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
