import { describe, expect, it, vi } from "vitest";

import {
  claimTooltip,
  releaseTooltip,
  tooltipEscape,
  tooltipHost,
} from "./tooltipState.ts";

type Box = { name: string; contains(node: unknown): boolean };

/** A control, perhaps in a dialog. */
function control(dialog: Box | null) {
  return { closest: () => dialog };
}
const page: Box = { name: "page", contains: () => true };

describe("tooltipHost", () => {
  it("draws inside the modal dialog the control sits in", () => {
    const dialog: Box = { name: "dialog", contains: () => true };
    const fullscreen: Box = { name: "fullscreen", contains: () => true };
    expect(tooltipHost(control(dialog), fullscreen, page)).toBe(dialog);
  });

  it("draws inside the fullscreen element that holds the control", () => {
    const anchor = control(null);
    const fullscreen: Box = {
      name: "fullscreen",
      contains: (node) => node === anchor,
    };
    expect(tooltipHost(anchor, fullscreen, page)).toBe(fullscreen);
    // Something else in fullscreen: the control is not seen there.
    expect(tooltipHost(control(null), fullscreen, page)).toBe(page);
  });

  it("draws on the page otherwise", () => {
    expect(tooltipHost(control(null), null, page)).toBe(page);
  });
});

function escape(key = "Escape", defaultPrevented = false) {
  const event = { key, defaultPrevented, preventDefault: vi.fn() };
  return event;
}

describe("tooltipEscape", () => {
  it("uses up the Esc of a tooltip shown for the keyboard focus", () => {
    const event = escape();
    expect(tooltipEscape(event, true)).toBe(true);
    expect(event.preventDefault).toHaveBeenCalledOnce();
  });

  it("lets the Esc under a resting pointer go on (als gelesen markieren)", () => {
    const event = escape();
    expect(tooltipEscape(event, false)).toBe(true);
    expect(event.preventDefault).not.toHaveBeenCalled();
  });

  it("goes along with an Esc a popover already took", () => {
    const event = escape("Escape", true);
    expect(tooltipEscape(event, true)).toBe(true);
    expect(event.preventDefault).not.toHaveBeenCalled();
  });

  it("stays for other keys", () => {
    const event = escape("m");
    expect(tooltipEscape(event, true)).toBe(false);
    expect(event.preventDefault).not.toHaveBeenCalled();
  });
});

describe("claimTooltip", () => {
  it("puts the tooltip up elsewhere away at once", () => {
    const mic = vi.fn();
    const headset = vi.fn();
    claimTooltip(mic);
    claimTooltip(mic);
    expect(mic).not.toHaveBeenCalled();
    claimTooltip(headset);
    expect(mic).toHaveBeenCalledOnce();
    // Gone by itself: the next one finds nothing to put away.
    releaseTooltip(headset);
    claimTooltip(mic);
    expect(headset).not.toHaveBeenCalled();
    releaseTooltip(mic);
  });

  it("does not forget the current one for a release of another", () => {
    const mic = vi.fn();
    const headset = vi.fn();
    claimTooltip(mic);
    releaseTooltip(headset);
    claimTooltip(headset);
    expect(mic).toHaveBeenCalledOnce();
    releaseTooltip(headset);
  });
});
