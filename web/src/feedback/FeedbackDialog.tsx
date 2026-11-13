// "Feedback geben": a kind, a text, one optional image, and the list of
// what goes along. One dialog for the whole app, opened through
// `openFeedbackDialog()`; its draft outlives closing it.

import { Link, useNavigate } from "@tanstack/react-router";
import {
  useEffect,
  useId,
  useRef,
  useState,
  type ClipboardEvent,
  type FormEvent,
} from "react";

import { OS_NAMES } from "../auth/devices.ts";
import { useFormErrors } from "../auth/useFormErrors.ts";
import { FormError } from "../components/FormError.tsx";
import { CloseIcon, PaperclipIcon } from "../components/Icons.tsx";
import { GhostButton, Modal } from "../components/Modal.tsx";
import { SubmitButton } from "../components/SubmitButton.tsx";
import { notify } from "../components/toasts.ts";
import { Button } from "../components/ui/Button.tsx";
import { ChipGroup } from "../components/ui/ChipGroup.tsx";
import { fieldDescription } from "../components/ui/fieldDescription.ts";
import { FormField } from "../components/ui/FormField.tsx";
import { IconButton } from "../components/ui/IconButton.tsx";
import { Textarea } from "../components/ui/Textarea.tsx";
import { T, translate, useT } from "../i18n/catalog.ts";
import { common } from "../i18n/catalogs/common.ts";
import { feedback } from "../i18n/catalogs/feedback.ts";
import { language } from "../i18n/catalogs/language.ts";
import { useCurrentLocale } from "../i18n/locale.ts";
import { currentPath, feedbackContext } from "./context.ts";
import {
  clearFeedbackDraft,
  closeFeedbackDialog,
  dropStaleFeedbackDialog,
  feedbackDialogOpen,
  setFeedbackImage,
  updateFeedbackDraft,
  useFeedbackDialog,
} from "./dialogStore.ts";
import {
  useFeedbackSending,
  useFeedbackStatus,
  useSendFeedback,
} from "./queries.ts";
import {
  COUNTER_LIVE_BELOW,
  IMAGE_TYPES,
  TEXT_MAX,
  charactersLeft,
  clipboardImage,
  imageProblem,
  sendFieldErrors,
} from "./rules.ts";
import { FEEDBACK_KINDS, type FeedbackContext } from "./types.ts";
import "./feedback.css";

export function FeedbackDialog() {
  const t = useT(feedback);
  const open = useFeedbackDialog(feedbackDialogOpen);
  const descriptionId = useId();
  // Mounted for each account (AppShell keys it by user).
  useEffect(dropStaleFeedbackDialog, []);
  return (
    <Modal
      open={open}
      onClose={closeFeedbackDialog}
      title={t("dialog.title")}
      describedBy={descriptionId}
      wide
    >
      <FeedbackForm descriptionId={descriptionId} />
    </Modal>
  );
}

