// Who is signed in, as far as this tab knows. The store is the single
// source of truth for the UI; the API calls below update it *before* the
// router moves, so login/logout feel instant and never trigger a reload.

import { create } from "zustand";

import {
  api,
  setCsrfToken,
  setSessionScope,
  setSessionSink,
  setSessionReconciler,
  synchronizeSharedSession,
} from "../api/client.ts";
import { releaseUserScope } from "./release.ts";
import {
  invalidateSessionRequests,
  sessionStampHolds,
  stampHolds,
  takeSessionStamp,
  takeStamp,
} from "./scope.ts";
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

function applySession(user: User | null, force = false): void {
  const previousId = useSession.getState().user?.id ?? null;
  const nextId = user?.id ?? null;
  useSession.setState({
    status: user ? "authenticated" : "anonymous",
    user,
  });
  // Same account (profile rename, CSRF refresh) keeps its cache. A different
  // id, or nobody, drops the previous account before the next paint.
  if (force || previousId !== nextId) {
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
function receiveSession(
  value: unknown,
  options?: { force?: boolean; shared?: boolean },
): void {
  const user = isUser(value) ? value : null;
  // Foreign cookies must not replace an explicitly requested login/register.
  if (options?.shared && user && activeAuthIntent !== null) return;
  applySession(user, options?.force);
}
setSessionSink(receiveSession);
setSessionScope(takeSessionStamp);

let bootstrap: Promise<void> | null = null;
let authIntent = 0;
let activeAuthIntent: number | null = null;
setSessionReconciler(async () => {
  if (activeAuthIntent !== null) return;
  await api<SessionResponse>("/auth/session", { reconcile: true });
});

/**
 * Resolves once the server has told us who we are (and handed out the CSRF
 * token). Concurrent callers share one request; after a failure the next
 * caller retries.
 */
export function ensureSession(): Promise<void> {
  if (useSession.getState().status !== "unknown") {
    return Promise.resolve();
  }
  if (bootstrap) return bootstrap;
  const stamp = takeSessionStamp();
  const request = api<SessionResponse>("/auth/session")
    .then(() => {
      if (
        useSession.getState().status === "unknown" &&
        sessionStampHolds(stamp)
      )
        applySession(null);
    })
    .catch(() => {
      // Offline or API down: treat as anonymous so the login page can render
      // and show the real error inline on submit.
      if (sessionStampHolds(stamp)) applySession(null);
    })
    .finally(() => {
      if (bootstrap === request) bootstrap = null;
    });
  bootstrap = request;
  return bootstrap;
}

function startAuthIntent(): number {
  invalidateSessionRequests();
  bootstrap = null;
  activeAuthIntent = ++authIntent;
  return authIntent;
}

function finishAuthIntent(intent: number): void {
  if (activeAuthIntent !== intent) return;
  activeAuthIntent = null;
  synchronizeSharedSession();
}

async function signIn(path: string, body: unknown): Promise<User> {
  const intent = startAuthIntent();
  try {
    const session = await api<SessionResponse>(path, {
      method: "POST",
      body,
      authIntent: () => authIntent === intent,
    });
    if (authIntent !== intent)
      throw new DOMException("The session changed. Try again.", "AbortError");
    return session.user as User;
  } finally {
    finishAuthIntent(intent);
  }
}

export function login(email: string, password: string): Promise<User> {
  return signIn("/auth/login", { email, password });
}

export function register(
  email: string,
  password: string,
  name: string,
): Promise<User> {
  return signIn("/auth/register", { email, password, name });
}

/** Optimistically release the current account, then revoke its cookie. */
export async function logout(): Promise<void> {
  const logoutUserId = useSession.getState().user?.id ?? null;
  const intent = startAuthIntent();
  applySession(null);
  try {
    await api<LogoutResponse>("/auth/logout", {
      method: "POST",
      logoutUserId,
      authIntent: () => authIntent === intent,
    });
  } catch {
    // Already signed out as far as this tab is concerned.
  } finally {
    finishAuthIntent(intent);
  }
}

/**
 * Optimistic: the new name/avatar show up at once; on error the previous
 * user is restored and the error goes back to the form.
 *
 * The user id and generation are captured before the request. Success and
 * rollback both no-op once that stamp is stale, so a slow PATCH from the
 * previous account cannot overwrite whoever is signed in now. A response
 * that actually changes the user id still goes through `applySession`.
 */
export async function updateProfile(patch: ProfilePatch): Promise<User> {
  const previous = useSession.getState().user;
  const stamp = takeStamp();
  if (previous && stamp) {
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
    if (stampHolds(stamp)) {
      if (user.id === stamp.userId) {
        useSession.setState({ status: "authenticated", user });
      } else {
        applySession(user);
      }
    }
    return user;
  } catch (error) {
    // Roll back the optimistic write only while this attempt's account is
    // still the one in the store. A 401 or a later login must win.
    if (previous && stampHolds(stamp)) {
      useSession.setState({ user: previous });
    }
    throw error;
  }
}

/** Test/dev helper: forget everything this tab knows. */
export function resetSessionForTests(): void {
  bootstrap = null;
  authIntent += 1;
  activeAuthIntent = null;
  setCsrfToken(null);
  setSessionSink(receiveSession);
  setSessionScope(takeSessionStamp);
  useSession.setState({ status: "unknown", user: null });
  releaseUserScope(null);
}
