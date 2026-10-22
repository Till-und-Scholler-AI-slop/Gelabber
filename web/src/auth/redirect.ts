// Where to go after signing in. A target only ever is a path of this app,
// and one that has to survive a detour (register → mail → confirmation link
// in a new tab) is parked in localStorage for a day.

/** Only same-app paths may be used as post-login targets. */
export function safeRedirect(value: unknown): string | undefined {
  return typeof value === "string" &&
    value.startsWith("/") &&
    !value.startsWith("//") &&
    // Browsers read a backslash like a slash: `/\host` leaves the app too.
    !value.startsWith("/\\")
    ? value
    : undefined;
}

const PENDING_KEY = "gelabber.pending-redirect";
export const PENDING_REDIRECT_MS = 24 * 60 * 60 * 1000;

/**
 * Keeps `target` for the tab that opens the confirmation link. The link is
 * valid for 24 hours, and so is the target.
 */
export function savePendingRedirect(target: string, now = Date.now()): void {
  const safe = safeRedirect(target);
  if (!safe) return;
  try {
    localStorage.setItem(
      PENDING_KEY,
      JSON.stringify({ target: safe, expires: now + PENDING_REDIRECT_MS }),
    );
  } catch {
    // Storage blocked: the confirmation lands on "/" as before.
  }
}

/** The parked target, once: reading it removes it. */
export function takePendingRedirect(now = Date.now()): string | undefined {
  let raw: string | null;
  try {
    raw = localStorage.getItem(PENDING_KEY);
    localStorage.removeItem(PENDING_KEY);
  } catch {
    return undefined;
  }
  if (!raw) return undefined;
  try {
    const value: unknown = JSON.parse(raw);
    if (
      typeof value !== "object" ||
      value === null ||
      !("expires" in value) ||
      typeof value.expires !== "number" ||
      value.expires <= now ||
      !("target" in value)
    )
      return undefined;
    return safeRedirect(value.target);
  } catch {
    return undefined;
  }
}

/** The invite a sign-in page was opened for, if any. */
export function isInviteRedirect(target: string | undefined): boolean {
  return target !== undefined && /^\/invite\/[^/?#]+/.test(target);
}
