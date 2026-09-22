// Who is signed in, as far as this tab knows. The store is the single
// source of truth for the UI; the API calls below update it *before* the
// router moves, so login/logout feel instant and never trigger a reload.

import { create } from "zustand";

import { api, setCsrfToken, setSessionSink } from "../api/client.ts";
import { releaseUserScope } from "./release.ts";
import type {
  LogoutResponse,
  ProfilePatch,
  SessionResponse,
  User,
} from "./types.ts";

export type SessionStatus = "unknown" | "anonymous" | "authenticated";

export type SessionState = {
  status: SessionStatus;
  user: User | null;
};

export const useSession = create<SessionState>(() => ({
  status: "unknown",
  user: null,
}));

function applySession(user: User | null): void {
  const previousId = useSession.getState().user?.id ?? null;
  const nextId = user?.id ?? null;
  useSession.setState({
    status: user ? "authenticated" : "anonymous",
    user,
  });
  // Same account (profile rename, CSRF refresh) keeps its cache. A different
  // id, or nobody, drops the previous account before the next paint.
  if (previousId !== nextId) {
    releaseUserScope(nextId);
  }
}

function isUser(value: unknown): value is User {
  return (
    value !== null &&
    typeof value === "object" &&
    "id" in value &&
    typeof value.id === "string" &&
    "email" in value &&
    typeof value.email === "string" &&
    "name" in value &&
    typeof value.name === "string"
  );
}

// Anything the API client learns about the session on the side (a 401, a
// re-bootstrap) lands here, so a session that died in another tab flips
// this tab to anonymous as instantly as an explicit logout would.
setSessionSink((value) => applySession(isUser(value) ? value : null));

let bootstrap: Promise<void> | null = null;

/**
 * Resolves once the server has told us who we are (and handed out the CSRF
 * token). Concurrent callers share one request; after a failure the next
 * caller retries.
 */
export function ensureSession(): Promise<void> {
  if (useSession.getState().status !== "unknown") {
    return Promise.resolve();
  }
  bootstrap ??= api<SessionResponse>("/auth/session")
    .then((session) => applySession(session.user))
    .catch(() => {
      // Offline or API down: treat as anonymous so the login page can render
      // and show the real error inline on submit.
      applySession(null);
    })
    .finally(() => {
      bootstrap = null;
    });
  return bootstrap;
}

export async function login(email: string, password: string): Promise<User> {
  const session = await api<SessionResponse>("/auth/login", {
    method: "POST",
    body: { email, password },
  });
  applySession(session.user);
  return session.user as User;
}

export async function register(
  email: string,
  password: string,
  name: string,
): Promise<User> {
  const session = await api<SessionResponse>("/auth/register", {
    method: "POST",
    body: { email, password, name },
  });
  applySession(session.user);
  return session.user as User;
}

/**
 * Drops the session locally first — the UI is "out" immediately — then
 * tells the server. A failed request leaves a dangling cookie that the next
 * bootstrap simply picks up again, so there is nothing to roll back.
 */
export async function logout(): Promise<void> {
  applySession(null);
  try {
    const response = await api<LogoutResponse>("/auth/logout", {
      method: "POST",
    });
    setCsrfToken(response.csrf_token);
  } catch {
    // Already signed out as far as this tab is concerned.
  }
}

/**
 * Optimistic: the new name/avatar show up at once; on error the previous
 * user is restored and the error goes back to the form.
 */
export async function updateProfile(patch: ProfilePatch): Promise<User> {
  const previous = useSession.getState().user;
  if (previous) {
    useSession.setState({
      user: {
        ...previous,
        ...(patch.name !== undefined ? { name: patch.name.trim() } : {}),
        ...(patch.avatar_url !== undefined
          ? { avatar_url: patch.avatar_url.trim() || null }
          : {}),
      },
    });
  }
  try {
    const user = await api<User>("/me", { method: "PATCH", body: patch });
    useSession.setState({ status: "authenticated", user });
    return user;
  } catch (error) {
    // Roll back the optimistic write — unless the failure was the session
    // itself ending, in which case the store is already anonymous and must
    // stay that way.
    if (previous && useSession.getState().status === "authenticated") {
      useSession.setState({ user: previous });
    }
    throw error;
  }
}

/** Test/dev helper: forget everything this tab knows. */
export function resetSessionForTests(): void {
  bootstrap = null;
  setCsrfToken(null);
  setSessionSink((value) => applySession(isUser(value) ? value : null));
  useSession.setState({ status: "unknown", user: null });
  releaseUserScope(null);
}
