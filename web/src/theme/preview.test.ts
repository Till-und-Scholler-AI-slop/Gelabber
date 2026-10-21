import { afterEach, describe, expect, it, vi } from "vitest";

import { builtinTheme } from "./presets.ts";
import {
  displayedThemeId,
  endThemePreview,
  escapeEndsPreview,
  previewTheme,
  useThemePreview,
} from "./preview.ts";
import { applyDefinition } from "./theme.ts";

describe("theme preview", () => {
  afterEach(() => {
    endThemePreview();
    vi.unstubAllGlobals();
  });

  it("shows the previewed theme over the active one, until it ends", () => {
    expect(displayedThemeId("dark", useThemePreview.getState().id)).toBe(
      "dark",
    );
    previewTheme("nord");
    expect(displayedThemeId("dark", useThemePreview.getState().id)).toBe(
      "nord",
    );
    // Leaving the themes area or pressing Esc ends it.
    endThemePreview();
    expect(displayedThemeId("dark", useThemePreview.getState().id)).toBe(
      "dark",
    );
  });

  it("ends on an Esc that nothing else handled", () => {
    const target = { closest: () => null } as unknown as EventTarget;
    const field = { closest: () => ({}) } as unknown as EventTarget;
    const esc = { key: "Escape", defaultPrevented: false, target };
    expect(escapeEndsPreview(esc)).toBe(true);
    expect(escapeEndsPreview({ ...esc, key: "Enter" })).toBe(false);
    expect(escapeEndsPreview({ ...esc, defaultPrevented: true })).toBe(false);
    // A dialog or a text field has its own Esc.
    expect(escapeEndsPreview({ ...esc, target: field })).toBe(false);
  });

  it("never keeps a preview for the next start", () => {
    const stored = new Map<string, string>();
    const setProperty = vi.fn();
    vi.stubGlobal("document", {
      documentElement: {
        classList: { toggle: vi.fn() },
        dataset: {},
        style: { setProperty },
      },
      querySelector: () => null,
    });
    vi.stubGlobal("localStorage", {
      setItem: (key: string, value: string) => stored.set(key, value),
    });
    applyDefinition(builtinTheme("nord"), false, { remember: false });
    expect(setProperty).toHaveBeenCalled();
    expect(stored.size).toBe(0);
    applyDefinition(builtinTheme("dark"));
    expect(stored.has("gelabber.theme.paint")).toBe(true);
  });
});
