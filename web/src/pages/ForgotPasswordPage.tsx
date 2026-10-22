import { useMutation } from "@tanstack/react-query";
import { Link, useSearch } from "@tanstack/react-router";
import { useRef, useState, type FormEvent } from "react";

import { ApiError } from "../api/client.ts";
import { errorMessage, fieldMessage, validateEmail } from "../auth/rules.ts";
import { requestPasswordReset } from "../auth/session.ts";
import { RESEND_COOLDOWN_S, useCooldown } from "../auth/useCooldown.ts";
import { useFocusOnMount } from "../auth/useFocusOnMount.ts";
import { useFormErrors } from "../auth/useFormErrors.ts";
import { Field } from "../components/Field.tsx";
import { FormError } from "../components/FormError.tsx";
import { SubmitButton } from "../components/SubmitButton.tsx";
import { Button } from "../components/ui/Button.tsx";
import { InlineStatus } from "../components/ui/InlineStatus.tsx";
import { useInlineStatus } from "../components/ui/useInlineStatus.ts";

export function ForgotPasswordPage() {
  const search = useSearch({ from: "/auth/forgot-password" });
  const [email, setEmail] = useState(search.email ?? "");
  const [sentTo, setSentTo] = useState<string | null>(null);
  const form = useRef<HTMLFormElement>(null);
  const { errors, clearField, setFields, fromError, cooldown } =
    useFormErrors(form);

  const mutation = useMutation({
    mutationFn: () => requestPasswordReset(email),
    onSuccess: () => setSentTo(email.trim()),
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

  if (sentTo !== null) {
    return (
      <ResetMailSent
        email={sentTo}
        onOtherAddress={() => {
          setSentTo(null);
          requestAnimationFrame(() =>
            form.current?.querySelector<HTMLInputElement>("#email")?.focus(),
          );
        }}
      />
    );
  }

  return (
    <section className="lr-auth-card">
      <h1>Passwort vergessen</h1>
      <p className="auth-lead">
        Wir schicken dir einen Link, mit dem du ein neues Passwort festlegst.
      </p>
      <form ref={form} onSubmit={onSubmit} noValidate className="auth-form">
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
          size="lg"
          pending={mutation.isPending}
          pendingLabel="Wird gesendet…"
          cooldown={cooldown}
        >
          Link schicken
        </SubmitButton>
      </form>
      <p className="auth-footer">
        <Link to="/login" className="auth-link">
          Zurück zur Anmeldung
        </Link>
      </p>
    </section>
  );
}

function ResetMailSent({
  email,
  onOtherAddress,
}: {
  email: string;
  onOtherAddress: () => void;
}) {
  const heading = useFocusOnMount<HTMLHeadingElement>();
  // The first mail just went out.
  const { seconds, start } = useCooldown(RESEND_COOLDOWN_S);
  const { status, show } = useInlineStatus();
  const resend = useMutation({
    mutationFn: () => requestPasswordReset(email),
    onSuccess: () => {
      start(RESEND_COOLDOWN_S);
      show({ tone: "success", message: "Neuer Link ist unterwegs." });
    },
    onError: (error) => {
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
          Wenn es zu <strong>{email}</strong> ein Konto gibt, ist jetzt eine
          Mail mit einem Link unterwegs. Er gilt eine Stunde.
        </p>
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
          {seconds > 0 ? `Erneut senden (${seconds} s)` : "Erneut senden"}
        </Button>
        <InlineStatus status={status} />
        <Button variant="ghost" size="lg" onClick={onOtherAddress}>
          Andere Adresse verwenden
        </Button>
      </div>
      <p className="auth-footer">
        <Link to="/login" className="auth-link">
          Zurück zur Anmeldung
        </Link>
      </p>
    </section>
  );
}
