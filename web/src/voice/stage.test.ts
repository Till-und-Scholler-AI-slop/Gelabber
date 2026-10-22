import { describe, expect, it, vi } from "vitest";

import {
  arrangeStage,
  createLongPress,
  fullscreenRoute,
  LONG_PRESS_MS,
  type StageKind,
} from "./stage.ts";

const tile = (id: string, kind: StageKind) => ({ id, kind });
const live = tile("mia-s", "live");
const screen = tile("lena-s", "live");
const other = tile("kim-s", "live");
const camera = tile("rafi-v", "camera");

describe("stage", () => {
  it("puts the first Live in the spotlight, the others in the filmstrip", () => {
    expect(arrangeStage([live, screen, other], [camera], null)).toEqual({
      spotlight: live,
      filmstrip: [screen, other],
    });
  });

  it("swaps a pinned share into the spotlight", () => {
    expect(arrangeStage([live, screen, other], [camera], "kim-s")).toEqual({
      spotlight: other,
      filmstrip: [live, screen],
    });
  });

  it("takes a pinned camera there too, the shares going to the filmstrip", () => {
    expect(arrangeStage([screen], [camera], "rafi-v")).toEqual({
      spotlight: camera,
      filmstrip: [screen],
    });
  });

  it("has no stage for cameras alone", () => {
    expect(arrangeStage([], [camera], null)).toEqual({
      spotlight: null,
      filmstrip: [],
    });
  });

  it("forgets a pin whose picture ended", () => {
    expect(arrangeStage([screen], [], "gone-s").spotlight).toBe(screen);
  });
});

describe("fullscreen route", () => {
  it("prefers the element where the page may go fullscreen", () => {
    expect(fullscreenRoute(true, { webkitEnterFullscreen() {} })).toBe(
      "element",
    );
  });

  it("uses the video's own player on an iPhone", () => {
    expect(fullscreenRoute(undefined, { webkitEnterFullscreen() {} })).toBe(
      "video",
    );
  });

  it("falls back to the large view where neither exists", () => {
    expect(fullscreenRoute(false, {})).toBe("viewer");
    // A drawn picture of the desktop app has no video element.
    expect(fullscreenRoute(false, null)).toBe("viewer");
  });
});

describe("long press", () => {
  function setup() {
    let pending: (() => void) | null = null;
    const opened = vi.fn();
    const press = createLongPress(opened, {
      set: (run, ms) => {
        expect(ms).toBe(LONG_PRESS_MS);
        pending = run;
        return 1;
      },
      clear: () => {
        pending = null;
      },
    });
    return { press, opened, elapse: () => pending?.() };
  }
  const touch = { pointerType: "touch", clientX: 10, clientY: 10 };

  it("opens after holding a finger still, and the click after it does nothing", () => {
    const { press, opened, elapse } = setup();
    press.down(touch);
    elapse();
    expect(opened).toHaveBeenCalledOnce();
    expect(press.consumeClick()).toBe(true);
    expect(press.consumeClick()).toBe(false);
  });

  it("leaves a tap and a scroll alone", () => {
    const { press, opened, elapse } = setup();
    press.down(touch);
    press.up();
    elapse();
    press.down(touch);
    press.move({ clientX: 10, clientY: 40 });
    elapse();
    expect(opened).not.toHaveBeenCalled();
    expect(press.consumeClick()).toBe(false);
  });

  it("leaves the mouse to its right click", () => {
    const { press, opened, elapse } = setup();
    press.down({ ...touch, pointerType: "mouse" });
    elapse();
    expect(opened).not.toHaveBeenCalled();
  });
});
