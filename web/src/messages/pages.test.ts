import { describe, expect, it } from "vitest";

import {
  arrivedSince,
  confirmedPendingIds,
  encodeCursor,
  flattenPages,
  isContinued,
  olderCursor,
  stampOlder,
  unreadAnchor,
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

describe("unread anchor", () => {
  const rows = ["a", "b", "c", "d"].map((id, n) => ({
    ...msg(id),
    created_order: (n + 1) * 10,
  }));
  const read = (id: string, order?: number) => ({ id, order });

  it("opens at the first row after the last read one", () => {
    expect(unreadAnchor(rows, read("b"), true)).toEqual({
      kind: "row",
      index: 2,
    });
  });

  it("opens at the end when nothing is unread", () => {
    expect(unreadAnchor(rows, null, true)).toEqual({ kind: "end" });
    expect(unreadAnchor(rows, read("d"), true)).toEqual({ kind: "end" });
  });

  it("pages further while the last read message is not loaded", () => {
    expect(unreadAnchor(rows, read("x"), true)).toEqual({ kind: "older" });
    expect(unreadAnchor(rows, read("x", 5), true)).toEqual({ kind: "older" });
  });

  it("opens at the end when the last read message is gone for good", () => {
    expect(unreadAnchor(rows, read("x"), false)).toEqual({ kind: "end" });
  });

  it("finds the first unread row by order when the read one was deleted", () => {
    expect(unreadAnchor(rows, read("x", 25), true)).toEqual({
      kind: "row",
      index: 2,
    });
    expect(unreadAnchor(rows, read("x", 5), false)).toEqual({
      kind: "row",
      index: 0,
    });
    expect(unreadAnchor(rows, read("x", 45), true)).toEqual({ kind: "end" });
  });

  it("does not count an own message that is still being sent", () => {
    expect(unreadAnchor([...rows, msg("tmp:1")], read("d"), false)).toEqual({
      kind: "end",
    });
  });
});

describe("arrivals while scrolled up", () => {
  const from = (id: string, author: string) => ({
    ...msg(id),
    author: { id: author, name: author, avatar_url: null },
  });

  it("counts what others wrote below the row the reader left", () => {
    const rows = [
      from("a", "ada"),
      from("b", "bo"),
      from("c", "me"),
      from("d", "bo"),
      from("tmp:1", "me"),
    ];
    expect(arrivedSince(rows, "a", "me")).toBe(2);
    expect(arrivedSince(rows, "d", "me")).toBe(0);
    expect(arrivedSince(rows, "gone", "me")).toBe(0);
  });
});
