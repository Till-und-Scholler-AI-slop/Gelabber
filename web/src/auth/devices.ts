// The signed-in devices of an account (#519): `GET /api/auth/sessions` and
// `DELETE /api/auth/sessions/{id}`, and how Sicherheit words a device.

import { useQuery, type QueryClient } from "@tanstack/react-query";

import { ApiError, api } from "../api/client.ts";
import { logOut } from "../components/logout.ts";
import { translate } from "../i18n/catalog.ts";
import { account } from "../i18n/catalogs/account.ts";

export type DeviceClient =
  "desktop" | "firefox" | "chrome" | "edge" | "safari" | "browser" | "unknown";

export type DeviceOs =
  "linux" | "windows" | "macos" | "android" | "ios" | "other";

/** One live session of the account, as the API lists it. */
export type DeviceSession = {
  id: string;
  client: DeviceClient;
  os: DeviceOs;
  created_at: string;
  last_used_at: string;
  current: boolean;
};

/** Live sessions: the current one first, then by last use. */
export async function listSessions(
  signal?: AbortSignal,
): Promise<DeviceSession[]> {
  const body = await api<{ sessions: DeviceSession[] }>(
    "/auth/sessions",
    signal ? { signal } : {},
  );
  return body.sessions;
}

/** Signs another device of the account out (204). */
export async function revokeSession(id: string): Promise<void> {
  await api<null>(`/auth/sessions/${encodeURIComponent(id)}`, {
    method: "DELETE",
  });
}

export function sessionsKey(userId: string) {
  return ["auth", "sessions", userId] as const;
}

/** The signed-in devices, for "Sicherheit". */
export function useSessions(userId: string) {
  return useQuery({
    queryKey: sessionsKey(userId),
    queryFn: ({ signal }) => listSessions(signal),
  });
}

/**
 * "Abmelden" on a device row. This device signs out as the user panel does
 * (after asking when that ends a call); another one is ended through the
 * API without asking, and the list is fetched again. A device that is gone
 * already (expired, or signed out elsewhere) only refreshes the list.
 */
export async function signOutDevice(
  session: Pick<DeviceSession, "id" | "current">,
  userId: string,
  queryClient: QueryClient,
): Promise<"signedOutHere" | "stayed" | "revoked"> {
  if (session.current) return (await logOut()) ? "signedOutHere" : "stayed";
  try {
    await revokeSession(session.id);
  } catch (error) {
    if (!(error instanceof ApiError && error.code === "not_found")) throw error;
  }
  // Pending until the list no longer has the row.
  await queryClient.invalidateQueries({ queryKey: sessionsKey(userId) });
  return "revoked";
}

// Product and system names are the same in every language.
export const OS_NAMES: Record<Exclude<DeviceOs, "other">, string> = {
  linux: "Linux",
  windows: "Windows",
  macos: "macOS",
  android: "Android",
  ios: "iOS",
};

const BROWSER_NAMES: Record<
  Exclude<DeviceClient, "desktop" | "browser" | "unknown">,
  string
> = {
  firefox: "Firefox",
  chrome: "Chrome",
  edge: "Edge",
  safari: "Safari",
};

/**
 * "Gelabber für Linux", "Firefox unter Windows", "Browser unter Android";
 * without a known system only the app or browser, and "Unbekanntes Gerät"
 * for a client the server could not tell.
 */
export function deviceLabel(device: Pick<DeviceSession, "client" | "os">) {
  const { client, os } = device;
  if (client === "unknown") return translate(account, "devices.unknown");
  const osName = os === "other" ? null : OS_NAMES[os];
  if (client === "desktop") {
    return osName
      ? translate(account, "devices.desktop", { os: osName })
      : translate(account, "devices.desktopOnly");
  }
  const browser =
    client === "browser"
      ? translate(account, "devices.browser")
      : BROWSER_NAMES[client];
  return osName
    ? translate(account, "devices.browserOn", { browser, os: osName })
    : browser;
}

const DAY_MS = 24 * 60 * 60 * 1000;

/** Local midnight of a time, as a day number that ignores DST shifts. */
function localDay(time: Date): number {
  return Math.round(
    Date.UTC(time.getFullYear(), time.getMonth(), time.getDate()) / DAY_MS,
  );
}

/**
 * "Zuletzt aktiv: …". The API's time is accurate to an hour at most, so
 * only the local calendar day is named; this device is "gerade eben".
 */
export function lastActiveLabel(
  session: Pick<DeviceSession, "last_used_at" | "current">,
  now: Date = new Date(),
): string {
  const when = session.current
    ? translate(account, "devices.lastActive.now")
    : dayWords(new Date(session.last_used_at), now);
  return translate(account, "devices.lastActive", { when });
}

function dayWords(time: Date, now: Date): string {
  const days = localDay(now) - localDay(time);
  if (Number.isNaN(days) || days <= 0)
    return translate(account, "devices.lastActive.today");
  if (days === 1) return translate(account, "devices.lastActive.yesterday");
  return translate(account, "devices.lastActive.daysAgo", { count: days });
}
