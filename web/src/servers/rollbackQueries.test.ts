// A failed kick, ban, leave, delete or edit undoes only its own optimistic
// change (GEL-54): whatever reached the cache while it was in flight (a
// later kick, a live join, a channel created elsewhere) stays.

import { MutationObserver, QueryClient } from "@tanstack/react-query";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { takeStamp } from "../auth/scope.ts";
import { login, resetSessionForTests } from "../auth/session.ts";
import { clearToasts, useToasts } from "../components/toasts.ts";
import { getServer } from "./api.ts";
import {
  applyMemberJoined,
  banMemberOptions,
  createCategoryOptions,
  createChannelOptions,
  createRoleOptions,
  createServerOptions,
  deleteCategoryOptions,
  deleteChannelOptions,
  deleteServerOptions,
  kickMemberOptions,
  leaveServerOptions,
  rememberJoinedServer,
  serverKeys,
  updateCategoryOptions,
  updateChannelOptions,
} from "./queries.ts";
import type {
  Category,
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
  locale: "de",
  avatar_url: null,
  created_at,
};

const member = (user_id: string): Member => ({
  user_id,
  name: user_id,
  avatar_url: null,
  joined_at: created_at,
  role: user_id === "lena" ? "owner" : "member",
  role_ids: [],
});
const category = (id: string, name = id): Category => ({
  id,
  server_id: "s1",
  name,
  created_at,
});
const channel = (id: string, category_id: string | null = null): Channel => ({
  id,
  server_id: "s1",
  category_id,
  name: id,
  kind: "text",
  created_at,
  limited: false,
  role_ids: [],
});
const server = (id: string, name = id): Server => ({
  id,
  name,
  owner_id: "lena",
  created_at,
  role: "member",
  permissions: ["moderate_members", "manage_channels", "send_messages"],
  member_permissions: ["send_messages"],
  blocked: false,
});
const listItem = server("s1", "Spieleabend");
const detail: ServerDetail = {
  ...listItem,
  categories: [category("talk", "Talk"), category("games", "Games")],
  channels: [channel("allgemein", "talk"), channel("spiele", "games")],
  members: [member("lena"), member("jonas"), member("mira"), member("tom")],
  roles: [],
};

const json = (body: unknown, status = 200) =>
  new Response(body === null ? null : JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
const failure = () => json({ error: "forbidden" }, 403);
const notFound = () => json({ error: "not_found" }, 404);

let client: QueryClient;
let respond: (url: URL, init: RequestInit) => Response | Promise<Response>;

function held() {
  let resolve!: (r: Response) => void;
  const promise = new Promise<Response>((r) => {
    resolve = r;
  });
  return { resolve, promise };
}

function stamp() {
  return takeStamp()!;
}
function keys() {
  const { userId, generation } = stamp();
  return {
    detail: serverKeys.detail(userId, generation, "s1"),
    list: serverKeys.list(userId, generation),
  };
}
const cachedDetail = () => client.getQueryData<ServerDetail>(keys().detail)!;
const cachedList = () => client.getQueryData<Server[]>(keys().list)!;
const editDetail = (update: (detail: ServerDetail) => ServerDetail) =>
  client.setQueryData<ServerDetail>(keys().detail, (current) =>
    update(current!),
  );
const ids = (rows: readonly { id: string }[]) => rows.map((row) => row.id);
const userIds = (rows: readonly Member[]) => rows.map((row) => row.user_id);

beforeEach(async () => {
  resetSessionForTests();
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
      return Promise.resolve(respond(url, init));
    }),
  );
  await login(me.email, "password");
  client.setQueryData(keys().detail, detail);
  client.setQueryData(keys().list, [listItem, server("s2")]);
});

afterEach(() => {
  client.clear();
  resetSessionForTests();
  vi.unstubAllGlobals();
});

/** Answers each request with the next held reply, in order. */
function holdEach(...replies: ReturnType<typeof held>[]) {
  respond = () => replies.shift()!.promise;
}

