import { describe, expect, it } from "vitest";

import { inList, scrollsList, type ScrollNode } from "./listInput.ts";

// A tree of plain objects in place of the DOM: node has none.
function tree() {
  const list: ScrollNode = {
    parentElement: null,
    scrollTop: 500,
    scrollHeight: 4000,
    clientHeight: 600,
  };
  const pre: ScrollNode = {
    parentElement: list,
    scrollTop: 0,
    scrollHeight: 900,
    clientHeight: 320,
  };
  const code: ScrollNode = {
    parentElement: pre,
    scrollTop: 0,
    scrollHeight: 0,
    clientHeight: 0,
  };
  const menu: ScrollNode = {
    parentElement: null,
    scrollTop: 0,
    scrollHeight: 0,
    clientHeight: 0,
  };
  const scrollers = new Set([pre]);
  return {
    list,
    pre,
    code,
    menu,
    scroller: (n: ScrollNode) => scrollers.has(n),
  };
}

describe("input on the message list", () => {
  it("counts only what happens in the list's own DOM", () => {
    const { list, code, menu } = tree();
    expect(inList(code, list)).toBe(true);
    // The action menu is portaled out of its row; React still passes its
    // events up through the list.
    expect(inList(menu, list)).toBe(false);
    expect(inList(null, list)).toBe(false);
  });

  it("leaves a wheel to a scroller inside the list while it can move", () => {
    const { list, pre, code, menu, scroller } = tree();
    pre.scrollTop = 200;
    expect(scrollsList(code, list, "up", scroller)).toBe(false);
    expect(scrollsList(code, list, "down", scroller)).toBe(false);
    expect(scrollsList(menu, list, "up", scroller)).toBe(false);
  });

  it("gives the list a wheel that the inner scroller passes on", () => {
    const { list, pre, code, scroller } = tree();
    pre.scrollTop = 0;
    expect(scrollsList(code, list, "up", scroller)).toBe(true);
    pre.scrollTop = pre.scrollHeight - pre.clientHeight;
    expect(scrollsList(code, list, "down", scroller)).toBe(true);
    expect(scrollsList(list, list, "up", scroller)).toBe(true);
  });
});
