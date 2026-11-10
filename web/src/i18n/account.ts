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

/**
 * Switches the interface at once and saves the choice with the account.
 * A failed save switches back; resolves whether it was saved.
 */
export async function changeAccountLocale(next: Locale): Promise<boolean> {
  const previous = currentLocale();
  const stamp = takeStamp();
  setLocale(next);
  try {
    const saved = await api<{ locale: Locale }>("/me/locale", {
      method: "PUT",
      body: { locale: next },
    });
    if (stampHolds(stamp) && isLocale(saved.locale))
      recordOnUser(saved.locale, stamp.userId);
    return true;
  } catch {
    // Only while nothing newer chose a language meanwhile.
    if (currentLocale() === next) setLocale(previous);
    return false;
  }
}
