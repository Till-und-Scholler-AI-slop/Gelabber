import { describe, expect, it } from "vitest";

import { isContinued } from "./pages.ts";
import { asSystem, systemText } from "./system.ts";
import { asReply, replyTo, type Message } from "./types.ts";

const live = {
  kind: "live_started",
  params: { voice_channel_id: "v1", voice_name: "Lounge" },
};

function msg(id: string, extra: Partial<Message> = {}): Message {
  return {
    id,
    channel_id: "c1",
    author: { id: "u1", name: "Ada", avatar_url: null },
    content: id,
    created_at: "2026-10-22T12:00:00.000Z",
    edited_at: null,
    attachments: [],
    ...extra,
  };
}

describe("system messages", () => {
  it("renders the live notice like the server's push", () => {
    expect(systemText(live, "Ada")).toBe("Ada ist live in Lounge.");
  });

  it("names a kind it does not know yet", () => {
    expect(systemText({ kind: "later", params: {} }, "Ada")).toBe(
      "Systemnachricht von Ada",
    );
  });

  it("accepts only a usable wire shape", () => {
    expect(asSystem(null)).toBeNull();
    expect(asSystem({ params: {} })).toBeNull();
    expect(asSystem({ kind: "live_started", params: [] })).toEqual({
      kind: "live_started",
      params: {},
    });
    expect(asSystem(live)).toEqual(live);
  });

  it("is a line of its own, not part of a group", () => {
    const notice = msg("n", { content: "", system: live });
    expect(isContinued(msg("a"), notice)).toBe(false);
    expect(isContinued(notice, msg("b"))).toBe(false);
  });

  it("carries the kind into a reply's preview", () => {
    const notice = msg("n", { content: "", system: live });
    expect(replyTo(notice)).toMatchObject({ excerpt: "", system: live });
    expect(replyTo(msg("a"))).not.toHaveProperty("system");
    expect(
      asReply({ id: "n", deleted: false, excerpt: "", system: live }),
    ).toMatchObject({ system: live });
  });
});
