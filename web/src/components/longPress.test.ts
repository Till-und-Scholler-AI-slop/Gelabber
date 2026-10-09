import { describe, expect, it } from "vitest";

import { isChromeLongPress } from "./longPress.ts";

const list = (selectors: string) => selectors.split(",").map((s) => s.trim());

/** A pressed element: what it and its ancestors match, nearest first. */
function pressed(...path: string[]) {
  const target = {
    closest(selectors: string) {
      const hit = path.find((own) => list(selectors).includes(own));
      return hit === undefined
        ? null
        : { matches: (wanted: string) => list(wanted).includes(hit) };
    },
  };
  return target as unknown as EventTarget;
}

describe("long press on chrome", () => {
  it("is recognised in the drawers, the headers and the call dock", () => {
    for (const chrome of [
      ".workspace-drawer",
      ".workspace-mobile-topbar",
      ".lr-channel-header",
      ".app-header",
      ".voice-session-dock",
    ])
      expect(
        isChromeLongPress(
          { target: pressed("a", chrome), pointerType: "touch" },
          true,
        ),
      ).toBe(true);
  });

  it("leaves messages and other content alone", () => {
    expect(
      isChromeLongPress(
        { target: pressed("a", ".lr-message-text"), pointerType: "touch" },
        true,
      ),
    ).toBe(false);
  });

  it("leaves fields and dialogs inside chrome alone", () => {
    for (const path of [
      ["input", ".lr-channel-header"],
      ["textarea", ".workspace-drawer"],
      ["a", ".gel-modal", ".workspace-drawer"],
      ["input", ".gel-modal", ".workspace-drawer"],
    ])
      expect(
        isChromeLongPress(
          { target: pressed(...path), pointerType: "touch" },
          true,
        ),
      ).toBe(false);
  });

  it("keeps the right-click menu", () => {
    const target = pressed("a", ".workspace-drawer");
    expect(isChromeLongPress({ target, pointerType: "mouse" }, true)).toBe(
      false,
    );
    expect(isChromeLongPress({ target, pointerType: "touch" }, false)).toBe(
      false,
    );
    expect(isChromeLongPress({ target }, false)).toBe(false);
  });

  it("works where the event carries no pointer type", () => {
    const target = pressed("a", ".workspace-drawer");
    expect(isChromeLongPress({ target }, true)).toBe(true);
    expect(isChromeLongPress({ target: null }, true)).toBe(false);
  });
});
