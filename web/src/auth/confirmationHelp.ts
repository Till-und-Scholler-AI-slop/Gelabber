import type { ApiErrorCode } from "../api/client.ts";
import type { OidcError } from "./rules.ts";

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
