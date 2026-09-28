// Thin fetch wrapper for the Axum API: same-origin cookies, JSON in/out,
// `X-CSRF-Token` on every mutation, a hard timeout so no button ever spins
// forever, and the API's error envelope surfaced as `ApiError`.

import type { SessionStamp } from "../auth/scope.ts";

export type FieldCode =
  "required" | "invalid" | "too_short" | "too_long" | "taken";

export type FieldErrors = Partial<Record<string, FieldCode>>;

export type ApiErrorCode =
  | "validation_failed"
  | "bad_request"
  | "unauthenticated"
  | "invalid_credentials"
  | "csrf_invalid"
  | "forbidden"
  | "not_found"
  | "email_taken"
  | "invite_invalid"
  | "banned"
  | "rate_limited"
  | "quota_exceeded"
  | "internal"
  | "network"
  | "timeout";

export class ApiError extends Error {
  readonly code: ApiErrorCode;
  readonly status: number;
  readonly fields: FieldErrors;
  readonly retryAfter: number | null;

  constructor(
    code: ApiErrorCode,
    status: number,
    message: string,
    fields: FieldErrors = {},
    retryAfter: number | null = null,
  ) {
    super(message);
    this.name = "ApiError";
    this.code = code;
    this.status = status;
    this.fields = fields;
    this.retryAfter = retryAfter;
  }
}

export const REQUEST_TIMEOUT_MS = 10_000;

const configuredBase: string | undefined = import.meta.env.VITE_API_BASE_URL;
export const API_BASE = (configuredBase ?? "/api").replace(/\/+$/, "");

let csrfToken: string | null = null;

export function setCsrfToken(token: string | null): void {
  csrfToken = token;
}

/**
 * Where the client reports what the server says about the session outside
 * of an explicit auth call: `null` on `401 unauthenticated` or when the
 * silent CSRF re-bootstrap comes back without a user. The session store
 * registers itself here (keeps this module free of a store import).
 */
export type SessionSink = (user: unknown) => void;

let sessionSink: SessionSink | null = null;
let sessionScope: () => SessionStamp = () => ({ userId: null, generation: 0 });
let cookieQueue: Promise<void> = Promise.resolve();

/** Set-Cookie is applied by the browser before JS can discard a stale result.
 * Web Locks serialize cookie-changing auth requests across same-origin tabs;
 * the local queue also covers non-browser callers without a LockManager. */
function withSessionCookieLock<T>(
  action: () => Promise<T>,
  signal?: AbortSignal,
): Promise<T> {
  if (typeof navigator !== "undefined" && navigator.locks) {
    return navigator.locks.request("gelabber:auth-cookies", { signal }, action);
  }
  const request = cookieQueue.then(action);
  cookieQueue = request.then(
    () => undefined,
    () => undefined,
  );
  return request;
}

function changesAuthCookies(path: string): boolean {
  return (
    path === "/auth/session" ||
    path === "/auth/login" ||
    path === "/auth/register" ||
    path === "/auth/logout"
  );
}

export function setSessionSink(sink: SessionSink | null): void {
  sessionSink = sink;
}

/** The store supplies identity/generation without a runtime store import. */
export function setSessionScope(source: () => SessionStamp): void {
  sessionScope = source;
}

function scopeHolds(stamp: SessionStamp): boolean {
  const current = sessionScope();
  return (
    current.userId === stamp.userId && current.generation === stamp.generation
  );
}

function requireScope(stamp: SessionStamp, signal?: AbortSignal): void {
  signal?.throwIfAborted();
  if (!scopeHolds(stamp)) {
    throw new DOMException("The session changed. Try again.", "AbortError");
  }
}

export function getCsrfToken(): string | null {
  return csrfToken;
}

type Method = "GET" | "POST" | "PATCH" | "DELETE";

export type RequestOptions = {
  method?: Method;
  body?: unknown;
  signal?: AbortSignal;
  /** Carry the original intent through later steps (e.g. an upload). */
  scope?: SessionStamp;
  /** /auth/logout only: the identity captured before the optimistic logout. */
  logoutUserId?: string | null;
};

type ErrorBody = {
  error?: string;
  message?: string;
  fields?: FieldErrors;
  retry_after?: number;
};

const KNOWN_CODES: ReadonlySet<string> = new Set<ApiErrorCode>([
  "validation_failed",
  "bad_request",
  "unauthenticated",
  "invalid_credentials",
  "csrf_invalid",
  "forbidden",
  "not_found",
  "email_taken",
  "invite_invalid",
  "banned",
  "rate_limited",
  "quota_exceeded",
  "internal",
]);

function toErrorCode(raw: string | undefined): ApiErrorCode {
  return raw !== undefined && KNOWN_CODES.has(raw)
    ? (raw as ApiErrorCode)
    : "internal";
}

/**
 * Performs one request. `path` is relative to `API_BASE` (e.g. `/auth/login`).
 * Responses with `csrf_token` in the body refresh the in-memory token, so a
 * caller never has to thread it through by hand.
 */
export async function api<T>(
  path: string,
  options: RequestOptions = {},
): Promise<T> {
  const stamp = options.scope ?? sessionScope();
  const run = () => performApi<T>(path, options, stamp);
  return changesAuthCookies(path)
    ? withSessionCookieLock(run, options.signal)
    : run();
}

