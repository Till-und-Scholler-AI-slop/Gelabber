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
// itself. Nothing arrives here while the app is closed: there is no push.
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
