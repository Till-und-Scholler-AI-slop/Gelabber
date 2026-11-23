import { ApiError, type ApiErrorCode, type FieldCode } from "../api/client.ts";
import { validateEmail, type OidcError } from "./rules.ts";

/** `email_unverified`: a password sign-in was refused and sent a link.
 * `provider`: a provider sign-in found an account whose address was never
 * confirmed (GEL-40). */
export type ConfirmationHelpReason = "email_unverified" | "provider";

/**
 * Which help the sign-in page shows (`ConfirmationHelp`), from the form's
 * last refusal and the provider's `oidc_error`. A refusal of this page's
 * own sign-in replaces the provider's. Only with account mail: without it
 * there is no link to send.
 */
export function confirmationHelpReason(
  formError: ApiErrorCode | null,
  oidcError: OidcError | undefined,
  accountMail: boolean | undefined,
): ConfirmationHelpReason | null {
  if (accountMail !== true) return null;
  if (formError === "email_unverified") return "email_unverified";
  if (formError === null && oidcError === "account_unverified")
    return "provider";
  return null;
}

/** What one press of the help's button came to. `retryAfter` (seconds) is
 * set when the server asked to wait, so the button counts that down. */
export type ConfirmationSend =
  | { kind: "invalid"; code: FieldCode }
  | { kind: "sent" }
  | { kind: "failed"; code: ApiErrorCode; retryAfter: number | null };

/**
 * Asks for a new link for the address typed in the sign-in form, trimmed.
 * An address the form would refuse is not sent, so its field can say why.
 */
export async function sendConfirmationLink(
  typed: string,
  request: (email: string) => Promise<void>,
): Promise<ConfirmationSend> {
  const code = validateEmail(typed);
  if (code) return { kind: "invalid", code };
  try {
    await request(typed.trim());
    return { kind: "sent" };
  } catch (error) {
    if (!(error instanceof ApiError))
      return { kind: "failed", code: "internal", retryAfter: null };
    return {
      kind: "failed",
      code: error.code,
      retryAfter:
        error.code === "rate_limited" && error.retryAfter
          ? Math.ceil(error.retryAfter)
          : null,
    };
  }
}
