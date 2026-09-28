// TanStack Query layer for a channel's messages.
//
// Feel rule: a send lands in the pending overlay *before* the request
// leaves, so the row is on screen in the same frame (< 50 ms). An error
// drops that row (no ghost). Edit/delete patch the cache first and roll
// back on failure. History is an infinite query: first page = latest,
// `fetchNextPage` walks older via the id cursor.

import {
  useInfiniteQuery,
  useMutation,
  useQueryClient,
  type InfiniteData,
  type QueryClient,
  infiniteQueryOptions,
  replaceEqualDeep,
} from "@tanstack/react-query";

import { scopeGeneration, stampHolds, takeStamp } from "../auth/scope.ts";
import { useSession } from "../auth/session.ts";
import { notifyError } from "../components/toasts.ts";
import { isPendingId } from "../servers/queries.ts";
import * as remote from "./api.ts";
import {
  applyMessageChanges,
  combineMessageChange,
  olderCursor,
  stampOlder,
  type MessageChange,
} from "./pages.ts";
import {
  addPending,
  confirmPending,
  removePending,
  usePendingMessages,
} from "./pending.ts";
import {
  PAGE_SIZE,
  inferContentType,
  isImageType,
  normaliseContent,
} from "./rules.ts";
import type {
  Attachment,
  Message,
  MessageAuthor,
  MessagePage,
} from "./types.ts";
import { asAttachmentList } from "./types.ts";

export const messageKeys = {
  channel: (userId: string, generation: number, channelId: string) =>
    ["user", userId, generation, "messages", channelId] as const,
};

type Cache = InfiniteData<MessagePage, string | undefined>;

const STALE_MS = 30_000;

function tmpId(): string {
  return `tmp:${crypto.randomUUID()}`;
}

function now(): string {
  return new Date().toISOString();
}

type ReadChanges = { changes: Map<string, MessageChange>; manual: boolean };
const journals = new WeakMap<QueryClient, Map<string, ReadChanges>>();

/** Changes live only for the lifetime of a REST read, including all its pages. */
function readJournals(client: QueryClient): Map<string, ReadChanges> {
  const existing = journals.get(client);
  if (existing) return existing;
  const reads = new Map<string, ReadChanges>();
  journals.set(client, reads);
  client.getQueryCache().subscribe((event) => {
    if (event.query.queryKey[3] !== "messages") return;
    const hash = event.query.queryHash;
    if (event.type === "removed") reads.delete(hash);
    if (event.type !== "updated") return;
    if (event.action.type === "fetch")
      reads.set(hash, { changes: new Map(), manual: false });
    if (
      event.action.type === "error" ||
      (event.action.type === "success" && !event.action.manual)
    )
      reads.delete(hash);
  });
  return reads;
}

function recordChange(
  client: QueryClient,
  userId: string,
  generation: number,
  channelId: string,
  change: MessageChange,
): MessageChange {
  const query = client.getQueryCache().find({
    queryKey: messageKeys.channel(userId, generation, channelId),
    exact: true,
  });
  if (!query) return change;
  const read = readJournals(client).get(query.queryHash);
  if (!read) return change;
  const previous = read.changes.get(change.id);
  const merged = combineMessageChange(previous, change);
  read.changes.set(change.id, merged);
  return merged;
}

function patchPages(
  client: QueryClient,
  userId: string,
  generation: number,
  channelId: string,
  update: (pages: MessagePage[]) => MessagePage[],
): void {
  const key = messageKeys.channel(userId, generation, channelId);
  const query = client.getQueryCache().find({ queryKey: key, exact: true });
  const previous = query?.state;
  const read = query && readJournals(client).get(query.queryHash);
  if (read) read.manual = true;
  try {
    client.setQueryData<Cache>(
      key,
      (current) => {
        // An event is a delta, never proof that an unopened history is complete.
        if (!current) return undefined;
        return { ...current, pages: update(current.pages) };
      },
      { updatedAt: previous?.dataUpdatedAt },
    );
  } finally {
    if (read) read.manual = false;
  }
  if (previous?.isInvalidated) {
    void client.invalidateQueries({
      queryKey: key,
      exact: true,
      refetchType: "none",
    });
  }
}

function mapMessages(
  pages: MessagePage[],
  update: (message: Message) => Message | null,
): MessagePage[] {
  return pages.map((page) => ({
    ...page,
    messages: page.messages
      .map((message) => update(message))
      .filter((message): message is Message => message !== null),
  }));
}

