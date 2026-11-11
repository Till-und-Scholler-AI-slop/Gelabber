// Einstellungen → Sicherheit: the section "Anmeldung" with the password and
// one row per sign-in provider (#518), and signing out every device.

import {
  useMutation,
  useQueryClient,
  type QueryClient,
} from "@tanstack/react-query";
import { useNavigate, useSearch } from "@tanstack/react-router";
import {
  useEffect,
  useId,
  useRef,
  useState,
  type FormEvent,
  type ReactNode,
} from "react";

import { ApiError } from "../api/client.ts";
import {
  errorMessage,
  fieldMessage,
  oidcErrorMessage,
  validatePassword,
} from "../auth/rules.ts";
import {
  changePassword,
  disconnectProvider,
  logoutEverywhere,
  setPassword,
  startOidc,
  useSession,
} from "../auth/session.ts";
import {
  SECURITY_PATH,
  oidcErrorRows,
  signInFeedback,
  signInMethodsKey,
  disconnectConfirm,
  signInProvider,
  needsReauth,
  useSignInMethods,
  withPassword,
  withoutFeedback,
  type SignInFeedback,
} from "../auth/signInMethods.ts";
import type { AuthProvider, SignInMethods } from "../auth/types.ts";
import { useFormErrors } from "../auth/useFormErrors.ts";
import { confirmAction } from "../components/confirm.ts";
import { FormError } from "../components/FormError.tsx";
import { LoadError } from "../components/LoadError.tsx";
import { GhostButton, Modal } from "../components/Modal.tsx";
import { SubmitButton } from "../components/SubmitButton.tsx";
import { Button } from "../components/ui/Button.tsx";
import { InlineStatus } from "../components/ui/InlineStatus.tsx";
import { PasswordField } from "../components/ui/PasswordField.tsx";
import { SkeletonRows } from "../components/ui/Skeleton.tsx";
import {
  useInlineStatus,
  type InlineStatusValue,
} from "../components/ui/useInlineStatus.ts";
import { T, translate, useT } from "../i18n/catalog.ts";
import { auth } from "../i18n/catalogs/auth.ts";
import { common } from "../i18n/catalogs/common.ts";
import { settings } from "../i18n/catalogs/settings.ts";
import { leaveVoice } from "../voice/session.ts";
import "./settings.css";

/** Settings area "Sicherheit". */
export function SecurityArea() {
  const user = useSession((state) => state.user);
  if (!user) return null;
  return <AccountSecurity key={user.id} userId={user.id} email={user.email} />;
}

/**
 * The sign-in methods (#518) and signing out every device (issue #188).
 */
function AccountSecurity({ userId, email }: { userId: string; email: string }) {
  const t = useT(settings);
  const navigate = useNavigate();

  const onLogoutEverywhere = async () => {
    if (
      !(await confirmAction({
        title: translate(settings, "security.everywhere.confirmTitle"),
        message: translate(settings, "security.everywhere.confirmMessage"),
        confirmLabel: translate(settings, "security.everywhere.confirmLabel"),
      }))
    )
      return;
    leaveVoice();
    void logoutEverywhere();
    void navigate({ to: "/login" });
  };

  return (
    <>
      <p className="settings-hint">
        <T
          catalog={settings}
          k="security.signedInAs"
          params={{ email }}
          tags={{
            b: (text) => <strong className="settings-strong">{text}</strong>,
          }}
        />
      </p>
      <SignInSection userId={userId} />

      <section
        className="settings-group settings-danger"
        aria-labelledby="logout-everywhere-heading"
      >
        <h3 id="logout-everywhere-heading" className="settings-group-title">
          {t("security.everywhere.title")}
        </h3>
        <p className="settings-hint">{t("security.everywhere.hint")}</p>
        <div className="settings-actions">
          <Button variant="danger-outline" onClick={onLogoutEverywhere}>
            {t("security.everywhere.button")}
          </Button>
        </div>
      </section>
    </>
  );
}

/** The section "Anmeldung", with the rows once the methods are known. */
export function SignInSection({ userId }: { userId: string }) {
  const t = useT(settings);
  const query = useSignInMethods(userId);
  const feedback = useSignInFeedback();
  return (
    <section
      className="settings-group settings-action-section"
      aria-labelledby="sign-in-heading"
    >
      <h3 id="sign-in-heading" className="settings-group-title">
        {t("security.signIn.title")}
      </h3>
      {query.data ? (
        <SignInRows userId={userId} methods={query.data} feedback={feedback} />
      ) : query.isError ? (
        <LoadError
          message={t("security.signIn.loadFailed")}
          pending={query.isFetching}
          onRetry={() => void query.refetch()}
        />
      ) : (
        <SkeletonRows count={2} />
      )}
    </section>
  );
}

