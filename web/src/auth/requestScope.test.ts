import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { api, getCsrfToken } from "../api/client.ts";
import { queryClient, queryScopeUser } from "../queryClient.ts";
import { getGateway, resetGatewayForTests } from "../ws/client.ts";
import { takeSessionStamp, takeStamp } from "./scope.ts";
import {
  ensureSession,
  login,
  logout,
  register,
  resetSessionForTests,
  updateProfile,
  useSession,
} from "./session.ts";
import type { User } from "./types.ts";

const ada: User = {
  id: "user-a",
  email: "ada@example.com",
  name: "Ada",
  avatar_url: null,
  created_at: "2026-09-13T00:00:00Z",
};
const bob: User = {
  ...ada,
  id: "user-b",
  email: "bob@example.com",
  name: "Bob",
};

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

function deferred() {
  let resolve!: (response: Response) => void;
  const promise = new Promise<Response>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

type Call = { key: string; init: RequestInit };
function install(
  routes: Record<string, (call: Call) => Response | Promise<Response>>,
) {
  const calls: Call[] = [];
  vi.stubGlobal(
    "fetch",
    vi.fn((input: RequestInfo | URL, init?: RequestInit) => {
      const call = {
        key: `${init?.method ?? "GET"} ${String(input)}`,
        init: init ?? {},
      };
      calls.push(call);
      const route = routes[call.key];
      if (!route) throw new Error(`Unexpected request: ${call.key}`);
      return Promise.resolve(route(call));
    }),
  );
  return calls;
}

describe("controlled responses across session changes", () => {
  beforeEach(() => resetSessionForTests());
  afterEach(() => {
    resetSessionForTests();
    resetGatewayForTests();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it("does not repeat A's profile write after another tab changes the cookie to B", async () => {
    const firstPatch = deferred();
    let cookieUser = ada;
    let writes = 0;
    const calls = install({
      "POST /api/auth/login": () =>
        json(200, { user: ada, csrf_token: "csrf-a" }),
      "PATCH /api/me": () =>
        ++writes === 1
          ? firstPatch.promise
          : json(200, { ...cookieUser, name: "A's intended edit" }),
      "GET /api/auth/session": () =>
        json(200, { user: cookieUser, csrf_token: "csrf-b" }),
    });
    await login(ada.email, "password123");
    const oldStamp = takeStamp();
    const done = updateProfile({ name: "A's intended edit" }).catch(
      (error: unknown) => error,
    );

    cookieUser = bob; // Server-side cookie identity changes; this tab still knows A.
    firstPatch.resolve(json(403, { error: "csrf_invalid" }));
    expect(await done).toMatchObject({ name: "AbortError" });

    expect(calls.map((call) => call.key)).toEqual([
      "POST /api/auth/login",
      "PATCH /api/me",
      "GET /api/auth/session",
    ]);
    expect(writes).toBe(1);
    expect(useSession.getState().user).toEqual(bob);
    expect(takeStamp()?.generation).not.toBe(oldStamp?.generation);
    expect(queryScopeUser()).toBe(bob.id);
    expect(getCsrfToken()).toBe("csrf-b");
  });

  it("a late A 401 preserves B's session, cache, and gateway", async () => {
    const oldResponse = deferred();
    let who = ada;
    install({
      "POST /api/auth/login": () =>
        json(200, { user: who, csrf_token: `csrf-${who.id}` }),
      "GET /api/servers": () => oldResponse.promise,
    });
    await login(ada.email, "password123");
    const done = api("/servers").catch((error: unknown) => error);
    who = bob;
    await login(bob.email, "password123");
    const stamp = takeStamp()!;
    const key = ["user", bob.id, stamp.generation, "servers"];
    queryClient.setQueryData(key, ["B's server"]);
    getGateway().setTopics([{ s: "server-b", c: "channel-b" }]);
    const resetGateway = vi.spyOn(getGateway(), "resetSession");

    oldResponse.resolve(json(401, { error: "unauthenticated" }));
    expect(await done).toMatchObject({ code: "unauthenticated" });
    expect(useSession.getState().user).toEqual(bob);
    expect(takeStamp()).toEqual(stamp);
    expect(queryScopeUser()).toBe(bob.id);
    expect(queryClient.getQueryData(key)).toEqual(["B's server"]);
    expect(resetGateway).not.toHaveBeenCalled();
    expect(getCsrfToken()).toBe(`csrf-${bob.id}`);
  });

  it("retries exactly once with renewed CSRF when the same identity still holds", async () => {
    const firstPatch = deferred();
    let writes = 0;
    const calls = install({
      "POST /api/auth/login": () =>
        json(200, { user: ada, csrf_token: "csrf-old" }),
      "PATCH /api/me": () =>
        ++writes === 1
          ? firstPatch.promise
          : json(200, { ...ada, name: "New name" }),
      "GET /api/auth/session": () =>
        json(200, { user: ada, csrf_token: "csrf-new" }),
    });
    await login(ada.email, "password123");
    const stamp = takeStamp();
    const done = updateProfile({ name: "New name" });
    firstPatch.resolve(json(403, { error: "csrf_invalid" }));
    await done;

    const patches = calls.filter((call) => call.key === "PATCH /api/me");
    expect(patches).toHaveLength(2);
    expect(
      patches.map((call) => new Headers(call.init.headers).get("X-CSRF-Token")),
    ).toEqual(["csrf-old", "csrf-new"]);
    expect(patches[1]?.init.body).toBe(patches[0]?.init.body);
    expect(takeStamp()).toEqual(stamp);
    expect(useSession.getState().user?.name).toBe("New name");
  });

  it("skips bootstrap and retry when A's CSRF error arrives after B login", async () => {
    const firstPatch = deferred();
    let who = ada;
    let writes = 0;
    const calls = install({
      "POST /api/auth/login": () =>
        json(200, { user: who, csrf_token: `csrf-${who.id}` }),
      "PATCH /api/me": () =>
        ++writes === 1 ? firstPatch.promise : json(200, bob),
      "GET /api/auth/session": () =>
        json(200, { user: bob, csrf_token: "boot-b" }),
    });
    await login(ada.email, "password123");
    const done = updateProfile({ name: "Old edit" }).catch(
      (error: unknown) => error,
    );
    who = bob;
    await login(bob.email, "password123");
    firstPatch.resolve(json(403, { error: "csrf_invalid" }));

    expect(await done).toMatchObject({ name: "AbortError" });
    expect(calls.map((call) => call.key)).toEqual([
      "POST /api/auth/login",
      "PATCH /api/me",
      "POST /api/auth/login",
    ]);
    expect(useSession.getState().user).toEqual(bob);
    expect(getCsrfToken()).toBe(`csrf-${bob.id}`);
  });

  it("ignores a CSRF bootstrap already in flight when logout wins", async () => {
    const bootstrap = deferred();
    const bootstrapStarted = deferred();
    let writes = 0;
    const calls = install({
      "POST /api/auth/login": () =>
        json(200, { user: ada, csrf_token: "csrf-a" }),
      "PATCH /api/me": () =>
        ++writes === 1
          ? json(403, { error: "csrf_invalid" })
          : json(200, { ...ada, name: "Old edit" }),
      "GET /api/auth/session": () => {
        bootstrapStarted.resolve(json(200, {}));
        return bootstrap.promise;
      },
      "POST /api/auth/logout": () => json(200, { csrf_token: "csrf-out" }),
    });
    await login(ada.email, "password123");
    const done = updateProfile({ name: "Old edit" }).catch(
      (error: unknown) => error,
    );
    await bootstrapStarted.promise;
    const signedOut = logout();
    bootstrap.resolve(json(200, { user: ada, csrf_token: "late-csrf" }));
    await signedOut;

    expect(await done).toMatchObject({ name: "AbortError" });
    expect(calls.filter((call) => call.key === "PATCH /api/me")).toHaveLength(
      1,
    );
    expect(useSession.getState()).toEqual({ status: "anonymous", user: null });
    expect(getCsrfToken()).toBe("csrf-out");
  });

  it.each(["login", "register", "bootstrap"])(
    "a late %s cannot undo logout",
    async (operation) => {
      const oldResponse = deferred();
      const started = deferred();
      const hold = () => {
        started.resolve(json(200, {}));
        return oldResponse.promise;
      };
      const calls = install({
        "POST /api/auth/login": hold,
        "POST /api/auth/register": hold,
        "GET /api/auth/session": hold,
        "POST /api/auth/logout": () => json(200, { csrf_token: "csrf-out" }),
      });
      const done = (
        operation === "login"
          ? login(ada.email, "password123")
          : operation === "register"
            ? register(ada.email, "password123", ada.name)
            : ensureSession()
      ).catch((error: unknown) => error);
      await started.promise;
      const signedOut = logout();
      expect(calls.some((call) => call.key === "POST /api/auth/logout")).toBe(
        false,
      );
      oldResponse.resolve(json(200, { user: ada, csrf_token: "late-csrf" }));
      await done;
      await signedOut;

      expect(useSession.getState()).toEqual({
        status: "anonymous",
        user: null,
      });
      expect(getCsrfToken()).toBe("csrf-out");
      expect(queryScopeUser()).toBeNull();
    },
  );

  it("a late success cannot replace B's CSRF token", async () => {
    const oldResponse = deferred();
    let who = ada;
    install({
      "POST /api/auth/login": () =>
        json(200, { user: who, csrf_token: `csrf-${who.id}` }),
      "GET /api/servers": () => oldResponse.promise,
    });
    await login(ada.email, "password123");
    const done = api("/servers");
    who = bob;
    await login(bob.email, "password123");
    oldResponse.resolve(json(200, { csrf_token: "late-a", servers: [] }));
    await done;
    expect(getCsrfToken()).toBe(`csrf-${bob.id}`);
    expect(useSession.getState().user).toEqual(bob);
  });

  it("does not let an old anonymous 401 sign out a new anonymous-to-A login", async () => {
    const oldResponse = deferred();
    install({
      "GET /api/servers": () => oldResponse.promise,
      "POST /api/auth/login": () =>
        json(200, { user: ada, csrf_token: "csrf-a" }),
    });
    const done = api("/servers").catch((error: unknown) => error);
    await login(ada.email, "password123");
    oldResponse.resolve(json(401, { error: "unauthenticated" }));
    await done;
    expect(useSession.getState().user).toEqual(ada);
  });

  it.each([401, 403])(
    "rejects stale effects even after A logs back in (status %s)",
    async (status) => {
      const oldResponse = deferred();
      let writes = 0;
      const calls = install({
        "POST /api/auth/login": () =>
          json(200, { user: ada, csrf_token: "csrf-current" }),
        "POST /api/auth/logout": () => json(200, { csrf_token: "csrf-out" }),
        "PATCH /api/me": () =>
          ++writes === 1 ? oldResponse.promise : json(200, ada),
        "GET /api/auth/session": () =>
          json(200, { user: ada, csrf_token: "stale-bootstrap" }),
      });
      await login(ada.email, "password123");
      const firstStamp = takeStamp();
      const done = updateProfile({ name: "Old edit" }).catch(
        (error: unknown) => error,
      );
      await logout();
      await login(ada.email, "password123");
      oldResponse.resolve(
        json(status, {
          error: status === 401 ? "unauthenticated" : "csrf_invalid",
        }),
      );
      await done;

      expect(useSession.getState().user).toEqual(ada);
      expect(takeStamp()?.generation).not.toBe(firstStamp?.generation);
      expect(getCsrfToken()).toBe("csrf-current");
      expect(writes).toBe(1);
      expect(calls.some((call) => call.key === "GET /api/auth/session")).toBe(
        false,
      );
    },
  );

  it("checks the scope after reading a delayed response body", async () => {
    let finishBody!: (body: string) => void;
    const response = json(401, {});
    vi.spyOn(response, "text").mockImplementation(
      () =>
        new Promise<string>((resolve) => {
          finishBody = resolve;
        }),
    );
    const bodyStarted = deferred();
    const originalText = response.text;
    response.text = () => {
      bodyStarted.resolve(json(200, {}));
      return originalText();
    };
    let who = ada;
    install({
      "POST /api/auth/login": () =>
        json(200, { user: who, csrf_token: `csrf-${who.id}` }),
      "GET /api/servers": () => response,
    });
    await login(ada.email, "password123");
    const done = api("/servers").catch((error: unknown) => error);
    await bodyStarted.promise;
    who = bob;
    await login(bob.email, "password123");
    finishBody(JSON.stringify({ error: "unauthenticated" }));
    await done;
    expect(useSession.getState().user).toEqual(bob);
  });

  it("does not let a failed old bootstrap make the new login anonymous", async () => {
    const oldResponse = deferred();
    const started = deferred();
    install({
      "GET /api/auth/session": () => {
        started.resolve(json(200, {}));
        return oldResponse.promise;
      },
      "POST /api/auth/login": () =>
        json(200, { user: bob, csrf_token: "csrf-b" }),
    });
    const boot = ensureSession();
    await started.promise;
    const signedIn = login(bob.email, "password123");
    oldResponse.resolve(json(503, { error: "internal" }));
    await boot;
    await signedIn;
    expect(useSession.getState().user).toEqual(bob);
    expect(getCsrfToken()).toBe("csrf-b");
  });

  it("serializes a newer login behind an already-started login", async () => {
    const oldResponse = deferred();
    const started = deferred();
    let attempts = 0;
    install({
      "POST /api/auth/login": () =>
        ++attempts === 1
          ? (started.resolve(json(200, {})), oldResponse.promise)
          : json(200, { user: bob, csrf_token: "csrf-b" }),
    });
    const first = login(ada.email, "password123").catch(
      (error: unknown) => error,
    );
    await started.promise;
    const signedIn = login(bob.email, "password123");
    expect(attempts).toBe(1);
    oldResponse.resolve(json(200, { user: ada, csrf_token: "late-csrf" }));
    expect(await first).toMatchObject({ name: "AbortError" });
    await signedIn;
    expect(useSession.getState().user).toEqual(bob);
    expect(getCsrfToken()).toBe("csrf-b");
  });

  it("rejects an upload follow-up carrying a previous account's intent before fetch", async () => {
    let who = ada;
    const calls = install({
      "POST /api/auth/login": () =>
        json(200, { user: who, csrf_token: `csrf-${who.id}` }),
      "POST /api/channels/one/messages": () => json(200, {}),
    });
    await login(ada.email, "password123");
    const scope = takeSessionStamp();
    who = bob;
    await login(bob.email, "password123");
    await expect(
      api("/channels/one/messages", {
        method: "POST",
        body: { content: "A's upload" },
        scope,
      }),
    ).rejects.toMatchObject({ name: "AbortError" });
    expect(calls).toHaveLength(2);
  });

  it.each(["login", "register"])(
    "allows anonymous %s to retry after a CSRF refresh",
    async (operation) => {
      let attempts = 0;
      const path = `POST /api/auth/${operation}`;
      const calls = install({
        [path]: () =>
          ++attempts === 1
            ? json(403, { error: "csrf_invalid" })
            : json(200, { user: ada, csrf_token: "csrf-signed-in" }),
        "GET /api/auth/session": () =>
          json(200, { user: null, csrf_token: "csrf-anon" }),
      });
      if (operation === "login") await login(ada.email, "password123");
      else await register(ada.email, "password123", ada.name);

      expect(calls.map((call) => call.key)).toEqual([
        path,
        "GET /api/auth/session",
        path,
      ]);
      expect(new Headers(calls[2]?.init.headers).get("X-CSRF-Token")).toBe(
        "csrf-anon",
      );
      expect(useSession.getState().user).toEqual(ada);
      expect(getCsrfToken()).toBe("csrf-signed-in");
    },
  );

  it.each(["login", "register"])(
    "honors an explicit %s even if its CSRF bootstrap discovers another cookie account",
    async (operation) => {
      let attempts = 0;
      const path = `POST /api/auth/${operation}`;
      const calls = install({
        [path]: () =>
          ++attempts === 1
            ? json(403, { error: "csrf_invalid" })
            : json(200, { user: bob, csrf_token: "csrf-b" }),
        "GET /api/auth/session": () =>
          json(200, { user: ada, csrf_token: "cookie-a" }),
      });
      const observed: (string | null)[] = [];
      const unsubscribe = useSession.subscribe((state) =>
        observed.push(state.user?.id ?? null),
      );
      if (operation === "login") await login(bob.email, "password123");
      else await register(bob.email, "password123", bob.name);
      unsubscribe();
      expect(calls.map((call) => call.key)).toEqual([
        path,
        "GET /api/auth/session",
        path,
      ]);
      expect(new Headers(calls[2]?.init.headers).get("X-CSRF-Token")).toBe(
        "cookie-a",
      );
      expect(observed).toEqual([bob.id]);
      expect(useSession.getState().user).toEqual(bob);
      expect(getCsrfToken()).toBe("csrf-b");
    },
  );

  it("retries logout for A without restoring A in the UI", async () => {
    const sessionResponse = deferred();
    const sessionStarted = deferred();
    let attempts = 0;
    const calls = install({
      "POST /api/auth/login": () =>
        json(200, { user: ada, csrf_token: "csrf-a" }),
      "POST /api/auth/logout": () =>
        ++attempts === 1
          ? json(403, { error: "csrf_invalid" })
          : json(200, { csrf_token: "csrf-out" }),
      "GET /api/auth/session": () => {
        sessionStarted.resolve(json(200, {}));
        return sessionResponse.promise;
      },
    });
    await login(ada.email, "password123");
    const states: (string | null)[] = [];
    const unsubscribe = useSession.subscribe((state) =>
      states.push(state.user?.id ?? null),
    );
    const done = logout();
    await sessionStarted.promise;
    expect(useSession.getState().user).toBeNull();
    sessionResponse.resolve(
      json(200, { user: ada, csrf_token: "csrf-renewed" }),
    );
    await done;
    unsubscribe();

    expect(states).toEqual([null]);
    expect(attempts).toBe(2);
    expect(new Headers(calls[3]?.init.headers).get("X-CSRF-Token")).toBe(
      "csrf-renewed",
    );
    expect(useSession.getState().user).toBeNull();
    expect(getCsrfToken()).toBe("csrf-out");
  });

  it("does not retry A's logout against a different cookie account B", async () => {
    let attempts = 0;
    const calls = install({
      "POST /api/auth/login": () =>
        json(200, { user: ada, csrf_token: "csrf-a" }),
      "POST /api/auth/logout": () =>
        ++attempts === 1
          ? json(403, { error: "csrf_invalid" })
          : json(200, { csrf_token: "wrongly-signed-out-b" }),
      "GET /api/auth/session": () =>
        json(200, { user: bob, csrf_token: "csrf-b" }),
    });
    await login(ada.email, "password123");
    await logout();
    expect(calls.map((call) => call.key)).toEqual([
      "POST /api/auth/login",
      "POST /api/auth/logout",
      "GET /api/auth/session",
    ]);
    expect(useSession.getState().user).toBeNull();
    expect(getCsrfToken()).toBe("csrf-a");
  });

  it("a late logout response cannot replace B's token", async () => {
    const oldResponse = deferred();
    const started = deferred();
    let who = ada;
    install({
      "POST /api/auth/login": () =>
        json(200, { user: who, csrf_token: `csrf-${who.id}` }),
      "POST /api/auth/logout": () => {
        started.resolve(json(200, {}));
        return oldResponse.promise;
      },
    });
    await login(ada.email, "password123");
    const done = logout();
    await started.promise;
    who = bob;
    const signedIn = login(bob.email, "password123");
    oldResponse.resolve(json(200, { csrf_token: "late-csrf-out" }));
    await done;
    await signedIn;
    expect(useSession.getState().user).toEqual(bob);
    expect(getCsrfToken()).toBe(`csrf-${bob.id}`);
  });

  it("an aborted intent cannot retry even if the mock bootstrap returns successfully", async () => {
    const controller = new AbortController();
    const sessionResponse = deferred();
    const sessionStarted = deferred();
    let writes = 0;
    const calls = install({
      "POST /api/auth/login": () =>
        json(200, { user: ada, csrf_token: "csrf-a" }),
      "PATCH /api/me": () =>
        ++writes === 1 ? json(403, { error: "csrf_invalid" }) : json(200, ada),
      "GET /api/auth/session": () => {
        sessionStarted.resolve(json(200, {}));
        return sessionResponse.promise;
      },
    });
    await login(ada.email, "password123");
    const done = api("/me", {
      method: "PATCH",
      body: { name: "Old edit" },
      signal: controller.signal,
    }).catch((error: unknown) => error);
    await sessionStarted.promise;
    controller.abort();
    sessionResponse.resolve(
      json(200, { user: ada, csrf_token: "should-not-adopt" }),
    );
    expect(await done).toMatchObject({ name: "AbortError" });
    expect(calls.filter((call) => call.key === "PATCH /api/me")).toHaveLength(
      1,
    );
    expect(getCsrfToken()).toBe("csrf-a");
  });
});
