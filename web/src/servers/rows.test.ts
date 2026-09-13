import { describe, expect, it } from "vitest";

import { buildRows, pickChannel } from "./rows.ts";
import type { Category, Channel, ServerDetail } from "./types.ts";

const server: ServerDetail = {
  id: "s1",
  name: "Team",
  owner_id: "u1",
  created_at: "2026-09-13T00:00:00Z",
  role: "owner",
  permissions: ["manage_server"],
  member_permissions: [],
  categories: [
    category("cat-text", "Textkanäle"),
    category("cat-empty", "Leer"),
    category("cat-voice", "Voice"),
  ],
  channels: [
    channel("loose", null, "loose", "text"),
    channel("allgemein", "cat-text", "allgemein", "text"),
    channel("lounge", "cat-voice", "Lounge", "voice"),
    channel("dev", "cat-text", "dev", "text"),
  ],
  members: [],
};

function category(id: string, name: string): Category {
  return { id, server_id: "s1", name, created_at: "2026-09-13T00:00:00Z" };
}

function channel(
  id: string,
  category_id: string | null,
  name: string,
  kind: Channel["kind"],
): Channel {
  return {
    id,
    server_id: "s1",
    category_id,
    name,
    kind,
    created_at: "2026-09-13T00:00:00Z",
  };
}

describe("sidebar rows", () => {
  it("lists uncategorised channels first, then categories with their channels", () => {
    const rows = buildRows(server).map((row) =>
      row.kind === "channel"
        ? `channel:${row.channel.id}`
        : row.kind === "category"
          ? `category:${row.category.id}`
          : `empty:${row.categoryId}`,
    );
    expect(rows).toEqual([
      "channel:loose",
      "category:cat-text",
      "channel:allgemein",
      "channel:dev",
      "category:cat-empty",
      "empty:cat-empty",
      "category:cat-voice",
      "channel:lounge",
    ]);
  });

  it("uses unique keys per row", () => {
    const keys = buildRows(server).map((row) => row.key);
    expect(new Set(keys).size).toBe(keys.length);
  });

  it("picks the remembered channel, else the first text channel", () => {
    expect(pickChannel(server, "dev")?.id).toBe("dev");
    expect(pickChannel(server, "gone")?.id).toBe("loose");
    expect(pickChannel(server, undefined)?.id).toBe("loose");
    expect(
      pickChannel(
        { ...server, channels: [channel("v", null, "Voice only", "voice")] },
        undefined,
      )?.id,
    ).toBe("v");
    expect(pickChannel({ ...server, channels: [] }, undefined)).toBeUndefined();
  });
});
