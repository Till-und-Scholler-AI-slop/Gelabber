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
  oidcErrorMessage,
  validateName,
  validateNewEmail,
  validatePassword,
} from "../auth/rules.ts";
import {
  admissionCodes,
  isAdmissionError,
  showsInvitationField,
  showsInvitationOnly,
  useRegistrationMode,
  validateInvitation,
  withoutInvitationField,
} from "../auth/registration.ts";
import { login, register } from "../auth/session.ts";
import { RESEND_COOLDOWN_S, useCooldown } from "../auth/useCooldown.ts";
import { useFocusOnMount } from "../auth/useFocusOnMount.ts";
import { useFormErrors } from "../auth/useFormErrors.ts";
import { Field } from "../components/Field.tsx";
import { FormError } from "../components/FormError.tsx";
import { ProviderSignIn } from "../components/ProviderButton.tsx";
import { SubmitButton } from "../components/SubmitButton.tsx";
import { Button } from "../components/ui/Button.tsx";
import { InlineStatus } from "../components/ui/InlineStatus.tsx";
import { PasswordField } from "../components/ui/PasswordField.tsx";
import { useInlineStatus } from "../components/ui/useInlineStatus.ts";
import { T, useT } from "../i18n/catalog.ts";
import { auth } from "../i18n/catalogs/auth.ts";
import { InviteContextBanner } from "./AuthLayout.tsx";

export function RegisterPage() {
  const t = useT(auth);
  const navigate = useNavigate();
  const {
    redirect,
    invitation: invitationParam,
    oidc_error: oidcError,
  } = useSearch({ from: "/auth/register" });
  const [name, setName] = useState("");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [invitation, setInvitation] = useState(invitationParam ?? "");
  const mode = useRegistrationMode();
  // A refusal for want of a code brings the field up even before (or
  // without) the mode check.
  const [asked, setAsked] = useState(
    oidcError === "invitation_required" || oidcError === "invitation_invalid",
  );
  const codeState = { mode, invitation: invitationParam, redirect, asked };
  const askCode = showsInvitationField(codeState);
  const form = useRef<HTMLFormElement>(null);
  const { errors, clearField, setFields, fromError, cooldown } =
    useFormErrors(form);
  const [mailedTo, setMailedTo] = useState<string | null>(null);
  const invited = isInviteRedirect(redirect);

  const mutation = useMutation({
    mutationFn: () =>
      register(
        email,
        password,
        name,
        admissionCodes(askCode ? invitation : null, redirect),
      ),
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
    onError: (error) => {
      if (!isAdmissionError(error)) {
        fromError(error);
      } else if (showsInvitationField({ ...codeState, asked: true })) {
        setAsked(true);
        fromError(error);
      } else {
        // A server invite was followed: no field to point at.
        fromError(withoutInvitationField(error));
      }
    },
  });

  const onSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const fields: Record<string, string> = {};
    const checks = [
      ["name", validateName(name)],
      ["email", validateNewEmail(email)],
      ["password", validatePassword(password)],
      [
        "invitation",
        askCode ? validateInvitation(invitation, mode, redirect) : null,
      ],
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
      {/* A refused provider sign-in says why at the form, as on /login. */}
      {!oidcError && showsInvitationOnly(codeState) ? (
        <p role="status" className="auth-banner">
          {t("register.invitationOnly")}
        </p>
      ) : null}
      <h1>{t("register.title")}</h1>
      <p className="auth-lead">
        {t("register.haveAccount")}{" "}
        <Link
          to="/login"
          search={redirect ? { redirect } : {}}
          className="auth-link"
        >
          {t("register.signIn")}
        </Link>
      </p>
      <ProviderSignIn
        redirect={redirect}
        // A new account on an invitation-only instance needs the code too.
        invitation={() =>
          admissionCodes(askCode ? invitation : null, undefined).invitation
        }
        onError={fromError}
      />

      <form ref={form} onSubmit={onSubmit} noValidate className="auth-form">
        <Field
          id="name"
          label={t("fieldLabel.name")}
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
          hint={t("register.nameHint")}
        />
        <Field
          id="email"
          label={t("fieldLabel.email")}
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
          label={t("fieldLabel.password")}
          autoComplete="new-password"
          value={password}
          onChange={(event) => {
            setPassword(event.target.value);
            clearField("password");
          }}
          error={errors.fields.password}
          hint={t("page.passwordHint", { min: PASSWORD_MIN })}
        />
        {askCode ? (
          <Field
            id="invitation"
            label={t("fieldLabel.invitation")}
            type="text"
            autoComplete="off"
            autoCapitalize="none"
            spellCheck={false}
            maxLength={64}
            value={invitation}
            onChange={(event) => {
              setInvitation(event.target.value);
              clearField("invitation");
            }}
            error={errors.fields.invitation}
          />
        ) : null}
        {errors.form === null && oidcError ? (
          // Why the provider sign-in came back here; nothing failed on
          // this page, so no alert.
          <p role="status" className="ui-form-error">
            {oidcErrorMessage(oidcError)}
          </p>
        ) : (
          <FormError message={errors.form} />
        )}
        <SubmitButton
          size="lg"
          pending={mutation.isPending}
          pendingLabel={t("register.submitting")}
          cooldown={cooldown}
        >
          {t("register.submit")}
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
  const t = useT(auth);
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
        show({ tone: "success", message: t("page.newLinkSent") });
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
        {t("page.checkInbox")}
      </h1>
      <div className="auth-result">
        <p>
          <T
            catalog={auth}
            k="register.mailSent"
            params={{ email }}
            tags={{ b: (text) => <strong>{text}</strong> }}
          />
        </p>
        {isInviteRedirect(redirect) ? <p>{t("register.thenInvite")}</p> : null}
        <p className="auth-quiet">{t("page.noMail")}</p>
      </div>
      <div className="auth-actions">
        <Button
          size="lg"
          disabled={seconds > 0}
          pending={resend.isPending}
          pendingLabel={t("page.sending")}
          onClick={() => resend.mutate()}
        >
          {seconds > 0 ? t("page.resendIn", { seconds }) : t("register.resend")}
        </Button>
        <InlineStatus status={status} />
        <Button variant="ghost" size="lg" onClick={onOtherAddress}>
          {t("page.otherAddress")}
        </Button>
      </div>
      <p className="auth-footer">
        <Link
          to="/login"
          search={redirect ? { redirect } : {}}
          className="auth-link"
        >
          {t("page.toSignIn")}
        </Link>
      </p>
    </section>
  );
}
