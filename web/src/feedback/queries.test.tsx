import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";

const stamp = { userId: "tom", generation: 3 };

const remote = vi.hoisted(() => ({
  feedbackStatus: vi.fn(),
  sendFeedback: vi.fn(),
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
  useFeedbackStatus,
  useSendFeedback,
} from "./queries.ts";

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
