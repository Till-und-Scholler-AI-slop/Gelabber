// System notifications for messages. Phones do not let a page create one
// itself (`new Notification()` throws there), so with an active service worker
// the notification goes through its registration and public/sw.js handles the
// tap. Where no worker runs (vite dev, the desktop app, plain HTTP) the page
// shows it directly, as it always did. Either way this only works while the
// page is alive; delivery to a closed app would need Web Push.
import { isConversationPath } from "../messages/notify.ts";
import { useInstallation } from "./install.ts";
import { activeServiceWorker } from "./register.ts";

/** Posted by sw.js to the window it brought forward after a tap. */
export const NOTIFICATION_OPEN = "gelabber:open-conversation";

export type MessageNotification = {
  title: string;
  body: string;
  /** Conversation it is about. A newer one replaces the older. */
  channelId: string;
  /** The message itself. Every window hears it; only one may announce it. */
  messageId: string;
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
  data?: { path: string; user: string; message: string };
};
type NotificationApi = {
  permission: string;
  new (title: string, options: Options): PageNotification;
};
// The part of ServiceWorkerRegistration used here, with the options above
// (lib.dom no longer lists `renotify`).
type Registration = {
  showNotification(title: string, options: Options): Promise<void>;
  getNotifications(filter?: {
    tag: string;
  }): Promise<{ tag: string; data?: unknown; close(): void }[]>;
};

// One notification per account and conversation. Without a conversation this
// is what all tags of the account start with.
const tagOf = (user: string, channelId = "") => `gelabber:${user}:${channelId}`;
const shownByPage = new Map<string, PageNotification>();

// Every window of the app hears the same messages, and each decides from its
// own visibility. Two Web Locks keep the windows of one account in step; the
// lock manager is shared by all windows of the origin and lets go of the locks
// of a window that closes or crashes.
//  - A visible window holds the first one, shared, for the conversation it
//    shows. A window in the background that finds it held shows nothing.
//  - The second is held, alone, for the moment a window shows, lists or
//    closes notifications of the account. A window coming back on screen
//    therefore closes what another one was about to show, instead of missing
//    it. And Chromium loses a notification that one window shows while
//    another one lists them: it is gone from every later list.
const viewingLock = (user: string, channelId: string) =>
  `gelabber:viewing:${user}:${channelId}`;
const changeLock = (user: string) => `gelabber:notifications:${user}`;
const lockManager = (): LockManager | undefined =>
  typeof navigator === "undefined" ? undefined : navigator.locks;
// Viewing locks this window has let go of. The lock manager may still list
// them for a moment.
let letGo: Promise<unknown> = Promise.resolve();
// How long one change may keep the other windows waiting. A browser call that
// never answers must not stop every window's notifications for good.
const CHANGE_HOLD_MS = 5_000;

/** Run `change` while no other window shows or closes this account's notifications. */
async function alone<T>(user: string, change: () => Promise<T>): Promise<T> {
  const locks = lockManager();
  if (!locks) return change();
  let changing: Promise<T> | undefined;
  try {
    await locks.request(changeLock(user), () => {
      const work = (changing = change());
      return new Promise<void>((through) => {
        const limit = setTimeout(through, CHANGE_HOLD_MS);
        const done = () => {
          clearTimeout(limit);
          through();
        };
        work.then(done, done);
      });
    });
  } catch {
    // The lock manager refused (blocked storage, for one). That must not
    // cost the notification.
  }
  return changing ?? change();
}

/** Whether a visible window of the app shows this conversation right now. */
async function onScreen(user: string, channelId: string): Promise<boolean> {
  const locks = lockManager();
  if (!locks) return false;
  try {
    await letGo;
    const name = viewingLock(user, channelId);
    const { held = [] } = await locks.query();
    return held.some((lock) => lock.name === name);
  } catch {
    return false;
  }
}

/** Whether a window of the app has already put this message on screen. */
async function announced(
  registration: Registration,
  tag: string,
  messageId: string,
): Promise<boolean> {
  try {
    return (await registration.getNotifications({ tag })).some(
      (shown) =>
        (shown.data as { message?: unknown } | null | undefined)?.message ===
        messageId,
    );
  } catch {
    return false;
  }
}

