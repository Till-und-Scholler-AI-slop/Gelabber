import { useQuery } from "@tanstack/react-query";

import { ApiError } from "../api/client.ts";
import type { ConfirmRequest } from "../components/confirm.ts";
import { translate } from "../i18n/catalog.ts";
import { settings } from "../i18n/catalogs/settings.ts";
import { isOidcError, type OidcError } from "./rules.ts";
import { signInMethods } from "./session.ts";
import type { AuthProvider, SignInMethods } from "./types.ts";

/** Where a provider flow started from Sicherheit comes back to. */
export const SECURITY_PATH = "/settings/konto";

/** The query key of an account's sign-in methods (`GET /api/auth/methods`). */
export function signInMethodsKey(userId: string) {
  return ["auth", "methods", userId] as const;
}

/** How the signed-in account can sign in, for "Sicherheit". */
export function useSignInMethods(userId: string) {
  return useQuery({
    queryKey: signInMethodsKey(userId),
    queryFn: ({ signal }) => signInMethods(signal),
  });
}

/**
 * What a provider's link flow reported when it came back to
 * `/settings/konto`: `?connected={provider}` or `?oidc_error={code}`.
 */
export type SignInFeedback = {
  connected?: string;
  oidcError?: OidcError;
};

export function signInFeedback(search: unknown): SignInFeedback {
  if (typeof search !== "object" || search === null) return {};
  const { connected, oidc_error } = search as Record<string, unknown>;
  const out: SignInFeedback = {};
  if (typeof connected === "string" && /^[a-z0-9_-]{1,32}$/.test(connected))
    out.connected = connected;
  if (isOidcError(oidc_error)) out.oidcError = oidc_error;
  return out;
}

/**
 * Where "Mit … bestätigen" in the Konto löschen dialog comes back to, so
 * the dialog opens again once the person has signed in afresh (#519).
 */
export const DELETE_ACCOUNT_RETURN = `${SECURITY_PATH}?delete=1`;

/** Back from that sign-in, and it worked: open Konto löschen again. */
export function reopensDeletion(search: unknown): boolean {
  if (typeof search !== "object" || search === null) return false;
  const { delete: reopen, oidc_error } = search as Record<string, unknown>;
  return (reopen === 1 || reopen === "1") && oidc_error === undefined;
}

/**
 * The address's search without what `signInFeedback` and
 * `reopensDeletion` read.
 */
export function withoutFeedback<T extends object>(search: T): T {
  const rest: Partial<Record<string, unknown>> = { ...search };
  delete rest.connected;
  delete rest.oidc_error;
  delete rest.delete;
  return rest as T;
}

/**
 * The provider a password-less account signs in with: named in the
 * Passwort row, and the one "Mit … bestätigen" signs in again through.
 */
export function signInProvider(methods: SignInMethods): AuthProvider | null {
  const connected = methods.providers.filter((p) => p.connected);
  const provider = connected.find((p) => p.configured) ?? connected[0] ?? null;
  return provider ? { id: provider.id, name: provider.name } : null;
}

/**
 * The rows an `oidc_error` shows on. The code does not say which provider
 * it came from; only a row that is not connected can have started a link,
 * and failing that the first row.
 */
export function oidcErrorRows(methods: SignInMethods): string[] {
  const open = methods.providers.filter((p) => !p.connected).map((p) => p.id);
  if (open.length > 0) return open;
  const first = methods.providers[0];
  return first ? [first.id] : [];
}

/**
 * A first password the API refused for an old sign-in: the dialog then
 * offers to sign in again through the provider.
 */
export function needsReauth(error: unknown): boolean {
  return error instanceof ApiError && error.code === "reauth_required";
}

/** The methods with a password set, after "Festlegen" succeeded. */
export function withPassword(
  methods: SignInMethods | undefined,
): SignInMethods | undefined {
  return methods ? { ...methods, password: true } : methods;
}

/** The question before "Trennen". */
export function disconnectConfirm(providerName: string): ConfirmRequest {
  const params = { provider: providerName };
  return {
    title: translate(
      settings,
      "security.signIn.disconnect.confirmTitle",
      params,
    ),
    message: translate(
      settings,
      "security.signIn.disconnect.confirmMessage",
      params,
    ),
    confirmLabel: translate(
      settings,
      "security.signIn.disconnect.confirmLabel",
    ),
  };
}
