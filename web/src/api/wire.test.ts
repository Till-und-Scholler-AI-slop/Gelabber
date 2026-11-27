import { afterEach, describe, expect, it, vi } from "vitest";

import { ApiError } from "./client.ts";
import {
  isMessage,
  isReadSync,
  isServerDetail,
  isUser,
  resetMalformedReports,
  whole,
  wholePage,
  wholeRows,
} from "./wire.ts";

const message = {
  id: "m1",
  channel_id: "c1",
  author: { id: "u1", name: "Ada", avatar_url: null },
  content: "Hallo",
  created_at: "2026-11-27T18:00:00Z",
  edited_at: null,
  revision: 1,
  created_order: 1,
  attachments: [],
  reactions: [{ emoji: "👍", user_ids: ["u2"] }],
  mentions: [],
  mention_everyone: false,
};
const without = (value: object, key: string) =>
  Object.fromEntries(Object.entries(value).filter(([k]) => k !== key));

afterEach(() => {
  vi.restoreAllMocks();
  resetMalformedReports();
});

describe("what the server hands over", () => {
  it("takes a message only with every field its type promises", () => {
    expect(isMessage(message)).toBe(true);
    for (const key of [
      "mentions",
      "mention_everyone",
      "reactions",
      "attachments",
      "edited_at",
    ])
      expect(isMessage(without(message, key)), key).toBe(false);
    expect(isMessage({ ...message, reactions: [{ emoji: "👍" }] })).toBe(false);
    // An unsent message has neither, and the server's always has both.
    expect(isMessage(without(message, "revision"))).toBe(true);
  });

  it("leaves a malformed row out of a page and keeps the others", () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const broken = { ...without(message, "mentions"), id: "m2" };
    const page = { messages: [message, broken], has_more: false };
    expect(wholePage(page).messages).toEqual([message]);
    // Told once per row, not on every refetch.
    wholePage(page);
    expect(warn).toHaveBeenCalledTimes(1);
    expect(() => wholePage({ messages: [message] })).toThrow(ApiError);
  });

  it("fails a single answer that is not whole", () => {
    vi.spyOn(console, "warn").mockImplementation(() => {});
    const detail = {
      id: "s1",
      name: "Wohnzimmer",
      owner_id: "u1",
      icon_url: null,
      created_at: "2026-01-01T00:00:00Z",
      role: "owner",
      permissions: [],
      member_permissions: [],
      blocked: false,
      categories: [],
      channels: [],
      members: [],
      roles: [],
    };
    expect(whole(detail, isServerDetail, "server detail")).toBe(detail);
    expect(() =>
      whole(without(detail, "roles"), isServerDetail, "server detail"),
    ).toThrow(ApiError);
    expect(
      isUser({
        id: "u1",
        email: "ada@example.test",
        name: "Ada",
        avatar_url: null,
        created_at: "2026-01-01T00:00:00Z",
      }),
    ).toBe(false);
  });

  it("drops read rows without their position", () => {
    vi.spyOn(console, "warn").mockImplementation(() => {});
    const row = {
      channel_id: "c1",
      server_id: null,
      read_message_id: null,
      read_at: null,
      read_order: null,
      membership_at: "2026-01-01T00:00:00Z",
      latest_order: 3,
      unread_count: 2,
      mention_count: 0,
    };
    expect(
      wholeRows([row, without(row, "membership_at")], isReadSync, "read sync"),
    ).toEqual([row]);
  });
});