describe.each([
  ["kick", kickMemberOptions],
  ["ban", banMemberOptions],
] as const)("a failed %s", (_name, options) => {
  it("does not bring back a member a later one removed", async () => {
    const first = held();
    const second = held();
    holdEach(first, second);
    const a = new MutationObserver(client, options(client, "s1"));
    const b = new MutationObserver(client, options(client, "s1"));
    const doneA = a.mutate("mira").catch(() => null);
    const doneB = b.mutate("tom");
    await vi.waitFor(() =>
      expect(userIds(cachedDetail().members)).toEqual(["lena", "jonas"]),
    );

    second.resolve(json(null, 204));
    await doneB;
    first.resolve(failure());
    await doneA;

    expect(userIds(cachedDetail().members)).toEqual(["lena", "jonas", "mira"]);
  });

  it("keeps a member who joined while it was in flight", async () => {
    const reply = held();
    holdEach(reply);
    const observer = new MutationObserver(client, options(client, "s1"));
    const done = observer.mutate("mira").catch(() => null);
    await vi.waitFor(() =>
      expect(userIds(cachedDetail().members)).not.toContain("mira"),
    );

    const { userId, generation } = stamp();
    applyMemberJoined(client, userId, generation, "s1", member("paul"));
    reply.resolve(failure());
    await done;

    expect(userIds(cachedDetail().members)).toEqual([
      "lena",
      "jonas",
      "mira",
      "tom",
      "paul",
    ]);
  });

  it("does not bring back a member who had left already", async () => {
    respond = notFound;
    const observer = new MutationObserver(client, options(client, "s1"));
    await observer.mutate("mira").catch(() => null);

    expect(userIds(cachedDetail().members)).toEqual(["lena", "jonas", "tom"]);
  });
});

describe("a failed channel delete", () => {
  it("puts the channel back and keeps one created meanwhile", async () => {
    const reply = held();
    holdEach(reply);
    const observer = new MutationObserver(
      client,
      deleteChannelOptions(client, "s1"),
    );
    const done = observer.mutate("allgemein").catch(() => null);
    await vi.waitFor(() =>
      expect(ids(cachedDetail().channels)).toEqual(["spiele"]),
    );

    editDetail((d) => ({ ...d, channels: [...d.channels, channel("neu")] }));
    reply.resolve(failure());
    await done;

    expect(ids(cachedDetail().channels)).toEqual([
      "allgemein",
      "spiele",
      "neu",
    ]);
  });

  it("does not bring back a channel deleted elsewhere", async () => {
    respond = notFound;
    const observer = new MutationObserver(
      client,
      deleteChannelOptions(client, "s1"),
    );
    await observer.mutate("allgemein").catch(() => null);

    expect(ids(cachedDetail().channels)).toEqual(["spiele"]);
  });
});

describe("a failed channel edit", () => {
  it("reverts only its own fields and keeps other changes", async () => {
    const reply = held();
    holdEach(reply);
    const observer = new MutationObserver(
      client,
      updateChannelOptions(client, "s1"),
    );
    const done = observer
      .mutate({ id: "spiele", patch: { name: "Brettspiele" } })
      .catch(() => null);
    await vi.waitFor(() =>
      expect(cachedDetail().channels[1]!.name).toBe("brettspiele"),
    );

    // Moved elsewhere meanwhile, and a channel came in.
    editDetail((d) => ({
      ...d,
      channels: [
        ...d.channels.map((ch) =>
          ch.id === "spiele" ? { ...ch, category_id: "talk" } : ch,
        ),
        channel("neu"),
      ],
    }));
    reply.resolve(failure());
    await done;

    expect(cachedDetail().channels).toEqual([
      channel("allgemein", "talk"),
      channel("spiele", "talk"),
      channel("neu"),
    ]);
  });

  it("does not undo a later rename of the same channel", async () => {
    const first = held();
    const second = held();
    holdEach(first, second);
    const a = new MutationObserver(client, updateChannelOptions(client, "s1"));
    const b = new MutationObserver(client, updateChannelOptions(client, "s1"));
    const doneA = a
      .mutate({ id: "spiele", patch: { name: "eins" } })
      .catch(() => null);
    const doneB = b.mutate({ id: "spiele", patch: { name: "zwei" } });
    await vi.waitFor(() =>
      expect(cachedDetail().channels[1]!.name).toBe("zwei"),
    );

    first.resolve(failure());
    await doneA;
    expect(cachedDetail().channels[1]!.name).toBe("zwei");

    second.resolve(json({ ...channel("spiele", "games"), name: "zwei" }));
    await doneB;
    expect(cachedDetail().channels[1]!.name).toBe("zwei");
  });
});

