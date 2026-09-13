// Thin fetch wrapper for the Axum API: same-origin cookies, JSON in/out,
// `X-CSRF-Token` on every mutation, a hard timeout so no button ever spins
// forever, and the API's error envelope surfaced as `ApiError`.

export type FieldCode =
  "required" | "invalid" | "too_short" | "too_long" | "taken";

export type FieldErrors = Partial<Record<string, FieldCode>>;

export type ApiErrorCode =
  | "validation_failed"
  | "bad_request"
  | "unauthenticated"
  | "invalid_credentials"
  | "csrf_invalid"
  | "email_taken"
  | "internal"
  | "network"
  | "timeout";

export class ApiError extends Error {
  readonly code: ApiErrorCode;
  readonly status: number;
  readonly fields: FieldErrors;

  constructor(
    code: ApiErrorCode,
    status: number,
    message: string,
    fields: FieldErrors = {},
  ) {
    super(message);
    this.name = "ApiError";
    this.code = code;
    this.status = status;
    this.fields = fields;
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

export function setSessionSink(sink: SessionSink | null): void {
  sessionSink = sink;
}

export function getCsrfToken(): string | null {
  return csrfToken;
}

type Method = "GET" | "POST" | "PATCH" | "DELETE";

export type RequestOptions = {
  method?: Method;
  body?: unknown;
  signal?: AbortSignal;
};

type ErrorBody = {
  error?: string;
  message?: string;
  fields?: FieldErrors;
};

const KNOWN_CODES: ReadonlySet<string> = new Set<ApiErrorCode>([
  "validation_failed",
  "bad_request",
  "unauthenticated",
  "invalid_credentials",
  "csrf_invalid",
  "email_taken",
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
  const response = await requestWithCsrfRetry(path, options);
  const payload = await readJson(response);

  if (!response.ok) {
    const body = (payload ?? {}) as ErrorBody;
    const code = toErrorCode(body.error);
    if (code === "unauthenticated") {
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
    );
  }

  rememberCsrf(payload);
  return payload as T;
}

// A stale token (server restarted, cookie expired, other tab logged out)
// yields exactly one silent re-bootstrap and retry; a second failure is
// surfaced to the caller.
async function requestWithCsrfRetry(
  path: string,
  options: RequestOptions,
): Promise<Response> {
  const first = await send(path, options);
  if (first.status !== 403 || (options.method ?? "GET") === "GET") {
    return first;
  }
  const body = (await peekJson(first)) as ErrorBody | null;
  if (body?.error !== "csrf_invalid") {
    return first;
  }
  const session = await send("/auth/session", { signal: options.signal });
  const payload = await readJson(session);
  rememberCsrf(payload);
  if (session.ok && payload !== null && typeof payload === "object") {
    // The bootstrap is authoritative about who we are, not just the token.
    sessionSink?.("user" in payload ? payload.user : null);
  }
  return send(path, options);
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
