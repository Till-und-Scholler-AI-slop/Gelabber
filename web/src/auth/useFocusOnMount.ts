import { useEffect, useRef } from "react";

/**
 * Focuses the element once it mounts. For the heading of a state that
 * replaces a form (mail sent, link expired): the focused button is gone, so
 * focus would fall back to the page. The element needs `tabIndex={-1}`.
 */
export function useFocusOnMount<T extends HTMLElement>() {
  const ref = useRef<T>(null);
  useEffect(() => {
    ref.current?.focus();
  }, []);
  return ref;
}
