import { useCallback, useRef } from "react";

/** A `<details>` used as a pop-up menu closes like one: on Esc (focus goes
 * back to its button), on a press outside it and once one of its entries is
 * chosen. Native `<details>` does none of that by itself. A callback ref, so
 * a menu that mounts later than its component still gets it. */
export function useMenuDetails() {
  const detach = useRef<(() => void) | null>(null);
  return useCallback((details: HTMLDetailsElement | null) => {
    detach.current?.();
    detach.current = null;
    if (!details) return;
    const close = () => details.removeAttribute("open");
    const onPointerDown = (event: Event) => {
      if (details.open && !details.contains(event.target as Node)) close();
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || !details.open) return;
      close();
      details.querySelector("summary")?.focus();
    };
    const onClick = (event: Event) => {
      const target = event.target as Element | null;
      if (target?.closest("a, button") && !target.closest("summary")) close();
    };
    document.addEventListener("pointerdown", onPointerDown, true);
    document.addEventListener("keydown", onKeyDown);
    details.addEventListener("click", onClick);
    detach.current = () => {
      document.removeEventListener("pointerdown", onPointerDown, true);
      document.removeEventListener("keydown", onKeyDown);
      details.removeEventListener("click", onClick);
    };
  }, []);
}
