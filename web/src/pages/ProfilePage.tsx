import { useMutation } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import {
  useId,
  useRef,
  useState,
  type ChangeEvent,
  type FormEvent,
} from "react";

import {
  AvatarImageError,
  avatarImageMessage,
  prepareAvatar,
} from "../auth/avatarImage.ts";
import {
  avatarFieldAfterRemoval,
  fieldMessage,
  isUploadedAvatar,
  profileFormPatch,
  removesAvatarAtOnce,
  validateAvatarUrl,
  validateName,
  validatePassword,
} from "../auth/rules.ts";
import {
  changePassword,
  claimProfileWrite,
  logoutEverywhere,
  updateProfile,
  uploadAvatar,
  useProfileWrites,
  useSession,
} from "../auth/session.ts";
import type { User } from "../auth/types.ts";
import { useFormErrors } from "../auth/useFormErrors.ts";
import { Avatar } from "../components/Avatar.tsx";
import { confirmAction } from "../components/confirm.ts";
import { Field } from "../components/Field.tsx";
import { FormError } from "../components/FormError.tsx";
import { SubmitButton } from "../components/SubmitButton.tsx";
import { Button } from "../components/ui/Button.tsx";
import { InlineStatus } from "../components/ui/InlineStatus.tsx";
import { PasswordField } from "../components/ui/PasswordField.tsx";
import { useDebouncedValue } from "../components/useDebouncedValue.ts";
import { leaveVoice } from "../voice/session.ts";
import { ProfileCardForm } from "./ProfileCardForm.tsx";
import "./settings.css";

/** Settings area "Profil": name and picture. */
export function ProfileArea() {
  const user = useSession((state) => state.user);
  // The route guard guarantees a user; the fallback keeps the type narrow.
  if (!user) return null;
  return <ProfileForm key={user.id} user={user} />;
}

/** Settings area "Profilkarte". */
export function ProfileCardArea() {
  const user = useSession((state) => state.user);
  if (!user) return null;
  return <ProfileCardForm key={user.id} user={user} />;
}

/** Settings area "Sicherheit". */
export function SecurityArea() {
  const user = useSession((state) => state.user);
  if (!user) return null;
  return <AccountSecurity key={user.id} email={user.email} />;
}

type Upload = { file: File; release: () => void };

function ProfileForm({ user }: { user: User }) {
  const [name, setName] = useState(user.name);
  const [avatarUrl, setAvatarUrl] = useState(user.avatar_url ?? "");
  const [saved, setSaved] = useState(false);
  const { errors, clearField, setFields, fromError } = useFormErrors();

  // Every write below holds the tab-wide profile write slot (claimed by the
  // handler, released when the write settles, even after an unmount).
  const mutation = useMutation({
    mutationFn: async (release: () => void) => {
      try {
        return await updateProfile(
          profileFormPatch(name, avatarUrl, user.avatar_url),
        );
      } finally {
        release();
      }
    },
    onSuccess: (updated) => {
      setName(updated.name);
      setAvatarUrl(updated.avatar_url ?? "");
      setSaved(true);
    },
    onError: fromError,
  });

  const dirty =
    name.trim() !== user.name || avatarUrl.trim() !== (user.avatar_url ?? "");
  const unchangedId = useId();

  // Uploads apply at once; the URL field and Speichern are for links.
  const picker = useRef<HTMLInputElement>(null);
  const upload = useMutation({
    mutationFn: async ({ file, release }: Upload) => {
      try {
        return await uploadAvatar(await prepareAvatar(file));
      } finally {
        release();
      }
    },
    onSuccess: (updated) => {
      setAvatarUrl(updated.avatar_url ?? "");
      clearField("avatar_url");
      setSaved(true);
    },
    onError: (error) => {
      if (error instanceof AvatarImageError) {
        setFields({ avatar_url: avatarImageMessage(error) });
      } else {
        fromError(error);
      }
    },
  });
  // Removing an uploaded image applies at once, like the upload; a link is
  // only cleared in the form until Speichern.
  const removal = useMutation({
    mutationFn: async (release: () => void) => {
      try {
        return await updateProfile({ avatar_url: "" });
      } finally {
        release();
      }
    },
    onSuccess: () => {
      setAvatarUrl(avatarFieldAfterRemoval);
      clearField("avatar_url");
      setSaved(true);
    },
    onError: fromError,
  });
  // One profile write at a time, across remounts of this form: a late or
  // failed answer of one would otherwise put an older user back into the
  // session store.
  const busy = useProfileWrites((state) => state.pending > 0);
  // Upload and removal save at once, so after them it says "Gespeichert.".
  const unchanged = !dirty && !saved && !busy;
  const onPick = (event: ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0];
    event.target.value = "";
    if (!file) return;
    const release = claimProfileWrite();
    if (!release) return;
    setSaved(false);
    upload.mutate({ file, release });
  };
  const onRemove = () => {
    if (busy) return;
    setSaved(false);
    clearField("avatar_url");
    if (removesAvatarAtOnce(user.avatar_url, avatarUrl)) {
      const release = claimProfileWrite();
      if (release) removal.mutate(release);
    } else {
      setAvatarUrl("");
    }
  };
  const uploaded = isUploadedAvatar(avatarUrl);
  const removalUnsaved = avatarUrl.trim() === "" && Boolean(user.avatar_url);

  const onSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (busy) return;
    setSaved(false);
    const fields: Record<string, string> = {};
    const nameCode = validateName(name);
    if (nameCode) fields.name = fieldMessage("name", nameCode);
    const avatarCode = validateAvatarUrl(avatarUrl);
    if (avatarCode) fields.avatar_url = fieldMessage("avatar_url", avatarCode);
    if (Object.keys(fields).length > 0) {
      setFields(fields);
      return;
    }
    const release = claimProfileWrite();
    if (release) mutation.mutate(release);
  };

  // Preview trails typing so the browser does not fetch every half-typed URL.
  const settledAvatarUrl = useDebouncedValue(avatarUrl.trim(), 400);
  const previewUrl =
    validateAvatarUrl(settledAvatarUrl) === null ? settledAvatarUrl : "";

  return (
    <form onSubmit={onSubmit} noValidate className="settings-form">
      <div className="settings-avatar-row">
        <Avatar name={name || user.name} url={previewUrl || null} size="lg" />
        <div className="settings-avatar-actions">
          <div className="settings-actions">
            <Button
              size="sm"
              onClick={() => picker.current?.click()}
              disabled={busy}
              pending={upload.isPending}
              pendingLabel="Lädt hoch…"
            >
              Bild hochladen
            </Button>
            {avatarUrl.trim() ? (
              <Button
                size="sm"
                variant="ghost"
                onClick={onRemove}
                disabled={busy}
                pending={removal.isPending}
                pendingLabel="Entfernt…"
              >
                Entfernen
              </Button>
            ) : null}
          </div>
          <input
            ref={picker}
            type="file"
            accept="image/png,image/jpeg,image/webp,image/gif"
            hidden
            aria-label="Avatar-Bild auswählen"
            onChange={onPick}
          />
        </div>
      </div>

      <Field
        id="name"
        label="Name"
        type="text"
        autoComplete="nickname"
        maxLength={64}
        value={name}
        onChange={(event) => {
          setName(event.target.value);
          setSaved(false);
          clearField("name");
        }}
        error={errors.fields.name}
      />
      <Field
        id="avatar_url"
        label="Avatar-URL"
        type="url"
        inputMode="url"
        placeholder={uploaded ? "Hochgeladenes Bild" : "https://…"}
        value={uploaded ? "" : avatarUrl}
        onChange={(event) => {
          setAvatarUrl(event.target.value);
          setSaved(false);
          clearField("avatar_url");
        }}
        error={errors.fields.avatar_url}
        hint={
          uploaded
            ? "Eine URL hier ersetzt das hochgeladene Bild."
            : removalUnsaved
              ? "Wird beim Speichern entfernt."
              : "Wird bei jedem Aufruf von der fremden Seite geladen."
        }
      />
      <FormError message={errors.form} />
      <div className="settings-actions">
        <SubmitButton
          pending={mutation.isPending}
          pendingLabel="Speichern…"
          disabled={!dirty || (busy && !mutation.isPending)}
          aria-describedby={unchanged ? unchangedId : undefined}
        >
          Speichern
        </SubmitButton>
        {unchanged ? (
          <span id={unchangedId} className="settings-hint">
            Keine Änderungen.
          </span>
        ) : null}
        <InlineStatus
          status={
            saved && !dirty
              ? { tone: "success", message: "Gespeichert." }
              : null
          }
        />
      </div>
    </form>
  );
}

