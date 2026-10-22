// One message in the list: content, reactions, own edit/delete, and the
// state of a send that is still on its way or did not arrive.

import {
  useContext,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type MouseEvent,
  type PointerEvent,
  type ReactNode,
  type Ref,
} from "react";
import { MessageContent } from "../messages/MessageContent.tsx";
import { setReplyTarget } from "../messages/replyTarget.ts";
import { fieldMessage } from "../auth/rules.ts";
import { type Arrival } from "../messages/arrivals.ts";
import { discardAttempt, type SendAttempt } from "../messages/pending.ts";
import { isPendingId } from "../messages/queries.ts";
import { validateContent } from "../messages/rules.ts";
import type { Gif, Message, Reply } from "../messages/types.ts";
import { asGif, asReactionList, asReply } from "../messages/types.ts";
import { useReducedMotion } from "../interface/useReducedMotion.ts";
import { GifMessageButton } from "./GifPicker.tsx";
import { Avatar } from "./Avatar.tsx";
import { ReactionBar } from "./ReactionBar.tsx";
import { useReaction } from "./useReaction.ts";
import { LinkPreviews } from "./LinkPreviews.tsx";
import {
  MoreIcon,
  PaperclipIcon,
  PencilIcon,
  ReplyIcon,
  WarningIcon,
} from "./Icons.tsx";
import {
  MessageActionSheet,
  type MessageActions,
} from "./MessageActionSheet.tsx";
import { longPressGesture, swallowNextClick } from "./longPress.ts";
import { Button } from "./ui/Button.tsx";
import { copyText } from "../servers/clipboard.ts";
import { notify, notifyError } from "./toasts.ts";
import { AttachmentList } from "./Attachments.tsx";
import { useMentionRenderer } from "./mentionRenderer.tsx";
import { useMentions } from "./mentionContext.ts";
import { useMentionSuggestions } from "./useMentionSuggestions.tsx";
import {
  fromWire,
  namesIn,
  mentionsMe,
  readableMentions,
  toWire,
} from "../messages/mentions.ts";
import { AttemptActionsContext, JumpTo } from "./messagePaneContext.ts";
import { formatClock, formatFull, formatWhen } from "./messageTime.ts";

