/* Only a public offline page is cached. App assets and authenticated data
   stay on the network, so deployment updates never mix cached app versions. */
/* global self, caches, fetch, URL, Request, Response */
const CACHE_PREFIX = "gelabber-pwa-offline-";
const CACHE_NAME = `${CACHE_PREFIX}v1`;
const OFFLINE_URL = "/offline.html";

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
  const appPage =
    /^(?:\/(?:login|register|profile|settings)\/?|\/(?:s|d|invite)(?:\/.*)?|\/)$/;
  if (
    request.method !== "GET" ||
    request.mode !== "navigate" ||
    url.origin !== self.location.origin ||
    !appPage.test(url.pathname)
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
