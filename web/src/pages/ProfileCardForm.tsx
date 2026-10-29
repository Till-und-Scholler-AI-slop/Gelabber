import { useMutation } from "@tanstack/react-query";
import { useBlocker } from "@tanstack/react-router";
import {
  useId,
  useRef,
  useState,
  type ChangeEvent,
  type CSSProperties,
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
import { confirmAction } from "../components/confirm.ts";
import { Field } from "../components/Field.tsx";
import { FormError } from "../components/FormError.tsx";
import { ChevronIcon, CloseIcon } from "../components/Icons.tsx";
import { ProfileCardView } from "../components/ProfileCard.tsx";
import { DECORATION_LABEL } from "../components/profileStyle.ts";
import { SubmitButton } from "../components/SubmitButton.tsx";
import { Button } from "../components/ui/Button.tsx";
import { IconButton } from "../components/ui/IconButton.tsx";
import { InlineStatus } from "../components/ui/InlineStatus.tsx";
import { Textarea } from "../components/ui/Textarea.tsx";

/** What a color picker shows while no color is set. */
const PICKER_DEFAULT = "#7c6cf2";

type Upload = { file: File; release: () => void };

/**
 * Profile card editor: banner, colors, avatar decoration, pronouns, status
 * text and "Über mich", next to a live preview of the card others see. The
 * banner image saves at once; everything else waits for the save bar.
 */
export function ProfileCardForm({ user }: { user: User }) {
  const [form, setForm] = useState<CardForm>(() => cardFormOf(user));
  const [saved, setSaved] = useState(false);
  const [bannerSaved, setBannerSaved] = useState(false);
  const [previewOpen, setPreviewOpen] = useState(false);
  const { errors, clear, clearField, setFields, fromError } = useFormErrors();
  const busy = useProfileWrites((state) => state.pending > 0);
  const dirty = Object.keys(profileCardPatch(form, user)).length > 0;
  const bioCountId = useId();
  // Every settings area is its own page now: leaving one with unsaved card
  // changes asks first, like the theme editor.
  useBlocker({
    shouldBlockFn: async () =>
      dirty &&
      !(await confirmAction({
        title: "Ungespeicherte Änderungen verwerfen?",
        message: "Deine Änderungen an der Profilkarte gehen verloren.",
        confirmLabel: "Verwerfen",
      })),
    enableBeforeUnload: dirty,
  });

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
      setBannerSaved(true);
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
      setBannerSaved(true);
    },
    onError: fromError,
  });

  const onPick = (event: ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0];
    event.target.value = "";
    if (!file) return;
    const release = claimProfileWrite();
    if (!release) return;
    setBannerSaved(false);
    upload.mutate({ file, release });
  };
  const onRemoveBanner = () => {
    const release = claimProfileWrite();
    if (!release) return;
    setBannerSaved(false);
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

  const bioLength = profileTextLength(form.bio);
  // No `maxLength` on the textarea: pasting a longer text and trimming it
  // must work, and the browser would count UTF-16 units before the API's
  // trim. Over the limit the field is marked invalid right away instead.
  const bioInvalid = Boolean(errors.fields.bio) || bioLength > BIO_MAX;
  const bioDescription = [errors.fields.bio ? "bio-error" : null, bioCountId]
    .filter(Boolean)
    .join(" ");

  return (
    <div className="profile-card-editor">
      <div
        className={`profile-card-preview${previewOpen ? " is-open" : ""}`}
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
        {/* Phones show a strip of the card while editing; this opens all of it. */}
        <button
          type="button"
          className="profile-card-preview__toggle"
          aria-expanded={previewOpen}
          onClick={() => setPreviewOpen((open) => !open)}
        >
          Ganze Karte
          <ChevronIcon size={14} />
        </button>
      </div>

      <form
        onSubmit={onSubmit}
        noValidate
        className="settings-form"
        aria-label="Profilkarte bearbeiten"
      >
        <div className="ui-form-field">
          <span className="ui-label">Banner</span>
          <div className="settings-actions">
            <Button
              size="sm"
              onClick={() => picker.current?.click()}
              disabled={busy}
              pending={upload.isPending}
              pendingLabel="Lädt hoch…"
            >
              Bannerbild hochladen
            </Button>
            {user.banner_url ? (
              <Button
                size="sm"
                variant="ghost"
                onClick={onRemoveBanner}
                disabled={busy}
                pending={removal.isPending}
                pendingLabel="Entfernt…"
              >
                Bannerbild entfernen
              </Button>
            ) : null}
            <InlineStatus
              status={
                bannerSaved
                  ? { tone: "success", message: "Banner gespeichert." }
                  : null
              }
            />
          </div>
          <input
            ref={picker}
            type="file"
            accept="image/png,image/jpeg,image/webp,image/gif"
            hidden
            aria-label="Bannerbild auswählen"
            onChange={onPick}
          />
          {errors.fields.banner_url ? (
            <p role="alert" className="ui-field-error">
              {errors.fields.banner_url}
            </p>
          ) : (
            <p className="ui-hint">
              Wird auf 3 : 1 zugeschnitten und sofort gespeichert.
            </p>
          )}
        </div>

        <div className="settings-field-pair">
          <ColorField
            id="banner_color"
            label="Bannerfarbe"
            value={form.banner_color}
            fallback={form.accent_color}
            // As ProfileCard draws a banner without its own color.
            unsetSwatch={
              form.accent_color ||
              "color-mix(in srgb, var(--lr-accent) 65%, var(--lr-panel))"
            }
            onChange={(value) => edit("banner_color", value)}
            error={errors.fields.banner_color}
          />
          <ColorField
            id="accent_color"
            label="Akzentfarbe"
            value={form.accent_color}
            unsetSwatch="var(--lr-accent)"
            onChange={(value) => edit("accent_color", value)}
            error={errors.fields.accent_color}
          />
        </div>

        <fieldset className="settings-fieldset">
          <legend className="ui-label">Avatar-Rahmen</legend>
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
                  <Avatar
                    name={user.name}
                    url={user.avatar_url}
                    size="md"
                    decorative
                  />
                </AvatarFrame>
                <span>
                  {decoration ? DECORATION_LABEL[decoration] : "Keiner"}
                </span>
              </label>
            ))}
          </div>
          {errors.fields.avatar_decoration ? (
            <p className="ui-field-error">{errors.fields.avatar_decoration}</p>
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
          value={form.status_text}
          onChange={(event) => edit("status_text", event.target.value)}
          error={errors.fields.status_text}
          hint="Steht in der Mitgliederliste unter deinem Namen."
        />
        <div className="ui-form-field">
          <label htmlFor="bio" className="ui-label">
            Über mich
          </label>
          <Textarea
            id="bio"
            rows={4}
            value={form.bio}
            onChange={(event) => edit("bio", event.target.value)}
            invalid={bioInvalid}
            aria-describedby={bioDescription}
            className="resize-y"
          />
          {errors.fields.bio ? (
            <p id="bio-error" className="ui-field-error">
              {errors.fields.bio}
            </p>
          ) : null}
          <p
            id={bioCountId}
            className={bioLength > BIO_MAX ? "ui-field-error" : "ui-hint"}
          >
            {bioLength} / {BIO_MAX} Zeichen
          </p>
        </div>

        <FormError message={errors.form} />
        {/* Sticks to the bottom while there is something to save. */}
        <div
          className={`settings-save-bar${dirty || mutation.isPending ? " is-dirty" : ""}`}
        >
          {dirty || mutation.isPending ? (
            <>
              <span className="settings-save-bar__text">
                Ungespeicherte Änderungen
              </span>
              <Button
                variant="ghost"
                disabled={mutation.isPending}
                onClick={() => {
                  setForm(cardFormOf(user));
                  clear();
                }}
              >
                Verwerfen
              </Button>
              <SubmitButton
                pending={mutation.isPending}
                pendingLabel="Speichern…"
                disabled={busy && !mutation.isPending}
              >
                Speichern
              </SubmitButton>
            </>
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
    </div>
  );
}

function ColorField({
  id,
  label,
  value,
  fallback,
  unsetSwatch,
  onChange,
  error,
}: {
  id: string;
  label: string;
  value: string;
  /** Where the picker starts while this color is unset. */
  fallback?: string | undefined;
  /** The CSS color the card shows while this color is unset. The swatch
   *  shows it too, not the picker's start value (#375). */
  unsetSwatch: string;
  onChange: (value: string) => void;
  error?: string | undefined;
}) {
  return (
    <div className="ui-form-field">
      <label htmlFor={id} className="ui-label">
        {label}
      </label>
      <div className="settings-color-field">
        <input
          id={id}
          type="color"
          value={value || fallback || PICKER_DEFAULT}
          onChange={(event) => onChange(event.target.value)}
          aria-invalid={error ? true : undefined}
          aria-describedby={error ? `${id}-error` : `${id}-value`}
          className={`settings-color-swatch${value ? "" : " is-unset"}`}
          style={
            value
              ? undefined
              : ({ "--settings-swatch": unsetSwatch } as CSSProperties)
          }
        />
        <span id={`${id}-value`} className="settings-hint">
          {value ? value.toLowerCase() : "Standard"}
        </span>
        {value ? (
          <IconButton
            size="sm"
            label={`${label} zurücksetzen`}
            icon={<CloseIcon size={16} />}
            onClick={() => onChange("")}
          />
        ) : null}
      </div>
      {error ? (
        <p id={`${id}-error`} className="ui-field-error">
          {error}
        </p>
      ) : null}
    </div>
  );
}
