import { useMutation } from "@tanstack/react-query";
import { Link, useNavigate, useSearch } from "@tanstack/react-router";
import { useState, type FormEvent } from "react";

import {
  PASSWORD_MIN,
  fieldMessage,
  validateEmail,
  validateName,
  validatePassword,
} from "../auth/rules.ts";
import { register } from "../auth/session.ts";
import { useFormErrors } from "../auth/useFormErrors.ts";
import { Field } from "../components/Field.tsx";
import { FormError } from "../components/FormError.tsx";
import { SubmitButton } from "../components/SubmitButton.tsx";

export function RegisterPage() {
  const navigate = useNavigate();
  const { redirect } = useSearch({ from: "/centered/register" });
  const [name, setName] = useState("");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const { errors, clearField, setFields, fromError } = useFormErrors();

  const mutation = useMutation({
    mutationFn: () => register(email, password, name),
    onSuccess: () => {
      void navigate({ href: redirect ?? "/", replace: true });
    },
    onError: fromError,
  });

  const onSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const fields: Record<string, string> = {};
    const checks = [
      ["name", validateName(name)],
      ["email", validateEmail(email)],
      ["password", validatePassword(password)],
    ] as const;
    for (const [field, code] of checks) {
      if (code) fields[field] = fieldMessage(field, code);
    }
    if (Object.keys(fields).length > 0) {
      setFields(fields);
      return;
    }
    mutation.mutate();
  };

  return (
    <section className="mx-auto max-w-sm">
      <h1 className="text-2xl font-semibold tracking-tight">Konto anlegen</h1>
      <p className="mt-1 text-sm text-neutral-600 dark:text-neutral-400">
        Schon registriert?{" "}
        <Link
          to="/login"
          search={redirect ? { redirect } : {}}
          className="font-medium text-neutral-900 dark:text-neutral-100 underline underline-offset-2"
        >
          Anmelden
        </Link>
      </p>

      <form onSubmit={onSubmit} noValidate className="mt-8 flex flex-col gap-5">
        <Field
          id="name"
          label="Name"
          type="text"
          autoComplete="nickname"
          autoFocus
          maxLength={64}
          value={name}
          onChange={(event) => {
            setName(event.target.value);
            clearField("name");
          }}
          error={errors.fields.name}
          hint="So sehen dich andere im Chat."
        />
        <Field
          id="email"
          label="E-Mail-Adresse"
          type="email"
          autoComplete="email"
          inputMode="email"
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
          autoComplete="new-password"
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
          pendingLabel="Konto wird angelegt…"
        >
          Registrieren
        </SubmitButton>
      </form>
    </section>
  );
}
