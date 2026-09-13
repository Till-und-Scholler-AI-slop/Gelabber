// Text-channel body: virtualised history, optimistic composer, own
// edit/delete. A send writes the pending overlay first, so the row is
// painted in the same frame; the server answer swaps the `tmp:` id, an
// error drops the row.

import { useVirtualizer } from "@tanstack/react-virtual";
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type FormEvent,
  type KeyboardEvent,
  type ReactNode,
} from "react";

import { useSession } from "../auth/session.ts";
import { fieldMessage } from "../auth/rules.ts";
import {
  confirmedPendingIds,
  isContinued,
  visibleMessages,
} from "../messages/pages.ts";
import {
  nonePending,
  removePending,
  usePendingMessages,
} from "../messages/pending.ts";
import {
  isPendingId,
  useDeleteMessage,
  useEditMessage,
  useMessages,
  useSendMessage,
} from "../messages/queries.ts";
import { attachmentUrl } from "../messages/api.ts";
import {
  ALLOWED_TYPES,
  CONTENT_MAX,
  inferContentType,
  isImageType,
  validateAttachment,
  validateContent,
} from "../messages/rules.ts";
import type { Attachment, Message } from "../messages/types.ts";
import { Avatar } from "./Avatar.tsx";
import { PaperclipIcon, PencilIcon, TrashIcon } from "./Icons.tsx";

export function MessagePane({
  channelId,
  channelName,
  canSend,
  canSendFiles = false,
  canModerate = false,
  mention = "#",
  footer,
  onDraftChange,
  onDraftStop,
}: {
  channelId: string;
  channelName: string;
  canSend: boolean;
  canSendFiles?: boolean;
  canModerate?: boolean;
  /** `#` for a server channel, `@` for a DM. */
  mention?: "#" | "@";
  footer?: ReactNode;
  onDraftChange?: (value: string) => void;
  onDraftStop?: () => void;
}) {
  const user = useSession((s) => s.user);
  const query = useMessages(channelId, true);
  const pending = usePendingMessages(
    (s) => s.byChannel[channelId] ?? nonePending,
  );
  const items = useMemo(
    () => visibleMessages(query.data?.pages ?? [], pending),
    [query.data?.pages, pending],
  );

  useEffect(() => {
    const pages = query.data?.pages ?? [];
    for (const id of confirmedPendingIds(pages, pending)) {
      removePending(channelId, id);
    }
  }, [channelId, pending, query.data?.pages]);
  const fetchNextPage = query.fetchNextPage;
  const hasNextPage = query.hasNextPage;
  const isFetchingNextPage = query.isFetchingNextPage;
  const onLoadOlder = useCallback(() => {
    if (hasNextPage && !isFetchingNextPage) void fetchNextPage();
  }, [hasNextPage, isFetchingNextPage, fetchNextPage]);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <MessageList
        channelId={channelId}
        items={items}
        meId={user?.id}
        canModerate={canModerate}
        hasOlder={Boolean(hasNextPage)}
        loadingOlder={isFetchingNextPage}
        onLoadOlder={onLoadOlder}
        ready={!query.isPending}
      />
      {footer}
      <Composer
        channelId={channelId}
        channelName={channelName}
        canSend={canSend}
        canSendFiles={canSendFiles}
        mention={mention}
        author={
          user
            ? { id: user.id, name: user.name, avatar_url: user.avatar_url }
            : null
        }
        onDraftChange={onDraftChange}
        onDraftStop={onDraftStop}
      />
    </div>
  );
}

