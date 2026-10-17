import { describe, expect, it } from "vitest";

import { isContinued, refreshReplies } from "./pages.ts";
import {
  REPLY_EXCERPT_MAX,
  asReply,
  replyExcerpt,
  replyTo,
  type Message,
  type MessagePage,
} from "./types.ts";

function msg(id: string, extra: Partial<Message> = {}): Message {
  return {
    id,
    channel_id: "c1",
    author: { id: "u1", name: "Ada", avatar_url: null },
    content: id,
    created_at: "2026-10-16T18:00:00.000Z",
    edited_at: null,
    attachments: [],
    ...extra,
  };
}

describe("replies", () => {
  it("cuts the excerpt like the server", () => {
    expect(replyExcerpt("hallo\n  welt")).toBe("hallo welt");
    const long = replyExcerpt("ä".repeat(REPLY_EXCERPT_MAX + 3));
    expect(Array.from(long)).toHaveLength(REPLY_EXCERPT_MAX + 1);
    expect(long.endsWith("…")).toBe(true);
  });

  it("builds the optimistic preview from the original", () => {
    expect(
      replyTo(
        msg("o", {
          content: "frage",
          attachments: [
            { id: "f", filename: "a.pdf", content_type: "x", size: 1 },
          ],
        }),
      ),
    ).toEqual({
      id: "o",
      deleted: false,
      author: { id: "u1", name: "Ada" },
      excerpt: "frage",
      has_attachment: true,
      has_gif: false,
    });
  });

  it("accepts only usable wire replies", () => {
    expect(asReply(null)).toBeNull();
    expect(asReply({ deleted: true })).toBeNull();
    expect(asReply({ id: "o", deleted: true, excerpt: "x" })).toEqual({
      id: "o",
      deleted: true,
    });
    expect(asReply({ id: "o", deleted: false })).toMatchObject({
      excerpt: "",
      has_attachment: false,
    });
  });

  it("starts a new group with a reply", () => {
    const first = msg("a");
    expect(isContinued(first, msg("b"))).toBe(true);
    expect(isContinued(first, msg("b", { reply: replyTo(first) }))).toBe(false);
  });

  it("follows an edit or delete of the original", () => {
    const original = msg("o", { content: "alt" });
    const reply = msg("r", { reply: replyTo(original) });
    const other: MessagePage = { messages: [msg("x")], has_more: false };
    const pages: MessagePage[] = [
      { messages: [original, reply], has_more: true },
      other,
    ];
    const edited = refreshReplies(pages, "o", { ...original, content: "neu" });
    expect(edited[0]!.messages[1]!.reply?.excerpt).toBe("neu");
    expect(edited[1]).toBe(other);
    const deleted = refreshReplies(edited, "o", null);
    expect(deleted[0]!.messages[1]!.reply).toEqual({ id: "o", deleted: true });
  });
});
