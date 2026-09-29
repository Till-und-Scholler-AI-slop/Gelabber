import { QueryClient, type InfiniteData } from "@tanstack/react-query";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { login, resetSessionForTests } from "../auth/session.ts";
import { takeStamp } from "../auth/scope.ts";
import { applyChannelEvent, messageQueryOptions } from "./queries.ts";
import type { Message, MessagePage } from "./types.ts";

const user = {
  id: "revision-user",
  name: "R",
  email: "r@example.test",
  avatar_url: null,
  created_at: "2026-09-28T00:00:00Z",
};
const message = (
  id: string,
  revision: number,
  content = `revision ${revision}`,
): Message => ({
  id,
  revision,
  content,
  channel_id: "channel",
  author: { id: user.id, name: user.name, avatar_url: null },
  created_at: "2026-09-28T00:00:00Z",
  edited_at: null,
  attachments: [],
});
const json = (body: unknown) =>
  new Response(JSON.stringify(body), {
    headers: { "Content-Type": "application/json" },
  });
let client: QueryClient;
let response: (method: string) => Response | Promise<Response>;
function options() {
  const stamp = takeStamp()!;
  return {
    ...messageQueryOptions(client, stamp.userId, stamp.generation, "channel"),
    staleTime: 0,
  };
}
function event(t: "c" | "e" | "d", id: string, r: number, d?: Message) {
  const stamp = takeStamp()!;
  applyChannelEvent(client, stamp.userId, stamp.generation, {
    t,
    c: "channel",
    i: id,
    r,
    d,
  });
}
function cached() {
  return client
    .getQueryData<InfiniteData<MessagePage>>(options().queryKey)
    ?.pages.flatMap((p) => p.messages);
}
function held() {
  let resolve!: (r: Response) => void;
  const promise = new Promise<Response>((r) => {
    resolve = r;
  });
  return { resolve, promise };
}
beforeEach(async () => {
  resetSessionForTests();
  client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  response = () =>
    json({ messages: [message("one", 1), message("two", 2)], has_more: false });
  vi.stubGlobal(
    "fetch",
    vi.fn((input: RequestInfo | URL, init: RequestInit = {}) =>
      Promise.resolve(
        String(input).includes("/auth/login")
          ? json({ user, csrf_token: "csrf" })
          : response(init.method ?? "GET"),
      ),
    ),
  );
  await login(user.email, "password");
});
afterEach(() => {
  client.clear();
  resetSessionForTests();
  vi.unstubAllGlobals();
});

describe("durable entity revisions across REST lifetimes and transport epochs", () => {
  it("a delete seen before a later read remains deleted despite stale HTTP and replay", async () => {
    await client.fetchInfiniteQuery(options());
    event("d", "one", 10);
    await client.fetchInfiniteQuery(options());
    event("c", "one", 1, message("one", 1));
    event("e", "one", 9, message("one", 9));
    expect(cached()?.map((m) => m.id)).toEqual(["two"]);
  });
  it("keeps a delete floor after cancellation, cache removal, and reopening", async () => {
    await client.fetchInfiniteQuery(options());
    const late = held();
    response = () => late.promise;
    const read = client.fetchInfiniteQuery(options()).catch(() => undefined);
    await vi.waitFor(() => expect(client.isFetching()).toBe(1));
    event("d", "one", 11);
    await client.cancelQueries({ queryKey: options().queryKey });
    client.removeQueries({ queryKey: options().queryKey });
    late.resolve(json({ messages: [message("one", 1)], has_more: false }));
    await read;
    response = () =>
      json({
        messages: [message("one", 1), message("two", 2)],
        has_more: false,
      });
    await client.fetchInfiniteQuery(options());
    expect(cached()?.map((m) => m.id)).toEqual(["two"]);
  });
  it("higher DB revision wins with an older timestamp, and lower revisions cannot overwrite it", async () => {
    response = () =>
      json({
        messages: [
          { ...message("one", 20), edited_at: "2099-01-01T00:00:00Z" },
        ],
        has_more: false,
      });
    await client.fetchInfiniteQuery(options());
    event("e", "one", 21, {
      ...message("one", 21),
      edited_at: "2001-01-01T00:00:00Z",
    });
    event("e", "one", 19, {
      ...message("one", 19),
      edited_at: "2100-01-01T00:00:00Z",
    });
    await client.fetchInfiniteQuery(options());
    expect(cached()?.[0]).toMatchObject({
      revision: 21,
      content: "revision 21",
    });
  });
  it("held stale HTTP cannot replace newer edits or revive deleted rows", async () => {
    await client.fetchInfiniteQuery(options());
    const late = held();
    response = () => late.promise;
    const read = client.fetchInfiniteQuery(options());
    await vi.waitFor(() => expect(client.isFetching()).toBe(1));
    event("e", "one", 30, message("one", 30));
    event("d", "two", 31);
    late.resolve(
      json({
        messages: [message("one", 1), message("two", 2)],
        has_more: false,
      }),
    );
    await read;
    expect(cached()).toEqual([message("one", 30)]);
  });
  it("a new authoritative HTTP revision raises the floor against later stale events", async () => {
    await client.fetchInfiniteQuery(options());
    response = () => json({ messages: [message("one", 40)], has_more: false });
    await client.fetchInfiniteQuery(options());
    event("e", "one", 39, message("one", 39));
    event("c", "one", 1, message("one", 1));
    expect(cached()).toEqual([message("one", 40)]);
  });
});

