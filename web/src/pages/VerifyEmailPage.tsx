import { useMutation } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";

import { ApiError } from "../api/client.ts";
import { useLinkToken } from "../auth/mail.ts";
import { errorMessage } from "../auth/rules.ts";
import { verifyEmail } from "../auth/session.ts";
import { FormError } from "../components/FormError.tsx";
import { SubmitButton } from "../components/SubmitButton.tsx";

const linkClass =
  "font-medium text-neutral-900 dark:text-neutral-100 underline underline-offset-2";

/**
 * Landing page of the confirmation mail. Confirming takes a click: mail
 * scanners that open links on their own must not use the link up.
 */
export function VerifyEmailPage() {
  const navigate = useNavigate();
  const token = useLinkToken();

  const mutation = useMutation({
    mutationFn: () => verifyEmail(token),
    onSuccess: () => {
      void navigate({ to: "/", replace: true });
    },
  });

  const deadLink =
    token === "" ||
    (mutation.error instanceof ApiError &&
      mutation.error.code === "token_invalid");

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
        <div className="mt-6 flex flex-col gap-5">
          <p className="text-sm text-neutral-600 dark:text-neutral-400">
            Ein Klick noch, dann ist dein Konto bereit und du bist angemeldet.
          </p>
          <FormError
            message={
              mutation.error
                ? errorMessage(
                    mutation.error instanceof ApiError
                      ? mutation.error.code
                      : "internal",
                  )
                : null
            }
          />
          <SubmitButton
            pending={mutation.isPending}
            pendingLabel="Wird bestätigt…"
            onClick={() => mutation.mutate()}
          >
            Bestätigen
          </SubmitButton>
        </div>
      )}
    </section>
  );
}
