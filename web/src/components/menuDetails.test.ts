import { afterEach, describe, expect, it, vi } from "vitest";

import { attachMenuDetails } from "./menuDetails.ts";

/** Just enough of a `<details>` menu: no DOM in this test environment. */
function menu() {
  const summary = { focus: vi.fn(), closest: () => summary };
  const entry = {
    closest: (sel: string) => (sel === "summary" ? null : entry),
  };
  const inside = new Set<unknown>([summary, entry]);
  const details = Object.assign(new EventTarget(), {
    open: true,
    removeAttribute(name: string) {
      if (name === "open") this.open = false;
    },
    contains: (node: unknown) => node === details || inside.has(node),
    querySelector: () => summary,
  });
  return { details, summary, entry };
}

/** A stand-in `document` whose focused element the test sets. */
function stubDocument(activeElement: unknown) {
  const document = Object.assign(new EventTarget(), { activeElement });
  vi.stubGlobal("document", document);
  return document;
}

/** An event whose target is a plain object, as `dispatchEvent` would set. */
function fire(on: EventTarget, type: string, target: unknown, extra = {}) {
  const event = Object.assign(new Event(type, { cancelable: true }), extra);
  Object.defineProperty(event, "target", { value: target });
  on.dispatchEvent(event);
  return event;
}

afterEach(() => vi.unstubAllGlobals());

describe("details menus", () => {
  it("close on Esc, on a press outside and after choosing an entry", () => {
    const { details, summary, entry } = menu();
    const document = stubDocument(entry);
    const detach = attachMenuDetails(details as unknown as HTMLDetailsElement);

    fire(document, "pointerdown", summary);
    expect(details.open).toBe(true);
    fire(document, "keydown", null, { key: "Escape" });
    expect(details.open).toBe(false);
    expect(summary.focus).toHaveBeenCalledOnce();

    details.open = true;
    fire(document, "pointerdown", { outside: true });
    expect(details.open).toBe(false);

    details.open = true;
    fire(details, "click", summary);
    expect(details.open).toBe(true);
    fire(details, "click", entry);
    expect(details.open).toBe(false);

    detach();
    details.open = true;
    fire(document, "keydown", null, { key: "Escape" });
    expect(details.open).toBe(true);
  });

  it("take Esc only while open and refocus only from inside", () => {
    const { details, summary, entry } = menu();
    const document = stubDocument({ elsewhere: true });
    attachMenuDetails(details as unknown as HTMLDetailsElement);
    const esc = () => fire(document, "keydown", null, { key: "Escape" });

    // Focus outside the menu: it closes, the focus stays where it was.
    expect(esc().defaultPrevented).toBe(true);
    expect(details.open).toBe(false);
    expect(summary.focus).not.toHaveBeenCalled();

    // Closed: this Esc belongs to someone else.
    expect(esc().defaultPrevented).toBe(false);

    details.open = true;
    document.activeElement = entry;
    expect(esc().defaultPrevented).toBe(true);
    expect(details.open).toBe(false);
    expect(summary.focus).toHaveBeenCalledOnce();
  });

  it("close when focus leaves the menu", () => {
    const { details, entry } = menu();
    stubDocument(entry);
    attachMenuDetails(details as unknown as HTMLDetailsElement);
    const focusOut = (relatedTarget: unknown) =>
      fire(details, "focusout", entry, { relatedTarget });

    focusOut(entry);
    expect(details.open).toBe(true);
    // Focus leaving the page entirely is the pointer handler's business.
    focusOut(null);
    expect(details.open).toBe(true);
    focusOut({ outside: true });
    expect(details.open).toBe(false);
  });
});
