import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import { ApiError } from "../api/client.ts";

const fixture = vi.hoisted(() => ({ dm: {} as Record<string, unknown> }));

vi.mock("@tanstack/react-router", () => ({
  useParams: () => ({ channelId: "d1" }),
  useNavigate: () => () => Promise.resolve(),
}));
vi.mock("@tanstack/react-query", async (importOriginal) => ({
  ...(await importOriginal<object>()),
  useQueryClient: () => ({}),
}));
vi.mock("../auth/scope.ts", async (importOriginal) => ({
  ...(await importOriginal<object>()),
  useUserId: () => "u1",
}));
vi.mock("../dms/queries.ts", () => ({
  useDm: () => fixture.dm,
  useDms: () => ({ data: undefined }),
  forgetDm: () => {},
}));
vi.mock("../components/MessagePane.tsx", () => ({
  MessagePane: ({ channelName }: { channelName: string }) => (
    <section>Unterhaltung mit {channelName}</section>
  ),
}));
vi.mock("../components/TypingBar.tsx", () => ({ TypingBar: () => null }));
vi.mock("../components/UserProfileDialog.tsx", () => ({
  UserProfileDialog: () => null,
}));
vi.mock("../ws/useLive.ts", () => ({ useTypingInput: () => ({}) }));

import { DmChannelPage } from "./DmChannelPage.tsx";

const dm = {
  id: "d1",
  created_at: "2026-01-01T00:00:00Z",
  peer: { id: "u2", name: "Ada", avatar_url: null },
};

describe("DM page", () => {
  it("keeps the open conversation when a background refetch fails", () => {
    // TanStack Query keeps `data` beside the refetch's `error`.
    fixture.dm = {
      data: dm,
      error: new ApiError("internal", 502, "Serverfehler"),
      isFetching: false,
      refetch: vi.fn(),
    };
    const html = renderToStaticMarkup(<DmChannelPage />);
    expect(html).toContain("Unterhaltung mit Ada");
    expect(html).not.toContain('role="alert"');
  });

  it("offers a retry when the conversation never loaded", () => {
    fixture.dm = {
      data: undefined,
      error: new ApiError("internal", 502, "Serverfehler"),
      isFetching: false,
      refetch: vi.fn(),
    };
    expect(renderToStaticMarkup(<DmChannelPage />)).toContain(
      "Diese Unterhaltung konnte nicht geladen werden.",
    );
  });
});
