import { useState } from "react";
import { flushSync } from "react-dom";
import { describe, expect, it } from "vitest";
import { render } from "vitest-browser-react";

import type { Message } from "../messages/types.ts";
import { useMessageScroll } from "./useMessageScroll.ts";

const messages = Array.from(
  { length: 60 },
  (_, i) => ({ id: `m${i}`, created_order: i }) as unknown as Message,
);

// Stable callbacks, as MessagePane's (state setters, useCallback): new ones
// on each render would re-run the hook's layout effects, which record the
// position again, and hide the race these tests are about.
const nothing = () => {};
const rowSize = () => 40;
// What the hook last said about being at the newest row.
const state = { atLatest: null as boolean | null };
const onAtLatest = (value: boolean) => {
  state.atLatest = value;
};

// The message list's scrolling with plain 40 px rows. The spacer below the
// rows, inside the observed content, stands for a row that grew (a picture
// that loaded): it changes the list's height at once.
function List() {
  const [spacer, setSpacer] = useState(0);
  const { scrollRef, contentRef, virtualizer, handlers } = useMessageScroll({
    active: true,
    items: messages,
    newAfter: null,
    hasOlder: false,
    loadingOlder: false,
    onLoadOlder: nothing,
    ready: true,
    loadError: null,
    onAtLatest,
    latestRequest: 0,
    focusRequest: null,
    unreadJump: 0,
    onOpened: nothing,
    estimateSize: rowSize,
  });
  return (
    <div
      ref={scrollRef}
      role="log"
      data-grow={spacer}
      {...handlers}
      style={{ height: 300, overflowY: "auto", overflowAnchor: "none" }}
      onClick={() => flushSync(() => setSpacer((value) => value + 120))}
    >
      <div
        ref={contentRef}
        style={{
          height: virtualizer.getTotalSize() + spacer,
          position: "relative",
        }}
      >
        {virtualizer.getVirtualItems().map((row) => (
          <div
            key={row.key}
            data-index={row.index}
            ref={virtualizer.measureElement}
            style={{ position: "absolute", top: row.start, height: 40 }}
          />
        ))}
      </div>
    </div>
  );
}

const frame = () => new Promise(requestAnimationFrame);
const settled = async () => {
  for (let i = 0; i < 4; i++) await frame();
};
const gap = (el: HTMLElement) =>
  el.scrollHeight - el.scrollTop - el.clientHeight;

async function mount() {
  state.atLatest = null;
  const screen = await render(<List />);
  const list = screen.container.querySelector<HTMLElement>('[role="log"]')!;
  await expect.poll(() => state.atLatest).toBe(true);
  return { state, list };
}

/** The list grows (the spacer) and moves to `top` in one go, before the
 * scroll event: what a row measured during a scroll looks like to it. */
const growAndMove = (list: HTMLElement, top: (el: HTMLElement) => number) => {
  list.click();
  list.scrollTop = top(list);
};

describe("message list: where it stands after a scroll during a size change", () => {
  it("knows it is at its end after a scripted move down to it (GEL-456)", async () => {
    const { state, list } = await mount();
    list.scrollTop = 0;
    await expect.poll(() => state.atLatest).toBe(false);
    growAndMove(list, (el) => el.scrollHeight);
    await settled();
    expect(gap(list)).toBeLessThanOrEqual(1);
    expect(state.atLatest).toBe(true);
  });

  it("stays where a scripted move up put it, and the next growth does not pull it back (GEL-461)", async () => {
    const { state, list } = await mount();
    growAndMove(list, (el) => el.scrollHeight - el.clientHeight - 400);
    await settled();
    list.click();
    await settled();
    expect(gap(list)).toBeGreaterThan(150);
    expect(state.atLatest).toBe(false);
  });

  it("takes a move with the scrollbar as the reader's: down near the end pins it", async () => {
    const { state, list } = await mount();
    list.scrollTop = 0;
    await expect.poll(() => state.atLatest).toBe(false);
    // A press on the list itself, not a row: its scrollbar.
    list.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
    growAndMove(list, (el) => el.scrollHeight - el.clientHeight - 60);
    await settled();
    window.dispatchEvent(new PointerEvent("pointerup", { bubbles: true }));
    await settled();
    // Pinned: the next growth keeps it at the end, and the newest is read.
    list.click();
    await settled();
    expect(gap(list)).toBeLessThanOrEqual(1);
    expect(state.atLatest).toBe(true);
  });

  it("keeps a reader at the end there when the list grows without a move", async () => {
    const { state, list } = await mount();
    list.click();
    await settled();
    expect(gap(list)).toBeLessThanOrEqual(1);
    expect(state.atLatest).toBe(true);
  });
});
