// Phones: a swipe in from the left edge opens the navigation drawer, a swipe
// to the left on the open drawer closes it.

import { useEffect, useRef } from "react";

/** A swipe that opens has to start this close to the screen's left edge. */
export const EDGE_PX = 20;
/** And travel this far sideways. */
export const TRAVEL_PX = 60;

type Point = { x: number; y: number };

/** Whether the finger went far enough, and more sideways than up or down. */
export function swiped(
  start: Point,
  now: Point,
  direction: "right" | "left",
): boolean {
  const dx = now.x - start.x;
  const dy = now.y - start.y;
  if (Math.abs(dy) >= Math.abs(dx)) return false;
  return direction === "right" ? dx > TRAVEL_PX : dx < -TRAVEL_PX;
}

/**
 * Listens while `enabled` (the drawer layout): a touch from the edge calls
 * `onOpen`, a leftward one that starts inside `drawerSelector` `onClose`.
 */
export function useEdgeSwipe({
  enabled,
  open,
  drawerSelector,
  onOpen,
  onClose,
}: {
  enabled: boolean;
  open: boolean;
  drawerSelector: string;
  onOpen: () => void;
  onClose: () => void;
}) {
  const latest = useRef({ open, onOpen, onClose });
  useEffect(() => {
    latest.current = { open, onOpen, onClose };
  });
  useEffect(() => {
    if (!enabled) return;
    let gesture: { start: Point; direction: "right" | "left" } | null = null;
    const onStart = (event: TouchEvent) => {
      gesture = null;
      const touch = event.touches[0];
      if (event.touches.length !== 1 || !touch) return;
      const start = { x: touch.clientX, y: touch.clientY };
      const target = event.target instanceof Element ? event.target : null;
      if (latest.current.open) {
        if (target?.closest(drawerSelector))
          gesture = { start, direction: "left" };
      } else if (start.x <= EDGE_PX && !target?.closest("dialog")) {
        gesture = { start, direction: "right" };
      }
    };
    const onMove = (event: TouchEvent) => {
      const touch = event.touches[0];
      if (!gesture || !touch) return;
      const now = { x: touch.clientX, y: touch.clientY };
      if (!swiped(gesture.start, now, gesture.direction)) return;
      const opening = gesture.direction === "right";
      gesture = null;
      if (opening) latest.current.onOpen();
      else latest.current.onClose();
    };
    const onEnd = () => {
      gesture = null;
    };
    const passive = { passive: true };
    document.addEventListener("touchstart", onStart, passive);
    document.addEventListener("touchmove", onMove, passive);
    document.addEventListener("touchend", onEnd, passive);
    document.addEventListener("touchcancel", onEnd, passive);
    return () => {
      document.removeEventListener("touchstart", onStart);
      document.removeEventListener("touchmove", onMove);
      document.removeEventListener("touchend", onEnd);
      document.removeEventListener("touchcancel", onEnd);
    };
  }, [enabled, drawerSelector]);
}
