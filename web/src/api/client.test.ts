import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { ApiError, api, getCsrfToken, setCsrfToken } from "./client.ts";

type Call = { url: string; init: RequestInit };

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

function install(handler: (call: Call, index: number) => Response) {
  const calls: Call[] = [];
  const fetchMock = vi.fn((input: RequestInfo | URL, init?: RequestInit) => {
    const call = { url: String(input), init: init ?? {} };
    calls.push(call);
    return Promise.resolve(handler(call, calls.length - 1));
  });
  vi.stubGlobal("fetch", fetchMock);
  return calls;
}

function header(call: Call, name: string): string | null {
  return new Headers(call.init.headers).get(name);
}

describe("api client", () => {
  beforeEach(() => setCsrfToken(null));
  afterEach(() => vi.unstubAllGlobals());

  it("sends JSON with same-origin credentials under the /api base", async () => {
    const calls = install(() => jsonResponse(200, { ok: true }));

    await expect(api("/auth/session")).resolves.toEqual({ ok: true });

    expect(calls[0]?.url).toBe("/api/auth/session");
    expect(calls[0]?.init.method).toBe("GET");
    expect(calls[0]?.init.credentials).toBe("same-origin");
    expect(header(calls[0]!, "Accept")).toBe("application/json");
  });

  it("remembers the csrf token from any body and sends it on mutations only", async () => {
    const calls = install(() =>
      jsonResponse(200, { user: null, csrf_token: "tok-1" }),
    );

    await api("/auth/session");
    expect(getCsrfToken()).toBe("tok-1");
    expect(header(calls[0]!, "X-CSRF-Token")).toBeNull();

    await api("/auth/login", { method: "POST", body: { email: "a@b.co" } });
    expect(header(calls[1]!, "X-CSRF-Token")).toBe("tok-1");
    expect(header(calls[1]!, "Content-Type")).toBe("application/json");
    expect(calls[1]?.init.body).toBe(JSON.stringify({ email: "a@b.co" }));
  });

  it("surfaces the error envelope as ApiError with fields", async () => {
    install(() =>
      jsonResponse(422, {
        error: "validation_failed",
        message: "Some fields are invalid.",
        fields: { email: "invalid", password: "too_short" },
      }),
    );

    const error = await api("/auth/register", {
      method: "POST",
      body: {},
    }).catch((e: unknown) => e);
    expect(error).toBeInstanceOf(ApiError);
    const apiError = error as ApiError;
    expect(apiError.code).toBe("validation_failed");
    expect(apiError.status).toBe(422);
    expect(apiError.fields).toEqual({
      email: "invalid",
      password: "too_short",
    });
  });

  it("maps unknown codes and empty bodies to internal", async () => {
    install(() => new Response("", { status: 502 }));
    const error = (await api("/me").catch((e: unknown) => e)) as ApiError;
    expect(error.code).toBe("internal");
    expect(error.status).toBe(502);
  });

  it("re-bootstraps once on csrf_invalid and retries the mutation", async () => {
    setCsrfToken("stale");
    const calls = install((call, index) => {
      if (index === 0) {
        return jsonResponse(403, { error: "csrf_invalid", message: "nope" });
      }
      if (call.url.endsWith("/auth/session")) {
        return jsonResponse(200, { user: null, csrf_token: "fresh" });
      }
      return jsonResponse(200, { user: { id: "u1" }, csrf_token: "rotated" });
    });

    const result = await api<{ csrf_token: string }>("/auth/login", {
      method: "POST",
      body: {},
    });

    expect(calls.map((c) => c.url)).toEqual([
      "/api/auth/login",
      "/api/auth/session",
      "/api/auth/login",
    ]);
    expect(header(calls[0]!, "X-CSRF-Token")).toBe("stale");
    expect(header(calls[2]!, "X-CSRF-Token")).toBe("fresh");
    expect(result.csrf_token).toBe("rotated");
    expect(getCsrfToken()).toBe("rotated");
  });

  it("gives up after the single retry", async () => {
    setCsrfToken("stale");
    const calls = install((call) =>
      call.url.endsWith("/auth/session")
        ? jsonResponse(200, { user: null, csrf_token: "fresh" })
        : jsonResponse(403, { error: "csrf_invalid", message: "nope" }),
    );

    const error = (await api("/me", { method: "PATCH", body: {} }).catch(
      (e: unknown) => e,
    )) as ApiError;
    expect(error.code).toBe("csrf_invalid");
    expect(calls).toHaveLength(3);
  });

  it("does not retry a 403 that is not about csrf", async () => {
    const calls = install(() =>
      jsonResponse(403, { error: "forbidden_other", message: "no" }),
    );
    await expect(
      api("/me", { method: "PATCH", body: {} }),
    ).rejects.toBeInstanceOf(ApiError);
    expect(calls).toHaveLength(1);
  });

  it("turns a fetch failure into a network ApiError", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(() => Promise.reject(new TypeError("Failed to fetch"))),
    );
    const error = (await api("/me").catch((e: unknown) => e)) as ApiError;
    expect(error).toBeInstanceOf(ApiError);
    expect(error.code).toBe("network");
  });

  it("surfaces a 429 rate_limited envelope without hanging", async () => {
    install(() =>
      jsonResponse(429, {
        error: "rate_limited",
        message: "Too many requests. Try again in a moment.",
        retry_after: 8,
      }),
    );
    const error = (await api("/auth/login", {
      method: "POST",
      body: {},
    }).catch((e: unknown) => e)) as ApiError;
    expect(error.code).toBe("rate_limited");
    expect(error.status).toBe(429);
    expect(error.retryAfter).toBe(8);
  });

  it("turns a timeout into a timeout ApiError", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(() =>
        Promise.reject(
          new DOMException("The operation timed out.", "TimeoutError"),
        ),
      ),
    );
    const error = (await api("/me").catch((e: unknown) => e)) as ApiError;
    expect(error.code).toBe("timeout");
  });
});
