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

export function ResetPasswordPage() {
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
      notify("Passwort geändert. Andere Geräte sind abgemeldet.");
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
      <h1>Neues Passwort</h1>
      <p className="auth-lead">
        Danach bist du auf allen anderen Geräten abgemeldet.
      </p>
      <form ref={form} onSubmit={onSubmit} noValidate className="auth-form">
        <PasswordField
          id="password"
          label="Neues Passwort"
          autoComplete="new-password"
          autoFocus
          value={password}
          onChange={(event) => {
            setPassword(event.target.value);
            clearField("password");
          }}
          error={errors.fields.password}
          hint={`Mindestens ${PASSWORD_MIN} Zeichen.`}
        />
        <FormError message={errors.form} />
        <SubmitButton
          size="lg"
          pending={mutation.isPending}
          pendingLabel="Wird gespeichert…"
          cooldown={cooldown}
        >
          Passwort speichern
        </SubmitButton>
      </form>
    </section>
  );
}

function DeadResetLink() {
  const heading = useFocusOnMount<HTMLHeadingElement>();
  return (
    <section className="lr-auth-card">
      <h1 ref={heading} tabIndex={-1}>
        Link abgelaufen
      </h1>
      <div className="auth-result">
        <p>Dieser Link ist abgelaufen oder wurde schon benutzt.</p>
      </div>
      <div className="auth-actions">
        <Link to="/forgot-password" className="auth-link self-start">
          Neuen Link anfordern
        </Link>
      </div>
    </section>
  );
}