function MessageList({
  channelId,
  items,
  meId,
  canModerate,
  hasOlder,
  loadingOlder,
  onLoadOlder,
  ready,
}: {
  channelId: string;
  items: Message[];
  meId: string | undefined;
  canModerate: boolean;
  hasOlder: boolean;
  loadingOlder: boolean;
  onLoadOlder: () => void;
  ready: boolean;
}) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const stickToBottom = useRef(true);
  const olderAnchor = useRef<string | null>(null);
  const lastCount = useRef(0);
  const lastTail = useRef<string | undefined>(undefined);
  const pin = useRef<{ id: string; offset: number } | null>(null);
  const edit = useEditMessage(channelId);
  const remove = useDeleteMessage(channelId);

  // Not on the React Compiler; the warning is about memoising its return value.
  // eslint-disable-next-line react-hooks/incompatible-library
  const virtualizer = useVirtualizer({
    count: items.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: (index) => {
      const message = items[index]!;
      const image = (message.attachments ?? []).some((a) =>
        isImageType(a.content_type),
      );
      if (image) return isContinued(items[index - 1], message) ? 168 : 220;
      return isContinued(items[index - 1], message) ? 28 : 72;
    },
    getItemKey: (index) => items[index]?.id ?? index,
    overscan: 12,
  });

  useLayoutEffect(() => {
    const tail = items[items.length - 1]?.id;
    const grewAtEnd =
      tail !== lastTail.current && items.length >= lastCount.current;
    const prepended =
      olderAnchor.current !== null && items.length > lastCount.current;

    if (prepended) {
      const idx = items.findIndex((m) => m.id === olderAnchor.current);
      olderAnchor.current = null;
      if (idx >= 0) virtualizer.scrollToIndex(idx, { align: "start" });
    } else if (
      stickToBottom.current &&
      (grewAtEnd || lastCount.current === 0)
    ) {
      if (items.length > 0)
        virtualizer.scrollToIndex(items.length - 1, { align: "end" });
    } else if (
      items.length < lastCount.current &&
      pin.current &&
      scrollRef.current
    ) {
      // A row vanished (own delete or WS). Keep the first visible id
      // at the same offset so the list does not jump.
      const idx = items.findIndex((m) => m.id === pin.current?.id);
      if (idx >= 0) {
        const offset = virtualizer.getOffsetForIndex(idx, "start");
        if (offset) {
          scrollRef.current.scrollTop = offset[0] - pin.current.offset;
        }
      }
    }

    lastCount.current = items.length;
    lastTail.current = tail;
    const first = virtualizer.getVirtualItems()[0];
    const firstMessage = first ? items[first.index] : undefined;
    if (first && firstMessage && scrollRef.current) {
      pin.current = {
        id: firstMessage.id,
        offset: first.start - scrollRef.current.scrollTop,
      };
    }
  }, [items, virtualizer]);

  const firstVisible = virtualizer.getVirtualItems()[0]?.index ?? 0;
  const firstId = items[0]?.id;
  useEffect(() => {
    if (!ready || !hasOlder || loadingOlder) return;
    // First paint is pinned to the newest row; do not walk older pages
    // until the user actually scrolls up.
    if (stickToBottom.current) return;
    if (firstVisible > 4) return;
    olderAnchor.current = firstId ?? null;
    onLoadOlder();
  }, [firstVisible, firstId, hasOlder, loadingOlder, onLoadOlder, ready]);

  return (
    <div
      ref={scrollRef}
      role="log"
      aria-label="Nachrichten"
      aria-busy={!ready || undefined}
      onScroll={() => {
        const el = scrollRef.current;
        if (!el) return;
        stickToBottom.current =
          el.scrollHeight - el.scrollTop - el.clientHeight < 96;
      }}
      className="flex min-h-0 flex-1 flex-col overflow-y-auto"
    >
      {ready && items.length === 0 ? (
        <div className="flex h-full items-center justify-center px-6 text-center text-sm text-neutral-500">
          Noch keine Nachrichten. Schreib die erste.
        </div>
      ) : null}
      {hasOlder || loadingOlder ? (
        <p className="px-4 py-2 text-center text-xs text-neutral-400">
          {loadingOlder ? "Ältere Nachrichten…" : ""}
        </p>
      ) : null}
      <div
        style={{ height: virtualizer.getTotalSize() }}
        className="relative mt-auto w-full"
      >
        {virtualizer.getVirtualItems().map((row) => {
          const message = items[row.index];
          if (!message) return null;
          const continued = isContinued(items[row.index - 1], message);
          return (
            <div
              key={row.key}
              data-index={row.index}
              ref={virtualizer.measureElement}
              className="absolute inset-x-0"
              style={{ top: row.start }}
            >
              <MessageRow
                message={message}
                continued={continued}
                mine={message.author.id === meId}
                canDelete={message.author.id === meId || canModerate}
                onEdit={(content) => edit.mutate({ id: message.id, content })}
                onDelete={() => {
                  const own = message.author.id === meId;
                  const ok = window.confirm(
                    own
                      ? "Diese Nachricht wirklich löschen?"
                      : `Nachricht von ${message.author.name} löschen?`,
                  );
                  if (ok) remove.mutate(message.id);
                }}
              />
            </div>
          );
        })}
      </div>
    </div>
  );
}

