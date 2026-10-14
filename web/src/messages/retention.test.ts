import {
  InfiniteQueryObserver,
  QueryClient,
  type InfiniteData,
} from "@tanstack/react-query";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { login, resetSessionForTests } from "../auth/session.ts";
import { takeStamp } from "../auth/scope.ts";
import { encodeCursor, flattenPages } from "./pages.ts";
import {
  ENTITY_LIMIT,
  applyChannelEvent,
  messageKeys,
  messageQueryOptions,
  retainedEntityCount,
} from "./queries.ts";
import { PAGE_SIZE } from "./rules.ts";
import type { Message, MessagePage } from "./types.ts";

const user = {
  id: "retention-user",
  name: "R",
  email: "r@example.test",
  avatar_url: null,
  created_at: "2026-09-28T00:00:00Z",
};
function row(channel: string, n: number): Message {
  return {
    id: `${channel}-${String(n).padStart(6, "0")}`,
    channel_id: channel,
    author: { id: "author", name: "Author", avatar_url: null },
    content: "x".repeat(1000),
    created_at: new Date(Date.UTC(2026, 8, 28, 9, 0, 0, n)).toISOString(),
    edited_at: null,
    attachments: [],
  };
}
const json = (body: unknown) =>
  new Response(JSON.stringify(body), {
    headers: { "Content-Type": "application/json" },
  });

let client: QueryClient;
/** What the server holds per channel, oldest first. */
let history: Map<string, Message[]>;
beforeEach(async () => {
  resetSessionForTests();
  client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: Infinity } },
  });
  history = new Map();
  vi.stubGlobal(
    "fetch",
    vi.fn((input: RequestInfo | URL) => {
      const url = new URL(String(input), "https://test.invalid");
      if (url.pathname.endsWith("/auth/login"))
        return Promise.resolve(json({ user, csrf_token: "csrf" }));
      const channel = url.pathname.split("/").at(-2)!;
      const before = url.searchParams.get("before");
      const eligible = (history.get(channel) ?? []).filter(
        (message) => !before || encodeCursor(message) < before,
      );
      return Promise.resolve(
        json({
          messages: eligible.slice(-PAGE_SIZE),
          has_more: eligible.length > PAGE_SIZE,
        }),
      );
    }),
  );
  await login(user.email, "password");
});
afterEach(() => {
  client.clear();
  resetSessionForTests();
  vi.unstubAllGlobals();
});

function options(channel: string) {
  const stamp = takeStamp()!;
  return messageQueryOptions(client, stamp.userId, stamp.generation, channel);
}
function pages(channel: string) {
  const stamp = takeStamp()!;
  return client.getQueryData<InfiniteData<MessagePage>>(
    messageKeys.channel(stamp.userId, stamp.generation, channel),
  )!.pages;
}
function event(t: "c" | "d", message: Message) {
  const stamp = takeStamp()!;
  applyChannelEvent(client, stamp.userId, stamp.generation, {
    t,
    c: message.channel_id,
    i: message.id,
    d: t === "c" ? message : undefined,
  });
}
/** A live create the server has committed before it is delivered. */
function create(channel: string, n: number) {
  const message = row(channel, n);
  history.set(channel, [...(history.get(channel) ?? []), message]);
  event("c", message);
}

