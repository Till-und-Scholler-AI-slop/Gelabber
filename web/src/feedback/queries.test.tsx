import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";

const stamp = { userId: "tom", generation: 3 };

const remote = vi.hoisted(() => ({
  feedbackStatus: vi.fn(),
  sendFeedback: vi.fn(),
  readFeedback: vi.fn(),
  replyFeedback: vi.fn(),
}));
const queryOptions = vi.hoisted(() => ({ last: null as unknown }));

vi.mock("./api.ts", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./api.ts")>()),
  ...remote,
}));
vi.mock("../auth/scope.ts", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../auth/scope.ts")>()),
  takeStamp: () => stamp,
  stampHolds: () => true,
  useUserId: () => stamp.userId,
  scopeGeneration: () => stamp.generation,
}));
vi.mock("@tanstack/react-query", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@tanstack/react-query")>();
  return {
    ...actual,
    useQuery: (options: Parameters<typeof actual.useQuery>[0]) => {
      queryOptions.last = options;
      return actual.useQuery(options);
    },
  };
});

import {
  STATUS_REFETCH_MS,
  ensureFeedbackStatus,
  feedbackKeys,
  markFeedbackRead,
  nextFeedbackOffset,
  useFeedbackStatus,
  useReplyFeedback,
  useSendFeedback,
} from "./queries.ts";
import type { FeedbackEntry, FeedbackPage } from "./types.ts";

/** The hook's result, taken from one render under `queryClient`. */
function hookOf<T>(queryClient: QueryClient, useHook: () => T): T {
  let result: T | undefined;
  function Probe() {
    result = useHook();
    return null;
  }
  renderToStaticMarkup(
    <QueryClientProvider client={queryClient}>
      <Probe />
    </QueryClientProvider>,
  );
  return result as T;
}

const context = {
  client: "web" as const,
  os: "linux" as const,
  app_version: "0.10.0",
  locale: "de" as const,
  page: "/s/home",
};

beforeEach(() => {
  for (const mock of Object.values(remote)) mock.mockReset();
});

describe("the feedback status", () => {
  it("is read again on focus and every five minutes, never retried", () => {
    hookOf(new QueryClient(), useFeedbackStatus);
    expect(queryOptions.last).toMatchObject({
      queryKey: ["user", "tom", 3, "feedback", "status"],
      enabled: true,
      refetchInterval: 300_000,
      refetchOnWindowFocus: true,
      retry: false,
    });
    expect(STATUS_REFETCH_MS).toBe(5 * 60 * 1000);
  });

  it("is fetched once for the route guard, then taken from the cache", async () => {
    const queryClient = new QueryClient();
    remote.feedbackStatus.mockResolvedValue({ enabled: true, unread: 1 });
    await expect(ensureFeedbackStatus(queryClient, stamp)).resolves.toEqual({
      enabled: true,
      unread: 1,
    });
    await ensureFeedbackStatus(queryClient, stamp);
    expect(remote.feedbackStatus).toHaveBeenCalledOnce();
  });
});

describe("sending", () => {
  it("trims the text and reads the status and the list again", async () => {
    const queryClient = new QueryClient();
    const status = feedbackKeys.status(stamp.userId, stamp.generation);
    const list = feedbackKeys.list(stamp.userId, stamp.generation);
    queryClient.setQueryData(status, { enabled: true, unread: 0 });
    queryClient.setQueryData(list, { entries: [], total: 0 });
    remote.sendFeedback.mockResolvedValue({ id: "entry" });
    const image = new File(["x"], "a.png", { type: "image/png" });
    await hookOf(queryClient, useSendFeedback).mutateAsync({
      kind: "problem",
      text: "  Schwarz  ",
      image,
      context,
    });
    expect(remote.sendFeedback).toHaveBeenCalledWith(
      { kind: "problem", text: "Schwarz", context },
      image,
    );
    expect(queryClient.getQueryState(status)?.isInvalidated).toBe(true);
    expect(queryClient.getQueryState(list)?.isInvalidated).toBe(true);
  });
});

const entry = (id: string, overrides: Partial<FeedbackEntry> = {}) =>
  ({
    id,
    kind: "idea",
    status: "open",
    text: "Mehr Emojis",
    created_at: "2026-11-03T10:00:00Z",
    last_activity_at: "2026-11-03T10:00:00Z",
    image: null,
    context: { ...context, page: "/" },
    unread: false,
    messages: [],
    ...overrides,
  }) satisfies FeedbackEntry;

