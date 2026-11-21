// The sidebars' widths as the page gets them (GEL-34): the chosen ones,
// fitted to the window, and written where the stylesheets read them.

import { useEffect, useSyncExternalStore } from "react";

import {
  MEMBERS_DEFAULT,
  SIDEBAR_DEFAULT,
  SIDEBAR_DEFAULT_COARSE,
  fitWidths,
  useSidebarLayout,
  type Fitted,
} from "./sidebarLayout.ts";
import { useMediaQuery } from "./useMediaQuery.ts";

function subscribeResize(onChange: () => void) {
  window.addEventListener("resize", onChange);
  return () => window.removeEventListener("resize", onChange);
}

function useViewportWidth(): number {
  return useSyncExternalStore(
    subscribeResize,
    () => window.innerWidth,
    () => 1280,
  );
}

/** The widths the columns get now: the chosen ones, fitted to the window. */
export function useFittedWidths(): Fitted {
  const viewport = useViewportWidth();
  const coarse = useMediaQuery("(pointer: coarse)");
  const sidebar = useSidebarLayout((state) => state.sidebar);
  const members = useSidebarLayout((state) => state.members);
  const collapsed = useSidebarLayout((state) => state.collapsed);
  const membersShown = useSidebarLayout((state) => state.membersShown > 0);
  return fitWidths({
    viewport,
    sidebar: sidebar ?? (coarse ? SIDEBAR_DEFAULT_COARSE : SIDEBAR_DEFAULT),
    members: members ?? MEMBERS_DEFAULT,
    collapsed,
    membersShown,
  });
}

/** Writes the fitted widths where the stylesheets read them: the columns,
 * the phone drawers and the call bar's left edge. */
export function useSidebarWidthVariables(): void {
  const fitted = useFittedWidths();
  const collapsed = useSidebarLayout((state) => state.collapsed);
  useEffect(() => {
    const root = document.documentElement.style;
    root.setProperty("--lr-sidebar-width", `${fitted.sidebar}px`);
    root.setProperty("--lr-members-width", `${fitted.members}px`);
    root.setProperty(
      "--lr-nav-edge",
      collapsed
        ? "var(--lr-rail-width)"
        : "calc(var(--lr-rail-width) + var(--lr-sidebar-width))",
    );
  }, [fitted.sidebar, fitted.members, collapsed]);
  useEffect(
    () => () => {
      const root = document.documentElement.style;
      root.removeProperty("--lr-sidebar-width");
      root.removeProperty("--lr-members-width");
      root.removeProperty("--lr-nav-edge");
    },
    [],
  );
}
