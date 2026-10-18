import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it } from "vitest";

import { resetUserVolumesForTests } from "../voice/userVolume.ts";
import {
  LocallyMutedIcon,
  LocallyMutedMark,
  UserVolumeControl,
  UserVolumeView,
} from "./UserVolumeControl.tsx";

afterEach(() => {
  resetUserVolumesForTests();
});

describe("per-user volume control", () => {
  it("starts at 100 % with nothing to reset", () => {
    const html = renderToStaticMarkup(
      <UserVolumeControl userId="u-bob" name="Bob" />,
    );
    expect(html).toContain('aria-label="Lautstärke von Bob"');
    // The visible label names the slider; the screen reader hears percent.
    expect(html).toContain('aria-valuetext="100 %"');
    expect(html).toContain("nicht für Stream-Ton");
    expect(html).toContain('max="200"');
    expect(html).toContain('value="100"');
    expect(html).toContain("100 %");
    expect(html).toMatch(/<button[^>]*disabled[^>]*>Zurücksetzen/);
    expect(renderToStaticMarkup(<LocallyMutedIcon userId="u-bob" />)).toBe("");
  });

  it("shows the remembered level and the local mute", () => {
    // A server render reads the store's initial state, so the level is given.
    const html = renderToStaticMarkup(
      <UserVolumeView
        userId="u-bob"
        name="Bob"
        level={{ volume: 1.4, muted: true }}
      />,
    );
    expect(html).toContain('value="140"');
    expect(html).toContain('aria-valuetext="140 %"');
    expect(html).toContain("140 %");
    expect(html).toMatch(/type="checkbox"[^>]*checked/);
    expect(html).not.toMatch(/<button[^>]*disabled[^>]*>Zurücksetzen/);
    expect(renderToStaticMarkup(<LocallyMutedMark />)).toContain(
      "Lokal stummgeschaltet",
    );
  });
});
