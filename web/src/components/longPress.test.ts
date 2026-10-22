import { afterEach, describe, expect, it, vi } from "vitest";

import {
  isChromeLongPress,
  LONG_PRESS_MS,
  longPressGesture,
} from "./longPress.ts";

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

describe("long press gesture", () => {
  function setup() {
    vi.useFakeTimers();
    const pressed = vi.fn();
    const gesture = longPressGesture(pressed);
    return { pressed, gesture };
  }
  const finger = { pointerType: "touch", isPrimary: true, x: 10, y: 10 };
  afterEach(() => vi.useRealTimers());

  it("fires once after a finger rests", () => {
    const { pressed, gesture } = setup();
    gesture.down(finger);
    vi.advanceTimersByTime(LONG_PRESS_MS - 1);
    expect(pressed).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(pressed).toHaveBeenCalledTimes(1);
    gesture.end();
    vi.advanceTimersByTime(LONG_PRESS_MS);
    expect(pressed).toHaveBeenCalledTimes(1);
  });

  it("leaves a short tap alone", () => {
    const { pressed, gesture } = setup();
    gesture.down(finger);
    vi.advanceTimersByTime(100);
    gesture.end();
    vi.advanceTimersByTime(LONG_PRESS_MS);
    expect(pressed).not.toHaveBeenCalled();
  });

  it("is a scroll once the finger moves", () => {
    const { pressed, gesture } = setup();
    gesture.down(finger);
    gesture.move({ x: 10, y: 14 });
    gesture.move({ x: 10, y: 30 });
    vi.advanceTimersByTime(LONG_PRESS_MS);
    expect(pressed).not.toHaveBeenCalled();
  });

  it("ignores the mouse, which has its right click", () => {
    const { pressed, gesture } = setup();
    gesture.down({ ...finger, pointerType: "mouse" });
    vi.advanceTimersByTime(LONG_PRESS_MS);
    expect(pressed).not.toHaveBeenCalled();
  });
});
