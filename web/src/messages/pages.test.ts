import { describe, expect, it } from "vitest";

import {
  confirmedPendingIds,
  encodeCursor,
  flattenPages,
  isContinued,
  olderCursor,
  stampOlder,
  visibleMessages,
} from "./pages.ts";
import type { Message, MessagePage } from "./types.ts";

function msg(id: string, created = "2026-09-13T18:00:00.000Z"): Message {
  return {
    id,
    channel_id: "c1",
    author: { id: "u1", name: "Ada", avatar_url: null },
    content: id,
    created_at: created,
    edited_at: null,
    attachments: [],
  };
}

describe("message pages", () => {
  it("flattens older pages in front of the newest batch", () => {
    const newest: MessagePage = {
      messages: [msg("b"), msg("c")],
      has_more: true,
    };
    const older: MessagePage = { messages: [msg("a")], has_more: false };
    expect(flattenPages([newest, older]).map((m) => m.id)).toEqual([
      "a",
      "b",
      "c",
    ]);
    expect(olderCursor(newest)).toBe(encodeCursor(msg("b")));
    expect(olderCursor(older)).toBeUndefined();
  });

  it("keeps paging after the oldest row was deleted from the page", () => {
    const stamped = stampOlder({
      messages: [msg("b"), msg("c")],
      has_more: true,
    });
    expect(olderCursor({ ...stamped, messages: [] })).toBe(
      encodeCursor(msg("b")),
    );
  });

  it("appends pending rows that are not already in a page", () => {
    const pages: MessagePage[] = [{ messages: [msg("a")], has_more: false }];
    const pending = [msg("tmp:1"), msg("a")];
    expect(visibleMessages(pages, pending).map((m) => m.id)).toEqual([
      "a",
      "tmp:1",
    ]);
    expect(confirmedPendingIds(pages, [msg("a"), msg("tmp:1")])).toEqual(["a"]);
  });

  it("does not show a confirmed overlay row next to the same GET id", () => {
    const real = msg("real-1");
    const pages: MessagePage[] = [{ messages: [real], has_more: false }];
    expect(visibleMessages(pages, [real]).map((m) => m.id)).toEqual(["real-1"]);
  });

  it("groups the same author inside a five-minute window", () => {
    const first = msg("a", "2026-09-13T18:00:00.000Z");
    const soon = {
      ...msg("b", "2026-09-13T18:03:00.000Z"),
    };
    const later = msg("c", "2026-09-13T18:06:00.000Z");
    const other = {
      ...msg("d", "2026-09-13T18:03:30.000Z"),
      author: { id: "u2", name: "Bob", avatar_url: null },
    };
    expect(isContinued(first, soon)).toBe(true);
    expect(isContinued(first, later)).toBe(false);
    expect(isContinued(first, other)).toBe(false);
    expect(isContinued(undefined, first)).toBe(false);
  });
});
