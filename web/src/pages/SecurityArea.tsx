// Einstellungen → Sicherheit: the section "Anmeldung" with the password and
// one row per sign-in provider (#518), "Angemeldete Geräte" with one row
// per signed-in device and signing out every device, and "Kontodaten" with
// the data export and deleting the account (#519).

import {
  useMutation,
  useQueryClient,
  type QueryClient,
} from "@tanstack/react-query";
import { Link, useNavigate, useSearch } from "@tanstack/react-router";
import {
  useEffect,
  useId,
  useRef,
  useState,
  type FormEvent,
  type ReactNode,
} from "react";

import {
  deleteAndLeave,
  deletionRefusal,
  ownedServers,
  type DeleteConfirmation,
  type OwnedServer,
} from "../account/deletion.ts";
import {
  EXPORT_DOWNLOAD_URL,
  exportKey,
  exportRequestError,
  exportRow,
  requestExport,
  useExport,
} from "../account/export.ts";
import { ApiError } from "../api/client.ts";
import {
  errorMessage,
  fieldMessage,
  oidcErrorMessage,
  validatePassword,
} from "../auth/rules.ts";
import {
  deviceLabel,
  lastActiveLabel,
  sessionsKey,
  signOutDevice,
  useSessions,
  type DeviceSession,
} from "../auth/devices.ts";
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
import { Field } from "../components/Field.tsx";
import { FormError } from "../components/FormError.tsx";
import { LoadError } from "../components/LoadError.tsx";
import { GhostButton, Modal } from "../components/Modal.tsx";
import { SubmitButton } from "../components/SubmitButton.tsx";
import { Button } from "../components/ui/Button.tsx";
import { Badge } from "../components/ui/Badge.tsx";
import { buttonClass } from "../components/ui/buttonClass.ts";
import { InlineStatus } from "../components/ui/InlineStatus.tsx";
import { PasswordField } from "../components/ui/PasswordField.tsx";
import { SkeletonRows } from "../components/ui/Skeleton.tsx";
import {
  useInlineStatus,
  type InlineStatusValue,
} from "../components/ui/useInlineStatus.ts";
import { T, translate, useT } from "../i18n/catalog.ts";
import { account } from "../i18n/catalogs/account.ts";
import { auth } from "../i18n/catalogs/auth.ts";
import { common } from "../i18n/catalogs/common.ts";
import { settings } from "../i18n/catalogs/settings.ts";
import { useServer, useServers } from "../servers/queries.ts";
import { leaveVoice } from "../voice/session.ts";
import "./settings.css";

/** Settings area "Sicherheit". */
export function SecurityArea() {
  const user = useSession((state) => state.user);
  if (!user) return null;
  return <AccountSecurity key={user.id} userId={user.id} email={user.email} />;
}