/**
 * What a provider flow reported on arrival. It is read once and the
 * parameters leave the address, so a reload or a later visit from the
 * history does not show it again.
 */
function useSignInFeedback(): SignInFeedback {
  const search = useSearch({ strict: false });
  const navigate = useNavigate();
  const [feedback] = useState(() => signInFeedback(search));
  const reported = "connected" in search || "oidc_error" in search;
  useEffect(() => {
    if (!reported) return;
    void navigate({
      to: ".",
      search: withoutFeedback,
      replace: true,
      resetScroll: false,
    });
  }, [reported, navigate]);
  return feedback;
}

export function SignInRows({
  userId,
  methods,
  feedback = {},
}: {
  userId: string;
  methods: SignInMethods;
  feedback?: SignInFeedback;
}) {
  const t = useT(settings);
  const errorRows = feedback.oidcError ? oidcErrorRows(methods) : [];
  return (
    <ul className="settings-action-list">
      <PasswordRow userId={userId} methods={methods} />
      {methods.providers.map((provider) => (
        <ProviderRow
          key={provider.id}
          userId={userId}
          provider={provider}
          hasPassword={methods.password}
          feedback={
            feedback.connected === provider.id
              ? {
                  tone: "success",
                  message: t("security.signIn.provider.justConnected", {
                    provider: provider.name,
                  }),
                }
              : feedback.oidcError && errorRows.includes(provider.id)
                ? {
                    tone: "error",
                    message: oidcErrorMessage(feedback.oidcError),
                  }
                : null
          }
        />
      ))}
    </ul>
  );
}

/** Title and hint on the left, the row's button on the right. */
function ActionRow({
  title,
  hint,
  status,
  action,
  children,
}: {
  title: string;
  hint: string;
  status: InlineStatusValue;
  action: (describedBy: string) => ReactNode;
  children?: ReactNode;
}) {
  const id = useId();
  return (
    <li className="settings-action-row">
      <div className="settings-action-row__text">
        <span id={`${id}-title`} className="settings-action-row__title">
          {title}
        </span>
        <span id={`${id}-hint`} className="settings-hint">
          {hint}
        </span>
        <InlineStatus status={status} />
      </div>
      {action(`${id}-title ${id}-hint`)}
      {children}
    </li>
  );
}

function PasswordRow({
  userId,
  methods,
}: {
  userId: string;
  methods: SignInMethods;
}) {
  const t = useT(settings);
  const [open, setOpen] = useState(false);
  const { status, show } = useInlineStatus();
  const provider = signInProvider(methods);
  const hint = methods.password
    ? t("security.signIn.password.hint")
    : provider
      ? t("security.signIn.password.none", { provider: provider.name })
      : t("security.signIn.password.unset");
  return (
    <ActionRow
      title={t("security.signIn.password")}
      hint={hint}
      status={status}
      action={(describedBy) => (
        <Button
          size="sm"
          aria-describedby={describedBy}
          onClick={() => {
            show(null);
            setOpen(true);
          }}
        >
          {methods.password
            ? t("security.signIn.password.change")
            : t("security.signIn.password.set")}
        </Button>
      )}
    >
      {methods.password ? (
        <ChangePasswordDialog
          open={open}
          onClose={() => setOpen(false)}
          onChanged={() =>
            show({ tone: "success", message: t("security.changed") })
          }
        />
      ) : (
        <SetPasswordDialog
          userId={userId}
          provider={provider}
          open={open}
          onClose={() => setOpen(false)}
          onSet={() =>
            show({
              tone: "success",
              message: t("security.signIn.password.setDone"),
            })
          }
        />
      )}
    </ActionRow>
  );
}

