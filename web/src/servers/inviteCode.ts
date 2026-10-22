// What someone pastes into "Einladungslink oder Code": a full link, a link
// without "https://", a bare `/invite/<code>` path or just the code.

/** Same shape as the API's codes: 12 characters, nothing confusable. */
const CODE = /^[abcdefghjkmnpqrstuvwxyz23456789]{12}$/;

export type InviteInputError = "empty" | "foreign" | "invalid";

export type ParsedInvite = { code: string } | { error: InviteInputError };

function codeFromPath(path: string): ParsedInvite {
  const match = /^\/invite\/([^/?#]+)\/?$/.exec(path);
  if (!match) return { error: "invalid" };
  return bareCode(decodeURIComponent(match[1]));
}

function bareCode(value: string): ParsedInvite {
  const code = value.toLowerCase();
  return CODE.test(code) ? { code } : { error: "invalid" };
}

/** `origin` is this app's own, e.g. `window.location.origin`. */
export function parseInvite(input: string, origin: string): ParsedInvite {
  const value = input.trim();
  if (!value) return { error: "empty" };
  const own = new URL(origin);
  if (value.startsWith("/")) return codeFromPath(value);
  let url: URL | null = null;
  try {
    if (/^https?:\/\//i.test(value)) url = new URL(value);
    else if (/^[^\s/]+\.[^\s/]+\/invite\//i.test(value))
      url = new URL(`${own.protocol}//${value}`);
  } catch {
    return { error: "invalid" };
  }
  if (url) {
    if (url.host !== own.host) return { error: "foreign" };
    return codeFromPath(url.pathname);
  }
  return bareCode(value);
}

export function inviteInputMessage(error: InviteInputError): string {
  switch (error) {
    case "empty":
      return "Bitte einen Einladungslink oder Code eingeben.";
    case "foreign":
      return "Dieser Link gehört zu einem anderen Gelabber-Server.";
    case "invalid":
      return "Das sieht nicht wie ein Einladungslink aus.";
  }
}

const UNITS: [Intl.RelativeTimeFormatUnit, number][] = [
  ["day", 86_400_000],
  ["hour", 3_600_000],
  ["minute", 60_000],
];

/** "läuft in 6 Tagen ab", "läuft morgen ab"; `null` for a link without end. */
export function inviteExpiry(
  expiresAt: string | null,
  now = Date.now(),
): string | null {
  if (!expiresAt) return null;
  const left = Date.parse(expiresAt) - now;
  if (Number.isNaN(left)) return null;
  if (left <= 0) return "abgelaufen";
  const format = new Intl.RelativeTimeFormat("de", { numeric: "auto" });
  const [unit, size] =
    UNITS.find(([, ms]) => left >= ms) ?? UNITS[UNITS.length - 1];
  return `läuft ${format.format(Math.max(1, Math.round(left / size)), unit)} ab`;
}
