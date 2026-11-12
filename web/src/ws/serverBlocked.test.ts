import { QueryClient } from "@tanstack/react-query";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { serverKeys } from "../servers/queries.ts";
import type { Server } from "../servers/types.ts";

const voice = vi.hoisted(() => ({
  state: { status: "idle", serverId: null as string | null },
  leave: vi.fn(),
}));

vi.mock("../voice/session.ts", () => ({
  useVoice: { getState: () => voice.state },
  leaveVoice: voice.leave,
}));

import { applyServerBlocked } from "./useRealtime.ts";

const server = (id: string, extra: Partial<Server> = {}): Server => ({
  id,
  name: `Server ${id}`,
  owner_id: "owner",
  created_at: "2026-01-01T00:00:00Z",
  role: "member",
  permissions: [],
  member_permissions: [],
  ...extra,
});

function client() {
  return new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: Infinity } },
  });
}

beforeEach(() => {
  voice.state = { status: "idle", serverId: null };
  voice.leave.mockClear();
});

describe("err server_blocked", () => {
  const list = serverKeys.list("user-a", 1);
  const detail = serverKeys.detail("user-a", 1, "s1");
  const invites = serverKeys.invites("user-a", 1, "s1");
  const other = serverKeys.detail("user-a", 1, "s2");

  it("keeps the server in the list, marked, and drops its cached details", () => {
    const queryClient = client();
    queryClient.setQueryData(list, [server("s1"), server("s2")]);
    for (const key of [detail, invites, other])
      queryClient.setQueryData(key, {});

    expect(applyServerBlocked(queryClient, "user-a", 1, "s1")).toBe(true);

    expect(queryClient.getQueryData<Server[]>(list)).toEqual([
      server("s1", { blocked: true }),
      server("s2"),
    ]);
    expect(queryClient.getQueryData(detail)).toBeUndefined();
    expect(queryClient.getQueryData(invites)).toBeUndefined();
    expect(queryClient.getQueryData(other)).toEqual({});
  });

  it("ends voice on that server only", () => {
    const queryClient = client();
    queryClient.setQueryData(list, [server("s1"), server("s2")]);
    voice.state = { status: "joined", serverId: "s2" };
    applyServerBlocked(queryClient, "user-a", 1, "s1");
    expect(voice.leave).not.toHaveBeenCalled();
    voice.state = { status: "joined", serverId: "s1" };
    applyServerBlocked(queryClient, "user-a", 1, "s1");
    expect(voice.leave).toHaveBeenCalledTimes(1);
  });

  it("is news only once: a later subscribe's error changes nothing more", () => {
    const queryClient = client();
    queryClient.setQueryData(list, [server("s1")]);
    expect(applyServerBlocked(queryClient, "user-a", 1, "s1")).toBe(true);
    queryClient.setQueryData(detail, {});
    expect(applyServerBlocked(queryClient, "user-a", 1, "s1")).toBe(false);
    // Nothing removed the second time.
    expect(queryClient.getQueryData(detail)).toEqual({});
    expect(queryClient.getQueryData<Server[]>(list)).toEqual([
      server("s1", { blocked: true }),
    ]);
  });
});
