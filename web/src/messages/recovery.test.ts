import {
  InfiniteQueryObserver,
  QueryClient,
  type InfiniteData,
} from "@tanstack/react-query";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  applyChannelEvent,
  messageKeys,
  messageQueryOptions,
} from "./queries.ts";
import { encodeCursor, flattenPages } from "./pages.ts";
import type { Message, MessagePage } from "./types.ts";

function message(id: string): Message {
  return {
    id,
    channel_id: "channel",
    author: { id: "author", name: "Author", avatar_url: null },
    content: id,
    created_at: "2026-09-28T09:00:00Z",
    edited_at: null,
    attachments: [],
  };
}

function rows(count: number, start = 1): Message[] {
  return Array.from({ length: count }, (_, index) => {
    const n = start + index;
    return {
      ...message(`m${String(n).padStart(3, "0")}`),
      created_at: new Date(Date.UTC(2026, 8, 28, 9, 0, n)).toISOString(),
    };
  });
}
function json(page: MessagePage): Response {
  return new Response(JSON.stringify(page), {
    headers: { "Content-Type": "application/json" },
  });
}
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}
const clients: QueryClient[] = [];
function queryClient() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: Infinity } },
  });
  clients.push(client);
  return client;
}
function options(client: QueryClient) {
  return {
    ...messageQueryOptions(client, "user", 0, "channel"),
    retry: false as const,
  };
}
function cached(client: QueryClient) {
  return client.getQueryData<InfiniteData<MessagePage>>(
    messageKeys.channel("user", 0, "channel"),
  )!;
}
function event(client: QueryClient, t: "c" | "e" | "d", row: Message) {
  applyChannelEvent(client, "user", 0, { t, c: "channel", i: row.id, d: row });
}
afterEach(() => {
  for (const client of clients.splice(0)) client.clear();
  vi.unstubAllGlobals();
});

describe("message snapshot completeness", () => {
  it("a background create cannot masquerade as the unopened channel's complete history", () => {
    const client = new QueryClient();
    applyChannelEvent(client, "user", 0, {
      t: "c",
      c: "channel",
      d: message("new"),
    });
    expect(
      client.getQueryData(messageKeys.channel("user", 0, "channel")),
    ).toBeUndefined();
    client.clear();
  });

  it("a live write preserves invalidation and the last snapshot's freshness", async () => {
    const client = new QueryClient();
    const key = messageKeys.channel("user", 0, "channel");
    client.setQueryData(
      key,
      {
        pages: [{ messages: [message("old")], has_more: false }],
        pageParams: [undefined],
      },
      { updatedAt: 1 },
    );
    await client.invalidateQueries({
      queryKey: key,
      exact: true,
      refetchType: "none",
    });
    applyChannelEvent(client, "user", 0, {
      t: "c",
      c: "channel",
      d: message("new"),
    });
    expect(client.getQueryState(key)?.isInvalidated).toBe(true);
    expect(client.getQueryState(key)?.dataUpdatedAt).toBe(1);
    client.clear();
  });
});

