import {
  QueryClient,
  InfiniteQueryObserver,
  type InfiniteData,
} from "@tanstack/react-query";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { api } from "../api/client.ts";
import { takeStamp } from "../auth/scope.ts";
import { login, logout, resetSessionForTests } from "../auth/session.ts";
import {
  applyChannelEvent,
  messageKeys,
  messageQueryOptions,
} from "../messages/queries.ts";
import { encodeCursor, flattenPages } from "../messages/pages.ts";
import type { Message, MessagePage } from "../messages/types.ts";
import { Gateway, type SocketLike } from "./client.ts";
import { READY_SPREAD_MS, attachRealtimeRecovery } from "./useRealtime.ts";

class Socket implements SocketLike {
  sent: string[] = [];
  listeners = new Map<string, Set<(event: { data?: string }) => void>>();
  send(data: string) {
    this.sent.push(data);
  }
  close() {
    this.emit("close");
  }
  addEventListener(type: string, handler: (event: { data?: string }) => void) {
    const listeners = this.listeners.get(type) ?? new Set();
    listeners.add(handler);
    this.listeners.set(type, listeners);
  }
  removeEventListener(
    type: string,
    handler: (event: { data?: string }) => void,
  ) {
    this.listeners.get(type)?.delete(handler);
  }
  emit(type: string, data?: string) {
    for (const handler of this.listeners.get(type) ?? []) handler({ data });
  }
  frame(data: unknown) {
    this.emit("message", JSON.stringify(data));
  }
}
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}
function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}
function rows(count: number, start = 1): Message[] {
  return Array.from({ length: count }, (_, index) => ({
    id: `m${String(start + index).padStart(3, "0")}`,
    channel_id: "channel",
    author: { id: "author", name: "Author", avatar_url: null },
    content: `message ${start + index}`,
    created_at: new Date(
      Date.UTC(2026, 8, 28, 9, 0, start + index),
    ).toISOString(),
    edited_at: null,
    attachments: [],
  }));
}
const ada = {
  id: "user-a",
  email: "a@example.com",
  name: "A",
  avatar_url: null,
  created_at: "2026-09-28T09:00:00Z",
};
const clients: QueryClient[] = [];
const gateways: Gateway[] = [];
const cleanups: (() => void)[] = [];
function client() {
  const result = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: Infinity } },
  });
  clients.push(result);
  return result;
}
function gateway() {
  const sockets: Socket[] = [];
  const result = new Gateway({
    open: () => {
      const socket = new Socket();
      sockets.push(socket);
      return socket;
    },
  });
  gateways.push(result);
  result.start();
  sockets[0]!.emit("open");
  return { gateway: result, sockets, socket: sockets[0]! };
}
function install(
  handler: (url: URL, init: RequestInit) => Response | Promise<Response>,
) {
  const calls: string[] = [];
  vi.stubGlobal(
    "fetch",
    vi.fn((input: RequestInfo | URL, init: RequestInit = {}) => {
      const url = new URL(String(input), "https://test.invalid");
      calls.push(url.pathname);
      if (url.pathname === "/api/auth/login")
        return Promise.resolve(json({ user: ada, csrf_token: "csrf-a" }));
      if (url.pathname === "/api/auth/logout")
        return Promise.resolve(json({ csrf_token: "csrf-out" }));
      return Promise.resolve(handler(url, init));
    }),
  );
  return calls;
}
function options(queryClient: QueryClient) {
  const stamp = takeStamp()!;
  return {
    ...messageQueryOptions(
      queryClient,
      stamp.userId,
      stamp.generation,
      "channel",
    ),
    retry: false as const,
  };
}
/** Load the history and keep it on screen, as an open channel does. */
async function observe(queryClient: QueryClient) {
  await queryClient.fetchInfiniteQuery(options(queryClient));
  const observer = new InfiniteQueryObserver(queryClient, options(queryClient));
  const off = observer.subscribe(() => undefined);
  cleanups.push(() => {
    off();
    observer.destroy();
  });
}
function cache(queryClient: QueryClient) {
  const stamp = takeStamp()!;
  return queryClient.getQueryData<InfiniteData<MessagePage>>(
    messageKeys.channel(stamp.userId, stamp.generation, "channel"),
  )!;
}
beforeEach(() => resetSessionForTests());
afterEach(() => {
  for (const cleanup of cleanups.splice(0)) cleanup();
  for (const gateway of gateways.splice(0)) gateway.stop();
  for (const queryClient of clients.splice(0)) queryClient.clear();
  resetSessionForTests();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("gap recovery on the existing gateway protocol", () => {
  it("recovers missed create/edit/delete and workspace lists, preserving live changes and delaying the cursor", async () => {
    const queryClient = client();
    const { gateway: ws, socket } = gateway();
    let history = rows(100);
    let changed = false;
    let hold = false;
    const newestResponse = deferred<Response>();
    const newestStarted = deferred<MessagePage>();
    install((url) => {
      if (url.pathname === "/api/servers")
        return json(
          changed ? [{ id: "new-membership" }] : [{ id: "removed-membership" }],
        );
      if (url.pathname === "/api/servers/server")
        return json({
          id: "server",
          channels: changed
            ? [{ id: "renamed-channel", name: "new" }]
            : [{ id: "deleted-channel", name: "old" }],
        });
      if (url.pathname === "/api/dms")
        return json(changed ? [{ id: "new-dm" }] : [{ id: "old-dm" }]);
      const before = url.searchParams.get("before");
      const eligible = history.filter(
        (row) => !before || encodeCursor(row) < before,
      );
      const page = {
        messages: eligible.slice(-50),
        has_more: eligible.length > 50,
      };
      if (hold && !before) {
        newestStarted.resolve(page);
        return newestResponse.promise;
      }
      return json(page);
    });
    await login(ada.email, "password123");
    const stamp = takeStamp()!;
    await queryClient.fetchInfiniteQuery(options(queryClient));
    const observer = new InfiniteQueryObserver(
      queryClient,
      options(queryClient),
    );
    const off = observer.subscribe(() => undefined);
    cleanups.push(() => {
      off();
      observer.destroy();
    });
    await observer.fetchNextPage();
    const serverKey = [
      "user",
      stamp.userId,
      stamp.generation,
      "servers",
      "list",
    ];
    const detailKey = [
      "user",
      stamp.userId,
      stamp.generation,
      "servers",
      "detail",
      "server",
    ];
    const dmKey = ["user", stamp.userId, stamp.generation, "dms", "list"];
    await Promise.all([
      queryClient.fetchQuery({
        queryKey: serverKey,
        queryFn: () => api("/servers"),
      }),
      queryClient.fetchQuery({
        queryKey: detailKey,
        queryFn: () => api("/servers/server"),
      }),
      queryClient.fetchQuery({ queryKey: dmKey, queryFn: () => api("/dms") }),
    ]);
    cleanups.push(
      attachRealtimeRecovery(queryClient, stamp, ws, { readySpreadMs: 0 }),
    );
    cleanups.push(
      ws.onEvent((event) =>
        applyChannelEvent(queryClient, stamp.userId, stamp.generation, event),
      ),
    );
    socket.frame({ op: "ok", s: "server", c: "channel", n: 100 });
    const edited = {
      ...history[18]!,
      content: "missed offline edit",
      edited_at: "2026-09-28T09:03:00Z",
    };
    history = [
      ...history
        .filter((row) => row.id !== "m030")
        .map((row) => (row.id === edited.id ? edited : row)),
      ...rows(1, 101),
    ];
    changed = true;
    hold = true;
    socket.frame({ op: "gap", s: "server", c: "channel" });
    socket.frame({ op: "ok", s: "server", c: "channel", n: 150 });
    const snapshot = await newestStarted.promise;
    expect(ws.cursorsSnapshot.get("c:channel")).toBe(100);
    const live = rows(1, 102)[0]!;
    socket.frame({
      op: "e",
      t: "c",
      s: "server",
      c: "channel",
      n: 151,
      i: live.id,
      d: live,
    });
    expect(flattenPages(cache(queryClient).pages)).toHaveLength(101); // Old visible history remains until recovery succeeds.
    newestResponse.resolve(json(snapshot));
    await vi.waitFor(() => expect(ws.gapRecoveries()).toEqual([]));
    const messages = flattenPages(cache(queryClient).pages);
    expect(messages).toHaveLength(101);
    expect(messages.find((row) => row.id === edited.id)).toEqual(edited);
    expect(messages.some((row) => row.id === "m030")).toBe(false);
    expect(messages.at(-1)).toEqual(live);
    expect(ws.cursorsSnapshot.get("c:channel")).toBe(151);
    expect(queryClient.getQueryData(serverKey)).toEqual([
      { id: "new-membership" },
    ]);
    expect(queryClient.getQueryData(detailKey)).toEqual({
      id: "server",
      channels: [{ id: "renamed-channel", name: "new" }],
    });
    expect(queryClient.getQueryData(dmKey)).toEqual([{ id: "new-dm" }]);
  });

  it("replaces a pending initial GET from before the gap and discards its late body", async () => {
    const queryClient = client();
    const { gateway: ws, socket } = gateway();
    const staleResponse = deferred<Response>();
    let reads = 0;
    let oldSignal: AbortSignal | undefined;
    install((_url, init) => {
      if (++reads === 1) {
        oldSignal = init.signal ?? undefined;
        return staleResponse.promise;
      }
      return json({ messages: rows(1, 2), has_more: false });
    });
    await login(ada.email, "password123");
    const stamp = takeStamp()!;
    const oldRead = queryClient
      .fetchInfiniteQuery(options(queryClient))
      .catch((error: unknown) => error);
    const observer = new InfiniteQueryObserver(
      queryClient,
      options(queryClient),
    );
    const off = observer.subscribe(() => undefined);
    cleanups.push(() => {
      off();
      observer.destroy();
    });
    cleanups.push(
      attachRealtimeRecovery(queryClient, stamp, ws, { readySpreadMs: 0 }),
    );
    socket.frame({ op: "gap", s: "server", c: "channel" });
    socket.frame({ op: "ok", s: "server", c: "channel", n: 5 });
    await vi.waitFor(() => expect(ws.gapRecoveries()).toEqual([]));
    expect(oldSignal?.aborted).toBe(true);
    expect(reads).toBe(2);
    expect(flattenPages(cache(queryClient).pages)).toEqual(rows(1, 2));
    staleResponse.resolve(json({ messages: rows(1), has_more: false }));
    await oldRead;
    await Promise.resolve();
    expect(flattenPages(cache(queryClient).pages)).toEqual(rows(1, 2));
  });

  it("keeps failed recovery stale and the old cursor, then recovers on reconnect", async () => {
    const queryClient = client();
    const { gateway: ws, socket, sockets } = gateway();
    let failing = false;
    let reads = 0;
    const calls = install(() => {
      reads++;
      return failing
        ? json({ error: "internal" }, 503)
        : json({ messages: rows(1, reads), has_more: false });
    });
    await login(ada.email, "password123");
    const stamp = takeStamp()!;
    await observe(queryClient);
    cleanups.push(
      attachRealtimeRecovery(queryClient, stamp, ws, { readySpreadMs: 0 }),
    );
    socket.frame({ op: "ok", s: "server", c: "channel", n: 1 });
    failing = true;
    socket.frame({ op: "gap", s: "server", c: "channel" });
    socket.frame({ op: "ok", s: "server", c: "channel", n: 9 });
    const key = options(queryClient).queryKey;
    await vi.waitFor(() =>
      expect(queryClient.getQueryState(key)?.error).toMatchObject({
        code: "internal",
      }),
    );
    expect(queryClient.getQueryState(key)?.isInvalidated).toBe(true);
    expect(flattenPages(cache(queryClient).pages)).toEqual(rows(1));
    expect(ws.cursorsSnapshot.get("c:channel")).toBe(1);
    expect(ws.gapRecoveries()).toHaveLength(1);
    expect(calls.filter((path) => path.includes("/messages"))).toHaveLength(2);
    failing = false;
    socket.close();
    await vi.waitFor(() => expect(sockets).toHaveLength(2));
    sockets[1]!.emit("open");
    await vi.waitFor(() => expect(ws.gapRecoveries()).toEqual([]));
    expect(flattenPages(cache(queryClient).pages)).toEqual(rows(1, 3));
    expect(ws.cursorsSnapshot.get("c:channel")).toBe(9);
  });

  it("coalesces a burst of server/channel gaps into one canonical refetch", async () => {
    const queryClient = client();
    const { gateway: ws, socket } = gateway();
    let reads = 0;
    install(() => {
      reads++;
      return json({ messages: rows(1, reads), has_more: false });
    });
    await login(ada.email, "password123");
    const stamp = takeStamp()!;
    await observe(queryClient);
    cleanups.push(
      attachRealtimeRecovery(queryClient, stamp, ws, { readySpreadMs: 0 }),
    );
    socket.frame({ op: "gap", s: "server" });
    socket.frame({ op: "gap", s: "server", c: "channel" });
    socket.frame({ op: "gap", s: "server", c: "channel" });
    socket.frame({ op: "ok", s: "server", c: "channel", n: 7 });
    await vi.waitFor(() => expect(ws.gapRecoveries()).toEqual([]));
    expect(reads).toBe(2);
    expect(ws.cursorsSnapshot.get("c:channel")).toBe(7);
  });

  it("reconciles offline changes on ready even when replay reports no gap", async () => {
    const queryClient = client();
    const { gateway: ws, socket, sockets } = gateway();
    let history = rows(3);
    let reads = 0;
    install(() => {
      reads++;
      return json({ messages: history, has_more: false });
    });
    await login(ada.email, "password123");
    const stamp = takeStamp()!;
    await observe(queryClient);
    cleanups.push(
      attachRealtimeRecovery(queryClient, stamp, ws, { readySpreadMs: 0 }),
    );
    socket.frame({ op: "ok", s: "server", c: "channel", n: 3 });
    socket.close();
    history = [
      history[0]!,
      {
        ...history[2]!,
        content: "offline edit",
        edited_at: "2026-09-28T10:00:00Z",
      },
      ...rows(1, 4),
    ];
    await vi.waitFor(() => expect(sockets).toHaveLength(2));
    sockets[1]!.emit("open");
    await vi.waitFor(() =>
      expect(flattenPages(cache(queryClient).pages)).toEqual(history),
    );
    expect(reads).toBe(2);
    expect(ws.gapRecoveries()).toEqual([]);
  });

  it("a replacement bridge recovers a gap that preceded listener attachment", async () => {
    const queryClient = client();
    const { gateway: ws, socket } = gateway();
    let reads = 0;
    install(() => json({ messages: rows(1, ++reads), has_more: false }));
    await login(ada.email, "password123");
    const stamp = takeStamp()!;
    await observe(queryClient);
    socket.frame({ op: "ok", s: "server", c: "channel", n: 1 });
    socket.frame({ op: "gap", s: "server", c: "channel" });
    socket.frame({ op: "ok", s: "server", c: "channel", n: 8 });
    cleanups.push(
      attachRealtimeRecovery(queryClient, stamp, ws, { readySpreadMs: 0 }),
    );
    await vi.waitFor(() => expect(ws.gapRecoveries()).toEqual([]), {
      timeout: 1000,
    });
    expect(flattenPages(cache(queryClient).pages)).toEqual(rows(1, 2));
    expect(ws.cursorsSnapshot.get("c:channel")).toBe(8);
  });

  it("a newer gap during held recovery requires a second snapshot before acknowledgement", async () => {
    const queryClient = client();
    const { gateway: ws, socket } = gateway();
    const response = deferred<Response>(),
      started = deferred<void>();
    let reads = 0;
    install(() => {
      reads++;
      if (reads === 2) {
        started.resolve();
        return response.promise;
      }
      return json({ messages: rows(1, reads), has_more: false });
    });
    await login(ada.email, "password123");
    const stamp = takeStamp()!;
    await observe(queryClient);
    cleanups.push(
      attachRealtimeRecovery(queryClient, stamp, ws, { readySpreadMs: 0 }),
    );
    socket.frame({ op: "ok", s: "server", c: "channel", n: 1 });
    socket.frame({ op: "gap", s: "server", c: "channel" });
    socket.frame({ op: "ok", s: "server", c: "channel", n: 8 });
    await started.promise;
    socket.frame({ op: "gap", s: "server", c: "channel" });
    socket.frame({ op: "ok", s: "server", c: "channel", n: 12 });
    expect(ws.cursorsSnapshot.get("c:channel")).toBe(1);
    response.resolve(json({ messages: rows(1, 2), has_more: false }));
    await vi.waitFor(() => expect(ws.gapRecoveries()).toEqual([]));
    expect(reads).toBe(3);
    expect(flattenPages(cache(queryClient).pages)).toEqual(rows(1, 3));
    expect(ws.cursorsSnapshot.get("c:channel")).toBe(12);
  });

  it("does not acknowledge a held recovery after the same account starts a new session", async () => {
    const queryClient = client();
    const { gateway: ws, socket } = gateway();
    const response = deferred<Response>(),
      started = deferred<void>();
    let reads = 0;
    install(() => {
      if (++reads === 2) {
        started.resolve();
        return response.promise;
      }
      return json({ messages: rows(1), has_more: false });
    });
    await login(ada.email, "password123");
    const stamp = takeStamp()!;
    await observe(queryClient);
    cleanups.push(
      attachRealtimeRecovery(queryClient, stamp, ws, { readySpreadMs: 0 }),
    );
    socket.frame({ op: "ok", s: "server", c: "channel", n: 1 });
    socket.frame({ op: "gap", s: "server", c: "channel" });
    socket.frame({ op: "ok", s: "server", c: "channel", n: 8 });
    await started.promise;
    await login(ada.email, "password123");
    response.resolve(json({ messages: rows(1, 2), has_more: false }));
    const key = messageKeys.channel(stamp.userId, stamp.generation, "channel");
    await vi.waitFor(() =>
      expect(queryClient.getQueryState(key)?.error).toMatchObject({
        name: "AbortError",
      }),
    );
    expect(ws.cursorsSnapshot.get("c:channel")).toBe(1);
    expect(ws.gapRecoveries()).toHaveLength(1);
    expect(
      queryClient.getQueryData<InfiniteData<MessagePage>>(key)?.pages[0]
        ?.messages,
    ).toEqual(rows(1));
  });

  it("does not refetch after bridge cleanup or a queued account logout", async () => {
    const queryClient = client();
    const { gateway: ws, socket } = gateway();
    let reads = 0;
    install(() => {
      reads++;
      return json({ messages: rows(1), has_more: false });
    });
    await login(ada.email, "password123");
    const stamp = takeStamp()!;
    await observe(queryClient);
    const off = attachRealtimeRecovery(queryClient, stamp, ws, {
      readySpreadMs: 0,
    });
    socket.frame({ op: "gap", s: "server", c: "channel" });
    off();
    await Promise.resolve();
    expect(reads).toBe(1);
    cleanups.push(
      attachRealtimeRecovery(queryClient, stamp, ws, { readySpreadMs: 0 }),
    );
    socket.frame({ op: "gap", s: "server", c: "channel" });
    await logout();
    expect(reads).toBe(1);
  });
});

describe("private DM discovery and topicless resync", () => {
  it("resync fetches lists with no existing topics or cache, subscribes newly discovered DMs and reads first history", async () => {
    const queryClient = client();
    const { gateway: ws, socket } = gateway();
    const calls = install((url) => {
      if (url.pathname === "/api/dms") return json([{ id: "first-dm" }]);
      if (url.pathname === "/api/servers")
        return json([{ id: "joined-server" }]);
      if (url.pathname === "/api/channels/first-dm/messages")
        return json({
          messages: [{ ...rows(1)[0]!, channel_id: "first-dm", revision: 1 }],
          has_more: false,
        });
      throw new Error(`Unexpected path ${url.pathname}`);
    });
    await login(ada.email, "password123");
    const stamp = takeStamp()!;
    cleanups.push(
      attachRealtimeRecovery(queryClient, stamp, ws, { readySpreadMs: 0 }),
    );
    socket.frame({ op: "resync" });
    await vi.waitFor(() =>
      expect(calls).toContain("/api/channels/first-dm/messages"),
    );
    await vi.waitFor(() =>
      expect(
        queryClient.getQueryData<InfiniteData<MessagePage>>(
          messageKeys.channel(stamp.userId, stamp.generation, "first-dm"),
        )?.pages[0]?.messages,
      ).toHaveLength(1),
    );
    expect(socket.sent.map((s) => JSON.parse(s))).toContainEqual({
      op: "s",
      s: "first-dm",
      c: "first-dm",
    });
    expect(
      queryClient.getQueryData([
        "user",
        stamp.userId,
        stamp.generation,
        "dms",
        "list",
      ]),
    ).toEqual([{ id: "first-dm" }]);
  });
  it("coalesces duplicate discovery and cannot subscribe unconfirmed foreign IDs", async () => {
    const queryClient = client();
    const { gateway: ws, socket } = gateway();
    let lists = 0;
    install((url) => {
      if (url.pathname === "/api/dms") {
        lists++;
        return json([{ id: "allowed" }]);
      }
      if (url.pathname === "/api/servers") return json([]);
      return json({
        messages: [{ ...rows(1)[0]!, channel_id: "allowed", revision: 1 }],
        has_more: false,
      });
    });
    await login(ada.email, "password123");
    cleanups.push(attachRealtimeRecovery(queryClient, takeStamp()!, ws));
    socket.frame({ op: "dm", c: "allowed" });
    socket.frame({ op: "dm", c: "allowed" });
    socket.frame({ op: "dm", c: "foreign" });
    await vi.waitFor(() =>
      expect(socket.sent.map((s) => JSON.parse(s))).toContainEqual({
        op: "s",
        s: "allowed",
        c: "allowed",
      }),
    );
    expect(lists).toBe(1);
    expect(
      socket.sent.map((s) => JSON.parse(s)).some((s) => s.c === "foreign"),
    ).toBe(false);
  });
  it("preserves first-message deltas arriving while the discovered REST history is held", async () => {
    const queryClient = client();
    const { gateway: ws, socket } = gateway();
    const held = deferred<Response>();
    const calls = install((url) => {
      if (url.pathname === "/api/dms") return json([{ id: "first-dm" }]);
      if (url.pathname === "/api/servers") return json([]);
      return held.promise;
    });
    await login(ada.email, "password123");
    const stamp = takeStamp()!;
    cleanups.push(
      attachRealtimeRecovery(queryClient, stamp, ws, { readySpreadMs: 0 }),
    );
    const offEvent = ws.onEvent((event) =>
      applyChannelEvent(queryClient, stamp.userId, stamp.generation, event),
    );
    cleanups.push(offEvent);
    socket.frame({ op: "dm", c: "first-dm" });
    await vi.waitFor(() =>
      expect(calls).toContain("/api/channels/first-dm/messages"),
    );
    const first = { ...rows(1)[0]!, channel_id: "first-dm", revision: 10 };
    socket.frame({
      op: "e",
      s: "first-dm",
      c: "first-dm",
      ep: "epoch",
      n: 1,
      t: "c",
      r: 10,
      d: first,
    });
    held.resolve(json({ messages: [], has_more: false }));
    await vi.waitFor(() =>
      expect(
        queryClient.getQueryData<InfiniteData<MessagePage>>(
          messageKeys.channel(stamp.userId, stamp.generation, "first-dm"),
        )?.pages[0]?.messages,
      ).toEqual([first]),
    );
  });
  it("a failed discovery has no tight retry loop and reconnect retries it", async () => {
    const queryClient = client();
    const { gateway: ws, socket, sockets } = gateway();
    let failing = true;
    let lists = 0;
    install((url) => {
      if (url.pathname === "/api/dms") {
        lists++;
        return failing
          ? json({ error: "unavailable" }, 503)
          : json([{ id: "first-dm" }]);
      }
      if (url.pathname === "/api/servers") return json([]);
      return json({ messages: [], has_more: false });
    });
    await login(ada.email, "password123");
    cleanups.push(attachRealtimeRecovery(queryClient, takeStamp()!, ws));
    socket.frame({ op: "dm", c: "first-dm" });
    await vi.waitFor(() => expect(lists).toBe(1));
    await new Promise((r) => setTimeout(r, 30));
    expect(lists).toBe(1);
    failing = false;
    socket.close();
    await vi.waitFor(() => expect(sockets).toHaveLength(2));
    sockets[1]!.emit("open");
    await vi.waitFor(() =>
      expect(sockets[1]!.sent.map((s) => JSON.parse(s))).toContainEqual({
        op: "s",
        s: "first-dm",
        c: "first-dm",
      }),
    );
  });
  it("a held discovery after same-account session replacement cannot subscribe or fetch history", async () => {
    const queryClient = client();
    const { gateway: ws, socket } = gateway();
    const held = deferred<Response>();
    const calls = install((url) =>
      url.pathname === "/api/dms" ? held.promise : json([]),
    );
    await login(ada.email, "password123");
    cleanups.push(attachRealtimeRecovery(queryClient, takeStamp()!, ws));
    socket.frame({ op: "dm", c: "old-dm" });
    await vi.waitFor(() => expect(calls).toContain("/api/dms"));
    await login(ada.email, "password123");
    held.resolve(json([{ id: "old-dm" }]));
    await new Promise((r) => setTimeout(r, 30));
    expect(
      socket.sent.map((s) => JSON.parse(s)).some((s) => s.c === "old-dm"),
    ).toBe(false);
    expect(calls.some((path) => path.includes("/channels/old-dm"))).toBe(false);
  });
  it("resync reads a first history only for DMs that were not listed before", async () => {
    const queryClient = client();
    const { gateway: ws, socket } = gateway();
    const calls = install((url) => {
      if (url.pathname === "/api/dms")
        return json([{ id: "dm-a" }, { id: "dm-b" }, { id: "dm-c" }]);
      if (url.pathname === "/api/servers") return json([]);
      if (url.pathname === "/api/channels/dm-c/messages")
        return json({
          messages: [{ ...rows(1)[0]!, channel_id: "dm-c", revision: 1 }],
          has_more: false,
        });
      throw new Error(`Unexpected path ${url.pathname}`);
    });
    await login(ada.email, "password123");
    const stamp = takeStamp()!;
    queryClient.setQueryData(
      ["user", stamp.userId, stamp.generation, "dms", "list"],
      [{ id: "dm-a" }, { id: "dm-b" }],
    );
    ws.setTopics([
      { s: "dm-a", c: "dm-a" },
      { s: "dm-b", c: "dm-b" },
    ]);
    cleanups.push(
      attachRealtimeRecovery(queryClient, stamp, ws, { readySpreadMs: 0 }),
    );
    socket.frame({ op: "resync" });
    // The resync itself re-sends the resume frames of the known topics.
    const sentBefore = socket.sent.length;
    socket.frame({ op: "ok", s: "dm-a", c: "dm-a", n: 3 });
    socket.frame({ op: "ok", s: "dm-b", c: "dm-b", n: 4 });
    await vi.waitFor(() => expect(ws.pendingResync).toBeUndefined());
    await vi.waitFor(() => expect(ws.gapRecoveries()).toEqual([]));
    expect(ws.cursorsSnapshot.get("c:dm-a")).toBe(3);
    expect(ws.cursorsSnapshot.get("c:dm-b")).toBe(4);
    expect(calls.filter((path) => path.startsWith("/api/channels/"))).toEqual([
      "/api/channels/dm-c/messages",
    ]);
    const frames = socket.sent.slice(sentBefore).map((s) => JSON.parse(s));
    expect(frames).toContainEqual({ op: "s", s: "dm-c", c: "dm-c" });
    expect(frames.some((f) => f.s === "dm-a" || f.s === "dm-b")).toBe(false);
    expect(
      queryClient.getQueryData<InfiniteData<MessagePage>>(
        messageKeys.channel(stamp.userId, stamp.generation, "dm-c"),
      )?.pages[0]?.messages,
    ).toHaveLength(1);
  });
  it("an announced DM is read again even when it is listed and its history is cached", async () => {
    const queryClient = client();
    const { gateway: ws, socket } = gateway();
    let reads = 0;
    install((url) => {
      if (url.pathname === "/api/dms") return json([{ id: "dm-a" }]);
      if (url.pathname === "/api/servers") return json([]);
      if (url.pathname === "/api/channels/dm-a/messages") {
        reads++;
        return json({
          messages: [{ ...rows(1)[0]!, channel_id: "dm-a", revision: 1 }],
          has_more: false,
        });
      }
      throw new Error(`Unexpected path ${url.pathname}`);
    });
    await login(ada.email, "password123");
    const stamp = takeStamp()!;
    queryClient.setQueryData(
      ["user", stamp.userId, stamp.generation, "dms", "list"],
      [{ id: "dm-a" }],
    );
    await queryClient.fetchInfiniteQuery(
      messageQueryOptions(queryClient, stamp.userId, stamp.generation, "dm-a"),
    );
    expect(reads).toBe(1);
    ws.setTopics([{ s: "dm-a", c: "dm-a" }]);
    cleanups.push(
      attachRealtimeRecovery(queryClient, stamp, ws, { readySpreadMs: 0 }),
    );
    socket.frame({ op: "dm", c: "dm-a" });
    expect(ws.dmDiscoveries()).toEqual(["dm-a"]);
    await vi.waitFor(() => expect(ws.dmDiscoveries()).toEqual([]));
    // The cached history is off screen, so only the announced read runs.
    expect(reads).toBe(2);
  });
  it("a DM announcement reads only the announced history, not every listed DM", async () => {
    const queryClient = client();
    const { gateway: ws, socket } = gateway();
    const calls = install((url) => {
      if (url.pathname === "/api/dms")
        return json([{ id: "dm-a" }, { id: "dm-x" }, { id: "dm-b" }]);
      if (url.pathname === "/api/servers") return json([]);
      if (url.pathname === "/api/channels/dm-b/messages")
        return json({ messages: [], has_more: false });
      throw new Error(`Unexpected path ${url.pathname}`);
    });
    await login(ada.email, "password123");
    const stamp = takeStamp()!;
    queryClient.setQueryData(
      ["user", stamp.userId, stamp.generation, "dms", "list"],
      [{ id: "dm-a" }, { id: "dm-x" }],
    );
    ws.setTopics([
      { s: "dm-a", c: "dm-a" },
      { s: "dm-x", c: "dm-x" },
    ]);
    cleanups.push(
      attachRealtimeRecovery(queryClient, stamp, ws, { readySpreadMs: 0 }),
    );
    const sentBefore = socket.sent.length;
    socket.frame({ op: "dm", c: "dm-b" });
    await vi.waitFor(() => expect(ws.dmDiscoveries()).toEqual([]));
    expect(calls.filter((path) => path.startsWith("/api/channels/"))).toEqual([
      "/api/channels/dm-b/messages",
    ]);
    expect(socket.sent.slice(sentBefore).map((s) => JSON.parse(s))).toEqual([
      { op: "s", s: "dm-b", c: "dm-b" },
    ]);
  });
});

it("a second private DM announcement during a held list gets another authoritative discovery read", async () => {
  const queryClient = client();
  const { gateway: ws, socket } = gateway();
  const firstList = deferred<Response>();
  let reads = 0;
  install((url) => {
    if (url.pathname === "/api/dms")
      return ++reads === 1
        ? firstList.promise
        : json([{ id: "first" }, { id: "second" }]);
    if (url.pathname === "/api/servers") return json([]);
    return json({ messages: [], has_more: false });
  });
  await login(ada.email, "password123");
  const stamp = takeStamp()!;
  cleanups.push(
    attachRealtimeRecovery(queryClient, stamp, ws, { readySpreadMs: 0 }),
  );
  socket.frame({ op: "dm", c: "first" });
  await vi.waitFor(() => expect(reads).toBe(1));
  socket.frame({ op: "dm", c: "second" });
  firstList.resolve(json([{ id: "first" }]));
  await vi.waitFor(() =>
    expect(socket.sent.map((s) => JSON.parse(s))).toContainEqual({
      op: "s",
      s: "second",
      c: "second",
    }),
  );
  expect(
    queryClient.getQueryData([
      "user",
      stamp.userId,
      stamp.generation,
      "dms",
      "list",
    ]),
  ).toEqual([{ id: "first" }, { id: "second" }]);
});

it("a canonical 404 for an open deleted history does not block recovery of surviving topics", async () => {
  const queryClient = client();
  const { gateway: ws, socket } = gateway();
  let removed = false;
  install((url) => {
    if (url.pathname === "/api/channels/deleted/messages")
      return removed
        ? json({ error: "not_found" }, 404)
        : json({ messages: rows(1), has_more: false });
    return json({ messages: rows(1), has_more: false });
  });
  await login(ada.email, "password123");
  const stamp = takeStamp()!;
  const gone = messageQueryOptions(
    queryClient,
    stamp.userId,
    stamp.generation,
    "deleted",
  );
  await queryClient.fetchInfiniteQuery(gone);
  const observer = new InfiniteQueryObserver(queryClient, gone);
  const off = observer.subscribe(() => undefined);
  cleanups.push(() => {
    off();
    observer.destroy();
  });
  await queryClient.fetchInfiniteQuery(options(queryClient));
  ws.setTopics([
    { s: "server", c: "deleted" },
    { s: "server", c: "channel" },
  ]);
  cleanups.push(
    attachRealtimeRecovery(queryClient, stamp, ws, { readySpreadMs: 0 }),
  );
  removed = true;
  socket.frame({ op: "gap", s: "server", c: "channel", ep: "epoch" });
  socket.frame({ op: "ok", s: "server", c: "channel", ep: "epoch", n: 2 });
  await vi.waitFor(() => expect(ws.cursorsSnapshot.get("c:channel")).toBe(2));
  expect(queryClient.getQueryData(gone.queryKey)).toBeUndefined();
  expect(queryClient.getQueryState(gone.queryKey)?.error).toMatchObject({
    code: "not_found",
  });
  expect(ws.gapRecoveries()).toEqual([]);
  expect(socket.sent.map((s) => JSON.parse(s))).toContainEqual({
    op: "u",
    s: "server",
    c: "deleted",
  });
});

it("an off-screen history is only invalidated and reads its canonical snapshot when opened", async () => {
  const queryClient = client();
  const { gateway: ws, socket } = gateway();
  let history = rows(2);
  const calls = install(() => json({ messages: history, has_more: false }));
  await login(ada.email, "password123");
  const stamp = takeStamp()!;
  await queryClient.fetchInfiniteQuery(options(queryClient));
  cleanups.push(
    attachRealtimeRecovery(queryClient, stamp, ws, { readySpreadMs: 0 }),
  );
  cleanups.push(
    ws.onEvent((event) =>
      applyChannelEvent(queryClient, stamp.userId, stamp.generation, event),
    ),
  );
  socket.frame({ op: "ok", s: "server", c: "channel", n: 2 });
  // A message lands in the gap and is never delivered live.
  history = [...history, ...rows(1, 3)];
  socket.frame({ op: "gap", s: "server", c: "channel" });
  socket.frame({ op: "ok", s: "server", c: "channel", n: 3 });
  await vi.waitFor(() => expect(ws.gapRecoveries()).toEqual([]));
  expect(calls.filter((path) => path.includes("/messages"))).toHaveLength(1);
  expect(ws.cursorsSnapshot.get("c:channel")).toBe(3);
  const key = options(queryClient).queryKey;
  expect(queryClient.getQueryState(key)?.isInvalidated).toBe(true);
  // A live event patches the hole-y cache but keeps it invalidated.
  const live = rows(1, 4)[0]!;
  history = [...history, live];
  socket.frame({
    op: "e",
    t: "c",
    s: "server",
    c: "channel",
    n: 4,
    i: live.id,
    d: live,
  });
  expect(flattenPages(cache(queryClient).pages).map((row) => row.id)).toEqual([
    "m001",
    "m002",
    "m004",
  ]);
  expect(queryClient.getQueryState(key)?.isInvalidated).toBe(true);
  await observe(queryClient);
  await vi.waitFor(() =>
    expect(flattenPages(cache(queryClient).pages)).toEqual(history),
  );
  expect(calls.filter((path) => path.includes("/messages"))).toHaveLength(2);
});

it("spreads the refetch after a reconnect and folds the resume gaps into it", async () => {
  vi.useFakeTimers();
  cleanups.push(() => vi.useRealTimers());
  vi.spyOn(Math, "random").mockReturnValue(0.5);
  const queryClient = client();
  const { gateway: ws, socket, sockets } = gateway();
  let reads = 0;
  install(() => json({ messages: rows(1, ++reads), has_more: false }));
  await login(ada.email, "password123");
  const stamp = takeStamp()!;
  await observe(queryClient);
  cleanups.push(attachRealtimeRecovery(queryClient, stamp, ws));
  socket.frame({ op: "ok", s: "server", c: "channel", n: 1 });
  socket.close();
  await vi.advanceTimersByTimeAsync(0);
  sockets[1]!.emit("open");
  // The answers to the resume frames arrive in later tasks.
  await vi.advanceTimersByTimeAsync(0);
  sockets[1]!.frame({ op: "gap", s: "server", c: "channel" });
  sockets[1]!.frame({ op: "ok", s: "server", c: "channel", n: 6 });
  await vi.advanceTimersByTimeAsync(READY_SPREAD_MS / 2 - 1);
  expect(reads).toBe(1);
  await vi.advanceTimersByTimeAsync(1);
  await vi.waitFor(() => expect(ws.gapRecoveries()).toEqual([]));
  // One refetch covers the reconnect and the gap it reported.
  expect(reads).toBe(2);
  expect(ws.cursorsSnapshot.get("c:channel")).toBe(6);
});

it("a second reconnect during the spread joins the waiting round", async () => {
  vi.useFakeTimers();
  cleanups.push(() => vi.useRealTimers());
  vi.spyOn(Math, "random").mockReturnValue(0.5);
  const queryClient = client();
  const { gateway: ws, socket, sockets } = gateway();
  let reads = 0;
  install(() => json({ messages: rows(1, ++reads), has_more: false }));
  await login(ada.email, "password123");
  const stamp = takeStamp()!;
  await observe(queryClient);
  cleanups.push(attachRealtimeRecovery(queryClient, stamp, ws));
  socket.close();
  await vi.advanceTimersByTimeAsync(0);
  sockets[1]!.emit("open");
  await vi.advanceTimersByTimeAsync(300);
  sockets[1]!.close();
  await vi.advanceTimersByTimeAsync(0);
  sockets[2]!.emit("open");
  // The first wait still ends on time; the second reconnect adds none.
  await vi.advanceTimersByTimeAsync(READY_SPREAD_MS / 2 - 301);
  expect(reads).toBe(1);
  await vi.advanceTimersByTimeAsync(1);
  expect(reads).toBe(2);
  await vi.advanceTimersByTimeAsync(READY_SPREAD_MS * 2);
  expect(reads).toBe(2);
});

it("a resync during the spread joins the waiting round", async () => {
  vi.useFakeTimers();
  cleanups.push(() => vi.useRealTimers());
  vi.spyOn(Math, "random").mockReturnValue(0.5);
  const queryClient = client();
  const { gateway: ws, socket, sockets } = gateway();
  const calls = install((url) =>
    url.pathname === "/api/dms" || url.pathname === "/api/servers"
      ? json([])
      : json({ messages: rows(1), has_more: false }),
  );
  await login(ada.email, "password123");
  const stamp = takeStamp()!;
  await observe(queryClient);
  cleanups.push(attachRealtimeRecovery(queryClient, stamp, ws));
  socket.close();
  await vi.advanceTimersByTimeAsync(0);
  sockets[1]!.emit("open");
  await vi.advanceTimersByTimeAsync(100);
  sockets[1]!.frame({ op: "resync" });
  expect(ws.pendingResync).toBeDefined();
  await vi.advanceTimersByTimeAsync(READY_SPREAD_MS / 2 - 101);
  expect(calls.filter((path) => path.includes("/messages"))).toHaveLength(1);
  await vi.advanceTimersByTimeAsync(1);
  // The first wait still ends on time and the round carries the resync.
  expect(calls.filter((path) => path.includes("/messages"))).toHaveLength(2);
  await vi.waitFor(() => expect(ws.pendingResync).toBeUndefined());
  await vi.advanceTimersByTimeAsync(READY_SPREAD_MS * 2);
  expect(calls.filter((path) => path === "/api/dms")).toHaveLength(1);
  expect(calls.filter((path) => path === "/api/servers")).toHaveLength(1);
  expect(calls.filter((path) => path.includes("/messages"))).toHaveLength(2);
});

it("DM and topicless resync received before bridge attachment remain recoverable", async () => {
  const queryClient = client();
  const { gateway: ws, socket } = gateway();
  install((url) =>
    url.pathname === "/api/dms"
      ? json([{ id: "first-dm" }])
      : url.pathname === "/api/servers"
        ? json([])
        : json({ messages: rows(1), has_more: false }),
  );
  await login(ada.email, "password123");
  const stamp = takeStamp()!;
  socket.frame({ op: "resync" });
  socket.frame({ op: "dm", c: "first-dm" });
  expect(ws.pendingResync).toBeDefined();
  expect(ws.dmDiscoveries()).toEqual(["first-dm"]);
  cleanups.push(
    attachRealtimeRecovery(queryClient, stamp, ws, { readySpreadMs: 0 }),
  );
  await vi.waitFor(() => expect(ws.dmDiscoveries()).toEqual([]));
  expect(ws.pendingResync).toBeUndefined();
  expect(
    queryClient.getQueryData<InfiniteData<MessagePage>>(
      messageKeys.channel(stamp.userId, stamp.generation, "first-dm"),
    )?.pages[0]?.messages,
  ).toHaveLength(1);
});

it("navigation replacing a bridge while discovery is held retains its pending DM intent", async () => {
  const queryClient = client();
  const { gateway: ws, socket } = gateway();
  const oldResponse = deferred<Response>();
  let reads = 0;
  install((url) =>
    url.pathname === "/api/dms"
      ? ++reads === 1
        ? oldResponse.promise
        : json([{ id: "first-dm" }])
      : url.pathname === "/api/servers"
        ? json([])
        : json({ messages: rows(1), has_more: false }),
  );
  await login(ada.email, "password123");
  const stamp = takeStamp()!;
  const oldBridge = attachRealtimeRecovery(queryClient, stamp, ws, {
    readySpreadMs: 0,
  });
  socket.frame({ op: "dm", c: "first-dm" });
  await vi.waitFor(() => expect(reads).toBe(1));
  oldBridge();
  cleanups.push(
    attachRealtimeRecovery(queryClient, stamp, ws, { readySpreadMs: 0 }),
  );
  await vi.waitFor(() => expect(ws.dmDiscoveries()).toEqual([]));
  oldResponse.resolve(json([]));
  await new Promise((r) => setTimeout(r, 20));
  expect(
    queryClient.getQueryData([
      "user",
      stamp.userId,
      stamp.generation,
      "dms",
      "list",
    ]),
  ).toEqual([{ id: "first-dm" }]);
  expect(socket.sent.map((s) => JSON.parse(s))).toContainEqual({
    op: "s",
    s: "first-dm",
    c: "first-dm",
  });
});
