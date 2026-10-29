import { describe, expect, it, vi } from "vitest";

import {
  arrangeStage,
  createLongPress,
  fitGrid,
  focusOf,
  focusRects,
  fullscreenRoute,
  LONG_PRESS_MS,
  rasterOrder,
  rasterRects,
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

describe("raster", () => {
  it("puts a pinned picture first, then the Lives, then the cameras", () => {
    expect(rasterOrder([live, screen], [camera], null)).toEqual([
      live,
      screen,
      camera,
    ]);
    expect(rasterOrder([live, screen], [camera], "rafi-v")).toEqual([
      camera,
      live,
      screen,
    ]);
    expect(rasterOrder([live], [camera], "gone-s")).toEqual([live, camera]);
  });

  // The stage of a 1280x800 window with both sidebars, the member list
  // closed: 944 wide, 544 high under the facts and over the people.
  const W = 944;
  const H = 544;
  const shape = (count: number, width = W, height = H) => {
    const fit = fitGrid(count, width, height, 12);
    return fit && `${fit.columns}x${fit.rows} ${fit.width}x${fit.height}`;
  };

  it("gives one camera the stage, two side by side", () => {
    expect(shape(1)).toBe("1x1 944x544");
    // Square at most: the stage is taller than half its width.
    expect(shape(2)).toBe("2x1 466x466");
  });

  it("goes 2x2 for three and four, 3x2 for five and six", () => {
    expect(shape(3)).toBe("2x2 466x266");
    expect(shape(4)).toBe("2x2 466x266");
    expect(shape(5)).toBe("3x2 306x266");
    expect(shape(6)).toBe("3x2 306x266");
  });

  it("keeps a tile between square and 16:9", () => {
    // A wide, low stage: a 16:9 tile, not a letterbox.
    expect(shape(1, 1600, 300)).toBe("1x1 533x300");
    // A narrow, tall one: square, not a column.
    expect(shape(1, 300, 900)).toBe("1x1 300x300");
    // Two on a tall stage stack instead of going thin.
    expect(shape(2, 500, 900)).toBe("1x2 500x444");
  });

  it("has no grid without pictures or room", () => {
    expect(fitGrid(0, W, H, 12)).toBeNull();
    expect(fitGrid(2, 0, H, 12)).toBeNull();
    expect(fitGrid(2, W, Number.NaN, 12)).toBeNull();
    // Three tiles and their gaps do not fit in 10 pixels.
    expect(fitGrid(3, 10, 10, 12)).toBeNull();
  });
});

describe("raster rects", () => {
  it("centres the grid and a short last row", () => {
    expect(rasterRects(2, 944, 544)).toEqual([
      { x: 0, y: 39, width: 466, height: 466 },
      { x: 478, y: 39, width: 466, height: 466 },
    ]);
    // 2x2 with one tile in the last row, under the middle.
    const [, , last] = rasterRects(3, 944, 544);
    expect(last).toEqual({ x: 239, y: 278, width: 466, height: 266 });
  });

  it("is empty before the stage has a size", () => {
    expect(rasterRects(2, 0, 0)).toEqual([]);
  });
});

describe("fokus", () => {
  type Tile = { id: string; kind: StageKind; userId: string };
  const mine: Tile = { id: "me-s", kind: "live", userId: "me" };
  const theirs: Tile = { id: "mia-s", kind: "live", userId: "mia" };
  const face: Tile = { id: "kim-v", kind: "camera", userId: "kim" };

  it("fills the stage with the pinned picture, else someone else's Live", () => {
    expect(focusOf([mine, theirs], [face], "kim-v", "me")).toBe(face);
    expect(focusOf([mine, theirs], [face], null, "me")).toBe(theirs);
    expect(focusOf([mine], [face], null, "me")).toBe(mine);
    expect(focusOf([], [face], "gone-v", "me")).toBe(face);
    const own: Tile = { id: "me-v", kind: "camera", userId: "me" };
    expect(focusOf([], [own, face], null, "me")).toBe(face);
    expect(focusOf([], [own], null, "me")).toBe(own);
    expect(focusOf([], [], null, "me")).toBeNull();
  });

  it("puts a 16:9 picture over a row of 171x96 tiles", () => {
    // The stage of a 1280x800 window in Fokus: no chips under it.
    expect(focusRects(3, 944, 612)).toEqual({
      focus: { x: 24, y: 0, width: 896, height: 504 },
      strip: [
        { x: 206, y: 516, width: 171, height: 96 },
        { x: 387, y: 516, width: 171, height: 96 },
        { x: 568, y: 516, width: 171, height: 96 },
      ],
    });
  });

  it("gives a lone picture the whole stage and narrows a long row", () => {
    expect(focusRects(0, 944, 612)?.focus).toEqual({
      x: 0,
      y: 41,
      width: 944,
      height: 531,
    });
    const row = focusRects(8, 944, 612)!.strip;
    expect(row[0]).toEqual({ x: 1, y: 516, width: 109, height: 96 });
    expect(row[7]!.x + row[7]!.width).toBeLessThanOrEqual(944);
    expect(focusRects(2, 0, 0)).toBeNull();
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
