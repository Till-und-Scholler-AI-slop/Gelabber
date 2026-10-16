import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it, vi } from "vitest";

import { resetUserVolumesForTests } from "../voice/userVolume.ts";
import { LocallyMutedIcon, UserVolumeControl } from "./UserVolumeControl.tsx";

afterEach(() => {
  resetUserVolumesForTests();
  vi.unstubAllGlobals();
  vi.resetModules();
});

describe("per-user volume control", () => {
  it("starts at 100 % with nothing to reset", () => {
    const html = renderToStaticMarkup(
      <UserVolumeControl userId="u-bob" name="Bob" />,
    );
    expect(html).toContain('aria-label="Lautstärke von Bob"');
    expect(html).toContain('max="200"');
    expect(html).toContain('value="100"');
    expect(html).toContain("100 %");
    expect(html).toMatch(/<button[^>]*disabled[^>]*>Zurücksetzen/);
    expect(renderToStaticMarkup(<LocallyMutedIcon userId="u-bob" />)).toBe("");
  });

  it("shows the remembered level and the local mute", async () => {
    // A server render reads the store's initial state: what storage held.
    const saved = JSON.stringify({ "u-bob": { volume: 1.4, muted: true } });
    vi.stubGlobal("localStorage", {
      getItem: () => saved,
      setItem: () => undefined,
      removeItem: () => undefined,
    });
    vi.resetModules();
    const { LocallyMutedIcon, UserVolumeControl } =
      await import("./UserVolumeControl.tsx");
    const html = renderToStaticMarkup(
      <UserVolumeControl userId="u-bob" name="Bob" />,
    );
    expect(html).toContain('value="140"');
    expect(html).toContain("140 %");
    expect(html).toMatch(/type="checkbox"[^>]*checked/);
    expect(html).not.toMatch(/<button[^>]*disabled[^>]*>Zurücksetzen/);
    expect(renderToStaticMarkup(<LocallyMutedIcon userId="u-bob" />)).toContain(
      "Lokal stummgeschaltet",
    );
    expect(renderToStaticMarkup(<LocallyMutedIcon userId="u-cara" />)).toBe("");
  });
});
