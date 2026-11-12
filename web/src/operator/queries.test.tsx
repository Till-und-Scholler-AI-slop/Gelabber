import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it, vi } from "vitest";

const fixture = vi.hoisted(() => ({
  options: [] as Record<string, unknown>[],
  operator: false,
}));

// A server render reads a store's initial state: the session comes from here.
vi.mock("../auth/store.ts", () => ({
  useSession: (select: (state: { operator: boolean }) => unknown) =>
    select({ operator: fixture.operator }),
}));
vi.mock("../auth/scope.ts", () => ({
  useUserId: () => "u1",
  scopeGeneration: () => 3,
}));

vi.mock("@tanstack/react-query", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@tanstack/react-query")>()),
  useQuery: (options: Record<string, unknown>) => {
    fixture.options.push(options);
    return { data: undefined };
  },
}));

import {
  OVERVIEW_REFETCH_MS,
  operatorKeys,
  useOperatorOverview,
  useOperatorRefresh,
} from "./queries.ts";

function Probe() {
  useOperatorOverview();
  return null;
}

afterEach(() => {
  fixture.options = [];
  fixture.operator = false;
});

describe("operator queries", () => {
  it("refetches the overview every minute under the user's scope", () => {
    fixture.operator = true;
    renderToStaticMarkup(<Probe />);
    expect(OVERVIEW_REFETCH_MS).toBe(60_000);
    expect(fixture.options[0]).toMatchObject({
      queryKey: ["user", "u1", 3, "operator", "overview"],
      enabled: true,
      refetchInterval: 60_000,
    });
  });

  it("does not ask the server for someone who is not an operator", () => {
    renderToStaticMarkup(<Probe />);
    expect(fixture.options[0]).toMatchObject({ enabled: false });
  });

  it("marks every cached list stale after an action that asks first", () => {
    // Delist and relist run outside useOperatorAction and refresh this way;
    // the Server page and Meldungen must show the new state without a reload.
    const client = new QueryClient();
    const servers = operatorKeys.servers("u1", 3, { filter: "listed" });
    const reports = operatorKeys.reports("u1", 3, { status: "open" });
    const other = ["user", "u1", 3, "servers"];
    for (const key of [servers, reports, other]) client.setQueryData(key, {});
    const refreshes: (() => void)[] = [];
    function Actions({ found }: { found: (refresh: () => void) => void }) {
      found(useOperatorRefresh());
      return null;
    }
    renderToStaticMarkup(
      <QueryClientProvider client={client}>
        <Actions found={(refresh) => refreshes.push(refresh)} />
      </QueryClientProvider>,
    );
    refreshes[0]?.();
    const stale = (key: readonly unknown[]) =>
      client.getQueryState(key)?.isInvalidated;
    expect(stale(servers)).toBe(true);
    expect(stale(reports)).toBe(true);
    expect(stale(other)).toBe(false);
  });
});
