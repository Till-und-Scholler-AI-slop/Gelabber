// "Feedback": ideas, problems and praise from accounts and from visitors of
// the public form. Each entry is a card that opens into its text, image,
// technical details and thread, with a reply form and the actions. The link
// in an operator mail (`?entry=`) shows its entry open above the list.

import { useNavigate } from "@tanstack/react-router";
import { useEffect, useId, useRef, useState, type FormEvent } from "react";

import { ApiError } from "../../api/client.ts";
import { useFormErrors } from "../../auth/useFormErrors.ts";
import { FormError } from "../../components/FormError.tsx";
import { ImageIcon } from "../../components/Icons.tsx";
import { ImageLightbox } from "../../components/ImageLightbox.tsx";
import { LoadError } from "../../components/LoadError.tsx";
import { SubmitButton } from "../../components/SubmitButton.tsx";
import { notify } from "../../components/toasts.ts";
import { Badge } from "../../components/ui/Badge.tsx";
import { Button } from "../../components/ui/Button.tsx";
import { ChipGroup } from "../../components/ui/ChipGroup.tsx";
import { fieldDescription } from "../../components/ui/fieldDescription.ts";
import { FormField } from "../../components/ui/FormField.tsx";
import { SkeletonRows } from "../../components/ui/Skeleton.tsx";
import { Textarea } from "../../components/ui/Textarea.tsx";
import { TEXT_MAX, sendFieldErrors } from "../../feedback/rules.ts";
import { translate, useT } from "../../i18n/catalog.ts";
import { feedback } from "../../i18n/catalogs/feedback.ts";
import { operator } from "../../i18n/catalogs/operator.ts";
import { formatDate } from "../../i18n/format.ts";
import { OperatorPageFrame } from "../OperatorPageFrame.tsx";
import * as remote from "../api.ts";
import {
  FEEDBACK_KIND_FILTERS,
  FEEDBACK_STATUSES,
  authorName,
  deleteFeedback,
  feedbackMenuEntries,
  feedbackQuery,
  replyNote,
  senderEmail,
  senderName,
  setFeedbackDone,
  technicalDetails,
  type FeedbackKindFilter,
} from "../feedbackActions.ts";
import { formatWhen } from "../format.ts";
import { usePaging } from "../paging.ts";
import { Pager, RowMenuButton } from "../parts.tsx";
import {
  useOperatorFeedback,
  useOperatorFeedbackEntry,
  useOperatorOverview,
  useOperatorRefresh,
} from "../queries.ts";
import type {
  FeedbackListStatus,
  OperatorFeedback,
  OperatorFeedbackMessage,
} from "../types.ts";

const WHEN: Intl.DateTimeFormatOptions = {
  dateStyle: "medium",
  timeStyle: "short",
};

export function FeedbackPage({ entry }: { entry?: string }) {
  const t = useT(operator);
  const mail = useOperatorOverview().data?.mail ?? false;
  const refresh = useOperatorRefresh();
  const [status, setStatus] = useState<FeedbackListStatus>("open");
  const [kind, setKind] = useState<FeedbackKindFilter>("all");
  const [offset, setOffset, settle] = usePaging(`${status}/${kind}`);
  const query = useOperatorFeedback(feedbackQuery(status, kind, offset));
  const data = query.data;
  settle(
    data && !query.isPlaceholderData
      ? { rows: data.entries.length, total: data.total }
      : undefined,
  );

  return (
    <OperatorPageFrame page="feedback">
      {entry ? (
        <MailEntry key={entry} id={entry} mail={mail} refresh={refresh} />
      ) : null}
      <div className="operator-toolbar operator-toolbar--feedback">
        <ChipGroup
          label={t("feedback.statusLabel")}
          options={FEEDBACK_STATUSES.map((value) => ({
            value,
            label: t(`feedback.status.${value}`),
          }))}
          value={status}
          onChange={setStatus}
        />
        <ChipGroup
          label={t("feedback.kindLabel")}
          options={FEEDBACK_KIND_FILTERS.map((value) => ({
            value,
            label: t(`feedback.kind.${value}`),
          }))}
          value={kind}
          onChange={setKind}
        />
      </div>
      {query.isError && !data ? (
        <LoadError
          message={t("feedback.loadFailed")}
          pending={query.isFetching}
          onRetry={() => void query.refetch()}
        />
      ) : !data ? (
        <SkeletonRows count={4} />
      ) : data.entries.length === 0 ? (
        <p className="operator-empty">{t(`feedback.empty.${status}`)}</p>
      ) : (
        <>
          <ul className="operator-feedback-list">
            {data.entries.map((item) => (
              <li key={item.id}>
                <FeedbackCard entry={item} mail={mail} refresh={refresh} />
              </li>
            ))}
          </ul>
          <Pager offset={offset} total={data.total} onOffset={setOffset} />
        </>
      )}
    </OperatorPageFrame>
  );
}