/** The sign-in methods (#518) and the signed-in devices (#519). */
function AccountSecurity({ userId, email }: { userId: string; email: string }) {
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
      <DevicesSection userId={userId} />
      <AccountDataSection userId={userId} />
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
  badge,
  hint,
  status,
  action,
  children,
}: {
  title: string;
  /** A tag after the title, such as "Dieses Gerät". */
  badge?: ReactNode;
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
          {badge}
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
  const queryClient = useQueryClient();
  const [open, setOpen] = useState(false);
  const { status, show } = useInlineStatus();
  const provider = signInProvider(methods);
  // A new password ends every other session, so the device list is stale.
  const refreshDevices = () =>
    void queryClient.invalidateQueries({ queryKey: sessionsKey(userId) });
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
          onChanged={() => {
            refreshDevices();
            show({ tone: "success", message: t("security.changed") });
          }}
        />
      ) : (
        <SetPasswordDialog
          userId={userId}
          provider={provider}
          open={open}
          onClose={() => setOpen(false)}
          onSet={() => {
            refreshDevices();
            show({
              tone: "success",
              message: t("security.signIn.password.setDone"),
            });
          }}
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

/**
 * The section "Angemeldete Geräte": one row per session in the API's
 * order, then "Überall abmelden", which does not wait for the list.
 */
export function DevicesSection({ userId }: { userId: string }) {
  const t = useT(account);
  const query = useSessions(userId);
  return (
    <section
      className="settings-group settings-action-section"
      aria-labelledby="devices-heading"
    >
      <h3 id="devices-heading" className="settings-group-title">
        {t("devices.title")}
      </h3>
      <ul className="settings-action-list">
        {query.data ? (
          query.data.map((session) => (
            <DeviceRow key={session.id} userId={userId} session={session} />
          ))
        ) : (
          <li className="settings-action-list__state">
            {query.isError ? (
              <LoadError
                message={t("devices.loadFailed")}
                pending={query.isFetching}
                onRetry={() => void query.refetch()}
              />
            ) : (
              <SkeletonRows count={3} />
            )}
          </li>
        )}
        <EverywhereRow />
      </ul>
    </section>
  );
}

function DeviceRow({
  userId,
  session,
}: {
  userId: string;
  session: DeviceSession;
}) {
  const t = useT(account);
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const { status, show } = useInlineStatus();

  const signOut = useMutation({
    mutationFn: () => signOutDevice(session, userId, queryClient),
    onSuccess: (result) => {
      if (result === "signedOutHere") void navigate({ to: "/login" });
    },
    onError: (error) => {
      const message = failureMessage(error);
      if (message) show({ tone: "error", message });
    },
  });

  return (
    <ActionRow
      title={deviceLabel(session)}
      badge={
        session.current ? (
          <Badge tone="accent">{t("devices.current")}</Badge>
        ) : null
      }
      hint={lastActiveLabel(session)}
      status={status}
      action={(describedBy) => (
        <Button
          size="sm"
          aria-describedby={describedBy}
          pending={signOut.isPending && !session.current}
          pendingLabel={t("devices.signingOut")}
          onClick={() => {
            show(null);
            signOut.mutate();
          }}
        >
          {t("devices.signOut")}
        </Button>
      )}
    />
  );
}

/** "Überall abmelden" (issue #188), as the last row of the devices. */
function EverywhereRow() {
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
    <ActionRow
      title={t("security.everywhere.title")}
      hint={t("security.everywhere.hint")}
      status={null}
      action={(describedBy) => (
        <Button
          size="sm"
          variant="danger-outline"
          aria-describedby={describedBy}
          onClick={() => void onLogoutEverywhere()}
        >
          {t("security.everywhere.button")}
        </Button>
      )}
    />
  );
}

/** A row's error line for a failed request; nothing for an abort. */
function failureMessage(error: unknown): string | null {
  if (error instanceof ApiError) return errorMessage(error.code);
  if (error instanceof DOMException && error.name === "AbortError") return null;
  return errorMessage("internal");
}

/** The section "Kontodaten": the data export and deleting the account. */
export function AccountDataSection({ userId }: { userId: string }) {
  const t = useT(account);
  return (
    <section
      className="settings-group settings-action-section"
      aria-labelledby="account-data-heading"
    >
      <h3 id="account-data-heading" className="settings-group-title">
        {t("data.title")}
      </h3>
      <ul className="settings-action-list">
        <ExportRow userId={userId} />
        <DeleteAccountRow userId={userId} />
      </ul>
    </section>
  );
}

