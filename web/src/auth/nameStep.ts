import { ApiError } from "../api/client.ts";
import { safeRedirect } from "./redirect.ts";
import { isAdmissionError, withoutInvitationField } from "./registration.ts";
import { completeOidc } from "./session.ts";

/** What "Konto anlegen" on `/sign-in/name` comes to. */
export type NameStepOutcome =
  /** The account exists and this tab is signed in; go on to `href`. */
  | { kind: "signed_in"; href: string }
  /** The step's token is used, expired or bound to another browser. */
  | { kind: "dead" }
  /** Refused (name, admission, account): the form shows `error`. */
  | { kind: "refused"; error: unknown };

/** The step's token is gone (`410 token_invalid`). */
export function isDeadToken(error: unknown): boolean {
  return error instanceof ApiError && error.code === "token_invalid";
}

/** Creates the account with `name` and says where the page goes next. */
export async function completeNameStep(
  token: string,
  name: string,
): Promise<NameStepOutcome> {
  try {
    const session = await completeOidc(token, name.trim());
    return { kind: "signed_in", href: safeRedirect(session.redirect) ?? "/" };
  } catch (error) {
    if (isDeadToken(error)) return { kind: "dead" };
    // No invitation field on the page: a refusal is the form's message.
    return {
      kind: "refused",
      error: isAdmissionError(error) ? withoutInvitationField(error) : error,
    };
  }
}