export function MessageRow({
  message,
  continued,
  arrival,
  mine,
  canSend,
  canDelete,
  attempt,
  onEdit,
  onDelete,
}: {
  message: Message;
  continued: boolean;
  /** Just came in live: fades in once, as it mounts. */
  arrival?: Arrival;
  mine: boolean;
  canSend: boolean;
  canDelete: boolean;
  /** The send attempt of a `tmp:` row. */
  attempt?: SendAttempt;
  /** `failed` runs if the server turns the change down. */
  onEdit: (content: string, failed: () => void) => void;
  /** `skipConfirm` with Shift held. */
  onDelete: (skipConfirm: boolean) => void;
}) {
  const pending = isPendingId(message.id);
  // Kept from the mount: a later render, or the same row scrolled back
  // into view, does not play it again.
  const [arrivedAs] = useState(arrival);
  const { members, me } = useMentions();
  const mentioned = !mine && mentionsMe(message, me);
  const renderMention = useMentionRenderer(message);
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(message.content);
  /** Whether the editor takes focus when it opens. */
  const [editFocus, setEditFocus] = useState(true);
  const editInput = useRef<HTMLTextAreaElement>(null);
  const editSuggestions = useMentionSuggestions(draft, (text, caret) => {
    setDraft(text);
    requestAnimationFrame(() =>
      editInput.current?.setSelectionRange(caret, caret),
    );
  });
  const sentNames = useMemo(
    () => namesIn(message.content, members),
    [message.content, members],
  );
  const wireDraft = toWire(
    draft,
    members,
    new Map([...sentNames, ...editSuggestions.picked]),
  );
  const rowRef = useRef<HTMLDivElement>(null);
  const editOpener = useRef<HTMLElement | null>(null);
  const restoreEditFocus = useRef(false);
  useLayoutEffect(() => {
    if (!editing && restoreEditFocus.current) {
      restoreEditFocus.current = false;
      const opener = editOpener.current;
      (opener?.isConnected ? opener : rowRef.current)?.focus({
        preventScroll: true,
      });
    }
  }, [editing]);
  const finishEditing = () => {
    restoreEditFocus.current = true;
    setEditing(false);
  };

  const startEdit = () => {
    restoreEditFocus.current = false;
    // Back to the edit button afterwards, or to the row when the edit came
    // from the action sheet.
    const active = document.activeElement;
    editOpener.current =
      active instanceof HTMLElement && rowRef.current?.contains(active)
        ? active
        : rowRef.current;
    setDraft(fromWire(message.content, members));
    setEditFocus(true);
    setEditing(true);
  };
  const cancelEdit = () => {
    finishEditing();
    setDraft(fromWire(message.content, members));
  };
  const saveEdit = () => {
    // The row shows the change at once. Turned down, it rolls back, and the
    // editor opens again with what was typed instead of losing it; focus
    // comes along only if the reader is not busy elsewhere by then.
    const typed = draft;
    onEdit(wireDraft, () => {
      const active = document.activeElement;
      setEditFocus(
        !active ||
          active === document.body ||
          Boolean(rowRef.current?.contains(active)),
      );
      restoreEditFocus.current = false;
      setDraft(typed);
      setEditing(true);
    });
    finishEditing();
  };

  const moreButton = useRef<HTMLButtonElement>(null);
  // The actions menu, at the "Mehr" button or for the whole row (sheet).
  const [menu, setMenu] = useState<"more" | "row" | null>(null);
  const [gesture] = useState(() =>
    longPressGesture(() => {
      navigator.vibrate?.(10);
      swallowNextClick();
      setMenu("row");
    }),
  );
  const reaction = useReaction(message, canSend);

  const when = formatWhen(message.created_at);
  const fullWhen = formatFull(message.created_at);
  const error = editing
    ? draft.length === 0
      ? message.attachments.length > 0
        ? null
        : validateContent(wireDraft)
      : validateContent(wireDraft)
    : null;
  // A `tmp:` row is being sent until its attempt says otherwise.
  const sendState = pending ? (attempt?.status ?? "sending") : null;
  const showDelete = canDelete && !pending && !editing;
  const gif = asGif(message.gif);
  const showEdit = mine && canSend && !pending && !editing && !gif;
  const canReact = canSend && !pending && !editing;
  const canReply = canSend && !pending && !editing;
  const reply = asReply(message.reply);
  const [reactionSlot, setReactionSlot] = useState<HTMLSpanElement | null>(
    null,
  );

  const copy = async () => {
    if (await copyText(fromWire(message.content, members)))
      notify("Text kopiert.");
    else notifyError(new Error("Der Text konnte nicht kopiert werden."));
  };
  const menuActions: MessageActions =
    pending || editing
      ? {}
      : {
          react: canReact ? reaction.toggle : undefined,
          moreReactions: canReact ? () => reaction.setPicking(true) : undefined,
          reply: canReply
            ? () => setReplyTarget(message.channel_id, message)
            : undefined,
          edit: showEdit ? startEdit : undefined,
          copy: message.content ? () => void copy() : undefined,
          remove: showDelete ? onDelete : undefined,
        };
  const hasMenu =
    !pending &&
    !editing &&
    (canReact ||
      canReply ||
      showEdit ||
      showDelete ||
      Boolean(message.content));
  const myReactions = asReactionList(message.reactions)
    .filter((entry) => entry.user_ids.includes(reaction.userId))
    .map((entry) => entry.emoji);

  // A finger held on the row opens the sheet; a right click the menu,
  // except on a link, a picture or selected text, which keep the browser's.
  // Not in what the row opens over the page (the menu, the emoji picker's
  // dialog): React passes their events up to the row too.
  const ownTarget = (event: { currentTarget: Element; target: EventTarget }) =>
    event.target instanceof Element &&
    event.currentTarget.contains(event.target) &&
    event.target.closest("dialog") === event.currentTarget.closest("dialog");
  const menuHandlers = hasMenu
    ? {
        onPointerDown: (event: PointerEvent<HTMLDivElement>) => {
          if (
            !ownTarget(event) ||
            (event.target as Element).closest("textarea, input")
          )
            return;
          gesture.down({
            pointerType: event.pointerType,
            isPrimary: event.isPrimary,
            x: event.clientX,
            y: event.clientY,
          });
        },
        onPointerMove: (event: PointerEvent<HTMLDivElement>) =>
          gesture.move({ x: event.clientX, y: event.clientY }),
        onPointerUp: gesture.end,
        onPointerCancel: gesture.end,
        onPointerLeave: gesture.end,
        onContextMenu: (event: MouseEvent<HTMLDivElement>) => {
          if (!ownTarget(event)) return;
          // Already open, e.g. by the long press before a browser's own
          // contextmenu for the same finger.
          if (menu !== null) {
            event.preventDefault();
            return;
          }
          const pointer = (
            event.nativeEvent as Partial<globalThis.PointerEvent>
          ).pointerType;
          // Android's own long press: the gesture above answers it.
          if (pointer === "touch" || pointer === "pen") {
            event.preventDefault();
            return;
          }
          if (
            (event.target as Element).closest(
              "a, img, video, audio, input, textarea",
            ) ||
            String(window.getSelection() ?? "").trim()
          )
            return;
          event.preventDefault();
          setMenu("more");
        },
      }
    : {};

  const actions = hasMenu ? (
    <span className="lr-message-actions">
      <span ref={setReactionSlot} className="lr-message-reaction-slot" />
      {canReply ? (
        <IconButton
          label="Antworten"
          onClick={() => setReplyTarget(message.channel_id, message)}
        >
          <ReplyIcon size={14} />
        </IconButton>
      ) : null}
      {showEdit ? (
        <IconButton label="Nachricht bearbeiten" onClick={startEdit}>
          <PencilIcon size={14} />
        </IconButton>
      ) : null}
      <IconButton
        buttonRef={moreButton}
        label="Weitere Aktionen"
        menu={menu === "more"}
        onClick={() => setMenu(menu === "more" ? null : "more")}
      >
        <MoreIcon size={14} />
      </IconButton>
    </span>
  ) : null;
  const sheet = hasMenu ? (
    <MessageActionSheet
      open={menu !== null}
      onClose={() => setMenu(null)}
      anchor={menu === "row" ? rowRef : moreButton}
      author={message.author.name}
      actions={menuActions}
      mine={myReactions}
    />
  ) : null;
  const status = sendState ? <SendStatus state={sendState} /> : null;
  const failure =
    attempt && attempt.status !== "sending" ? (
      <SendFailure attempt={attempt} />
    ) : null;
  const edited = message.edited_at ? (
    <span
      className="lr-message-edited"
      title={`Bearbeitet am ${formatFull(message.edited_at)}`}
    >
      (bearbeitet)
    </span>
  ) : null;
  const rowClass = [
    "lr-message-row group flex items-start gap-3",
    continued ? "lr-message-continued" : "",
    sendState === "sending" ? "is-sending" : "",
    sendState === "failed" || sendState === "uncertain" ? "is-failed" : "",
    mentioned ? "is-mentioned" : "",
  ].join(" ");

  if (continued && !editing) {
    return (
      <div
        ref={rowRef}
        tabIndex={-1}
        className={rowClass}
        data-arrival={arrivedAs}
        {...menuHandlers}
      >
        <span className="lr-message-gutter" aria-hidden="true">
          {sendState ? null : (
            <time dateTime={message.created_at} title={fullWhen}>
              {formatClock(message.created_at)}
            </time>
          )}
        </span>
        <div className="min-w-0 flex-1">
          <span className="sr-only">
            {message.author.name}, {when}
          </span>
          {message.content ? (
            <div className="lr-message-text">
              <MessageContent
                content={message.content}
                renderMention={renderMention}
              />
              {edited}
            </div>
          ) : null}
          {gif ? <GifMessage gif={gif} /> : null}
          <AttachmentList attachments={message.attachments ?? []} />
          <LinkPreviews
            messageId={message.id}
            previews={message.previews}
            canRemove={canDelete && !pending}
          />
          <ReactionBar
            message={message}
            canSend={canSend}
            reaction={reaction}
            addSlot={reactionSlot}
          />
          {status ? <p className="lr-message-status-line">{status}</p> : null}
          {failure}
        </div>
        {actions}
        {sheet}
      </div>
    );
  }

  return (
    <div
      ref={rowRef}
      tabIndex={-1}
      className={rowClass}
      data-arrival={arrivedAs}
      {...menuHandlers}
    >
      <Avatar name={message.author.name} url={message.author.avatar_url} />
      <div className="min-w-0 flex-1">
        {reply ? <ReplyHeader reply={reply} /> : null}
        <div className="flex flex-wrap items-baseline gap-2">
          <span className="lr-message-author">{message.author.name}</span>
          {status ?? (
            <time
              dateTime={message.created_at}
              title={fullWhen}
              className="lr-message-time"
            >
              {when}
            </time>
          )}
        </div>
        {editing ? (
          <form
            className="lr-message-edit"
            onSubmit={(event) => {
              event.preventDefault();
              if (error) return;
              saveEdit();
            }}
          >
            {editSuggestions.list}
            <textarea
              ref={editInput}
              value={draft}
              {...editSuggestions.fieldProps}
              onChange={(e) => {
                setDraft(e.target.value);
                editSuggestions.track(e.target);
              }}
              onKeyDown={(event) => {
                if (editSuggestions.onKeyDown(event)) return;
                if (event.key === "Escape") {
                  event.preventDefault();
                  cancelEdit();
                }
                if (
                  event.key === "Enter" &&
                  !event.shiftKey &&
                  !event.nativeEvent.isComposing
                ) {
                  event.preventDefault();
                  if (!error) saveEdit();
                }
              }}
              rows={2}
              aria-label="Nachricht bearbeiten"
              autoFocus={editFocus}
            />
            {error ? (
              <p className="lr-field-error">{fieldMessage("content", error)}</p>
            ) : null}
            <div className="lr-message-edit-actions">
              <span>Esc bricht ab · Enter speichert</span>
              <button type="button" onClick={cancelEdit}>
                Abbrechen
              </button>
              <button type="submit" disabled={Boolean(error)}>
                Speichern
              </button>
            </div>
          </form>
        ) : message.content || !gif ? (
          <div className="lr-message-text">
            <MessageContent
              content={message.content}
              renderMention={renderMention}
            />
            {edited}
          </div>
        ) : null}
        {!editing && gif ? <GifMessage gif={gif} /> : null}
        {!editing ? (
          <AttachmentList attachments={message.attachments ?? []} />
        ) : null}
        {!editing ? (
          <LinkPreviews
            messageId={message.id}
            previews={message.previews}
            canRemove={canDelete && !pending}
          />
        ) : null}
        {!editing && (
          <ReactionBar
            message={message}
            canSend={canSend}
            reaction={reaction}
            addSlot={reactionSlot}
          />
        )}
        {failure}
      </div>
      {actions}
      {sheet}
    </div>
  );
}

