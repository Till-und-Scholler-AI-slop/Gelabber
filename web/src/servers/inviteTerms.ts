// The two choices behind "Ablauf & Limit ändern": how long an invite link
// works and how often. The default is a link that keeps working for a week.

import { translate } from "../i18n/catalog.ts";
import { servers } from "../i18n/catalogs/servers.ts";
import type { CreateInviteInput } from "./types.ts";

export type InviteExpiry = "1h" | "24h" | "7d" | "never";
export type InviteUses = "1" | "5" | "25" | "unlimited";
export type InviteTerms = { expiry: InviteExpiry; uses: InviteUses };

export const DEFAULT_INVITE_TERMS: InviteTerms = {
  expiry: "7d",
  uses: "unlimited",
};

const EXPIRY_KEY = {
  "1h": "terms.expiry.hour",
  "24h": "terms.expiry.day",
  "7d": "terms.expiry.week",
  never: "terms.expiry.never",
} as const satisfies Record<InviteExpiry, string>;

const EXPIRIES: readonly InviteExpiry[] = ["1h", "24h", "7d", "never"];

/** The expiry chips, labelled in the interface's language. */
export function expiryOptions(): { value: InviteExpiry; label: string }[] {
  return EXPIRIES.map((value) => ({
    value,
    label: translate(servers, EXPIRY_KEY[value]),
  }));
}

/** The use-limit chips; the counts need no words. */
export function usesOptions(): { value: InviteUses; label: string }[] {
  return [
    { value: "1", label: "1" },
    { value: "5", label: "5" },
    { value: "25", label: "25" },
    { value: "unlimited", label: translate(servers, "terms.uses.unlimited") },
  ];
}

const HOURS: Record<InviteExpiry, number | null> = {
  "1h": 1,
  "24h": 24,
  "7d": 7 * 24,
  never: null,
};

/** The request body for these terms; a missing field means "no limit". */
export function inviteInput(terms: InviteTerms): CreateInviteInput {
  const hours = HOURS[terms.expiry];
  return {
    ...(hours === null ? {} : { expires_in_hours: hours }),
    ...(terms.uses === "unlimited" ? {} : { max_uses: Number(terms.uses) }),
  };
}

/** "gültig 7 Tage · unbegrenzt nutzbar" */
export function termsSummary(terms: InviteTerms): string {
  const expiry =
    terms.expiry === "never"
      ? translate(servers, "terms.validForever")
      : translate(servers, "terms.validFor", {
          duration: translate(servers, EXPIRY_KEY[terms.expiry]),
        });
  const uses =
    terms.uses === "unlimited"
      ? translate(servers, "terms.usesUnlimited")
      : translate(servers, "terms.usesLimited", { count: Number(terms.uses) });
  return `${expiry} · ${uses}`;
}
