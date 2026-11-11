// Who may register (`GET /api/auth/registration`, #517): an open instance
// takes anyone, an invitation-only one a beta invitation or a server invite.
// The register page decides from this whether it asks for a code.

import { useQuery } from "@tanstack/react-query";

import { api, ApiError, type FieldCode } from "../api/client.ts";
import { inviteCodeOf, isInviteRedirect } from "./redirect.ts";
import type { AdmissionCodes } from "./session.ts";

export type RegistrationMode = "open" | "invitation";

/**
 * The instance's registration mode. Fixed per deployment, so it is asked
 * once; `undefined` while unknown or after a failed check.
 */
export function useRegistrationMode(): RegistrationMode | undefined {
  const { data } = useQuery({
    queryKey: ["instance", "registration"],
    queryFn: ({ signal }) =>
      api<{ mode: RegistrationMode }>("/auth/registration", { signal }),
    staleTime: Infinity,
    retry: false,
  });
  return data?.mode;
}

export type InvitationState = {
  mode: RegistrationMode | undefined;
  /** `?invitation=` of the page's address. */
  invitation: string | undefined;
  /** The page's `?redirect=`. */
  redirect: string | undefined;
  /** The API (or a sign-in redirect) already asked for a code. */
  asked: boolean;
};

/**
 * Whether the register page shows the "Einladungscode" field: with a code
 * in the address, or when the instance needs one and no server invite is
 * being followed (the invite itself admits then).
 */
export function showsInvitationField(state: InvitationState): boolean {
  if (state.invitation !== undefined) return true;
  if (isInviteRedirect(state.redirect)) return false;
  return state.mode === "invitation" || state.asked;
}

/** "Gelabber ist gerade nur mit Einladung offen.": invitation-only, and
 * the visitor brought no code of either kind. */
export function showsInvitationOnly(state: InvitationState): boolean {
  return (
    state.mode === "invitation" &&
    state.invitation === undefined &&
    !isInviteRedirect(state.redirect)
  );
}

/** The codes the registration sends: the typed invitation (when its field
 * shows) and the server invite of an `/invite/{code}` redirect. */
export function admissionCodes(
  invitationField: string | null,
  redirect: string | undefined,
): AdmissionCodes {
  const codes: AdmissionCodes = {};
  const invitation = invitationField?.trim();
  if (invitation) codes.invitation = invitation;
  const serverInvite = inviteCodeOf(redirect);
  if (serverInvite) codes.server_invite = serverInvite;
  return codes;
}

/** The client-side check of the invitation field: empty is fine unless the
 * instance needs a code and no server invite comes along. */
export function validateInvitation(
  raw: string,
  mode: RegistrationMode | undefined,
  redirect: string | undefined,
): FieldCode | null {
  if (raw.trim().length > 0 || mode !== "invitation") return null;
  return inviteCodeOf(redirect) ? null : "required";
}

/** Whether the API refused for want of a valid code. */
export function isAdmissionError(error: unknown): error is ApiError {
  return (
    error instanceof ApiError &&
    (error.code === "invitation_required" ||
      error.code === "invitation_invalid")
  );
}

/**
 * The refusal as the form shows it: without an invitation field on the
 * page, the API's `{"invitation": …}` would point at nothing, so its code
 * becomes the form's message instead.
 */
export function withoutInvitationField(error: ApiError): ApiError {
  if (!("invitation" in error.fields)) return error;
  const fields = { ...error.fields };
  delete fields.invitation;
  return new ApiError(
    error.code,
    error.status,
    error.message,
    fields,
    error.retryAfter,
  );
}