/** Password change and signing out every device (issue #188). */
function AccountSecurity({ email }: { email: string }) {
  const navigate = useNavigate();
  const [currentPassword, setCurrentPassword] = useState("");
  const [password, setPassword] = useState("");
  const [changed, setChanged] = useState(false);
  const { errors, clearField, setFields, fromError } = useFormErrors();

  const mutation = useMutation({
    mutationFn: () => changePassword(currentPassword, password),
    onSuccess: () => {
      setCurrentPassword("");
      setPassword("");
      setChanged(true);
    },
    onError: fromError,
  });

  const onSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setChanged(false);
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

  const onLogoutEverywhere = async () => {
    if (
      !(await confirmAction({
        title: "Auf allen Geräten abmelden?",
        message:
          "Auch dieses Gerät. Danach musst du dich überall neu anmelden.",
        confirmLabel: "Überall abmelden",
      }))
    )
      return;
    leaveVoice();
    void logoutEverywhere();
    void navigate({ to: "/login" });
  };

  return (
    <>
      <form onSubmit={onSubmit} noValidate className="settings-form">
        <p className="settings-hint">
          Angemeldet als <strong className="settings-strong">{email}</strong>
        </p>
        <PasswordField
          id="current_password"
          label="Aktuelles Passwort"
          autoComplete="current-password"
          value={currentPassword}
          onChange={(event) => {
            setCurrentPassword(event.target.value);
            setChanged(false);
            clearField("current_password");
          }}
          error={errors.fields.current_password}
        />
        <PasswordField
          id="new_password"
          label="Neues Passwort"
          autoComplete="new-password"
          value={password}
          onChange={(event) => {
            setPassword(event.target.value);
            setChanged(false);
            clearField("password");
          }}
          error={errors.fields.password}
          hint="Andere Geräte werden dabei abgemeldet."
        />
        <FormError message={errors.form} />
        <div className="settings-actions">
          <SubmitButton pending={mutation.isPending} pendingLabel="Ändern…">
            Passwort ändern
          </SubmitButton>
          <InlineStatus
            status={
              changed
                ? {
                    tone: "success",
                    message: "Geändert. Andere Geräte sind abgemeldet.",
                  }
                : null
            }
          />
        </div>
      </form>

      <section
        className="settings-group settings-danger"
        aria-labelledby="logout-everywhere-heading"
      >
        <h3 id="logout-everywhere-heading" className="settings-group-title">
          Überall abmelden
        </h3>
        <div className="settings-actions">
          <Button variant="danger-outline" onClick={onLogoutEverywhere}>
            Auf allen Geräten abmelden
          </Button>
        </div>
      </section>
    </>
  );
}
