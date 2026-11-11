import { useMutation } from "@tanstack/react-query";

import { useAuthProviders } from "../auth/providers.ts";
import { startOidc } from "../auth/session.ts";
import type { AuthProvider } from "../auth/types.ts";
import { useT } from "../i18n/catalog.ts";
import { auth } from "../i18n/catalogs/auth.ts";
import { usePageTone } from "./pageTone.ts";
import { Button } from "./ui/Button.tsx";

/**
 * "Über Google anmelden": leaves for the provider's sign-in page. Sized like
 * the form's `SubmitButton size="lg"`. Google's button follows its branding
 * rules: the standard colour G, Google's wording, and its light or dark
 * theme after the page's own. Any other provider gets a neutral initial.
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
  const tone = usePageTone();
  const google = provider.id === "google";
  return (
    <Button
      size="lg"
      className={
        google
          ? `auth-provider auth-provider--google auth-provider--${tone}`
          : "auth-provider"
      }
      icon={
        google ? (
          <GoogleMark />
        ) : (
          <span className="auth-provider__mark" aria-hidden="true">
            {Array.from(provider.name)[0]?.toUpperCase()}
          </span>
        )
      }
      pending={pending}
      pendingLabel={t("oidc.redirecting", { provider: provider.name })}
      disabled={disabled}
      onClick={onClick}
    >
      {google
        ? t("oidc.signInWithGoogle")
        : t("oidc.signInWith", { provider: provider.name })}
    </Button>
  );
}

/** Google's standard colour "G", as its branding guidelines give it. */
function GoogleMark() {
  return (
    <svg
      className="auth-provider__logo"
      viewBox="0 0 48 48"
      width="20"
      height="20"
      aria-hidden="true"
      focusable="false"
    >
      <path
        fill="#EA4335"
        d="M24 9.5c3.54 0 6.71 1.22 9.21 3.6l6.85-6.85C35.9 2.38 30.47 0 24 0 14.62 0 6.51 5.38 2.56 13.22l7.98 6.19C12.43 13.72 17.74 9.5 24 9.5z"
      />
      <path
        fill="#4285F4"
        d="M46.98 24.55c0-1.57-.15-3.09-.38-4.55H24v9.02h12.94c-.58 2.96-2.26 5.48-4.78 7.18l7.73 6c4.51-4.18 7.09-10.36 7.09-17.65z"
      />
      <path
        fill="#FBBC05"
        d="M10.53 28.59c-.48-1.45-.76-2.99-.76-4.59s.27-3.14.76-4.59l-7.98-6.19C.92 16.46 0 20.12 0 24c0 3.88.92 7.54 2.56 10.78l7.97-6.19z"
      />
      <path
        fill="#34A853"
        d="M24 48c6.48 0 11.93-2.13 15.89-5.81l-7.73-6c-2.15 1.45-4.92 2.3-8.16 2.3-6.26 0-11.57-4.22-13.47-9.91l-7.98 6.19C6.51 42.62 14.62 48 24 48z"
      />
    </svg>
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