/** "Daten exportieren": its hint and button follow the export's state. */
function ExportRow({ userId }: { userId: string }) {
  const t = useT(account);
  const queryClient = useQueryClient();
  const query = useExport(userId);
  const { status, show } = useInlineStatus();

  const request = useMutation({
    mutationFn: requestExport,
    onSuccess: (next) => queryClient.setQueryData(exportKey(userId), next),
    onError: (error) => {
      // Another tab asked already: show that one being built.
      if (error instanceof ApiError && error.code === "export_pending")
        void queryClient.invalidateQueries({ queryKey: exportKey(userId) });
      const message = exportRequestError(error);
      if (message) show({ tone: "error", message });
    },
  });

  if (!query.data) {
    return (
      <li className="settings-action-list__state">
        {query.isError ? (
          <LoadError
            message={t("export.loadFailed")}
            pending={query.isFetching}
            onRetry={() => void query.refetch()}
          />
        ) : (
          <SkeletonRows count={1} />
        )}
      </li>
    );
  }

  const row = exportRow(query.data);
  return (
    <ActionRow
      title={t("export.title")}
      hint={row.hint}
      status={status}
      action={(describedBy) =>
        row.action === "download" ? (
          <a
            href={EXPORT_DOWNLOAD_URL}
            className={buttonClass({ variant: "primary", size: "sm" })}
            aria-describedby={describedBy}
          >
            {t("export.download")}
          </a>
        ) : row.action === "building" ? (
          <Button size="sm" disabled aria-describedby={describedBy}>
            {t("export.building")}
          </Button>
        ) : (
          <Button
            size="sm"
            aria-describedby={describedBy}
            pending={request.isPending}
            pendingLabel={t("export.building")}
            onClick={() => {
              show(null);
              request.mutate();
            }}
          >
            {row.action === "retry" ? t("export.retry") : t("export.request")}
          </Button>
        )
      }
    />
  );
}

function DeleteAccountRow({ userId }: { userId: string }) {
  const t = useT(account);
  const [open, setOpen] = useState(false);
  return (
    <ActionRow
      title={t("delete.title")}
      hint={t("delete.hint")}
      status={null}
      action={(describedBy) => (
        <Button
          size="sm"
          variant="danger-outline"
          aria-describedby={describedBy}
          onClick={() => setOpen(true)}
        >
          {t("delete.title")}
        </Button>
      )}
    >
      <DeleteAccountDialog
        userId={userId}
        open={open}
        onClose={() => setOpen(false)}
      />
    </ActionRow>
  );
}

/**
 * "Konto löschen". While the account owns servers (by the server list, or
 * by the API's `409 owns_servers`, which outranks it) the dialog lists them
 * with a way to their settings. Otherwise it asks for the password, or for
 * the address of an account without one; that path may first want a fresh
 * sign-in through the provider.
 */
export function DeleteAccountDialog({
  userId,
  open,
  onClose,
  initialOwned = null,
  initialReauth = false,
}: {
  userId: string;
  open: boolean;
  onClose: () => void;
  /** Opens with these servers in the way (for tests and previews). */
  initialOwned?: OwnedServer[] | null;
  /** Opens on the "confirm it is you" step (for tests and previews). */
  initialReauth?: boolean;
}) {
  const listed = ownedServers(useServers().data);
  const methods = useSignInMethods(userId).data;
  // Until the methods are known, ask for the password as before sign-in.
  const hasPassword = methods?.password ?? true;
  const provider = methods ? signInProvider(methods) : null;
  const [refused, setRefused] = useState<OwnedServer[] | null>(initialOwned);

  const close = () => {
    setRefused(null);
    onClose();
  };

  const owned = refused ?? (listed.length > 0 ? listed : null);
  if (owned) {
    return <OwnedServersDialog servers={owned} open={open} onClose={close} />;
  }
  return (
    <ConfirmDeleteDialog
      hasPassword={hasPassword}
      provider={provider}
      open={open}
      onClose={close}
      onOwned={setRefused}
      initialReauth={initialReauth}
    />
  );
}

function OwnedServersDialog({
  servers,
  open,
  onClose,
}: {
  servers: OwnedServer[];
  open: boolean;
  onClose: () => void;
}) {
  const t = useT(account);
  const tc = useT(common);
  const leadId = useId();
  return (
    <Modal
      open={open}
      onClose={onClose}
      title={t("delete.title")}
      describedBy={leadId}
    >
      <div className="settings-form">
        <p id={leadId} className="settings-hint">
          {t("delete.owned.lead")}
        </p>
        <ul className="settings-action-list">
          {servers.map((server) => (
            <OwnedServerRow key={server.id} server={server} />
          ))}
        </ul>
        <div className="settings-dialog-actions">
          <Button onClick={onClose}>{tc("close")}</Button>
        </div>
      </div>
    </Modal>
  );
}

