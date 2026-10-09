// Thin fetch wrapper for the Axum API: same-origin cookies, JSON in/out,
// `X-CSRF-Token` on every mutation, a hard timeout so no button ever spins
// forever, and the API's error envelope surfaced as `ApiError`.

import type { SessionStamp } from "../auth/scope.ts";
import { randomUuid } from "../lib/uuid.ts";

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
  | "theme_conflict"
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
export type SessionSinkOptions = {
  force?: boolean;
  shared?: boolean;
  /** Shared cookie state changed; its authoritative identity is still loading. */
  invalidate?: boolean;
};
export type SessionSink = (user: unknown, options?: SessionSinkOptions) => void;

let sessionSink: SessionSink | null = null;
let sessionScope: () => SessionStamp = () => ({
  userId: null,
  generation: 0,
  sharedGeneration: sessionCookieGeneration(),
});

// No profile, token, or cookie is persisted here. Storage is the synchronous
// fence; notifications merely prompt earlier cleanup of caches and media.
const COOKIE_STATE_KEY = "gelabber:auth-generation";
type CookieState = {
  generation: string;
  phase: "changing" | "settled" | "unknown";
  userId: string | null;
};
let observedCookieState = "";
let adoptedCookieGeneration = "";
let reconcileSession: (() => Promise<void>) | null = null;
let reconciliation: Promise<void> | null = null;
let listening = false;

function readCookieState(): CookieState | null {
  if (typeof window === "undefined") return null;
  const raw = window.localStorage.getItem(COOKIE_STATE_KEY);
  if (raw === null) return null;
  const value: unknown = JSON.parse(raw);
  if (
    value === null ||
    typeof value !== "object" ||
    !("generation" in value) ||
    typeof value.generation !== "string" ||
    !("phase" in value) ||
    !["changing", "settled", "unknown"].includes(String(value.phase)) ||
    !("userId" in value) ||
    (value.userId !== null && typeof value.userId !== "string")
  ) {
    throw new DOMException(
      "Session coordination is unavailable.",
      "AbortError",
    );
  }
  return value as CookieState;
}

export function sessionCookieGeneration(): string {
  return adoptedCookieGeneration;
}

function cookieFingerprint(state: CookieState | null): string {
  return state ? JSON.stringify(state) : "";
}

function publishCookieState(state: CookieState): void {
  if (typeof window === "undefined") return;
  const serialized = JSON.stringify(state);
  // Fail closed when storage is unavailable: never change cookies without
  // first publishing the generation other tabs must check.
  window.localStorage.setItem(COOKIE_STATE_KEY, serialized);
  observedCookieState = serialized;
  adoptedCookieGeneration = state.generation;
}

function scheduleReconciliation(): void {
  if (reconciliation || !reconcileSession) return;
  const request = Promise.resolve()
    .then(() => reconcileSession?.())
    .catch(() => {
      /* Remain anonymous on offline/failed reconciliation. */
    })
    .finally(() => {
      if (reconciliation === request) reconciliation = null;
    });
  reconciliation = request;
}

/** Always read current storage, never trust the (possibly delayed) event value. */
export function synchronizeSharedSession(): void {
  const state = readCookieState();
  if (!state && !observedCookieState) return;
  const fingerprint = cookieFingerprint(state);
  const mismatched =
    state?.phase !== "settled" || state?.userId !== sessionScope().userId;
  if (fingerprint !== observedCookieState) {
    observedCookieState = fingerprint;
    adoptedCookieGeneration = state?.generation ?? "";
    csrfToken = null;
    sessionSink?.(null, { force: true, shared: true, invalidate: true });
    scheduleReconciliation();
  } else if (mismatched) {
    // An intent may have suppressed reconciliation when this generation was
    // first observed; the identity check also catches stamps taken too late.
    sessionSink?.(null, { shared: true, invalidate: true });
    scheduleReconciliation();
  }
}

export function setSessionReconciler(reconcile: () => Promise<void>): void {
  reconcileSession = reconcile;
  if (typeof window === "undefined" || listening) return;
  listening = true;
  window.addEventListener("storage", (event) => {
    if (event.key === COOKIE_STATE_KEY || event.key === null)
      synchronizeSharedSession();
  });
  window.addEventListener("pageshow", () => synchronizeSharedSession());
  window.addEventListener("focus", () => synchronizeSharedSession());
}

let cookieQueue: Promise<void> = Promise.resolve();

/** Set-Cookie is applied by the browser before JS can discard a stale result.
 * Web Locks serialize cookie-changing auth requests across same-origin tabs;
 * the local queue also covers non-browser callers without a LockManager. */
function withSessionCookieLock<T>(
  action: () => Promise<T>,
  signal?: AbortSignal,
): Promise<T> {
  if (typeof window !== "undefined" && !navigator.locks) {
    throw new DOMException(
      "This browser cannot coordinate account changes.",
      "AbortError",
    );
  }
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
  if (!scopeHolds(stamp)) throw staleSession();
}

function staleSession(): DOMException {
  return new DOMException("The session changed. Try again.", "AbortError");
}

function requireSharedScope(stamp: SessionStamp, signal?: AbortSignal): void {
  requireScope(stamp, signal);
  const shared = readCookieState();
  if (
    (shared?.generation ?? "") !== stamp.sharedGeneration ||
    (shared && (shared.phase !== "settled" || shared.userId !== stamp.userId))
  ) {
    synchronizeSharedSession();
    throw staleSession();
  }
}

export function getCsrfToken(): string | null {
  return csrfToken;
}