/** "Ändern": today's current and new password form, in a dialog. */
export function ChangePasswordDialog({
  open,
  onClose,
  onChanged,
}: {
  open: boolean;
  onClose: () => void;
  onChanged: () => void;
}) {
  const t = useT(settings);
  const tc = useT(common);
  const form = useRef<HTMLFormElement>(null);
  const [currentPassword, setCurrentPassword] = useState("");
  const [password, setNewPassword] = useState("");
  const { errors, clear, clearField, setFields, fromError } =
    useFormErrors(form);

  const close = () => {
    setCurrentPassword("");
    setNewPassword("");
    clear();
    onClose();
  };

  const mutation = useMutation({
    mutationFn: () => changePassword(currentPassword, password),
    onSuccess: () => {
      close();
      onChanged();
    },
    onError: fromError,
  });

  const onSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const fields: Record<string, string> = {};
    if (currentPassword.length === 0)
      fields.current_password = fieldMessage("current_password", "required");
    const passwordCode = validatePassword(password);
    if (passwordCode) fields.password = fieldMessage("password", passwordCode);
    if (Object.keys(fields).length > 0) {
      setFields(fields);
      return;
    }
    mutation.mutate();
  };

  return (
    <Modal
      open={open}
      onClose={close}
      title={t("security.change")}
      dirty={currentPassword !== "" || password !== ""}
    >
      <form ref={form} onSubmit={onSubmit} noValidate className="settings-form">
        <PasswordField
          id="current_password"
          label={t("security.currentPassword")}
          autoComplete="current-password"
          value={currentPassword}
          onChange={(event) => {
            setCurrentPassword(event.target.value);
            clearField("current_password");
          }}
          error={errors.fields.current_password}
        />
        <PasswordField
          id="new_password"
          label={t("security.newPassword")}
          autoComplete="new-password"
          value={password}
          onChange={(event) => {
            setNewPassword(event.target.value);
            clearField("password");
          }}
          error={errors.fields.password}
          hint={t("security.newPasswordHint")}
        />
        <FormError message={errors.form} />
        <div className="settings-dialog-actions">
          <GhostButton onClick={close}>{tc("cancel")}</GhostButton>
          <SubmitButton
            pending={mutation.isPending}
            pendingLabel={t("security.changing")}
          >
            {t("security.change")}
          </SubmitButton>
        </div>
      </form>
    </Modal>
  );
}

/**
 * "Festlegen": the first password of an account that signs in through a
 * provider. The API wants a recent sign-in for it; when it says so, the
 * dialog offers to sign in again through that provider.
 */
