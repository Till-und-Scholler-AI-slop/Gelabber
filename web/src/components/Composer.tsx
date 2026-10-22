// The message composer: draft, attachments, GIFs, mentions, and the
// optimistic send.

import {
  useCallback,
  useEffect,
  useEffectEvent,
  useImperativeHandle,
  useLayoutEffect,
  useRef,
  useState,
  type FormEvent,
  type KeyboardEvent,
  type Ref,
} from "react";
import { setReplyTarget, useReplyTarget } from "../messages/replyTarget.ts";
import { useChatDraft } from "../messages/drafts.ts";
import { fieldMessage } from "../auth/rules.ts";
import { discardAttempt, type SendAttempt } from "../messages/pending.ts";
import { useSendMessage } from "../messages/queries.ts";
import {
  carriesFiles,
  droppedFiles,
  pastedFiles,
} from "../messages/clipboard.ts";
import {
  ACCEPT,
  CONTENT_MAX,
  formatSize,
  validateAttachment,
  validateAttachmentSet,
  validateContent,
} from "../messages/rules.ts";
import { type GifPick } from "../gifs/api.ts";
import { useGifStatus } from "../gifs/queries.ts";
import GifPicker from "./GifPicker.tsx";
import { sizeByDraft, type DraftHeight } from "./composerHeight.ts";
import { EmojiPickerLoader } from "./ReactionBar.tsx";
import { Modal } from "./Modal.tsx";
import {
  CloseIcon,
  PaperclipIcon,
  ReplyIcon,
  SendIcon,
  SmileIcon,
  WarningIcon,
} from "./Icons.tsx";
import { useMentions } from "./mentionContext.ts";
import { useMentionSuggestions } from "./useMentionSuggestions.tsx";
import { fromWire, toWire } from "../messages/mentions.ts";
import { type ComposerHandle } from "./messagePaneContext.ts";