describe("a failed category delete", () => {
  it("puts the category and its channels back and keeps newer rows", async () => {
    const reply = held();
    holdEach(reply);
    const observer = new MutationObserver(
      client,
      deleteCategoryOptions(client, "s1"),
    );
    const done = observer.mutate("talk").catch(() => null);
    await vi.waitFor(() =>
      expect(ids(cachedDetail().categories)).toEqual(["games"]),
    );

    editDetail((d) => ({
      ...d,
      categories: [...d.categories, category("neu")],
      channels: [...d.channels, channel("frisch", "neu")],
    }));
    reply.resolve(failure());
    await done;

    expect(ids(cachedDetail().categories)).toEqual(["talk", "games", "neu"]);
    expect(cachedDetail().channels).toEqual([
      channel("allgemein", "talk"),
      channel("spiele", "games"),
      channel("frisch", "neu"),
    ]);
  });

  it("leaves a channel moved elsewhere meanwhile where it is", async () => {
    const reply = held();
    holdEach(reply);
    const observer = new MutationObserver(
      client,
      deleteCategoryOptions(client, "s1"),
    );
    const done = observer.mutate("talk").catch(() => null);
    await vi.waitFor(() =>
      expect(ids(cachedDetail().categories)).toEqual(["games"]),
    );

    editDetail((d) => ({
      ...d,
      channels: d.channels.map((ch) =>
        ch.id === "allgemein" ? { ...ch, category_id: "games" } : ch,
      ),
    }));
    reply.resolve(failure());
    await done;

    expect(cachedDetail().channels[0]).toEqual(channel("allgemein", "games"));
  });

  it("does not bring back a category deleted elsewhere", async () => {
    respond = notFound;
    const observer = new MutationObserver(
      client,
      deleteCategoryOptions(client, "s1"),
    );
    await observer.mutate("talk").catch(() => null);

    expect(ids(cachedDetail().categories)).toEqual(["games"]);
    expect(cachedDetail().channels[0]).toEqual(channel("allgemein"));
  });
});

describe("a failed category rename", () => {
  it("reverts the name and keeps a category added meanwhile", async () => {
    const reply = held();
    holdEach(reply);
    const observer = new MutationObserver(
      client,
      updateCategoryOptions(client, "s1"),
    );
    const done = observer
      .mutate({ id: "talk", name: " Plaudern " })
      .catch(() => null);
    await vi.waitFor(() =>
      expect(cachedDetail().categories[0]!.name).toBe("Plaudern"),
    );

    editDetail((d) => ({
      ...d,
      categories: [...d.categories, category("neu")],
    }));
    reply.resolve(failure());
    await done;

    expect(cachedDetail().categories).toEqual([
      category("talk", "Talk"),
      category("games", "Games"),
      category("neu"),
    ]);
  });

  it("does not undo a later rename of the same category", async () => {
    const first = held();
    const second = held();
    holdEach(first, second);
    const a = new MutationObserver(client, updateCategoryOptions(client, "s1"));
    const b = new MutationObserver(client, updateCategoryOptions(client, "s1"));
    const doneA = a.mutate({ id: "talk", name: "Eins" }).catch(() => null);
    const doneB = b.mutate({ id: "talk", name: "Zwei" });
    await vi.waitFor(() =>
      expect(cachedDetail().categories[0]!.name).toBe("Zwei"),
    );

    first.resolve(failure());
    await doneA;
    expect(cachedDetail().categories[0]!.name).toBe("Zwei");

    second.resolve(json(category("talk", "Zwei")));
    await doneB;
  });
});