function FeedbackForm({ descriptionId }: { descriptionId: string }) {
  const t = useT(feedback);
  const tc = useT(common);
  const locale = useCurrentLocale();
  const draft = useFeedbackDialog((state) => state.draft);
  const status = useFeedbackStatus();
  const unread = status.data?.enabled ? status.data.unread : 0;
  const navigate = useNavigate();
  const form = useRef<HTMLFormElement>(null);
  const fileInput = useRef<HTMLInputElement>(null);
  const { errors, clearField, setFields, fromError, cooldown } =
    useFormErrors(form);
  const [imageNote, setImageNote] = useState("");
  // The page the dialog was opened on, fixed while it is open.
  const [path] = useState(currentPath);
  const context = feedbackContext(path, locale);
  const send = useSendFeedback(() => {
    closeFeedbackDialog();
    clearFeedbackDraft();
    notify(translate(feedback, "toast.sent"), {
      action: {
        label: translate(feedback, "toast.view"),
        onClick: () =>
          void navigate({
            to: "/settings/$area",
            params: { area: "feedback" },
          }),
      },
    });
  });
  // A send from before a close and reopen still counts.
  const sending = useFeedbackSending();
  const pending = send.isPending || sending;

  /** An image refusal, shown under the button and said by the status line. */
  const refuseImage = (message: string) => {
    setImageNote(message);
    setFields({ image: message });
  };

  const attach = (file: File) => {
    const problem = imageProblem(file);
    if (problem) {
      refuseImage(t(`image.${problem}`));
      return;
    }
    clearField("image");
    setImageNote(t(draft.image ? "image.replaced" : "image.attached"));
    setFeedbackImage(file);
  };

  const onPaste = (event: ClipboardEvent<HTMLFormElement>) => {
    if (pending) return;
    // Only the image is taken; text on the clipboard pastes as usual.
    const image = clipboardImage(event.clipboardData?.items);
    if (image) attach(image);
  };

  const onSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (pending) return;
    if (!draft.text.trim()) {
      setFields({ text: t("text.required") });
      return;
    }
    send.mutate(
      {
        kind: draft.kind,
        text: draft.text,
        image: draft.image,
        context,
      },
      {
        // Success is the hook's (above); an error only shows while open.
        onError: (error) => {
          const fields = sendFieldErrors(error);
          if (fields?.image) setImageNote(fields.image);
          if (fields) setFields(fields);
          else fromError(error);
        },
      },
    );
  };

  const left = charactersLeft(draft.text);
  const textError = errors.fields.text;
  const imageError = errors.fields.image;
  // A refused context field has no control here: the form says it.
  const otherError = Object.entries(errors.fields).find(
    ([field]) => field !== "text" && field !== "image",
  )?.[1];
  const counterId = "feedback-text-counter";
  const imageHint = t("image.hint");

  return (
    <form
      ref={form}
      onSubmit={onSubmit}
      onPaste={onPaste}
      noValidate
      className="feedback-form"
    >
      <p id={descriptionId} className="feedback-form__lead">
        {t("dialog.description")}
      </p>
      {unread > 0 ? (
        <p className="feedback-form__unread">
          <span className="unread-dot" aria-hidden="true" />
          <span>
            <T
              catalog={feedback}
              k="dialog.unread"
              tags={{
                link: (text) => (
                  <Link
                    to="/settings/$area"
                    params={{ area: "feedback" }}
                    onClick={closeFeedbackDialog}
                  >
                    {text}
                  </Link>
                ),
              }}
            />
          </span>
        </p>
      ) : null}
      <div className="feedback-form__kind">
        <span className="ui-label" aria-hidden="true">
          {t("kind.label")}
        </span>
        <ChipGroup
          label={t("kind.label")}
          options={FEEDBACK_KINDS.map((kind) => ({
            value: kind,
            label: t(`kind.${kind}`),
            disabled: pending,
          }))}
          value={draft.kind}
          onChange={(kind) => updateFeedbackDraft({ kind })}
        />
      </div>
      <FormField id="feedback-text" label={t("text.label")} error={textError}>
        <Textarea
          id="feedback-text"
          className="feedback-form__text"
          rows={6}
          required
          maxLength={TEXT_MAX}
          readOnly={pending}
          placeholder={t(`text.placeholder.${draft.kind}`)}
          value={draft.text}
          onChange={(event) => {
            updateFeedbackDraft({ text: event.target.value });
            clearField("text");
          }}
          invalid={Boolean(textError)}
          aria-describedby={fieldDescription("feedback-text", {
            error: textError,
            extra: left === null ? undefined : counterId,
          })}
        />
      </FormField>
      {left === null ? null : (
        <p
          id={counterId}
          className="ui-hint feedback-form__counter"
          aria-live={left < COUNTER_LIVE_BELOW ? "polite" : undefined}
        >
          {t("text.left", { count: left })}
        </p>
      )}
      <div className="feedback-form__image">
        <input
          ref={fileInput}
          type="file"
          accept={IMAGE_TYPES.join(",")}
          hidden
          tabIndex={-1}
          onChange={(event) => {
            const file = event.target.files?.[0];
            // The same file can be chosen again after a removal.
            event.target.value = "";
            if (file) attach(file);
          }}
        />
        {draft.image && draft.preview ? (
          <div className="feedback-form__thumb">
            <img src={draft.preview} alt={t("image.alt")} />
            <IconButton
              label={t("image.remove")}
              icon={<CloseIcon size={16} />}
              size="sm"
              variant="secondary"
              disabled={pending}
              onClick={() => {
                setFeedbackImage(null);
                setImageNote("");
                clearField("image");
              }}
            />
          </div>
        ) : null}
        <div>
          <Button
            size="sm"
            icon={<PaperclipIcon size={16} />}
            disabled={pending}
            aria-describedby={
              imageError ? "feedback-image-error" : "feedback-image-hint"
            }
            onClick={() => fileInput.current?.click()}
          >
            {t("image.attach")}
          </Button>
        </div>
        {imageError ? (
          <p id="feedback-image-error" className="ui-field-error">
            {imageError}
          </p>
        ) : (
          <p id="feedback-image-hint" className="ui-hint">
            {imageHint}
          </p>
        )}
        <p className="sr-only" role="status">
          {imageNote}
        </p>
      </div>
      <SentAlong context={context} />
      <FormError message={errors.form ?? otherError ?? null} />
      <div className="feedback-form__actions">
        <GhostButton onClick={closeFeedbackDialog}>{tc("cancel")}</GhostButton>
        <SubmitButton
          pending={pending}
          pendingLabel={t("sending")}
          cooldown={cooldown}
        >
          {t("submit")}
        </SubmitButton>
      </div>
    </form>
  );
}

/** "Wird mitgeschickt": exactly the values that go along. */
function SentAlong({ context }: { context: FeedbackContext }) {
  const t = useT(feedback);
  const tl = useT(language);
  return (
    <details open className="feedback-context">
      <summary>{t("context.title")}</summary>
      <dl>
        <dt>{t("context.app")}</dt>
        <dd>{t("context.webApp")}</dd>
        <dt>{t("context.system")}</dt>
        <dd>
          {context.os === "other"
            ? t("context.unknownSystem")
            : OS_NAMES[context.os]}
        </dd>
        <dt>{t("context.version")}</dt>
        <dd>{context.app_version}</dd>
        <dt>{t("context.language")}</dt>
        <dd>{tl(`option.${context.locale}`)}</dd>
        <dt>{t("context.page")}</dt>
        <dd>
          <code>{context.page}</code>
        </dd>
      </dl>
      <p>{t("context.nothingElse")}</p>
    </details>
  );
}
