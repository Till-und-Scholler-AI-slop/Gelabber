import { isNotFound, isRedirect } from "@tanstack/react-router";
import { describe, expect, it } from "vitest";
import { router } from "./routes.tsx";
import { SETTINGS_AREAS } from "./pages/settingsAreas.ts";

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
    // Pending UI renders nothing, at once, and is not held: TanStack's
    // default minimum of 500 ms left every cold start blank (#227).
    expect(router.options.defaultPendingMs).toBe(0);
    expect(router.options.defaultPendingMinMs).toBe(0);
    expect(router.options.defaultPendingComponent?.({} as never)).toBeNull();
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