function MessageRow({
  message,
  continued,
  mine,
  canDelete,
  onEdit,
  onDelete,
}: {
  message: Message;
  continued: boolean;
  mine: boolean;
  canDelete: boolean;
  onEdit: (content: string) => void;
  onDelete: () => void;
}) {
  const pending = isPendingId(message.id);
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(message.content);

  const startEdit = () => {
    setDraft(message.content);
    setEditing(true);
  };
  const cancelEdit = () => {
    setEditing(false);
    setDraft(message.content);
  };

  const when = formatWhen(message.created_at);
  const error = editing
    ? draft.length === 0
      ? message.attachments.length > 0
        ? null
        : validateContent(draft)
      : validateContent(draft)
    : null;
  const showDelete = canDelete && !pending && !editing;
  const showEdit = mine && !pending && !editing;

  const actions =
    showEdit || showDelete ? (
      <span className="flex shrink-0 items-center opacity-70 transition group-hover:opacity-100 group-focus-within:opacity-100">
        {showEdit ? (
          <IconButton label="Nachricht bearbeiten" onClick={startEdit}>
            <PencilIcon size={14} />
          </IconButton>
        ) : null}
        {showDelete ? (
          <IconButton label="Nachricht löschen" onClick={onDelete}>
            <TrashIcon size={14} />
          </IconButton>
        ) : null}
      </span>
    ) : null;

  if (continued && !editing) {
    return (
      <div
        className={[
          "group flex items-start gap-3 px-4 py-0.5 hover:bg-neutral-100/80",
          pending ? "opacity-60" : "",
        ].join(" ")}
      >
        <span className="w-8 shrink-0" aria-hidden />
        <div className="min-w-0 flex-1">
          {message.content ? (
            <p className="whitespace-pre-wrap break-words text-sm text-neutral-800">
              {message.content}
              {message.edited_at ? (
                <span className="ml-1 text-xs text-neutral-400">
                  (bearbeitet)
                </span>
              ) : null}
            </p>
          ) : null}
          <AttachmentList attachments={message.attachments ?? []} />
        </div>
        {actions}
      </div>
    );
  }

  return (
    <div
      className={[
        "group flex items-start gap-3 px-4 py-1.5 hover:bg-neutral-100/80",
        pending ? "opacity-60" : "",
      ].join(" ")}
    >
      <Avatar name={message.author.name} url={message.author.avatar_url} />
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-baseline gap-2">
          <span className="text-sm font-semibold text-neutral-900">
            {message.author.name}
          </span>
          <time
            dateTime={message.created_at}
            className="text-xs text-neutral-400"
          >
            {when}
          </time>
        </div>
        {editing ? (
          <form
            className="mt-1 flex flex-col gap-1.5"
            onSubmit={(event) => {
              event.preventDefault();
              if (error) return;
              onEdit(draft);
              setEditing(false);
            }}
          >
            <textarea
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
              onKeyDown={(event) => {
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
                  if (!error) {
                    onEdit(draft);
                    setEditing(false);
                  }
                }
              }}
              rows={2}
              className="w-full resize-none rounded-md border border-neutral-300 bg-white px-2 py-1.5 text-sm outline-none focus:border-neutral-500 focus:ring-2 focus:ring-neutral-200"
              autoFocus
            />
            {error ? (
              <p className="text-xs text-red-600">
                {fieldMessage("content", error)}
              </p>
            ) : null}
            <div className="flex gap-2 text-xs">
              <button
                type="submit"
                disabled={Boolean(error)}
                className="rounded-md bg-neutral-900 px-2 py-1 font-medium text-white disabled:opacity-50"
              >
                Speichern
              </button>
              <button
                type="button"
                onClick={cancelEdit}
                className="rounded-md px-2 py-1 text-neutral-600 hover:bg-neutral-100"
              >
                Abbrechen
              </button>
            </div>
          </form>
        ) : (
          <p className="whitespace-pre-wrap break-words text-sm text-neutral-800">
            {message.content}
            {message.edited_at ? (
              <span className="ml-1 text-xs text-neutral-400">
                (bearbeitet)
              </span>
            ) : null}
          </p>
        )}
        {!editing ? (
          <AttachmentList attachments={message.attachments ?? []} />
        ) : null}
      </div>
      {actions}
    </div>
  );
}

