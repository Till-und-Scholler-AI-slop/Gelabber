import { useMutation, useQuery } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { useRef, useState, type FormEvent } from "react";

import { ApiError } from "../api/client.ts";
import { useLinkToken } from "../auth/mail.ts";
import { completeNameStep, isDeadToken } from "../auth/nameStep.ts";
import {
  NAME_MAX,
  errorMessage,
  fieldMessage,
  validateName,
} from "../auth/rules.ts";
import { oidcPending } from "../auth/session.ts";
import type { OidcPending } from "../auth/types.ts";
import { useFocusOnMount } from "../auth/useFocusOnMount.ts";
import { useFormErrors } from "../auth/useFormErrors.ts";
import { Field } from "../components/Field.tsx";
import { FormError } from "../components/FormError.tsx";
import { LoadError } from "../components/LoadError.tsx";
import { SubmitButton } from "../components/SubmitButton.tsx";
import { SkeletonCard } from "../components/ui/Skeleton.tsx";
import { T, useT } from "../i18n/catalog.ts";
import { auth } from "../i18n/catalogs/auth.ts";

/**
 * The last step of a first sign-in through a provider: the new account's
 * display name. The provider's callback sends the browser here with the
 * step's token in the fragment; the account exists only after "Konto
 * anlegen", and then this tab is signed in.
 */
export function SignInNamePage() {
  const token = useLinkToken();
  const pending = useQuery({
    queryKey: ["oidc", "pending", token],
    queryFn: ({ signal }) => oidcPending(token, signal),
    enabled: token !== "",
    retry: false,
    staleTime: Infinity,
  });

  if (token === "" || isDeadToken(pending.error)) return <DeadSignIn />;
  if (pending.data) return <NameForm token={token} pending={pending.data} />;
  if (pending.isError) {
    return (
      <section className="lr-auth-card">
        <SignInNameHeading />
        <div className="auth-result">
          <LoadError
            message={errorMessage(
              pending.error instanceof ApiError
                ? pending.error.code
                : "internal",
            )}
            pending={pending.isFetching}
            onRetry={() => void pending.refetch()}
          />
        </div>
      </section>
    );
  }
  return <SkeletonCard />;
}

function SignInNameHeading() {
  const t = useT(auth);
  return <h1>{t("signInName.title")}</h1>;
}

function NameForm({ token, pending }: { token: string; pending: OidcPending }) {
  const t = useT(auth);
  const navigate = useNavigate();
  const [name, setName] = useState(pending.name ?? "");
  const [deadLink, setDeadLink] = useState(false);
  const form = useRef<HTMLFormElement>(null);
  const { errors, clearField, setFields, fromError, cooldown } =
    useFormErrors(form);

  const mutation = useMutation({
    mutationFn: () => completeNameStep(token, name),
    onSuccess: (outcome) => {
      if (outcome.kind === "signed_in")
        void navigate({ href: outcome.href, replace: true });
      else if (outcome.kind === "dead") setDeadLink(true);
      else fromError(outcome.error);
    },
  });

  const onSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const code = validateName(name);
    if (code) {
      setFields({ name: fieldMessage("name", code) });
      return;
    }
    mutation.mutate();
  };

  if (deadLink) return <DeadSignIn />;

  return (
    <section className="lr-auth-card">
      <SignInNameHeading />
      <p className="auth-lead">{t("signInName.lead")}</p>
      <p className="auth-lead">
        <T
          catalog={auth}
          k="signInName.signedInAs"
          params={{ provider: pending.provider.name, email: pending.email }}
          tags={{ b: (text) => <strong>{text}</strong> }}
        />
      </p>
      <form ref={form} onSubmit={onSubmit} noValidate className="auth-form">
        <Field
          id="name"
          label={t("signInName.nameLabel")}
          type="text"
          autoComplete="nickname"
          autoFocus
          maxLength={NAME_MAX}
          value={name}
          onChange={(event) => {
            setName(event.target.value);
            clearField("name");
          }}
          error={errors.fields.name}
          hint={t("register.nameHint")}
        />
        <FormError message={errors.form} />
        <SubmitButton
          size="lg"
          pending={mutation.isPending}
          pendingLabel={t("signInName.submitting")}
          cooldown={cooldown}
        >
          {t("signInName.submit")}
        </SubmitButton>
      </form>
    </section>
  );
}

/** The step's token is gone: used, expired or never there. */
function DeadSignIn() {
  const t = useT(auth);
  const heading = useFocusOnMount<HTMLHeadingElement>();
  return (
    <section className="lr-auth-card">
      <h1 ref={heading} tabIndex={-1}>
        {t("signInName.expiredTitle")}
      </h1>
      <div className="auth-result">
        <p>{t("signInName.expired")}</p>
      </div>
      <div className="auth-actions">
        <Link to="/login" className="auth-link self-start">
          {t("page.toSignIn")}
        </Link>
      </div>
    </section>
  );
}
