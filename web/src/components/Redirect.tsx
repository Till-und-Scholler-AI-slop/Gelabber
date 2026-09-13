import { useNavigate, type NavigateOptions } from "@tanstack/react-router";
import { useEffect } from "react";

/**
 * Like the router's `<Navigate>`, but fires once per *target*, not once per
 * props object. `<Navigate>` re-navigates whenever it re-renders with a new
 * props literal; while an async `beforeLoad` is pending, that re-render is
 * caused by the navigation itself — a synchronous loop that hangs the tab.
 * Always replaces, so the abandoned page never ends up in history.
 */
export function Redirect(options: NavigateOptions) {
  const navigate = useNavigate();
  const key = JSON.stringify(options);
  useEffect(() => {
    const target = JSON.parse(key) as NavigateOptions;
    void navigate({ ...target, replace: true });
  }, [navigate, key]);
  return null;
}
