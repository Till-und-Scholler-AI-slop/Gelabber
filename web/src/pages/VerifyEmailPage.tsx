import { useMutation } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { useRef, useState, type FormEvent } from "react";

import { ApiError } from "../api/client.ts";
import { useLinkToken } from "../auth/mail.ts";
import { takePendingRedirect } from "../auth/redirect.ts";
import { fieldMessage } from "../auth/rules.ts";
import { verifyEmail } from "../auth/session.ts";
import { useFocusOnMount } from "../auth/useFocusOnMount.ts";
import { useFormErrors } from "../auth/useFormErrors.ts";
import { FormError } from "../components/FormError.tsx";
import { SubmitButton } from "../components/SubmitButton.tsx";
import { notify } from "../components/toasts.ts";
import { PasswordField } from "../components/ui/PasswordField.tsx";

/**
 * Landing page of the confirmation mail. Confirming takes the account's
 * password: the link alone must not hand someone an account a stranger
 * registered with their address, nor get used up by a mail scanner.
 * Afterwards it continues to the invite the account was made for, if any.
 */
export function VerifyEmailPage() {
  const navigate = useNavigate();
  const token = useLinkToken();
  const [password, setPassword] = useState("");
  const [deadLink, setDeadLink] = useState(token === "");
  const form = useRef<HTMLFormElement>(null);
  const { errors, clearField, setFields, fromError, cooldown } =
    useFormErrors(form);

  const mutation = useMutation({
    mutationFn: () => verifyEmail(token, password),
    onSuccess: () => {
      notify("E-Mail-Adresse bestätigt.");
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
    if (password.length === 0) {
      setFields({ password: fieldMessage("password", "required") });
      return;
    }
    mutation.mutate();
  };

  if (deadLink) return <DeadVerifyLink />;

  return (
    <section className="lr-auth-card">
      <h1>E-Mail-Adresse bestätigen</h1>
      <p className="auth-lead">Gib zur Bestätigung dein Passwort ein.</p>
      <form ref={form} onSubmit={onSubmit} noValidate className="auth-form">
        <PasswordField
          id="password"
          label="Passwort"
          autoComplete="current-password"
          autoFocus
          value={password}
          onChange={(event) => {
            setPassword(event.target.value);
            clearField("password");
          }}
          error={errors.fields.password}
        />
        <FormError message={errors.form} />
        <SubmitButton
          size="lg"
          pending={mutation.isPending}
          pendingLabel="Wird bestätigt…"
          cooldown={cooldown}
        >
          Bestätigen
        </SubmitButton>
        <p className="auth-recovery">
          <Link to="/forgot-password" className="auth-link">
            Passwort vergessen?
          </Link>
        </p>
      </form>
    </section>
  );
}

function DeadVerifyLink() {
  const heading = useFocusOnMount<HTMLHeadingElement>();
  return (
    <section className="lr-auth-card">
      <h1 ref={heading} tabIndex={-1}>
        Link abgelaufen
      </h1>
      <div className="auth-result">
        <p>Dieser Link ist abgelaufen oder wurde schon benutzt.</p>
        <p className="auth-quiet">Melde dich an, um einen neuen zu erhalten.</p>
      </div>
      <div className="auth-actions">
        <Link to="/login" className="auth-link self-start">
          Zur Anmeldung
        </Link>
      </div>
    </section>
  );
}
