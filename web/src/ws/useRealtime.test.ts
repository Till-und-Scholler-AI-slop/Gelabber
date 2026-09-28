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
import { attachRealtimeRecovery } from "./useRealtime.ts";

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
    cleanups.push(attachRealtimeRecovery(queryClient, stamp, ws));
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
    cleanups.push(attachRealtimeRecovery(queryClient, stamp, ws));
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
    await queryClient.fetchInfiniteQuery(options(queryClient));
    cleanups.push(attachRealtimeRecovery(queryClient, stamp, ws));
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
    await queryClient.fetchInfiniteQuery(options(queryClient));
    cleanups.push(attachRealtimeRecovery(queryClient, stamp, ws));
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
    await queryClient.fetchInfiniteQuery(options(queryClient));
    cleanups.push(attachRealtimeRecovery(queryClient, stamp, ws));
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
    await queryClient.fetchInfiniteQuery(options(queryClient));
    socket.frame({ op: "ok", s: "server", c: "channel", n: 1 });
    socket.frame({ op: "gap", s: "server", c: "channel" });
    socket.frame({ op: "ok", s: "server", c: "channel", n: 8 });
    cleanups.push(attachRealtimeRecovery(queryClient, stamp, ws));
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
    await queryClient.fetchInfiniteQuery(options(queryClient));
    cleanups.push(attachRealtimeRecovery(queryClient, stamp, ws));
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
    await queryClient.fetchInfiniteQuery(options(queryClient));
    cleanups.push(attachRealtimeRecovery(queryClient, stamp, ws));
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
    await queryClient.fetchInfiniteQuery(options(queryClient));
    const off = attachRealtimeRecovery(queryClient, stamp, ws);
    socket.frame({ op: "gap", s: "server", c: "channel" });
    off();
    await Promise.resolve();
    expect(reads).toBe(1);
    cleanups.push(attachRealtimeRecovery(queryClient, stamp, ws));
    socket.frame({ op: "gap", s: "server", c: "channel" });
    await logout();
    expect(reads).toBe(1);
  });
});
