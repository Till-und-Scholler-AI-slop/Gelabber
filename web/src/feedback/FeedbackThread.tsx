// One entry of Einstellungen → Feedback: what the person sent, what went
// along, the thread with the operators, and an inline reply.

import { useRef, useState, type FormEvent } from "react";

import { useFormErrors } from "../auth/useFormErrors.ts";
import { FormError } from "../components/FormError.tsx";
import { ImageLightbox } from "../components/ImageLightbox.tsx";
import { GhostButton } from "../components/Modal.tsx";
import { SubmitButton } from "../components/SubmitButton.tsx";
import { Badge } from "../components/ui/Badge.tsx";
import { Button } from "../components/ui/Button.tsx";
import { fieldDescription } from "../components/ui/fieldDescription.ts";
import { FormField } from "../components/ui/FormField.tsx";
import { Textarea } from "../components/ui/Textarea.tsx";
import { useT } from "../i18n/catalog.ts";
import { common } from "../i18n/catalogs/common.ts";
import { feedback } from "../i18n/catalogs/feedback.ts";
import { formatDate } from "../i18n/format.ts";
import { useReplyFeedback } from "./queries.ts";
import { TEXT_MAX, sendFieldErrors } from "./rules.ts";
import { sentAlongLine } from "./thread.ts";
import type { FeedbackEntry } from "./types.ts";

const WHEN: Intl.DateTimeFormatOptions = {
  dateStyle: "medium",
  timeStyle: "short",
};

export function FeedbackThread({
  entry,
  isNew,
}: {
  entry: FeedbackEntry;
  /** Whether a message was new when this visit first showed it. */
  isNew: (messageId: string) => boolean;
}) {
  const t = useT(feedback);
  const titleId = `feedback-entry-${entry.id}`;
  const [enlarged, setEnlarged] = useState(false);
  return (
    <article className="feedback-entry" aria-labelledby={titleId}>
      <h3 id={titleId} className="feedback-entry__head">
        <Badge>{t(`kind.${entry.kind}`)}</Badge>
        <time dateTime={entry.created_at}>{formatDate(entry.created_at)}</time>
        <Badge tone={entry.status === "done" ? "success" : "neutral"}>
          {t(`status.${entry.status}`)}
        </Badge>
      </h3>
      <p className="feedback-entry__text">{entry.text}</p>
      {entry.image ? (
        <button
          type="button"
          className="feedback-entry__image"
          onClick={() => setEnlarged(true)}
        >
          <img
            src={entry.image.url}
            alt={t("image.alt")}
            decoding="async"
            loading="lazy"
          />
        </button>
      ) : null}
      {entry.image && enlarged ? (
        <ImageLightbox
          image={{
            id: entry.id,
            src: entry.image.url,
            filename: t("image.alt"),
          }}
          onClose={() => setEnlarged(false)}
        />
      ) : null}
      <details className="feedback-entry__context">
        <summary>{t("thread.sentAlong")}</summary>
        <p>{sentAlongLine(entry.context)}</p>
      </details>
      {entry.messages.length > 0 ? (
        <ol className="feedback-thread">
          {entry.messages.map((message) => (
            <li
              key={message.id}
              className={`feedback-message feedback-message--${message.from}`}
            >
              <p className="feedback-message__meta">
                <strong>
                  {t(
                    message.from === "you" ? "thread.you" : "thread.operators",
                  )}
                </strong>
                <time dateTime={message.created_at}>
                  {formatDate(message.created_at, WHEN)}
                </time>
                {message.new || isNew(message.id) ? (
                  <Badge tone="accent">{t("thread.new")}</Badge>
                ) : null}
              </p>
              <p className="feedback-message__text">{message.text}</p>
            </li>
          ))}
        </ol>
      ) : null}
      {entry.status === "done" ? (
        <p className="ui-hint">{t("thread.doneNote")}</p>
      ) : null}
      <ReplyForm entryId={entry.id} />
    </article>
  );
}

/** "Antworten", opening a form in place; after sending, focus comes back. */
function ReplyForm({ entryId }: { entryId: string }) {
  const t = useT(feedback);
  const tc = useT(common);
  const [open, setOpen] = useState(false);
  const [text, setText] = useState("");
  const [note, setNote] = useState("");
  const form = useRef<HTMLFormElement>(null);
  const toggle = useRef<HTMLButtonElement>(null);
  const { errors, clearField, setFields, fromError, clear, cooldown } =
    useFormErrors(form);
  const reply = useReplyFeedback();
  const pending = reply.isPending;
  const fieldId = `feedback-reply-${entryId}`;

  const close = () => {
    setOpen(false);
    clear();
    // Back on "Antworten" once the form is gone.
    if (typeof requestAnimationFrame !== "undefined")
      requestAnimationFrame(() => toggle.current?.focus());
  };

  const onSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (pending) return;
    if (!text.trim()) {
      setFields({ text: t("text.required") });
      return;
    }
    reply.mutate(
      { id: entryId, text },
      {
        onSuccess: () => {
          setText("");
          setNote(t("thread.replySent"));
          close();
        },
        onError: (error) => {
          const fields = sendFieldErrors(error);
          if (fields) setFields(fields);
          else fromError(error);
        },
      },
    );
  };

  const textError = errors.fields.text;
  return (
    <div className="feedback-reply">
      {open ? (
        <form
          ref={form}
          onSubmit={onSubmit}
          noValidate
          className="feedback-reply__form"
        >
          <FormField
            id={fieldId}
            label={t("thread.replyLabel")}
            error={textError}
          >
            <Textarea
              id={fieldId}
              className="feedback-reply__text"
              rows={3}
              required
              maxLength={TEXT_MAX}
              readOnly={pending}
              autoFocus
              value={text}
              onChange={(event) => {
                setText(event.target.value);
                clearField("text");
              }}
              invalid={Boolean(textError)}
              aria-describedby={fieldDescription(fieldId, {
                error: textError,
              })}
            />
          </FormField>
          <FormError message={errors.form} />
          <div className="feedback-reply__actions">
            <GhostButton onClick={close} disabled={pending}>
              {tc("cancel")}
            </GhostButton>
            <SubmitButton
              pending={pending}
              pendingLabel={t("sending")}
              cooldown={cooldown}
            >
              {t("submit")}
            </SubmitButton>
          </div>
        </form>
      ) : (
        <Button
          ref={toggle}
          size="sm"
          onClick={() => {
            setNote("");
            setOpen(true);
          }}
        >
          {t("thread.reply")}
        </Button>
      )}
      <p className="sr-only" role="status">
        {note}
      </p>
    </div>
  );
}