describe.each([
  ["delete", deleteServerOptions],
  ["leave", leaveServerOptions],
] as const)("a failed server %s", (_name, options) => {
  it("puts the server back and keeps one joined meanwhile", async () => {
    const reply = held();
    holdEach(reply);
    const observer = new MutationObserver(client, options(client));
    const done = observer.mutate("s1").catch(() => null);
    await vi.waitFor(() => expect(ids(cachedList())).toEqual(["s2"]));
    expect(client.getQueryData(keys().detail)).toBeUndefined();

    rememberJoinedServer(client, stamp(), server("s3"));
    reply.resolve(failure());
    await done;

    expect(ids(cachedList())).toEqual(["s1", "s2", "s3"]);
    expect(cachedDetail()).toEqual(detail);
  });

  it("does not overwrite a list row changed meanwhile", async () => {
    const reply = held();
    holdEach(reply);
    const observer = new MutationObserver(client, options(client));
    const done = observer.mutate("s1").catch(() => null);
    await vi.waitFor(() => expect(ids(cachedList())).toEqual(["s2"]));

    client.setQueryData<Server[]>(keys().list, (list) =>
      list!.map((s) => (s.id === "s2" ? { ...s, name: "Umbenannt" } : s)),
    );
    reply.resolve(failure());
    await done;

    expect(cachedList()).toEqual([listItem, server("s2", "Umbenannt")]);
  });

  it("reads the list again, which missed changes to the row", async () => {
    respond = failure;
    const observer = new MutationObserver(client, options(client));
    expect(client.getQueryState(keys().list)?.isInvalidated).toBe(false);
    await observer.mutate("s1").catch(() => null);

    expect(ids(cachedList())).toEqual(["s1", "s2"]);
    expect(client.getQueryState(keys().list)?.isInvalidated).toBe(true);
  });

  it("reads the list again only once no other removal is in flight", async () => {
    const first = held();
    const second = held();
    holdEach(first, second);
    const a = new MutationObserver(client, options(client));
    const b = new MutationObserver(client, deleteServerOptions(client));
    const doneA = a.mutate("s1").catch(() => null);
    const doneB = b.mutate("s2");
    await vi.waitFor(() => expect(cachedList()).toEqual([]));

    first.resolve(failure());
    await doneA;
    // A read now could answer before the other removal and bring s2 back.
    expect(ids(cachedList())).toEqual(["s1"]);
    expect(client.getQueryState(keys().list)?.isInvalidated).toBe(false);

    second.resolve(json(null, 204));
    await doneB;
    expect(client.getQueryState(keys().list)?.isInvalidated).toBe(true);
  });

  it("does not bring back a server that is gone", async () => {
    respond = notFound;
    const observer = new MutationObserver(client, options(client));
    await observer.mutate("s1").catch(() => null);

    expect(ids(cachedList())).toEqual(["s2"]);
    expect(client.getQueryData(keys().detail)).toBeUndefined();
    expect(client.getQueryState(keys().list)?.isInvalidated).toBe(false);
  });
});

describe("a create whose answer is rejected", () => {
  // The server may have created it: what the screen shows is read again, so
  // that a second click does not create a second one (GEL-420).
  const incomplete = () => json({ id: "new" }, 201);

  it("reads the server list again after a server create", async () => {
    vi.spyOn(console, "warn").mockImplementation(() => {});
    respond = incomplete;
    const create = new MutationObserver(client, createServerOptions(client));
    await expect(create.mutate("Neu")).rejects.toMatchObject({
      code: "malformed_answer",
    });
    expect(client.getQueryState(keys().list)?.isInvalidated).toBe(true);
  });

  it("reads the server again after a channel create and says it may exist", async () => {
    vi.spyOn(console, "warn").mockImplementation(() => {});
    clearToasts();
    respond = incomplete;
    const create = new MutationObserver(
      client,
      createChannelOptions(client, "s1"),
    );
    await expect(
      create.mutate({ name: "neu", kind: "text", category_id: null }),
    ).rejects.toMatchObject({ code: "malformed_answer" });
    expect(client.getQueryState(keys().detail)?.isInvalidated).toBe(true);
    expect(useToasts.getState().toasts.map((toast) => toast.message)).toEqual([
      "Die Antwort des Servers war unvollständig. Es kann trotzdem geklappt haben.",
    ]);
  });
});

const role = (id: string): ServerRole => ({
  id,
  server_id: "s1",
  name: id,
  color: "blue",
  permissions: [],
  position: 0,
  created_at,
});

