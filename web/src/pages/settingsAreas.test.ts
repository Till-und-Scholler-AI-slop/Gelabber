import { describe, expect, it } from "vitest";

import { visibleSettingsAreas } from "./settingsAreas.ts";

const ids = (...args: Parameters<typeof visibleSettingsAreas>) =>
  visibleSettingsAreas(...args).map((area) => area.id);

describe("settings areas", () => {
  it("lists Feedback only while the instance takes it", () => {
    expect(ids(false)).not.toContain("feedback");
    expect(ids(true)).toContain("feedback");
  });

  it("always lists App, installed or not", () => {
    // Once installed it still offers the desktop app and says it is installed.
    expect(ids(false)).toContain("app");
    expect(ids(true)).toContain("app");
  });
});