export function Composer({
  handle,
  channelId,
  channelName,
  canSend,
  canSendFiles,
  mention,
  author,
  attempts,
  onSent,
  onShowAttempt,
  onDraftChange,
  onDraftStop,
}: {
  handle: Ref<ComposerHandle>;
  channelId: string;
  channelName: string;
  canSend: boolean;
  canSendFiles: boolean;
  mention: "#" | "@";
  author: { id: string; name: string; avatar_url: string | null } | null;
  attempts: SendAttempt[];
  /** A message left: the list shows the end, where it appears. */
  onSent: () => void;
  /** Scrolls to the row of a send that did not go through. */
  onShowAttempt: (id: string) => void;
  onDraftChange?: (value: string) => void;
  onDraftStop?: () => void;
}) {
  const send = useSendMessage(
    channelId,
    author ?? { id: "", name: "", avatar_url: null },
  );
  const [savedDraft, updateDraft, previews] = useChatDraft(
    author?.id ?? "",
    channelId,
  );
  const draft = savedDraft.text;
  const files = savedDraft.files;
  const setDraft = (text: string) => updateDraft({ text });
  const setFiles = (files: File[]) => updateDraft({ files });
  const [fileError, setFileError] = useState<string | null>(null);
  const [emojiOpen, setEmojiOpen] = useState(false);
  const [gifOpen, setGifOpen] = useState(false);
  const gifs = useGifStatus();
  const gifsEnabled = gifs.data?.enabled === true;
  const replyTarget = useReplyTarget(channelId);
  const fileInput = useRef<HTMLInputElement>(null);
  const composerInput = useRef<HTMLTextAreaElement>(null);
  const composerForm = useRef<HTMLFormElement>(null);
  const keepKeyboard = useRef(false);
  const revealComposerFocus = useCallback(() => {
    const form = composerForm.current;
    const pane = form?.closest<HTMLElement>(".lr-message-pane");
    const focused = document.activeElement;
    if (
      !form ||
      !pane ||
      !(focused instanceof HTMLElement) ||
      !form.contains(focused)
    )
      return;
    const boundary = pane.getBoundingClientRect();
    const target = focused.getBoundingClientRect();
    const focusMargin = 4;
    if (target.bottom + focusMargin > boundary.bottom)
      pane.scrollTop += target.bottom + focusMargin - boundary.bottom;
    else if (target.top - focusMargin < boundary.top)
      pane.scrollTop -= boundary.top - target.top + focusMargin;
  }, []);
  // "Antworten" moves the keyboard to the field it fills.
  useEffect(() => {
    if (replyTarget) composerInput.current?.focus({ preventScroll: true });
  }, [replyTarget]);
  // Toolbar/read-state changes can move a focused form without resizing it.
  useLayoutEffect(revealComposerFocus, [revealComposerFocus]);
  useLayoutEffect(() => {
    const form = composerForm.current;
    const pane = form?.closest<HTMLElement>(".lr-message-pane");
    if (!form || !pane) return;
    const observer = new ResizeObserver(revealComposerFocus);
    observer.observe(form);
    observer.observe(pane);
    revealComposerFocus();
    return () => observer.disconnect();
  }, [canSend, revealComposerFocus]);
  // chat.css sizes the field by its draft on phones. Browsers without
  // field-sizing (iOS before 26.2) get the same height from composerHeight.
  const draftHeight = useRef<DraftHeight | null>(null);
  useLayoutEffect(() => {
    const input = composerInput.current;
    const form = composerForm.current;
    if (!input || !form) return;
    const sizing = sizeByDraft(window, input, form);
    draftHeight.current = sizing;
    return () => {
      draftHeight.current = null;
      sizing?.stop();
    };
  }, [canSend]);
  useLayoutEffect(() => draftHeight.current?.fit(), [draft, canSend]);
  const { members } = useMentions();
  const suggestions = useMentionSuggestions(draft, (text, caret) => {
    setDraft(text);
    onDraftChange?.(text);
    requestAnimationFrame(() =>
      composerInput.current?.setSelectionRange(caret, caret),
    );
  });
  // The field shows `@Name`; what is sent and counted is `<@id>`.
  const wire = toWire(draft, members, suggestions.picked);
  const error = draft.length === 0 ? null : validateContent(wire);
  const remaining = CONTENT_MAX - Array.from(normalisedLength(wire)).length;
  const emptyText = draft.trim().length === 0;
  const disabled =
    !canSend ||
    !author ||
    Boolean(error) ||
    Boolean(files.length > 0 && !canSendFiles) ||
    (emptyText && files.length === 0);

  /** Add files to `base` (the composer's files): invalid ones are left
   *  out, and so is whatever would go over the message's limits. One note
   *  names every file left out. */
  const addFiles = (next: File[], base: File[] = files) => {
    setFileError(null);
    let kept = [...base];
    const notes: string[] = [];
    const over = new Map<string, string[]>();
    for (const candidate of next) {
      const invalid = validateAttachment(candidate);
      if (invalid) {
        notes.push(
          `${candidate.name}: ${fieldMessage(invalid.field, invalid.code)}`,
        );
        continue;
      }
      // A smaller file further on may still fit under the size limit.
      const limit = validateAttachmentSet([...kept, candidate]);
      if (limit) {
        const reason = fieldMessage(limit.field, limit.code);
        over.set(reason, [...(over.get(reason) ?? []), candidate.name]);
        continue;
      }
      kept = [...kept, candidate];
    }
    for (const [reason, names] of over)
      notes.push(`Nicht angehängt: ${names.join(", ")}. ${reason}`);
    if (notes.length > 0) setFileError(notes.join(" "));
    setFiles(kept);
  };
  const removeFile = (index: number) => {
    setFileError(null);
    setFiles(files.filter((_, i) => i !== index));
  };

  // Files dropped anywhere on the conversation attach like the paperclip.
  const [dropping, setDropping] = useState(false);
  const attachDropped = useEffectEvent((next: File[]) => {
    addFiles(next);
    composerInput.current?.focus();
  });
  useEffect(() => {
    const pane = composerForm.current?.closest<HTMLElement>(".lr-message-pane");
    if (!pane || !canSendFiles) return;
    // enter/leave fire for every child the pointer crosses.
    let depth = 0;
    const show = (on: boolean) => {
      pane.classList.toggle("is-drop-target", on);
      setDropping(on);
    };
    const enter = (event: DragEvent) => {
      if (!carriesFiles(event.dataTransfer)) return;
      event.preventDefault();
      depth += 1;
      show(true);
    };
    const over = (event: DragEvent) => {
      if (!carriesFiles(event.dataTransfer) || !event.dataTransfer) return;
      event.preventDefault();
      event.dataTransfer.dropEffect = "copy";
    };
    const leave = (event: DragEvent) => {
      if (!carriesFiles(event.dataTransfer)) return;
      depth = Math.max(0, depth - 1);
      if (depth === 0) show(false);
    };
    const drop = (event: DragEvent) => {
      if (!carriesFiles(event.dataTransfer)) return;
      event.preventDefault();
      depth = 0;
      show(false);
      const dropped = droppedFiles(event.dataTransfer);
      if (dropped.length > 0) attachDropped(dropped);
    };
    pane.addEventListener("dragenter", enter);
    pane.addEventListener("dragover", over);
    pane.addEventListener("dragleave", leave);
    pane.addEventListener("drop", drop);
    return () => {
      pane.removeEventListener("dragenter", enter);
      pane.removeEventListener("dragover", over);
      pane.removeEventListener("dragleave", leave);
      pane.removeEventListener("drop", drop);
      show(false);
    };
  }, [canSend, canSendFiles]);

  const submit = (event?: FormEvent) => {
    event?.preventDefault();
    if (disabled) return;
    const text = wire;
    send.mutate({ content: text, files, replyTo: replyTarget });
    onSent();
    setReplyTarget(channelId, null);
    updateDraft({ text: "", files: [] });
    setFileError(null);
    onDraftStop?.();
    if (keepKeyboard.current) composerInput.current?.focus();
    keepKeyboard.current = false;
  };

  const sendGif = (pick: GifPick) => {
    setGifOpen(false);
    if (!canSend || !author) return;
    send.mutate({ content: "", gif: pick, replyTo: replyTarget });
    onSent();
    setReplyTarget(channelId, null);
    composerInput.current?.focus();
  };
  // "Bearbeiten" on a row that was not sent: its text and files come back
  // here, after anything written since, which must not be lost.
  useImperativeHandle(handle, () => ({
    takeBack: (attempt) => {
      const text = fromWire(attempt.content, members);
      setDraft(draft.trim() ? `${draft}\n${text}` : text);
      addFiles(attempt.files);
      discardAttempt(attempt.id);
      composerInput.current?.focus();
    },
  }));
  const unsent = attempts.filter((attempt) => attempt.status !== "sending");

  const insertText = (text: string) => {
    const input = composerInput.current;
    const start = input?.selectionStart ?? draft.length;
    const end = input?.selectionEnd ?? draft.length;
    const next = draft.slice(0, start) + text + draft.slice(end);
    setDraft(next);
    onDraftChange?.(next);
    requestAnimationFrame(() => {
      input?.focus();
      input?.setSelectionRange(start + text.length, start + text.length);
    });
  };

  const onKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (suggestions.onKeyDown(event)) return;
    if (event.key === "Escape" && replyTarget) {
      event.preventDefault();
      setReplyTarget(channelId, null);
      return;
    }
    if (
      event.key === "Enter" &&
      !event.shiftKey &&
      !event.nativeEvent.isComposing
    ) {
      event.preventDefault();
      submit();
    }
  };

  if (!canSend) {
    return (
      <p className="lr-composer-readonly">
        Du kannst in diesem Kanal nicht schreiben.
      </p>
    );
  }

  return (
    <form
      ref={composerForm}
      onFocusCapture={revealComposerFocus}
      onSubmit={submit}
      className="lr-composer"
    >
      {unsent.length > 0 ? (
        <div role="alert" className="lr-send-summary">
          <WarningIcon size={14} />
          <span>
            {unsent.length === 1
              ? "1 Nachricht nicht gesendet"
              : `${unsent.length} Nachrichten nicht gesendet`}
          </span>
          <button type="button" onClick={() => onShowAttempt(unsent[0]!.id)}>
            Anzeigen
          </button>
        </div>
      ) : null}
      <label className="sr-only" htmlFor={`compose-${channelId}`}>
        Nachricht an {mention}
        {channelName}
      </label>
      <div className="lr-composer-field">
        {replyTarget ? (
          <div className="lr-composer-reply">
            <ReplyIcon size={14} />
            <p>
              Antwort an <strong>{replyTarget.author.name}</strong>
              <span className="lr-composer-reply-hint"> · Esc bricht ab</span>
            </p>
            <button
              type="button"
              onClick={() => {
                setReplyTarget(channelId, null);
                composerInput.current?.focus();
              }}
              aria-label="Antworten abbrechen"
              title="Antworten abbrechen"
            >
              <CloseIcon size={14} />
            </button>
          </div>
        ) : null}
        {dropping ? (
          <p className="lr-composer-drop" aria-hidden="true">
            Loslassen zum Anhängen
          </p>
        ) : null}
        {files.length > 0 ? (
          <ul className="lr-composer-attachments" aria-label="Anhänge">
            {files.map((attached, index) => {
              const preview = previews.get(attached);
              return (
                <li
                  key={`${index}:${attached.name}:${attached.size}:${attached.lastModified}`}
                  className="lr-composer-attachment"
                >
                  {preview ? <img src={preview} alt="" /> : null}
                  <div>
                    <p>{attached.name}</p>
                    <p>{formatSize(attached.size)}</p>
                  </div>
                  <button
                    type="button"
                    onClick={() => removeFile(index)}
                    aria-label={`${attached.name} entfernen`}
                    title="Anhang entfernen"
                  >
                    <CloseIcon size={14} />
                  </button>
                </li>
              );
            })}
          </ul>
        ) : null}
        {suggestions.list}
        <div className="lr-composer-row">
          {canSendFiles ? (
            <>
              <input
                ref={fileInput}
                type="file"
                accept={ACCEPT}
                multiple
                className="sr-only"
                onChange={(event) => {
                  const selected = Array.from(event.target.files ?? []);
                  addFiles(selected);
                  event.target.value = "";
                  if (selected.length > 0) composerInput.current?.focus();
                }}
              />
              <button
                type="button"
                title="Datei anhängen"
                aria-label="Datei anhängen"
                onClick={() => fileInput.current?.click()}
                className="lr-composer-icon"
              >
                <PaperclipIcon size={18} />
              </button>
            </>
          ) : null}
          <textarea
            ref={composerInput}
            id={`compose-${channelId}`}
            value={draft}
            {...suggestions.fieldProps}
            onChange={(e) => {
              setDraft(e.target.value);
              onDraftChange?.(e.target.value);
              suggestions.track(e.target);
            }}
            onBlur={() => {
              suggestions.fieldProps.onBlur();
              onDraftStop?.();
            }}
            onKeyDown={onKeyDown}
            onPaste={(event) => {
              if (!canSendFiles) return;
              const pasted = pastedFiles(event.clipboardData);
              if (pasted.length === 0) return;
              event.preventDefault();
              addFiles(pasted);
            }}
            rows={1}
            placeholder={`Nachricht an ${mention}${channelName}`}
          />
          <button
            type="button"
            title="Emoji einfügen"
            aria-label="Emoji einfügen"
            onClick={() => setEmojiOpen(true)}
            className="lr-composer-icon"
          >
            <SmileIcon size={18} />
          </button>
          {gifsEnabled ? (
            <button
              type="button"
              title="GIF senden"
              aria-label="GIF senden"
              onClick={() => setGifOpen(true)}
              className="lr-composer-icon lr-composer-gif"
            >
              <span aria-hidden="true">GIF</span>
            </button>
          ) : null}
          <button
            type="submit"
            disabled={disabled}
            aria-label="Senden"
            title="Senden"
            // A tap would move focus to the button and close the on-screen
            // keyboard after every message.
            onPointerDown={(event) => {
              keepKeyboard.current =
                event.pointerType !== "mouse" &&
                document.activeElement === composerInput.current;
              if (keepKeyboard.current) event.preventDefault();
            }}
            className="lr-composer-send"
          >
            <SendIcon size={17} />
          </button>
        </div>
      </div>
      <Modal
        open={emojiOpen}
        onClose={() => setEmojiOpen(false)}
        title="Emoji einfügen"
        wide
      >
        {emojiOpen ? (
          <EmojiPickerLoader
            onSelect={(emoji) => {
              setEmojiOpen(false);
              insertText(emoji);
            }}
          />
        ) : null}
      </Modal>
      {gifsEnabled ? (
        <Modal
          open={gifOpen}
          onClose={() => setGifOpen(false)}
          title="GIF senden"
          wide
        >
          {gifOpen ? <GifPicker onSelect={sendGif} /> : null}
        </Modal>
      ) : null}
      <div className="lr-composer-status" role="status">
        {remaining < 200 ? (
          <span
            className={remaining < 0 ? "lr-field-error" : undefined}
            // Read out only near the limit, not with every letter.
            aria-live={remaining < 20 ? "polite" : "off"}
          >
            {remaining < 0
              ? `${-remaining} Zeichen zu viel`
              : `${remaining} Zeichen übrig`}
          </span>
        ) : null}
        {error ? (
          <span className="lr-field-error">
            {fieldMessage("content", error)}
          </span>
        ) : null}
      </div>
      {fileError ? (
        <p role="alert" className="lr-composer-file-error lr-field-error">
          {fileError}
        </p>
      ) : null}
    </form>
  );
}

function normalisedLength(raw: string): string {
  return raw.replace(/\r\n/g, "\n").replace(/\r/g, "\n");
}
