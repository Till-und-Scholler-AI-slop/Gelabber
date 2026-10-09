import { describe, expect, it } from "vitest";

import { sizeByDraft } from "./composerHeight.ts";

// chat.css: the field's line, its padding and its limit; then what the form
// adds around it, the pane both share and the history in the list.
const LINE = 22;
const PADDING = 12;
const LIMIT = 160;
const CHROME = 24;
const PANE = 600;
const HISTORY = 3000;

/** A message pane as a browser lays it out: the list gets what the form
 * leaves. Reading a size brings the layout up to date first, and a list that
 * grew past its end is clamped there, as by a real engine. */
function pane({ fieldSizing = false, coarse = true } = {}) {
  const page = {
    /** Lines the draft takes at the field's width. */
    lines: 1,
    width: 246,
    hidden: false,
    coarse,
    /** The list's scrollTop. */
    top: 0,
    /** The shortest the form has been in any layout. */
    shortest: Infinity,
  };
  const px = (value: string) => (value.endsWith("px") ? parseFloat(value) : 0);
  const fieldStyle = { height: "" };
  const formStyle = { minHeight: "" };
  const fieldHeight = () =>
    fieldStyle.height.endsWith("px")
      ? Math.min(LIMIT, Math.max(LINE + PADDING, px(fieldStyle.height)))
      : LINE + PADDING;
  const formHeight = () =>
    Math.max(px(formStyle.minHeight), fieldHeight() + CHROME);
  const viewport = () => PANE - formHeight();
  const layout = () => {
    page.shortest = Math.min(page.shortest, formHeight());
    page.top = Math.min(page.top, HISTORY - viewport());
  };

  const observed: Array<() => void> = [];
  const frames = new Map<number, () => void>();
  let handle = 0;
  const target = {
    CSS: {
      supports: (property: string, value: string) =>
        fieldSizing && property === "field-sizing" && value === "content",
    },
    matchMedia: (query: string) => ({
      get matches() {
        return query === "(pointer: coarse)" && page.coarse;
      },
    }),
    ResizeObserver: class {
      private readonly callback: () => void;
      constructor(callback: () => void) {
        this.callback = callback;
      }
      observe() {
        observed.push(this.callback);
      }
      disconnect() {
        observed.splice(0);
      }
    },
    requestAnimationFrame(callback: () => void) {
      frames.set(++handle, callback);
      return handle;
    },
    cancelAnimationFrame: (id: number) => void frames.delete(id),
  } as unknown as Window & typeof globalThis;
  const field = {
    style: fieldStyle,
    get scrollHeight() {
      layout();
      return page.hidden
        ? 0
        : Math.max(page.lines * LINE + PADDING, fieldHeight());
    },
    get offsetWidth() {
      layout();
      return page.hidden ? 0 : page.width;
    },
  } as unknown as HTMLTextAreaElement;
  const form = {
    style: formStyle,
    getBoundingClientRect() {
      layout();
      return { height: page.hidden ? 0 : formHeight() };
    },
  } as unknown as HTMLElement;

  return {
    page,
    target,
    field,
    form,
    observing: () => observed.length > 0,
    /** The browser reports a new box of the field to its observers. */
    resized() {
      layout();
      for (const callback of [...observed]) callback();
    },
    /** The next frame: what was asked for with requestAnimationFrame runs. */
    frame() {
      const due = [...frames.values()];
      frames.clear();
      for (const callback of due) callback();
      return due.length;
    },
    /** The list scrolled to its newest message, as the pane keeps it. */
    pin() {
      page.top = HISTORY - viewport();
    },
    fromEnd() {
      layout();
      return HISTORY - page.top - viewport();
    },
  };
}

