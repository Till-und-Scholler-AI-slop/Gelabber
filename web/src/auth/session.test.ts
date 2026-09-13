import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { ApiError, getCsrfToken } from "../api/client.ts";
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
  id: "u1",
  email: "ada@example.com",
  name: "Ada",
  avatar_url: null,
  created_at: "2026-09-13T00:00:00Z",
};

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

/** Routes by "METHOD path" and records every call. */
function fakeApi(routes: Record<string, () => Response | Promise<Response>>) {
  const calls: string[] = [];
  vi.stubGlobal(
    "fetch",
    vi.fn((input: RequestInfo | URL, init?: RequestInit) => {
      const key = `${init?.method ?? "GET"} ${String(input)}`;
      calls.push(key);
      const route = routes[key];
      if (!route) throw new Error(`unexpected request ${key}`);
      return Promise.resolve(route());
    }),
  );
  return calls;
}

describe("session store", () => {
  beforeEach(() => resetSessionForTests());
  afterEach(() => vi.unstubAllGlobals());

  it("bootstraps once and shares the request", async () => {
    const calls = fakeApi({
      "GET /api/auth/session": () => json(200, { user: ada, csrf_token: "c1" }),
    });

    await Promise.all([ensureSession(), ensureSession(), ensureSession()]);

    expect(calls).toEqual(["GET /api/auth/session"]);
    expect(useSession.getState()).toEqual({
      status: "authenticated",
      user: ada,
    });
    expect(getCsrfToken()).toBe("c1");

    await ensureSession();
    expect(calls).toHaveLength(1);
  });

  it("treats an unreachable API as anonymous so the login page can render", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(() => Promise.reject(new TypeError("Failed to fetch"))),
    );
    await ensureSession();
    expect(useSession.getState().status).toBe("anonymous");
  });

  it("login flips the store before the caller continues", async () => {
    fakeApi({
      "POST /api/auth/login": () => json(200, { user: ada, csrf_token: "c2" }),
    });

    const user = await login("ada@example.com", "password123");

    expect(user).toEqual(ada);
    expect(useSession.getState()).toEqual({
      status: "authenticated",
      user: ada,
    });
    expect(getCsrfToken()).toBe("c2");
  });

  it("login failure leaves the store anonymous and rethrows the ApiError", async () => {
    fakeApi({
      "GET /api/auth/session": () =>
        json(200, { user: null, csrf_token: "c0" }),
      "POST /api/auth/login": () =>
        json(401, { error: "invalid_credentials", message: "wrong" }),
    });
    await ensureSession();

    await expect(login("ada@example.com", "nope")).rejects.toMatchObject({
      code: "invalid_credentials",
    });
    expect(useSession.getState()).toEqual({ status: "anonymous", user: null });
  });

  it("register signs in like login", async () => {
    fakeApi({
      "POST /api/auth/register": () =>
        json(201, { user: ada, csrf_token: "c3" }),
    });
    await register("ada@example.com", "password123", "Ada");
    expect(useSession.getState().status).toBe("authenticated");
  });

  it("logout is optimistic: the tab is out before the server answers", async () => {
    let release: (() => void) | undefined;
    const pending = new Promise<Response>((resolve) => {
      release = () => resolve(json(200, { csrf_token: "c4" }));
    });
    fakeApi({
      "POST /api/auth/login": () => json(200, { user: ada, csrf_token: "c2" }),
      "POST /api/auth/logout": () => pending,
    });
    await login("ada@example.com", "password123");

    const done = logout();
    expect(useSession.getState()).toEqual({ status: "anonymous", user: null });

    release?.();
    await done;
    expect(getCsrfToken()).toBe("c4");
  });

  it("logout swallows a failing request", async () => {
    fakeApi({
      "POST /api/auth/login": () => json(200, { user: ada, csrf_token: "c2" }),
      "POST /api/auth/logout": () => json(500, { error: "internal" }),
    });
    await login("ada@example.com", "password123");
    await expect(logout()).resolves.toBeUndefined();
    expect(useSession.getState().status).toBe("anonymous");
  });

  it("profile update is optimistic and confirms with the server row", async () => {
    let resolveMe: ((r: Response) => void) | undefined;
    fakeApi({
      "POST /api/auth/login": () => json(200, { user: ada, csrf_token: "c2" }),
      "PATCH /api/me": () =>
        new Promise<Response>((resolve) => {
          resolveMe = resolve;
        }),
    });
    await login("ada@example.com", "password123");

    const done = updateProfile({
      name: " Ada Lovelace ",
      avatar_url: "https://cdn.example/a.png",
    });
    expect(useSession.getState().user).toMatchObject({
      name: "Ada Lovelace",
      avatar_url: "https://cdn.example/a.png",
    });

    const fromServer = { ...ada, name: "Ada Lovelace", avatar_url: null };
    resolveMe?.(json(200, fromServer));
    await done;
    expect(useSession.getState().user).toEqual(fromServer);
  });

  it("a 401 from any request flips the tab to anonymous", async () => {
    fakeApi({
      "POST /api/auth/login": () => json(200, { user: ada, csrf_token: "c2" }),
      "PATCH /api/me": () =>
        json(401, { error: "unauthenticated", message: "Sign in required." }),
    });
    await login("ada@example.com", "password123");
    expect(useSession.getState().status).toBe("authenticated");

    await expect(updateProfile({ name: "X" })).rejects.toMatchObject({
      code: "unauthenticated",
    });
    expect(useSession.getState()).toEqual({ status: "anonymous", user: null });
  });

  it("a csrf re-bootstrap without a user flips the tab to anonymous", async () => {
    let attempt = 0;
    fakeApi({
      "POST /api/auth/login": () => json(200, { user: ada, csrf_token: "c2" }),
      "GET /api/auth/session": () =>
        json(200, { user: null, csrf_token: "c9" }),
      "PATCH /api/me": () => {
        attempt += 1;
        return attempt === 1
          ? json(403, { error: "csrf_invalid", message: "stale" })
          : json(401, { error: "unauthenticated", message: "gone" });
      },
    });
    await login("ada@example.com", "password123");

    await expect(updateProfile({ name: "X" })).rejects.toMatchObject({
      code: "unauthenticated",
    });
    expect(useSession.getState()).toEqual({ status: "anonymous", user: null });
    expect(getCsrfToken()).toBe("c9");
  });

  it("a csrf re-bootstrap that still has the user keeps the tab signed in", async () => {
    let attempt = 0;
    const renamed = { ...ada, name: "Ada (other tab)" };
    fakeApi({
      "POST /api/auth/login": () => json(200, { user: ada, csrf_token: "c2" }),
      "GET /api/auth/session": () =>
        json(200, { user: renamed, csrf_token: "c9" }),
      "PATCH /api/me": () => {
        attempt += 1;
        return attempt === 1
          ? json(403, { error: "csrf_invalid", message: "stale" })
          : json(200, { ...renamed, name: "X" });
      },
    });
    await login("ada@example.com", "password123");

    await updateProfile({ name: "X" });
    expect(useSession.getState()).toEqual({
      status: "authenticated",
      user: { ...renamed, name: "X" },
    });
  });

  it("profile update rolls back on error", async () => {
    fakeApi({
      "POST /api/auth/login": () => json(200, { user: ada, csrf_token: "c2" }),
      "PATCH /api/me": () =>
        json(422, {
          error: "validation_failed",
          message: "x",
          fields: { avatar_url: "invalid" },
        }),
    });
    await login("ada@example.com", "password123");

    const error = await updateProfile({ avatar_url: "javascript:x" }).catch(
      (e: unknown) => e,
    );
    expect(error).toBeInstanceOf(ApiError);
    expect((error as ApiError).fields).toEqual({ avatar_url: "invalid" });
    expect(useSession.getState().user).toEqual(ada);
  });
});
