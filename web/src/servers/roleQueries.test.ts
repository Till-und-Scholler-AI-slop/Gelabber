import { MutationObserver, QueryClient } from "@tanstack/react-query";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { takeStamp } from "../auth/scope.ts";
import { login, resetSessionForTests } from "../auth/session.ts";
import {
  createRoleOptions,
  deleteRoleOptions,
  isPendingId,
  refetchOnAccessChange,
  serverKeys,
  setMemberRolesOptions,
  updateRoleOptions,
} from "./queries.ts";
import type {
  Channel,
  Member,
  Server,
  ServerDetail,
  ServerRole,
} from "./types.ts";

const created_at = "2026-11-01T00:00:00Z";
const me = {
  id: "jonas",
  name: "Jonas",
  email: "jonas@test.invalid",
  avatar_url: null,
  created_at,
};

const role = (
  id: string,
  name: string,
  position: number,
  permissions: ServerRole["permissions"] = [],
): ServerRole => ({
  id,
  server_id: "s1",
  name,
  color: "blue",
  permissions,
  position,
  created_at,
});
const member = (user_id: string, role_ids: string[] = []): Member => ({
  user_id,
  name: user_id,
  avatar_url: null,
  joined_at: created_at,
  role: user_id === "lena" ? "owner" : "member",
  role_ids,
});
const spiele: Channel = {
  id: "spiele",
  server_id: "s1",
  category_id: null,
  name: "spiele",
  kind: "text",
  created_at,
  limited: true,
  role_ids: ["r-mod"],
};
const moderation = role("r-mod", "Moderation", 0, ["moderate_members"]);
const regulars = role("r-reg", "Stammgäste", 1);
const listItem: Server = {
  id: "s1",
  name: "Spieleabend",
  owner_id: "lena",
  created_at,
  role: "member",
  permissions: ["moderate_members", "send_messages"],
  member_permissions: ["send_messages"],
};
const detail: ServerDetail = {
  ...listItem,
  categories: [],
  channels: [spiele],
  members: [member("lena"), member("jonas", ["r-mod"]), member("mira")],
  roles: [moderation, regulars],
};

