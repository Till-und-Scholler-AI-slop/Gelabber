import { useMutation } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { useRef, useState, type ChangeEvent, type FormEvent } from "react";

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
import { Field } from "../components/Field.tsx";
import { FormError } from "../components/FormError.tsx";
import { SubmitButton } from "../components/SubmitButton.tsx";
import { useDebouncedValue } from "../components/useDebouncedValue.ts";
import { leaveVoice, stopWatching } from "../voice/session.ts";

export function ProfilePage() {
  const user = useSession((state) => state.user);
  // The route guard guarantees a user; the fallback keeps the type narrow.
  if (!user) return null;
  return (
    <>
      <ProfileForm key={user.id} user={user} />
      <AccountSecurity key={`security-${user.id}`} />
    </>
  );
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
    },
    onError: fromError,
  });
  // One profile write at a time, across remounts of this form: a late or
  // failed answer of one would otherwise put an older user back into the
  // session store.
  const busy = useProfileWrites((state) => state.pending > 0);
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
    <section className="lr-account-page mx-auto max-w-md">
      <h1 className="text-2xl font-semibold tracking-tight">Profil</h1>
      <p className="mt-1 text-sm text-neutral-600 dark:text-neutral-400">
        {user.email}
      </p>

      <form onSubmit={onSubmit} noValidate className="mt-8 flex flex-col gap-6">
        <div className="flex items-center gap-4">
          <Avatar name={name || user.name} url={previewUrl || null} size="lg" />
          <div className="flex flex-col gap-2">
            <p className="text-sm text-neutral-600 dark:text-neutral-400">
              Dein Bild begleitet dich in Chats und Sprachräumen. Ohne Bild
              zeigen wir deine Initialen.
            </p>
            <div className="flex flex-wrap gap-2">
              <button
                type="button"
                onClick={() => picker.current?.click()}
                disabled={busy}
                className="rounded-lg border border-neutral-300 px-3 py-1.5 text-sm font-medium transition hover:bg-neutral-100 disabled:opacity-60 dark:border-neutral-700 dark:hover:bg-neutral-800"
              >
                {upload.isPending ? "Lädt hoch…" : "Bild hochladen"}
              </button>
              {avatarUrl.trim() ? (
                <button
                  type="button"
                  onClick={onRemove}
                  disabled={busy}
                  className="rounded-lg px-3 py-1.5 text-sm font-medium text-neutral-600 transition hover:bg-neutral-100 disabled:opacity-60 dark:text-neutral-400 dark:hover:bg-neutral-800"
                >
                  {removal.isPending ? "Entfernt…" : "Entfernen"}
                </button>
              ) : null}
            </div>
            <input
              ref={picker}
              type="file"
              accept="image/png,image/jpeg,image/webp,image/gif"
              className="hidden"
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
                ? "Noch nicht gespeichert: Speichern entfernt das Bild."
                : "Lieber hochladen: Links laden das Bild bei jedem Aufruf von der fremden Seite."
          }
        />
        <FormError message={errors.form} />
        <div className="flex items-center gap-4">
          <SubmitButton
            pending={mutation.isPending}
            pendingLabel="Speichern…"
            disabled={!dirty || (busy && !mutation.isPending)}
          >
            Speichern
          </SubmitButton>
          {saved && !dirty ? (
            <span
              role="status"
              className="text-sm text-emerald-700 dark:text-emerald-300"
            >
              Gespeichert.
            </span>
          ) : null}
        </div>
      </form>
    </section>
  );
}

/** Password change and signing out every device (issue #188). */
function AccountSecurity() {
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

  const onLogoutEverywhere = () => {
    if (
      !window.confirm(
        "Auf allen Geräten abmelden, auch hier? Du musst dich danach überall neu anmelden.",
      )
    )
      return;
    leaveVoice();
    stopWatching();
    void logoutEverywhere();
    void navigate({ to: "/login" });
  };

  return (
    <section className="lr-account-page mx-auto mt-12 max-w-md">
      <h2 className="text-xl font-semibold tracking-tight">Sicherheit</h2>

      <form onSubmit={onSubmit} noValidate className="mt-6 flex flex-col gap-6">
        <Field
          id="current_password"
          label="Aktuelles Passwort"
          type="password"
          autoComplete="current-password"
          value={currentPassword}
          onChange={(event) => {
            setCurrentPassword(event.target.value);
            setChanged(false);
            clearField("current_password");
          }}
          error={errors.fields.current_password}
        />
        <Field
          id="new_password"
          label="Neues Passwort"
          type="password"
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
        <div className="flex items-center gap-4">
          <SubmitButton pending={mutation.isPending} pendingLabel="Ändern…">
            Passwort ändern
          </SubmitButton>
          {changed ? (
            <span
              role="status"
              className="text-sm text-emerald-700 dark:text-emerald-300"
            >
              Geändert. Andere Geräte sind abgemeldet.
            </span>
          ) : null}
        </div>
      </form>

      <div className="mt-8 flex flex-col gap-2">
        <p className="text-sm text-neutral-600 dark:text-neutral-400">
          Ein Gerät verloren oder irgendwo angemeldet geblieben?
        </p>
        <div>
          <button
            type="button"
            onClick={onLogoutEverywhere}
            className="rounded-lg border border-red-300 px-3 py-2 text-sm font-medium text-red-700 transition hover:bg-red-50 dark:border-red-800 dark:text-red-300 dark:hover:bg-red-950"
          >
            Überall abmelden
          </button>
        </div>
      </div>
    </section>
  );
}
