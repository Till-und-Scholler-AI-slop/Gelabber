import {
  QueryClient,
  QueryClientProvider,
  type InfiniteData,
} from "@tanstack/react-query";
import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { ApiError } from "../api/client.ts";
import type { JoinRequest, Listing, ListingPage } from "./types.ts";

const stamp = { userId: "lena", generation: 1 };

const remote = vi.hoisted(() => ({
  joinListing: vi.fn(),
  requestJoin: vi.fn(),
  withdrawRequest: vi.fn(),
  acceptJoinRequest: vi.fn(),
  declineJoinRequest: vi.fn(),
}));

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

import {
  directoryKeys,
  useAnswerJoinRequest,
  useJoinListing,
  useRequestJoin,
  useWithdrawRequest,
} from "./queries.ts";

const listing = (overrides: Partial<Listing> = {}): Listing => ({
  server_id: "keller",
  name: "Modellbau-Keller",
  icon_url: null,
  description: "",
  language: "de",
  member_count: 3,
  join_mode: "request",
  member: false,
  request: null,
  retry_at: null,
  ...overrides,
});

const searchKey = directoryKeys.search(stamp.userId, stamp.generation, {});

function client(row: Listing) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: Infinity } },
  });
  queryClient.setQueryData<InfiniteData<ListingPage, number>>(searchKey, {
    pages: [{ listings: [row], next_offset: null }],
    pageParams: [0],
  });
  return queryClient;
}

const rowOf = (queryClient: QueryClient) =>
  queryClient.getQueryData<InfiniteData<ListingPage, number>>(searchKey)
    ?.pages[0].listings[0];

/** The hook's mutation, taken from one render under `queryClient`. */
function mutationOf<T>(queryClient: QueryClient, useHook: () => T): T {
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

beforeEach(() => {
  for (const mock of Object.values(remote)) mock.mockReset();
});

describe("the row after an action", () => {
  it("opens the server after a join", async () => {
    const queryClient = client(listing({ join_mode: "open" }));
    remote.joinListing.mockResolvedValue({ id: "keller", name: "Keller" });
    await mutationOf(queryClient, useJoinListing).mutateAsync("keller");
    expect(rowOf(queryClient)).toMatchObject({
      member: true,
      request: null,
      member_count: 4,
    });
  });

  it("reads Angefragt after a request", async () => {
    const queryClient = client(listing());
    remote.requestJoin.mockResolvedValue({ request: "pending" });
    await mutationOf(queryClient, useRequestJoin).mutateAsync({
      serverId: "keller",
      message: " Hallo ",
    });
    expect(remote.requestJoin).toHaveBeenCalledWith("keller", "Hallo");
    expect(rowOf(queryClient)?.request).toBe("pending");
  });

  it("offers the request again after a withdrawal", async () => {
    const queryClient = client(listing({ request: "pending" }));
    remote.withdrawRequest.mockResolvedValue(undefined);
    await mutationOf(queryClient, useWithdrawRequest).mutateAsync("keller");
    expect(rowOf(queryClient)?.request).toBeNull();
  });

  it("reads the directory again after a refusal", async () => {
    const queryClient = client(listing());
    remote.requestJoin.mockRejectedValue(
      new ApiError("join_request_declined", 409, "declined"),
    );
    await expect(
      mutationOf(queryClient, useRequestJoin).mutateAsync({
        serverId: "keller",
        message: "",
      }),
    ).rejects.toThrow();
    expect(queryClient.getQueryState(searchKey)?.isInvalidated).toBe(true);
    expect(rowOf(queryClient)?.request).toBeNull();
  });
});

describe("answering a request", () => {
  const request = (user_id: string): JoinRequest => ({
    user_id,
    name: user_id,
    avatar_url: null,
    message: "",
    created_at: "2026-11-01T10:00:00Z",
  });
  const queueKey = directoryKeys.requests(
    stamp.userId,
    stamp.generation,
    "keller",
  );

  it("takes the answered row off the queue at once", async () => {
    const queryClient = client(listing());
    queryClient.setQueryData(queueKey, [request("tom"), request("ada")]);
    remote.declineJoinRequest.mockResolvedValue(undefined);
    await mutationOf(queryClient, () =>
      useAnswerJoinRequest("keller"),
    ).mutateAsync({ userId: "tom", accept: false });
    expect(
      queryClient
        .getQueryData<JoinRequest[]>(queueKey)
        ?.map((entry) => entry.user_id),
    ).toEqual(["ada"]);
  });
});
