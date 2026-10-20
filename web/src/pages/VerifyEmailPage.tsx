import { useMutation } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { useState, type FormEvent } from "react";

import { ApiError } from "../api/client.ts";
import { useLinkToken } from "../auth/mail.ts";
import { fieldMessage } from "../auth/rules.ts";
import { verifyEmail } from "../auth/session.ts";
import { useFormErrors } from "../auth/useFormErrors.ts";
import { Field } from "../components/Field.tsx";
import { FormError } from "../components/FormError.tsx";
import { SubmitButton } from "../components/SubmitButton.tsx";

const linkClass =
  "font-medium text-neutral-900 dark:text-neutral-100 underline underline-offset-2";

/**
 * Landing page of the confirmation mail. Confirming takes the account's
 * password: the link alone must not hand someone an account a stranger
 * registered with their address, nor get used up by a mail scanner.
 */
export function VerifyEmailPage() {
  const navigate = useNavigate();
  const token = useLinkToken();
  const [password, setPassword] = useState("");
  const [deadLink, setDeadLink] = useState(token === "");
  const { errors, clearField, setFields, fromError } = useFormErrors();

  const mutation = useMutation({
    mutationFn: () => verifyEmail(token, password),
    onSuccess: () => {
      void navigate({ to: "/", replace: true });
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

  return (
    <section className="lr-auth-card mx-auto max-w-sm">
      <h1 className="text-2xl font-semibold tracking-tight">
        E-Mail-Adresse bestätigen
      </h1>
      {deadLink ? (
        <div role="alert" className="mt-6 flex flex-col gap-3 text-sm">
          <p>Dieser Link ist abgelaufen oder wurde schon benutzt.</p>
          <p>
            Melde dich an, dann schicken wir dir einen neuen.{" "}
            <Link to="/login" className={linkClass}>
              Zur Anmeldung
            </Link>
          </p>
        </div>
      ) : (
        <>
          <p className="mt-1 text-sm text-neutral-600 dark:text-neutral-400">
            Gib das Passwort ein, das du bei der Registrierung gewählt hast.
            Dann ist dein Konto bereit und du bist angemeldet.
          </p>
          <form
            onSubmit={onSubmit}
            noValidate
            className="mt-8 flex flex-col gap-5"
          >
            <Field
              id="password"
              label="Passwort"
              type="password"
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
              pending={mutation.isPending}
              pendingLabel="Wird bestätigt…"
            >
              Bestätigen
            </SubmitButton>
            <Link to="/forgot-password" className={`self-start ${linkClass}`}>
              Passwort vergessen?
            </Link>
          </form>
        </>
      )}
    </section>
  );
}
