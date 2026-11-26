import { describe, expect, it } from "vitest";
import type { Badge } from "../messages/notifyLevel.ts";
import type { ReadState } from "../messages/readState.ts";
import type { Message } from "../messages/types.ts";
import type { Channel } from "./types.ts";
import {
  BUSY_UNREAD,
  callRooms,
  CARDS_MAX,
  greetingKey,
  isPicture,
  mentionsMe,
  missedCards,
  sharedShown,
  shownCount,
  unreadTail,
} from "./overview.ts";

const text = (id: string, name = id) =>
  ({ id, name, kind: "text", server_id: "s" }) as Channel;
const row = (
  channel_id: string,
  unread_count: number,
  latest_order: number,
  mention_count = 0,
  read_order: number | null = null,
) =>
  ({
    channel_id,
    server_id: "s",
    unread_count,
    latest_order,
    mention_count,
    read_order,
  }) as ReadState;
const unread = (count = 0): Badge => ({ count, dot: true });

describe("greeting", () => {
  it("greets by the local hour", () => {
    expect(greetingKey(4)).toBe("overview.greetingEvening");
    expect(greetingKey(5)).toBe("overview.greetingMorning");
    expect(greetingKey(10)).toBe("overview.greetingMorning");
    expect(greetingKey(11)).toBe("overview.greetingDay");
    expect(greetingKey(17)).toBe("overview.greetingDay");
    expect(greetingKey(18)).toBe("overview.greetingEvening");
    expect(greetingKey(23)).toBe("overview.greetingEvening");
    expect(greetingKey(0)).toBe("overview.greetingEvening");
  });
});

describe("missed channels", () => {
  const channels = [
    text("general"),
    text("homework"),
    text("memes"),
    { id: "voice", name: "Voice", kind: "voice" } as Channel,
    text("tmp:new"),
    text("quiet"),
  ];

  it("puts mentions first, then the newest message, and leaves out read channels", () => {
    const rows = new Map([
      ["general", row("general", 5, 40, 1, 35)],
      ["homework", row("homework", 3, 50)],
      ["memes", row("memes", 12, 45)],
      ["quiet", row("quiet", 0, 60)],
    ]);
    const badges = new Map<string, Badge>([
      ["general", unread(1)],
      ["homework", unread()],
      ["memes", unread()],
      ["quiet", { count: 0, dot: false }],
      ["voice", unread()],
      ["tmp:new", unread()],
    ]);
    const { cards, hidden } = missedCards(channels, rows, badges);
    expect(cards.map((c) => c.channel.id)).toEqual([
      "general",
      "homework",
      "memes",
    ]);
    expect(hidden).toBe(0);
    expect(cards[0]).toMatchObject({
      unread: 5,
      mentioned: true,
      readOrder: 35,
      busy: false,
    });
    // Busy without a mention: dimmed, one message.
    expect(cards[2]?.busy).toBe(true);
    expect(cards[2]?.unread).toBeGreaterThanOrEqual(BUSY_UNREAD);
  });

  it("keeps the sidebar's order between equals and caps the cards", () => {
    const many = Array.from({ length: CARDS_MAX + 2 }, (_, n) => text(`c${n}`));
    const rows = new Map(many.map((c) => [c.id, row(c.id, 1, 7)]));
    const badges = new Map(many.map((c) => [c.id, unread()]));
    const { cards, hidden } = missedCards(many, rows, badges);
    expect(cards.map((c) => c.channel.id)).toEqual(["c0", "c1", "c2", "c3"]);
    expect(hidden).toBe(2);
  });

  it("gives one channel alone more room and a busy one less", () => {
    const card = {
      channel: text("a"),
      unread: 3,
      mentioned: false,
      latest: 1,
      readOrder: null,
      busy: false,
    };
    expect(shownCount(card, true)).toBe(5);
    expect(shownCount(card, false)).toBe(2);
    expect(shownCount({ ...card, busy: true }, false)).toBe(1);
    expect(shownCount({ ...card, busy: true }, true)).toBe(5);
  });
});

describe("unread messages of a card", () => {
  const page = [10, 11, 12, 13].map(
    (order) => ({ id: `m${order}`, created_order: order }) as Message,
  );
  it("takes the newest ones after the read position", () => {
    expect(unreadTail(page, 11, 5).map((m) => m.id)).toEqual(["m12", "m13"]);
    expect(unreadTail(page, 11, 1).map((m) => m.id)).toEqual(["m13"]);
    expect(unreadTail(page, null, 2).map((m) => m.id)).toEqual(["m12", "m13"]);
    expect(unreadTail(page, 13, 2)).toEqual([]);
  });
});

describe("mentions and files", () => {
  it("knows a message that reaches me", () => {
    expect(mentionsMe({ mentions: ["me"] }, "me")).toBe(true);
    expect(mentionsMe({ mentions: ["mia"] }, "me")).toBe(false);
    expect(mentionsMe({ mentions: [], mention_everyone: true }, "me")).toBe(
      true,
    );
  });

  it("leaves out files of messages already shown and keeps four", () => {
    const files = ["a", "b", "c", "d", "e", "f"].map((id) => ({
      id,
      message_id: `m-${id}`,
    }));
    expect(sharedShown(files, new Set(["m-b"])).map((file) => file.id)).toEqual(
      ["a", "c", "d", "e"],
    );
  });

  it("tells pictures from other files", () => {
    expect(isPicture("image/png")).toBe(true);
    expect(isPicture("image/svg+xml")).toBe(false);
    expect(isPicture("application/pdf")).toBe(false);
  });
});

describe("who is in which call", () => {
  const voice = (id: string) =>
    ({ id, name: id, kind: "voice", server_id: "s" }) as Channel;
  const order = [
    text("allgemein"),
    voice("sofa"),
    voice("kino"),
    voice("leer"),
  ];
  const seat = (channelId: string) => ({ channelId });

  it("lists only voice channels with someone in them, fullest first", () => {
    const rooms = callRooms(
      order,
      { a: seat("sofa"), b: seat("kino"), c: seat("kino") },
      {},
      null,
    );
    expect(rooms.map((r) => [r.channel.id, r.people])).toEqual([
      ["kino", ["b", "c"]],
      ["sofa", ["a"]],
    ]);
  });

  it("puts the user's own call first, then keeps the sidebar's order", () => {
    const rooms = callRooms(
      order,
      { a: seat("sofa"), b: seat("kino"), c: seat("kino"), d: seat("leer") },
      {},
      "leer",
    );
    expect(rooms.map((r) => [r.channel.id, r.mine])).toEqual([
      ["leer", true],
      ["kino", false],
      ["sofa", false],
    ]);
    const even = callRooms(
      order,
      { a: seat("kino"), b: seat("sofa") },
      {},
      null,
    );
    expect(even.map((r) => r.channel.id)).toEqual(["sofa", "kino"]);
  });

  it("marks a channel where someone shares a screen", () => {
    const rooms = callRooms(
      order,
      { a: seat("sofa"), b: seat("kino") },
      { sofa: { a: { s: true } }, kino: { b: {} } },
      null,
    );
    expect(rooms.map((r) => [r.channel.id, r.live])).toEqual([
      ["sofa", true],
      ["kino", false],
    ]);
  });

  it("ignores seats in channels the server does not list or that hold text", () => {
    expect(
      callRooms(order, { a: seat("gone"), b: seat("allgemein") }, {}, null),
    ).toEqual([]);
  });
});
