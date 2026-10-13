import { useCallback, useRef } from "react";

/** A `<details>` used as a pop-up menu closes like one: on Esc (focus goes
 * back to its button when it was inside), when focus leaves it, on a press
 * outside it and once one of its entries is chosen. Native `<details>` does none of that by itself. A callback ref, so
 * a menu that mounts later than its component still gets it. */
export function useMenuDetails() {
  const detach = useRef<(() => void) | null>(null);
  return useCallback((details: HTMLDetailsElement | null) => {
    detach.current?.();
    detach.current = details ? attachMenuDetails(details) : null;
  }, []);
}

/** The listeners behind `useMenuDetails`; returns their removal. */
export function attachMenuDetails(details: HTMLDetailsElement): () => void {
  const close = () => details.removeAttribute("open");
  const onPointerDown = (event: Event) => {
    if (details.open && !details.contains(event.target as Node)) close();
  };
  const onKeyDown = (event: KeyboardEvent) => {
    if (event.key !== "Escape" || !details.open) return;
    // This Esc closed the menu: mark it handled (the browser's own Esc
    // action, and any listener that checks, leave it alone).
    event.preventDefault();
    // Focus only returns to the button when it was in the menu: Esc pressed
    // while typing elsewhere does not pull focus away from there.
    const inside = details.contains(document.activeElement);
    close();
    if (inside) details.querySelector("summary")?.focus();
  };
  const onFocusOut = (event: FocusEvent) => {
    const next = event.relatedTarget as Node | null;
    // Tabbing out of an open menu closes it; a click away from the page
    // (no next target) is left to the pointer handler.
    if (details.open && next && !details.contains(next)) close();
  };
  const onClick = (event: Event) => {
    const target = event.target as Element | null;
    if (target?.closest("a, button") && !target.closest("summary")) close();
  };
  document.addEventListener("pointerdown", onPointerDown, true);
  document.addEventListener("keydown", onKeyDown);
  details.addEventListener("click", onClick);
  details.addEventListener("focusout", onFocusOut);
  return () => {
    document.removeEventListener("pointerdown", onPointerDown, true);
    document.removeEventListener("keydown", onKeyDown);
    details.removeEventListener("click", onClick);
    details.removeEventListener("focusout", onFocusOut);
  };
}
