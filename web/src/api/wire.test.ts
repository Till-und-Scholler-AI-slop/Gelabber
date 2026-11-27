import { afterEach, describe, expect, it, vi } from "vitest";

import { ApiError } from "./client.ts";
import {
  isMessage,
  isReadState,
  isReadSync,
  isServerDetail,
  isUser,
  reportMalformed,
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

describe("paging past left-out rows", () => {
  it("goes on from the oldest row the server sent, even if none was kept", async () => {
    const { olderCursor } = await import("../messages/pages.ts");
    vi.spyOn(console, "warn").mockImplementation(() => {});
    const broken = (id: string, created_at: string) => ({
      ...without(message, "mentions"),
      id,
      created_at,
    });
    const page = wholePage({
      messages: [
        broken("m1", "2026-11-27T18:00:00Z"),
        broken("m2", "2026-11-27T18:01:00Z"),
      ],
      has_more: true,
    });
    expect(page.messages).toEqual([]);
    expect(olderCursor(page)).toBe("2026-11-27T18:00:00Z|m1");
  });
});

describe("everything a server sends, filled in", () => {
  // One object per type with every field the backend's structs serialise,
  // optional ones included, as v0.13 and v0.14 send them.
  it("takes a message with every optional part", () => {
    const full = {
      ...message,
      edited_at: "2026-11-27T18:05:00Z",
      nonce: "00000000-0000-4000-8000-000000000009",
      attachments: [
        {
          id: "a1",
          filename: "bild.png",
          content_type: "image/png",
          size: 2048,
          width: 640,
          height: 480,
          placeholder: "#336699",
          placeholder_version: 1,
          animated: true,
          thumbnail: true,
          description: "Ein Hund am Strand",
        },
      ],
      reactions: [{ emoji: "❤️", user_ids: ["u2", "u3"] }],
      mentions: ["u2"],
      mention_everyone: true,
      gif: {
        provider: "klipy",
        slug: "dog",
        title: "Hund",
        width: 200,
        height: 150,
        url: "https://static.klipy.com/dog.webp",
        still: "https://static.klipy.com/dog.png",
      },
      reply: {
        id: "m0",
        deleted: false,
        author: { id: "u2", name: "Bea" },
        excerpt: "Wer kommt mit?",
        has_attachment: true,
        has_gif: true,
        system: { kind: "live_started", params: { voice_name: "Sofa" } },
      },
      previews: [
        {
          url: "https://example.test/artikel",
          title: "Artikel",
          description: "Ein Text",
          site_name: "Example",
          image: { url: "/api/previews/p1", width: 1200, height: 630 },
          youtube_id: "dQw4w9WgXcQ",
        },
      ],
      system: { kind: "live_started", params: { voice_name: "Sofa" } },
    };
    expect(isMessage(full)).toBe(true);
  });

  it("takes a server detail with roles, a limited channel and full members", () => {
    const full = {
      id: "s1",
      name: "Wohnzimmer",
      owner_id: "u1",
      icon_url: "/api/server-icons/s1",
      created_at: "2026-01-01T00:00:00Z",
      role: "member",
      permissions: ["send_messages", "join_voice"],
      member_permissions: ["send_messages"],
      blocked: true,
      categories: [
        {
          id: "k1",
          server_id: "s1",
          name: "Plaudern",
          created_at: "2026-01-01T00:00:00Z",
        },
      ],
      channels: [
        {
          id: "c1",
          server_id: "s1",
          category_id: "k1",
          name: "spiele",
          kind: "text",
          created_at: "2026-01-01T00:00:00Z",
          limited: true,
          role_ids: ["r1"],
        },
      ],
      members: [
        {
          user_id: "u2",
          name: "Bea",
          avatar_url: "/api/avatars/u2",
          joined_at: "2026-01-02T00:00:00Z",
          status_text: "Im Urlaub",
          accent_color: "#336699",
          avatar_decoration: "ring",
          role: "member",
          role_ids: ["r1"],
        },
      ],
      roles: [
        {
          id: "r1",
          server_id: "s1",
          name: "Moderation",
          color: "blue",
          permissions: ["moderate_members"],
          position: 0,
          created_at: "2026-01-01T00:00:00Z",
        },
      ],
    };
    expect(isServerDetail(full)).toBe(true);
  });

  it("takes a read row with its settings and an account with its card", () => {
    expect(
      isReadState({
        channel_id: "c1",
        server_id: "s1",
        read_message_id: "m1",
        read_at: "2026-11-27T18:00:00Z",
        read_order: 41,
        membership_at: "2026-01-02T00:00:00Z",
        latest_order: 44,
        unread_count: 3,
        mention_count: 1,
        level: "mentions",
        muted: true,
        muted_until: "2026-11-28T08:00:00Z",
      }),
    ).toBe(true);
    expect(
      isUser({
        id: "u1",
        email: "ada@example.test",
        name: "Ada",
        avatar_url: "/api/avatars/u1",
        created_at: "2026-01-01T00:00:00Z",
        bio: "Hallo",
        pronouns: "sie/ihr",
        status_text: "Da",
        accent_color: "#336699",
        banner_color: "#112233",
        banner_url: "/api/banners/u1",
        avatar_decoration: "stars",
        locale: "en",
      }),
    ).toBe(true);
  });
});

describe("telling what was left out", () => {
  it("tells once per message, whether it came by REST or by the gateway", () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const broken = without(message, "mentions");
    wholeRows([broken], isMessage, "message");
    reportMalformed("message event", broken);
    expect(warn).toHaveBeenCalledTimes(1);
  });
});
