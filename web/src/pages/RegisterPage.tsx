import { useMutation } from "@tanstack/react-query";
import { Link, useNavigate, useSearch } from "@tanstack/react-router";
import { useRef, useState, type FormEvent } from "react";

import { ApiError } from "../api/client.ts";
import {
  isInviteRedirect,
  savePendingRedirect,
  takePendingRedirect,
} from "../auth/redirect.ts";
import {
  PASSWORD_MIN,
  errorMessage,
  fieldMessage,
  validateEmail,
  validateName,
  validatePassword,
} from "../auth/rules.ts";
import { login, register } from "../auth/session.ts";
import { RESEND_COOLDOWN_S, useCooldown } from "../auth/useCooldown.ts";
import { useFocusOnMount } from "../auth/useFocusOnMount.ts";
import { useFormErrors } from "../auth/useFormErrors.ts";
import { Field } from "../components/Field.tsx";
import { FormError } from "../components/FormError.tsx";
import { SubmitButton } from "../components/SubmitButton.tsx";
import { Button } from "../components/ui/Button.tsx";
import { InlineStatus } from "../components/ui/InlineStatus.tsx";
import { PasswordField } from "../components/ui/PasswordField.tsx";
import { useInlineStatus } from "../components/ui/useInlineStatus.ts";
import { InviteContextBanner } from "./AuthLayout.tsx";

export function RegisterPage() {
  const navigate = useNavigate();
  const { redirect } = useSearch({ from: "/auth/register" });
  const [name, setName] = useState("");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const form = useRef<HTMLFormElement>(null);
  const { errors, clearField, setFields, fromError, cooldown } =
    useFormErrors(form);
  const [mailedTo, setMailedTo] = useState<string | null>(null);
  const invited = isInviteRedirect(redirect);

  const mutation = useMutation({
    mutationFn: () => register(email, password, name),
    onSuccess: (user) => {
      // `null`: the server mailed a confirmation link and nobody is signed in.
      if (user === null) {
        // The link opens in a new tab, which only knows the target from here.
        if (redirect) savePendingRedirect(redirect);
        setMailedTo(email.trim());
        return;
      }
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

  if (mailedTo !== null) {
    return (
      <MailSent
        email={mailedTo}
        password={password}
        redirect={redirect}
        onOtherAddress={() => {
          setMailedTo(null);
          // The form mounts again; its address field is the one to fix.
          requestAnimationFrame(() =>
            form.current?.querySelector<HTMLInputElement>("#email")?.focus(),
          );
        }}
      />
    );
  }

  return (
    <section className="lr-auth-card">
      {invited ? <InviteContextBanner /> : null}
      <h1>Konto anlegen</h1>
      <p className="auth-lead">
        Schon registriert?{" "}
        <Link
          to="/login"
          search={redirect ? { redirect } : {}}
          className="auth-link"
        >
          Anmelden
        </Link>
      </p>

      <form ref={form} onSubmit={onSubmit} noValidate className="auth-form">
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
        <PasswordField
          id="password"
          label="Passwort"
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
          size="lg"
          pending={mutation.isPending}
          pendingLabel="Konto wird angelegt…"
          cooldown={cooldown}
        >
          Registrieren
        </SubmitButton>
      </form>
    </section>
  );
}

/**
 * "Postfach prüfen": what happens next, a resend that does not need a
 * detour through the sign-in page, and a way back to fix the address.
 */
export function MailSent({
  email,
  password,
  redirect,
  onOtherAddress,
}: {
  email: string;
  password: string;
  redirect: string | undefined;
  onOtherAddress: () => void;
}) {
  const navigate = useNavigate();
  const heading = useFocusOnMount<HTMLHeadingElement>();
  // The first mail just went out.
  const { seconds, start } = useCooldown(RESEND_COOLDOWN_S);
  const { status, show } = useInlineStatus();

  const resend = useMutation({
    // Signing in to an unconfirmed account sends a fresh link.
    mutationFn: () => login(email, password),
    onSuccess: () => {
      // Confirmed meanwhile, on another device: signed in now, and the
      // parked target is not needed any more.
      takePendingRedirect();
      void navigate({ href: redirect ?? "/", replace: true });
    },
    onError: (error) => {
      if (error instanceof ApiError && error.code === "email_unverified") {
        start(RESEND_COOLDOWN_S);
        show({ tone: "success", message: "Neuer Link ist unterwegs." });
        return;
      }
      if (
        error instanceof ApiError &&
        error.code === "rate_limited" &&
        error.retryAfter
      )
        start(Math.ceil(error.retryAfter));
      show({
        tone: "error",
        message: errorMessage(
          error instanceof ApiError ? error.code : "internal",
        ),
      });
    },
  });

  return (
    <section className="lr-auth-card">
      <h1 ref={heading} tabIndex={-1}>
        Postfach prüfen
      </h1>
      <div className="auth-result">
        <p>
          Wir haben eine Mail an <strong>{email}</strong> geschickt. Öffne den
          Link darin, um dein Konto zu bestätigen. Er gilt 24 Stunden.
        </p>
        {isInviteRedirect(redirect) ? (
          <p>Nach der Bestätigung geht es direkt weiter zur Einladung.</p>
        ) : null}
        <p className="auth-quiet">
          Nichts angekommen? Schau im Spam-Ordner nach oder lass dir den Link
          noch einmal schicken.
        </p>
      </div>
      <div className="auth-actions">
        <Button
          size="lg"
          disabled={seconds > 0}
          pending={resend.isPending}
          pendingLabel="Wird gesendet…"
          onClick={() => resend.mutate()}
        >
          {seconds > 0 ? `Erneut senden (${seconds} s)` : "Link erneut senden"}
        </Button>
        <InlineStatus status={status} />
        <Button variant="ghost" size="lg" onClick={onOtherAddress}>
          Andere Adresse verwenden
        </Button>
      </div>
      <p className="auth-footer">
        <Link
          to="/login"
          search={redirect ? { redirect } : {}}
          className="auth-link"
        >
          Zur Anmeldung
        </Link>
      </p>
    </section>
  );
}
