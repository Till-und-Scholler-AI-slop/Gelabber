import { describe, expect, it } from "vitest";

import { parsePreferences } from "./preferences.ts";

describe("interface preferences", () => {
  it("keeps the compact text size unless another one was chosen", () => {
    expect(parsePreferences(null).uiScale).toBe("compact");
    expect(parsePreferences({ compactRooms: true }).uiScale).toBe("compact");
    expect(parsePreferences({ uiScale: "huge" }).uiScale).toBe("compact");
    expect(parsePreferences({ uiScale: "large" }).uiScale).toBe("large");
  });
});
