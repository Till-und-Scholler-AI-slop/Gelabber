// How an invite reads in a list: its use count and when it runs out, in
// words instead of a raw URL and a timestamp.

import type { Invite } from "./types.ts";

const relative = new Intl.RelativeTimeFormat("de-DE", { numeric: "always" });

/** "3/∞ benutzt", "1/5 benutzt". */
export function inviteUses(invite: Pick<Invite, "uses" | "max_uses">): string {
  return `${invite.uses}/${invite.max_uses ?? "∞"} benutzt`;
}

/** "läuft in 6 Tagen ab", "läuft nie ab", "abgelaufen". */
export function inviteExpiry(expiresAt: string | null, now: number): string {
  if (expiresAt === null) return "läuft nie ab";
  const left = new Date(expiresAt).getTime() - now;
  if (Number.isNaN(left)) return "läuft nie ab";
  if (left <= 0) return "abgelaufen";
  const minutes = Math.ceil(left / 60_000);
  if (minutes < 60) return `läuft ${relative.format(minutes, "minute")} ab`;
  const hours = Math.round(minutes / 60);
  if (hours < 48) return `läuft ${relative.format(hours, "hour")} ab`;
  return `läuft ${relative.format(Math.round(hours / 24), "day")} ab`;
}
