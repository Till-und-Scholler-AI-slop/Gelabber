import { useMutation } from "@tanstack/react-query";
import { useState, type FormEvent } from "react";

import {
  fieldMessage,
  validateAvatarUrl,
  validateName,
} from "../auth/rules.ts";
import { updateProfile, useSession } from "../auth/session.ts";
import type { User } from "../auth/types.ts";
import { useFormErrors } from "../auth/useFormErrors.ts";
import { Avatar } from "../components/Avatar.tsx";
import { Field } from "../components/Field.tsx";
import { FormError } from "../components/FormError.tsx";
import { SubmitButton } from "../components/SubmitButton.tsx";
import { useDebouncedValue } from "../components/useDebouncedValue.ts";

export function ProfilePage() {
  const user = useSession((state) => state.user);
  // The route guard guarantees a user; the fallback keeps the type narrow.
  if (!user) return null;
  return <ProfileForm key={user.id} user={user} />;
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
    <section className="mx-auto max-w-md">
      <h1 className="text-2xl font-semibold tracking-tight">Profil</h1>
      <p className="mt-1 text-sm text-neutral-600 dark:text-neutral-400">
        {user.email}
      </p>

      <form onSubmit={onSubmit} noValidate className="mt-8 flex flex-col gap-6">
        <div className="flex items-center gap-4">
          <Avatar name={name || user.name} url={previewUrl || null} size="lg" />
          <p className="text-sm text-neutral-600 dark:text-neutral-400">
            Ohne Bild zeigen wir deine Initialen. Datei-Upload kommt mit dem
            Dateien-Ticket; bis dahin reicht ein Link.
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
