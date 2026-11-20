// The sidebars' widths on wide screens, and whether the channel list is
// folded away (GEL-34). Remembered per browser, like the member list's
// toggle (memberPanelState.ts). Phones and narrow windows keep their
// drawers; these values only shape the columns beside the page.

import { create } from "zustand";

/** Below this the channel list is a drawer (NAVIGATION_DRAWER_MAX_WIDTH). */
export const NAVIGATION_BREAKPOINT = 800;
/** Below this the member list is a drawer (MemberPanel's breakpoint). */
export const MEMBERS_BREAKPOINT = 1130;

export const RAIL_WIDTH = 64;
export const SIDEBAR_DEFAULT = 224;
/** Touch screens: room for finger-sized actions (index.css). */
export const SIDEBAR_DEFAULT_COARSE = 256;
export const MEMBERS_DEFAULT = 248;
export const WIDTH_MIN = 200;
export const WIDTH_MAX = 360;
/** A drag this far below the minimum folds the sidebar away. */
export const COLLAPSE_BELOW = WIDTH_MIN - 60;
/** The page between the sidebars keeps at least this much. */
export const PAGE_MIN = 440;
/** One arrow key press on a sidebar's edge. */
export const KEY_STEP = 16;

const SIDEBAR_KEY = "gelabber.sidebar.width";
const MEMBERS_KEY = "gelabber.members.width";
const COLLAPSED_KEY = "gelabber.sidebar.collapsed";

export type SidebarSide = "channels" | "members";

type LayoutState = {
  /** null: the default width. */
  sidebar: number | null;
  members: number | null;
  /** The channel list is folded away; the server rail stays. */
  collapsed: boolean;
  /** Member lists on the page now, shown beside it (not hidden). */
  membersShown: number;
};

export function clampWidth(width: number): number {
  return Math.round(Math.min(WIDTH_MAX, Math.max(WIDTH_MIN, width)));
}

function loadWidth(key: string): number | null {
  try {
    const value = localStorage.getItem(key);
    if (value === null) return null;
    const width = Number(value);
    return Number.isFinite(width) ? clampWidth(width) : null;
  } catch {
    return null;
  }
}

function loadFlag(key: string): boolean {
  try {
    return localStorage.getItem(key) === "1";
  } catch {
    return false;
  }
}

function store(key: string, value: string | null): void {
  try {
    if (value === null) localStorage.removeItem(key);
    else localStorage.setItem(key, value);
  } catch {
    /* Storage can be blocked; the layout still changes for this tab. */
  }
}

const useLayout = create<LayoutState>(() => ({
  sidebar: loadWidth(SIDEBAR_KEY),
  members: loadWidth(MEMBERS_KEY),
  collapsed: loadFlag(COLLAPSED_KEY),
  membersShown: 0,
}));

export function useSidebarLayout<T>(select: (state: LayoutState) => T): T {
  return useLayout(select);
}

export function sidebarLayout(): LayoutState {
  return useLayout.getState();
}

/** A width for one side; null goes back to its default. */
export function setSidebarWidth(side: SidebarSide, width: number | null) {
  const value = width === null ? null : clampWidth(width);
  if (side === "channels") {
    if (useLayout.getState().sidebar === value) return;
    useLayout.setState({ sidebar: value });
    store(SIDEBAR_KEY, value === null ? null : String(value));
  } else {
    if (useLayout.getState().members === value) return;
    useLayout.setState({ members: value });
    store(MEMBERS_KEY, value === null ? null : String(value));
  }
}

export function setChannelListCollapsed(collapsed: boolean): void {
  if (useLayout.getState().collapsed === collapsed) return;
  useLayout.setState({ collapsed });
  store(COLLAPSED_KEY, collapsed ? "1" : null);
}

/** A member list beside the page says it is there; the returned function
 * says it went. The channel list gives way only to a list that is shown. */
export function memberListShown(): () => void {
  useLayout.setState((state) => ({ membersShown: state.membersShown + 1 }));
  return () =>
    useLayout.setState((state) => ({ membersShown: state.membersShown - 1 }));
}

export function toggleChannelList(): void {
  setChannelListCollapsed(!useLayout.getState().collapsed);
}

export type Fitted = { sidebar: number; members: number };

/**
 * The widths the columns get in a window `viewport` wide: the chosen ones,
 * unless the page would drop under PAGE_MIN. Then the member list gives way
 * first, then the channel list, each down to WIDTH_MIN. A column that is a
 * drawer at this width, folded or hidden takes no room.
 */
export function fitWidths({
  viewport,
  sidebar,
  members,
  collapsed,
  membersShown,
}: {
  viewport: number;
  sidebar: number;
  members: number;
  collapsed: boolean;
  membersShown: boolean;
}): Fitted {
  const sidebarTakes = !collapsed && viewport > NAVIGATION_BREAKPOINT;
  const membersTake = membersShown && viewport > MEMBERS_BREAKPOINT;
  const railTakes = viewport > NAVIGATION_BREAKPOINT ? RAIL_WIDTH : 0;
  let over =
    railTakes +
    (sidebarTakes ? sidebar : 0) +
    (membersTake ? members : 0) +
    PAGE_MIN -
    viewport;
  let fittedMembers = members;
  let fittedSidebar = sidebar;
  if (over > 0 && membersTake) {
    const give = Math.min(over, Math.max(0, members - WIDTH_MIN));
    fittedMembers -= give;
    over -= give;
  }
  if (over > 0 && sidebarTakes) {
    const give = Math.min(over, Math.max(0, sidebar - WIDTH_MIN));
    fittedSidebar -= give;
  }
  return { sidebar: fittedSidebar, members: fittedMembers };
}

/** What a key does on a sidebar's edge (the WAI-ARIA window splitter):
 * the new width, "collapse", or null for a key it leaves alone. Arrows
 * move the edge, so on the right-hand member list Left widens it. */
export function edgeKey(
  side: SidebarSide,
  key: string,
  width: number,
): number | "collapse" | null {
  const widen = side === "channels" ? "ArrowRight" : "ArrowLeft";
  const narrow = side === "channels" ? "ArrowLeft" : "ArrowRight";
  if (key === widen) return clampWidth(width + KEY_STEP);
  if (key === narrow) return clampWidth(width - KEY_STEP);
  if (key === "Home") return WIDTH_MIN;
  if (key === "End") return WIDTH_MAX;
  if (key === "Enter") return "collapse";
  return null;
}

/** The width a drag asks for, or "collapse" once it goes well under the
 * minimum. `delta` is how far the pointer moved to the right. */
export function dragWidth(
  side: SidebarSide,
  start: number,
  delta: number,
): number | "collapse" {
  const raw = side === "channels" ? start + delta : start - delta;
  if (raw < COLLAPSE_BELOW) return "collapse";
  return clampWidth(raw);
}
