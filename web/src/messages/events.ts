// Apply compact chat events (`op: "e"`) to the message cache so a create
// that carries an attachment shows up without waiting for the next GET.

import type { QueryClient } from "@tanstack/react-query";

import { messageKeys } from "./queries.ts";
import { asAttachmentList, type Message, type MessagePage } from "./types.ts";
import type { ChatEvent } from "../ws/protocol.ts";

type Cache = {
  pages: MessagePage[];
  pageParams: (string | undefined)[];
};

function asMessage(delta: unknown): Message | null {
  if (delta === null || typeof delta !== "object") return null;
  const value = delta as Partial<Message>;
  if (
    typeof value.id !== "string" ||
    typeof value.channel_id !== "string" ||
    typeof value.content !== "string" ||
    !value.author ||
    typeof value.author.id !== "string"
  ) {
    return null;
  }
  return {
    id: value.id,
    channel_id: value.channel_id,
    author: value.author,
    content: value.content,
    created_at: value.created_at ?? new Date().toISOString(),
    edited_at: value.edited_at ?? null,
    attachments: asAttachmentList(value.attachments),
  };
}

function emptyCache(): Cache {
  return {
    pages: [{ messages: [], has_more: false }],
    pageParams: [undefined],
  };
}

export function applyChatEvent(
  client: QueryClient,
  userId: string,
  generation: number,
  event: ChatEvent,
): void {
  const channelId = event.c;
  if (!channelId || userId.length === 0) return;
  const key = messageKeys.channel(userId, generation, channelId);

  if (event.t === "c") {
    const message = asMessage(event.d);
    if (!message) return;
    client.setQueryData<Cache>(key, (current) => {
      const base = current ?? emptyCache();
      if (
        base.pages.some((page) =>
          page.messages.some((row) => row.id === message.id),
        )
      ) {
        return base;
      }
      const pages = base.pages.map((page, index) =>
        index === 0 ? { ...page, messages: [...page.messages, message] } : page,
      );
      return { ...base, pages };
    });
    return;
  }

  if (event.t === "e") {
    const message = asMessage(event.d);
    if (!message) return;
    client.setQueryData<Cache>(key, (current) => {
      if (!current) return current;
      return {
        ...current,
        pages: current.pages.map((page) => ({
          ...page,
          messages: page.messages.map((row) =>
            row.id === message.id ? message : row,
          ),
        })),
      };
    });
    return;
  }

  if (event.t === "d" && event.i) {
    const id = event.i;
    client.setQueryData<Cache>(key, (current) => {
      if (!current) return current;
      return {
        ...current,
        pages: current.pages.map((page) => ({
          ...page,
          messages: page.messages.filter((row) => row.id !== id),
        })),
      };
    });
  }
}