type Method = "GET" | "POST" | "PUT" | "PATCH" | "DELETE";

export type RequestOptions = {
  method?: Method;
  body?: unknown;
  signal?: AbortSignal;
  /** Carry the original intent through later steps (e.g. an upload). */
  scope?: SessionStamp;
  /** /auth/logout only: the identity captured before the optimistic logout. */
  logoutUserId?: string | null;
  /** Explicit credentials/logout are intents independent of cookie identity. */
  authIntent?: () => boolean;
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
  "theme_conflict",
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
  if (changesAuthCookies(path)) {
    return withSessionCookieLock(
      () => performAuthApi<T>(path, options, stamp),
      options.signal,
    );
  }
  return performApi<T>(path, options, stamp);
}

function requireAuthRequest(
  options: RequestOptions,
  stamp: SessionStamp,
): void {
  options.signal?.throwIfAborted();
  if (options.authIntent) {
    if (!options.authIntent()) throw staleSession();
  } else {
    requireScope(stamp, options.signal);
  }
}

/** The lock covers Set-Cookie, body decoding, the shared fence, and UI adoption. */
async function performAuthApi<T>(
  path: string,
  options: RequestOptions,
  stamp: SessionStamp,
): Promise<T> {
  requireAuthRequest(options, stamp);
  const explicit = path !== "/auth/session";
  const previous = readCookieState();
  let changing: CookieState | null = null;
  if (explicit && typeof window !== "undefined") {
    changing = {
      generation: randomUuid(),
      phase: "changing",
      userId: previous?.userId ?? stamp.userId,
    };
    publishCookieState(changing);
  }
  try {
    const response = await requestWithCsrfRetry(path, options, stamp);
    const payload = await readJson(response);
    // Even a superseded response has already changed the browser's cookies.
    // Publish its actual identity before allowing the next auth intent to send.
    const valid = response.ok && isSessionPayload(payload);
    const logout = response.ok && path === "/auth/logout";
    const user = logout ? null : valid ? payload.user : undefined;
    if (user !== undefined) {
      settleCookieIdentity(user?.id ?? null, changing?.generation);
    } else if (changing) {
      // A failed/timed-out response might still have committed on the server.
      // A subsequent authoritative bootstrap is required, never guess the uid.
      publishCookieState({ ...changing, phase: "unknown", userId: null });
    }
    requireAuthRequest(options, stamp);
    if (!response.ok) throwResponseError(response, payload);
    rememberCsrf(payload);
    if (user !== undefined && !(logout && sessionScope().userId === null)) {
      sessionSink?.(user, {
        force: explicit && !logout,
      });
    }
    return payload as T;
  } finally {
    if (changing && readCookieState()?.phase === "changing") {
      publishCookieState({ ...changing, phase: "unknown", userId: null });
    }
  }
}

function settleCookieIdentity(
  userId: string | null,
  generation?: string,
): void {
  if (typeof window === "undefined") return;
  const previous = readCookieState();
  publishCookieState({
    generation:
      generation ??
      (previous?.phase === "settled" && previous.userId === userId
        ? previous.generation
        : randomUuid()),
    phase: "settled",
    userId,
  });
}

function throwResponseError(response: Response, payload: unknown): never {
  const body = (payload ?? {}) as ErrorBody;
  throw new ApiError(
    toErrorCode(body.error),
    response.status,
    body.message ?? `Request failed with status ${response.status}`,
    body.fields ?? {},
    typeof body.retry_after === "number" ? body.retry_after : null,
  );
}

async function performApi<T>(
  path: string,
  options: RequestOptions,
  stamp: SessionStamp,
): Promise<T> {
  // Synchronous pre-fetch fence catches identity changes before notifications
  // have arrived. Ordinary reads do not hold the auth lock while on the wire.
  requireSharedScope(stamp, options.signal);
  const response = await requestWithCsrfRetry(path, options, stamp);
  const payload = await readJson(response);
  // Recheck under the auth lock: no account switch can occur between checking
  // the shared generation and handing this payload to the caller/cache.
  return withSessionCookieLock(async () => {
    requireSharedScope(stamp, options.signal);
    if (!response.ok) {
      if (
        toErrorCode((payload as ErrorBody | null)?.error) === "unauthenticated"
      ) {
        settleCookieIdentity(null);
        sessionSink?.(null);
      }
      throwResponseError(response, payload);
    }
    rememberCsrf(payload);
    return payload as T;
  }, options.signal);
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
  const auth = changesAuthCookies(path);
  const requireIntent = () =>
    auth
      ? requireAuthRequest(options, stamp)
      : requireSharedScope(stamp, options.signal);
  requireIntent();
  const session = await send("/auth/session", { signal: options.signal });
  const payload = await readJson(session);
  requireIntent();
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
    settleCookieIdentity(actualId);
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
  requireIntent();
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

// Local clock minus server clock, from the latest API response's `Date`.
let clockOffsetMs = 0;

/** Server wall clock (second precision), independent of a skewed local clock. */
export function serverNow(): number {
  return Date.now() - clockOffsetMs;
}

function rememberServerClock(response: Response): void {
  const date = Date.parse(response.headers.get("Date") ?? "");
  if (Number.isFinite(date)) clockOffsetMs = Date.now() - date;
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
    const response = await fetch(`${API_BASE}${path}`, {
      method,
      headers,
      credentials: "same-origin",
      body:
        options.body === undefined ? undefined : JSON.stringify(options.body),
      signal,
    });
    rememberServerClock(response);
    return response;
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