/** In place of the time while a message is on its way or did not arrive. */
function SendStatus({ state }: { state: SendAttempt["status"] }) {
  if (state === "sending")
    return (
      <span className="lr-message-status is-sending">
        <span className="lr-message-spinner" aria-hidden="true" />
        Wird gesendet…
      </span>
    );
  return (
    <span className="lr-message-status is-failed">
      <WarningIcon size={13} />
      {state === "failed" ? "Nicht gesendet" : "Status unklar – Verlauf prüfen"}
    </span>
  );
}

/** Why a message was not sent, and what to do about it, on its row. */
function SendFailure({ attempt }: { attempt: SendAttempt }) {
  const actions = useContext(AttemptActionsContext);
  return (
    <div className="lr-message-failure">
      <p>
        {attempt.error}
        {attempt.status === "uncertain"
          ? " Die Nachricht kann bereits gespeichert sein. Prüfe den Verlauf, bevor du sie erneut sendest."
          : null}
      </p>
      <div className="lr-message-failure-actions">
        {attempt.status === "failed" && actions ? (
          <Button
            size="sm"
            variant="secondary"
            disabled={attempt.files.length > 0 && !actions.canSendFiles}
            onClick={() => actions.retry(attempt)}
          >
            Erneut senden
          </Button>
        ) : null}
        {attempt.gif || !actions ? null : (
          <Button
            size="sm"
            variant="ghost"
            onClick={() => actions.edit(attempt)}
          >
            Bearbeiten
          </Button>
        )}
        <Button
          size="sm"
          variant="ghost"
          onClick={() => discardAttempt(attempt.id)}
        >
          Verwerfen
        </Button>
      </div>
    </div>
  );
}

