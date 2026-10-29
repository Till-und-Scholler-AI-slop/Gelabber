import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { ApiError } from "../api/client.ts";

const fixture = vi.hoisted(() => ({
  server: {} as Record<string, unknown>,
  servers: {} as Record<string, unknown>,
  retry: undefined as (() => void) | undefined,
}));

vi.mock("@tanstack/react-router", () => ({
  useParams: () => ({ serverId: "s1", channelId: "c1" }),
  useNavigate: () => () => Promise.resolve(),
  Link: ({ children, to }: { children: ReactNode; to: string }) => (
    <a href={to}>{children}</a>
  ),
}));
vi.mock("../servers/queries.ts", () => ({
  useServer: () => fixture.server,
  useServers: () => fixture.servers,
}));
vi.mock("../auth/scope.ts", () => ({ useUserId: () => "u1" }));
vi.mock("../dms/queries.ts", () => ({
  useDms: () => ({ data: [], isPending: false, isError: false }),
}));
vi.mock("../auth/session.ts", () => ({
  useSession: (select: (state: unknown) => unknown) =>
    select({ user: { id: "u1", name: "Rafi" } }),
}));
vi.mock("../servers/lastChannel.ts", () => ({
  useLastChannel: (select: (state: unknown) => unknown) =>
    select({ remember: () => {} }),
}));
// Records the retry the page hands over; the markup stays the real alert.
vi.mock("../components/LoadError.tsx", () => ({
  LoadError: ({
    message,
    onRetry,
  }: {
    message: string;
    onRetry: () => void;
  }) => {
    fixture.retry = onRetry;
    return <div role="alert">{message}</div>;
  },
}));
vi.mock("../components/MemberPanel.tsx", () => ({
  MemberPanel: () => null,
  MemberPanelToggle: () => null,
}));
vi.mock("../components/MessagePane.tsx", () => ({ MessagePane: () => null }));
vi.mock("../components/TypingBar.tsx", () => ({ TypingBar: () => null }));
vi.mock("../voice/VoiceRoom.tsx", () => ({ VoiceRoom: () => null }));
vi.mock("../voice/LiveHint.tsx", () => ({ LiveHint: () => null }));
vi.mock("../ws/useLive.ts", () => ({ useTypingInput: () => ({}) }));
vi.mock("../components/ServerDialogs.tsx", () => ({
  CreateServerDialog: () => null,
  JoinServerForm: () => <form aria-label="Einladung" />,
}));

import { ChannelPage } from "./ChannelPage.tsx";
import { WorkspaceIndexPage } from "./WorkspaceIndexPage.tsx";

const failure = new ApiError("internal", 500, "Serverfehler");

beforeEach(() => {
  fixture.retry = undefined;
});

describe("channel page load error", () => {
  it("offers a retry that refetches the server", () => {
    const refetch = vi.fn(() => Promise.resolve());
    fixture.server = {
      data: undefined,
      error: failure,
      isFetching: false,
      refetch,
    };
    const html = renderToStaticMarkup(<ChannelPage />);
    expect(html).toContain(
      '<div role="alert">Dieser Kanal konnte nicht geladen werden.</div>',
    );
    fixture.retry?.();
    expect(refetch).toHaveBeenCalledOnce();
  });

  it("shows the chat skeleton while the server is still loading", () => {
    fixture.server = {
      data: undefined,
      error: null,
      isFetching: true,
      refetch: vi.fn(),
    };
    expect(renderToStaticMarkup(<ChannelPage />)).toContain(
      "ui-main-skeleton--chat",
    );
    expect(fixture.retry).toBeUndefined();
  });
});

describe("workspace start page load error", () => {
  const base = {
    data: undefined,
    isPending: false,
    isError: false,
    isFetching: false,
    refetch: vi.fn(),
  };

  it("mounts the failure as its own alert with a retry", () => {
    fixture.servers = { ...base, isPending: true, isError: true };
    const html = renderToStaticMarkup(<WorkspaceIndexPage />);
    expect(html).toMatch(
      /<p role="alert"[^>]*>Die Server konnten nicht geladen werden\.<\/p>/,
    );
    expect(html).toContain('<button type="button" class=');
    expect(html).toContain(">Erneut versuchen</button>");
  });

  it("disables the retry while it runs", () => {
    fixture.servers = { ...base, isError: true, isFetching: true };
    const html = renderToStaticMarkup(<WorkspaceIndexPage />);
    expect(html).toMatch(
      /<button[^>]*disabled=""[^>]*>Wird geladen…<\/button>/,
    );
  });

  it("welcomes someone without a server with both ways in", () => {
    fixture.servers = { ...base, data: [] };
    const html = renderToStaticMarkup(<WorkspaceIndexPage />);
    expect(html).toContain("Willkommen, Rafi");
    expect(html).toContain("<h2>Eigenen Server erstellen</h2>");
    expect(html).toMatch(/ui-button--primary[^>]*>Server erstellen<\/button>/);
    expect(html).toContain("<h2>Server beitreten</h2>");
    expect(html).toContain('<form aria-label="Einladung">');
    expect(html).not.toContain('role="alert"');
    expect(html).not.toContain("Erneut versuchen");
  });

  it("holds the place of the welcome while the list loads", () => {
    fixture.servers = { ...base, isPending: true };
    const html = renderToStaticMarkup(<WorkspaceIndexPage />);
    expect(html).toContain("ui-main-skeleton--overview");
    expect(html).toContain("Wird geladen…");
    expect(html).not.toContain("Willkommen");
  });
});
