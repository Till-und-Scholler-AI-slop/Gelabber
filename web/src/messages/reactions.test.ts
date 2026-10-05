import {
  MutationObserver,
  QueryClient,
  type InfiniteData,
} from "@tanstack/react-query";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { login, resetSessionForTests } from "../auth/session.ts";
import { takeStamp } from "../auth/scope.ts";
import { applyChannelEvent, messageQueryOptions } from "./queries.ts";
import { displayedReactions, reactionMutationOptions } from "./reactions.ts";
import type { Message, MessagePage } from "./types.ts";

const user = {
  id: "me",
  name: "Ada",
  email: "ada@test.invalid",
  avatar_url: null,
  created_at: "2026-10-05T00:00:00Z",
};
const row = (
  revision: number,
  content = "text",
  reactions: Message["reactions"] = [],
): Message => ({
  id: "one",
  channel_id: "chat",
  revision,
  content,
  reactions,
  author: { id: "author", name: "Bob", avatar_url: null },
  created_at: "2026-10-05T00:00:00Z",
  edited_at: null,
  attachments: [],
});
const json = (body: unknown) =>
  new Response(JSON.stringify(body), {
    headers: { "Content-Type": "application/json" },
  });
let client: QueryClient;
let authUser = user;
let respond: (method: string) => Response | Promise<Response>;
function options() {
  const s = takeStamp()!;
  return {
    ...messageQueryOptions(client, s.userId, s.generation, "chat"),
    staleTime: 0,
  };
}
function cached() {
  return client
    .getQueryData<InfiniteData<MessagePage>>(options().queryKey)
    ?.pages.flatMap((p) => p.messages);
}
function event(type: "e" | "d", revision: number, message?: Message) {
  const s = takeStamp()!;
  applyChannelEvent(client, s.userId, s.generation, {
    t: type,
    c: "chat",
    i: "one",
    r: revision,
    d: message,
  });
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
  authUser = user;
  client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  respond = () => json({ messages: [row(1)], has_more: false });
  vi.stubGlobal(
    "fetch",
    vi.fn((input: RequestInfo | URL, init: RequestInit = {}) =>
      Promise.resolve(
        String(input).includes("/auth/login")
          ? json({ user: authUser, csrf_token: "csrf" })
          : respond(init.method ?? "GET"),
      ),
    ),
  );
  await login(user.email, "password");
  await client.fetchInfiniteQuery(options());
});
afterEach(() => {
  client.clear();
  resetSessionForTests();
  vi.unstubAllGlobals();
});

