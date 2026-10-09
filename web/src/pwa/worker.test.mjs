/* global Response, Request, URL */
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import { describe, expect, it, vi } from "vitest";

const source = readFileSync(
  new URL("../../public/sw.js", import.meta.url),
  "utf8",
);
const offlinePage = readFileSync(
  new URL("../../public/offline.html", import.meta.url),
  "utf8",
);
const origin = "https://gelabber.example";
// The cache name under which browsers keep their copy of offline.html, and
// the page it was last raised for. See the test at the end of this file.
const OFFLINE_CACHE = "gelabber-pwa-offline-v2";
const OFFLINE_PAGE_SHA256 =
  "9575b6ef542b870852a51873acbf649baf6231415f8899f80e8cb115b64c76d8";

function worker() {
  const handlers = new Map();
  const entries = new Map();
  const fetch = vi.fn().mockResolvedValue(new Response("public offline page"));
  const add = vi.fn(async (request) => {
    entries.set(new URL(request.url).pathname, await fetch(request));
  });
  const claim = vi.fn().mockResolvedValue(undefined);
  const deleteCache = vi.fn().mockResolvedValue(true);
  const names = [
    "gelabber-pwa-offline-v0",
    "gelabber-pwa-offline-v1",
    OFFLINE_CACHE,
    "unrelated-cache",
  ];
  runInNewContext(source, {
    self: {
      location: { origin },
      clients: { claim },
      addEventListener: (name, handler) => handlers.set(name, handler),
    },
    caches: {
      open: async () => ({
        add,
        match: async (path) => entries.get(path),
      }),
      keys: async () => names,
      delete: deleteCache,
    },
    fetch,
    URL,
    Request: class extends Request {
      constructor(path, init) {
        super(new URL(path, origin), init);
      }
    },
    Response,
  });
  async function lifecycle(name) {
    let pending;
    handlers.get(name)({
      waitUntil: (value) => {
        pending = value;
      },
    });
    await pending;
  }
  function navigate(path, mode = "navigate", method = "GET") {
    let response;
    handlers.get("fetch")({
      request: { url: new URL(path, origin).href, mode, method },
      respondWith: (value) => {
        response = value;
      },
    });
    return response;
  }
  return { lifecycle, navigate, fetch, add, claim, deleteCache, entries };
}

describe("PWA offline worker", () => {
  it("precaches only the public offline document, with a fresh network request", async () => {
    const w = worker();
    await w.lifecycle("install");
    expect(w.add).toHaveBeenCalledOnce();
    expect(w.add.mock.calls[0][0].cache).toBe("reload");
    expect([...w.entries.keys()]).toEqual(["/offline.html"]);
  });

  it("deletes only obsolete caches owned by this feature", async () => {
    const w = worker();
    await w.lifecycle("activate");
    expect(w.deleteCache.mock.calls).toEqual([
      ["gelabber-pwa-offline-v0"],
      ["gelabber-pwa-offline-v1"],
    ]);
    expect(w.claim).toHaveBeenCalledOnce();
  });

  it("passes live app documents through without caching them", async () => {
    const w = worker();
    const network = new Response("fresh app shell");
    w.fetch.mockResolvedValue(network);
    expect(await w.navigate("/s/server/c/channel")).toBe(network);
    expect(w.add).not.toHaveBeenCalled();
  });

  it("serves the public offline page for unreachable app deep links", async () => {
    const w = worker();
    await w.lifecycle("install");
    w.fetch.mockRejectedValue(new TypeError("offline"));
    for (const path of [
      "/",
      "/login",
      "/settings",
      "/s/abc/c/xyz",
      "/d/123",
      "/invite/code",
    ]) {
      expect(await w.navigate(path)).toBe(w.entries.get("/offline.html"));
    }
    expect(w.add).toHaveBeenCalledOnce();
  });

  it("never substitutes HTML for API, media, uploads or static resources", async () => {
    const w = worker();
    for (const path of [
      "/api",
      "/api/auth/session",
      "/ws",
      "/media/ready",
      "/health",
      "/ready",
      "/bucket/file.png",
      "/assets/app.js",
      "/icons/icon-192.png",
      "https://objects.example/file",
    ]) {
      expect(w.navigate(path)).toBeUndefined();
    }
    expect(w.navigate("/", "cors")).toBeUndefined();
    expect(w.navigate("/", "navigate", "POST")).toBeUndefined();
    expect(w.fetch).not.toHaveBeenCalled();
  });

  it("preserves HTTP failures instead of disguising them as offline success", async () => {
    const w = worker();
    const unavailable = new Response("upstream unavailable", { status: 503 });
    w.fetch.mockResolvedValue(unavailable);
    expect(await w.navigate("/")).toBe(unavailable);
  });

  it("returns a network error if the offline cache is unavailable", async () => {
    const w = worker();
    w.fetch.mockRejectedValue(new TypeError("offline"));
    expect((await w.navigate("/"))?.type).toBe("error");
  });

  it("refetches the offline page whenever that page changes", () => {
    // Browsers keep offline.html under the cache name in sw.js and only fetch
    // it again when that name changes. After editing offline.html, raise
    // CACHE_NAME in public/sw.js, then record the new name and hash above.
    const version = OFFLINE_CACHE.replace("gelabber-pwa-offline-", "");
    expect(source).toContain(
      "const CACHE_NAME = `${CACHE_PREFIX}" + version + "`;",
    );
    expect(
      createHash("sha256")
        .update(offlinePage.replaceAll("\r\n", "\n"))
        .digest("hex"),
    ).toBe(OFFLINE_PAGE_SHA256);
  });

  it("retries the address the user wanted, not the start page", () => {
    expect(offlinePage).toContain("location.reload()");
    expect(offlinePage).toContain('addEventListener("online", retry)');
    expect(offlinePage).not.toContain('href="/"');
  });
});
