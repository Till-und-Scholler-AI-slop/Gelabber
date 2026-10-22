// The message list's scrolling: pinned to the newest row until the reader
// scrolls away, opening at the first unread row, keeping the row on top in
// place while older pages or deletes change the rows above it, and asking
// for older pages near the top.

import { useVirtualizer } from "@tanstack/react-virtual";
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type PointerEvent,
  type TouchEvent,
  type UIEvent,
  type WheelEvent,
} from "react";

import {
  endDistance,
  scrollEndIntent,
  type ScrollPosition,
} from "../messages/scrollPosition.ts";
import {
  UNREAD_SEEK_PAGES,
  unreadAnchor,
  type ReadMark,
} from "../messages/pages.ts";
import type { Message } from "../messages/types.ts";
import { anchoredScrollTop } from "./scrollAnchor.ts";

export function useMessageScroll({
  active,
  items,
  newAfter,
  hasOlder,
  loadingOlder,
  onLoadOlder,
  ready,
  loadError,
  onAtLatest,
  latestRequest,
  focusRequest,
  unreadJump,
  onOpened,
  estimateSize,
}: {
  active: boolean;
  items: Message[];
  newAfter: ReadMark | null | undefined;
  hasOlder: boolean;
  loadingOlder: boolean;
  onLoadOlder: () => void;
  ready: boolean;
  loadError: "history" | "paging" | null;
  onAtLatest: (value: boolean) => void;
  latestRequest: number;
  focusRequest: { id: string; n: number } | null;
  unreadJump: number;
  onOpened: (atUnread: boolean) => void;
  /** A row's height before it is measured. */
  estimateSize: (index: number) => number;
}) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const stickToBottom = useRef(true);
  const contentRef = useRef<HTMLDivElement>(null);
  const lastPosition = useRef<ScrollPosition | null>(null);
  const touchY = useRef<number | null>(null);
  const lastRequest = useRef(latestRequest);
  // Older history was asked for; its rows are prepended above `pin`.
  const olderAnchor = useRef(false);
  const lastCount = useRef(0);
  const lastFirst = useRef<string | undefined>(undefined);
  // The rows the virtualizer measured, as committed (not as rendering).
  const itemsRef = useRef(items);
  const pin = useRef<{ id: string; offset: number } | null>(null);
  // The row a prepend or trim must keep in place, held until the reader
  // scrolls: the new rows above it are measured (and grow) in later commits.
  // Not `pin`, which every scroll and layout pass rewrites.
  const hold = useRef<{ id: string; offset: number } | null>(null);
  // Where the held anchor put the list; a scroll elsewhere is the reader's.
  const heldTop = useRef<number | null>(null);
  // Whether the browser put the list exactly at the held position.
  const heldLanded = useRef(false);
  // With unread messages the list opens at the first of them, which may
  // take older pages first. Until it settled it is not at the newest, so
  // nothing is marked read. `pending` waits for the read state.
  const opening = useRef<"pending" | "seek" | "done">("pending");
  const seekPages = useRef(0);
  const seekBudget = useRef(UNREAD_SEEK_PAGES);
  const lastJump = useRef(unreadJump);

  // Not on the React Compiler; the warning is about memoising its return value.
  // eslint-disable-next-line react-hooks/incompatible-library
  const virtualizer = useVirtualizer({
    count: items.length,
    getScrollElement: () => scrollRef.current,
    estimateSize,
    getItemKey: (index) => items[index]?.id ?? index,
    overscan: 12,
  });

  useLayoutEffect(() => {
    itemsRef.current = items;
  }, [items]);

  // `pinEnd` is for size changes only. Scroll handlers just read: writing
  // scrollTop while the user scrolls cuts WebKit's momentum scrolling.
  const updateLatest = useCallback(
    (pinEnd = true) => {
      const element = scrollRef.current;
      if (!element) return;
      if (pinEnd && stickToBottom.current)
        element.scrollTop = element.scrollHeight;
      const position = {
        top: element.scrollTop,
        height: element.scrollHeight,
        viewport: element.clientHeight,
      };
      lastPosition.current = position;
      // The row at the top and its offset as of the last scroll or resize:
      // what a prepend or a vanished row must not move. From the
      // measurements, not the rendered range: this runs before the
      // virtualizer's own scroll listener, and after a long jump that range
      // still describes the old position.
      const first = virtualizer.getVirtualItemForOffset(element.scrollTop);
      const firstId = first ? itemsRef.current[first.index]?.id : undefined;
      if (first && firstId)
        pin.current = { id: firstId, offset: first.start - element.scrollTop };
      onAtLatest(
        active &&
          opening.current === "done" &&
          stickToBottom.current &&
          position.viewport > 16 &&
          endDistance(position) <= 16,
      );
    },
    [active, onAtLatest, virtualizer],
  );

  useLayoutEffect(() => {
    const element = scrollRef.current,
      content = contentRef.current;
    if (!element || !content) return;
    // The viewport also changes when the keyboard model, dock, or search
    // controls resize. Keep history intent even if the browser clamps to end.
    const observer = new ResizeObserver(() => updateLatest());
    observer.observe(element);
    observer.observe(content);
    updateLatest();
    return () => observer.disconnect();
  }, [updateLatest]);

  useLayoutEffect(() => {
    // Preserve history anchors; while pinned, the end owns resize adjustments.
    virtualizer.shouldAdjustScrollPositionOnItemSizeChange = (
      item,
      _delta,
      instance,
    ) =>
      !stickToBottom.current &&
      // A held anchor sets the position itself, absolutely.
      !hold.current &&
      item.start < (instance.scrollOffset ?? 0);
  }, [virtualizer]);

  const totalSize = virtualizer.getTotalSize();
  // Fresh and unclamped: getTotalSize() recomputes the measurements from
  // the sizes measured so far. getOffsetForIndex() would clamp to a scroll
  // height the list has not grown to yet, and so jump (#201).
  const startOf = useCallback(
    (index: number) => {
      virtualizer.getTotalSize();
      return virtualizer.measurementsCache[index]?.start;
    },
    [virtualizer],
  );
  // The reader moved the list: from here on the position is theirs.
  const release = () => {
    hold.current = null;
    heldTop.current = null;
    opening.current = "done";
  };
  // Any difference from where the hold put the list, unless the browser
  // clamped that position: the rows above were not laid out yet, and the
  // browser moves a clamped scrollTop on its own as they are.
  const movedByReader = (top: number) =>
    heldTop.current !== null && heldLanded.current && top !== heldTop.current;
  useLayoutEffect(() => {
    if (latestRequest !== lastRequest.current) {
      stickToBottom.current = true;
      lastRequest.current = latestRequest;
      opening.current = "done";
    }
    if (unreadJump !== lastJump.current) {
      lastJump.current = unreadJump;
      if (newAfter) {
        opening.current = "seek";
        seekBudget.current = seekPages.current + UNREAD_SEEK_PAGES;
      }
    }
    if (opening.current === "pending" && ready && newAfter !== undefined)
      opening.current = newAfter ? "seek" : "done";
    if (opening.current === "seek" && loadError) opening.current = "done";
    // Where the first unread row goes, once it is loaded (or the pages to
    // look for it are used up: then the oldest loaded one).
    let unreadTop: { id: string; offset: number } | null = null;
    if (opening.current === "seek") {
      const anchor = unreadAnchor(items, newAfter ?? null, hasOlder);
      const settled =
        anchor.kind !== "older" || seekPages.current >= seekBudget.current;
      if (settled) {
        opening.current = "done";
        const index =
          anchor.kind === "row"
            ? anchor.index
            : anchor.kind === "older"
              ? 0
              : -1;
        const start = index >= 0 ? startOf(index) : undefined;
        const viewport = scrollRef.current?.clientHeight ?? 0;
        // Unread rows that fit below the top third stay at the end, where
        // they are all in view and get marked read.
        if (
          start !== undefined &&
          virtualizer.getTotalSize() - start > (viewport * 2) / 3
        ) {
          stickToBottom.current = false;
          // Row offsets count from the rows' box, below the paging line.
          const above = contentRef.current?.offsetTop ?? 0;
          unreadTop = {
            id: items[index]!.id,
            offset: Math.round(viewport / 3) - above,
          };
        }
        onOpened(unreadTop !== null);
      }
    }
    // The reader moved the list since the hold was applied, possibly before
    // the scroll event fired: even one pixel is theirs.
    if (scrollRef.current && movedByReader(scrollRef.current.scrollTop))
      release();
    const prepended = olderAnchor.current && items.length > lastCount.current;
    const firstChanged =
      lastFirst.current !== undefined && items[0]?.id !== lastFirst.current;

    if (prepended) {
      // Keep the row that was on top at the same place, offset included,
      // also while the new rows above it are measured in later commits.
      olderAnchor.current = false;
      hold.current = pin.current;
    } else if (stickToBottom.current) {
      // Native bottom pin below owns the end. scrollToIndex's asynchronous
      // measurement retries can otherwise pull a measured list back upwards.
      hold.current = null;
      heldTop.current = null;
    } else if (
      (items.length < lastCount.current || firstChanged) &&
      pin.current
    ) {
      // A row vanished (own delete or WS), or the oldest rows moved out of
      // the list. Keep the first visible id at the same offset so the list
      // does not jump.
      hold.current = pin.current;
    }
    if (unreadTop) hold.current = unreadTop;
    if (hold.current && scrollRef.current) {
      const top = anchoredScrollTop(items, hold.current, startOf);
      if (top === undefined) {
        // The row itself is gone: nothing left to hold.
        hold.current = null;
        heldTop.current = null;
      } else {
        scrollRef.current.scrollTop = top;
        heldTop.current = scrollRef.current.scrollTop;
        heldLanded.current = Math.abs(heldTop.current - top) < 1;
      }
    }

    updateLatest();
    lastCount.current = items.length;
    lastFirst.current = items[0]?.id;
    // `updateLatest()` pinned the row at scrollTop. Not the first rendered
    // row: that can be an overscan row far above the viewport.
  }, [
    items,
    virtualizer,
    startOf,
    updateLatest,
    totalSize,
    latestRequest,
    unreadJump,
    newAfter,
    ready,
    hasOlder,
    loadError,
    onOpened,
  ]);

  // Older pages until the last read message is loaded, also when
  // "Springen" adds pages to look through.
  useEffect(() => {
    if (
      opening.current !== "seek" ||
      !ready ||
      !hasOlder ||
      loadingOlder ||
      loadError ||
      seekPages.current >= seekBudget.current ||
      unreadAnchor(items, newAfter ?? null, hasOlder).kind !== "older"
    )
      return;
    seekPages.current += 1;
    onLoadOlder();
  }, [
    items,
    newAfter,
    hasOlder,
    loadingOlder,
    loadError,
    ready,
    onLoadOlder,
    unreadJump,
  ]);

  const [highlighted, setHighlighted] = useState<string | null>(null);
  const handledFocus = useRef(focusRequest?.n);
  useEffect(() => {
    if (!focusRequest || focusRequest.n === handledFocus.current) return;
    handledFocus.current = focusRequest.n;
    const index = itemsRef.current.findIndex(
      (message) => message.id === focusRequest.id,
    );
    if (index < 0) return;
    // A jump into history is the reader's position, not the end's.
    stickToBottom.current = false;
    hold.current = null;
    heldTop.current = null;
    opening.current = "done";
    virtualizer.scrollToIndex(index, { align: "center" });
    setHighlighted(focusRequest.id);
    const done = window.setTimeout(() => setHighlighted(null), 2000);
    return () => window.clearTimeout(done);
  }, [focusRequest, virtualizer]);

  const firstVisible = virtualizer.getVirtualItems()[0]?.index ?? 0;
  const firstId = items[0]?.id;
  useEffect(() => {
    if (!active || !ready || !hasOlder || loadingOlder || loadError) return;
    // First paint is pinned to the newest row; do not walk older pages
    // until the user actually scrolls up.
    if (stickToBottom.current) return;
    if (firstVisible > 4) return;
    olderAnchor.current = true;
    onLoadOlder();
  }, [
    active,
    firstVisible,
    firstId,
    hasOlder,
    loadingOlder,
    onLoadOlder,
    ready,
    loadError,
  ]);

  return {
    scrollRef,
    contentRef,
    virtualizer,
    highlighted,
    /** The list's own listeners: what the reader does decides between the
     * newest row and a place in the history. */
    handlers: {
      onWheelCapture: (event: WheelEvent<HTMLDivElement>) => {
        if (
          event.target instanceof Element &&
          event.target.closest("dialog:modal")
        )
          return;
        release();
        if (
          event.deltaY < 0 &&
          event.currentTarget.scrollHeight >
            event.currentTarget.clientHeight + 16
        )
          stickToBottom.current = false;
        else if (
          event.deltaY > 0 &&
          lastPosition.current &&
          endDistance(lastPosition.current) < 96
        )
          stickToBottom.current = true;
        updateLatest(false);
      },
      onKeyDownCapture: (event: KeyboardEvent<HTMLDivElement>) => {
        if (
          event.target instanceof HTMLElement &&
          event.target.closest(
            "dialog:modal,input,textarea,select,[contenteditable='true']",
          )
        )
          return;
        release();
        if (
          ["ArrowUp", "PageUp", "Home"].includes(event.key) &&
          event.currentTarget.scrollHeight >
            event.currentTarget.clientHeight + 16
        )
          stickToBottom.current = false;
        if (event.key === "End") stickToBottom.current = true;
        updateLatest(false);
      },
      onTouchStartCapture: (event: TouchEvent<HTMLDivElement>) => {
        if (
          event.target instanceof Element &&
          event.target.closest("dialog:modal")
        )
          return;
        touchY.current = event.touches[0]?.clientY ?? null;
      },
      onTouchMoveCapture: (event: TouchEvent<HTMLDivElement>) => {
        if (
          event.target instanceof Element &&
          event.target.closest("dialog:modal")
        )
          return;
        const current = event.touches[0]?.clientY;
        if (current === undefined || touchY.current === null) return;
        release();
        if (
          current > touchY.current &&
          event.currentTarget.scrollHeight >
            event.currentTarget.clientHeight + 16
        )
          stickToBottom.current = false;
        else if (
          current < touchY.current &&
          lastPosition.current &&
          endDistance(lastPosition.current) < 96
        )
          stickToBottom.current = true;
        touchY.current = current;
        updateLatest(false);
      },
      onTouchEndCapture: () => {
        touchY.current = null;
      },
      onPointerDownCapture: (event: PointerEvent<HTMLDivElement>) => {
        // On the list itself, not a row: its scrollbar.
        if (event.target === event.currentTarget) release();
      },
      onScrollCapture: (event: UIEvent<HTMLDivElement>) => {
        if (event.target !== event.currentTarget) return;
        const element = scrollRef.current;
        if (!element) return;
        // Momentum or a scrollbar drag moved it away from the held anchor.
        if (movedByReader(element.scrollTop)) release();
        // Observe intent before virtualizer measurement adjusts the geometry.
        stickToBottom.current = scrollEndIntent(
          stickToBottom.current,
          lastPosition.current,
          {
            top: element.scrollTop,
            height: element.scrollHeight,
            viewport: element.clientHeight,
          },
        );
        updateLatest(false);
      },
    },
  };
}
