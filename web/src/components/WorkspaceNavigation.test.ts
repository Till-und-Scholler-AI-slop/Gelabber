import { describe, expect, it } from "vitest";

import { swiped, TRAVEL_PX } from "./edgeSwipe.ts";
import {
  closedState,
  closeStep,
  drawerOver,
  openedState,
} from "./navigationDrawer.ts";

// The phone drawer is open while the history entry says so. Node has no
// DOM; these follow the entries a tap on a rail tile or a channel makes.
describe("phone navigation drawer", () => {
  // As the router keeps it: its own keys next to the app's.
  const router = JSON.parse('{"__TSR_index":3,"__TSR_key":"k3","key":"k3"}');

  it("opens in an entry of its own over the current address", () => {
    const opened = openedState(router, "/s/a/c/1");
    expect(drawerOver(opened)).toBe("/s/a/c/1");
    expect(opened).not.toHaveProperty("__TSR_key");
    expect(closeStep(opened, "/s/a/c/1")).toBe("back");
  });

  it("stays open when a rail tile is tapped", () => {
    const opened = openedState(router, "/s/a/c/1");
    // The tile replaces the entry and carries the drawer along.
    const onServer = { navigationDrawer: drawerOver(opened) };
    expect(drawerOver(onServer)).toBe("/s/a/c/1");
    // Closing then keeps the server that was picked.
    expect(closeStep(onServer, "/s/b")).toBe("replace");
    expect(drawerOver(closedState(onServer))).toBeUndefined();
  });

  it("closes when a channel is tapped", () => {
    // A channel link pushes a plain entry on top of the drawer's one, so
    // back from the conversation opens the list again.
    expect(drawerOver({})).toBeUndefined();
    expect(closeStep({}, "/s/a/c/2")).toBeNull();
  });
});

describe("drawer swipes", () => {
  const start = { x: 4, y: 300 };
  it("open from the edge after a sideways travel", () => {
    expect(swiped(start, { x: 4 + TRAVEL_PX + 1, y: 310 }, "right")).toBe(true);
    expect(swiped(start, { x: 4 + TRAVEL_PX - 1, y: 300 }, "right")).toBe(
      false,
    );
  });

  it("leave vertical scrolling alone", () => {
    expect(swiped(start, { x: 90, y: 400 }, "right")).toBe(false);
  });

  it("close to the left only", () => {
    expect(swiped({ x: 200, y: 300 }, { x: 120, y: 290 }, "left")).toBe(true);
    expect(swiped({ x: 200, y: 300 }, { x: 280, y: 300 }, "left")).toBe(false);
  });
});