export function SetPasswordDialog({
  userId,
  provider,
  open,
  onClose,
  onSet,
  initialReauth = false,
}: {
  userId: string;
  provider: AuthProvider | null;
  open: boolean;
  onClose: () => void;
  onSet: () => void;
  /** Opens on the "confirm it is you" step (for tests and previews). */
  initialReauth?: boolean;
}) {
  const t = useT(settings);
  const tc = useT(common);
  const ta = useT(auth);
  const queryClient = useQueryClient();
  const form = useRef<HTMLFormElement>(null);
  const [password, setNewPassword] = useState("");
  const [reauth, setReauth] = useState(initialReauth);
  const { errors, clear, clearField, setFields, fromError } =
    useFormErrors(form);

  const close = () => {
    setNewPassword("");
    setReauth(false);
    clear();
    onClose();
  };

  const mutation = useMutation({
    mutationFn: () => setPassword(password),
    onSuccess: () => {
      markPasswordSet(queryClient, userId);
      close();
      onSet();
    },
    onError: (error) => {
      if (needsReauth(error)) {
        clear();
        setReauth(true);
        return;
      }
      fromError(error);
    },
  });

  const confirm = useMutation({
    mutationFn: (id: string) =>
      startOidc(id, { purpose: "sign_in", redirect: SECURITY_PATH }),
    onError: fromError,
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
    <Modal
      open={open}
      onClose={close}
      title={t("security.signIn.password.setTitle")}
      dirty={password !== ""}
    >
      <form ref={form} onSubmit={onSubmit} noValidate className="settings-form">
        <PasswordField
          id="new_password"
          label={t("security.newPassword")}
          autoComplete="new-password"
          value={password}
          onChange={(event) => {
            setNewPassword(event.target.value);
            clearField("password");
          }}
          error={errors.fields.password}
          hint={t("security.newPasswordHint")}
        />
        {reauth ? (
          <div role="alert" className="settings-row-note">
            <p>{errorMessage("reauth_required")}</p>
            {provider ? (
              <Button
                size="sm"
                className="settings-signin-reauth"
                pending={confirm.isPending}
                pendingLabel={ta("oidc.redirecting", {
                  provider: provider.name,
                })}
                onClick={() => confirm.mutate(provider.id)}
              >
                {t("security.signIn.reauth", { provider: provider.name })}
              </Button>
            ) : null}
          </div>
        ) : null}
        <FormError message={errors.form} />
        <div className="settings-dialog-actions">
          <GhostButton onClick={close}>{tc("cancel")}</GhostButton>
          <SubmitButton
            pending={mutation.isPending}
            pendingLabel={t("security.signIn.password.setting")}
          >
            {t("security.signIn.password.setSubmit")}
          </SubmitButton>
        </div>
      </form>
    </Modal>
  );
}

function markPasswordSet(queryClient: QueryClient, userId: string) {
  queryClient.setQueryData<SignInMethods>(
    signInMethodsKey(userId),
    withPassword,
  );
}

type ProviderMethod = SignInMethods["providers"][number];

function ProviderRow({
  userId,
  provider,
  hasPassword,
  feedback,
}: {
  userId: string;
  provider: ProviderMethod;
  hasPassword: boolean;
  /** What the last link flow reported, until this row does something. */
  feedback: InlineStatusValue;
}) {
  const t = useT(settings);
  const ta = useT(auth);
  const queryClient = useQueryClient();
  const [dialog, setDialog] = useState(false);
  const [acted, setActed] = useState(false);
  const { status, show } = useInlineStatus();
  const report = (next: InlineStatusValue) => {
    setActed(true);
    show(next);
  };

  const disconnect = useMutation({
    mutationFn: () => disconnectProvider(provider.id),
    onSuccess: (methods) => {
      queryClient.setQueryData(signInMethodsKey(userId), methods);
      report({
        tone: "success",
        message: t("security.signIn.provider.disconnected"),
      });
    },
    onError: (error) => {
      const message = failureMessage(error);
      if (message) report({ tone: "error", message });
    },
  });

  // Without a password the provider is left at once: there is nothing
  // to confirm with.
  const connect = useMutation({
    mutationFn: () => startOidc(provider.id, { purpose: "link" }),
    onError: (error) => {
      const message = failureMessage(error);
      if (message) report({ tone: "error", message });
    },
  });

  const onDisconnect = async () => {
    report(null);
    if (!(await confirmAction(disconnectConfirm(provider.name)))) return;
    disconnect.mutate();
  };

  const onConnect = () => {
    report(null);
    if (hasPassword) setDialog(true);
    else connect.mutate();
  };

  return (
    <ActionRow
      title={provider.name}
      hint={
        provider.connected
          ? t("security.signIn.provider.connected", {
              email: provider.connected.email,
            })
          : t("security.signIn.provider.notConnected")
      }
      status={acted ? status : feedback}
      action={(describedBy) =>
        provider.connected ? (
          <Button
            size="sm"
            aria-describedby={describedBy}
            disabled={!hasPassword}
            pending={disconnect.isPending}
            pendingLabel={t("security.signIn.provider.disconnecting")}
            onClick={() => void onDisconnect()}
          >
            {t("security.signIn.provider.disconnect")}
          </Button>
        ) : provider.configured ? (
          <Button
            size="sm"
            aria-describedby={describedBy}
            pending={connect.isPending}
            pendingLabel={ta("oidc.redirecting", { provider: provider.name })}
            onClick={onConnect}
          >
            {t("security.signIn.provider.connect")}
          </Button>
        ) : null
      }
    >
      {hasPassword && !provider.connected ? (
        <ConnectProviderDialog
          provider={provider}
          open={dialog}
          onClose={() => setDialog(false)}
        />
      ) : null}
    </ActionRow>
  );
}

/**
 * "Verbinden" on an account with a password: the API wants it before a
 * provider account is attached, then the page leaves for the provider.
 */
export function ConnectProviderDialog({
  provider,
  open,
  onClose,
}: {
  provider: AuthProvider;
  open: boolean;
  onClose: () => void;
}) {
  const t = useT(settings);
  const tc = useT(common);
  const ta = useT(auth);
  const form = useRef<HTMLFormElement>(null);
  const leadId = useId();
  const [currentPassword, setCurrentPassword] = useState("");
  const { errors, clear, clearField, setFields, fromError } =
    useFormErrors(form);
  const params = { provider: provider.name };

  const close = () => {
    setCurrentPassword("");
    clear();
    onClose();
  };

  const mutation = useMutation({
    mutationFn: () =>
      startOidc(provider.id, { purpose: "link", currentPassword }),
    onError: fromError,
  });

  const onSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (currentPassword.length === 0) {
      setFields({
        current_password: fieldMessage("current_password", "required"),
      });
      return;
    }
    mutation.mutate();
  };

  return (
    <Modal
      open={open}
      onClose={close}
      title={t("security.signIn.connect.title", params)}
      describedBy={leadId}
      dirty={currentPassword !== ""}
    >
      <form ref={form} onSubmit={onSubmit} noValidate className="settings-form">
        <p id={leadId} className="settings-hint">
          {t("security.signIn.connect.lead", params)}
        </p>
        <PasswordField
          id="link_current_password"
          label={t("security.currentPassword")}
          autoComplete="current-password"
          value={currentPassword}
          onChange={(event) => {
            setCurrentPassword(event.target.value);
            clearField("current_password");
          }}
          error={errors.fields.current_password}
        />
        <FormError message={errors.form} />
        <div className="settings-dialog-actions">
          <GhostButton onClick={close}>{tc("cancel")}</GhostButton>
          <SubmitButton
            pending={mutation.isPending}
            pendingLabel={ta("oidc.redirecting", params)}
          >
            {t("security.signIn.connect.submit", params)}
          </SubmitButton>
        </div>
      </form>
    </Modal>
  );
}

/** A row's error line for a failed request; nothing for an abort. */
function failureMessage(error: unknown): string | null {
  if (error instanceof ApiError) return errorMessage(error.code);
  if (error instanceof DOMException && error.name === "AbortError") return null;
  return errorMessage("internal");
}
