import { useEffect, useRef, type PointerEvent } from "react";

import { longPressGesture, swallowNextClick } from "./longPress.ts";

export type HoldHandlers = {
  onPointerDown?: (event: PointerEvent<HTMLElement>) => void;
  onPointerMove?: (event: PointerEvent<HTMLElement>) => void;
  onPointerUp?: () => void;
  onPointerCancel?: () => void;
  onPointerLeave?: () => void;
};

/**
 * A finger held on an entry does what a right click does. iOS sends no
 * contextmenu event for it; on Android this and the browser's own event
 * open the same menu, whichever comes first.
 */
export function useHoldMenu(
  open: ((anchor: HTMLElement) => void) | undefined,
): HoldHandlers {
  const latest = useRef(open);
  useEffect(() => {
    latest.current = open;
  });
  // One gesture per touch, made when the finger lands on the entry.
  const press = useRef<ReturnType<typeof longPressGesture> | null>(null);
  // An entry that leaves the list mid-press opens nothing.
  useEffect(() => {
    const gesture = press;
    return () => gesture.current?.end();
  }, []);
  if (!open) return {};
  const end = () => press.current?.end();
  return {
    onPointerDown: (event: PointerEvent<HTMLElement>) => {
      const anchor = event.currentTarget;
      press.current?.end();
      press.current = longPressGesture(() => {
        navigator.vibrate?.(10);
        swallowNextClick();
        latest.current?.(anchor);
      });
      press.current.down({
        pointerType: event.pointerType,
        isPrimary: event.isPrimary,
        x: event.clientX,
        y: event.clientY,
      });
    },
    onPointerMove: (event: PointerEvent<HTMLElement>) =>
      press.current?.move({ x: event.clientX, y: event.clientY }),
    onPointerUp: end,
    onPointerCancel: end,
    onPointerLeave: end,
  };
}
