// The channel list's fold button and the draggable edges of both sidebars
// (GEL-34). Wide screens with a mouse only: phones keep their drawers, and
// touch screens get the button but no edge (index.css).

import { useRef, useState } from "react";

import { SidebarIcon } from "./Icons.tsx";
import {
  WIDTH_MAX,
  WIDTH_MIN,
  dragWidth,
  edgeKey,
  setChannelListCollapsed,
  setSidebarWidth,
  toggleChannelList,
  useSidebarLayout,
  type SidebarSide,
} from "./sidebarLayout.ts";
import { setMemberPanelHidden } from "./memberPanelState.ts";
import { useT } from "../i18n/catalog.ts";
import { shell } from "../i18n/catalogs/shell.ts";

/** The fold button's id, where the edge hands the focus when Enter folds
 * the channel list away. */
export const CHANNEL_LIST_TOGGLE_ID = "channel-list-toggle";

/** The header button at the page's top left: fold the channel list away
 * or bring it back. The server rail stays either way. */
export function ChannelListToggle({ controls }: { controls: string }) {
  const t = useT(shell);
  const collapsed = useSidebarLayout((state) => state.collapsed);
  return (
    <button
      type="button"
      id={CHANNEL_LIST_TOGGLE_ID}
      className={`lr-header-icon channel-list-toggle${collapsed ? "" : " is-on"}`}
      aria-expanded={!collapsed}
      aria-controls={controls}
      aria-label={t("channelList.toggle")}
      title={collapsed ? t("channelList.show") : t("channelList.hide")}
      onClick={() => toggleChannelList()}
    >
      <SidebarIcon size={18} />
    </button>
  );
}

function collapse(side: SidebarSide): void {
  if (side === "channels") setChannelListCollapsed(true);
  else setMemberPanelHidden(true);
}

function expand(side: SidebarSide): void {
  if (side === "channels") setChannelListCollapsed(false);
  else setMemberPanelHidden(false);
}

/** Where the focus goes once Enter folded a side away: its button. */
function focusToggle(side: SidebarSide): void {
  const button =
    side === "channels"
      ? document.getElementById(CHANNEL_LIST_TOGGLE_ID)
      : document.querySelector<HTMLElement>(".member-panel-toggle");
  button?.focus({ preventScroll: true });
}

/**
 * A sidebar's edge: drag it to set the width, double-click it for the
 * default, or focus it and use the arrow keys, Home, End and Enter (the
 * WAI-ARIA window splitter). Dragging well under the minimum folds the
 * sidebar away; dragging back out brings it back while the button is held.
 */
export function SidebarEdge({
  side,
  width,
  controls,
  hidden = false,
}: {
  side: SidebarSide;
  /** The width the sidebar has now. */
  width: number;
  /** The sidebar's id. */
  controls?: string;
  /** Folded away: the edge goes too, unless a drag is still holding it. */
  hidden?: boolean;
}) {
  const t = useT(shell);
  const drag = useRef<{ x: number; width: number; pointer: number } | null>(
    null,
  );
  const [dragging, setDragging] = useState(false);
  if (hidden && !dragging) return null;
  const label =
    side === "channels" ? t("edge.channelsLabel") : t("edge.membersLabel");

  return (
    <div
      role="separator"
      tabIndex={0}
      aria-orientation="vertical"
      aria-label={label}
      aria-controls={controls}
      aria-valuenow={width}
      aria-valuemin={WIDTH_MIN}
      aria-valuemax={WIDTH_MAX}
      title={t("edge.hint")}
      className={`sidebar-edge is-${side}${dragging ? " is-dragging" : ""}`}
      onPointerDown={(event) => {
        if (event.button !== 0 || !event.isPrimary) return;
        event.preventDefault();
        event.currentTarget.setPointerCapture(event.pointerId);
        drag.current = { x: event.clientX, width, pointer: event.pointerId };
        setDragging(true);
      }}
      onPointerMove={(event) => {
        const start = drag.current;
        if (!start || start.pointer !== event.pointerId) return;
        const next = dragWidth(side, start.width, event.clientX - start.x);
        if (next === "collapse") {
          collapse(side);
          return;
        }
        expand(side);
        setSidebarWidth(side, next);
      }}
      onPointerUp={(event) => {
        if (drag.current?.pointer !== event.pointerId) return;
        drag.current = null;
        setDragging(false);
      }}
      onPointerCancel={() => {
        drag.current = null;
        setDragging(false);
      }}
      onLostPointerCapture={() => {
        drag.current = null;
        setDragging(false);
      }}
      onDoubleClick={() => setSidebarWidth(side, null)}
      onKeyDown={(event) => {
        const next = edgeKey(side, event.key, width);
        if (next === null) return;
        event.preventDefault();
        if (next === "collapse") {
          collapse(side);
          focusToggle(side);
          return;
        }
        setSidebarWidth(side, next);
      }}
    />
  );
}
