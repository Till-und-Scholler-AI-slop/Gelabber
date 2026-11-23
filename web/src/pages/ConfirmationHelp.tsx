// The way out for an account whose address was never confirmed (GEL-40):
// a button that mails a new confirmation link, and after a provider refused
// the account, the reset path for someone who forgot the password. Only
// while the instance sends account mail; without it the sign-in page keeps
// the plain refusal.

import { useMutation } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";

import type { FieldCode } from "../api/client.ts";
import {
  sendConfirmationLink,
  type ConfirmationHelpReason,
} from "../auth/confirmationHelp.ts";
import { errorMessage } from "../auth/rules.ts";
import { requestConfirmationMail } from "../auth/session.ts";
import { RESEND_COOLDOWN_S, useCooldown } from "../auth/useCooldown.ts";
import { Button } from "../components/ui/Button.tsx";
import { buttonClass } from "../components/ui/buttonClass.ts";
import { InlineStatus } from "../components/ui/InlineStatus.tsx";
import { useInlineStatus } from "../components/ui/useInlineStatus.ts";
import { useT } from "../i18n/catalog.ts";
import { auth } from "../i18n/catalogs/auth.ts";

export function ConfirmationHelp({
  reason,
  email,
  onInvalidEmail,
}: {
  reason: ConfirmationHelpReason;
  /** As typed in the sign-in form. */
  email: string;
  /** The typed address cannot be used; the form marks its field. */
  onInvalidEmail: (code: FieldCode) => void;
}) {
  const t = useT(auth);
  // A refused password sign-in has just sent a link. Read on mount only:
  // the sign-in page remounts this when the reason changes.
  const { seconds, start } = useCooldown(
    reason === "email_unverified" ? RESEND_COOLDOWN_S : 0,
  );
  const { status, show } = useInlineStatus();
  const send = useMutation({
    mutationFn: () => sendConfirmationLink(email, requestConfirmationMail),
    onSuccess: (outcome) => {
      if (outcome.kind === "invalid") {
        onInvalidEmail(outcome.code);
        return;
      }
      if (outcome.kind === "sent") {
        start(RESEND_COOLDOWN_S);
        show({ tone: "success", message: t("confirmHelp.sent") });
        return;
      }
      if (outcome.retryAfter) start(outcome.retryAfter);
      show({ tone: "error", message: errorMessage(outcome.code) });
    },
  });
  const typed = email.trim();

  return (
    <div className="auth-confirm-help">
      <p className="auth-quiet">
        {reason === "provider"
          ? t("confirmHelp.providerLead")
          : t("page.noMail")}
      </p>
      <Button
        disabled={seconds > 0}
        pending={send.isPending}
        pendingLabel={t("page.sending")}
        onClick={() => send.mutate()}
      >
        {seconds > 0
          ? t("page.resendIn", { seconds })
          : reason === "provider"
            ? t("confirmHelp.send")
            : t("register.resend")}
      </Button>
      <InlineStatus status={status} />
      {reason === "provider" ? (
        <Link
          to="/forgot-password"
          search={typed ? { email: typed } : {}}
          className={buttonClass({ variant: "ghost" })}
        >
          {t("page.forgotPassword")}
        </Link>
      ) : null}
    </div>
  );
}
