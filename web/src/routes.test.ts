import { describe, expect, it } from "vitest";
import { router } from "./routes.tsx";

describe("router", () => {
  it("shows the first page as soon as the bootstrap request is done", () => {
    // Pending UI renders nothing, at once, and is not held: TanStack's
    // default minimum of 500 ms left every cold start blank (#227).
    expect(router.options.defaultPendingMs).toBe(0);
    expect(router.options.defaultPendingMinMs).toBe(0);
    expect(router.options.defaultPendingComponent?.({} as never)).toBeNull();
  });
});
