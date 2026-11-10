// The account's language (users.locale) and the interface's: the session
// user brings it, `PUT /api/me/locale` changes it, and the `lc` frame
// tells every other tab and device of the account.

import { useEffect } from "react";

import { api } from "../api/client.ts";
import { stampHolds, takeStamp } from "../auth/scope.ts";
import { useSession } from "../auth/store.ts";
import { getGateway } from "../ws/client.ts";
import { currentLocale, isLocale, setLocale, type Locale } from "./locale.ts";

/** Writes the language into the signed-in user, while it is still them. */
function recordOnUser(locale: Locale, userId?: string): void {
  const user = useSession.getState().user;
  if (!user || (userId !== undefined && user.id !== userId)) return;
  if (user.locale !== locale)
    useSession.setState({ user: { ...user, locale } });
}

/**
 * Shows the account's language whenever the session user changes to
 * another account or another language. Signing out keeps the last one.
 * Returns the unsubscribe.
 */
export function followAccountLocale(): () => void {
  return useSession.subscribe((state, previous) => {
    const locale = state.user?.locale;
    if (!isLocale(locale)) return;
    if (
      state.user?.id === previous.user?.id &&
      locale === previous.user?.locale
    )
      return;
    if (locale !== currentLocale()) setLocale(locale);
  });
}

/** The language changed on another tab or device of this account. */
export function useAccountLocaleBridge(): void {
  const userId = useSession((state) => state.user?.id);
  useEffect(() => {
    if (!userId) return;
    return getGateway().onAccount((frame) => {
      if (frame.op !== "lc" || !isLocale(frame.l)) return;
      setLocale(frame.l);
      recordOnUser(frame.l, userId);
    });
  }, [userId]);
}

// Only the newest change speaks for the account: an older answer that
// arrives late must not undo a newer choice.
let latestChange = 0;
let latestSettled = 0;
// What the server confirmed for an older change while a newer one was
// still in flight, so a failing newer change knows what is really saved.
let savedMeanwhile: { userId?: string; locale: Locale } | null = null;

/**
 * Switches the interface at once and saves the choice with the account.
 * A failed save switches back to the language the server last confirmed;
 * resolves whether it was saved.
 */
export async function changeAccountLocale(next: Locale): Promise<boolean> {
  const change = ++latestChange;
  const previous = currentLocale();
  const stamp = takeStamp();
  setLocale(next);
  try {
    const saved = await api<{ locale: Locale }>("/me/locale", {
      method: "PUT",
      body: { locale: next },
    });
    const confirmed =
      stampHolds(stamp) && isLocale(saved.locale) ? saved.locale : null;
    if (change === latestChange) {
      latestSettled = change;
      savedMeanwhile = null;
      if (confirmed) recordOnUser(confirmed, stamp?.userId);
    } else if (confirmed && latestSettled !== latestChange) {
      savedMeanwhile = { userId: stamp?.userId, locale: confirmed };
    }
    return true;
  } catch {
    if (change !== latestChange) return false;
    latestSettled = change;
    const meanwhile =
      savedMeanwhile &&
      stampHolds(stamp) &&
      savedMeanwhile.userId === stamp.userId
        ? savedMeanwhile.locale
        : null;
    savedMeanwhile = null;
    const user = useSession.getState().user;
    const confirmed =
      meanwhile ?? (isLocale(user?.locale) ? user.locale : previous);
    // Only while nothing else chose a language meanwhile.
    if (currentLocale() === next) setLocale(confirmed);
    if (meanwhile && stampHolds(stamp)) recordOnUser(meanwhile, stamp.userId);
    return false;
  }
}
