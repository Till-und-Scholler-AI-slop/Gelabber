// What opens an entry's menu (RowMenu.tsx) besides its "…" button: a right
// click, a held finger (useHoldMenu) and, from the keyboard, Umschalt+F10
// or the context-menu key, as on the desktop.
import { useRef, useState, type KeyboardEvent } from "react";

import { useHoldMenu } from "./useHoldMenu.ts";

/** Whether a key press asks for the focused entry's menu. */
export function isMenuKey(event: { key: string; shiftKey: boolean }): boolean {
  return event.key === "ContextMenu" || (event.key === "F10" && event.shiftKey);
}

/** The right click and the menu keys on an entry, as event handlers. */
export function menuKeys(open: (anchor: HTMLElement) => void) {
  return {
    onContextMenu: (event: {
      preventDefault: () => void;
      currentTarget: HTMLElement;
    }) => {
      event.preventDefault();
      open(event.currentTarget);
    },
    onKeyDown: (event: KeyboardEvent<HTMLElement>) => {
      if (!isMenuKey(event)) return;
      event.preventDefault();
      open(event.currentTarget);
    },
  };
}

/**
 * One entry's menu: whether it is open, where it hangs, and `trigger` to
 * spread on the entry (right click, long press, keys). `show` opens it at
 * another element, such as the "…" button. `onOpen` runs as it opens.
 */
export function useRowMenu(onOpen?: () => void) {
  const [open, setOpen] = useState(false);
  const anchor = useRef<HTMLElement | null>(null);
  const show = (element: HTMLElement) => {
    anchor.current = element;
    onOpen?.();
    setOpen(true);
  };
  const hold = useHoldMenu(show);
  return {
    open,
    anchor,
    show,
    close: () => setOpen(false),
    trigger: {
      onContextMenu: (event: {
        preventDefault: () => void;
        currentTarget: HTMLElement;
      }) => {
        event.preventDefault();
        show(event.currentTarget);
      },
      onKeyDown: (event: KeyboardEvent<HTMLElement>) => {
        if (!isMenuKey(event)) return;
        event.preventDefault();
        show(event.currentTarget);
      },
      ...hold,
    },
  };
}