const cachedList = (queryClient: QueryClient, entries: FeedbackEntry[]) => {
  const list = feedbackKeys.list(stamp.userId, stamp.generation);
  queryClient.setQueryData(list, {
    pages: [{ entries, total: entries.length }],
    pageParams: [0],
  });
  return () =>
    queryClient.getQueryData<{ pages: FeedbackPage[] }>(list)!.pages[0]!
      .entries;
};

describe("the list", () => {
  it("pages by 20 until every entry is loaded", () => {
    const page = (count: number, total: number): FeedbackPage => ({
      entries: Array.from({ length: count }, (_, index) => entry(`${index}`)),
      total,
    });
    const first = page(20, 45);
    expect(nextFeedbackOffset(first, [first])).toBe(20);
    const second = page(20, 45);
    const third = page(5, 45);
    expect(nextFeedbackOffset(second, [first, second])).toBe(40);
    expect(nextFeedbackOffset(third, [first, second, third])).toBeUndefined();
    // An empty page ends it even when the total moved meanwhile.
    expect(nextFeedbackOffset(page(0, 50), [first, page(0, 50)])).toBe(
      undefined,
    );
  });
});

describe("reading", () => {
  it("marks the entry read, then reads the status again, not the list", async () => {
    const queryClient = new QueryClient();
    const status = feedbackKeys.status(stamp.userId, stamp.generation);
    queryClient.setQueryData(status, { enabled: true, unread: 1 });
    const entries = cachedList(queryClient, [
      entry("a", {
        unread: true,
        messages: [
          {
            id: "m1",
            from: "operators",
            text: "Gute Idee!",
            created_at: "2026-11-03T11:00:00Z",
            new: true,
          },
        ],
      }),
    ]);
    remote.readFeedback.mockResolvedValue(null);
    await markFeedbackRead(queryClient, stamp, "a", "m1");
    expect(remote.readFeedback).toHaveBeenCalledWith("a", "m1");
    expect(queryClient.getQueryState(status)?.isInvalidated).toBe(true);
    const list = feedbackKeys.list(stamp.userId, stamp.generation);
    expect(queryClient.getQueryState(list)?.isInvalidated).toBe(false);
    // The next visit starts without the marks.
    expect(entries()[0]).toMatchObject({
      unread: false,
      messages: [{ id: "m1", new: false }],
    });
  });

  it("keeps a reply after the one read through new", async () => {
    const queryClient = new QueryClient();
    const reply = (id: string) => ({
      id,
      from: "operators" as const,
      text: "Antwort",
      created_at: "2026-11-03T11:00:00Z",
      new: true,
    });
    const entries = cachedList(queryClient, [
      entry("a", { unread: true, messages: [reply("m1"), reply("m2")] }),
    ]);
    remote.readFeedback.mockResolvedValue(null);
    await markFeedbackRead(queryClient, stamp, "a", "m1");
    expect(entries()[0]).toMatchObject({
      unread: true,
      messages: [
        { id: "m1", new: false },
        { id: "m2", new: true },
      ],
    });
  });
});

describe("replying", () => {
  it("appends the reply, opens the entry and reads both again", async () => {
    const queryClient = new QueryClient();
    const status = feedbackKeys.status(stamp.userId, stamp.generation);
    queryClient.setQueryData(status, { enabled: true, unread: 0 });
    const entries = cachedList(queryClient, [entry("a", { status: "done" })]);
    const message = {
      id: "m9",
      from: "you" as const,
      text: "Noch da",
      created_at: "2026-11-05T09:00:00Z",
      new: false,
    };
    remote.replyFeedback.mockResolvedValue(message);
    await hookOf(queryClient, useReplyFeedback).mutateAsync({
      id: "a",
      text: "  Noch da \n",
    });
    expect(remote.replyFeedback).toHaveBeenCalledWith("a", "Noch da");
    expect(entries()[0]).toMatchObject({
      status: "open",
      last_activity_at: "2026-11-05T09:00:00Z",
      messages: [message],
    });
    expect(queryClient.getQueryState(status)?.isInvalidated).toBe(true);
    const list = feedbackKeys.list(stamp.userId, stamp.generation);
    expect(queryClient.getQueryState(list)?.isInvalidated).toBe(true);
  });
});
