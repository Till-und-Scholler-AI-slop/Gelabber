import { useMutation } from "@tanstack/react-query";
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
  prepareBanner,
} from "../auth/avatarImage.ts";
import {
  AVATAR_DECORATIONS,
  BIO_MAX,
  PRONOUNS_MAX,
  STATUS_TEXT_MAX,
  cardFormOf,
  fieldMessage,
  profileCardPatch,
  profileTextLength,
  validateCardForm,
  type CardForm,
} from "../auth/rules.ts";
import {
  claimProfileWrite,
  updateProfile,
  uploadBanner,
  useProfileWrites,
} from "../auth/session.ts";
import type { AvatarDecoration, User } from "../auth/types.ts";
import { useFormErrors } from "../auth/useFormErrors.ts";
import { Avatar } from "../components/Avatar.tsx";
import { AvatarFrame } from "../components/AvatarFrame.tsx";
import { Field } from "../components/Field.tsx";
import { FormError } from "../components/FormError.tsx";
import { ProfileCardView } from "../components/ProfileCard.tsx";
import { DECORATION_LABEL } from "../components/profileStyle.ts";
import { SubmitButton } from "../components/SubmitButton.tsx";

/** What a color picker shows while no color is set. */
const PICKER_DEFAULT = "#7c6cf2";

type Upload = { file: File; release: () => void };

/**
 * Profile card editor: banner, colors, avatar decoration, pronouns, status
 * text and "Über mich", with a live preview of the card others see.
 */
