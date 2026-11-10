// How an invite reads in a list: its use count and when it runs out, in
// words instead of a raw URL and a timestamp.

import { translate } from "../i18n/catalog.ts";
import { servers } from "../i18n/catalogs/servers.ts";
import { formatRelative } from "../i18n/format.ts";
import type { Invite } from "./types.ts";

/** "läuft in 6 Tagen ab" */
const runsOut = (value: number, unit: Intl.RelativeTimeFormatUnit) =>
  translate(servers, "expiry.in", { when: formatRelative(value, unit) });

/** "3/∞ benutzt", "1/5 benutzt". */
export function inviteUses(invite: Pick<Invite, "uses" | "max_uses">): string {
  return translate(servers, "expiry.uses", {
    uses: invite.uses,
    max: invite.max_uses ?? "∞",
  });
}

/** "läuft in 6 Tagen ab", "läuft nie ab", "abgelaufen". */
export function inviteExpiry(expiresAt: string | null, now: number): string {
  if (expiresAt === null) return translate(servers, "expiry.never");
  const left = new Date(expiresAt).getTime() - now;
  if (Number.isNaN(left)) return translate(servers, "expiry.never");
  if (left <= 0) return translate(servers, "expiry.expired");
  const minutes = Math.ceil(left / 60_000);
  if (minutes < 60) return runsOut(minutes, "minute");
  const hours = Math.round(minutes / 60);
  if (hours < 48) return runsOut(hours, "hour");
  return runsOut(Math.round(hours / 24), "day");
}
