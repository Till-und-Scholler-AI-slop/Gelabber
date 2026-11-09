// What opens an entry's menu (RowMenu.tsx) besides its "…" button: a right
// click, a held finger (useHoldMenu) and, from the keyboard, Umschalt+F10
// or the context-menu key, as on the desktop.
import { useRef, useState, type KeyboardEvent, type MouseEvent } from "react";

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
 * The right click and the menu keys anywhere in a row, on its link and on
 * its buttons alike: the menu hangs on the control they came from, which
 * gets the focus back. `onKeyDown` takes every other key. Events from a
 * menu drawn outside the row (a portal) are not the row's.
 */
export function rowMenuKeys(
  open: (anchor: HTMLElement) => void,
  onKeyDown?: (event: KeyboardEvent<HTMLElement>) => void,
) {
  const anchorOf = (event: {
    target: EventTarget;
    currentTarget: HTMLElement;
  }) => {
    const row = event.currentTarget;
    const target = event.target as HTMLElement;
    if (!row.contains(target)) return null;
    const control = target.closest<HTMLElement>("a, button");
    return control && row.contains(control) ? control : row;
  };
  return {
    onContextMenu: (event: MouseEvent<HTMLElement>) => {
      const anchor = anchorOf(event);
      if (!anchor) return;
      event.preventDefault();
      open(anchor);
    },
    onKeyDown: (event: KeyboardEvent<HTMLElement>) => {
      const anchor = isMenuKey(event) ? anchorOf(event) : null;
      if (!anchor) return onKeyDown?.(event);
      event.preventDefault();
      open(anchor);
    },
  };
}

/**
 * One entry's menu: whether it is open, where it hangs, and `trigger` to
 * spread on the entry (right click, long press, keys), or `hold` alone
 * when `rowMenuKeys` covers the row. `show` opens it at another element,
 * such as the "…" button. `onOpen` runs as it opens.
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
    hold,
    // Spelled out, not `menuKeys(show)`: handing `show` (which sets the
    // anchor ref) to a function during render trips react-hooks/refs.
    trigger: {
      onContextMenu: (event: MouseEvent<HTMLElement>) => {
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
