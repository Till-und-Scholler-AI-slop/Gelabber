import { useMutation } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { useState, type FormEvent } from "react";

import {
  fieldMessage,
  validateAvatarUrl,
  validateName,
  validatePassword,
} from "../auth/rules.ts";
import {
  changePassword,
  logoutEverywhere,
  updateProfile,
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

function ProfileForm({ user }: { user: User }) {
  const [name, setName] = useState(user.name);
  const [avatarUrl, setAvatarUrl] = useState(user.avatar_url ?? "");
  const [saved, setSaved] = useState(false);
  const { errors, clearField, setFields, fromError } = useFormErrors();

  const mutation = useMutation({
    mutationFn: () =>
      updateProfile({ name: name.trim(), avatar_url: avatarUrl.trim() }),
    onSuccess: (updated) => {
      setName(updated.name);
      setAvatarUrl(updated.avatar_url ?? "");
      setSaved(true);
    },
    onError: fromError,
  });

  const dirty =
    name.trim() !== user.name || avatarUrl.trim() !== (user.avatar_url ?? "");

  const onSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
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
    mutation.mutate();
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
          <p className="text-sm text-neutral-600 dark:text-neutral-400">
            Dein Bild begleitet dich in Chats und Sprachräumen. Ohne Bild zeigen
            wir deine Initialen.
          </p>
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
          placeholder="https://…"
          value={avatarUrl}
          onChange={(event) => {
            setAvatarUrl(event.target.value);
            setSaved(false);
            clearField("avatar_url");
          }}
          error={errors.fields.avatar_url}
          hint="Leer lassen, um das Bild zu entfernen."
        />
        <FormError message={errors.form} />
        <div className="flex items-center gap-4">
          <SubmitButton
            pending={mutation.isPending}
            pendingLabel="Speichern…"
            disabled={!dirty}
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
