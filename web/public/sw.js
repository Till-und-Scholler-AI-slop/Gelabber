/* Only a public offline page is cached. App assets and authenticated data
   stay on the network, so deployment updates never mix cached app versions. */
/* global self, caches, fetch, URL, Request, Response */
const CACHE_PREFIX = "gelabber-pwa-offline-";
// Raise with every change to offline.html: nothing else replaces the cached
// copy. src/pwa/worker.test.mjs fails when the page changes without it.
const CACHE_NAME = `${CACHE_PREFIX}v2`;
const OFFLINE_URL = "/offline.html";
// Pages of the app itself. API, gateway/media traffic, attachment downloads
// and static files never match.
const APP_PAGE =
  /^(?:\/(?:login|register|profile|settings)\/?|\/(?:s|d|invite)(?:\/.*)?|\/)$/;

self.addEventListener("install", (event) => {
  event.waitUntil(
    caches
      .open(CACHE_NAME)
      .then((cache) =>
        cache.add(new Request(OFFLINE_URL, { cache: "reload" })),
      ),
  );
  // Updates wait for existing tabs to close. Never reload an active call.
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    caches.keys().then(async (names) => {
      await Promise.all(
        names
          .filter(
            (name) => name.startsWith(CACHE_PREFIX) && name !== CACHE_NAME,
          )
          .map((name) => caches.delete(name)),
      );
      await self.clients.claim();
    }),
  );
});

self.addEventListener("fetch", (event) => {
  const request = event.request;
  const url = new URL(request.url);
  // Only real app-page navigations receive an offline fallback. In particular,
  // API, gateway/media traffic and attachment downloads are never intercepted.
  if (
    request.method !== "GET" ||
    request.mode !== "navigate" ||
    url.origin !== self.location.origin ||
    !APP_PAGE.test(url.pathname)
  ) {
    return;
  }
  event.respondWith(
    fetch(request).catch(async () => {
      const cache = await caches.open(CACHE_NAME);
      const offline = await cache.match(OFFLINE_URL);
      if (offline) return offline;
      return Response.error();
    }),
  );
});

// Message notifications are shown through this worker's registration
// (src/pwa/notifications.ts), because phones do not let a page create one
// itself. While the app is closed, direct messages and mentions arrive here
// as Web Push (src/pwa/push.ts, api/src/push).
function notificationTarget(data) {
  try {
    const url = new URL(data.path, self.location.origin);
    if (url.origin === self.location.origin && APP_PAGE.test(url.pathname)) {
      return url.pathname;
    }
  } catch {
    // No usable address on the notification.
  }
  return "/";
}

self.addEventListener("notificationclick", (event) => {
  const data = event.notification.data ?? {};
  const path = notificationTarget(data);
  event.notification.close();
  event.waitUntil(
    self.clients
      .matchAll({ type: "window", includeUncontrolled: true })
      .then(async (clients) => {
        // A tab showing an attachment shares the origin but is not the app.
        const windows = clients.filter((client) =>
          APP_PAGE.test(new URL(client.url).pathname),
        );
        const open =
          windows.find((client) => client.focused) ??
          windows.find((client) => client.visibilityState === "visible") ??
          windows[0];
        if (!open) {
          await self.clients.openWindow(path);
          return;
        }
        // The running app changes route itself. Navigating its window from
        // here would reload the page and end a call.
        open.postMessage({
          type: "gelabber:open-conversation",
          path,
          user: data.user,
        });
        await open.focus().catch(() => {});
      }),
  );
});

// The same order as `chronological` in src/messages/pages.ts, enough for two
// RFC 3339 times of the server: the time, then the id.
function notOlder(shown, message, at) {
  if (shown.message === message) return true;
  if (typeof shown.at !== "string") return false;
  const a = Date.parse(shown.at);
  const b = Date.parse(at);
  if (a !== b) return a > b;
  return shown.at === at ? shown.message > message : shown.at > at;
}

// The lock the windows take while they show or close the account's
// notifications (`alone` in src/pwa/notifications.ts).
function alone(user, change) {
  const locks = self.navigator && self.navigator.locks;
  if (!locks) return change();
  return locks
    .request(`gelabber:notifications:${user}`, change)
    .catch(() => change());
}

// Every push shows a notification: a browser takes one that shows nothing as
// a silent push and may end the subscription (`userVisibleOnly`). That holds
// with a window open too, since the server pushes only when no window is in
// use. A window may announce the same message; the tag and the lock keep it
// to one.
async function showPushed(data) {
  const tag = `gelabber:${data.user}:${data.channel}`;
  await alone(data.user, async () => {
    const shown = await self.registration.getNotifications({ tag });
    // A window may have shown it already, or a newer one: show that again,
    // quietly, in its place.
    const current = shown.find(
      (n) => n.data && notOlder(n.data, data.message, data.at),
    );
    if (current) {
      await self.registration.showNotification(current.title, {
        body: current.body,
        tag,
        silent: true,
        icon: "/icons/icon-192.png",
        data: current.data,
      });
      return;
    }
    await self.registration.showNotification(data.title, {
      body: data.body,
      tag,
      renotify: true,
      icon: "/icons/icon-192.png",
      data: {
        path: data.path,
        user: data.user,
        message: data.message,
        at: data.at,
      },
    });
  });
}

self.addEventListener("push", (event) => {
  let data = null;
  try {
    data = event.data ? event.data.json() : null;
  } catch {
    // Not from Gelabber's server.
  }
  if (
    !data ||
    typeof data.title !== "string" ||
    typeof data.body !== "string" ||
    typeof data.user !== "string" ||
    typeof data.channel !== "string" ||
    typeof data.message !== "string" ||
    typeof data.at !== "string"
  )
    return;
  event.waitUntil(showPushed(data));
});
