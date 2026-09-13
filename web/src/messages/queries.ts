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

import { notifyError } from "../components/toasts.ts";
import { isPendingId } from "../servers/queries.ts";
import * as remote from "./api.ts";
import { olderCursor } from "./pages.ts";
import { addPending, removePending } from "./pending.ts";
import { PAGE_SIZE, normaliseContent } from "./rules.ts";
import type { Message, MessageAuthor, MessagePage } from "./types.ts";

export const messageKeys = {
  channel: (channelId: string) => ["messages", channelId] as const,
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
  channelId: string,
  update: (pages: MessagePage[]) => MessagePage[],
): void {
  client.setQueryData<Cache>(messageKeys.channel(channelId), (current) => {
    const base = current ?? emptyCache();
    return { ...base, pages: update(base.pages) };
  });
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
  return useInfiniteQuery({
    queryKey: messageKeys.channel(channelId ?? ""),
    queryFn: ({ pageParam, signal }) =>
      remote.listMessages(
        channelId ?? "",
        { before: pageParam, limit: PAGE_SIZE },
        signal,
      ),
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (page) => olderCursor(page),
    enabled: Boolean(channelId) && enabled,
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

export function useSendMessage(channelId: string, author: MessageAuthor) {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (content: string) =>
      remote.createMessage(channelId, normaliseContent(content)),
    onMutate: (content) => {
      const tmp = tmpId();
      const pending: Message = {
        id: tmp,
        channel_id: channelId,
        author,
        content: normaliseContent(content),
        created_at: now(),
        edited_at: null,
      };
      addPending(channelId, pending);
      return tmp;
    },
    onSuccess: (message, _content, tmp) => {
      patchPages(client, channelId, (pages) => {
        if (pages.length === 0) {
          return [{ messages: [message], has_more: false }];
        }
        const next = pages.slice();
        const newest = next[0];
        if (!newest) return [{ messages: [message], has_more: false }];
        next[0] = { ...newest, messages: [...newest.messages, message] };
        return next;
      });
      if (tmp) removePending(channelId, tmp);
    },
    onError: (error, _content, tmp) => {
      if (tmp) removePending(channelId, tmp);
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
      const previous = client.getQueryData<Cache>(
        messageKeys.channel(channelId),
      );
      const next = normaliseContent(content);
      patchPages(client, channelId, (pages) =>
        mapMessages(pages, (message) =>
          message.id === id
            ? { ...message, content: next, edited_at: now() }
            : message,
        ),
      );
      return previous;
    },
    onError: (error, _vars, previous) => {
      if (previous)
        client.setQueryData(messageKeys.channel(channelId), previous);
      notifyError(error);
    },
    onSuccess: (message) =>
      patchPages(client, channelId, (pages) =>
        mapMessages(pages, (row) => (row.id === message.id ? message : row)),
      ),
  });
}

export function useDeleteMessage(channelId: string) {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => remote.deleteMessage(id),
    onMutate: (id) => {
      const previous = client.getQueryData<Cache>(
        messageKeys.channel(channelId),
      );
      patchPages(client, channelId, (pages) =>
        mapMessages(pages, (message) => (message.id === id ? null : message)),
      );
      return previous;
    },
    onError: (error, _id, previous) => {
      if (previous)
        client.setQueryData(messageKeys.channel(channelId), previous);
      notifyError(error);
    },
  });
}

export { isPendingId };
