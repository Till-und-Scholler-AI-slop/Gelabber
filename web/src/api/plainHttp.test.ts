// A plain-http origin (API_ALLOW_PLAIN_HTTP) has no `navigator.locks`. Kept
// apart from client.test.ts: the modules are loaded fresh against a browser
// stand-in without a LockManager, and their state must not leak elsewhere.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const ada = {
  id: "user-a",
  email: "a@example.com",
  name: "A",
  avatar_url: null,
  created_at: "2026-09-28T09:00:00Z",
};

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

function memoryStorage(): Storage {
  const items = new Map<string, string>();
  return {
    get length() {
      return items.size;
    },
    clear: () => items.clear(),
    getItem: (key) => items.get(key) ?? null,
    key: (index) => [...items.keys()][index] ?? null,
    removeItem: (key) => {
      items.delete(key);
    },
    setItem: (key, value) => {
      items.set(key, String(value));
    },
  };
}

type Route = (init: RequestInit) => Response | Promise<Response>;

/** Routes by "METHOD path" and records every call in order. */
function install(routes: Record<string, Route>) {
  const calls: string[] = [];
  vi.stubGlobal(
    "fetch",
    vi.fn((input: RequestInfo | URL, init: RequestInit = {}) => {
      const key = `${init.method ?? "GET"} ${String(input)}`;
      calls.push(key);
      const route = routes[key];
      if (!route) throw new Error(`unexpected request ${key}`);
      return Promise.resolve(route(init));
    }),
  );
  return calls;
}

async function load() {
  vi.resetModules();
  vi.stubGlobal(
    "window",
    Object.assign(new EventTarget(), { localStorage: memoryStorage() }),
  );
  vi.stubGlobal(
    "document",
    Object.assign(new EventTarget(), { visibilityState: "visible" }),
  );
  vi.stubGlobal("navigator", {});
  const client = await import("./client.ts");
  const session = await import("../auth/session.ts");
  return { client, session };
}

let warn: ReturnType<typeof vi.spyOn>;

beforeEach(() => {
  warn = vi.spyOn(console, "warn").mockImplementation(() => undefined);
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("plain-http origin without Web Locks", () => {
  it("bootstraps the session", async () => {
    const calls = install({
      "GET /api/auth/session": () => json({ user: ada, csrf_token: "c1" }),
    });
    const { client, session } = await load();
    await session.ensureSession();
    expect(calls).toEqual(["GET /api/auth/session"]);
    expect(session.useSessionProblems.getState().bootstrap).toBeNull();
    expect(session.useSession.getState()).toEqual({
      status: "authenticated",
      user: ada,
    });
    expect(client.getCsrfToken()).toBe("c1");
    expect(warn).toHaveBeenCalledTimes(1);
  });

  it("signs in and serves ordinary requests", async () => {
    const calls = install({
      "POST /api/auth/login": () => json({ user: ada, csrf_token: "c1" }),
      "GET /api/servers": () => json([{ id: "server" }]),
    });
    const { client, session } = await load();
    await expect(session.login(ada.email, "password123")).resolves.toEqual(ada);
    expect(session.useSession.getState().user).toEqual(ada);
    await expect(client.api("/servers")).resolves.toEqual([{ id: "server" }]);
    expect(calls).toEqual(["POST /api/auth/login", "GET /api/servers"]);
    // Warned once per page, not per request.
    expect(warn).toHaveBeenCalledTimes(1);
  });

  it("runs cookie-changing requests strictly one after another", async () => {
    const events: string[] = [];
    const first = deferred<Response>();
    let sessions = 0;
    install({
      "POST /api/auth/login": () => json({ user: ada, csrf_token: "c1" }),
      "GET /api/auth/session": () => {
        const index = ++sessions;
        events.push(`fetch ${index}`);
        return index === 1
          ? first.promise
          : json({ user: ada, csrf_token: `c${index + 1}` });
      },
    });
    const { client, session } = await load();
    await session.login(ada.email, "password123");
    const one = client.api("/auth/session");
    const two = client.api("/auth/session");
    await vi.waitFor(() => expect(events).toEqual(["fetch 1"]));
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(events).toEqual(["fetch 1"]);
    events.push("answer 1");
    first.resolve(json({ user: ada, csrf_token: "c2" }));
    await Promise.all([one, two]);
    expect(events).toEqual(["fetch 1", "answer 1", "fetch 2"]);
    expect(client.getCsrfToken()).toBe("c3");
  });

  it("rejects a request aborted while queued without sending it", async () => {
    const first = deferred<Response>();
    let sessions = 0;
    install({
      "POST /api/auth/login": () => json({ user: ada, csrf_token: "c1" }),
      "GET /api/auth/session": () =>
        ++sessions === 1
          ? first.promise
          : json({ user: ada, csrf_token: "c3" }),
    });
    const { client, session } = await load();
    await session.login(ada.email, "password123");
    const one = client.api("/auth/session");
    const controller = new AbortController();
    const two = client.api("/auth/session", { signal: controller.signal });
    const rejected = expect(two).rejects.toMatchObject({ name: "AbortError" });
    await vi.waitFor(() => expect(sessions).toBe(1));
    controller.abort();
    first.resolve(json({ user: ada, csrf_token: "c2" }));
    await one;
    await rejected;
    expect(sessions).toBe(1);
    // The queue keeps working after an aborted entry.
    await client.api("/auth/session");
    expect(sessions).toBe(2);
  });

  // The auth actions recheck the signal themselves, so the case above passes
  // either way. This one pins the queue's own check, as with Web Locks.
  it("never runs an action that was aborted while queued", async () => {
    const { client } = await load();
    const first = deferred<void>();
    const ran: string[] = [];
    const one = client.withSessionCookieLockForTests(async () => {
      ran.push("one");
      await first.promise;
    });
    const controller = new AbortController();
    const two = client.withSessionCookieLockForTests(async () => {
      ran.push("two");
    }, controller.signal);
    const rejected = expect(two).rejects.toMatchObject({ name: "AbortError" });
    await vi.waitFor(() => expect(ran).toEqual(["one"]));
    controller.abort();
    first.resolve();
    await one;
    await rejected;
    expect(ran).toEqual(["one"]);
  });
});
