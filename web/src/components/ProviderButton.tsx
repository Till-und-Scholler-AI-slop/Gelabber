import { useMutation } from "@tanstack/react-query";

import { useAuthProviders } from "../auth/providers.ts";
import { startOidc } from "../auth/session.ts";
import type { AuthProvider } from "../auth/types.ts";
import { useT } from "../i18n/catalog.ts";
import { auth } from "../i18n/catalogs/auth.ts";
import { Button } from "./ui/Button.tsx";

/**
 * "Mit Google anmelden": leaves for the provider's sign-in page. Sized like
 * the form's `SubmitButton size="lg"`. The mark is a neutral initial, not
 * the provider's logo.
 */
export function ProviderButton({
  provider,
  pending,
  disabled,
  onClick,
}: {
  provider: AuthProvider;
  pending: boolean;
  disabled?: boolean;
  onClick: () => void;
}) {
  const t = useT(auth);
  return (
    <Button
      size="lg"
      className="auth-provider"
      icon={
        <span className="auth-provider__mark" aria-hidden="true">
          {Array.from(provider.name)[0]?.toUpperCase()}
        </span>
      }
      pending={pending}
      pendingLabel={t("oidc.redirecting", { provider: provider.name })}
      disabled={disabled}
      onClick={onClick}
    >
      {t("oidc.signInWith", { provider: provider.name })}
    </Button>
  );
}

/**
 * One button per configured provider, then the "oder" divider above the
 * page's own form. Nothing at all while no provider is configured (or the
 * list is still loading), so such a page looks as it always did.
 */
export function ProviderSignIn({
  redirect,
  invitation,
  onError,
}: {
  /** Where the sign-in continues afterwards. */
  redirect: string | undefined;
  /** The register page's invitation code, read when a button is clicked. */
  invitation?: () => string | undefined;
  /** A refusal before leaving the page, for the page's form error. */
  onError: (error: unknown) => void;
}) {
  const t = useT(auth);
  const providers = useAuthProviders();
  const start = useMutation({
    mutationFn: (provider: string) =>
      startOidc(provider, {
        purpose: "sign_in",
        redirect,
        invitation: invitation?.(),
      }),
    onError,
  });
  if (providers.length === 0) return null;
  return (
    <>
      <div className="auth-providers">
        {providers.map((provider) => (
          <ProviderButton
            key={provider.id}
            provider={provider}
            pending={start.isPending && start.variables === provider.id}
            disabled={start.isPending}
            onClick={() => start.mutate(provider.id)}
          />
        ))}
      </div>
      <p className="auth-divider">{t("oidc.or")}</p>
    </>
  );
}
