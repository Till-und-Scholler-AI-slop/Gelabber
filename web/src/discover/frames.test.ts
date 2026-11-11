import { QueryClient } from "@tanstack/react-query";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { setLocale } from "../i18n/locale.ts";
import type { Server } from "../servers/types.ts";

const mocks = vi.hoisted(() => ({
  servers: [] as unknown[],
  notify: vi.fn(),
  holds: true,
}));

vi.mock("../servers/api.ts", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../servers/api.ts")>()),
  listServers: vi.fn(async () => mocks.servers),
}));
vi.mock("../components/toasts.ts", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../components/toasts.ts")>()),
  notify: mocks.notify,
}));
vi.mock("../auth/scope.ts", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../auth/scope.ts")>()),
  stampHolds: () => mocks.holds,
}));

import { serverKeys } from "../servers/queries.ts";
import { applyDirectoryFrame, directoryKeys } from "./queries.ts";

const stamp = { userId: "lena", generation: 1 };

const server = (id: string, name: string): Server => ({
  id,
  name,
  owner_id: "owner",
  created_at: "2026-01-01T00:00:00Z",
  role: "member",
  permissions: [],
  member_permissions: [],
});

function client() {
  return new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: Infinity } },
  });
}

beforeEach(() => {
  mocks.servers = [];
  mocks.notify.mockClear();
  mocks.holds = true;
});
afterEach(() => setLocale("de", { persist: false }));

describe("jn: a request was accepted", () => {
  it("reads the server list again and names the new server", async () => {
    const queryClient = client();
    const list = serverKeys.list(stamp.userId, stamp.generation);
    queryClient.setQueryData(list, [server("home", "Wohnzimmer")]);
    mocks.servers = [
      server("home", "Wohnzimmer"),
      server("keller", "Modellbau-Keller"),
    ];
    await applyDirectoryFrame(queryClient, stamp, { op: "jn", s: "keller" });
    expect(queryClient.getQueryData<Server[]>(list)?.map((s) => s.id)).toEqual([
      "home",
      "keller",
    ]);
    expect(mocks.notify).toHaveBeenCalledWith(
      "Du bist jetzt Mitglied von Modellbau-Keller.",
    );
  });

  it("speaks English", async () => {
    setLocale("en", { persist: false });
    mocks.servers = [server("keller", "Modellbau-Keller")];
    await applyDirectoryFrame(client(), stamp, { op: "jn", s: "keller" });
    expect(mocks.notify).toHaveBeenCalledWith(
      "You are now a member of Modellbau-Keller.",
    );
  });

  it("marks the directory stale, so the row turns into Öffnen", async () => {
    const queryClient = client();
    const search = directoryKeys.search(stamp.userId, stamp.generation, {});
    queryClient.setQueryData(search, { pages: [], pageParams: [] });
    await applyDirectoryFrame(queryClient, stamp, { op: "jn", s: "keller" });
    expect(queryClient.getQueryState(search)?.isInvalidated).toBe(true);
  });

  it("says nothing for a server the list does not hold, or another account", async () => {
    mocks.servers = [server("home", "Wohnzimmer")];
    await applyDirectoryFrame(client(), stamp, { op: "jn", s: "keller" });
    mocks.servers = [server("keller", "Modellbau-Keller")];
    mocks.holds = false;
    await applyDirectoryFrame(client(), stamp, { op: "jn", s: "keller" });
    expect(mocks.notify).not.toHaveBeenCalled();
  });
});

describe("jq: a queue changed", () => {
  it("sets the count and marks the queue stale", async () => {
    const queryClient = client();
    const requests = directoryKeys.requests(
      stamp.userId,
      stamp.generation,
      "home",
    );
    const other = directoryKeys.requests(
      stamp.userId,
      stamp.generation,
      "other",
    );
    queryClient.setQueryData(requests, []);
    queryClient.setQueryData(other, []);
    await applyDirectoryFrame(queryClient, stamp, {
      op: "jq",
      s: "home",
      n: 3,
    });
    expect(
      queryClient.getQueryData(
        directoryKeys.requestCount(stamp.userId, stamp.generation, "home"),
      ),
    ).toBe(3);
    expect(queryClient.getQueryState(requests)?.isInvalidated).toBe(true);
    expect(queryClient.getQueryState(other)?.isInvalidated).toBe(false);
    expect(mocks.notify).not.toHaveBeenCalled();
  });

  it("leaves the other account frames alone", async () => {
    const queryClient = client();
    await applyDirectoryFrame(queryClient, stamp, { op: "ns" });
    expect(queryClient.getQueryCache().getAll()).toHaveLength(0);
  });
});
