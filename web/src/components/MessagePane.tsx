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
import { CONTENT_MAX, validateContent } from "../messages/rules.ts";
import type { Message } from "../messages/types.ts";
import { can } from "../servers/permissions.ts";
import type { Channel, ServerDetail } from "../servers/types.ts";
import { Avatar } from "./Avatar.tsx";
import { PencilIcon, TrashIcon } from "./Icons.tsx";

export function MessagePane({
  server,
  channel,
  footer,
  onDraftChange,
  onDraftStop,
}: {
  server: ServerDetail;
  channel: Channel;
  footer?: ReactNode;
  onDraftChange?: (value: string) => void;
  onDraftStop?: () => void;
}) {
  const user = useSession((s) => s.user);
  const canSend = can(server, "send_messages");
  const query = useMessages(channel.id, true);
  const pending = usePendingMessages(
    (s) => s.byChannel[channel.id] ?? nonePending,
  );
  const items = useMemo(
    () => visibleMessages(query.data?.pages ?? [], pending),
    [query.data?.pages, pending],
  );

  useEffect(() => {
    const pages = query.data?.pages ?? [];
    for (const id of confirmedPendingIds(pages, pending)) {
      removePending(channel.id, id);
    }
  }, [channel.id, pending, query.data?.pages]);
  const fetchNextPage = query.fetchNextPage;
  const hasNextPage = query.hasNextPage;
  const isFetchingNextPage = query.isFetchingNextPage;
  const onLoadOlder = useCallback(() => {
    if (hasNextPage && !isFetchingNextPage) void fetchNextPage();
  }, [hasNextPage, isFetchingNextPage, fetchNextPage]);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <MessageList
        channelId={channel.id}
        items={items}
        meId={user?.id}
        hasOlder={Boolean(hasNextPage)}
        loadingOlder={isFetchingNextPage}
        onLoadOlder={onLoadOlder}
        ready={!query.isPending}
      />
      {footer}
      <Composer
        channelId={channel.id}
        channelName={channel.name}
        canSend={canSend}
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
  hasOlder,
  loadingOlder,
  onLoadOlder,
  ready,
}: {
  channelId: string;
  items: Message[];
  meId: string | undefined;
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
  const [viewport, setViewport] = useState(0);
  const edit = useEditMessage(channelId);
  const remove = useDeleteMessage(channelId);

  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const update = () => setViewport(el.clientHeight);
    update();
    const ro = new ResizeObserver(update);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // Not on the React Compiler; the warning is about memoising its return value.
  // eslint-disable-next-line react-hooks/incompatible-library
  const virtualizer = useVirtualizer({
    count: items.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: (index) =>
      isContinued(items[index - 1], items[index]!) ? 28 : 72,
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
    }

    lastCount.current = items.length;
    lastTail.current = tail;
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
      className="min-h-0 flex-1 overflow-y-auto"
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
        style={{
          height: virtualizer.getTotalSize(),
          marginTop:
            viewport > virtualizer.getTotalSize()
              ? viewport - virtualizer.getTotalSize()
              : 0,
        }}
        className="relative w-full"
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
                onEdit={(content) => edit.mutate({ id: message.id, content })}
                onDelete={() => {
                  if (window.confirm("Diese Nachricht wirklich löschen?"))
                    remove.mutate(message.id);
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
  onEdit,
  onDelete,
}: {
  message: Message;
  continued: boolean;
  mine: boolean;
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
  const error = editing ? validateContent(draft) : null;

  const actions =
    mine && !pending && !editing ? (
      <span className="flex shrink-0 items-center opacity-70 transition group-hover:opacity-100 group-focus-within:opacity-100">
        <IconButton label="Nachricht bearbeiten" onClick={startEdit}>
          <PencilIcon size={14} />
        </IconButton>
        <IconButton label="Nachricht löschen" onClick={onDelete}>
          <TrashIcon size={14} />
        </IconButton>
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
          <p className="whitespace-pre-wrap break-words text-sm text-neutral-800">
            {message.content}
            {message.edited_at ? (
              <span className="ml-1 text-xs text-neutral-400">
                (bearbeitet)
              </span>
            ) : null}
          </p>
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
      </div>
      {actions}
    </div>
  );
}

function Composer({
  channelId,
  channelName,
  canSend,
  author,
  onDraftChange,
  onDraftStop,
}: {
  channelId: string;
  channelName: string;
  canSend: boolean;
  author: { id: string; name: string; avatar_url: string | null } | null;
  onDraftChange?: (value: string) => void;
  onDraftStop?: () => void;
}) {
  const send = useSendMessage(
    channelId,
    author ?? { id: "", name: "", avatar_url: null },
  );
  const [draft, setDraft] = useState("");
  const error = draft.length === 0 ? null : validateContent(draft);
  const remaining = CONTENT_MAX - Array.from(normalisedLength(draft)).length;
  const disabled =
    !canSend || !author || Boolean(error) || draft.trim().length === 0;

  const submit = (event?: FormEvent) => {
    event?.preventDefault();
    if (disabled) return;
    const text = draft;
    send.mutate(text, {
      onError: () => {
        setDraft((current) => {
          const next = current.trim().length === 0 ? text : current;
          if (next) onDraftChange?.(next);
          return next;
        });
      },
    });
    setDraft("");
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
        Du hast in diesem Server kein Schreibrecht.
      </p>
    );
  }

  return (
    <form
      onSubmit={submit}
      className="border-t border-neutral-200 bg-white px-4 py-3"
    >
      <label className="sr-only" htmlFor={`compose-${channelId}`}>
        Nachricht in #{channelName}
      </label>
      <div className="flex items-end gap-2">
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
          placeholder={`Nachricht an #${channelName}`}
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
      </div>
    </form>
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
