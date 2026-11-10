// Client-side mirror of `backend/api/src/auth/validate.rs`, so obvious mistakes are
// shown inline before a round-trip. The server stays the authority; its
// field codes render through the same `fieldMessage` table, whose copy is in
// the `auth` catalog (`error.*`, `fieldLabel.*`, `fieldError.*`).

import type { ApiErrorCode, FieldCode, FieldErrors } from "../api/client.ts";
import { translate } from "../i18n/catalog.ts";
import { auth } from "../i18n/catalogs/auth.ts";
import type { AvatarDecoration, ProfileCard, ProfilePatch } from "./types.ts";

export const PASSWORD_MIN = 8;
export const PASSWORD_MAX = 128;
export const NAME_MAX = 64;
export const EMAIL_MAX = 254;
export const AVATAR_URL_MAX = 2048;
export const BIO_MAX = 190;
export const PRONOUNS_MAX = 40;
export const STATUS_TEXT_MAX = 80;
export const AVATAR_DECORATIONS = ["ring", "glow", "rainbow", "stars"] as const;

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

// RFC 5322 atext, plus any non-ASCII character as RFC 6531 allows.
const ATOM = /^(?:[A-Za-z0-9!#$%&'*+\-/=?^_`{|}~]|\P{ASCII})+$/u;
const LOCAL_PART_MAX_BYTES = 64;

/**
 * `validateEmail` for registration, mirroring the server's `new_email`:
 * one bare address, no display name (`a<x@y>`), quoted local part,
 * comment, or `%` in the domain. Sign-in keeps the lenient check.
 */
export function validateNewEmail(raw: string): FieldCode | null {
  const code = validateEmail(raw);
  if (code) return code;
  const value = raw.trim().toLowerCase();
  const at = value.indexOf("@");
  const local = value.slice(0, at);
  const domain = value.slice(at + 1);
  if (
    new TextEncoder().encode(local).length > LOCAL_PART_MAX_BYTES ||
    !local.split(".").every((atom) => ATOM.test(atom)) ||
    /[%"(),:;<>\\]/.test(domain)
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

const UUID = "[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}";
const UPLOADED_AVATAR = new RegExp(`^/api/avatars/${UUID}/${UUID}$`, "i");

/** An avatar the API stores itself (`PUT /api/me/avatar`), not a link. */
export function isUploadedAvatar(url: string | null | undefined): boolean {
  return url != null && UPLOADED_AVATAR.test(url.trim());
}

/**
 * The PATCH the profile form sends. The avatar goes along only when the
 * field differs from the saved value: echoing an avatar this form loaded
 * earlier would undo an upload made since in another tab (#218 review).
 */
export function profileFormPatch(
  name: string,
  avatarUrl: string,
  savedAvatarUrl: string | null,
): { name: string; avatar_url?: string } {
  const avatar = avatarUrl.trim();
  return {
    name: name.trim(),
    ...(avatar !== (savedAvatarUrl ?? "") ? { avatar_url: avatar } : {}),
  };
}

/**
 * Whether "Entfernen" removes the avatar at once (an uploaded image that is
 * still the saved one) instead of only clearing the field until Speichern.
 */
export function removesAvatarAtOnce(
  savedAvatarUrl: string | null,
  avatarUrl: string,
): boolean {
  return (
    isUploadedAvatar(savedAvatarUrl) && avatarUrl.trim() === savedAvatarUrl
  );
}

/**
 * The avatar field once an immediate removal succeeded: the removed upload
 * goes, a link typed while the removal ran stays.
 */
export function avatarFieldAfterRemoval(current: string): string {
  return isUploadedAvatar(current) ? "" : current;
}

export function validateAvatarUrl(raw: string): FieldCode | null {
  const value = raw.trim();
  if (value.length === 0) return null;
  if (isUploadedAvatar(value)) return null;
  if (chars(value) > AVATAR_URL_MAX) return "too_long";
  // https only: an http image would be mixed content behind TLS.
  if (!/^https:\/\/[^\s/.?#]\S*$/i.test(value)) return "invalid";
  return null;
}

const UPLOADED_BANNER = new RegExp(`^/api/banners/${UUID}/${UUID}$`, "i");

/** A banner the API stores itself (`PUT /api/me/banner`). */
export function isUploadedBanner(url: string | null | undefined): boolean {
  return url != null && UPLOADED_BANNER.test(url.trim());
}

/** Characters as the API counts them, after its `\r\n` → `\n` and trim. */
export function profileTextLength(raw: string): number {
  return chars(raw.replace(/\r\n/g, "\n").trim());
}

function validateOptionalText(
  raw: string,
  max: number,
  multiline: boolean,
): FieldCode | null {
  const value = raw.replace(/\r\n/g, "\n").trim();
  if (value.length === 0) return null;
  if (chars(value) > max) return "too_long";
  for (const char of value) {
    if (multiline && char === "\n") continue;
    if (isControl(char)) return "invalid";
  }
  return null;
}

/**
 * Rust's `char::is_control` (C0, DEL and C1) plus explicit bidi
 * embeddings, overrides and isolates, as the API's `optional_text`.
 */
function isControl(char: string): boolean {
  const code = char.codePointAt(0) ?? 0;
  return (
    code < 0x20 ||
    (code >= 0x7f && code <= 0x9f) ||
    (code >= 0x202a && code <= 0x202e) ||
    (code >= 0x2066 && code <= 0x2069)
  );
}

export const validateBio = (raw: string) =>
  validateOptionalText(raw, BIO_MAX, true);
export const validatePronouns = (raw: string) =>
  validateOptionalText(raw, PRONOUNS_MAX, false);
export const validateStatusText = (raw: string) =>
  validateOptionalText(raw, STATUS_TEXT_MAX, false);

/** `#rrggbb` or empty (unset). */
export function validateColor(raw: string): FieldCode | null {
  const value = raw.trim();
  if (value.length === 0) return null;
  return /^#[0-9a-f]{6}$/i.test(value) ? null : "invalid";
}

/** A stored color as a CSS value, or `null` for anything else. */
export function safeColor(value: string | null | undefined): string | null {
  return value != null && /^#[0-9a-f]{6}$/i.test(value) ? value : null;
}

/** The editable card fields as form strings ("" = unset). */
export type CardForm = {
  bio: string;
  pronouns: string;
  status_text: string;
  accent_color: string;
  banner_color: string;
  avatar_decoration: AvatarDecoration | "";
};

const CARD_FIELDS = [
  "bio",
  "pronouns",
  "status_text",
  "accent_color",
  "banner_color",
  "avatar_decoration",
] as const;

export function cardFormOf(card: ProfileCard): CardForm {
  return {
    bio: card.bio ?? "",
    pronouns: card.pronouns ?? "",
    status_text: card.status_text ?? "",
    accent_color: card.accent_color ?? "",
    banner_color: card.banner_color ?? "",
    avatar_decoration: card.avatar_decoration ?? "",
  };
}

function normalisedCardValue(field: keyof CardForm, raw: string): string {
  const value = raw.replace(/\r\n/g, "\n").trim();
  return field === "accent_color" || field === "banner_color"
    ? value.toLowerCase()
    : value;
}

/**
 * The card fields that differ from the saved profile, as the API stores
 * them. Unchanged fields stay out of the PATCH, so a form loaded earlier
 * does not undo a change made since in another tab.
 */
export function profileCardPatch(
  form: CardForm,
  saved: ProfileCard,
): ProfilePatch {
  const patch: ProfilePatch = {};
  for (const field of CARD_FIELDS) {
    const value = normalisedCardValue(field, form[field]);
    if (value !== (saved[field] ?? "")) {
      Object.assign(patch, { [field]: value });
    }
  }
  return patch;
}

/** Field codes for the card form; empty when everything is acceptable. */
export function validateCardForm(
  form: CardForm,
): Partial<Record<keyof CardForm, FieldCode>> {
  const codes: Partial<Record<keyof CardForm, FieldCode>> = {};
  const checks: [keyof CardForm, FieldCode | null][] = [
    ["bio", validateBio(form.bio)],
    ["pronouns", validatePronouns(form.pronouns)],
    ["status_text", validateStatusText(form.status_text)],
    ["accent_color", validateColor(form.accent_color)],
    ["banner_color", validateColor(form.banner_color)],
  ];
  for (const [field, code] of checks) if (code) codes[field] = code;
  if (
    form.avatar_decoration !== "" &&
    !AVATAR_DECORATIONS.includes(form.avatar_decoration)
  ) {
    codes.avatar_decoration = "invalid";
  }
  return codes;
}

type FieldName =
  | "email"
  | "password"
  | "current_password"
  | "name"
  | "avatar_url"
  | "bio"
  | "pronouns"
  | "status_text"
  | "accent_color"
  | "banner_color"
  | "banner_url"
  | "avatar_decoration"
  | "kind"
  | "category_id"
  | "member_permissions"
  | "max_uses"
  | "expires_in_hours"
  | "content"
  | "before"
  | "after"
  | "limit"
  | "user_id"
  | "filename"
  | "content_type"
  | "size"
  | "attachment_ids"
  | "gif"
  | "q"
  | "token"
  | "color"
  | "permissions"
  | "roles"
  | "role_ids"
  | "invitation";

type LabelKey = Extract<keyof typeof auth.de, `fieldLabel.${string}`>;

const FIELD_LABEL: Record<FieldName, LabelKey> = {
  email: "fieldLabel.email",
  password: "fieldLabel.password",
  current_password: "fieldLabel.currentPassword",
  name: "fieldLabel.name",
  avatar_url: "fieldLabel.avatarUrl",
  bio: "fieldLabel.bio",
  pronouns: "fieldLabel.pronouns",
  status_text: "fieldLabel.statusText",
  accent_color: "fieldLabel.accentColor",
  banner_color: "fieldLabel.bannerColor",
  banner_url: "fieldLabel.bannerUrl",
  avatar_decoration: "fieldLabel.avatarDecoration",
  kind: "fieldLabel.kind",
  category_id: "fieldLabel.categoryId",
  member_permissions: "fieldLabel.memberPermissions",
  max_uses: "fieldLabel.maxUses",
  expires_in_hours: "fieldLabel.expiresInHours",
  content: "fieldLabel.content",
  before: "fieldLabel.cursor",
  after: "fieldLabel.cursor",
  limit: "fieldLabel.limit",
  user_id: "fieldLabel.userId",
  filename: "fieldLabel.filename",
  content_type: "fieldLabel.contentType",
  size: "fieldLabel.size",
  attachment_ids: "fieldLabel.attachmentIds",
  gif: "fieldLabel.gif",
  q: "fieldLabel.q",
  token: "fieldLabel.token",
  color: "fieldLabel.color",
  permissions: "fieldLabel.memberPermissions",
  roles: "fieldLabel.roles",
  role_ids: "fieldLabel.roleIds",
  invitation: "fieldLabel.invitation",
};

/** Inline copy for a field code. Unknown fields fall back to generic text. */
export function fieldMessage(field: string, code: FieldCode): string {
  const label = translate(
    auth,
    (FIELD_LABEL as Record<string, LabelKey>)[field] ?? "fieldLabel.other",
  );
  switch (code) {
    case "required":
      // The API's `{"invitation":"required"}` (invitation_required).
      if (field === "invitation")
        return translate(auth, "error.invitationRequired");
      return translate(auth, "fieldError.required", { label });
    case "too_short":
      return field === "password"
        ? translate(auth, "fieldError.passwordTooShort", { min: PASSWORD_MIN })
        : translate(auth, "fieldError.tooShort", { label });
    case "too_long":
      return field === "size"
        ? translate(auth, "fieldError.fileTooLarge")
        : field === "attachment_ids"
          ? translate(auth, "fieldError.tooManyFiles")
          : field.startsWith("attachment_ids.")
            ? translate(auth, "fieldError.tooManyPixels")
            : translate(auth, "fieldError.tooLong", { label });
    case "too_large":
      return translate(auth, "fieldError.tooLarge");
    case "mismatch":
      return translate(auth, "fieldError.mismatch");
    case "taken":
      // Only a role's name is taken by name; an address by email.
      return field === "name"
        ? translate(auth, "fieldError.roleNameTaken")
        : translate(auth, "fieldError.taken");
    case "too_many":
      return translate(auth, "fieldError.tooManyRoles");
    case "wrong":
      return translate(auth, "fieldError.wrong");
    case "stale":
      return translate(auth, "fieldError.stale");
    case "exclusive":
      return translate(auth, "fieldError.exclusive");
    case "unavailable":
      return translate(auth, "fieldError.unavailable");
    case "not_editable":
      return translate(auth, "fieldError.notEditable");
    case "invalid":
      switch (field) {
        case "email":
          return translate(auth, "fieldError.emailInvalid");
        case "avatar_url":
          return translate(auth, "fieldError.avatarUrlInvalid");
        case "accent_color":
        case "banner_color":
          return translate(auth, "fieldError.colorInvalid");
        case "bio":
        case "pronouns":
        case "status_text":
          return field === "bio"
            ? translate(auth, "fieldError.bioInvalid")
            : translate(auth, "fieldError.lineInvalid");
        case "content_type":
          return translate(auth, "fieldError.contentTypeInvalid");
        case "role_ids":
          return translate(auth, "fieldError.roleIdsInvalid");
        case "invitation":
          return translate(auth, "error.invitationInvalid");
        default:
          return translate(auth, "fieldError.invalid", { label });
      }
  }
}

/** Copy for a form-level error code, in the interface's language. */
export function errorMessage(code: ApiErrorCode): string {
  switch (code) {
    case "invalid_credentials":
      return translate(auth, "error.invalidCredentials");
    case "theme_conflict":
      return translate(auth, "error.themeConflict");
    case "email_taken":
      return translate(auth, "error.emailTaken");
    case "email_unverified":
      return translate(auth, "error.emailUnverified");
    case "token_invalid":
      return translate(auth, "error.tokenInvalid");
    case "unauthenticated":
      return translate(auth, "error.unauthenticated");
    case "csrf_invalid":
      return translate(auth, "error.csrfInvalid");
    case "insecure_transport":
      return translate(auth, "error.insecureTransport");
    case "forbidden":
      return translate(auth, "error.forbidden");
    case "not_found":
      return translate(auth, "error.notFound");
    case "invite_invalid":
      return translate(auth, "error.inviteInvalid");
    case "banned":
      return translate(auth, "error.banned");
    case "rate_limited":
      return translate(auth, "error.rateLimited");
    case "quota_exceeded":
      return translate(auth, "error.quotaExceeded");
    case "upstream_unavailable":
      return translate(auth, "error.upstreamUnavailable");
    case "upstream_quota":
      return translate(auth, "error.upstreamQuota");
    case "validation_failed":
      return translate(auth, "error.validationFailed");
    case "timeout":
      return translate(auth, "error.timeout");
    case "network":
      return translate(auth, "error.network");
    case "bad_request":
    case "internal":
      return translate(auth, "error.internal");
    case "invitation_required":
      return translate(auth, "error.invitationRequired");
    case "invitation_invalid":
      return translate(auth, "error.invitationInvalid");
    case "account_disabled":
      return translate(auth, "error.accountDisabled");
    case "server_blocked":
      return translate(auth, "error.serverBlocked");
  }
}

/** Turns per-field codes into per-field messages. */
export function fieldMessages(fields: FieldErrors): Record<string, string> {
  const out: Record<string, string> = {};
  for (const [field, code] of Object.entries(fields)) {
    if (code) out[field] = fieldMessage(field, code);
  }
  return out;
}
