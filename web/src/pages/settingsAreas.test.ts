import { describe, expect, it } from "vitest";

import { SETTINGS_AREAS, visibleSettingsAreas } from "./settingsAreas.ts";

const ids = (...args: Parameters<typeof visibleSettingsAreas>) =>
  visibleSettingsAreas(...args).map((area) => area.id);

describe("settings areas", () => {
  it("puts Feedback in Konto, after Sicherheit", () => {
    const order = SETTINGS_AREAS.map((area) => area.id);
    expect(order.indexOf("feedback")).toBe(order.indexOf("konto") + 1);
    expect(SETTINGS_AREAS.find((area) => area.id === "feedback")).toEqual({
      id: "feedback",
      scope: "account",
      group: "konto",
    });
  });

  it("lists Feedback only while the instance takes it", () => {
    expect(ids(true, undefined, false)).not.toContain("feedback");
    expect(ids(true, "feedback", false)).not.toContain("feedback");
    expect(ids(true, undefined, true)).toContain("feedback");
  });

  it("still offers App only where it can be installed, or while open", () => {
    expect(ids(false, undefined, true)).not.toContain("app");
    expect(ids(false, "app", true)).toContain("app");
    expect(ids(true, undefined, false)).toContain("app");
  });
});