const json = (body: unknown, status = 200) =>
  new Response(body === null ? null : JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
const failure = () => json({ error: "forbidden" }, 403);

let client: QueryClient;
let respond: (url: URL, init: RequestInit) => Response | Promise<Response>;
const requests: { method: string; path: string; body: unknown }[] = [];

function held() {
  let resolve!: (r: Response) => void;
  const promise = new Promise<Response>((r) => {
    resolve = r;
  });
  return { resolve, promise };
}

function keys() {
  const stamp = takeStamp()!;
  return {
    detail: serverKeys.detail(stamp.userId, stamp.generation, "s1"),
    list: serverKeys.list(stamp.userId, stamp.generation),
  };
}
const cachedDetail = () => client.getQueryData<ServerDetail>(keys().detail)!;
const cachedList = () => client.getQueryData<Server[]>(keys().list)!;
const listStale = () => client.getQueryState(keys().list)?.isInvalidated;

beforeEach(async () => {
  resetSessionForTests();
  requests.length = 0;
  client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  respond = () => json(null);
  vi.stubGlobal(
    "fetch",
    vi.fn((input: RequestInfo | URL, init: RequestInit = {}) => {
      const url = new URL(String(input), "https://test.invalid");
      if (url.pathname === "/api/auth/login")
        return Promise.resolve(json({ user: me, csrf_token: "csrf" }));
      requests.push({
        method: init.method ?? "GET",
        path: url.pathname,
        body: typeof init.body === "string" ? JSON.parse(init.body) : null,
      });
      return Promise.resolve(respond(url, init));
    }),
  );
  await login(me.email, "password");
  client.setQueryData(keys().detail, detail);
  client.setQueryData(keys().list, [listItem]);
});

afterEach(() => {
  client.clear();
  resetSessionForTests();
  vi.unstubAllGlobals();
});

describe("useCreateRole", () => {
  it("shows a pending role at once and swaps in the created one", async () => {
    const reply = held();
    respond = () => reply.promise;
    const observer = new MutationObserver(
      client,
      createRoleOptions(client, "s1"),
    );
    const done = observer.mutate({ name: " Neue Rolle ", color: "green" });

    await vi.waitFor(() => expect(cachedDetail().roles).toHaveLength(3));
    const pending = cachedDetail().roles[2]!;
    expect(isPendingId(pending.id)).toBe(true);
    expect(pending).toMatchObject({
      name: "Neue Rolle",
      color: "green",
      permissions: [],
      position: 2,
    });
    expect(requests.at(-1)).toEqual({
      method: "POST",
      path: "/api/servers/s1/roles",
      body: { name: " Neue Rolle ", color: "green" },
    });

    const real = { ...role("r-new", "Neue Rolle", 2), color: "green" };
    reply.resolve(json(real, 201));
    await done;
    expect(cachedDetail().roles).toEqual([moderation, regulars, real]);
    expect(listStale()).toBe(true);
  });

  it("drops the pending role when the server refuses", async () => {
    respond = failure;
    const observer = new MutationObserver(
      client,
      createRoleOptions(client, "s1"),
    );
    await observer.mutate({ name: "Neu", color: "amber" }).catch(() => null);
    expect(cachedDetail().roles).toEqual([moderation, regulars]);
  });
});

describe("useUpdateRole", () => {
  it("applies the change and recomputes the caller's own flags", async () => {
    const reply = held();
    respond = () => reply.promise;
    const observer = new MutationObserver(
      client,
      updateRoleOptions(client, me.id, "s1"),
    );
    const done = observer.mutate({
      id: "r-mod",
      patch: { permissions: ["moderate_members", "manage_messages"] },
    });

    await vi.waitFor(() =>
      expect(cachedDetail().roles[0]!.permissions).toContain("manage_messages"),
    );
    const mine = ["moderate_members", "manage_messages", "send_messages"];
    expect(cachedDetail().permissions).toEqual(mine);
    expect(cachedList()[0]!.permissions).toEqual(mine);
    expect(requests.at(-1)).toEqual({
      method: "PATCH",
      path: "/api/roles/r-mod",
      body: { permissions: ["moderate_members", "manage_messages"] },
    });

    const saved = {
      ...moderation,
      permissions: ["moderate_members", "manage_messages"],
    } satisfies ServerRole;
    reply.resolve(json(saved));
    await done;
    expect(cachedDetail().roles[0]).toEqual(saved);
    expect(listStale()).toBe(true);
  });

  it("rolls back when the server refuses", async () => {
    respond = failure;
    const observer = new MutationObserver(
      client,
      updateRoleOptions(client, me.id, "s1"),
    );
    await observer
      .mutate({ id: "r-mod", patch: { name: "Mods", color: "pink" } })
      .catch(() => null);
    expect(cachedDetail()).toEqual(detail);
    expect(cachedList()).toEqual([listItem]);
  });

  it("does not let a failed click undo a later one; the burst ends with a refetch", async () => {
    const first = held();
    const second = held();
    const replies = [first, second];
    respond = (url) =>
      url.pathname === "/api/servers/s1"
        ? json(detail)
        : replies.shift()!.promise;
    const a = new MutationObserver(
      client,
      updateRoleOptions(client, me.id, "s1"),
    );
    const b = new MutationObserver(
      client,
      updateRoleOptions(client, me.id, "s1"),
    );
    const doneA = a
      .mutate({ id: "r-reg", patch: { name: "Gäste" } })
      .catch(() => null);
    const doneB = b.mutate({ id: "r-reg", patch: { color: "violet" } });
    await vi.waitFor(() =>
      expect(cachedDetail().roles[1]).toMatchObject({
        name: "Gäste",
        color: "violet",
      }),
    );

    first.resolve(failure());
    await doneA;
    // No rollback to the snapshot from before both clicks.
    expect(cachedDetail().roles[1]).toMatchObject({ color: "violet" });

    second.resolve(json({ ...regulars, color: "violet" }));
    await doneB;
    expect(client.getQueryState(keys().detail)?.isInvalidated).toBe(true);
  });
});

describe("access changes during a burst", () => {
  const detailStale = () => client.getQueryState(keys().detail)?.isInvalidated;

  it("read the server again at once when no write is in flight", () => {
    const { userId, generation } = takeStamp()!;
    refetchOnAccessChange(client, userId, generation, "s1");
    expect(detailStale()).toBe(true);
    expect(listStale()).toBe(true);
  });

  it("wait for the caller's own role writes, so a refetch cannot land between two clicks", async () => {
    const reply = held();
    respond = (url) =>
      url.pathname === "/api/servers/s1" ? json(detail) : reply.promise;
    const observer = new MutationObserver(
      client,
      updateRoleOptions(client, me.id, "s1"),
    );
    const done = observer.mutate({ id: "r-reg", patch: { color: "violet" } });
    await vi.waitFor(() =>
      expect(cachedDetail().roles[1]).toMatchObject({ color: "violet" }),
    );

    // Our own PATCH announces {"k":"a"} while it is still in flight.
    const { userId, generation } = takeStamp()!;
    refetchOnAccessChange(client, userId, generation, "s1");
    expect(detailStale()).toBe(false);
    expect(listStale()).toBe(false);

    reply.resolve(json({ ...regulars, color: "violet" }));
    await done;
    expect(detailStale()).toBe(true);
  });
});

describe("useDeleteRole", () => {
  it("drops the role from members and channels; the channel stays limited", async () => {
    const reply = held();
    respond = () => reply.promise;
    const observer = new MutationObserver(
      client,
      deleteRoleOptions(client, me.id, "s1"),
    );
    const done = observer.mutate("r-mod");

    await vi.waitFor(() => expect(cachedDetail().roles).toEqual([regulars]));
    const after = cachedDetail();
    expect(after.members.find((m) => m.user_id === "jonas")!.role_ids).toEqual(
      [],
    );
    expect(after.channels[0]).toMatchObject({ limited: true, role_ids: [] });
    expect(after.permissions).toEqual(["send_messages"]);
    expect(requests.at(-1)).toMatchObject({
      method: "DELETE",
      path: "/api/roles/r-mod",
    });

    reply.resolve(new Response(null, { status: 204 }));
    await done;
    expect(listStale()).toBe(true);
  });

  it("restores everything when the server refuses", async () => {
    respond = failure;
    const observer = new MutationObserver(
      client,
      deleteRoleOptions(client, me.id, "s1"),
    );
    await observer.mutate("r-mod").catch(() => null);
    expect(cachedDetail()).toEqual(detail);
    expect(cachedList()).toEqual([listItem]);
  });
});

describe("useSetMemberRoles", () => {
  it("replaces the member's roles in position order and keeps the answer", async () => {
    const reply = held();
    respond = () => reply.promise;
    const observer = new MutationObserver(
      client,
      setMemberRolesOptions(client, me.id, "s1"),
    );
    const done = observer.mutate({
      userId: "mira",
      roleIds: ["r-reg", "r-mod"],
    });

    await vi.waitFor(() =>
      expect(
        cachedDetail().members.find((m) => m.user_id === "mira")!.role_ids,
      ).toEqual(["r-mod", "r-reg"]),
    );
    expect(requests.at(-1)).toEqual({
      method: "PUT",
      path: "/api/servers/s1/members/mira/roles",
      body: { role_ids: ["r-reg", "r-mod"] },
    });
    // Someone else's roles leave the caller's flags alone.
    expect(cachedDetail().permissions).toEqual(listItem.permissions);

    reply.resolve(json({ user_id: "mira", role_ids: ["r-mod", "r-reg"] }));
    await done;
    expect(
      cachedDetail().members.find((m) => m.user_id === "mira")!.role_ids,
    ).toEqual(["r-mod", "r-reg"]);
    expect(listStale()).toBe(true);
  });

  it("takes the caller's own rights away with their last role", async () => {
    const observer = new MutationObserver(
      client,
      setMemberRolesOptions(client, me.id, "s1"),
    );
    respond = () => json({ user_id: "jonas", role_ids: [] });
    await observer.mutate({ userId: "jonas", roleIds: [] });
    expect(cachedDetail().permissions).toEqual(["send_messages"]);
    expect(cachedList()[0]!.permissions).toEqual(["send_messages"]);
  });

  it("rolls back when the server refuses", async () => {
    respond = failure;
    const observer = new MutationObserver(
      client,
      setMemberRolesOptions(client, me.id, "s1"),
    );
    await observer
      .mutate({ userId: "mira", roleIds: ["r-reg"] })
      .catch(() => null);
    expect(cachedDetail()).toEqual(detail);
  });
});