export function useMessages(channelId: string | undefined, enabled: boolean) {
  const client = useQueryClient();
  const userId = useSession((state) => state.user)?.id ?? "";
  const generation = scopeGeneration();
  return useInfiniteQuery({
    ...messageQueryOptions(client, userId, generation, channelId ?? ""),
    enabled: userId.length > 0 && Boolean(channelId) && enabled,
  });
}

/** Shared by the hook and recovery tests using the real infinite-query engine. */
export function messageQueryOptions(
  client: QueryClient,
  userId: string,
  generation: number,
  channelId: string,
) {
  const reads = readJournals(client);
  const key = messageKeys.channel(userId, generation, channelId);
  return infiniteQueryOptions({
    queryKey: key,
    queryFn: async ({ pageParam, signal }) =>
      stampOlder(
        await remote.listMessages(
          channelId,
          { before: pageParam, limit: PAGE_SIZE },
          signal,
        ),
      ),
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (page) => olderCursor(page),
    structuralSharing: (previous, incoming) => {
      const query = client.getQueryCache().find({ queryKey: key, exact: true });
      const read = query && reads.get(query.queryHash);
      const data = incoming as Cache;
      const merged =
        read && !read.manual && read.changes.size > 0
          ? {
              ...data,
              pages: applyMessageChanges(data.pages, read.changes.values()),
            }
          : data;
      return replaceEqualDeep(previous, merged);
    },
    staleTime: STALE_MS,
    retry: (count, error) =>
      count < 2 &&
      !(
        error instanceof Error &&
        "code" in error &&
        (error.code === "not_found" || error.code === "forbidden")
      ),
  });
}

export type SendInput = {
  content: string;
  file?: File;
};

function localAttachment(file: File): Attachment {
  const contentType = inferContentType(file);
  return {
    id: `tmp:${crypto.randomUUID()}`,
    filename: file.name.replace(/^.*[/\\]/, ""),
    content_type: contentType,
    size: file.size,
    preview_url: isImageType(contentType)
      ? URL.createObjectURL(file)
      : undefined,
  };
}

function revokePreviews(message: Message | undefined): void {
  if (!message) return;
  for (const attachment of asAttachmentList(message.attachments)) {
    if (attachment.preview_url?.startsWith("blob:")) {
      URL.revokeObjectURL(attachment.preview_url);
    }
  }
}

export function useSendMessage(channelId: string, author: MessageAuthor) {
  return useMutation({
    mutationFn: async ({ content, file }: SendInput) => {
      const ids: string[] = [];
      if (file) {
        const presign = await remote.presignAttachment(channelId, {
          filename: file.name,
          content_type: inferContentType(file),
          size: file.size,
        });
        await remote.putPresigned(presign.upload_url, file, presign.headers);
        ids.push(presign.id);
      }
      return remote.createMessage(channelId, normaliseContent(content), ids);
    },
    onMutate: ({ content, file }) => {
      const stamp = takeStamp();
      if (!stamp) return;
      const tmp = tmpId();
      const pending: Message = {
        id: tmp,
        channel_id: channelId,
        author,
        content: normaliseContent(content),
        created_at: now(),
        edited_at: null,
        attachments: file ? [localAttachment(file)] : [],
      };
      addPending(channelId, pending);
      return { ...stamp, tmp };
    },
    onSuccess: (message, _input, ctx) => {
      // Stay in the overlay until a fetched page already contains this id.
      // Blind append + drop races the in-flight first-page GET: duplicate
      // if GET includes the row, or a successful send vanishes if GET
      // lands without it and replaces an emptyCache write.
      // A send started by the previous account must not land in this one.
      if (!stampHolds(ctx)) return;
      confirmPending(channelId, ctx.tmp, message);
    },
    onError: (error, _input, ctx) => {
      if (!stampHolds(ctx)) return;
      const pending = usePendingMessages.getState().byChannel[channelId] ?? [];
      revokePreviews(pending.find((row) => row.id === ctx.tmp));
      removePending(channelId, ctx.tmp);
      notifyError(error);
    },
  });
}

