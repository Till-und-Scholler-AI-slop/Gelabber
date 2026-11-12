import { useMutation } from "@tanstack/react-query";
import { Link, useNavigate, useSearch } from "@tanstack/react-router";
import { useRef, useState, type FormEvent } from "react";

import { ApiError } from "../api/client.ts";
import { useAccountMailStatus } from "../auth/mail.ts";
import { isInviteRedirect, savePendingRedirect } from "../auth/redirect.ts";
import {
  fieldMessage,
  oidcErrorMessage,
  validateEmail,
} from "../auth/rules.ts";
import { login } from "../auth/session.ts";
import { useFormErrors } from "../auth/useFormErrors.ts";
import { Field } from "../components/Field.tsx";
import { FormError } from "../components/FormError.tsx";
import { ProviderSignIn } from "../components/ProviderButton.tsx";
import { SubmitButton } from "../components/SubmitButton.tsx";
import { buttonClass } from "../components/ui/buttonClass.ts";
import { PasswordField } from "../components/ui/PasswordField.tsx";
import { useT } from "../i18n/catalog.ts";
import { auth } from "../i18n/catalogs/auth.ts";
import { InstallDisclosure } from "../pwa/InstallApp.tsx";
import { installHintUseful, useInstallation } from "../pwa/install.ts";
import { InviteContextBanner } from "./AuthLayout.tsx";

export function LoginPage() {
  const t = useT(auth);
  const navigate = useNavigate();
  const {
    redirect,
    reason,
    oidc_error: oidcError,
  } = useSearch({ from: "/auth/login" });
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const form = useRef<HTMLFormElement>(null);
  const { errors, clearField, setFields, fromError, cooldown } =
    useFormErrors(form);
  const installHint = useInstallation(installHintUseful);
  const accountMail = useAccountMailStatus();
  const invited = isInviteRedirect(redirect);

  const mutation = useMutation({
    mutationFn: () => login(email, password),
    onSuccess: () => {
      // The session store already holds the user; this is a client-side
      // route change, nothing reloads.
      void navigate({ href: redirect ?? "/", replace: true });
    },
    onError: (error) => {
      // A fresh confirmation link is on its way; it opens in a new tab,
      // which only knows where to continue from here.
      if (
        redirect &&
        error instanceof ApiError &&
        error.code === "email_unverified"
      )
        savePendingRedirect(redirect);
      fromError(error);
    },
  });

  const onSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const fields: Record<string, string> = {};
    const emailCode = validateEmail(email);
    if (emailCode) fields.email = fieldMessage("email", emailCode);
    if (password.length === 0)
      fields.password = fieldMessage("password", "required");
    if (Object.keys(fields).length > 0) {
      setFields(fields);
      return;
    }
    mutation.mutate();
  };

  const typedEmail = email.trim();

  return (
    <section className="lr-auth-card">
      {invited ? <InviteContextBanner /> : null}
      {reason === "expired" ? (
        <p role="status" className="auth-banner">
          {t("login.sessionExpired")}
        </p>
      ) : reason === "deleted" ? (
        <p role="status" className="auth-banner">
          {t("login.accountDeleted")}
        </p>
      ) : null}
      <h1>{t("login.title")}</h1>
      {invited ? (
        <>
          <p className="auth-lead">{t("login.newHere")}</p>
          <Link
            to="/register"
            search={{ redirect }}
            className={buttonClass({ size: "lg", className: "mt-3" })}
          >
            {t("login.createAccount")}
          </Link>
        </>
      ) : (
        <p className="auth-lead">
          {t("login.noAccount")}{" "}
          <Link
            to="/register"
            search={redirect ? { redirect } : {}}
            className="auth-link"
          >
            {t("login.register")}
          </Link>
        </p>
      )}
      <ProviderSignIn redirect={redirect} onError={fromError} />

      <form ref={form} onSubmit={onSubmit} noValidate className="auth-form">
        <Field
          id="email"
          label={t("fieldLabel.email")}
          type="email"
          autoComplete="email"
          inputMode="email"
          autoFocus
          value={email}
          onChange={(event) => {
            setEmail(event.target.value);
            clearField("email");
          }}
          error={errors.fields.email}
        />
        <PasswordField
          id="password"
          label={t("fieldLabel.password")}
          autoComplete="current-password"
          value={password}
          onChange={(event) => {
            setPassword(event.target.value);
            clearField("password");
          }}
          error={errors.fields.password}
        />
        {errors.form === null && oidcError ? (
          // Why the provider sign-in came back here; nothing failed on
          // this page, so no alert.
          <p role="status" className="ui-form-error">
            {oidcErrorMessage(oidcError)}
          </p>
        ) : (
          <FormError message={errors.form} />
        )}
        <SubmitButton
          size="lg"
          pending={mutation.isPending}
          pendingLabel={t("login.submitting")}
          cooldown={cooldown}
        >
          {t("login.submit")}
        </SubmitButton>
        <p className="auth-recovery">
          {accountMail === true ? (
            <Link
              to="/forgot-password"
              search={typedEmail ? { email: typedEmail } : {}}
              className="auth-link"
            >
              {t("page.forgotPassword")}
            </Link>
          ) : accountMail === false ? (
            t("login.forgotPasswordNoMail")
          ) : null}
        </p>
      </form>
      {installHint && <InstallDisclosure />}
    </section>
  );
}
