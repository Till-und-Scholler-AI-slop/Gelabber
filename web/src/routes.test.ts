import { renderToStaticMarkup } from "react-dom/server";
import { createElement } from "react";
import { describe, expect, it } from "vitest";

import { useSession } from "./auth/session.ts";
import { router, validateAuthSearch } from "./routes.tsx";

describe("router", () => {
  it("shows the first page as soon as the bootstrap request is done", () => {
    // Pending UI shows at once and is not held: TanStack's default minimum
    // of 500 ms left every cold start blank (#227). It carries on the
    // static splash instead of leaving the page empty.
    expect(router.options.defaultPendingMs).toBe(0);
    expect(router.options.defaultPendingMinMs).toBe(0);
    const pending = router.options.defaultPendingComponent;
    expect(pending).toBeDefined();
    const html = renderToStaticMarkup(createElement(pending!));
    expect(html).toContain("<strong>Gelabber</strong>");
    expect(html).toContain('<span role="status">Verbinde…</span>');
  });

  it("sends a signed-out invitee to registration and back", async () => {
    useSession.setState({ status: "anonymous", user: null });
    const guard = router.routesById["/auth/invite/$code"].options.beforeLoad;
    let thrown: unknown;
    try {
      await guard?.({ location: { href: "/invite/x" } } as never);
    } catch (error) {
      thrown = error;
    }
    expect(thrown).toBeInstanceOf(Response);
    expect((thrown as { options: unknown }).options).toMatchObject({
      to: "/register",
      search: { redirect: "/invite/x" },
    });
  });

  it("keeps only known auth search values", () => {
    expect(
      validateAuthSearch({
        redirect: "/invite/x",
        reason: "expired",
        email: "ada@example.org",
      }),
    ).toEqual({
      redirect: "/invite/x",
      reason: "expired",
      email: "ada@example.org",
    });
    expect(
      validateAuthSearch({
        redirect: "//evil.example",
        reason: "hacked",
        email: 42,
      }),
    ).toEqual({});
    expect(validateAuthSearch({ email: "x".repeat(255) })).toEqual({});
  });

  it("draws the front door without the app header", () => {
    for (const id of [
      "/auth/login",
      "/auth/register",
      "/auth/forgot-password",
      "/auth/reset-password",
      "/auth/verify-email",
      "/auth/invite/$code",
    ] as const) {
      expect(router.routesById[id].parentRoute.options.staticData).toEqual({
        bare: true,
      });
    }
  });
});
