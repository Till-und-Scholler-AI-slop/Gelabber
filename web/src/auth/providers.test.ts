import { describe, expect, it } from "vitest";

import { ApiError } from "../api/client.ts";
import { retryProviders } from "./providers.ts";

describe("asking for the sign-in providers again", () => {
  it("happens when the session changed under the request", () => {
    const stale = new DOMException("The session changed.", "AbortError");
    expect(retryProviders(0, stale)).toBe(true);
    expect(retryProviders(2, stale)).toBe(true);
    expect(retryProviders(3, stale)).toBe(false);
  });

  it("does not happen when the API refused or failed", () => {
    expect(retryProviders(0, new ApiError("internal", 500, "x"))).toBe(false);
    expect(retryProviders(0, new TypeError("network"))).toBe(false);
  });
});
