import { describe, expect, it } from "vitest";

import { parsePreferences, UI_SCALES } from "./preferences.ts";

describe("interface preferences", () => {
  it("keeps the compact text size unless another one was chosen", () => {
    expect(parsePreferences(null).uiScale).toBe("compact");
    expect(parsePreferences({ compactRooms: true }).uiScale).toBe("compact");
    expect(parsePreferences({ uiScale: "huge" }).uiScale).toBe("compact");
    expect(parsePreferences({ uiScale: "large" }).uiScale).toBe("large");
  });

  it("scales the interface by an eighth per step", () => {
    expect(UI_SCALES).toEqual({ compact: 1, standard: 1.125, large: 1.25 });
  });
});
