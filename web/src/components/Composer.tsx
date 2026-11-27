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
  type ChangeEvent,
  type FormEvent,
  type KeyboardEvent,
  type Ref,
  type RefObject,
} from "react";
import { setReplyTarget, useReplyTarget } from "../messages/replyTarget.ts";
import { useChatDraft } from "../messages/drafts.ts";
import { fieldMessage } from "../auth/rules.ts";
import { discardAttempt, type SendAttempt } from "../messages/pending.ts";
import { useSendMessage } from "../messages/queries.ts";
import { altTextOf, setAltText } from "../messages/altText.ts";
import { AltTextDialog } from "./AltTextDialog.tsx";
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
import { capByPane, sizeByDraft, type DraftHeight } from "./composerHeight.ts";
import { enterSends } from "./composerEnter.ts";
import {
  focusComposer,
  mayFocusOnOpen,
  registerComposer,
} from "./composerFocus.ts";
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
import { T, useT } from "../i18n/catalog.ts";
import { chat } from "../i18n/catalogs/chat.ts";
import { FieldShell } from "./ui/FieldShell.tsx";
import { Textarea } from "./ui/Textarea.tsx";
import { useMediaQuery } from "./useMediaQuery.ts";
import { useFileNote } from "./useFileNote.ts";

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
  const t = useT(chat);
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
  // The files left out last, and why (GEL-8: it does not stay for good).
  const fileNote = useFileNote();
  const [emojiOpen, setEmojiOpen] = useState(false);
  const [gifOpen, setGifOpen] = useState(false);
  const gifs = useGifStatus();
  const gifsEnabled = gifs.data?.enabled === true;
  const replyTarget = useReplyTarget(channelId);
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
  // The shell hands letters typed on the page to this field (GEL-143).
  useLayoutEffect(() => {
    const input = composerInput.current;
    return input ? registerComposer(input) : undefined;
  }, [canSend]);
  // Opening a conversation puts the keyboard here, to write at once. In
  // the next frame: the quick switcher that opened it gives focus back to
  // its opener as it closes.
  useEffect(() => {
    if (!canSend) return;
    const frame = requestAnimationFrame(() => {
      if (mayFocusOnOpen(window)) focusComposer();
    });
    return () => cancelAnimationFrame(frame);
  }, [channelId, canSend]);
  // Toolbar/read-state changes can move a focused form without resizing it.
  useLayoutEffect(revealComposerFocus, [revealComposerFocus]);
  useLayoutEffect(() => {
    const form = composerForm.current;
    const pane = form?.closest<HTMLElement>(".lr-message-pane");
    if (!form || !pane) return;
    // The field grows with its draft up to half the conversation.
    const resized = () => {
      capByPane(form, pane);
      revealComposerFocus();
    };
    const observer = new ResizeObserver(resized);
    observer.observe(form);
    observer.observe(pane);
    resized();
    return () => observer.disconnect();
  }, [canSend, revealComposerFocus]);
  // chat.css sizes the field by its draft. Browsers without field-sizing
  // (Firefox, Safari before 26.2) get the same height from composerHeight.
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
    fileNote.close();
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
      notes.push(
        t("composer.notAttached", { names: names.join(", "), reason }),
      );
    if (notes.length > 0) fileNote.show(notes.join(" "));
    setFiles(kept);
  };
  const removeFile = (index: number) => {
    fileNote.close();
    setFiles(files.filter((_, i) => i !== index));
  };
  // The picture whose alt text is being written (#575).
  const [describing, setDescribing] = useState<File | null>(null);

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
    fileNote.close();
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
    fileNote.close();
    onDraftChange?.(next);
    requestAnimationFrame(() => {
      input?.focus();
      input?.setSelectionRange(start + text.length, start + text.length);
    });
  };

  const onKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (suggestions.onKeyDown(event)) return;
    // The note first, the reply with the next Escape.
    if (event.key === "Escape" && fileNote.text !== null) {
      event.preventDefault();
      fileNote.close();
      return;
    }
    if (event.key === "Escape" && replyTarget) {
      event.preventDefault();
      setReplyTarget(channelId, null);
      return;
    }
    if (
      event.key === "Enter" &&
      enterSends(
        {
          shiftKey: event.shiftKey,
          isComposing: event.nativeEvent.isComposing,
        },
        event.currentTarget.value,
        event.currentTarget.selectionStart,
      )
    ) {
      event.preventDefault();
      submit();
    }
  };

  if (!canSend) {
    return <p className="lr-composer-readonly">{t("composer.readOnly")}</p>;
  }

  return (
    <>
      <form
        ref={composerForm}
        onFocusCapture={revealComposerFocus}
        onSubmit={submit}
        className="lr-composer"
      >
        {unsent.length > 0 ? (
          <div role="alert" className="lr-send-summary">
            <WarningIcon size={14} />
            <span>{t("composer.unsent", { count: unsent.length })}</span>
            <button type="button" onClick={() => onShowAttempt(unsent[0]!.id)}>
              {t("composer.showUnsent")}
            </button>
          </div>
        ) : null}
        <label className="sr-only" htmlFor={`compose-${channelId}`}>
          {t("composer.messageTo", { target: `${mention}${channelName}` })}
        </label>
        <FieldShell className="lr-composer-field">
          {replyTarget ? (
            <div className="lr-composer-reply">
              <ReplyIcon size={14} />
              <p>
                <T
                  catalog={chat}
                  k="composer.replyTo"
                  params={{ name: replyTarget.author.name }}
                  tags={{ b: (name) => <strong>{name}</strong> }}
                />
                <span className="lr-composer-reply-hint">
                  {" "}
                  {t("composer.replyHint")}
                </span>
              </p>
              <button
                type="button"
                onClick={() => {
                  setReplyTarget(channelId, null);
                  composerInput.current?.focus();
                }}
                aria-label={t("composer.cancelReplyLabel")}
                title={t("composer.cancelReplyLabel")}
              >
                <CloseIcon size={14} />
              </button>
            </div>
          ) : null}
          {dropping ? (
            <p className="lr-composer-drop" aria-hidden="true">
              {t("composer.dropHint")}
            </p>
          ) : null}
          {files.length > 0 ? (
            <ul
              className="lr-composer-attachments"
              aria-label={t("composer.attachmentsLabel")}
            >
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
                    {preview ? (
                      <button
                        type="button"
                        className={
                          altTextOf(attached)
                            ? "lr-composer-alt is-set"
                            : "lr-composer-alt"
                        }
                        onClick={() => setDescribing(attached)}
                        aria-label={`${
                          altTextOf(attached)
                            ? t("altText.edit")
                            : t("altText.add")
                        }: ${attached.name}`}
                        title={altTextOf(attached) ?? t("altText.add")}
                      >
                        {t("altText.badge")}
                      </button>
                    ) : null}
                    <button
                      type="button"
                      onClick={() => removeFile(index)}
                      aria-label={t("composer.removeFileLabel", {
                        name: attached.name,
                      })}
                      title={t("composer.removeFile")}
                    >
                      <CloseIcon size={14} />
                    </button>
                  </li>
                );
              })}
            </ul>
          ) : null}
          {describing ? (
            <AltTextDialog
              key={`${describing.name}:${describing.lastModified}`}
              open
              name={describing.name}
              preview={previews.get(describing)}
              initial={altTextOf(describing) ?? ""}
              onSave={(text) => {
                setAltText(describing, text);
                setDescribing(null);
              }}
              onClose={() => setDescribing(null)}
            />
          ) : null}
          {suggestions.list}
          <div className="lr-composer-row">
            {canSendFiles ? (
              <AttachPicker
                onFiles={(selected) => {
                  addFiles(selected);
                  if (selected.length > 0) composerInput.current?.focus();
                }}
              />
            ) : null}
            <Textarea
              bare
              ref={composerInput}
              id={`compose-${channelId}`}
              value={draft}
              {...suggestions.fieldProps}
              onChange={(e) => {
                setDraft(e.target.value);
                // The note was read; writing on moves past it.
                fileNote.close();
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
              placeholder={t("composer.messageTo", {
                target: `${mention}${channelName}`,
              })}
            />
            <button
              type="button"
              title={t("composer.insertEmoji")}
              aria-label={t("composer.insertEmoji")}
              onClick={() => setEmojiOpen(true)}
              className="lr-composer-icon"
            >
              <SmileIcon size={18} />
            </button>
            {gifsEnabled ? (
              <button
                type="button"
                title={t("composer.sendGif")}
                aria-label={t("composer.sendGif")}
                onClick={() => setGifOpen(true)}
                className="lr-composer-icon lr-composer-gif"
              >
                <span aria-hidden="true">GIF{/* i18n-ok */}</span>
              </button>
            ) : null}
            <button
              type="submit"
              disabled={disabled}
              aria-label={t("composer.send")}
              title={t("composer.send")}
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
        </FieldShell>
        <div className="lr-composer-status" role="status">
          {remaining < 200 ? (
            <span
              className={remaining < 0 ? "lr-field-error" : undefined}
              // Read out only near the limit, not with every letter.
              aria-live={remaining < 20 ? "polite" : "off"}
            >
              {remaining < 0
                ? t("composer.charsOver", { count: -remaining })
                : t("composer.charsLeft", { count: remaining })}
            </span>
          ) : null}
          {error ? (
            <span className="lr-field-error">
              {fieldMessage("content", error)}
            </span>
          ) : null}
        </div>
        {fileNote.text !== null ? (
          <div className="lr-composer-file-error" {...fileNote.holdProps}>
            <p role="alert" className="lr-field-error">
              {fileNote.text}
            </p>
            <button
              type="button"
              onClick={() => {
                fileNote.close();
                composerInput.current?.focus();
              }}
              aria-label={t("composer.dismissFileError")}
              title={t("composer.dismissFileError")}
            >
              <CloseIcon size={14} />
            </button>
          </div>
        ) : null}
      </form>
      {/* Outside the form: Enter in a picker's search would submit it and
          send the half-written draft. */}
      <Modal
        open={emojiOpen}
        onClose={() => setEmojiOpen(false)}
        title={t("composer.insertEmoji")}
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
          title={t("composer.sendGif")}
          wide
        >
          {gifOpen ? <GifPicker onSelect={sendGif} /> : null}
        </Modal>
      ) : null}
    </>
  );
}