/** A server in the way: its name, its members, and its settings. */
function OwnedServerRow({ server }: { server: OwnedServer }) {
  const t = useT(account);
  // The list has no member count; the server's own detail does.
  const detail = useServer(
    server.member_count === undefined ? server.id : undefined,
  ).data;
  const count = server.member_count ?? detail?.members.length;
  return (
    <ActionRow
      title={server.name}
      hint={count === undefined ? "" : t("delete.owned.members", { count })}
      status={null}
      action={(describedBy) => (
        <Link
          to="/s/$serverId/settings"
          params={{ serverId: server.id }}
          hash="server-settings-gefahrenzone"
          className={buttonClass({ size: "sm" })}
          aria-describedby={describedBy}
        >
          {t("delete.owned.settings")}
        </Link>
      )}
    />
  );
}

function ConfirmDeleteDialog({
  hasPassword,
  provider,
  open,
  onClose,
  onOwned,
  initialReauth,
}: {
  hasPassword: boolean;
  provider: AuthProvider | null;
  open: boolean;
  onClose: () => void;
  onOwned: (servers: OwnedServer[]) => void;
  initialReauth: boolean;
}) {
  const t = useT(account);
  const tc = useT(common);
  const ts = useT(settings);
  const ta = useT(auth);
  const navigate = useNavigate();
  const form = useRef<HTMLFormElement>(null);
  const leadId = useId();
  const [value, setValue] = useState("");
  const [reauth, setReauth] = useState(initialReauth);
  const { errors, clear, clearField, setFields, fromError } =
    useFormErrors(form);
  const field = hasPassword ? "password" : "email";

  const close = () => {
    setValue("");
    setReauth(false);
    clear();
    onClose();
  };

  const remove = useMutation({
    mutationFn: (confirmation: DeleteConfirmation) =>
      deleteAndLeave(confirmation, navigate),
    onError: (error) => {
      const refusal = deletionRefusal(error);
      switch (refusal.kind) {
        case "owned":
          clear();
          onOwned(refusal.servers);
          return;
        case "reauth":
          clear();
          setReauth(true);
          return;
        case "emailWrong":
          setFields({ email: t("delete.emailWrong") });
          return;
        case "other":
          fromError(error);
      }
    },
  });

  const confirm = useMutation({
    mutationFn: (id: string) =>
      startOidc(id, { purpose: "sign_in", redirect: SECURITY_PATH }),
    onError: fromError,
  });

  const onSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (value.trim().length === 0) {
      setFields({ [field]: fieldMessage(field, "required") });
      return;
    }
    remove.mutate(hasPassword ? { password: value } : { email: value.trim() });
  };

  const onChange = (next: string) => {
    setValue(next);
    clearField(field);
  };

  return (
    <Modal
      open={open}
      onClose={close}
      title={t("delete.confirm.title")}
      describedBy={leadId}
      dirty={value !== ""}
    >
      <form ref={form} onSubmit={onSubmit} noValidate className="settings-form">
        <p id={leadId} className="settings-hint">
          {t("delete.confirm.lead")}
        </p>
        <p className="settings-hint">{t("delete.confirm.exportHint")}</p>
        {hasPassword ? (
          <PasswordField
            id="delete_password"
            label={t("delete.password")}
            autoComplete="current-password"
            value={value}
            onChange={(event) => onChange(event.target.value)}
            error={errors.fields.password}
          />
        ) : (
          <Field
            id="delete_email"
            label={t("delete.email")}
            type="email"
            autoComplete="email"
            inputMode="email"
            value={value}
            onChange={(event) => onChange(event.target.value)}
            error={errors.fields.email}
            hint={t("delete.emailHint")}
          />
        )}
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
                {ts("security.signIn.reauth", { provider: provider.name })}
              </Button>
            ) : null}
          </div>
        ) : null}
        <FormError message={errors.form} />
        <div className="settings-dialog-actions">
          <GhostButton onClick={close}>{tc("cancel")}</GhostButton>
          <Button
            type="submit"
            variant="danger"
            pending={remove.isPending}
            pendingLabel={t("delete.deleting")}
          >
            {t("delete.title")}
          </Button>
        </div>
      </form>
    </Modal>
  );
}
