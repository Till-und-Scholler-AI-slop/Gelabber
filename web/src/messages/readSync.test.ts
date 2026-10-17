import { QueryClient } from "@tanstack/react-query";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { login, resetSessionForTests } from "../auth/session.ts";
import { takeStamp } from "../auth/scope.ts";
import { applyReadSync, readKey, type ReadState } from "./readState.ts";

beforeEach(async () => {
  resetSessionForTests();
  vi.stubGlobal(
    "fetch",
    vi.fn(() =>
      Promise.resolve(
        new Response(
          JSON.stringify({
            user: {
              id: "sync-user",
              email: "sync@example.test",
              name: "Sync",
              avatar_url: null,
              created_at: "2026-10-05T00:00:00Z",
            },
            csrf_token: "csrf",
          }),
          { headers: { "Content-Type": "application/json" } },
        ),
      ),
    ),
  );
  await login("sync@example.test", "password");
});
afterEach(() => {
  resetSessionForTests();
  vi.unstubAllGlobals();
});
const row = (channel_id: string, read_at: string | null, unread: number) =>
  ({
    channel_id,
    server_id: "s",
    read_message_id: read_at ? `m-${read_at}` : null,
    read_at,
    unread_count: unread,
    mention_count: unread ? 1 : 0,
    level: "mentions",
    muted: true,
    muted_until: null,
  }) satisfies ReadState;

describe("read positions from another device", () => {
  it("move rows forward, never back, and keep the settings", () => {
    const client = new QueryClient();
    const stamp = takeStamp()!;
    client.setQueryData<ReadState[]>(readKey(stamp), [
      row("a", "2026-10-17T10:00:00Z", 4),
      row("b", "2026-10-17T12:00:00Z", 0),
      row("c", null, 2),
    ]);
    applyReadSync(client, stamp, [
      { ...row("a", "2026-10-17T11:00:00Z", 0) },
      // Older than what this window has: a frame that came late.
      { ...row("b", "2026-10-17T11:00:00Z", 3) },
    ]);
    const rows = client.getQueryData<ReadState[]>(readKey(stamp))!;
    expect(rows[0]).toMatchObject({
      read_at: "2026-10-17T11:00:00Z",
      unread_count: 0,
      mention_count: 0,
      level: "mentions",
      muted: true,
    });
    expect(rows[1]).toMatchObject({
      read_at: "2026-10-17T12:00:00Z",
      unread_count: 0,
    });
    expect(rows[2]).toMatchObject({ unread_count: 2 });
  });
});
