// Client-side mirror of `api/src/auth/validate.rs`, so obvious mistakes are
// shown inline before a round-trip. The server stays the authority; its
// field codes render through the same `fieldMessage` table.

import type { ApiErrorCode, FieldCode, FieldErrors } from "../api/client.ts";

export const PASSWORD_MIN = 8;
export const PASSWORD_MAX = 128;
export const NAME_MAX = 64;
export const EMAIL_MAX = 254;
export const AVATAR_URL_MAX = 2048;

const chars = (value: string): number => Array.from(value).length;

export function validateEmail(raw: string): FieldCode | null {
  const value = raw.trim().toLowerCase();
  if (value.length === 0) return "required";
  if (chars(value) > EMAIL_MAX) return "too_long";
  const at = value.indexOf("@");
  if (at <= 0 || at !== value.lastIndexOf("@")) return "invalid";
  const domain = value.slice(at + 1);
  if (
    domain.length === 0 ||
    !domain.includes(".") ||
    domain.startsWith(".") ||
    domain.endsWith(".") ||
    /\s/.test(value)
  ) {
    return "invalid";
  }
  return null;
}

export function validatePassword(raw: string): FieldCode | null {
  const count = chars(raw);
  if (count === 0) return "required";
  if (count < PASSWORD_MIN) return "too_short";
  if (count > PASSWORD_MAX) return "too_long";
  return null;
}

export function validateName(raw: string): FieldCode | null {
  const value = raw.trim();
  if (value.length === 0) return "required";
  if (chars(value) > NAME_MAX) return "too_long";
  // eslint-disable-next-line no-control-regex
  if (/[\u0000-\u001f\u007f]/.test(value)) return "invalid";
  return null;
}

export function validateAvatarUrl(raw: string): FieldCode | null {
  const value = raw.trim();
  if (value.length === 0) return null;
  if (chars(value) > AVATAR_URL_MAX) return "too_long";
  if (!/^https?:\/\/\S+$/i.test(value)) return "invalid";
  return null;
}

type FieldName = "email" | "password" | "name" | "avatar_url";

const FIELD_LABEL: Record<FieldName, string> = {
  email: "E-Mail-Adresse",
  password: "Passwort",
  name: "Name",
  avatar_url: "Avatar-URL",
};

/** German inline copy for a field code. Unknown fields fall back to generic text. */
export function fieldMessage(field: string, code: FieldCode): string {
  const label = (FIELD_LABEL as Record<string, string>)[field] ?? "Eingabe";
  switch (code) {
    case "required":
      return `${label} fehlt.`;
    case "too_short":
      return field === "password"
        ? `Mindestens ${PASSWORD_MIN} Zeichen.`
        : `${label} ist zu kurz.`;
    case "too_long":
      return `${label} ist zu lang.`;
    case "taken":
      return "Diese E-Mail-Adresse ist schon registriert.";
    case "invalid":
      switch (field) {
        case "email":
          return "Das sieht nicht wie eine E-Mail-Adresse aus.";
        case "avatar_url":
          return "Bitte eine vollständige http(s)-URL angeben.";
        default:
          return `${label} ist ungültig.`;
      }
  }
}

/** German copy for a form-level error code. */
export function errorMessage(code: ApiErrorCode): string {
  switch (code) {
    case "invalid_credentials":
      return "E-Mail oder Passwort ist falsch.";
    case "email_taken":
      return "Diese E-Mail-Adresse ist schon registriert.";
    case "unauthenticated":
      return "Bitte melde dich erneut an.";
    case "csrf_invalid":
      return "Sicherheits-Token abgelaufen. Bitte noch einmal versuchen.";
    case "validation_failed":
      return "Bitte die markierten Felder prüfen.";
    case "timeout":
      return "Der Server antwortet nicht. Bitte noch einmal versuchen.";
    case "network":
      return "Keine Verbindung zum Server.";
    case "bad_request":
    case "internal":
      return "Da ist etwas schiefgelaufen. Bitte noch einmal versuchen.";
  }
}

/** Turns per-field codes into per-field German messages. */
export function fieldMessages(fields: FieldErrors): Record<string, string> {
  const out: Record<string, string> = {};
  for (const [field, code] of Object.entries(fields)) {
    if (code) out[field] = fieldMessage(field, code);
  }
  return out;
}