describe("composer height from script", () => {
  it("is the height of the draft", () => {
    const { page, target, field, form } = pane();
    const sizing = sizeByDraft(target, field, form);
    sizing?.fit();
    expect(field.style.height).toBe("34px");
    page.lines = 3;
    sizing?.fit();
    expect(field.style.height).toBe("78px");
    // Taller than the stylesheet's limit: the field scrolls from there.
    page.lines = 9;
    sizing?.fit();
    expect(field.style.height).toBe("210px");
    page.lines = 1;
    sizing?.fit();
    expect(field.style.height).toBe("34px");
  });

  it("measures without moving the message list off its end", () => {
    const view = pane();
    const { page, target, field, form } = view;
    const sizing = sizeByDraft(target, field, form);
    page.lines = 3;
    sizing?.fit();
    view.pin();
    expect(view.fromEnd()).toBe(0);

    // Another letter in the third line. The field collapses to one line for
    // the measurement; a form that followed it would let the list grow by
    // two lines, and the browser would clamp the list 44px before its end.
    page.shortest = Infinity;
    sizing?.fit();
    expect(page.shortest).toBe(78 + CHROME);
    expect(view.fromEnd()).toBe(0);
    expect(form.style.minHeight).toBe("");

    // A draft that got shorter gives the room back to the list.
    page.lines = 1;
    sizing?.fit();
    expect(field.style.height).toBe("34px");
    expect(view.fromEnd()).toBe(0);
    expect(form.style.minHeight).toBe("");
  });

  it("gives the form back the min-height it had", () => {
    const { target, field, form } = pane();
    form.style.minHeight = "40px";
    sizeByDraft(target, field, form)?.fit();
    expect(form.style.minHeight).toBe("40px");
  });

  it("measures again in the frame after the field got another width", () => {
    const view = pane();
    const { page, target, field, form } = view;
    const sizing = sizeByDraft(target, field, form);
    page.lines = 4;
    sizing?.fit();
    expect(field.style.height).toBe("100px");
    // What the observer reports first is the width just measured at.
    view.resized();
    expect(view.frame()).toBe(0);

    // The phone on its side: the same draft in two lines.
    page.width = 372;
    page.lines = 2;
    view.pin();
    view.resized();
    // Not from inside the observer, which would be a ResizeObserver loop.
    expect(field.style.height).toBe("100px");
    expect(view.frame()).toBe(1);
    expect(field.style.height).toBe("56px");
    expect(view.fromEnd()).toBe(0);

    // The field's own new height is reported next: no reason to measure.
    view.resized();
    expect(view.frame()).toBe(0);

    // Two widths before the next frame are one measurement.
    page.width = 300;
    view.resized();
    page.width = 246;
    page.lines = 4;
    view.resized();
    expect(view.frame()).toBe(1);
    expect(field.style.height).toBe("100px");
  });

  it("keeps its height while the composer is hidden", () => {
    const view = pane();
    const { page, target, field, form } = view;
    const sizing = sizeByDraft(target, field, form);
    page.lines = 3;
    sizing?.fit();
    // Search on a short screen takes the composer's place for a while.
    page.hidden = true;
    view.resized();
    expect(view.frame()).toBe(0);
    page.hidden = false;
    view.resized();
    expect(view.frame()).toBe(0);
    expect(field.style.height).toBe("78px");
  });

  it("stops watching and drops a measurement that was still to come", () => {
    const view = pane();
    const { page, target, field, form } = view;
    const sizing = sizeByDraft(target, field, form);
    sizing?.fit();
    expect(view.observing()).toBe(true);
    page.width = 372;
    view.resized();
    sizing?.stop();
    expect(view.observing()).toBe(false);
    expect(view.frame()).toBe(0);
  });

  it("leaves the height to the stylesheet where field-sizing exists", () => {
    const view = pane({ fieldSizing: true });
    expect(sizeByDraft(view.target, view.field, view.form)).toBeNull();
    expect(view.observing()).toBe(false);
  });

  it("leaves the single line of a mouse-driven browser alone", () => {
    const view = pane({ coarse: false });
    const { page, target, field, form } = view;
    const sizing = sizeByDraft(target, field, form);
    page.lines = 3;
    sizing?.fit();
    page.width = 372;
    view.resized();
    expect(view.frame()).toBe(0);
    expect(field.style.height).toBe("");
    expect(form.style.minHeight).toBe("");

    // Asked like the stylesheet's media query: a touch screen that becomes
    // the pointer later gets the draft's height from then on.
    page.coarse = true;
    sizing?.fit();
    expect(field.style.height).toBe("78px");
  });
});
