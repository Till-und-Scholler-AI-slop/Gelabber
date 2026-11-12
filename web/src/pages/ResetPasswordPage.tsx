import { useMutation } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { useRef, useState, type FormEvent } from "react";

import { ApiError } from "../api/client.ts";
import { useLinkToken } from "../auth/mail.ts";
import { takePendingRedirect } from "../auth/redirect.ts";
import { PASSWORD_MIN, fieldMessage, validatePassword } from "../auth/rules.ts";
import { resetPassword } from "../auth/session.ts";
import { useFocusOnMount } from "../auth/useFocusOnMount.ts";
import { useFormErrors } from "../auth/useFormErrors.ts";
import { FormError } from "../components/FormError.tsx";
import { SubmitButton } from "../components/SubmitButton.tsx";
import { notify } from "../components/toasts.ts";
import { PasswordField } from "../components/ui/PasswordField.tsx";
import { translate, useT } from "../i18n/catalog.ts";
import { auth } from "../i18n/catalogs/auth.ts";

export function ResetPasswordPage() {
  const t = useT(auth);
  const navigate = useNavigate();
  const token = useLinkToken();
  const [password, setPassword] = useState("");
  const [deadLink, setDeadLink] = useState(token === "");
  const form = useRef<HTMLFormElement>(null);
  const { errors, clearField, setFields, fromError, cooldown } =
    useFormErrors(form);

  const mutation = useMutation({
    mutationFn: () => resetPassword(token, password),
    onSuccess: () => {
      notify(translate(auth, "reset.done"));
      void navigate({ href: takePendingRedirect() ?? "/", replace: true });
    },
    onError: (error) => {
      if (error instanceof ApiError && error.code === "token_invalid") {
        setDeadLink(true);
        return;
      }
      fromError(error);
    },
  });

  const onSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const code = validatePassword(password);
    if (code) {
      setFields({ password: fieldMessage("password", code) });
      return;
    }
    mutation.mutate();
  };

  if (deadLink) return <DeadResetLink />;

  return (
    <section className="lr-auth-card">
      <h1>{t("reset.title")}</h1>
      <p className="auth-lead">{t("reset.lead")}</p>
      <form ref={form} onSubmit={onSubmit} noValidate className="auth-form">
        <PasswordField
          id="password"
          label={t("reset.passwordLabel")}
          autoComplete="new-password"
          autoFocus
          value={password}
          onChange={(event) => {
            setPassword(event.target.value);
            clearField("password");
          }}
          error={errors.fields.password}
          hint={t("page.passwordHint", { min: PASSWORD_MIN })}
        />
        <FormError message={errors.form} />
        <SubmitButton
          size="lg"
          pending={mutation.isPending}
          pendingLabel={t("reset.submitting")}
          cooldown={cooldown}
        >
          {t("reset.submit")}
        </SubmitButton>
      </form>
    </section>
  );
}

function DeadResetLink() {
  const t = useT(auth);
  const heading = useFocusOnMount<HTMLHeadingElement>();
  return (
    <section className="lr-auth-card">
      <h1 ref={heading} tabIndex={-1}>
        {t("page.linkExpired")}
      </h1>
      <div className="auth-result">
        <p>{t("page.linkUsed")}</p>
      </div>
      <div className="auth-actions">
        <Link to="/forgot-password" className="auth-link self-start">
          {t("reset.requestNew")}
        </Link>
      </div>
    </section>
  );
}
