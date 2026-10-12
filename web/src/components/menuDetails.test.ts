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

/** An event whose target is a plain object, as `dispatchEvent` would set. */
function fire(on: EventTarget, type: string, target: unknown, extra = {}) {
  const event = Object.assign(new Event(type), extra);
  Object.defineProperty(event, "target", { value: target });
  on.dispatchEvent(event);
}

afterEach(() => vi.unstubAllGlobals());

describe("details menus", () => {
  it("close on Esc, on a press outside and after choosing an entry", () => {
    const document = new EventTarget();
    vi.stubGlobal("document", document);
    const { details, summary, entry } = menu();
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
});
