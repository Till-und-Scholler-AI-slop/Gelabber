import { QueryClient } from "@tanstack/react-query";
import { describe, expect, it } from "vitest";

import { applyChatEvent } from "./events.ts";
import { messageKeys } from "./queries.ts";
import type { Message, MessagePage } from "./types.ts";

function msg(id: string): Message {
  return {
    id,
    channel_id: "c1",
    author: { id: "u1", name: "Ada", avatar_url: null },
    content: id,
    created_at: "2026-09-13T18:00:00.000Z",
    edited_at: null,
    attachments: [],
  };
}

describe("applyChatEvent", () => {
  it("appends a create that carries an attachment", () => {
    const client = new QueryClient();
    client.setQueryData(messageKeys.channel("c1"), {
      pages: [{ messages: [msg("a")], has_more: false } satisfies MessagePage],
      pageParams: [undefined],
    });
    applyChatEvent(client, {
      op: "e",
      t: "c",
      s: "s1",
      c: "c1",
      n: 1,
      i: "b",
      d: {
        ...msg("b"),
        attachments: [
          {
            id: "att-1",
            filename: "cat.png",
            content_type: "image/png",
            size: 12,
          },
        ],
      },
    });
    const cache = client.getQueryData<{ pages: MessagePage[] }>(
      messageKeys.channel("c1"),
    );
    const last = cache?.pages[0]?.messages.at(-1);
    expect(last?.id).toBe("b");
    expect(last?.attachments[0]?.filename).toBe("cat.png");
  });

  it("does not duplicate an id already in the page", () => {
    const client = new QueryClient();
    client.setQueryData(messageKeys.channel("c1"), {
      pages: [{ messages: [msg("a")], has_more: false }],
      pageParams: [undefined],
    });
    applyChatEvent(client, {
      op: "e",
      t: "c",
      s: "s1",
      c: "c1",
      n: 2,
      i: "a",
      d: msg("a"),
    });
    const cache = client.getQueryData<{ pages: MessagePage[] }>(
      messageKeys.channel("c1"),
    );
    expect(cache?.pages[0]?.messages.map((m) => m.id)).toEqual(["a"]);
  });
});