async function performApi<T>(
  path: string,
  options: RequestOptions,
  stamp: SessionStamp,
): Promise<T> {
  requireScope(stamp, options.signal);
  const response = await requestWithCsrfRetry(path, options, stamp);
  const payload = await readJson(response);

  if (!response.ok) {
    const body = (payload ?? {}) as ErrorBody;
    const code = toErrorCode(body.error);
    if (code === "unauthenticated" && scopeHolds(stamp)) {
      // The cookie is gone or expired (other tab logged out, TTL, server
      // restart): tell the store so the UI flips instead of staying stuck
      // on a page that no longer works.
      sessionSink?.(null);
    }
    throw new ApiError(
      code,
      response.status,
      body.message ?? `Request failed with status ${response.status}`,
      body.fields ?? {},
      typeof body.retry_after === "number" ? body.retry_after : null,
    );
  }

  if (scopeHolds(stamp)) rememberCsrf(payload);
  return payload as T;
}

// A stale token (server restarted, cookie expired, other tab logged out)
// yields exactly one silent re-bootstrap and retry; a second failure is
// surfaced to the caller.
async function requestWithCsrfRetry(
  path: string,
  options: RequestOptions,
  stamp: SessionStamp,
): Promise<Response> {
  const first = await send(path, options);
  if (first.status !== 403 || (options.method ?? "GET") === "GET") {
    return first;
  }
  const body = (await peekJson(first)) as ErrorBody | null;
  if (body?.error !== "csrf_invalid") {
    return first;
  }
  const refresh = () => retryWithSession(path, options, stamp, first);
  // Auth already holds this lock. Other mutations keep bootstrap+retry in
  // one critical section so another tab cannot switch cookies between them.
  return changesAuthCookies(path)
    ? refresh()
    : withSessionCookieLock(refresh, options.signal);
}

async function retryWithSession(
  path: string,
  options: RequestOptions,
  stamp: SessionStamp,
  first: Response,
): Promise<Response> {
  requireScope(stamp, options.signal);
  const session = await send("/auth/session", { signal: options.signal });
  const payload = await readJson(session);
  requireScope(stamp, options.signal);
  // A failed or malformed bootstrap cannot authorize another write.
  if (!session.ok || !isSessionPayload(payload)) return first;

  const isLogout = path === "/auth/logout";
  const isSignIn = path === "/auth/login" || path === "/auth/register";
  const expectedId =
    isLogout && options.logoutUserId !== undefined
      ? options.logoutUserId
      : stamp.userId;
  const actualId = payload.user?.id ?? null;
  if (!isLogout && !isSignIn) {
    // Synchronize this tab even when another tab changed the cookie. That
    // new identity must never inherit the old mutation's intent.
    rememberCsrf(payload);
    sessionSink?.(payload.user);
  }
  // Credentials express an explicit sign-in intent, independent of whichever
  // account owns the shared cookie. An anonymous logout also revokes a sign-in
  // response that was still in flight when the user chose to log out.
  if (
    !isSignIn &&
    !(isLogout && expectedId === null) &&
    actualId !== expectedId
  ) {
    throw new DOMException("The session changed. Try again.", "AbortError");
  }
  // Logout deliberately keeps the UI anonymous while retrying for the
  // original account, and never adopts the bootstrap user.
  requireScope(stamp, options.signal);
  if (isLogout || isSignIn) rememberCsrf(payload);
  return send(path, options);
}

function isSessionPayload(payload: unknown): payload is {
  user: { id: string; email: string; name: string } | null;
  csrf_token: string;
} {
  if (
    payload === null ||
    typeof payload !== "object" ||
    !("user" in payload) ||
    !("csrf_token" in payload) ||
    typeof payload.csrf_token !== "string" ||
    !payload.csrf_token
  )
    return false;
  const user = payload.user;
  return (
    user === null ||
    (typeof user === "object" &&
      "id" in user &&
      typeof user.id === "string" &&
      user.id.length > 0 &&
      "email" in user &&
      typeof user.email === "string" &&
      "name" in user &&
      typeof user.name === "string")
  );
}

async function send(path: string, options: RequestOptions): Promise<Response> {
  const method = options.method ?? "GET";
  const headers = new Headers({ Accept: "application/json" });
  if (options.body !== undefined) {
    headers.set("Content-Type", "application/json");
  }
  if (method !== "GET" && csrfToken) {
    headers.set("X-CSRF-Token", csrfToken);
  }

  const signal = options.signal
    ? AbortSignal.any([options.signal, AbortSignal.timeout(REQUEST_TIMEOUT_MS)])
    : AbortSignal.timeout(REQUEST_TIMEOUT_MS);

  try {
    return await fetch(`${API_BASE}${path}`, {
      method,
      headers,
      credentials: "same-origin",
      body:
        options.body === undefined ? undefined : JSON.stringify(options.body),
      signal,
    });
  } catch (error) {
    if (error instanceof DOMException && error.name === "TimeoutError") {
      throw new ApiError("timeout", 0, "Request timed out.");
    }
    if (error instanceof DOMException && error.name === "AbortError") {
      throw error;
    }
    throw new ApiError("network", 0, "Network error.");
  }
}

function rememberCsrf(payload: unknown): void {
  if (
    payload !== null &&
    typeof payload === "object" &&
    "csrf_token" in payload &&
    typeof payload.csrf_token === "string"
  ) {
    csrfToken = payload.csrf_token;
  }
}

async function readJson(response: Response): Promise<unknown> {
  const text = await response.text();
  if (text.length === 0) {
    return null;
  }
  try {
    return JSON.parse(text) as unknown;
  } catch {
    return null;
  }
}

async function peekJson(response: Response): Promise<unknown> {
  return readJson(response.clone());
}