it("HTTP edit reply with a higher revision still wins over an intervening older event", async () => {
  const { MutationObserver } = await import("@tanstack/react-query");
  const { editMessageOptions } = await import("./queries.ts");
  await client.fetchInfiniteQuery(options());
  const late = held();
  response = () => late.promise;
  const observer = new MutationObserver(
    client,
    editMessageOptions(client, "channel"),
  );
  const done = observer.mutate({ id: "one", content: "own edit" });
  await vi.waitFor(() => expect(cached()?.[0]?.content).toBe("own edit"));
  event("e", "one", 2, message("one", 2, "older event"));
  late.resolve(json(message("one", 3, "own canonical edit")));
  await done;
  expect(cached()?.[0]).toMatchObject({
    revision: 3,
    content: "own canonical edit",
  });
});

it("confirmed HTTP delete without a body tombstones a versioned row before WS delivery", async () => {
  const { MutationObserver } = await import("@tanstack/react-query");
  const { deleteMessageOptions } = await import("./queries.ts");
  await client.fetchInfiniteQuery(options());
  response = () => new Response(null, { status: 204 });
  const observer = new MutationObserver(
    client,
    deleteMessageOptions(client, "channel"),
  );
  await observer.mutate("one");
  response = () =>
    json({ messages: [message("one", 1), message("two", 2)], has_more: false });
  await client.fetchInfiniteQuery(options());
  expect(cached()?.map((m) => m.id)).toEqual(["two"]);
});

it.each([
  {
    restRevision: 2,
    restTime: "2099-01-01T00:00:00Z",
    patchTime: "2001-01-01T00:00:00Z",
    winner: 3,
  },
  {
    restRevision: 4,
    restTime: "2001-01-01T00:00:00Z",
    patchTime: "2099-01-01T00:00:00Z",
    winner: 4,
  },
])(
  "PATCH revision 3 against intervening REST revision $restRevision obeys DB order",
  async ({ restRevision, restTime, patchTime, winner }) => {
    const { MutationObserver } = await import("@tanstack/react-query");
    const { editMessageOptions } = await import("./queries.ts");
    await client.fetchInfiniteQuery(options());
    const late = held();
    const rest = { ...message("one", restRevision), edited_at: restTime };
    const patch = { ...message("one", 3), edited_at: patchTime };
    response = (method) =>
      method === "GET"
        ? json({ messages: [rest, message("two", 2)], has_more: false })
        : late.promise;
    const observer = new MutationObserver(
      client,
      editMessageOptions(client, "channel"),
    );
    const done = observer.mutate({ id: "one", content: "optimistic edit" });
    await vi.waitFor(() =>
      expect(cached()?.[0]?.content).toBe("optimistic edit"),
    );
    await client.fetchInfiniteQuery(options());
    expect(cached()?.[0]).toEqual(rest);
    late.resolve(json(patch));
    await done;
    const expected = winner === 3 ? patch : rest;
    expect(cached()?.[0]).toEqual(expected);
    // Successful PATCH/REST also retain their floor beyond this mutation/read.
    event("e", "one", 1, message("one", 1));
    response = () => json({ messages: [message("one", 1)], has_more: false });
    await client.fetchInfiniteQuery(options());
    expect(cached()?.[0]).toEqual(expected);
  },
);
