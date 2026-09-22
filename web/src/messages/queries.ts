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
} from "@tanstack/react-query";

import {
  scopeGeneration,
  stampHolds,
  takeStamp,
  useUserId,
} from "../auth/scope.ts";
import { notifyError } from "../components/toasts.ts";
import { isPendingId } from "../servers/queries.ts";
import * as remote from "./api.ts";
import { olderCursor, stampOlder } from "./pages.ts";
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

function emptyCache(): Cache {
  return {
    pages: [{ messages: [], has_more: false }],
    pageParams: [undefined],
  };
}

function patchPages(
  client: QueryClient,
  userId: string,
  generation: number,
  channelId: string,
  update: (pages: MessagePage[]) => MessagePage[],
): void {
  client.setQueryData<Cache>(
    messageKeys.channel(userId, generation, channelId),
    (current) => {
      const base = current ?? emptyCache();
      return { ...base, pages: update(base.pages) };
    },
  );
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
  const userId = useUserId();
  const generation = scopeGeneration();
  return useInfiniteQuery({
    queryKey: messageKeys.channel(userId, generation, channelId ?? ""),
    queryFn: async ({ pageParam, signal }) =>
      stampOlder(
        await remote.listMessages(
          channelId ?? "",
          { before: pageParam, limit: PAGE_SIZE },
          signal,
        ),
      ),
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (page) => olderCursor(page),
    enabled: userId.length > 0 && Boolean(channelId) && enabled,
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
  patchPages(client, userId, generation, channelId, (pages) => {
    if (
      pages.some((page) => page.messages.some((row) => row.id === message.id))
    ) {
      return pages;
    }
    if (pages.length === 0) {
      return [{ messages: [message], has_more: false }];
    }
    const newest = pages[0];
    if (!newest) return pages;
    return [
      { ...newest, messages: [...newest.messages, message] },
      ...pages.slice(1),
    ];
  });
}

/** WS edit: replace the row in place. */
export function applyMessageEdited(
  client: QueryClient,
  userId: string,
  generation: number,
  channelId: string,
  message: Message,
): void {
  patchPages(client, userId, generation, channelId, (pages) =>
    mapMessages(pages, (row) => (row.id === message.id ? message : row)),
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
  patchPages(client, userId, generation, channelId, (pages) =>
    mapMessages(pages, (message) =>
      message.id === messageId ? null : message,
    ),
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
