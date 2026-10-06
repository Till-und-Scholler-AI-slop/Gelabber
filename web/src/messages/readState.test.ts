import { QueryClient, QueryObserver } from "@tanstack/react-query";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { login, resetSessionForTests } from "../auth/session.ts";
import { takeStamp } from "../auth/scope.ts";
import {
  attachReadRecovery,
  listReadState,
  mayMarkRead,
  readBoundary,
  readKey,
  refreshChatWorkflows,
} from "./readState.ts";
import { messageContext, searchMessages } from "./search.ts";
import type { Message } from "./types.ts";
import type { Gateway } from "../ws/client.ts";
const user = {
  id: "read-user",
  email: "read@example.test",
  name: "Read",
  avatar_url: null,
  created_at: "2026-10-05T00:00:00Z",
};
const json = (body: unknown) =>
  new Response(JSON.stringify(body), {
    headers: { "Content-Type": "application/json" },
  });
let client: QueryClient;
let response: () => Response | Promise<Response>;
beforeEach(async () => {
  resetSessionForTests();
  client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  response = () => json([]);
  vi.stubGlobal(
    "fetch",
    vi.fn((url: RequestInfo | URL) =>
      String(url).includes("/auth/login")
        ? Promise.resolve(json({ user, csrf_token: "csrf" }))
        : Promise.resolve(response()),
    ),
  );
  await login(user.email, "password");
});
afterEach(() => {
  client.clear();
  resetSessionForTests();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe("read eligibility and recovery", () => {
  it("does not mark background, history, search or incomplete snapshots", () => {
    expect(mayMarkRead(true, true, true, true)).toBe(true);
    for (const flags of [
      [false, true, true, true],
      [true, false, true, true],
      [true, true, false, true],
      [true, true, true, false],
    ])
      expect(
        mayMarkRead(...(flags as [boolean, boolean, boolean, boolean])),
      ).toBe(false);
  });
  it("cancels pre-event read snapshots before refreshing, including initial fetches", async () => {
    let resolve!: (r: Response) => void;
    response = () =>
      new Promise((r) => {
        resolve = r;
      });
    const stamp = takeStamp()!;
    const observer = new QueryObserver(client, {
      queryKey: readKey(stamp),
      queryFn: ({ signal }) => listReadState(signal),
      retry: false,
    });
    const unsubscribe = observer.subscribe(() => {});
    await vi.waitFor(() => expect(resolve).toBeTypeOf("function"));
    response = () => json([{ channel_id: "c", unread_count: 1 }]);
    await refreshChatWorkflows(client, stamp);
    resolve(json([{ channel_id: "c", unread_count: 100 }]));
    await vi.waitFor(() =>
      expect(client.getQueryData(readKey(stamp))).toEqual([
        { channel_id: "c", unread_count: 1 },
      ]),
    );
    unsubscribe();
    observer.destroy();
  });
  it("chooses immutable insertion order from delivered rows, excluding optimistic rows", () => {
    const row = (id: string, created_order: number, created_at: string) =>
      ({ id, created_order, created_at }) as Message;
    const olderClock = row("late", 3, "2026-10-05T00:00:00Z");
    const latestClock = row("seen", 2, "2026-10-05T01:00:00Z");
    expect(
      readBoundary([
        olderClock,
        latestClock,
        row("tmp:pending", 99, "2026-10-05T02:00:00Z"),
      ]),
    ).toBe(olderClock);
    expect(
      readBoundary([row("tmp:only", 100, "2026-10-05T02:00:00Z")]),
    ).toBeUndefined();
  });
  it("cancels a stale search during an edit/delete refresh and leaves newer scoped results", async () => {
    let resolve!: (response: Response) => void;
    response = () =>
      new Promise((r) => {
        resolve = r;
      });
    const stamp = takeStamp()!;
    const key = [
      "user",
      stamp.userId,
      stamp.generation,
      "message-search",
      "channel",
      "red fox",
    ];
    const observer = new QueryObserver(client, {
      queryKey: key,
      queryFn: ({ signal }) =>
        searchMessages("channel", "red fox", undefined, signal),
      retry: false,
    });
    const unsubscribe = observer.subscribe(() => {});
    await vi.waitFor(() => expect(resolve).toBeTypeOf("function"));
    response = () => json({ messages: [], has_more: false });
    await refreshChatWorkflows(client, stamp, "channel");
    resolve(json({ messages: [{ id: "deleted" }], has_more: false }));
    await vi.waitFor(() =>
      expect(client.getQueryData(key)).toEqual({
        messages: [],
        has_more: false,
      }),
    );
    unsubscribe();
    observer.destroy();
  });
  it("cancels a pre-delete context before refreshing the selected message", async () => {
    let resolve!: (r: Response) => void;
    response = () =>
      new Promise((r) => {
        resolve = r;
      });
    const stamp = takeStamp()!;
    const key = [
      "user",
      stamp.userId,
      stamp.generation,
      "message-context",
      "channel",
      "target",
    ];
    const observer = new QueryObserver(client, {
      queryKey: key,
      queryFn: ({ signal }) => messageContext("channel", "target", signal),
      retry: false,
    });
    const unsubscribe = observer.subscribe(() => {});
    await vi.waitFor(() => expect(resolve).toBeTypeOf("function"));
    const current = {
      target_id: "target",
      messages: [],
      before: null,
      after: null,
    };
    response = () => json(current);
    await refreshChatWorkflows(client, stamp, "channel");
    resolve(json({ ...current, messages: [{ id: "deleted" }] }));
    await vi.waitFor(() => expect(client.getQueryData(key)).toEqual(current));
    unsubscribe();
    observer.destroy();
  });
  it("aborts context requests on observer removal and rejects late private snapshots", async () => {
    let resolve!: (r: Response) => void;
    const signals: AbortSignal[] = [];
    vi.stubGlobal(
      "fetch",
      vi.fn((_url: RequestInfo | URL, options: RequestInit) => {
        signals.push(options.signal as AbortSignal);
        return new Promise<Response>((r) => {
          resolve = r;
        });
      }),
    );
    const stamp = takeStamp()!;
    const key = [
      "user",
      stamp.userId,
      stamp.generation,
      "message-context",
      "channel",
      "target",
    ];
    const observer = new QueryObserver(client, {
      queryKey: key,
      queryFn: ({ signal }) => messageContext("channel", "target", signal),
      retry: false,
    });
    const unsubscribe = observer.subscribe(() => {});
    await vi.waitFor(() => expect(signals).toHaveLength(1));
    unsubscribe();
    observer.destroy();
    expect(signals[0]!.aborted).toBe(true);
    resolve(
      json({
        target_id: "target",
        messages: [{ id: "private" }],
        before: null,
        after: null,
      }),
    );
    await Promise.resolve();
    expect(client.getQueryData(key)).toBeUndefined();
  });
  it("debounces separate replay frames, scopes refreshes and lets an in-flight search finish", async () => {
    vi.useFakeTimers();
    const win = new EventTarget() as EventTarget & {
      setInterval: typeof setInterval;
      clearInterval: typeof clearInterval;
    };
    win.setInterval = setInterval;
    win.clearInterval = clearInterval;
    const doc = Object.assign(new EventTarget(), {
      visibilityState: "visible",
    });
    vi.stubGlobal("window", win);
    vi.stubGlobal("document", doc);
    let event!: (event: { c: string }) => void;
    const gateway = {
      onEvent: (callback: typeof event) => {
        event = callback;
        return () => {};
      },
      onReady: () => () => {},
      onGap: () => () => {},
      onResync: () => () => {},
      onDm: () => () => {},
    } as unknown as Gateway;
    const stamp = takeStamp()!;
    const key = [
      "user",
      stamp.userId,
      stamp.generation,
      "message-search",
      "a",
      "query",
    ];
    const other = [...key.slice(0, 4), "b", "query"];
    client.setQueryData(key, "cached");
    client.setQueryData(other, "other");
    let complete!: (result: string) => void;
    let signal!: AbortSignal;
    const query = vi.fn(({ signal: input }: { signal: AbortSignal }) => {
      signal = input;
      return query.mock.calls.length === 1
        ? new Promise<string>((resolve) => {
            complete = resolve;
          })
        : Promise.resolve("current");
    });
    const unrelated = vi.fn(async () => "unrelated");
    const a = new QueryObserver(client, {
      queryKey: key,
      queryFn: query,
      staleTime: Infinity,
    });
    const b = new QueryObserver(client, {
      queryKey: other,
      queryFn: unrelated,
      staleTime: Infinity,
    });
    const stopA = a.subscribe(() => {}),
      stopB = b.subscribe(() => {});
    const inFlight = a.refetch();
    const cleanup = attachReadRecovery(client, stamp, gateway);
    for (let frame = 0; frame < 128; frame++) {
      event({ c: "a" });
      await vi.advanceTimersByTimeAsync(1);
    }
    expect(query).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(750);
    expect(signal.aborted).toBe(false);
    expect(query).toHaveBeenCalledTimes(1);
    expect(unrelated).not.toHaveBeenCalled();
    complete("old snapshot");
    await inFlight;
    await vi.advanceTimersByTimeAsync(0);
    expect(query).toHaveBeenCalledTimes(2);
    expect(client.getQueryData(key)).toBe("current");
    expect(unrelated).not.toHaveBeenCalled();
    cleanup();
    stopA();
    stopB();
    a.destroy();
    b.destroy();
  });

  it("coalesces events, reconnects and gaps; detaches on logout and hidden polling", async () => {
    vi.useFakeTimers();
    const window = new EventTarget() as EventTarget & {
      setInterval: typeof setInterval;
      clearInterval: typeof clearInterval;
    };
    window.setInterval = setInterval;
    window.clearInterval = clearInterval;
    const document = new EventTarget() as EventTarget & {
      visibilityState: string;
    };
    document.visibilityState = "visible";
    vi.stubGlobal("window", window);
    vi.stubGlobal("document", document);
    const callbacks: Record<string, (...args: never[]) => void> = {};
    const register = (type: string) => (cb: () => void) => {
      callbacks[type] = cb;
      return () => {
        delete callbacks[type];
      };
    };
    const gateway = {
      onEvent: register("event"),
      onReady: register("ready"),
      onGap: register("gap"),
      onResync: register("resync"),
      onDm: register("dm"),
    } as unknown as Gateway;
    const cancel = vi.spyOn(client, "invalidateQueries");
    const cleanup = attachReadRecovery(client, takeStamp()!, gateway);
    callbacks.event({ c: "channel" } as never);
    callbacks.ready();
    callbacks.gap();
    await vi.advanceTimersByTimeAsync(750);
    expect(cancel).toHaveBeenCalledTimes(3); // Search, context and private read snapshot.
    cancel.mockClear();
    document.visibilityState = "hidden";
    await vi.advanceTimersByTimeAsync(15_000);
    expect(cancel).not.toHaveBeenCalled();
    document.visibilityState = "visible";
    document.dispatchEvent(new Event("visibilitychange"));
    await vi.advanceTimersByTimeAsync(750);
    expect(cancel).toHaveBeenCalledTimes(3);
    cancel.mockClear();
    resetSessionForTests();
    callbacks.resync();
    await Promise.resolve();
    expect(cancel).not.toHaveBeenCalled();
    cleanup();
    expect(Object.keys(callbacks)).toEqual([]);
  });
});
