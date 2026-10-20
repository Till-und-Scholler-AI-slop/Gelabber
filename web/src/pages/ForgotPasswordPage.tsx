import { useMutation } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { useState, type FormEvent } from "react";

import { fieldMessage, validateEmail } from "../auth/rules.ts";
import { requestPasswordReset } from "../auth/session.ts";
import { useFormErrors } from "../auth/useFormErrors.ts";
import { Field } from "../components/Field.tsx";
import { FormError } from "../components/FormError.tsx";
import { SubmitButton } from "../components/SubmitButton.tsx";

export function ForgotPasswordPage() {
  const [email, setEmail] = useState("");
  const { errors, clearField, setFields, fromError } = useFormErrors();

  const mutation = useMutation({
    mutationFn: () => requestPasswordReset(email),
    onError: fromError,
  });

  const onSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const code = validateEmail(email);
    if (code) {
      setFields({ email: fieldMessage("email", code) });
      return;
    }
    mutation.mutate();
  };

  return (
    <section className="lr-auth-card mx-auto max-w-sm">
      <h1 className="text-2xl font-semibold tracking-tight">
        Passwort vergessen
      </h1>
      {mutation.isSuccess ? (
        <div role="status" className="mt-6 flex flex-col gap-3 text-sm">
          <p>
            Wenn es zu <strong>{email.trim()}</strong> ein Konto gibt, ist jetzt
            eine Mail mit einem Link unterwegs. Er gilt eine Stunde.
          </p>
          <p className="text-neutral-600 dark:text-neutral-400">
            Nichts angekommen? Schau im Spam-Ordner nach oder versuche es in ein
            paar Minuten noch einmal.
          </p>
        </div>
      ) : (
        <>
          <p className="mt-1 text-sm text-neutral-600 dark:text-neutral-400">
            Wir schicken dir einen Link, mit dem du ein neues Passwort
            festlegst.
          </p>
          <form
            onSubmit={onSubmit}
            noValidate
            className="mt-8 flex flex-col gap-5"
          >
            <Field
              id="email"
              label="E-Mail-Adresse"
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
            <FormError message={errors.form} />
            <SubmitButton
              pending={mutation.isPending}
              pendingLabel="Wird gesendet…"
            >
              Link schicken
            </SubmitButton>
          </form>
        </>
      )}
      <p className="mt-6 text-sm">
        <Link
          to="/login"
          className="font-medium text-neutral-900 dark:text-neutral-100 underline underline-offset-2"
        >
          Zurück zur Anmeldung
        </Link>
      </p>
    </section>
  );
}
