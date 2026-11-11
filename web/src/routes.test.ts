import { isNotFound, isRedirect } from "@tanstack/react-router";
import { renderToStaticMarkup } from "react-dom/server";
import { createElement } from "react";
import { afterEach, describe, expect, it } from "vitest";

import { useSession } from "./auth/session.ts";
import { OPERATOR_PAGES } from "./operator/areas.ts";
import { SETTINGS_AREAS } from "./pages/settingsAreas.ts";
import { router, validateAuthSearch } from "./routes.tsx";

/** Runs a route's guard and returns what it threw, if anything. */
function guard(routeId: string, params: Record<string, string> = {}) {
  const beforeLoad = (
    router.routesById as Record<
      string,
      { options: { beforeLoad?: (context: unknown) => unknown } }
    >
  )[routeId].options.beforeLoad;
  try {
    beforeLoad?.({ params });
  } catch (thrown) {
    return thrown;
  }
  return undefined;
}

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

  it("shows a German error with a reload when a page throws", () => {
    // TanStack's own fallback is an English "Something went wrong!".
    const failed = router.options.defaultErrorComponent;
    expect(failed).toBeDefined();
    const html = renderToStaticMarkup(
      createElement(failed!, { error: new Error("kaputt"), reset: () => {} }),
    );
    expect(html).toContain("Etwas ist schiefgelaufen");
    expect(html).toContain("Seite neu laden");
    expect(html).not.toContain("kaputt");
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

  it("lets a signed-in user ask for a new password", async () => {
    // Expired reset links and the verify page link here signed in too.
    useSession.setState({
      status: "authenticated",
      user: { id: "u1", name: "Ada", email: "ada@example.org" } as never,
    });
    try {
      await expect(
        router.routesById["/auth/forgot-password"].options.beforeLoad?.({
          search: {},
        } as never),
      ).resolves.toBeUndefined();
    } finally {
      useSession.setState({ status: "anonymous", user: null });
    }
  });

  it("hands a failed confirmation sign-in back to Sicherheit with its error", async () => {
    useSession.setState({
      status: "authenticated",
      user: { id: "u1", name: "Ada", email: "ada@example.org" } as never,
    });
    const login = router.routesById["/auth/login"].options.beforeLoad;
    const target = async (search: Record<string, unknown>) => {
      try {
        await login?.({ search: validateAuthSearch(search) } as never);
      } catch (thrown) {
        return (thrown as { options: { href?: string } }).options.href;
      }
      return undefined;
    };
    try {
      expect(
        await target({ oidc_error: "cancelled", redirect: "/settings/konto" }),
      ).toBe("/settings/konto?oidc_error=cancelled");
      expect(await target({ redirect: "/settings/konto" })).toBe(
        "/settings/konto",
      );
      // Elsewhere a signed-in visitor still just goes on.
      expect(
        await target({ oidc_error: "cancelled", redirect: "/invite/x" }),
      ).toBe("/invite/x");
      expect(await target({ oidc_error: "expired" })).toBe("/");
    } finally {
      useSession.setState({ status: "anonymous", user: null });
    }
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

  it("keeps the notice after an account deletion", () => {
    expect(validateAuthSearch({ reason: "deleted" })).toEqual({
      reason: "deleted",
    });
  });

  it("keeps an invitation code and a sign-in's admission refusal", () => {
    expect(
      validateAuthSearch({
        invitation: "k7m2p9qrs4tx",
        oidc_error: "invitation_invalid",
      }),
    ).toEqual({ invitation: "k7m2p9qrs4tx", oidc_error: "invitation_invalid" });
    expect(validateAuthSearch({ oidc_error: "invitation_required" })).toEqual({
      oidc_error: "invitation_required",
    });
    // The router parses an all-digit code as a number.
    expect(validateAuthSearch({ invitation: 234567892345 })).toEqual({
      invitation: "234567892345",
    });
    expect(
      validateAuthSearch({
        invitation: "<script>",
        oidc_error: "registration_closed",
      }),
    ).toEqual({});
    expect(validateAuthSearch({ invitation: "a".repeat(65) })).toEqual({});
    expect(validateAuthSearch({ invitation: "" })).toEqual({});
  });

  it("keeps every known oidc_error and drops the rest", () => {
    for (const code of [
      "cancelled",
      "expired",
      "failed",
      "email_unverified",
      "account_unverified",
      "identity_taken",
      "insecure_transport",
      "session_changed",
      "account_disabled",
    ])
      expect(validateAuthSearch({ oidc_error: code })).toEqual({
        oidc_error: code,
      });
    for (const code of ["registration_closed", "oidc_unavailable", "", 1])
      expect(validateAuthSearch({ oidc_error: code })).toEqual({});
  });

  it("draws the front door without the app header", () => {
    for (const id of [
      "/auth/login",
      "/auth/register",
      "/auth/forgot-password",
      "/auth/reset-password",
      "/auth/verify-email",
      "/auth/invite/$code",
      "/auth/sign-in/name",
    ] as const) {
      expect(router.routesById[id].parentRoute.options.staticData).toEqual({
        bare: true,
      });
    }
  });
});

describe("settings routes", () => {
  it("gives every settings area its own address inside the workspace", () => {
    for (const { id } of SETTINGS_AREAS) {
      const matches = router.matchRoutes(`/settings/${id}`, {});
      expect(matches.map((match) => match.routeId)).toEqual([
        "__root__",
        "/workspace",
        "/workspace/settings/$area",
      ]);
      expect(matches.at(-1)?.params).toMatchObject({ area: id });
      expect(guard("/workspace/settings/$area", { area: id })).toBeUndefined();
    }
  });

  it("knows no other areas", () => {
    expect(
      isNotFound(guard("/workspace/settings/$area", { area: "unbekannt" })),
    ).toBe(true);
  });

  it("sends old profile links to the profile area", () => {
    const thrown = guard("/profile");
    expect(isRedirect(thrown)).toBe(true);
    expect(thrown).toMatchObject({
      options: { to: "/settings/$area", params: { area: "profil" } },
    });
  });

  it("opens Darstellung where the sidebar already lists the areas", () => {
    // No window in this test, so it counts as a wide screen.
    const thrown = guard("/workspace/settings");
    expect(isRedirect(thrown)).toBe(true);
    expect(thrown).toMatchObject({
      options: { params: { area: "darstellung" }, replace: true },
    });
  });
});

describe("operator routes", () => {
  /** Runs an async guard and returns what it threw, if anything. */
  async function guardAsync(
    routeId: "/workspace/betrieb" | "/workspace/betrieb/$page",
    params: Record<string, string> = {},
  ) {
    try {
      await router.routesById[routeId].options.beforeLoad?.({
        params,
      } as never);
    } catch (thrown) {
      return thrown;
    }
    return undefined;
  }

  const signIn = (operator: boolean) =>
    useSession.setState({
      status: "authenticated",
      user: { id: "u1", name: "Lena", email: "lena@example.test" } as never,
      operator,
    });
  afterEach(() =>
    useSession.setState({ status: "anonymous", user: null, operator: false }),
  );

  it("gives every page of Betrieb its own address inside the workspace", async () => {
    signIn(true);
    for (const page of OPERATOR_PAGES) {
      const matches = router.matchRoutes(`/betrieb/${page}`, {});
      expect(matches.map((match) => match.routeId)).toEqual([
        "__root__",
        "/workspace",
        "/workspace/betrieb/$page",
      ]);
      expect(
        await guardAsync("/workspace/betrieb/$page", { page }),
      ).toBeUndefined();
    }
  });

  it("opens Server for /betrieb", async () => {
    signIn(true);
    const thrown = await guardAsync("/workspace/betrieb");
    expect(isRedirect(thrown)).toBe(true);
    expect(thrown).toMatchObject({
      options: {
        to: "/betrieb/$page",
        params: { page: "server" },
        replace: true,
      },
    });
  });

  it("is not found for anyone but an operator", async () => {
    signIn(false);
    expect(isNotFound(await guardAsync("/workspace/betrieb"))).toBe(true);
    for (const page of OPERATOR_PAGES)
      expect(
        isNotFound(await guardAsync("/workspace/betrieb/$page", { page })),
      ).toBe(true);
  });

  it("knows no other pages", async () => {
    signIn(true);
    expect(
      isNotFound(
        await guardAsync("/workspace/betrieb/$page", { page: "audit" }),
      ),
    ).toBe(true);
  });
});
