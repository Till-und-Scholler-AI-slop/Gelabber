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
  type SessionSinkOptions,
} from "../api/client.ts";
import { releaseUserScope } from "./release.ts";
import {
  invalidateSessionRequests,
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

/** Request failures are not proof that the server ended the session. */
export const useSessionProblems = create<{
  bootstrap: string | null;
  logout: string | null;
}>(() => ({ bootstrap: null, logout: null }));

let failedLogoutUserId: string | null = null;

function applySession(user: User | null, force = false): void {
  const previousId = useSession.getState().user?.id ?? null;
  const nextId = user?.id ?? null;
  useSessionProblems.setState({ bootstrap: null });
  if (user && failedLogoutUserId !== null && user.id !== failedLogoutUserId) {
    failedLogoutUserId = null;
    useSessionProblems.setState({ logout: null });
  }
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
function receiveSession(value: unknown, options?: SessionSinkOptions): void {
  if (options?.invalidate) {
    const previous = useSession.getState();
    // A newly loaded document already awaits its bootstrap. Initial pageshow
    // must not turn "not yet known" into a confirmed anonymous session or
    // invalidate the request the route guard is waiting for.
    if (previous.status === "unknown") return;
    if (activeAuthIntent === null || previous.user !== null) {
      useSession.setState({ status: "unknown", user: null });
    }
    if (options.force || previous.user !== null) releaseUserScope(null);
    return;
  }
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
  await ensureSession();
});

/**
 * Resolves once the server has told us who we are (and handed out the CSRF
 * token). Concurrent callers share one request; after a failure the next
 * caller retries.
 */
export function ensureSession(): Promise<void> {
  // Guards must await a pending reconciliation even if a callback has just
  // adopted its result and the shared bootstrap has not finished yet.
  if (bootstrap) return bootstrap;
  if (useSession.getState().status !== "unknown") {
    return Promise.resolve();
  }
  const stamp = takeSessionStamp();
  useSessionProblems.setState({ bootstrap: null });
  const canFallback = () => {
    const current = takeSessionStamp();
    // Initial pageshow may adopt shared metadata while this bootstrap loads.
    // Local intent/identity changes still supersede its anonymous fallback.
    return (
      bootstrap === request &&
      current.userId === stamp.userId &&
      current.generation === stamp.generation
    );
  };
  const request = api<SessionResponse>("/auth/session")
    .then(() => {
      if (useSession.getState().status === "unknown" && canFallback())
        applySession(null);
    })
    .catch(() => {
      if (canFallback()) {
        // Keep the identity unresolved: no authenticated content and no false
        // logout redirect while the API or network is unavailable.
        useSessionProblems.setState({
          bootstrap:
            "Deine Sitzung konnte nicht geprüft werden. Prüfe deine Verbindung und versuche es erneut.",
        });
      }
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

async function signIn(path: string, body: unknown): Promise<User | null> {
  const intent = startAuthIntent();
  useSessionProblems.setState({ bootstrap: null });
  try {
    const session = await api<SessionResponse>(path, {
      method: "POST",
      body,
      authIntent: () => authIntent === intent,
    });
    if (authIntent !== intent)
      throw new DOMException("The session changed. Try again.", "AbortError");
    failedLogoutUserId = null;
    useSessionProblems.setState({ logout: null });
    return session.user;
  } finally {
    finishAuthIntent(intent);
  }
}

export async function login(email: string, password: string): Promise<User> {
  return (await signIn("/auth/login", { email, password })) as User;
}

/**
 * Signs the new account in, or resolves to `null` when the server mails a
 * confirmation link first (account mail on); the tab stays signed out then.
 */
export function register(
  email: string,
  password: string,
  name: string,
): Promise<User | null> {
  return signIn("/auth/register", { email, password, name });
}

/**
 * Opens a mailed confirmation link with the account's password; this tab
 * is signed in afterwards.
 */
export async function verifyEmail(
  token: string,
  password: string,
): Promise<User> {
  return (await signIn("/auth/verify-email", { token, password })) as User;
}

/**
 * Sets a new password through a mailed reset link. Every session of the
 * account ends; this tab is signed in with a fresh one.
 */
export async function resetPassword(
  token: string,
  password: string,
): Promise<User> {
  return (await signIn("/auth/password-reset/confirm", {
    token,
    password,
  })) as User;
}

/** Asks for a reset link. The answer is the same for unknown addresses. */
export async function requestPasswordReset(email: string): Promise<void> {
  await api<Record<string, never>>("/auth/password-reset", {
    method: "POST",
    body: { email },
  });
}

/** Optimistically release the current account, then revoke its cookie. */
export function logout(): Promise<void> {
  return endSession("/auth/logout");
}

/** Like `logout`, and every other session of the account ends too. */
export function logoutEverywhere(): Promise<void> {
  return endSession("/auth/logout-all");
}

async function endSession(
  path: "/auth/logout" | "/auth/logout-all",
): Promise<void> {
  const logoutUserId = useSession.getState().user?.id ?? failedLogoutUserId;
  const intent = startAuthIntent();
  useSessionProblems.setState({ logout: null });
  applySession(null);
  try {
    await api<LogoutResponse>(path, {
      method: "POST",
      logoutUserId,
      authIntent: () => authIntent === intent,
    });
    if (authIntent === intent) failedLogoutUserId = null;
  } catch {
    if (authIntent === intent) {
      failedLogoutUserId = logoutUserId;
      useSessionProblems.setState({
        logout:
          "Die Abmeldung auf dem Server konnte nicht bestätigt werden. Bitte versuche es erneut.",
      });
    }
  } finally {
    finishAuthIntent(intent);
  }
}

/**
 * Sets a new password. The server ends every session of the account and
 * answers with a fresh one for this browser; the account and its caches stay.
 */
export async function changePassword(
  currentPassword: string,
  password: string,
): Promise<User> {
  const session = await api<SessionResponse>("/auth/password", {
    method: "POST",
    body: { current_password: currentPassword, password },
  });
  return session.user as User;
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
/**
 * Profile writes in flight, for every profile form of this tab. The count
 * lives here, not in the form: a write keeps running after its form
 * unmounts, and a form mounted again meanwhile must not start a second one,
 * whose late or failed answer would put an older user back into the store.
 */
export const useProfileWrites = create<{ pending: number }>(() => ({
  pending: 0,
}));

/**
 * Claims the single profile write slot synchronously, before the write
 * starts. Returns its (idempotent) release, or `null` while another
 * profile write is in flight.
 */
export function claimProfileWrite(): (() => void) | null {
  if (useProfileWrites.getState().pending > 0) return null;
  useProfileWrites.setState((state) => ({ pending: state.pending + 1 }));
  let released = false;
  return () => {
    if (released) return;
    released = true;
    useProfileWrites.setState((state) => ({
      pending: Math.max(0, state.pending - 1),
    }));
  };
}

export async function updateProfile(patch: ProfilePatch): Promise<User> {
  const previous = useSession.getState().user;
  const stamp = takeStamp();
  if (previous && stamp) {
    const optional: Partial<User> = {};
    for (const [field, value] of Object.entries(patch)) {
      if (field !== "name" && typeof value === "string") {
        Object.assign(optional, { [field]: value.trim() || null });
      }
    }
    useSession.setState({
      user: {
        ...previous,
        ...optional,
        ...(patch.name !== undefined ? { name: patch.name.trim() } : {}),
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

/**
 * Uploads an image prepared by `prepareAvatar`; the profile then points at
 * it (#208). Not optimistic: the new URL only exists once the server
 * stored the file.
 */
export async function uploadAvatar(image: Blob): Promise<User> {
  return uploadProfileImage("/me/avatar", image);
}

/** Uploads a banner prepared by `prepareBanner`; same rules as the avatar. */
export async function uploadBanner(image: Blob): Promise<User> {
  return uploadProfileImage("/me/banner", image);
}

async function uploadProfileImage(path: string, image: Blob): Promise<User> {
  const stamp = takeStamp();
  const user = await api<User>(path, { method: "PUT", body: image });
  if (stampHolds(stamp)) {
    if (user.id === stamp.userId) {
      useSession.setState({ status: "authenticated", user });
    } else {
      applySession(user);
    }
  }
  return user;
}

/** Test/dev helper: forget everything this tab knows. */
export function resetSessionForTests(): void {
  bootstrap = null;
  authIntent += 1;
  activeAuthIntent = null;
  failedLogoutUserId = null;
  useSessionProblems.setState({ bootstrap: null, logout: null });
  setCsrfToken(null);
  setSessionSink(receiveSession);
  setSessionScope(takeSessionStamp);
  useSession.setState({ status: "unknown", user: null });
  releaseUserScope(null);
}