export function useEditMessage(channelId: string) {
  const client = useQueryClient();
  return useMutation({
    mutationFn: ({ id, content }: { id: string; content: string }) =>
      remote.updateMessage(id, normaliseContent(content)),
    onMutate: ({ id, content }) => {
      const stamp = takeStamp();
      if (!stamp) return;
      const previous = client.getQueryData<Cache>(
        messageKeys.channel(stamp.userId, stamp.generation, channelId),
      );
      const next = normaliseContent(content);
      patchPages(client, stamp.userId, stamp.generation, channelId, (pages) =>
        mapMessages(pages, (message) =>
          message.id === id
            ? { ...message, content: next, edited_at: now() }
            : message,
        ),
      );
      return { ...stamp, previous };
    },
    onError: (error, _vars, ctx) => {
      if (!stampHolds(ctx)) return;
      if (ctx.previous) {
        client.setQueryData(
          messageKeys.channel(ctx.userId, ctx.generation, channelId),
          ctx.previous,
        );
      }
      notifyError(error);
    },
    onSuccess: (message, _vars, ctx) => {
      if (!stampHolds(ctx)) return;
      patchPages(client, ctx.userId, ctx.generation, channelId, (pages) =>
        mapMessages(pages, (row) => (row.id === message.id ? message : row)),
      );
    },
  });
}

function isMessage(value: unknown): value is Message {
  if (value === null || typeof value !== "object") return false;
  const row = value as Message;
  return (
    typeof row.id === "string" &&
    typeof row.channel_id === "string" &&
    typeof row.content === "string" &&
    typeof row.created_at === "string" &&
    row.author !== null &&
    typeof row.author === "object" &&
    typeof row.author.id === "string"
  );
}

/** WS create: append if the row is not already in a page. */
export function applyMessageCreated(
  client: QueryClient,
  userId: string,
  generation: number,
  channelId: string,
  message: Message,
): void {
  const change = recordChange(client, userId, generation, channelId, {
    id: message.id,
    message,
    created: true,
  });
  patchPages(client, userId, generation, channelId, (pages) =>
    applyMessageChanges(pages, [change]),
  );
}

/** WS edit: replace the row in place. */
export function applyMessageEdited(
  client: QueryClient,
  userId: string,
  generation: number,
  channelId: string,
  message: Message,
): void {
  const change = recordChange(client, userId, generation, channelId, {
    id: message.id,
    message,
    created: false,
  });
  patchPages(client, userId, generation, channelId, (pages) =>
    applyMessageChanges(pages, [change]),
  );
}

/** WS / optimistic delete: drop the row, keep page cursors. */
export function applyMessageDeleted(
  client: QueryClient,
  userId: string,
  generation: number,
  channelId: string,
  messageId: string,
): void {
  const change = recordChange(client, userId, generation, channelId, {
    id: messageId,
    message: null,
    created: false,
  });
  patchPages(client, userId, generation, channelId, (pages) =>
    applyMessageChanges(pages, [change]),
  );
}

export function applyChannelEvent(
  client: QueryClient,
  userId: string,
  generation: number,
  event: {
    t: "c" | "e" | "d";
    c?: string;
    i?: string;
    d?: unknown;
  },
): void {
  const channelId = event.c;
  if (!channelId || userId.length === 0) return;
  if (event.t === "d" && event.i) {
    applyMessageDeleted(client, userId, generation, channelId, event.i);
    return;
  }
  if ((event.t === "c" || event.t === "e") && isMessage(event.d)) {
    const message = {
      ...event.d,
      attachments: asAttachmentList(event.d.attachments),
    };
    if (event.t === "c")
      applyMessageCreated(client, userId, generation, channelId, message);
    else applyMessageEdited(client, userId, generation, channelId, message);
  }
}

export function useDeleteMessage(channelId: string) {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => remote.deleteMessage(id),
    onMutate: (id) => {
      const stamp = takeStamp();
      if (!stamp) return;
      const previous = client.getQueryData<Cache>(
        messageKeys.channel(stamp.userId, stamp.generation, channelId),
      );
      patchPages(client, stamp.userId, stamp.generation, channelId, (pages) =>
        mapMessages(pages, (message) => (message.id === id ? null : message)),
      );
      return { ...stamp, previous };
    },
    onError: (error, _id, ctx) => {
      if (!stampHolds(ctx)) return;
      if (ctx.previous) {
        client.setQueryData(
          messageKeys.channel(ctx.userId, ctx.generation, channelId),
          ctx.previous,
        );
      }
      notifyError(error);
    },
  });
}

export { isPendingId };