function Composer({
  channelId,
  channelName,
  canSend,
  canSendFiles,
  mention,
  author,
  onDraftChange,
  onDraftStop,
}: {
  channelId: string;
  channelName: string;
  canSend: boolean;
  canSendFiles: boolean;
  mention: "#" | "@";
  author: { id: string; name: string; avatar_url: string | null } | null;
  onDraftChange?: (value: string) => void;
  onDraftStop?: () => void;
}) {
  const send = useSendMessage(
    channelId,
    author ?? { id: "", name: "", avatar_url: null },
  );
  const [draft, setDraft] = useState("");
  const [file, setFile] = useState<File | null>(null);
  const [fileError, setFileError] = useState<string | null>(null);
  const [preview, setPreview] = useState<string | null>(null);
  const fileInput = useRef<HTMLInputElement>(null);
  const fileRef = useRef<File | null>(null);
  const error = draft.length === 0 ? null : validateContent(draft);
  const remaining = CONTENT_MAX - Array.from(normalisedLength(draft)).length;
  const emptyText = draft.trim().length === 0;
  const disabled =
    !canSend || !author || Boolean(error) || (emptyText && !file);

  const pickFile = (next: File | null) => {
    if (preview) URL.revokeObjectURL(preview);
    setPreview(null);
    setFileError(null);
    if (!next) {
      fileRef.current = null;
      setFile(null);
      return;
    }
    const invalid = validateAttachment(next);
    if (invalid) {
      fileRef.current = null;
      setFile(null);
      setFileError(fieldMessage(invalid.field, invalid.code));
      return;
    }
    fileRef.current = next;
    setFile(next);
    if (isImageType(inferContentType(next))) {
      setPreview(URL.createObjectURL(next));
    }
  };

  const submit = (event?: FormEvent) => {
    event?.preventDefault();
    if (disabled) return;
    const text = draft;
    const attached = file;
    send.mutate(
      { content: text, file: attached ?? undefined },
      {
        onError: () => {
          setDraft((current) => {
            const next = current.trim().length === 0 ? text : current;
            if (next) onDraftChange?.(next);
            return next;
          });
          // `file` from this render is `attached` — do not use it. pickFile(null)
          // already cleared the ref; restore unless the user picked another file.
          if (!fileRef.current && attached) pickFile(attached);
        },
      },
    );
    setDraft("");
    pickFile(null);
    onDraftStop?.();
  };

  const onKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
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
      <p className="border-t border-neutral-200 px-4 py-3 text-sm text-neutral-500">
        Du kannst in diesem Kanal nicht schreiben.
      </p>
    );
  }

  return (
    <form
      onSubmit={submit}
      className="border-t border-neutral-200 bg-white px-4 py-3"
    >
      <label className="sr-only" htmlFor={`compose-${channelId}`}>
        Nachricht in {mention}
        {channelName}
      </label>
      {file ? (
        <div className="mb-2 flex items-center gap-2 rounded-lg border border-neutral-200 bg-neutral-50 px-2 py-1.5">
          {preview ? (
            <img
              src={preview}
              alt=""
              className="h-12 w-12 rounded object-cover"
            />
          ) : null}
          <div className="min-w-0 flex-1">
            <p className="truncate text-sm text-neutral-800">{file.name}</p>
            <p className="text-xs text-neutral-400">
              {(file.size / 1024).toFixed(0)} KB
            </p>
          </div>
          <button
            type="button"
            onClick={() => pickFile(null)}
            className="rounded-md px-2 py-1 text-xs text-neutral-600 hover:bg-neutral-200"
          >
            Entfernen
          </button>
        </div>
      ) : null}
      <div className="flex items-end gap-2">
        {canSendFiles ? (
          <>
            <input
              ref={fileInput}
              type="file"
              accept={ALLOWED_TYPES.join(",")}
              className="sr-only"
              onChange={(event) => {
                pickFile(event.target.files?.[0] ?? null);
                event.target.value = "";
              }}
            />
            <button
              type="button"
              title="Datei anhängen"
              aria-label="Datei anhängen"
              onClick={() => fileInput.current?.click()}
              className="rounded-lg p-2 text-neutral-500 transition hover:bg-neutral-100 hover:text-neutral-900"
            >
              <PaperclipIcon size={18} />
            </button>
          </>
        ) : null}
        <textarea
          id={`compose-${channelId}`}
          value={draft}
          onChange={(e) => {
            setDraft(e.target.value);
            onDraftChange?.(e.target.value);
          }}
          onBlur={() => onDraftStop?.()}
          onKeyDown={onKeyDown}
          rows={1}
          placeholder={`Nachricht an ${mention}${channelName}`}
          className="max-h-40 min-h-10 flex-1 resize-none rounded-lg border border-neutral-300 bg-neutral-50 px-3 py-2 text-sm outline-none focus:border-neutral-500 focus:bg-white focus:ring-2 focus:ring-neutral-200"
        />
        <button
          type="submit"
          disabled={disabled}
          className="rounded-lg bg-neutral-900 px-3 py-2 text-sm font-medium text-white transition hover:bg-neutral-700 disabled:cursor-not-allowed disabled:opacity-50"
        >
          Senden
        </button>
      </div>
      <div className="mt-1 flex justify-between text-xs text-neutral-400">
        <span>Enter sendet · Shift+Enter neue Zeile</span>
        {remaining < 200 ? (
          <span className={remaining < 0 ? "text-red-600" : undefined}>
            {remaining}
          </span>
        ) : null}
        {error ? (
          <span className="text-red-600">{fieldMessage("content", error)}</span>
        ) : null}
        {fileError ? <span className="text-red-600">{fileError}</span> : null}
      </div>
    </form>
  );
}