/** The entry an operator mail links to, open and scrolled into view. */
function MailEntry({
  id,
  mail,
  refresh,
}: {
  id: string;
  mail: boolean;
  refresh: () => void;
}) {
  const t = useT(operator);
  const navigate = useNavigate();
  const query = useOperatorFeedbackEntry(id);
  const box = useRef<HTMLElement>(null);
  const loaded = query.data !== undefined;
  useEffect(() => {
    if (loaded && box.current) revealInPage(box.current);
  }, [loaded]);
  const gone =
    query.error instanceof ApiError && query.error.code === "not_found";
  return (
    <section
      ref={box}
      className="operator-feedback-mail"
      aria-labelledby="operator-feedback-mail"
    >
      <h2 id="operator-feedback-mail" className="operator-feedback-mail-title">
        {t("feedback.fromMail")}
      </h2>
      {query.data ? (
        <FeedbackCard
          entry={query.data}
          mail={mail}
          refresh={refresh}
          defaultOpen
          onDeleted={() =>
            void navigate({
              to: "/betrieb/$page",
              params: { page: "feedback" },
              search: {},
              replace: true,
            })
          }
        />
      ) : gone ? (
        <p className="operator-empty">{t("toasts.feedbackGone")}</p>
      ) : query.isError ? (
        <LoadError
          message={t("feedback.loadFailed")}
          pending={query.isFetching}
          onRetry={() => void query.refetch()}
        />
      ) : (
        <SkeletonRows count={1} />
      )}
    </section>
  );
}

/** Room above the mail's entry once it is scrolled to. */
const MAIL_ENTRY_MARGIN = 16;

/**
 * Scrolls the page's own scroller until `element` starts in view. Not
 * `scrollIntoView`, which also scrolls the shell and hides a phone's top bar.
 */
function revealInPage(element: HTMLElement) {
  const scroller = element.closest(".operator-scroll");
  if (!scroller) return;
  const top =
    element.getBoundingClientRect().top - scroller.getBoundingClientRect().top;
  if (top < 0 || top > scroller.clientHeight / 2)
    scroller.scrollTop += top - MAIL_ENTRY_MARGIN;
}

/**
 * One entry: a header button that opens it, and once open its text, image,
 * technical details, thread, reply form and actions.
 */
export function FeedbackCard({
  entry,
  mail,
  refresh,
  defaultOpen = false,
  onDeleted,
}: {
  entry: OperatorFeedback;
  /** Account mail is on (`GET /api/operator`). */
  mail: boolean;
  refresh: () => void;
  defaultOpen?: boolean;
  onDeleted?: () => void;
}) {
  const t = useT(operator);
  const id = useId();
  const [open, setOpen] = useState(defaultOpen);
  const bodyId = `${id}-body`;
  const email = senderEmail(entry);
  return (
    <article
      className={`operator-feedback${open ? " is-open" : ""}`}
      aria-labelledby={`${id}-head`}
    >
      <h3 className="operator-feedback-heading">
        <button
          type="button"
          id={`${id}-head`}
          className="operator-feedback-head"
          aria-expanded={open}
          aria-controls={open ? bodyId : undefined}
          onClick={() => setOpen((value) => !value)}
        >
          <span className="operator-feedback-line">
            <Badge>{t(`feedback.kind.${entry.kind}`)}</Badge>
            <span className="operator-feedback-sender">
              {entry.sender.type === "deleted" ? (
                <span className="operator-muted">{senderName(entry)}</span>
              ) : (
                <strong>{senderName(entry)}</strong>
              )}
              {email ? <span className="operator-muted"> {email}</span> : null}
            </span>
            {entry.awaiting_reply ? (
              <Badge tone="warning">{t("feedback.awaitingReply")}</Badge>
            ) : null}
            {entry.image ? (
              <span className="operator-feedback-has-image">
                <ImageIcon size={16} />
                <span className="sr-only">{t("feedback.hasImage")}</span>
              </span>
            ) : null}
            <time
              className="operator-feedback-when"
              dateTime={entry.last_activity_at}
              title={formatDate(entry.last_activity_at, WHEN)}
            >
              {formatWhen(entry.last_activity_at)}
            </time>
          </span>
          {open ? null : (
            <span className="operator-feedback-excerpt">{entry.text}</span>
          )}
        </button>
      </h3>
      {open ? (
        <div id={bodyId} className="operator-feedback-body">
          <p className="operator-feedback-text">{entry.text}</p>
          {entry.image ? <EntryImage url={entry.image.url} /> : null}
          <TechnicalDetails entry={entry} />
          {entry.messages.length > 0 ? (
            <ol className="operator-feedback-thread">
              {entry.messages.map((message) => (
                <ThreadMessage
                  key={message.id}
                  entry={entry}
                  message={message}
                />
              ))}
            </ol>
          ) : null}
          <ReplyForm entry={entry} mail={mail} refresh={refresh} />
          <EntryActions entry={entry} refresh={refresh} onDeleted={onDeleted} />
        </div>
      ) : null}
    </article>
  );
}

function EntryImage({ url }: { url: string }) {
  const t = useT(operator);
  const [enlarged, setEnlarged] = useState(false);
  return (
    <>
      <button
        type="button"
        className="operator-feedback-image"
        onClick={() => setEnlarged(true)}
      >
        <img
          src={url}
          alt={t("feedback.imageAlt")}
          decoding="async"
          loading="lazy"
        />
      </button>
      {enlarged ? (
        <ImageLightbox
          image={{ id: url, src: url, filename: t("feedback.imageAlt") }}
          onClose={() => setEnlarged(false)}
        />
      ) : null}
    </>
  );
}

