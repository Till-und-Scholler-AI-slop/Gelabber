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
