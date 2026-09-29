import {
  MutationObserver,
  QueryClient,
  type InfiniteData,
} from "@tanstack/react-query";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { login, resetSessionForTests } from "../auth/session.ts";
import { takeStamp } from "../auth/scope.ts";
import { releaseUserScope } from "../auth/release.ts";
import {
  resetPendingMessages,
  usePendingMessages,
  discardAttempt,
} from "./pending.ts";
import {
  applyChannelEvent,
  deleteMessageOptions,
  editMessageOptions,
  messageKeys,
  messageQueryOptions,
  sendMessageAttempt,
} from "./queries.ts";
import type { Message, MessagePage } from "./types.ts";

const user = {
  id: "a",
  name: "Ada",
  email: "a@example.test",
  avatar_url: null,
  created_at: "2026-09-28T00:00:00Z",
};
const author = { id: user.id, name: user.name, avatar_url: null };
const row = (id: string, content = id): Message => ({
  id,
  content,
  channel_id: "c",
  author,
  attachments: [],
  created_at: "2026-09-28T00:00:00Z",
  edited_at: null,
});
const json = (status: number, body: unknown) =>
  new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
function held() {
  let resolve!: (r: Response) => void;
  const promise = new Promise<Response>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}
let requests: string[];
function routes(
  handler: (key: string, init: RequestInit) => Response | Promise<Response>,
) {
  vi.stubGlobal(
    "fetch",
    vi.fn((url: RequestInfo | URL, init: RequestInit = {}) => {
      const key = `${init.method ?? "GET"} ${String(url)}`;
      requests.push(key);
      return Promise.resolve(
        key === "POST /api/auth/login"
          ? json(200, { user, csrf_token: "csrf" })
          : handler(key, init),
      );
    }),
  );
}
const attempts = () => Object.values(usePendingMessages.getState().attempts);
beforeEach(() => {
  resetSessionForTests();
  resetPendingMessages();
  requests = [];
});
afterEach(() => {
  resetSessionForTests();
  resetPendingMessages();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe("durable send attempts with controlled HTTP responses", () => {
  it("retains late failed A while parallel B succeeds without any observer", async () => {
    const a = held();
    routes((_key, init) =>
      JSON.parse(String(init.body)).content === "A"
        ? a.promise
        : json(200, row("server-b", "B")),
    );
    await login(user.email, "password");
    const first = sendMessageAttempt("c", author, { content: "A" }).catch(
      (error: unknown) => error,
    );
    await sendMessageAttempt("c", author, { content: "B" });
    a.resolve(json(429, { error: "rate_limited" }));
    await first;
    expect(attempts()).toMatchObject([
      { content: "A", status: "failed", stage: "bind" },
    ]);
    expect(
      usePendingMessages.getState().byChannel.c?.map((m) => m.content),
    ).toEqual(["A", "B"]);
    // Navigation does not own or remove the store; an exact retry reuses A's id.
    const failed = attempts()[0]!;
    routes(() => json(200, row("server-a", "A")));
    await sendMessageAttempt("c", author, {
      content: "ignored",
      attemptId: failed.id,
    });
    expect(attempts()).toEqual([]);
    expect(usePendingMessages.getState().byChannel.c?.map((m) => m.id)).toEqual(
      ["server-a", "server-b"],
    );
    expect(requests.filter((k) => k.includes("/messages"))).toHaveLength(3);
  });
  it("retries only bind after PUT succeeded, keeps the file and blocks double retry", async () => {
    const bind = held();
    let writes = 0;
    routes((key) => {
      if (key.endsWith("/attachments"))
        return json(200, {
          id: "attachment",
          upload_url: "https://objects.test/put",
          headers: {},
        });
      if (key.startsWith("PUT")) return new Response(null, { status: 200 });
      return ++writes === 1
        ? json(429, { error: "rate_limited" })
        : bind.promise;
    });
    await login(user.email, "password");
    const file = new File([new Uint8Array([1, 2, 3])], "test.png", {
      type: "image/png",
    });
    await sendMessageAttempt("c", author, { content: "image", file }).catch(
      () => undefined,
    );
    const failed = attempts()[0]!;
    expect(failed).toMatchObject({
      file,
      uploadedId: "attachment",
      stage: "bind",
      status: "failed",
    });
    const retry = sendMessageAttempt("c", author, {
      content: "",
      attemptId: failed.id,
    });
    await expect(
      sendMessageAttempt("c", author, { content: "", attemptId: failed.id }),
    ).rejects.toThrow("läuft bereits");
    bind.resolve(
      json(200, {
        ...row("server"),
        attachments: [
          {
            id: "attachment",
            filename: "test.png",
            content_type: "image/png",
            size: 3,
          },
        ],
      }),
    );
    await retry;
    expect(requests.filter((k) => k.endsWith("/attachments"))).toHaveLength(1);
    expect(requests.filter((k) => k.startsWith("PUT"))).toHaveLength(1);
    expect(writes).toBe(2);
  });
  it.each(["presign", "upload"])(
    "stops after account release during held %s",
    async (stage) => {
      const response = held();
      routes((key) =>
        key.endsWith("/attachments")
          ? stage === "presign"
            ? response.promise
            : json(200, {
                id: "file",
                upload_url: "https://objects.test/put",
                headers: {},
              })
          : response.promise,
      );
      await login(user.email, "password");
      const done = sendMessageAttempt("c", author, {
        content: "file",
        file: new File(["a"], "a.png", { type: "image/png" }),
      }).catch((error: unknown) => error);
      await vi.waitFor(() => expect(attempts()[0]?.stage).toBe(stage));
      const controller = attempts()[0]!.controller;
      releaseUserScope(null);
      expect(controller.signal.aborted).toBe(true);
      response.resolve(
        stage === "presign"
          ? json(200, {
              id: "file",
              upload_url: "https://objects.test/put",
              headers: {},
            })
          : new Response(null, { status: 200 }),
      );
      expect(await done).toMatchObject({ name: "AbortError" });
      expect(requests.some((k) => k.endsWith("/messages"))).toBe(false);
      expect(attempts()).toEqual([]);
    },
  );
  it("does not repeat a bind with unknown network outcome", async () => {
    routes(() => Promise.reject(new TypeError("network")));
    await login(user.email, "password");
    await sendMessageAttempt("c", author, { content: "maybe saved" }).catch(
      () => undefined,
    );
    const attempt = attempts()[0]!;
    expect(attempt.status).toBe("uncertain");
    await expect(
      sendMessageAttempt("c", author, { content: "", attemptId: attempt.id }),
    ).rejects.toThrow("Speicherstatus unbekannt");
    expect(requests.filter((k) => k.endsWith("/messages"))).toHaveLength(1);
    discardAttempt(attempt.id);
    expect(attempts()).toEqual([]);
  });
});

it.each([408, 500, 502, 503, 504])(
  "does not repeat bind after ambiguous HTTP %s",
  async (status) => {
    routes(() => json(status, { error: "upstream_failure" }));
    await login(user.email, "password");
    await sendMessageAttempt("c", author, {
      content: "possibly committed",
    }).catch(() => undefined);
    const attempt = attempts()[0]!;
    expect(attempt.status).toBe("uncertain");
    await expect(
      sendMessageAttempt("c", author, { content: "", attemptId: attempt.id }),
    ).rejects.toThrow("Speicherstatus unbekannt");
    expect(requests.filter((key) => key.endsWith("/messages"))).toHaveLength(1);
  },
);

describe("per-row rollback using real mutation observers", () => {
  let client: QueryClient;
  beforeEach(async () => {
    routes(() => json(200, null));
    await login(user.email, "password");
    client = new QueryClient({
      defaultOptions: { mutations: { retry: false } },
    });
  });
  afterEach(() => client.clear());
  function key() {
    const stamp = takeStamp()!;
    return messageKeys.channel(stamp.userId, stamp.generation, "c");
  }
  function cached() {
    return client
      .getQueryData<InfiniteData<MessagePage>>(key())!
      .pages.flatMap((p) => p.messages);
  }
  function seed() {
    client.setQueryData(key(), {
      pages: [
        {
          messages: [row("one"), row("two")],
          has_more: true,
          older: "boundary",
        },
      ],
      pageParams: [undefined],
    });
  }
  function event(id: string, t: "e" | "d" = "e") {
    const stamp = takeStamp()!;
    applyChannelEvent(client, stamp.userId, stamp.generation, {
      t,
      c: "c",
      i: id,
      d: { ...row(id, "new event"), edited_at: "2099-01-01T00:00:00Z" },
    });
  }
  it("late edit failure rolls back one row and preserves another row's event", async () => {
    seed();
    const response = held();
    routes(() => response.promise);
    const mutation = new MutationObserver(
      client,
      editMessageOptions(client, "c"),
    );
    const done = mutation
      .mutate({ id: "one", content: "optimistic" })
      .catch(() => undefined);
    await vi.waitFor(() => expect(cached()[0]?.content).toBe("optimistic"));
    event("two");
    response.resolve(json(503, { error: "unavailable" }));
    await done;
    expect(cached().map((r) => r.content)).toEqual(["one", "new event"]);
  });
  it("does not undo a newer event on the same edited row", async () => {
    seed();
    const response = held();
    routes(() => response.promise);
    const mutation = new MutationObserver(
      client,
      editMessageOptions(client, "c"),
    );
    const done = mutation
      .mutate({ id: "one", content: "optimistic" })
      .catch(() => undefined);
    await vi.waitFor(() => expect(cached()[0]?.content).toBe("optimistic"));
    event("one");
    response.resolve(json(503, { error: "unavailable" }));
    await done;
    expect(cached()[0]?.content).toBe("new event");
  });
  it("late delete failure restores only its row and keeps the page boundary", async () => {
    seed();
    const response = held();
    routes(() => response.promise);
    const mutation = new MutationObserver(
      client,
      deleteMessageOptions(client, "c"),
    );
    const done = mutation.mutate("one").catch(() => undefined);
    await vi.waitFor(() => expect(cached()).toHaveLength(1));
    event("two");
    response.resolve(json(503, { error: "unavailable" }));
    await done;
    expect(cached().map((r) => r.content)).toEqual(["one", "new event"]);
    expect(
      client.getQueryData<InfiniteData<MessagePage>>(key())?.pages[0]?.older,
    ).toBe("boundary");
  });
  it("does not resurrect a newer authoritative delete", async () => {
    seed();
    const response = held();
    routes(() => response.promise);
    const mutation = new MutationObserver(
      client,
      deleteMessageOptions(client, "c"),
    );
    const done = mutation.mutate("one").catch(() => undefined);
    await vi.waitFor(() => expect(cached()).toHaveLength(1));
    event("one", "d");
    response.resolve(json(503, { error: "unavailable" }));
    await done;
    expect(cached().map((r) => r.id)).toEqual(["two"]);
  });
  it("reviewer successful PATCH must replace an older REST snapshot", async () => {
    seed();
    const response = held();
    routes((key) =>
      key.startsWith("GET ")
        ? json(200, {
            messages: [row("one", "one"), row("two")],
            has_more: false,
          })
        : response.promise,
    );
    const mutation = new MutationObserver(
      client,
      editMessageOptions(client, "c"),
    );
    const done = mutation.mutate({ id: "one", content: "earlier edit" });
    await vi.waitFor(() => expect(cached()[0]?.content).toBe("earlier edit"));
    const stamp = takeStamp()!;
    await client.invalidateQueries({
      queryKey: key(),
      exact: true,
      refetchType: "none",
    });
    await client.fetchInfiniteQuery(
      messageQueryOptions(client, stamp.userId, stamp.generation, "c"),
    );
    expect(cached()[0]?.content).toBe("one");
    response.resolve(
      json(200, {
        ...row("one", "earlier edit"),
        edited_at: "2026-09-28T00:00:01Z",
      }),
    );
    await done;
    expect(cached()[0]?.content).toBe("earlier edit");
  });
  it("settles its own optimistic clock with the canonical server edit", async () => {
    seed();
    const response = held();
    routes(() => response.promise);
    const mutation = new MutationObserver(
      client,
      editMessageOptions(client, "c"),
    );
    const done = mutation.mutate({ id: "one", content: "own edit" });
    await vi.waitFor(() => expect(cached()[0]?.content).toBe("own edit"));
    const canonical = {
      ...row("one", "canonical edit"),
      edited_at: "2026-09-28T00:00:01Z",
    };
    response.resolve(json(200, canonical));
    await done;
    expect(cached()[0]).toEqual(canonical);
  });
  it("reviewer late PATCH response cannot replace a newer REST row", async () => {
    seed();
    const response = held();
    routes((key) =>
      key.startsWith("GET ")
        ? json(200, {
            messages: [
              {
                ...row("one", "newer REST"),
                edited_at: "2099-01-01T00:00:00Z",
              },
              row("two"),
            ],
            has_more: false,
          })
        : response.promise,
    );
    const mutation = new MutationObserver(
      client,
      editMessageOptions(client, "c"),
    );
    const done = mutation.mutate({ id: "one", content: "earlier edit" });
    await vi.waitFor(() => expect(cached()[0]?.content).toBe("earlier edit"));
    const stamp = takeStamp()!;
    await client.invalidateQueries({
      queryKey: key(),
      exact: true,
      refetchType: "none",
    });
    await client.fetchInfiniteQuery(
      messageQueryOptions(client, stamp.userId, stamp.generation, "c"),
    );
    expect(cached()[0]?.content).toBe("newer REST");
    response.resolve(
      json(200, {
        ...row("one", "earlier edit"),
        edited_at: "2026-09-28T00:00:01Z",
      }),
    );
    await done;
    expect(cached()[0]?.content).toBe("newer REST");
  });
});