function AttachmentList({ attachments }: { attachments: Attachment[] }) {
  if (attachments.length === 0) return null;
  return (
    <div className="mt-1 flex flex-col gap-1.5">
      {attachments.map((attachment) => {
        const src = attachment.preview_url ?? attachmentUrl(attachment.id);
        if (isImageType(attachment.content_type)) {
          return (
            <a
              key={attachment.id}
              href={src}
              target="_blank"
              rel="noreferrer"
              className="block max-w-xs"
            >
              <img
                src={src}
                alt={attachment.filename}
                className="max-h-56 max-w-full rounded-lg border border-neutral-200 object-contain"
              />
            </a>
          );
        }
        return (
          <a
            key={attachment.id}
            href={attachmentUrl(attachment.id)}
            className="inline-flex max-w-full items-center gap-2 rounded-md border border-neutral-200 bg-neutral-50 px-2 py-1 text-sm text-neutral-800 hover:bg-neutral-100"
          >
            <span className="truncate">{attachment.filename}</span>
            <span className="shrink-0 text-xs text-neutral-400">
              {(attachment.size / 1024).toFixed(0)} KB
            </span>
          </a>
        );
      })}
    </div>
  );
}

function normalisedLength(raw: string): string {
  return raw.replace(/\r\n/g, "\n").replace(/\r/g, "\n");
}

function formatWhen(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "";
  const now = new Date();
  const sameDay =
    date.getFullYear() === now.getFullYear() &&
    date.getMonth() === now.getMonth() &&
    date.getDate() === now.getDate();
  return new Intl.DateTimeFormat("de-DE", {
    hour: "2-digit",
    minute: "2-digit",
    ...(sameDay ? {} : { day: "2-digit", month: "2-digit" }),
  }).format(date);
}

function IconButton({
  label,
  onClick,
  children,
}: {
  label: string;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      title={label}
      aria-label={label}
      onClick={onClick}
      className="rounded-md p-1 text-neutral-500 transition hover:bg-neutral-200 hover:text-neutral-900"
    >
      {children}
    </button>
  );
}
