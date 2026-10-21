import { useMutation } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { useState, type FormEvent } from "react";

import { ApiError } from "../api/client.ts";
import { useLinkToken } from "../auth/mail.ts";
import { PASSWORD_MIN, fieldMessage, validatePassword } from "../auth/rules.ts";
import { resetPassword } from "../auth/session.ts";
import { useFormErrors } from "../auth/useFormErrors.ts";
import { Field } from "../components/Field.tsx";
import { FormError } from "../components/FormError.tsx";
import { SubmitButton } from "../components/SubmitButton.tsx";

const linkClass =
  "font-medium text-neutral-900 dark:text-neutral-100 underline underline-offset-2";

export function ResetPasswordPage() {
  const navigate = useNavigate();
  const token = useLinkToken();
  const [password, setPassword] = useState("");
  const [deadLink, setDeadLink] = useState(token === "");
  const { errors, clearField, setFields, fromError } = useFormErrors();

  const mutation = useMutation({
    mutationFn: () => resetPassword(token, password),
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
    const code = validatePassword(password);
    if (code) {
      setFields({ password: fieldMessage("password", code) });
      return;
    }
    mutation.mutate();
  };

  return (
    <section className="lr-auth-card mx-auto max-w-sm">
      <h1 className="text-2xl font-semibold tracking-tight">Neues Passwort</h1>
      {deadLink ? (
        <div role="alert" className="mt-6 flex flex-col gap-3 text-sm">
          <p>Dieser Link ist abgelaufen oder wurde schon benutzt.</p>
          <p>
            <Link to="/forgot-password" className={linkClass}>
              Neuen Link anfordern
            </Link>
          </p>
        </div>
      ) : (
        <>
          <p className="mt-1 text-sm text-neutral-600 dark:text-neutral-400">
            Danach bist du auf allen anderen Geräten abgemeldet.
          </p>
          <form
            onSubmit={onSubmit}
            noValidate
            className="mt-8 flex flex-col gap-5"
          >
            <Field
              id="password"
              label="Neues Passwort"
              type="password"
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
              pending={mutation.isPending}
              pendingLabel="Wird gespeichert…"
            >
              Passwort speichern
            </SubmitButton>
          </form>
        </>
      )}
    </section>
  );
}