function ReplyHeader({ reply }: { reply: Reply }) {
  const jumpTo = useContext(JumpTo);
  const { members } = useMentions();
  if (reply.deleted || !reply.author)
    return (
      <p className="lr-reply-header is-deleted">
        <ReplyIcon size={13} />
        <span>Die ursprüngliche Nachricht wurde gelöscht.</span>
      </p>
    );
  const summary =
    (reply.excerpt && readableMentions(reply.excerpt, members)) ||
    (reply.has_gif ? "GIF" : reply.has_attachment ? "Anhang" : "");
  return (
    <button
      type="button"
      className="lr-reply-header"
      onClick={() => jumpTo(reply.id)}
      aria-label={`Antwort auf ${reply.author.name}: ${summary}. Zur Nachricht springen`}
    >
      <ReplyIcon size={13} />
      <span className="lr-reply-author">{reply.author.name}</span>
      <span className="lr-reply-excerpt">
        {summary}
        {reply.excerpt && (reply.has_attachment || reply.has_gif) ? (
          <PaperclipIcon size={12} />
        ) : null}
      </span>
    </button>
  );
}

function GifMessage({ gif }: { gif: Gif }) {
  return <GifMessageButton gif={gif} reduced={useReducedMotion()} />;
}

function IconButton({
  buttonRef,
  label,
  onClick,
  children,
  menu,
}: {
  buttonRef?: Ref<HTMLButtonElement>;
  label: string;
  onClick: () => void;
  children: ReactNode;
  /** Opens a menu: whether it is open. */
  menu?: boolean;
}) {
  return (
    <button
      ref={buttonRef}
      type="button"
      title={label}
      aria-label={label}
      aria-haspopup={menu === undefined ? undefined : "menu"}
      aria-expanded={menu}
      onClick={onClick}
      className="lr-message-action"
    >
      {children}
    </button>
  );
}
