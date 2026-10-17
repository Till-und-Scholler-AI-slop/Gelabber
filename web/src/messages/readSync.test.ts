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

  it("orders by position and membership, not by read time", () => {
    const client = new QueryClient();
    const stamp = takeStamp()!;
    const at = (
      order: number | null,
      latest: number,
      joined = "2026-10-01T00:00:00Z",
    ) => ({
      read_order: order,
      membership_at: joined,
      latest_order: latest,
    });
    client.setQueryData<ReadState[]>(readKey(stamp), [
      { ...row("a", "2026-10-17T12:00:00Z", 2), ...at(5, 7) },
      { ...row("b", "2026-10-17T12:00:00Z", 2), ...at(5, 7) },
    ]);
    applyReadSync(client, stamp, [
      // Further along although its message is older (an edited clock).
      { ...row("a", "2026-10-17T09:00:00Z", 0), ...at(7, 7) },
      // A rejoin: the new membership's position wins.
      {
        ...row("b", "2026-10-17T08:00:00Z", 1),
        ...at(1, 7, "2026-10-10T00:00:00Z"),
      },
    ]);
    const rows = client.getQueryData<ReadState[]>(readKey(stamp))!;
    expect(rows[0]).toMatchObject({ read_order: 7, unread_count: 0 });
    expect(rows[1]).toMatchObject({ read_order: 1, unread_count: 1 });
    // Same position, counted before newer messages: stays.
    applyReadSync(client, stamp, [
      { ...row("a", "2026-10-17T09:00:00Z", 0), ...at(7, 6) },
    ]);
    expect(client.getQueryData<ReadState[]>(readKey(stamp))![0]).toMatchObject({
      latest_order: 7,
    });
  });

  it("asks for the counts again when the frame counted fewer messages", () => {
    const client = new QueryClient();
    const stamp = takeStamp()!;
    const invalidate = vi.spyOn(client, "invalidateQueries");
    // The snapshot already saw message 9; the read frame was taken at 8.
    client.setQueryData<ReadState[]>(readKey(stamp), [
      {
        ...row("a", "2026-10-17T10:00:00Z", 1),
        read_order: 8,
        membership_at: "2026-10-01T00:00:00Z",
        latest_order: 9,
      },
    ]);
    applyReadSync(client, stamp, [
      {
        ...row("a", "2026-10-17T10:00:01Z", 0),
        read_order: 8,
        membership_at: "2026-10-01T00:00:00Z",
        latest_order: 8,
      },
    ]);
    // Same position with an older count: nothing changes, nothing to ask.
    expect(client.getQueryData<ReadState[]>(readKey(stamp))![0]).toMatchObject({
      unread_count: 1,
    });
    applyReadSync(client, stamp, [
      {
        ...row("a", "2026-10-17T10:00:02Z", 0),
        read_order: 8.5,
        membership_at: "2026-10-01T00:00:00Z",
        latest_order: 8,
      },
    ]);
    expect(client.getQueryData<ReadState[]>(readKey(stamp))![0]).toMatchObject({
      read_order: 8.5,
      unread_count: 0,
    });
    return vi.waitFor(() => expect(invalidate).toHaveBeenCalled());
  });
});