describe.each([
  {
    kind: "channel",
    field: "channels",
    options: createChannelOptions,
    input: { name: "neu", kind: "text", category_id: null },
    row: (id: string) => channel(id),
  },
  {
    kind: "category",
    field: "categories",
    options: createCategoryOptions,
    input: "Neu",
    row: (id: string) => category(id),
  },
  {
    kind: "role",
    field: "roles",
    options: createRoleOptions,
    input: { name: "Neu", color: "blue" },
    row: role,
  },
] as const)(
  "a $kind created while the server is read again",
  ({ field, options, input, row }) => {
    // GEL-443: a channel without roles and a category reach their creator's
    // sidebar only through this create; no live event brings them. A read
    // of the server that left before the create was saved answers without
    // the new row.
    const rows = () => ids(cachedDetail()[field]);
    const holding = (...created: string[]) =>
      json({ ...detail, [field]: [...detail[field], ...created.map(row)] });
    const read = () =>
      client
        .fetchQuery({
          queryKey: keys().detail,
          queryFn: ({ signal }) => getServer("s1", signal),
        })
        .catch(() => null);
    const settled = () =>
      vi.waitFor(() =>
        expect(client.isFetching({ queryKey: keys().detail })).toBe(0),
      );
    let creates: ReturnType<typeof held>[];
    let reads: ReturnType<typeof held>[];
    beforeEach(() => {
      creates = [held(), held()];
      reads = [held(), held(), held()];
      const posts = [...creates];
      const gets = [...reads];
      respond = (_url, init) =>
        (init.method === "POST" ? posts : gets).shift()!.promise;
    });
    const start = () =>
      new MutationObserver(
        client,
        // One observer type for the three: each takes its own input.
        options(client, "s1") as never,
      ).mutate(input as never);
    const optimistic = (count: number) =>
      vi.waitFor(() =>
        expect(rows()).toHaveLength(detail[field].length + count),
      );

    it("keeps it when a read answers without it before the create does, then reads again", async () => {
      const before = rows();
      const [first, again] = reads as [
        ReturnType<typeof held>,
        ReturnType<typeof held>,
      ];
      const done = start();
      await optimistic(1);
      const reading = read();
      first.resolve(json(detail));
      await reading;
      expect(rows()).toEqual(before);

      creates[0]!.resolve(json(row("neu"), 201));
      await done;
      expect(rows()).toEqual([...before, "neu"]);
      // That read may have known it deleted already: the next read decides.
      again.resolve(json(detail));
      await settled();
      expect(rows()).toEqual(before);
    });

    it("keeps it when a read that left before it answers after it", async () => {
      const before = rows();
      const [stale, fresh] = reads as [
        ReturnType<typeof held>,
        ReturnType<typeof held>,
      ];
      const done = start();
      await optimistic(1);
      const reading = read();
      creates[0]!.resolve(json(row("neu"), 201));
      await done;
      expect(rows()).toEqual([...before, "neu"]);

      stale.resolve(json(detail));
      fresh.resolve(holding("neu"));
      await reading;
      await settled();
      expect(rows()).toEqual([...before, "neu"]);
    });

    it("shows it once when a read that holds it answers before the create does", async () => {
      const before = rows();
      const [first, again] = reads as [
        ReturnType<typeof held>,
        ReturnType<typeof held>,
      ];
      const done = start();
      await optimistic(1);
      const reading = read();
      first.resolve(holding("neu"));
      await reading;
      expect(rows()).toEqual([...before, "neu"]);

      creates[0]!.resolve(json(row("neu"), 201));
      await done;
      expect(rows()).toEqual([...before, "neu"]);
      again.resolve(holding("neu"));
      await settled();
      expect(rows()).toEqual([...before, "neu"]);
    });

    it("keeps both of two creates in flight when a read answers without them", async () => {
      const before = rows();
      const first = start();
      const second = start();
      await optimistic(2);
      const reading = read();
      reads[0]!.resolve(json(detail));
      await reading;
      expect(rows()).toEqual(before);

      creates[0]!.resolve(json(row("neu"), 201));
      creates[1]!.resolve(json(row("neu2"), 201));
      await Promise.all([first, second]);
      expect(rows()).toEqual([...before, "neu", "neu2"]);
      reads[1]!.resolve(holding("neu", "neu2"));
      reads[2]!.resolve(holding("neu", "neu2"));
      await settled();
      expect(rows()).toEqual([...before, "neu", "neu2"]);
    });

    it("adds nothing when the create fails after a read dropped its optimistic row", async () => {
      vi.spyOn(console, "warn").mockImplementation(() => {});
      clearToasts();
      const before = rows();
      const done = start().catch(() => null);
      await optimistic(1);
      const reading = read();
      reads[0]!.resolve(json(detail));
      await reading;

      creates[0]!.resolve(failure());
      await done;
      expect(rows()).toEqual(before);
      expect(client.isFetching({ queryKey: keys().detail })).toBe(0);
    });
  },
);
