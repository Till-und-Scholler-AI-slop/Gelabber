import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

const fixture = vi.hoisted(() => ({
  search: {} as Record<string, unknown>,
  user: null as { email: string } | null,
}));

vi.mock("@tanstack/react-router", () => ({
  useSearch: () => fixture.search,
  Link: ({ children, to }: { children: ReactNode; to: string }) => (
    <a href={to}>{children}</a>
  ),
}));
vi.mock("../auth/session.ts", async (importOriginal) => ({
  ...(await importOriginal<object>()),
  useSession: (select: (state: unknown) => unknown) =>
    select({ user: fixture.user }),
}));

import { ForgotPasswordPage } from "./ForgotPasswordPage.tsx";

const render = () =>
  renderToStaticMarkup(
    <QueryClientProvider client={new QueryClient()}>
      <ForgotPasswordPage />
    </QueryClientProvider>,
  );

describe("forgot password page", () => {
  it("fills in the signed-in account's address", () => {
    fixture.user = { email: "ada@example.org" };
    expect(render()).toContain('value="ada@example.org"');
  });

  it("prefers the address carried from the sign-in form", () => {
    fixture.user = null;
    fixture.search = { email: "bob@example.org" };
    expect(render()).toContain('value="bob@example.org"');
  });
});