describe("separate reaction intent overlay", () => {
  it("changes only own vote, retains concurrent foreign votes, and never mutates canonical data", () => {
    const canonical = [
      { emoji: "❤️", user_ids: ["other"] },
      { emoji: "👍", user_ids: ["me", "other"] },
    ];
    const original = structuredClone(canonical);
    const s = takeStamp()!;
    expect(
      displayedReactions(canonical, "me", { emoji: "❤️", add: true, scope: s }),
    ).toEqual([
      { emoji: "❤️", user_ids: ["other", "me"] },
      { emoji: "👍", user_ids: ["me", "other"] },
    ]);
    expect(
      displayedReactions(canonical, "me", {
        emoji: "👍",
        add: false,
        scope: s,
      }),
    ).toEqual([
      { emoji: "❤️", user_ids: ["other"] },
      { emoji: "👍", user_ids: ["other"] },
    ]);
    expect(canonical).toEqual(original);
    expect(
      displayedReactions(canonical, "new-account", {
        emoji: "❤️",
        add: true,
        scope: s,
      }),
    ).toEqual(canonical);
  });
  it("a failed click cannot roll back an intervening edit or another user's reaction", async () => {
    const late = held();
    respond = (method) =>
      method === "PUT"
        ? late.promise
        : json({
            messages: [
              row(5, "new text", [{ emoji: "👍", user_ids: ["other"] }]),
            ],
            has_more: false,
          });
    const observer = new MutationObserver(
      client,
      reactionMutationOptions(client, "chat", "one"),
    );
    const pending = observer
      .mutate({ emoji: "❤️", add: true, scope: takeStamp()! })
      .catch(() => undefined);
    await vi.waitFor(() =>
      expect(observer.getCurrentResult().isPending).toBe(true),
    );
    event("e", 5, row(5, "new text", [{ emoji: "👍", user_ids: ["other"] }]));
    late.resolve(
      new Response(JSON.stringify({ error: "forbidden" }), {
        status: 403,
        headers: { "Content-Type": "application/json" },
      }),
    );
    await pending;
    expect(cached()?.[0]).toEqual(
      row(5, "new text", [{ emoji: "👍", user_ids: ["other"] }]),
    );
  });
  it("a delayed lower revision reply loses to edit and does not resurrect a delete", async () => {
    const late = held();
    respond = () => late.promise;
    const observer = new MutationObserver(
      client,
      reactionMutationOptions(client, "chat", "one"),
    );
    const pending = observer.mutate({
      emoji: "❤️",
      add: true,
      scope: takeStamp()!,
    });
    await vi.waitFor(() =>
      expect(observer.getCurrentResult().isPending).toBe(true),
    );
    event(
      "e",
      8,
      row(8, "later edit", [{ emoji: "❤️", user_ids: ["me", "other"] }]),
    );
    event("d", 9);
    late.resolve(json(row(7, "old text", [{ emoji: "❤️", user_ids: ["me"] }])));
    await pending;
    expect(cached()).toEqual([]);
  });
  it("the next authoritative read carries reactions after reconnect", async () => {
    respond = () =>
      json({
        messages: [row(15, "text", [{ emoji: "👩‍💻", user_ids: ["other"] }])],
        has_more: false,
      });
    await client.fetchInfiniteQuery(options());
    event("e", 14, row(14));
    expect(cached()?.[0]?.reactions).toEqual([
      { emoji: "👩‍💻", user_ids: ["other"] },
    ]);
  });
  it("old account replies cannot write into a new account cache", async () => {
    const late = held();
    respond = () => late.promise;
    const old = takeStamp()!;
    const observer = new MutationObserver(
      client,
      reactionMutationOptions(client, "chat", "one"),
    );
    const pending = observer
      .mutate({ emoji: "❤️", add: true, scope: old })
      .catch(() => undefined);
    await vi.waitFor(() =>
      expect(observer.getCurrentResult().isPending).toBe(true),
    );
    const oldKey = messageQueryOptions(
      client,
      old.userId,
      old.generation,
      "chat",
    ).queryKey;
    authUser = { ...user, id: "new-user", email: "other@test.invalid" };
    await login(authUser.email, "password");
    client.setQueryData(options().queryKey, {
      pages: [{ messages: [row(1, "new user text")], has_more: false }],
      pageParams: [null],
    });
    late.resolve(
      json(row(20, "wrong account", [{ emoji: "❤️", user_ids: ["me"] }])),
    );
    await pending;
    expect(
      client.getQueryData<InfiniteData<MessagePage>>(oldKey)?.pages[0]
        ?.messages[0]?.revision,
    ).toBe(1);
    expect(cached()?.[0]?.content).toBe("new user text");
  });
  it("rejects temporary and stale identities before making HTTP requests", async () => {
    const s = takeStamp()!;
    const fetch = vi.mocked(globalThis.fetch);
    fetch.mockClear();
    await expect(
      reactionMutationOptions(client, "chat", "tmp:pending").mutationFn({
        emoji: "😀",
        add: true,
        scope: s,
      }),
    ).rejects.toHaveProperty("name", "AbortError");
    await expect(
      reactionMutationOptions(client, "chat", "one").mutationFn({
        emoji: "😀",
        add: true,
        scope: { ...s, generation: s.generation + 1 },
      }),
    ).rejects.toHaveProperty("name", "AbortError");
    expect(fetch).not.toHaveBeenCalled();
  });
});