function normalisedLength(raw: string): string {
  return raw.replace(/\r\n/g, "\n").replace(/\r/g, "\n");
}

/** The paperclip (#574). On touch screens a small menu: "Fotos & Videos"
 *  opens the system photo picker (`image/*,video/*`, which Android gives
 *  its photo picker and not the file chooser), "Kamera" takes one picture,
 *  "Datei" is the full list of types. Elsewhere the one file dialog. */
function AttachPicker({ onFiles }: { onFiles: (files: File[]) => void }) {
  const t = useT(chat);
  const touch = useMediaQuery("(pointer: coarse)");
  const [open, setOpen] = useState(false);
  const files = useRef<HTMLInputElement>(null);
  const photos = useRef<HTMLInputElement>(null);
  const camera = useRef<HTMLInputElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const toggle = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (!open) return;
    menu.current?.querySelector<HTMLElement>("[role=menuitem]")?.focus();
    const away = (event: PointerEvent) => {
      const target = event.target as Node;
      if (menu.current?.contains(target) || toggle.current?.contains(target))
        return;
      setOpen(false);
    };
    document.addEventListener("pointerdown", away);
    return () => document.removeEventListener("pointerdown", away);
  }, [open]);
  const picked = (event: ChangeEvent<HTMLInputElement>) => {
    const selected = Array.from(event.target.files ?? []);
    event.target.value = "";
    onFiles(selected);
  };
  const choose = (input: RefObject<HTMLInputElement | null>) => {
    setOpen(false);
    input.current?.click();
  };
  return (
    <span className="lr-composer-attach">
      <input
        ref={files}
        type="file"
        accept={ACCEPT}
        multiple
        className="sr-only"
        tabIndex={-1}
        onChange={picked}
      />
      <input
        ref={photos}
        type="file"
        accept="image/*,video/*"
        multiple
        className="sr-only"
        tabIndex={-1}
        aria-hidden="true"
        onChange={picked}
      />
      <input
        ref={camera}
        type="file"
        accept="image/*"
        capture="environment"
        className="sr-only"
        tabIndex={-1}
        aria-hidden="true"
        onChange={picked}
      />
      <button
        ref={toggle}
        type="button"
        title={t(touch ? "composer.attachMenu" : "composer.attachFile")}
        aria-label={t(touch ? "composer.attachMenu" : "composer.attachFile")}
        aria-haspopup={touch ? "menu" : undefined}
        aria-expanded={touch ? open : undefined}
        onClick={() => (touch ? setOpen((was) => !was) : choose(files))}
        className="lr-composer-icon"
      >
        <PaperclipIcon size={18} />
      </button>
      {open ? (
        <div
          ref={menu}
          role="menu"
          aria-label={t("composer.attachMenu")}
          className="lr-composer-attach-menu"
          onKeyDown={(event) => {
            if (event.key === "Escape") {
              event.preventDefault();
              event.stopPropagation();
              setOpen(false);
              toggle.current?.focus();
            }
          }}
        >
          <button type="button" role="menuitem" onClick={() => choose(photos)}>
            <span aria-hidden="true">🖼</span>
            {t("composer.attachPhotos")}
          </button>
          <button type="button" role="menuitem" onClick={() => choose(camera)}>
            <span aria-hidden="true">📷</span>
            {t("composer.attachCamera")}
          </button>
          <button type="button" role="menuitem" onClick={() => choose(files)}>
            <PaperclipIcon size={16} />
            {t("composer.attachDocument")}
          </button>
        </div>
      ) : null}
    </span>
  );
}