export function ProfileCardForm({ user }: { user: User }) {
  const [form, setForm] = useState<CardForm>(() => cardFormOf(user));
  const [saved, setSaved] = useState(false);
  const { errors, clearField, setFields, fromError } = useFormErrors();
  const busy = useProfileWrites((state) => state.pending > 0);
  const dirty = Object.keys(profileCardPatch(form, user)).length > 0;
  const unchangedId = useId();
  const bioCountId = useId();

  const edit = <K extends keyof CardForm>(field: K, value: CardForm[K]) => {
    setForm((current) => ({ ...current, [field]: value }));
    setSaved(false);
    clearField(field);
  };

  const mutation = useMutation({
    mutationFn: async (release: () => void) => {
      try {
        return await updateProfile(profileCardPatch(form, user));
      } finally {
        release();
      }
    },
    onSuccess: (updated) => {
      setForm(cardFormOf(updated));
      setSaved(true);
    },
    onError: fromError,
  });

  // Banner uploads and removals apply at once, like the avatar's.
  const picker = useRef<HTMLInputElement>(null);
  const upload = useMutation({
    mutationFn: async ({ file, release }: Upload) => {
      try {
        return await uploadBanner(await prepareBanner(file));
      } finally {
        release();
      }
    },
    onSuccess: () => {
      clearField("banner_url");
      setSaved(true);
    },
    onError: (error) => {
      if (error instanceof AvatarImageError) {
        setFields({ banner_url: avatarImageMessage(error) });
      } else {
        fromError(error);
      }
    },
  });
  const removal = useMutation({
    mutationFn: async (release: () => void) => {
      try {
        return await updateProfile({ banner_url: "" });
      } finally {
        release();
      }
    },
    onSuccess: () => {
      clearField("banner_url");
      setSaved(true);
    },
    onError: fromError,
  });

  const onPick = (event: ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0];
    event.target.value = "";
    if (!file) return;
    const release = claimProfileWrite();
    if (!release) return;
    setSaved(false);
    upload.mutate({ file, release });
  };
  const onRemoveBanner = () => {
    const release = claimProfileWrite();
    if (!release) return;
    setSaved(false);
    removal.mutate(release);
  };

  const onSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (busy) return;
    setSaved(false);
    const codes = validateCardForm(form);
    const fields: Record<string, string> = {};
    for (const [field, code] of Object.entries(codes)) {
      if (code) fields[field] = fieldMessage(field, code);
    }
    if (Object.keys(fields).length > 0) {
      setFields(fields);
      return;
    }
    const release = claimProfileWrite();
    if (release) mutation.mutate(release);
  };

  const unchanged = !dirty && !saved && !busy;
  const bioLength = profileTextLength(form.bio);
  // No `maxLength` on the textarea: pasting a longer text and trimming it
  // must work, and the browser would count UTF-16 units before the API's
  // trim. Over the limit the field is marked invalid right away instead.
  const bioInvalid = Boolean(errors.fields.bio) || bioLength > BIO_MAX;

  return (
    <section className="lr-account-page mx-auto mt-12 max-w-md">
      <h2 className="text-xl font-semibold tracking-tight">Profilkarte</h2>
      <p className="mt-1 text-sm text-neutral-600 dark:text-neutral-400">
        So sehen dich andere, wenn sie auf dich tippen – im Browser, in der App
        und auf dem Handy.
      </p>

      <div
        className="mt-6"
        role="group"
        aria-label="Vorschau deiner Profilkarte"
      >
        <ProfileCardView
          person={{
            ...form,
            avatar_decoration: form.avatar_decoration || null,
            name: user.name,
            avatar_url: user.avatar_url,
            banner_url: user.banner_url ?? null,
          }}
        />
      </div>

      <form
        onSubmit={onSubmit}
        noValidate
        className="mt-6 flex flex-col gap-6"
        aria-label="Profilkarte bearbeiten"
      >
        <div className="flex flex-col gap-2">
          <span className="text-sm font-medium text-neutral-800 dark:text-neutral-200">
            Banner
          </span>
          <div className="flex flex-wrap gap-2">
            <button
              type="button"
              onClick={() => picker.current?.click()}
              disabled={busy}
              className="rounded-lg border border-neutral-300 px-3 py-1.5 text-sm font-medium transition hover:bg-neutral-100 disabled:opacity-60 dark:border-neutral-700 dark:hover:bg-neutral-800"
            >
              {upload.isPending ? "Lädt hoch…" : "Bannerbild hochladen"}
            </button>
            {user.banner_url ? (
              <button
                type="button"
                onClick={onRemoveBanner}
                disabled={busy}
                className="rounded-lg px-3 py-1.5 text-sm font-medium text-neutral-600 transition hover:bg-neutral-100 disabled:opacity-60 dark:text-neutral-400 dark:hover:bg-neutral-800"
              >
                {removal.isPending ? "Entfernt…" : "Bannerbild entfernen"}
              </button>
            ) : null}
          </div>
          <input
            ref={picker}
            type="file"
            accept="image/png,image/jpeg,image/webp,image/gif"
            className="hidden"
            aria-label="Bannerbild auswählen"
            onChange={onPick}
          />
          {errors.fields.banner_url ? (
            <p role="alert" className="text-sm text-red-600 dark:text-red-400">
              {errors.fields.banner_url}
            </p>
          ) : (
            <p className="text-sm text-neutral-500 dark:text-neutral-400">
              Breites Bild, wird auf 3 : 1 zugeschnitten. Ohne Bild zeigen wir
              die Bannerfarbe.
            </p>
          )}
        </div>

        <div className="grid grid-cols-1 gap-6 sm:grid-cols-2">
          <ColorField
            id="banner_color"
            label="Bannerfarbe"
            value={form.banner_color}
            fallback={form.accent_color}
            onChange={(value) => edit("banner_color", value)}
            error={errors.fields.banner_color}
          />
          <ColorField
            id="accent_color"
            label="Akzentfarbe"
            value={form.accent_color}
            onChange={(value) => edit("accent_color", value)}
            error={errors.fields.accent_color}
          />
        </div>

        <fieldset className="flex flex-col gap-2">
          <legend className="mb-2 text-sm font-medium text-neutral-800 dark:text-neutral-200">
            Avatar-Rahmen
          </legend>
          <div className="deco-picker">
            {(["", ...AVATAR_DECORATIONS] as const).map((decoration) => (
              <label key={decoration || "none"} className="deco-option">
                <input
                  type="radio"
                  name="avatar_decoration"
                  value={decoration}
                  checked={form.avatar_decoration === decoration}
                  onChange={() =>
                    edit(
                      "avatar_decoration",
                      decoration as AvatarDecoration | "",
                    )
                  }
                />
                <AvatarFrame
                  decoration={decoration || null}
                  accent={form.accent_color || null}
                >
                  <Avatar name={user.name} url={user.avatar_url} size="md" />
                </AvatarFrame>
                <span>
                  {decoration ? DECORATION_LABEL[decoration] : "Keiner"}
                </span>
              </label>
            ))}
          </div>
          {errors.fields.avatar_decoration ? (
            <p role="alert" className="text-sm text-red-600 dark:text-red-400">
              {errors.fields.avatar_decoration}
            </p>
          ) : null}
        </fieldset>

        <Field
          id="pronouns"
          label="Pronomen"
          type="text"
          maxLength={PRONOUNS_MAX}
          placeholder="z. B. sie/ihr"
          value={form.pronouns}
          onChange={(event) => edit("pronouns", event.target.value)}
          error={errors.fields.pronouns}
        />
        <Field
          id="status_text"
          label="Status"
          type="text"
          maxLength={STATUS_TEXT_MAX}
          placeholder="Was machst du gerade?"
          value={form.status_text}
          onChange={(event) => edit("status_text", event.target.value)}
          error={errors.fields.status_text}
          hint="Steht in der Mitgliederliste unter deinem Namen."
        />
        <div className="flex flex-col gap-1.5">
          <label
            htmlFor="bio"
            className="text-sm font-medium text-neutral-800 dark:text-neutral-200"
          >
            Über mich
          </label>
          <textarea
            id="bio"
            rows={4}
            value={form.bio}
            onChange={(event) => edit("bio", event.target.value)}
            aria-invalid={bioInvalid ? true : undefined}
            aria-describedby={errors.fields.bio ? "bio-error" : bioCountId}
            className={[
              "resize-y rounded-lg border bg-white px-3 py-2 text-base text-neutral-900 outline-none transition focus:ring-2 focus:ring-offset-0 dark:bg-neutral-900 dark:text-neutral-100",
              bioInvalid
                ? "border-red-400 focus:border-red-500 focus:ring-red-200 dark:border-red-600 dark:focus:border-red-400 dark:focus:ring-red-900"
                : "border-neutral-300 focus:border-neutral-500 focus:ring-neutral-200 dark:border-neutral-600 dark:focus:border-neutral-400 dark:focus:ring-neutral-700",
            ].join(" ")}
          />
          {errors.fields.bio ? (
            <p
              id="bio-error"
              role="alert"
              className="text-sm text-red-600 dark:text-red-400"
            >
              {errors.fields.bio}
            </p>
          ) : null}
          <p
            id={bioCountId}
            className={`text-sm ${bioLength > BIO_MAX ? "text-red-600 dark:text-red-400" : "text-neutral-500 dark:text-neutral-400"}`}
          >
            {bioLength} / {BIO_MAX} Zeichen
          </p>
        </div>

        <FormError message={errors.form} />
        <div className="flex items-center gap-4">
          <SubmitButton
            pending={mutation.isPending}
            pendingLabel="Speichern…"
            disabled={!dirty || (busy && !mutation.isPending)}
            aria-describedby={unchanged ? unchangedId : undefined}
          >
            Profilkarte speichern
          </SubmitButton>
          {unchanged ? (
            <span
              id={unchangedId}
              className="text-sm text-neutral-600 dark:text-neutral-400"
            >
              Noch nichts geändert.
            </span>
          ) : null}
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

function ColorField({
  id,
  label,
  value,
  fallback,
  onChange,
  error,
}: {
  id: string;
  label: string;
  value: string;
  /** What the card shows while this color is unset. */
  fallback?: string | undefined;
  onChange: (value: string) => void;
  error?: string | undefined;
}) {
  return (
    <div className="flex flex-col gap-1.5">
      <label
        htmlFor={id}
        className="text-sm font-medium text-neutral-800 dark:text-neutral-200"
      >
        {label}
      </label>
      <div className="flex items-center gap-3">
        <input
          id={id}
          type="color"
          value={value || fallback || PICKER_DEFAULT}
          onChange={(event) => onChange(event.target.value)}
          aria-invalid={error ? true : undefined}
          aria-describedby={error ? `${id}-error` : `${id}-value`}
          className="h-9 w-12 shrink-0 cursor-pointer rounded-md border border-neutral-300 bg-transparent p-0.5 dark:border-neutral-600"
        />
        <span
          id={`${id}-value`}
          className="text-sm text-neutral-600 dark:text-neutral-400"
        >
          {value ? value.toLowerCase() : "Standard"}
        </span>
        {value ? (
          <button
            type="button"
            onClick={() => onChange("")}
            aria-label={`${label} zurücksetzen`}
            className="rounded-md px-2 py-1 text-sm text-neutral-600 transition hover:bg-neutral-100 dark:text-neutral-400 dark:hover:bg-neutral-800"
          >
            Zurücksetzen
          </button>
        ) : null}
      </div>
      {error ? (
        <p
          id={`${id}-error`}
          role="alert"
          className="text-sm text-red-600 dark:text-red-400"
        >
          {error}
        </p>
      ) : null}
    </div>
  );
}
