import { QueryClient } from "@tanstack/react-query";
import { describe, expect, it } from "vitest";

import {
  applyChannelEvent,
  applyMessageDeleted,
  messageKeys,
} from "./queries.ts";
import type { Message, MessagePage } from "./types.ts";

function msg(id: string, channel = "c1"): Message {
  return {
    id,
    channel_id: channel,
    author: { id: "u1", name: "Ada", avatar_url: null },
    content: id,
    created_at: "2026-09-13T18:00:00.000Z",
    edited_at: null,
  };
}

function cache(
  client: QueryClient,
  channelId: string,
  messages: Message[],
): void {
  client.setQueryData(messageKeys.channel(channelId), {
    pages: [{ messages, has_more: true, older: "stamp" }] satisfies MessagePage[],
    pageParams: [undefined],
  });
}

describe("apply channel events", () => {
  it("removes a deleted row and keeps the page cursor", () => {
    const client = new QueryClient();
    cache(client, "c1", [msg("a"), msg("b"), msg("c")]);
    applyMessageDeleted(client, "c1", "b");
    const data = client.getQueryData<{ pages: MessagePage[] }>(
      messageKeys.channel("c1"),
    );
    expect(data?.pages[0]?.messages.map((m) => m.id)).toEqual(["a", "c"]);
    expect(data?.pages[0]?.older).toBe("stamp");
    expect(data?.pages[0]?.has_more).toBe(true);
  });

  it("applies create/edit/delete from a compact WS event", () => {
    const client = new QueryClient();
    cache(client, "c1", [msg("a")]);
    applyChannelEvent(client, {
      t: "c",
      c: "c1",
      i: "b",
      d: msg("b"),
    });
    applyChannelEvent(client, {
      t: "e",
      c: "c1",
      i: "a",
      d: { ...msg("a"), content: "edited" },
    });
    applyChannelEvent(client, { t: "d", c: "c1", i: "b" });
    const data = client.getQueryData<{ pages: MessagePage[] }>(
      messageKeys.channel("c1"),
    );
    expect(data?.pages[0]?.messages).toEqual([
      { ...msg("a"), content: "edited" },
    ]);
  });

  it("does not duplicate a create that is already in the page", () => {
    const client = new QueryClient();
    cache(client, "c1", [msg("a")]);
    applyChannelEvent(client, { t: "c", c: "c1", i: "a", d: msg("a") });
    const data = client.getQueryData<{ pages: MessagePage[] }>(
      messageKeys.channel("c1"),
    );
    expect(data?.pages[0]?.messages).toHaveLength(1);
  });
});