/**
 * Show one notification, unless a visible window of the app shows that
 * conversation or another window has already shown this message. Apart from
 * that the caller has decided that it is wanted; permission is only ever
 * requested from a button, never from here.
 * `onClick` runs for a notification the page created itself. A tap on one the
 * worker showed arrives through `followNotificationTaps`.
 * The result names who showed it: the worker, the page, nobody because the
 * conversation is `viewed` in a visible window, or nobody for another reason.
 *
 * On a phone the notification is all there is to notice a message by: it
 * sounds and vibrates as the device is set, and again for each later message
 * of the conversation (the caller spaces those out). On a desktop it stays
 * silent, as it always was.
 */
export async function showMessageNotification(
  message: MessageNotification,
  onClick: () => void,
): Promise<"worker" | "page" | "viewed" | "none"> {
  const api = (globalThis as { Notification?: NotificationApi }).Notification;
  if (!api || api.permission !== "granted") return "none";
  return alone(message.user, async () => {
    if (await onScreen(message.user, message.channelId)) return "viewed";
    const tag = tagOf(message.user, message.channelId);
    const phone = useInstallation.getState().mobile;
    const options: Options = {
      body: message.body,
      silent: !phone,
      tag,
      // Without it a newer message replaces the older one unnoticed.
      renotify: phone,
    };
    const registration: Registration | undefined = await activeServiceWorker();
    if (registration) {
      // Showing it a second time would sound a second time.
      if (await announced(registration, tag, message.messageId)) return "none";
      try {
        await registration.showNotification(message.title, {
          ...options,
          icon: "/icons/icon-192.png",
          data: {
            path: message.path,
            user: message.user,
            message: message.messageId,
          },
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
  }).catch(() => "none" as const);
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
}): Promise<void> {
  const tag = tagOf(of.user, of.channelId);
  const meant = (shown: string) =>
    of.channelId === undefined ? shown.startsWith(tag) : shown === tag;
  for (const [shown, notification] of shownByPage) {
    if (!meant(shown)) continue;
    notification.close();
    shownByPage.delete(shown);
  }
  return alone(of.user, async () => {
    const registration: Registration | undefined = await activeServiceWorker();
    for (const shown of (await registration?.getNotifications()) ?? [])
      if (meant(shown.tag)) shown.close();
  }).catch(() => {
    // Nothing to clean up where the worker cannot list its notifications.
  });
}

/**
 * This window shows the conversation, until the returned function is called.
 * Its notification goes, and no window of the app raises a new one.
 */
function viewing(user: string, channelId: string): () => void {
  const withdraw = () => void closeMessageNotifications({ user, channelId });
  const locks = lockManager();
  if (!locks) {
    withdraw();
    return () => {};
  }
  let left = false;
  let leave = () => {};
  const until = new Promise<void>((resolve) => {
    leave = resolve;
  });
  const released = locks
    .request(viewingLock(user, channelId), { mode: "shared" }, () => {
      // Held from here on. A window that shows a notification after this
      // finds the lock; what one showed before this is taken away now.
      if (!left) withdraw();
      return until;
    })
    .catch(() => {
      if (!left) withdraw();
    });
  return () => {
    left = true;
    leave();
    letGo = Promise.all([letGo, released]);
  };
}

/**
 * A conversation that is on screen needs no notification. While the page is
 * visible its notification is withdrawn and the other windows of the app are
 * kept from raising one; both again whenever the page becomes visible with
 * that conversation open (a phone user coming back through the app switcher
 * instead of the tap).
 */
export function silenceWhileViewing(
  user: string,
  channelId: string,
  page: Pick<
    Document,
    "hidden" | "addEventListener" | "removeEventListener"
  > = document,
): () => void {
  let leave: (() => void) | undefined;
  const seen = () => {
    if (page.hidden) {
      leave?.();
      leave = undefined;
    } else leave ??= viewing(user, channelId);
  };
  seen();
  page.addEventListener("visibilitychange", seen);
  return () => {
    page.removeEventListener("visibilitychange", seen);
    leave?.();
  };
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