describe("bounded live message retention", () => {
  it("retains no entities for creates in channels that are not open", () => {
    for (let n = 0; n < 2_500; n++) create(`idle${n % 20}`, n);
    expect(retainedEntityCount(client)).toBe(0);
  });

  it("keeps a delete for an unopened channel against an older REST page", async () => {
    for (let n = 0; n < 3; n++) create("idle", n);
    const [kept, deleted] = history.get("idle")!;
    event("d", deleted!);
    // A stale page still holding the deleted row must not revive it.
    await client.fetchInfiniteQuery(options("idle"));
    expect(flattenPages(pages("idle")).map((m) => m.id)).not.toContain(
      deleted!.id,
    );
    expect(flattenPages(pages("idle"))[0]?.id).toBe(kept!.id);
  });

  it("bounds tombstones and the floors of open channels", async () => {
    for (const channel of ["a", "b", "c"]) {
      await client.fetchInfiniteQuery(options(channel));
      for (let n = 0; n < 2_000; n++) create(channel, n);
    }
    for (let n = 0; n < 2_000; n++) event("d", row("idle", n));
    expect(retainedEntityCount(client)).toBeLessThanOrEqual(4 * ENTITY_LIMIT);
  });

  it("keeps only tombstones once a channel's query is removed", async () => {
    await client.fetchInfiniteQuery(options("gone"));
    for (let n = 0; n < 20; n++) create("gone", n);
    event("d", row("gone", 3));
    expect(retainedEntityCount(client)).toBe(20);
    client.removeQueries({ queryKey: options("gone").queryKey });
    expect(retainedEntityCount(client)).toBe(1);
  });
});

describe("the newest page stays one page", () => {
  it("drops the oldest rows off screen and pages them back in", async () => {
    for (let n = 0; n < 70; n++)
      history.set("bg", [...(history.get("bg") ?? []), row("bg", n)]);
    await client.fetchInfiniteQuery(options("bg"));
    const observer = new InfiniteQueryObserver(client, options("bg"));
    const off = observer.subscribe(() => undefined);
    await observer.fetchNextPage();
    off();
    expect(pages("bg")).toHaveLength(2);
    for (let n = 70; n < 400; n++) {
      create("bg", n);
      expect(pages("bg")[0]!.messages.length).toBeLessThanOrEqual(PAGE_SIZE);
    }
    // Older pages would split from the dropped rows, so they go as well.
    const [latest, ...older] = pages("bg");
    expect(older).toEqual([]);
    expect(latest).toMatchObject({
      has_more: true,
      older: encodeCursor(latest!.messages[0]!),
    });
    const reader = observer.subscribe(() => undefined);
    while (observer.getCurrentResult().hasNextPage)
      await observer.fetchNextPage();
    reader();
    observer.destroy();
    expect(flattenPages(pages("bg"))).toEqual(history.get("bg"));
  });

  it("moves the oldest rows one page down while shown with history loaded", async () => {
    for (let n = 0; n < 70; n++)
      history.set("on", [...(history.get("on") ?? []), row("on", n)]);
    const observer = new InfiniteQueryObserver(client, options("on"));
    const off = observer.subscribe(() => undefined);
    await vi.waitFor(() =>
      expect(observer.getCurrentResult().data).toBeDefined(),
    );
    await observer.fetchNextPage();
    for (let n = 70; n < 200; n++) create("on", n);
    off();
    observer.destroy();
    expect(pages("on")[0]!.messages).toHaveLength(PAGE_SIZE);
    expect(flattenPages(pages("on"))).toEqual(history.get("on"));
  });

  it("keeps the rows of a shown single page instead of dropping them", async () => {
    for (let n = 0; n < 70; n++)
      history.set("one", [...(history.get("one") ?? []), row("one", n)]);
    const observer = new InfiniteQueryObserver(client, options("one"));
    const off = observer.subscribe(() => undefined);
    await vi.waitFor(() =>
      expect(observer.getCurrentResult().data).toBeDefined(),
    );
    expect(pages("one")).toHaveLength(1);
    const shown = flattenPages(pages("one"))[0];
    for (let n = 70; n < 75; n++) create("one", n);
    // A reader scrolled up keeps seeing the same first row.
    expect(flattenPages(pages("one"))[0]).toEqual(shown);
    expect(pages("one")[0]!.messages).toHaveLength(PAGE_SIZE);
    while (observer.getCurrentResult().hasNextPage)
      await observer.fetchNextPage();
    off();
    observer.destroy();
    expect(flattenPages(pages("one"))).toEqual(history.get("one"));
  });
});