function TechnicalDetails({ entry }: { entry: OperatorFeedback }) {
  const t = useT(operator);
  return (
    <div className="operator-snapshot operator-feedback-details">
      <p className="operator-snapshot-title">{t("feedback.details")}</p>
      <dl>
        {technicalDetails(entry.context).map((row) => (
          <div key={row.label}>
            <dt>{row.label}</dt>
            <dd>
              {row.value === null ? (
                <>
                  <span aria-hidden="true">{t("cell.none")}</span>
                  <span className="sr-only">{t("cell.noneLabel")}</span>
                </>
              ) : row.code ? (
                <code>{row.value}</code>
              ) : (
                row.value
              )}
            </dd>
          </div>
        ))}
      </dl>
    </div>
  );
}

function ThreadMessage({
  entry,
  message,
}: {
  entry: OperatorFeedback;
  message: OperatorFeedbackMessage;
}) {
  const t = useT(operator);
  return (
    <li
      className={`operator-feedback-message operator-feedback-message--${message.from}`}
    >
      <p className="operator-feedback-meta">
        <strong>{authorName(entry, message)}</strong>
        <time dateTime={message.created_at}>
          {formatDate(message.created_at, WHEN)}
        </time>
        {message.mail === "failed" ? (
          <Badge tone="danger">{t("feedback.mail.failed")}</Badge>
        ) : message.mail ? (
          <span>{t(`feedback.mail.${message.mail}`)}</span>
        ) : null}
      </p>
      <p className="operator-feedback-text">{message.text}</p>
    </li>
  );
}

/** "Antwort" and "Antworten", with what the reply will do under it. */
function ReplyForm({
  entry,
  mail,
  refresh,
}: {
  entry: OperatorFeedback;
  mail: boolean;
  refresh: () => void;
}) {
  const t = useT(operator);
  const form = useRef<HTMLFormElement>(null);
  const [text, setText] = useState("");
  const [pending, setPending] = useState(false);
  const { errors, clearField, setFields, fromError, cooldown } =
    useFormErrors(form);
  const fieldId = `operator-feedback-reply-${entry.id}`;
  const { form: shown, note } = replyNote(entry, mail);
  if (!shown) return <p className="operator-notice">{note}</p>;

  const onSubmit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (pending) return;
    if (!text.trim()) {
      setFields({ text: translate(feedback, "text.required") });
      return;
    }
    setPending(true);
    try {
      await remote.replyFeedback(entry.id, text.trim());
      setText("");
      notify(t("toasts.feedbackReplied"));
    } catch (error) {
      const fields = sendFieldErrors(error);
      if (fields) setFields(fields);
      else fromError(error);
    } finally {
      setPending(false);
      refresh();
    }
  };

  const textError = errors.fields.text;
  return (
    <form
      ref={form}
      onSubmit={onSubmit}
      noValidate
      className="operator-feedback-reply"
    >
      <FormField
        id={fieldId}
        label={t("feedback.replyLabel")}
        hint={note}
        error={textError}
      >
        <Textarea
          id={fieldId}
          rows={3}
          required
          maxLength={TEXT_MAX}
          readOnly={pending}
          value={text}
          onChange={(event) => {
            setText(event.target.value);
            clearField("text");
          }}
          invalid={Boolean(textError)}
          aria-describedby={fieldDescription(fieldId, {
            error: textError,
            hint: note,
          })}
        />
      </FormField>
      <FormError message={errors.form} />
      <div className="operator-feedback-reply-actions">
        <SubmitButton
          size="sm"
          pending={pending}
          pendingLabel={t("feedback.replySending")}
          cooldown={cooldown}
        >
          {t("feedback.replySubmit")}
        </SubmitButton>
      </div>
    </form>
  );
}

/** "Erledigt" or "Wieder öffnen", and the menu with "Löschen…". */
function EntryActions({
  entry,
  refresh,
  onDeleted,
}: {
  entry: OperatorFeedback;
  refresh: () => void;
  onDeleted?: () => void;
}) {
  const t = useT(operator);
  const [pending, setPending] = useState(false);
  const done = entry.status === "done";
  return (
    <div className="operator-feedback-actions">
      <Button
        size="sm"
        pending={pending}
        pendingLabel={t(done ? "feedback.reopen" : "feedback.done")}
        onClick={() => {
          setPending(true);
          void setFeedbackDone(entry, !done, refresh).finally(() =>
            setPending(false),
          );
        }}
      >
        {t(done ? "feedback.reopen" : "feedback.done")}
      </Button>
      <RowMenuButton
        label={t("row.menuLabel", { name: senderName(entry) })}
        entries={feedbackMenuEntries({
          delete: () =>
            void deleteFeedback(entry, refresh).then((deleted) => {
              if (deleted) onDeleted?.();
            }),
        })}
      />
    </div>
  );
}
