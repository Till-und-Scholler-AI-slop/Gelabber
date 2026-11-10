// Beta invitations: the form's request and error copy, and a row's menu
// (copy the link, revoke it). Kept apart from the page so tests can call
// them directly.

import { ApiError, type FieldErrors } from "../api/client.ts";
import { fieldMessage, validateEmail } from "../auth/rules.ts";
import type { MenuEntry } from "../components/rowMenuEntries.ts";
import { notify, notifyError } from "../components/toasts.ts";
import { translate } from "../i18n/catalog.ts";
import { operator } from "../i18n/catalogs/operator.ts";
import { copyText } from "../servers/clipboard.ts";
import * as remote from "./api.ts";
import { askThenRun } from "./serverActions.tsx";
import type { CreateInvitationInput, Invitation } from "./types.ts";

/** The longest note the API takes. */
export const NOTE_MAX = 200;

/** The validity choices, as chip values; `never` sends `null`. */
export const EXPIRY_CHOICES = ["day1", "day7", "day30", "never"] as const;
export type ExpiryChoice = (typeof EXPIRY_CHOICES)[number];
export const DEFAULT_EXPIRY: ExpiryChoice = "day7";

const EXPIRY_DAYS: Record<
  ExpiryChoice,
  CreateInvitationInput["expires_in_days"]
> = { day1: 1, day7: 7, day30: 30, never: null };

export type InvitationForm = {
  email: string;
  note: string;
  expiry: ExpiryChoice;
  sendMail: boolean;
};

export const EMPTY_INVITATION_FORM: InvitationForm = {
  email: "",
  note: "",
  expiry: DEFAULT_EXPIRY,
  sendMail: false,
};

/**
 * Whether "Per E-Mail schicken" is offered: only when this instance sends
 * mail and an address is entered.
 */
export function canSendMail(form: InvitationForm, mail: boolean): boolean {
  return mail && form.email.trim().length > 0;
}

/** What `POST /api/operator/invitations` gets from the form. */
export function invitationInput(
  form: InvitationForm,
  mail: boolean,
): CreateInvitationInput {
  const email = form.email.trim();
  const note = form.note.trim();
  return {
    ...(email ? { email } : {}),
    ...(note ? { note } : {}),
    expires_in_days: EXPIRY_DAYS[form.expiry],
    ...(form.sendMail && canSendMail(form, mail) ? { send_mail: true } : {}),
  };
}

/** The form's own checks before it asks: a wrong address, a long note. */
export function checkInvitationForm(form: InvitationForm): FieldErrors {
  const errors: FieldErrors = {};
  if (form.email.trim()) {
    const code = validateEmail(form.email);
    if (code) errors.email = code;
  }
  if (Array.from(form.note.trim()).length > NOTE_MAX) errors.note = "too_long";
  return errors;
}

/** Inline copy for the form's field errors, from the API or the checks. */
export function invitationFieldMessages(
  fields: FieldErrors,
): Record<string, string> {
  const out: Record<string, string> = {};
  for (const [field, code] of Object.entries(fields)) {
    if (!code) continue;
    if (field === "note")
      out.note =
        code === "too_long"
          ? translate(operator, "invitations.error.tooLong", { max: NOTE_MAX })
          : translate(operator, "invitations.error.lineInvalid");
    else if (field === "send_mail")
      out.send_mail = translate(operator, "invitations.error.mailUnavailable");
    else if (field === "expires_in_days")
      out.expires_in_days = translate(
        operator,
        "invitations.error.expiresInvalid",
      );
    else out[field] = fieldMessage(field, code);
  }
  return out;
}

export type InvitationHandlers = { copy: () => void; revoke: () => void };

/** An open invitation's menu; the others have none. */
export function invitationMenuEntries(
  invitation: Pick<Invitation, "status">,
  on: InvitationHandlers,
): MenuEntry[] {
  if (invitation.status !== "open") return [];
  return [
    {
      kind: "item",
      label: translate(operator, "invitations.menu.copy"),
      onSelect: on.copy,
    },
    {
      kind: "item",
      label: translate(operator, "invitations.menu.revoke"),
      danger: true,
      onSelect: on.revoke,
    },
  ];
}

/** Copies a link and says whether it worked. */
export async function copyInvitationLink(link: string): Promise<boolean> {
  const done = await copyText(link);
  if (done) notify(translate(operator, "toasts.linkCopied"));
  else notifyError(new Error(translate(operator, "toasts.copyFailed")));
  return done;
}

export function revokeInvitation(
  invitation: Pick<Invitation, "id">,
  refresh: () => void,
) {
  return askThenRun({
    title: translate(operator, "dialogs.revokeTitle"),
    message: translate(operator, "dialogs.revokeMessage"),
    confirmLabel: translate(operator, "invitations.menu.revoke"),
    run: async () => {
      try {
        await remote.revokeInvitation(invitation.id);
      } catch (error) {
        // 422 {"invitation":"used"}: someone registered with it meanwhile.
        // `used` is not a shared field code, so it is compared as text.
        if (
          error instanceof ApiError &&
          (error.fields.invitation as string | undefined) === "used"
        )
          throw new Error(translate(operator, "toasts.invitationUsed"), {
            cause: error,
          });
        throw error;
      }
    },
    done: translate(operator, "toasts.revoked"),
    failed: translate(operator, "toasts.revokeFailed"),
    gone: translate(operator, "toasts.invitationGone"),
    refresh,
  });
}
