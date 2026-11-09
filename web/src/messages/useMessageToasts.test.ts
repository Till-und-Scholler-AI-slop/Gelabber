import { QueryClient } from "@tanstack/react-query";
import { describe, expect, it } from "vitest";

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