describe("real infinite-query reads against controlled HTTP and WS responses", () => {
  it("does not replace a newer REST edit with an older WS edit within the same millisecond", async () => {
    const client = queryClient();
    const latest = {
      ...message("one"),
      content: "newer",
      edited_at: "2026-09-28T09:01:00.000002Z",
    };
    vi.stubGlobal(
      "fetch",
      vi.fn(() =>
        Promise.resolve(json({ messages: [latest], has_more: false })),
      ),
    );
    await client.fetchInfiniteQuery(options(client));
    event(client, "e", {
      ...latest,
      content: "older",
      edited_at: "2026-09-28T09:01:00.000001Z",
    });
    expect(flattenPages(cached(client).pages)).toEqual([latest]);
  });
  it("opens an unseen channel after a background event with all 61 messages and working older paging", async () => {
    const client = queryClient();
    const history = rows(61);
    const requests: string[] = [];
    vi.stubGlobal(
      "fetch",
      vi.fn((input: RequestInfo | URL) => {
        const url = new URL(String(input), "https://test.invalid");
        requests.push(url.search);
        const before = url.searchParams.get("before");
        const eligible = history.filter(
          (row) => !before || encodeCursor(row) < before,
        );
        return Promise.resolve(
          json({
            messages: eligible.slice(-50),
            has_more: eligible.length > 50,
          }),
        );
      }),
    );
    event(client, "c", history[60]!);
    expect(
      client.getQueryData(messageKeys.channel("user", 0, "channel")),
    ).toBeUndefined();
    await client.fetchInfiniteQuery(options(client));
    expect(cached(client).pages[0]?.messages).toHaveLength(50);
    expect(cached(client).pages[0]?.has_more).toBe(true);
    const observer = new InfiniteQueryObserver(client, options(client));
    const off = observer.subscribe(() => undefined);
    await observer.fetchNextPage();
    off();
    observer.destroy();
    expect(flattenPages(cached(client).pages)).toEqual(history);
    expect(cached(client).pages[1]?.has_more).toBe(false);
    expect(requests).toHaveLength(2);
    expect(new URLSearchParams(requests[1]).get("before")).toBe(
      encodeCursor(history[11]!),
    );
  });

  it("does not skip older history when a delayed create predates the latest REST page", async () => {
    const client = queryClient();
    const history = rows(61);
    const cursors: (string | null)[] = [];
    vi.stubGlobal(
      "fetch",
      vi.fn((input: RequestInfo | URL) => {
        const before = new URL(
          String(input),
          "https://test.invalid",
        ).searchParams.get("before");
        cursors.push(before);
        const eligible = history.filter(
          (row) => !before || encodeCursor(row) < before,
        );
        return Promise.resolve(
          json({
            messages: eligible.slice(-50),
            has_more: eligible.length > 50,
          }),
        );
      }),
    );
    await client.fetchInfiniteQuery(options(client));
    // Publish is delayed beyond the initial snapshot, but the immutable
    // creation time belongs to the still-unloaded, older part of history.
    event(client, "c", history[0]!);
    const observer = new InfiniteQueryObserver(client, options(client));
    const off = observer.subscribe(() => undefined);
    await observer.fetchNextPage();
    off();
    observer.destroy();
    expect(cursors[1]).toBe(encodeCursor(history[11]!));
    expect(flattenPages(cached(client).pages)).toEqual(history);
  });

  it("keeps create/edit/delete events that arrive before the first GET body", async () => {
    const client = queryClient();
    const response = deferred<Response>();
    vi.stubGlobal(
      "fetch",
      vi.fn(() => response.promise),
    );
    const history = rows(3);
    const done = client.fetchInfiniteQuery(options(client));
    const newest = rows(1, 4)[0]!;
    const edited = {
      ...history[0]!,
      content: "edited while reading",
      edited_at: "2026-09-28T09:01:00Z",
    };
    event(client, "c", newest);
    event(client, "e", edited);
    event(client, "d", history[1]!);
    event(client, "c", history[1]!); // A delayed duplicate must not undo the tombstone.
    expect(
      client.getQueryData(messageKeys.channel("user", 0, "channel")),
    ).toBeUndefined();
    response.resolve(json({ messages: history, has_more: false }));
    await done;
    expect(flattenPages(cached(client).pages)).toEqual([
      edited,
      history[2],
      newest,
    ]);
  });

  it("applies the same changes after the snapshot and ignores an older edit", async () => {
    const client = queryClient();
    const history = rows(3);
    vi.stubGlobal(
      "fetch",
      vi.fn(() =>
        Promise.resolve(json({ messages: history, has_more: false })),
      ),
    );
    await client.fetchInfiniteQuery(options(client));
    const newest = rows(1, 4)[0]!;
    const edited = {
      ...history[0]!,
      content: "latest edit",
      edited_at: "2026-09-28T09:02:00Z",
    };
    event(client, "c", newest);
    event(client, "e", edited);
    event(client, "e", {
      ...edited,
      content: "old edit",
      edited_at: "2026-09-28T09:01:00Z",
    });
    event(client, "d", history[1]!);
    expect(flattenPages(cached(client).pages)).toEqual([
      edited,
      history[2],
      newest,
    ]);
  });

  it("does not lose newer-page changes while fetching an older page", async () => {
    const client = queryClient();
    const history = rows(60);
    const response = deferred<Response>();
    vi.stubGlobal(
      "fetch",
      vi.fn((input: RequestInfo | URL) =>
        new URL(String(input), "https://test.invalid").searchParams.has(
          "before",
        )
          ? response.promise
          : Promise.resolve(
              json({ messages: history.slice(10), has_more: true }),
            ),
      ),
    );
    await client.fetchInfiniteQuery(options(client));
    const observer = new InfiniteQueryObserver(client, options(client));
    const off = observer.subscribe(() => undefined);
    const done = observer.fetchNextPage();
    const edited = {
      ...history[59]!,
      content: "new newest-page edit",
      edited_at: "2026-09-28T09:03:00Z",
    };
    event(client, "e", edited);
    event(client, "d", history[11]!);
    const created = rows(1, 61)[0]!;
    event(client, "c", created);
    response.resolve(json({ messages: history.slice(0, 10), has_more: false }));
    await done;
    off();
    observer.destroy();
    const result = flattenPages(cached(client).pages);
    expect(result).toHaveLength(60);
    expect(result.find((row) => row.id === edited.id)).toEqual(edited);
    expect(result.some((row) => row.id === history[11]!.id)).toBe(false);
    expect(result.at(-1)).toEqual(created);
  });

  it("overlays changes across a multi-page refetch even after its newest GET has returned", async () => {
    const client = queryClient();
    const history = rows(100);
    let refetch = false;
    const olderResponse = deferred<Response>();
    const olderStarted = deferred<void>();
    vi.stubGlobal(
      "fetch",
      vi.fn((input: RequestInfo | URL) => {
        const before = new URL(
          String(input),
          "https://test.invalid",
        ).searchParams.get("before");
        if (!before)
          return Promise.resolve(
            json({ messages: history.slice(50), has_more: true }),
          );
        if (refetch) {
          olderStarted.resolve();
          return olderResponse.promise;
        }
        return Promise.resolve(
          json({ messages: history.slice(0, 50), has_more: false }),
        );
      }),
    );
    await client.fetchInfiniteQuery(options(client));
    const observer = new InfiniteQueryObserver(client, options(client));
    const off = observer.subscribe(() => undefined);
    await observer.fetchNextPage();
    refetch = true;
    const done = observer.refetch();
    await olderStarted.promise;
    const edited = {
      ...history[59]!,
      content: "latest edit",
      edited_at: "2026-09-28T09:03:00Z",
    };
    event(client, "e", edited);
    event(client, "e", {
      ...edited,
      content: "out of order",
      edited_at: "2026-09-28T09:02:00Z",
    });
    event(client, "d", history[9]!);
    event(client, "c", history[9]!);
    const created = rows(1, 101)[0]!;
    event(client, "c", created);
    expect(
      flattenPages(cached(client).pages).some(
        (row) => row.id === history[9]!.id,
      ),
    ).toBe(false);
    olderResponse.resolve(
      json({ messages: history.slice(0, 50), has_more: false }),
    );
    await done;
    off();
    observer.destroy();
    const result = flattenPages(cached(client).pages);
    expect(result).toHaveLength(100);
    expect(result.find((row) => row.id === edited.id)).toEqual(edited);
    expect(result.some((row) => row.id === history[9]!.id)).toBe(false);
    expect(result.at(-1)).toEqual(created);
    expect(cached(client).pages[0]?.older).toBe(encodeCursor(history[50]!));
  });
});
