// The two choices behind "Ablauf & Limit ändern": how long an invite link
// works and how often. The default is a link that keeps working for a week.

import type { CreateInviteInput } from "./types.ts";

export type InviteExpiry = "1h" | "24h" | "7d" | "never";
export type InviteUses = "1" | "5" | "25" | "unlimited";
export type InviteTerms = { expiry: InviteExpiry; uses: InviteUses };

export const DEFAULT_INVITE_TERMS: InviteTerms = {
  expiry: "7d",
  uses: "unlimited",
};

export const EXPIRY_OPTIONS: readonly { value: InviteExpiry; label: string }[] =
  [
    { value: "1h", label: "1 Stunde" },
    { value: "24h", label: "24 Stunden" },
    { value: "7d", label: "7 Tage" },
    { value: "never", label: "Nie" },
  ];

export const USES_OPTIONS: readonly { value: InviteUses; label: string }[] = [
  { value: "1", label: "1" },
  { value: "5", label: "5" },
  { value: "25", label: "25" },
  { value: "unlimited", label: "Unbegrenzt" },
];

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
      ? "unbegrenzt gültig"
      : `gültig ${EXPIRY_OPTIONS.find((o) => o.value === terms.expiry)?.label}`;
  const uses =
    terms.uses === "unlimited"
      ? "unbegrenzt nutzbar"
      : `${terms.uses}-mal nutzbar`;
  return `${expiry} · ${uses}`;
}
