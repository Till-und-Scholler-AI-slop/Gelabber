import { useMutation } from "@tanstack/react-query";
import { Link, useNavigate, useSearch } from "@tanstack/react-router";
import { useState, type FormEvent } from "react";

import { fieldMessage, validateEmail } from "../auth/rules.ts";
import { login } from "../auth/session.ts";
import { useFormErrors } from "../auth/useFormErrors.ts";
import { Field } from "../components/Field.tsx";
import { FormError } from "../components/FormError.tsx";
import { SubmitButton } from "../components/SubmitButton.tsx";

export function LoginPage() {
  const navigate = useNavigate();
  const { redirect } = useSearch({ from: "/centered/login" });
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const { errors, clearField, setFields, fromError } = useFormErrors();

  const mutation = useMutation({
    mutationFn: () => login(email, password),
    onSuccess: () => {
      // The session store already holds the user; this is a client-side
      // route change, nothing reloads.
      void navigate({ href: redirect ?? "/", replace: true });
    },
    onError: fromError,
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

  return (
    <section className="mx-auto max-w-sm">
      <h1 className="text-2xl font-semibold tracking-tight">Anmelden</h1>
      <p className="mt-1 text-sm text-neutral-600">
        Noch kein Konto?{" "}
        <Link
          to="/register"
          search={redirect ? { redirect } : {}}
          className="font-medium text-neutral-900 underline underline-offset-2"
        >
          Registrieren
        </Link>
      </p>

      <form onSubmit={onSubmit} noValidate className="mt-8 flex flex-col gap-5">
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
        <Field
          id="password"
          label="Passwort"
          type="password"
          autoComplete="current-password"
          value={password}
          onChange={(event) => {
            setPassword(event.target.value);
            clearField("password");
          }}
          error={errors.fields.password}
        />
        <FormError message={errors.form} />
        <SubmitButton pending={mutation.isPending} pendingLabel="Anmelden…">
          Anmelden
        </SubmitButton>
      </form>
    </section>
  );
}
