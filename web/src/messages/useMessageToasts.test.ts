import { QueryClient } from "@tanstack/react-query";
import { describe, expect, it } from "vitest";

import { blockKeys, blocksAuthor } from "../blocks/queries.ts";
import { serverKeys } from "../servers/queries.ts";
import type { ServerDetail } from "../servers/types.ts";
import type { ChatEvent } from "../ws/protocol.ts";
import { channelLabel } from "./useMessageToasts.ts";

const event = (s: string, c: string): ChatEvent => ({
  op: "e",
  t: "c",
  s,
  c,
  n: 1,
});

describe("channelLabel", () => {
  it("says Direktnachricht for a DM, whose author already names it", () => {
    const client = new QueryClient();
    expect(channelLabel(client, "me", 0, event("dm", "dm"), true)).toBe(
      "Direktnachricht",
    );
  });

  it("names a cached channel and falls back to Kanal", () => {
    const client = new QueryClient();
    client.setQueryData(serverKeys.detail("me", 0, "srv"), {
      channels: [{ id: "ch", name: "spiele" }],
    } as unknown as ServerDetail);
    expect(channelLabel(client, "me", 0, event("srv", "ch"), false)).toBe(
      "#spiele",
    );
    expect(channelLabel(client, "me", 0, event("srv", "x"), false)).toBe(
      "Kanal",
    );
  });
});

describe("blocksAuthor", () => {
  it("knows a blocked author from the cached list, nobody before it", () => {
    const client = new QueryClient();
    expect(blocksAuthor(client, "me", 0, "bob")).toBe(false);
    client.setQueryData(blockKeys.list("me", 0), [
      { id: "bob", name: "Bob", avatar_url: null, created_at: "" },
    ]);
    expect(blocksAuthor(client, "me", 0, "bob")).toBe(true);
    expect(blocksAuthor(client, "me", 0, "cara")).toBe(false);
    expect(blocksAuthor(client, "me", 1, "bob")).toBe(false);
  });
});
